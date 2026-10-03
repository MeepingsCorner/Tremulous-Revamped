//! qcommon `cmd.c` / `cvar.c` for the client side of the port: the command
//! buffer (`Cbuf_AddText`, `Cbuf_InsertText`, `Cbuf_Execute`'s line
//! splitting), the tokenizer (`Cmd_TokenizeString`) and the client cvars
//! (`Cvar_Get`, `Cvar_Set2`, `Cvar_Command`, `Cvar_Toggle_f`, `Cvar_Set_f`,
//! `Cvar_Reset_f`, `Cvar_WriteVariables`). Pure Rust (unit tested); the
//! command dispatch (`Cmd_ExecuteString`) is `clientcmd.rs`.

use trem_sim::kernel::spawn::{c_atof, c_atoi};

/// qcommon.h MAX_CMD_LINE
const MAX_CMD_LINE: usize = 1024;
/// q_shared.h MAX_STRING_TOKENS
const MAX_STRING_TOKENS: usize = 1024;

/// q_shared.h CVAR_ARCHIVE: written to autogen.cfg.
pub const CVAR_ARCHIVE: i32 = 0x0001;
/// q_shared.h CVAR_USERINFO
pub const CVAR_USERINFO: i32 = 0x0002;
/// q_shared.h CVAR_SERVERINFO
pub const CVAR_SERVERINFO: i32 = 0x0004;
/// q_shared.h CVAR_ROM
pub const CVAR_ROM: i32 = 0x0040;
/// q_shared.h CVAR_USER_CREATED: created by a set command
pub const CVAR_USER_CREATED: i32 = 0x0080;

/// `Cmd_TokenizeString` (`Cmd_TokenizeString2` with quotes): whitespace
/// separated tokens, "quoted strings" as one token (no \" escaping), `//`
/// and `/* */` comments skipped.
pub fn tokenize(text: &str) -> Vec<String> {
    tokenize2(text, false)
}

/// `Cmd_TokenizeStringIgnoreQuotes`: quotes are ordinary characters (tab
/// completion's view of the console line).
pub fn tokenize_ignore_quotes(text: &str) -> Vec<String> {
    tokenize2(text, true)
}

/// `Cmd_TokenizeString2`.
fn tokenize2(text: &str, ignore_quotes: bool) -> Vec<String> {
    let b = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    loop {
        if out.len() == MAX_STRING_TOKENS {
            return out;
        }
        loop {
            // skip whitespace
            while i < b.len() && b[i] <= b' ' {
                i += 1;
            }
            if i >= b.len() {
                return out;
            }
            // skip // comments
            if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
                return out;
            }
            // skip /* */ comments
            if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                if i >= b.len() {
                    return out;
                }
                i += 2;
            } else {
                break;
            }
        }
        // handle quoted strings
        if !ignore_quotes && b[i] == b'"' {
            i += 1;
            let s = i;
            while i < b.len() && b[i] != b'"' {
                i += 1;
            }
            out.push(String::from_utf8_lossy(&b[s..i]).into_owned());
            if i >= b.len() {
                return out;
            }
            i += 1;
            continue;
        }
        // regular token: until whitespace, quote or comment
        let s = i;
        while i < b.len() && b[i] > b' ' {
            if !ignore_quotes && b[i] == b'"' {
                break;
            }
            if b[i] == b'/' && matches!(b.get(i + 1), Some(b'/') | Some(b'*')) {
                break;
            }
            i += 1;
        }
        out.push(String::from_utf8_lossy(&b[s..i]).into_owned());
        if i >= b.len() {
            return out;
        }
    }
}

/// `Cmd_ArgsFrom`: argv[n..] joined by single spaces.
pub fn args_from(argv: &[String], n: usize) -> String {
    argv.get(n..).map(|a| a.join(" ")).unwrap_or_default()
}

/// The command buffer (`cmd_text`) with the `wait` counter.
#[derive(Default, Debug)]
pub struct Cbuf {
    text: String,
    /// `cmd_wait`
    pub wait: i32,
}

impl Cbuf {
    /// `Cbuf_AddText`: append to the end of the buffer.
    pub fn add_text(&mut self, t: &str) {
        self.text.push_str(t);
    }

    /// `Cbuf_InsertText`: insert t and a newline at the start, so it runs
    /// right after the current command.
    pub fn insert_text(&mut self, t: &str) {
        self.text.insert(0, '\n');
        self.text.insert_str(0, t);
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// One pass of `Cbuf_Execute`'s loop: take the next line (split on `;`
    /// or a newline outside quotes and comments). None when the buffer is
    /// empty or a `wait` holds the rest for the next frame. `comments` is
    /// the (in_star_comment, in_slash_comment) state of this Cbuf_Execute.
    pub fn next_line(&mut self, comments: &mut (bool, bool)) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }
        if self.wait > 0 {
            // skip out while text still remains in buffer, leaving it for next frame
            self.wait -= 1;
            return None;
        }
        let (in_star, in_slash) = comments;
        let b = self.text.as_bytes();
        let mut quotes = 0;
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'"' {
                quotes += 1;
            }
            if quotes & 1 == 0 {
                if i + 1 < b.len() {
                    if !*in_star && b[i] == b'/' && b[i + 1] == b'/' {
                        *in_slash = true;
                    } else if !*in_slash && b[i] == b'/' && b[i + 1] == b'*' {
                        *in_star = true;
                    } else if *in_star && b[i] == b'*' && b[i + 1] == b'/' {
                        *in_star = false;
                        // the part after the comment is valid
                        i += 1;
                        break;
                    }
                }
                if !*in_slash && !*in_star && b[i] == b';' {
                    break;
                }
            }
            if !*in_star && (b[i] == b'\n' || b[i] == b'\r') {
                *in_slash = false;
                break;
            }
            i += 1;
        }
        let i = i.min(MAX_CMD_LINE - 1).min(b.len());
        let line = String::from_utf8_lossy(&b[..i]).into_owned();
        // delete the text from the command buffer (and the separator)
        let rest = (i + 1).min(b.len());
        self.text.drain(..rest);
        Some(line)
    }
}

/// `cvar_t` (the fields the client uses).
#[derive(Clone, Debug)]
pub struct Cvar {
    pub name: String,
    pub string: String,
    pub reset_string: String,
    pub flags: i32,
    pub modified: bool,
}

/// The client's cvars (`cvar_vars`) and `cvar_modifiedFlags`.
#[derive(Default, Debug)]
pub struct Cvars {
    vars: Vec<Cvar>,
    pub modified_flags: i32,
}

impl Cvars {
    fn index(&self, name: &str) -> Option<usize> {
        self.vars.iter().position(|v| v.name.eq_ignore_ascii_case(name))
    }

    /// `Cvar_FindVar`.
    pub fn find(&self, name: &str) -> Option<&Cvar> {
        self.index(name).map(|i| &self.vars[i])
    }

    /// `Cvar_Get`: register (or re-flag) a cvar with its default.
    pub fn get(&mut self, name: &str, value: &str, flags: i32) {
        match self.index(name) {
            Some(i) => {
                let v = &mut self.vars[i];
                // a cvar created by a set command takes the code's default as reset value
                if v.flags & CVAR_USER_CREATED != 0 {
                    v.flags &= !CVAR_USER_CREATED;
                    v.reset_string = value.to_string();
                }
                v.flags |= flags;
                self.modified_flags |= flags;
            }
            None => {
                self.vars.push(Cvar { name: name.to_string(), string: value.to_string(), reset_string: value.to_string(), flags, modified: true });
                self.modified_flags |= flags;
            }
        }
    }

    /// Every cvar (`Cvar_CommandCompletion`'s walk).
    pub fn iter(&self) -> impl Iterator<Item = &Cvar> {
        self.vars.iter()
    }

    /// `Cvar_VariableString` ("" when unset).
    pub fn string(&self, name: &str) -> String {
        self.find(name).map(|v| v.string.clone()).unwrap_or_default()
    }

    /// `Cvar_VariableValue` (atof).
    pub fn value(&self, name: &str) -> f32 {
        self.find(name).map(|v| c_atof(&v.string) as f32).unwrap_or(0.0)
    }

    /// `Cvar_VariableIntegerValue` (atoi).
    pub fn integer(&self, name: &str) -> i32 {
        self.find(name).map(|v| c_atoi(&v.string)).unwrap_or(0)
    }

    /// `Cvar_Set2` (unforced): creates a user cvar when missing.
    pub fn set(&mut self, name: &str, value: &str) -> &mut Cvar {
        let i = match self.index(name) {
            Some(i) => i,
            None => {
                self.vars.push(Cvar { name: name.to_string(), string: value.to_string(), reset_string: value.to_string(), flags: CVAR_USER_CREATED, modified: true });
                return self.vars.last_mut().unwrap();
            }
        };
        let v = &mut self.vars[i];
        if v.string != value {
            self.modified_flags |= v.flags;
            v.string = value.to_string();
            v.modified = true;
        }
        v
    }

    /// `Cvar_InfoString`: "\\name\\value" of the cvars with `flag` (userinfo).
    pub fn info_string(&self, flag: i32) -> String {
        let mut s = String::new();
        for v in self.vars.iter().filter(|v| v.flags & flag != 0 && !v.string.is_empty()) {
            s.push_str(&format!("\\{}\\{}", v.name, v.string));
        }
        s
    }

    /// Take the `modified` flag of a cvar (`vmCvar_t.modificationCount` checks).
    pub fn take_modified(&mut self, name: &str) -> bool {
        self.index(name).is_some_and(|i| std::mem::take(&mut self.vars[i].modified))
    }

    /// `Cvar_Print`.
    pub fn print(v: &Cvar) -> String {
        let mut s = format!("\"{}\" is:\"{}^7\"", v.name, v.string);
        if v.flags & CVAR_ROM == 0 {
            if v.string.eq_ignore_ascii_case(&v.reset_string) {
                s.push_str(", the default");
            } else {
                s.push_str(&format!(" default:\"{}^7\"", v.reset_string));
            }
        }
        s.push('\n');
        s
    }

    /// `Cvar_Command`: a cvar name as a command prints or sets it. None
    /// when argv[0] is not a cvar.
    pub fn command(&mut self, argv: &[String]) -> Option<String> {
        let name = argv.first()?;
        self.index(name)?;
        if argv.len() == 1 {
            return Some(Self::print(self.find(name).unwrap()));
        }
        // Cmd_Args
        self.set(name, &args_from(argv, 1));
        Some(String::new())
    }

    /// `Cvar_Toggle_f`: 0/1, or cycle through the given values.
    pub fn toggle_f(&mut self, argv: &[String]) -> String {
        let c = argv.len();
        if c < 2 {
            return "usage: toggle <variable> [value1, value2, ...]\n".into();
        }
        if c == 2 {
            let v = if self.value(&argv[1]) != 0.0 { "0" } else { "1" };
            self.set(&argv[1], v);
            return String::new();
        }
        if c == 3 {
            return "toggle: nothing to toggle to\n".into();
        }
        let cur = self.string(&argv[1]);
        // the last value need not be checked: no match = the first value
        for i in 2..c - 1 {
            if cur == argv[i] {
                let next = argv[i + 1].clone();
                self.set(&argv[1], &next);
                return String::new();
            }
        }
        let first = argv[2].clone();
        self.set(&argv[1], &first);
        String::new()
    }

    /// `Cvar_Set_f` (set / seta / setu / sets).
    pub fn set_f(&mut self, argv: &[String]) -> String {
        let cmd = argv[0].to_ascii_lowercase();
        match argv.len() {
            0 | 1 => return format!("usage: {} <variable> <value>\n", argv[0]),
            2 => {
                return match self.find(&argv[1]) {
                    Some(v) => Self::print(v),
                    None => format!("Cvar {} does not exist.\n", argv[1]),
                }
            }
            _ => {}
        }
        let value = args_from(argv, 2);
        let v = self.set(&argv[1], &value);
        let flag = match cmd.as_bytes().get(3) {
            Some(b'a') => CVAR_ARCHIVE,
            Some(b'u') => CVAR_USERINFO,
            Some(b's') => CVAR_SERVERINFO,
            _ => 0,
        };
        if flag != 0 && v.flags & flag == 0 {
            v.flags |= flag;
            self.modified_flags |= flag;
        }
        String::new()
    }

    /// `Cvar_Reset_f`.
    pub fn reset_f(&mut self, argv: &[String]) -> String {
        if argv.len() != 2 {
            return "usage: reset <variable>\n".into();
        }
        if let Some(r) = self.find(&argv[1]).map(|v| v.reset_string.clone()) {
            self.set(&argv[1], &r);
        }
        String::new()
    }

    /// `Cvar_List_f` (name and value only).
    pub fn list_f(&self) -> String {
        let mut s = String::new();
        for v in &self.vars {
            s.push_str(&format!("{} {} \"{}\"\n", if v.flags & CVAR_ARCHIVE != 0 { "A" } else { " " }, v.name, v.string));
        }
        s.push_str(&format!("\n{} total cvars\n", self.vars.len()));
        s
    }

    /// `Cvar_WriteVariables`: `seta name "value"` for every archived cvar.
    pub fn write_variables(&self) -> String {
        let mut s = String::new();
        for v in self.vars.iter().filter(|v| v.flags & CVAR_ARCHIVE != 0) {
            s.push_str(&format!("seta {} \"{}\"\n", v.name, v.string));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<String> {
        tokenize(s)
    }

    #[test]
    fn tokenize_quotes_and_comments() {
        assert_eq!(t("bind  b             \"buy ammo\""), vec!["bind", "b", "buy ammo"]);
        assert_eq!(t("bind x boost // sprint"), vec!["bind", "x", "boost"]);
        assert_eq!(t("a /* c */ b"), vec!["a", "b"]);
        assert_eq!(t("say \"hi there\"now"), vec!["say", "hi there", "now"]);
        assert_eq!(t("  "), Vec::<String>::new());
        assert_eq!(t("echo \"unterminated"), vec!["echo", "unterminated"]);
    }

    fn lines(text: &str) -> Vec<String> {
        let mut c = Cbuf::default();
        c.add_text(text);
        let mut st = (false, false);
        let mut out = Vec::new();
        while let Some(l) = c.next_line(&mut st) {
            out.push(l);
        }
        out
    }

    #[test]
    fn cbuf_splits_on_semicolons_outside_quotes() {
        assert_eq!(lines("a;b\nc"), vec!["a", "b", "c"]);
        assert_eq!(lines("bind q \"+a;+b\"; c"), vec!["bind q \"+a;+b\"", " c"]);
        // a // comment keeps its semicolons
        assert_eq!(lines("x // y; z\nw"), vec!["x // y; z", "w"]);
    }

    #[test]
    fn cbuf_insert_runs_first_and_wait_defers() {
        let mut c = Cbuf::default();
        c.add_text("b\n");
        c.insert_text("a");
        let mut st = (false, false);
        assert_eq!(c.next_line(&mut st).as_deref(), Some("a"));
        c.wait = 1;
        assert_eq!(c.next_line(&mut st), None);
        assert_eq!(c.next_line(&mut st).as_deref(), Some("b"));
    }

    #[test]
    fn cvar_set_toggle_archive() {
        let mut c = Cvars::default();
        c.get("sensitivity", "5", CVAR_ARCHIVE);
        c.modified_flags = 0;
        assert_eq!(c.command(&t("sensitivity")).unwrap(), "\"sensitivity\" is:\"5^7\", the default\n");
        c.command(&t("sensitivity 3.5"));
        assert_eq!(c.value("sensitivity"), 3.5);
        assert_eq!(c.modified_flags & CVAR_ARCHIVE, CVAR_ARCHIVE);
        c.toggle_f(&t("toggle cg_thirdPerson"));
        assert_eq!(c.integer("cg_thirdperson"), 1);
        c.toggle_f(&t("toggle cg_thirdPerson"));
        assert_eq!(c.integer("cg_thirdperson"), 0);
        c.toggle_f(&t("toggle r a b c"));
        assert_eq!(c.string("r"), "a");
        c.toggle_f(&t("toggle r a b c"));
        assert_eq!(c.string("r"), "b");
        c.set_f(&t("seta mine \"hello world\""));
        assert!(c.write_variables().contains("seta mine \"hello world\"\n"));
        assert!(c.write_variables().contains("seta sensitivity \"3.5\"\n"));
        c.reset_f(&t("reset sensitivity"));
        assert_eq!(c.string("sensitivity"), "5");
    }
}
