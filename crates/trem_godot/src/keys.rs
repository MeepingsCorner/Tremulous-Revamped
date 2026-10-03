//! client `cl_keys.c` / `keycodes.h`: key numbers and names
//! (`Key_StringToKeynum`, `Key_KeynumToString`), the bind table
//! (`Key_SetBinding`, `Key_GetBinding`, `Key_GetKey`, `Key_Bind_f`,
//! `Key_Unbind_f`, `Key_Unbindall_f`, `Key_Bindlist_f`, `Key_WriteBindings`)
//! and the key event path (`CL_KeyDownEvent`, `CL_KeyUpEvent`,
//! `CL_ParseBinding`, `Key_ClearStates`). Godot keys and mouse buttons map
//! onto keycodes.h numbers like sdl_input.c `IN_TranslateSDLToQ3Key` does.

use godot::global::{Key, MouseButton};
use godot::obj::EngineEnum;

// keycodes.h keyNum_t (enum order; normal keys are lowercase ascii)
pub const K_TAB: i32 = 9;
pub const K_ENTER: i32 = 13;
pub const K_ESCAPE: i32 = 27;
pub const K_SPACE: i32 = 32;
pub const K_BACKSPACE: i32 = 127;
pub const K_COMMAND: i32 = 128;
pub const K_CAPSLOCK: i32 = 129;
pub const K_POWER: i32 = 130;
pub const K_PAUSE: i32 = 131;
pub const K_UPARROW: i32 = 132;
pub const K_DOWNARROW: i32 = 133;
pub const K_LEFTARROW: i32 = 134;
pub const K_RIGHTARROW: i32 = 135;
pub const K_ALT: i32 = 136;
pub const K_CTRL: i32 = 137;
pub const K_SHIFT: i32 = 138;
pub const K_INS: i32 = 139;
pub const K_DEL: i32 = 140;
pub const K_PGDN: i32 = 141;
pub const K_PGUP: i32 = 142;
pub const K_HOME: i32 = 143;
pub const K_END: i32 = 144;
pub const K_F1: i32 = 145;
// K_F2 .. K_F15 = K_F1 + 1 .. K_F1 + 14
pub const K_KP_HOME: i32 = 160;
pub const K_KP_UPARROW: i32 = 161;
pub const K_KP_PGUP: i32 = 162;
pub const K_KP_LEFTARROW: i32 = 163;
pub const K_KP_5: i32 = 164;
pub const K_KP_RIGHTARROW: i32 = 165;
pub const K_KP_END: i32 = 166;
pub const K_KP_DOWNARROW: i32 = 167;
pub const K_KP_PGDN: i32 = 168;
pub const K_KP_ENTER: i32 = 169;
pub const K_KP_INS: i32 = 170;
pub const K_KP_DEL: i32 = 171;
pub const K_KP_SLASH: i32 = 172;
pub const K_KP_MINUS: i32 = 173;
pub const K_KP_PLUS: i32 = 174;
pub const K_KP_NUMLOCK: i32 = 175;
pub const K_KP_STAR: i32 = 176;
pub const K_KP_EQUALS: i32 = 177;
pub const K_MOUSE1: i32 = 178;
pub const K_MOUSE2: i32 = 179;
pub const K_MOUSE3: i32 = 180;
pub const K_MOUSE4: i32 = 181;
pub const K_MOUSE5: i32 = 182;
pub const K_MWHEELDOWN: i32 = 183;
pub const K_MWHEELUP: i32 = 184;
pub const K_JOY1: i32 = 185;
pub const K_AUX1: i32 = 217;
pub const K_WORLD_0: i32 = 233;
pub const K_SUPER: i32 = 329;
pub const K_COMPOSE: i32 = 330;
pub const K_MODE: i32 = 331;
pub const K_HELP: i32 = 332;
pub const K_PRINT: i32 = 333;
pub const K_SYSREQ: i32 = 334;
pub const K_SCROLLOCK: i32 = 335;
pub const K_BREAK: i32 = 336;
pub const K_MENU: i32 = 337;
pub const K_EURO: i32 = 338;
pub const K_UNDO: i32 = 339;
/// Pseudo-key that brings the console down (cl_consoleKeys; never bound).
pub const K_CONSOLE: i32 = 340;
pub const MAX_KEYS: i32 = 341;
/// K_LAST_KEY: the key range cgame / ui scan for bindings.
pub const K_LAST_KEY: i32 = 256;

/// client.h KEYCATCH_* (`Key_GetCatcher`).
pub const KEYCATCH_CONSOLE: i32 = 0x0001;
pub const KEYCATCH_UI: i32 = 0x0002;

/// cl_keys.c `keynames[]` (the fixed names; JOYn, AUXn and WORLD_n are generated).
const KEYNAMES: &[(&str, i32)] = &[
    ("TAB", K_TAB),
    ("ENTER", K_ENTER),
    ("ESCAPE", K_ESCAPE),
    ("SPACE", K_SPACE),
    ("BACKSPACE", K_BACKSPACE),
    ("UPARROW", K_UPARROW),
    ("DOWNARROW", K_DOWNARROW),
    ("LEFTARROW", K_LEFTARROW),
    ("RIGHTARROW", K_RIGHTARROW),
    ("ALT", K_ALT),
    ("CTRL", K_CTRL),
    ("SHIFT", K_SHIFT),
    ("COMMAND", K_COMMAND),
    ("CAPSLOCK", K_CAPSLOCK),
    ("F1", K_F1),
    ("F2", K_F1 + 1),
    ("F3", K_F1 + 2),
    ("F4", K_F1 + 3),
    ("F5", K_F1 + 4),
    ("F6", K_F1 + 5),
    ("F7", K_F1 + 6),
    ("F8", K_F1 + 7),
    ("F9", K_F1 + 8),
    ("F10", K_F1 + 9),
    ("F11", K_F1 + 10),
    ("F12", K_F1 + 11),
    ("F13", K_F1 + 12),
    ("F14", K_F1 + 13),
    ("F15", K_F1 + 14),
    ("INS", K_INS),
    ("DEL", K_DEL),
    ("PGDN", K_PGDN),
    ("PGUP", K_PGUP),
    ("HOME", K_HOME),
    ("END", K_END),
    ("MOUSE1", K_MOUSE1),
    ("MOUSE2", K_MOUSE2),
    ("MOUSE3", K_MOUSE3),
    ("MOUSE4", K_MOUSE4),
    ("MOUSE5", K_MOUSE5),
    ("MWHEELUP", K_MWHEELUP),
    ("MWHEELDOWN", K_MWHEELDOWN),
    ("KP_HOME", K_KP_HOME),
    ("KP_UPARROW", K_KP_UPARROW),
    ("KP_PGUP", K_KP_PGUP),
    ("KP_LEFTARROW", K_KP_LEFTARROW),
    ("KP_5", K_KP_5),
    ("KP_RIGHTARROW", K_KP_RIGHTARROW),
    ("KP_END", K_KP_END),
    ("KP_DOWNARROW", K_KP_DOWNARROW),
    ("KP_PGDN", K_KP_PGDN),
    ("KP_ENTER", K_KP_ENTER),
    ("KP_INS", K_KP_INS),
    ("KP_DEL", K_KP_DEL),
    ("KP_SLASH", K_KP_SLASH),
    ("KP_MINUS", K_KP_MINUS),
    ("KP_PLUS", K_KP_PLUS),
    ("KP_NUMLOCK", K_KP_NUMLOCK),
    ("KP_STAR", K_KP_STAR),
    ("KP_EQUALS", K_KP_EQUALS),
    ("PAUSE", K_PAUSE),
    // because a raw semicolon seperates commands
    ("SEMICOLON", b';' as i32),
    ("WINDOWS", K_SUPER),
    ("COMPOSE", K_COMPOSE),
    ("MODE", K_MODE),
    ("HELP", K_HELP),
    ("PRINT", K_PRINT),
    ("SYSREQ", K_SYSREQ),
    ("SCROLLOCK", K_SCROLLOCK),
    ("BREAK", K_BREAK),
    ("MENU", K_MENU),
    ("POWER", K_POWER),
    ("EURO", K_EURO),
    ("UNDO", K_UNDO),
];

/// The generated part of `keynames[]`: JOY1..32, AUX1..16, WORLD_0..95
/// (listed after AUX in the C table, before WINDOWS).
fn generated_name(keynum: i32) -> Option<String> {
    match keynum {
        k if (K_JOY1..K_JOY1 + 32).contains(&k) => Some(format!("JOY{}", k - K_JOY1 + 1)),
        k if (K_AUX1..K_AUX1 + 16).contains(&k) => Some(format!("AUX{}", k - K_AUX1 + 1)),
        k if (K_WORLD_0..K_WORLD_0 + 96).contains(&k) => Some(format!("WORLD_{}", k - K_WORLD_0)),
        _ => None,
    }
}

fn generated_keynum(name: &str) -> Option<i32> {
    let u = name.to_ascii_uppercase();
    let num = |p: &str, lo: i32, hi: i32| -> Option<i32> {
        let rest = u.strip_prefix(p)?;
        let n: i32 = rest.parse().ok()?;
        (n.to_string() == rest && (lo..=hi).contains(&n)).then_some(n)
    };
    if let Some(n) = num("JOY", 1, 32) {
        return Some(K_JOY1 + n - 1);
    }
    if let Some(n) = num("AUX", 1, 16) {
        return Some(K_AUX1 + n - 1);
    }
    num("WORLD_", 0, 95).map(|n| K_WORLD_0 + n)
}

/// `Key_KeynameCompletion`: every `keynames[]` name in the C table's order
/// (the fixed names up to MWHEELDOWN, JOY1..32, AUX1..16, the keypad,
/// PAUSE, SEMICOLON, WORLD_0..95, then WINDOWS..UNDO).
pub fn keyname_completion_list() -> Vec<String> {
    let mut out = Vec::new();
    for (name, k) in KEYNAMES {
        if *k == K_KP_HOME {
            out.extend((K_JOY1..K_JOY1 + 32).chain(K_AUX1..K_AUX1 + 16).filter_map(generated_name));
        }
        if *k == K_SUPER {
            out.extend((K_WORLD_0..K_WORLD_0 + 96).filter_map(generated_name));
        }
        out.push(name.to_string());
    }
    out
}

/// `Com_HexStrToInt` ("0x" followed by hex digits, else -1).
fn hex_str_to_int(s: &str) -> i32 {
    let b = s.as_bytes();
    if b.len() < 3 || b[0] != b'0' || (b[1] != b'x' && b[1] != b'X') {
        return -1;
    }
    let mut n: i32 = 0;
    for &c in &b[2..] {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return -1,
        };
        n = n * 16 + d as i32;
    }
    n
}

/// `Key_StringToKeynum`: single characters are their lowercase ascii,
/// "0xnn" is raw hex, else the `keynames[]` names (case-insensitive); -1
/// when unknown.
pub fn key_string_to_keynum(s: &str) -> i32 {
    let b = s.as_bytes();
    if b.is_empty() {
        return -1;
    }
    if b.len() == 1 {
        return b[0].to_ascii_lowercase() as i32;
    }
    // check for hex code
    if b.len() == 4 {
        let n = hex_str_to_int(s);
        if n >= 0 {
            return n;
        }
    }
    // scan for a text match
    for (name, k) in KEYNAMES {
        if name.eq_ignore_ascii_case(s) {
            return *k;
        }
    }
    generated_keynum(s).unwrap_or(-1)
}

/// `Key_KeynumToString`: a printable ascii character, a `keynames[]` name,
/// or a "0xnn" hex string.
pub fn key_keynum_to_string(keynum: i32) -> String {
    if keynum == -1 {
        return "<KEY NOT FOUND>".into();
    }
    if keynum < 0 || keynum >= MAX_KEYS {
        return "<OUT OF RANGE>".into();
    }
    // check for printable ascii (don't use quote)
    if keynum > 32 && keynum < 127 && keynum != b'"' as i32 && keynum != b';' as i32 {
        return (keynum as u8 as char).to_string();
    }
    // check for a key string (the C table order: fixed names up to AUX16,
    // then WORLD_n, then WINDOWS..UNDO)
    if let Some((name, _)) = KEYNAMES.iter().find(|(_, k)| *k == keynum) {
        return name.to_string();
    }
    if let Some(n) = generated_name(keynum) {
        return n;
    }
    // make a hex string
    let hex = |d: i32| if d > 9 { (d - 10 + b'a' as i32) as u8 as char } else { (d + b'0' as i32) as u8 as char };
    format!("0x{}{}", hex(keynum >> 4), hex(keynum & 15))
}

/// sdl_input.c `IN_TranslateSDLToQ3Key` for a Godot key (keycode, i.e. the
/// layout's key like SDL_Keysym.sym): printable keys are their lowercase
/// ascii; the console keys (cl_consoleKeys "~ ` 0x7e 0x60") become
/// K_CONSOLE. 0 when the key has no Q3 number.
pub fn keynum_from_godot_key(k: Key) -> i32 {
    let ord = k.ord();
    let key = if (32..127).contains(&ord) {
        (ord as u8).to_ascii_lowercase() as i32
    } else {
        match k {
            Key::PAGEUP => K_PGUP,
            Key::KP_9 => K_KP_PGUP,
            Key::PAGEDOWN => K_PGDN,
            Key::KP_3 => K_KP_PGDN,
            Key::KP_7 => K_KP_HOME,
            Key::HOME => K_HOME,
            Key::KP_1 => K_KP_END,
            Key::END => K_END,
            Key::KP_4 => K_KP_LEFTARROW,
            Key::LEFT => K_LEFTARROW,
            Key::KP_6 => K_KP_RIGHTARROW,
            Key::RIGHT => K_RIGHTARROW,
            Key::KP_2 => K_KP_DOWNARROW,
            Key::DOWN => K_DOWNARROW,
            Key::KP_8 => K_KP_UPARROW,
            Key::UP => K_UPARROW,
            Key::ESCAPE => K_ESCAPE,
            Key::KP_ENTER => K_KP_ENTER,
            Key::ENTER => K_ENTER,
            Key::TAB => K_TAB,
            Key::F1 => K_F1,
            Key::F2 => K_F1 + 1,
            Key::F3 => K_F1 + 2,
            Key::F4 => K_F1 + 3,
            Key::F5 => K_F1 + 4,
            Key::F6 => K_F1 + 5,
            Key::F7 => K_F1 + 6,
            Key::F8 => K_F1 + 7,
            Key::F9 => K_F1 + 8,
            Key::F10 => K_F1 + 9,
            Key::F11 => K_F1 + 10,
            Key::F12 => K_F1 + 11,
            Key::F13 => K_F1 + 12,
            Key::F14 => K_F1 + 13,
            Key::F15 => K_F1 + 14,
            Key::BACKSPACE => K_BACKSPACE,
            Key::KP_PERIOD => K_KP_DEL,
            Key::DELETE => K_DEL,
            Key::PAUSE => K_PAUSE,
            Key::SHIFT => K_SHIFT,
            Key::CTRL => K_CTRL,
            Key::META => K_SUPER,
            Key::ALT => K_ALT,
            Key::KP_5 => K_KP_5,
            Key::INSERT => K_INS,
            Key::KP_0 => K_KP_INS,
            Key::KP_MULTIPLY => K_KP_STAR,
            Key::KP_ADD => K_KP_PLUS,
            Key::KP_SUBTRACT => K_KP_MINUS,
            Key::KP_DIVIDE => K_KP_SLASH,
            Key::HELP => K_HELP,
            Key::PRINT => K_PRINT,
            Key::SYSREQ => K_SYSREQ,
            Key::MENU => K_MENU,
            Key::SCROLLLOCK => K_SCROLLOCK,
            Key::NUMLOCK => K_KP_NUMLOCK,
            Key::CAPSLOCK => K_CAPSLOCK,
            _ => 0,
        }
    };
    // IN_IsConsoleKey: console keys can't be bound or generate characters
    if key == b'~' as i32 || key == b'`' as i32 {
        return K_CONSOLE;
    }
    key
}

/// Godot mouse buttons as sdl_input.c numbers them (MOUSE1..5, the wheel).
pub fn keynum_from_godot_mouse(b: MouseButton) -> i32 {
    match b {
        MouseButton::LEFT => K_MOUSE1,
        MouseButton::RIGHT => K_MOUSE2,
        MouseButton::MIDDLE => K_MOUSE3,
        MouseButton::XBUTTON1 => K_MOUSE4,
        MouseButton::XBUTTON2 => K_MOUSE5,
        MouseButton::WHEEL_UP => K_MWHEELUP,
        MouseButton::WHEEL_DOWN => K_MWHEELDOWN,
        _ => 0,
    }
}

/// What a key event asks of the host besides the command text.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyAction {
    /// `Con_ToggleConsole_f` (the console key or Shift+Escape).
    ToggleConsole,
    /// Escape: `UI_SET_ACTIVE_MENU( UIMENU_INGAME )` / the UI's own Escape.
    Escape,
    /// Command text for `Cbuf_AddText` (`CL_ParseBinding`).
    Text(Vec<String>),
}

/// `qkey_t keys[MAX_KEYS]` + `anykeydown`.
#[derive(Debug)]
pub struct Keys {
    binding: Vec<String>,
    down: Vec<bool>,
    repeats: Vec<i32>,
    pub anykeydown: i32,
    /// `cvar_modifiedFlags |= CVAR_ARCHIVE` from `Key_SetBinding`
    pub modified: bool,
}

impl Default for Keys {
    fn default() -> Self {
        Keys { binding: vec![String::new(); MAX_KEYS as usize], down: vec![false; MAX_KEYS as usize], repeats: vec![0; MAX_KEYS as usize], anykeydown: 0, modified: false }
    }
}

impl Keys {
    /// `Key_IsDown` (`trap_Key_IsDown`: no caller in this port's cgame / ui).
    #[allow(dead_code)]
    pub fn is_down(&self, k: i32) -> bool {
        (0..MAX_KEYS).contains(&k) && self.down[k as usize]
    }

    /// `Key_SetBinding`.
    pub fn set_binding(&mut self, keynum: i32, binding: &str) {
        if keynum < 0 || keynum >= MAX_KEYS {
            return;
        }
        self.binding[keynum as usize] = binding.to_string();
        // consider this like modifying an archived cvar
        self.modified = true;
    }

    /// `Key_GetBinding` / `Key_GetBindingBuf` ("" when unbound).
    pub fn get_binding(&self, keynum: i32) -> &str {
        if keynum < 0 || keynum >= MAX_KEYS {
            return "";
        }
        &self.binding[keynum as usize]
    }

    /// The whole table (cgame / ui `trap_Key_GetBindingBuf` loops).
    pub fn bindings(&self) -> &[String] {
        &self.binding
    }

    /// `Key_GetKey`: the first key bound to exactly this command (-1)
    /// (`trap_Key_GetKey`: no caller in this port's cgame / ui).
    #[allow(dead_code)]
    pub fn get_key(&self, binding: &str) -> i32 {
        self.binding.iter().position(|b| !b.is_empty() && b.eq_ignore_ascii_case(binding)).map(|i| i as i32).unwrap_or(-1)
    }

    /// `Key_Unbind_f`.
    pub fn unbind_f(&mut self, argv: &[String]) -> String {
        if argv.len() != 2 {
            return "unbind <key> : remove commands from a key\n".into();
        }
        let b = key_string_to_keynum(&argv[1]);
        if b == -1 {
            return format!("\"{}\" isn't a valid key\n", argv[1]);
        }
        self.set_binding(b, "");
        String::new()
    }

    /// `Key_Unbindall_f`.
    pub fn unbindall_f(&mut self) {
        for i in 0..MAX_KEYS {
            if !self.binding[i as usize].is_empty() {
                self.set_binding(i, "");
            }
        }
    }

    /// `Key_Bind_f`: `bind <key> [command]` (the rest of argv joined by spaces).
    pub fn bind_f(&mut self, argv: &[String]) -> String {
        let c = argv.len();
        if c < 2 {
            return "bind <key> [command] : attach a command to a key\n".into();
        }
        let b = key_string_to_keynum(&argv[1]);
        if b == -1 {
            return format!("\"{}\" isn't a valid key\n", argv[1]);
        }
        if c == 2 {
            let cur = self.get_binding(b);
            return if !cur.is_empty() { format!("\"{}\" = \"{}\"\n", key_keynum_to_string(b), cur) } else { format!("\"{}\" is not bound\n", key_keynum_to_string(b)) };
        }
        // copy the rest of the command line
        let cmd = argv[2..].join(" ");
        self.set_binding(b, &cmd);
        String::new()
    }

    /// `Key_WriteBindings`: "unbindall" then `bind KEY "command"` lines.
    pub fn write_bindings(&self) -> String {
        let mut s = String::from("unbindall\n");
        for (i, b) in self.binding.iter().enumerate() {
            if !b.is_empty() {
                s.push_str(&format!("bind {} \"{}\"\n", key_keynum_to_string(i as i32), b));
            }
        }
        s
    }

    /// `Key_Bindlist_f`.
    pub fn bindlist_f(&self) -> String {
        let mut s = String::new();
        for (i, b) in self.binding.iter().enumerate() {
            if !b.is_empty() {
                s.push_str(&format!("{} \"{}\"\n", key_keynum_to_string(i as i32), b));
            }
        }
        s
    }

    /// `CL_ParseBinding`: the command text a key press / release adds.
    /// `+cmd`s get the key number and time appended (`+cmd key time` on
    /// press, `-cmd key time` on release); other commands only run on press.
    /// With a key catcher, presses run only toggleconsole / togglemenu
    /// (`CL_BindUICommand`) while releases of `+cmd`s still run.
    pub fn parse_binding(&self, key: i32, down: bool, time: u32, catcher: i32) -> Vec<String> {
        let mut out = Vec::new();
        let binding = self.get_binding(key);
        if binding.is_empty() {
            return out;
        }
        // run all bind commands if console, ui, etc aren't reading keys
        let all_commands = catcher == 0;
        // allow button up commands if in game even if key catcher is set
        let allow_up_cmds = true;
        for part in binding.split(';') {
            let p = part.trim_start();
            if let Some(rest) = p.strip_prefix('+') {
                // button commands add keynum and time as parameters so that
                // multiple sources can be discriminated and subframe corrected
                if all_commands || (allow_up_cmds && !down) {
                    out.push(format!("{}{} {} {}\n", if down { '+' } else { '-' }, rest, key, time));
                }
            } else if down {
                // normal commands only execute on key press
                if all_commands || bind_ui_command(p, catcher) {
                    out.push(format!("{p}\n"));
                }
            }
        }
        out
    }

    /// `CL_KeyDownEvent` (the parts before the key catchers' handlers).
    pub fn key_down_event(&mut self, key: i32, time: u32, catcher: i32) -> KeyAction {
        if !(0..MAX_KEYS).contains(&key) {
            return KeyAction::Text(Vec::new());
        }
        self.down[key as usize] = true;
        self.repeats[key as usize] += 1;
        if self.repeats[key as usize] == 1 {
            self.anykeydown += 1;
        }
        // console key is hardcoded, so the user can never unbind it
        if key == K_CONSOLE || (self.down[K_SHIFT as usize] && key == K_ESCAPE) {
            return KeyAction::ToggleConsole;
        }
        // escape is always handled special
        if key == K_ESCAPE {
            return KeyAction::Escape;
        }
        // send the bound action
        KeyAction::Text(self.parse_binding(key, true, time, catcher))
    }

    /// `CL_KeyUpEvent`: only `+cmd` bindings act on release.
    pub fn key_up_event(&mut self, key: i32, time: u32, catcher: i32) -> Vec<String> {
        if !(0..MAX_KEYS).contains(&key) {
            return Vec::new();
        }
        self.repeats[key as usize] = 0;
        self.down[key as usize] = false;
        self.anykeydown = (self.anykeydown - 1).max(0);
        // don't process key-up events for the console key
        if key == K_CONSOLE || (key == K_ESCAPE && self.down[K_SHIFT as usize]) {
            return Vec::new();
        }
        self.parse_binding(key, false, time, catcher)
    }

    /// `Key_ClearStates`: release every held key (its `-cmd`s run).
    pub fn clear_states(&mut self, catcher: i32) -> Vec<String> {
        self.anykeydown = 0;
        let mut out = Vec::new();
        for i in 0..MAX_KEYS {
            if self.down[i as usize] {
                out.extend(self.key_up_event(i, 0, catcher));
            }
            self.down[i as usize] = false;
            self.repeats[i as usize] = 0;
        }
        out
    }
}

/// `CL_BindUICommand`: commands a binding runs while the UI has the keys.
fn bind_ui_command(cmd: &str, catcher: i32) -> bool {
    if catcher & KEYCATCH_CONSOLE != 0 {
        return false;
    }
    let c = cmd.trim();
    c.eq_ignore_ascii_case("toggleconsole") || c.eq_ignore_ascii_case("togglemenu")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip() {
        assert_eq!(key_string_to_keynum("MOUSE1"), K_MOUSE1);
        assert_eq!(key_string_to_keynum("mwheelup"), K_MWHEELUP);
        assert_eq!(key_string_to_keynum("W"), b'w' as i32);
        assert_eq!(key_string_to_keynum("["), b'[' as i32);
        assert_eq!(key_string_to_keynum("0x41"), 0x41);
        assert_eq!(key_string_to_keynum("F12"), K_F1 + 11);
        assert_eq!(key_string_to_keynum("JOY32"), K_JOY1 + 31);
        assert_eq!(key_string_to_keynum("WORLD_0"), K_WORLD_0);
        assert_eq!(key_string_to_keynum("AUX16"), K_AUX1 + 15);
        assert_eq!(key_string_to_keynum("JOY0"), -1);
        assert_eq!(key_string_to_keynum("nosuchkey"), -1);
        assert_eq!(key_string_to_keynum(""), -1);
        assert_eq!(key_string_to_keynum("SEMICOLON"), b';' as i32);
        assert_eq!(key_keynum_to_string(b'w' as i32), "w");
        assert_eq!(key_keynum_to_string(b';' as i32), "SEMICOLON");
        assert_eq!(key_keynum_to_string(b'"' as i32), "0x22");
        assert_eq!(key_keynum_to_string(K_SPACE), "SPACE");
        assert_eq!(key_keynum_to_string(K_MOUSE2), "MOUSE2");
        // the C hex formatting only covers two digits
        assert_eq!(key_keynum_to_string(K_CONSOLE), "0xl4");
        assert_eq!(key_keynum_to_string(1), "0x01");
        assert_eq!(key_keynum_to_string(-1), "<KEY NOT FOUND>");
        for k in 1..K_CONSOLE {
            let s = key_keynum_to_string(k);
            if !s.starts_with("0x") || k < 256 {
                assert_eq!(key_string_to_keynum(&s), if (b'A' as i32..=b'Z' as i32).contains(&k) { k + 32 } else { k }, "{s}");
            }
        }
    }

    #[test]
    fn godot_keys_map_like_sdl() {
        assert_eq!(keynum_from_godot_key(Key::W), b'w' as i32);
        assert_eq!(keynum_from_godot_key(Key::BRACKETLEFT), b'[' as i32);
        assert_eq!(keynum_from_godot_key(Key::SPACE), K_SPACE);
        assert_eq!(keynum_from_godot_key(Key::CTRL), K_CTRL);
        assert_eq!(keynum_from_godot_key(Key::F11), K_F1 + 10);
        assert_eq!(keynum_from_godot_key(Key::QUOTELEFT), K_CONSOLE);
        assert_eq!(keynum_from_godot_key(Key::ASCIITILDE), K_CONSOLE);
        assert_eq!(keynum_from_godot_key(Key::KP_9), K_KP_PGUP);
        assert_eq!(keynum_from_godot_key(Key::ENTER), K_ENTER);
        assert_eq!(keynum_from_godot_mouse(MouseButton::WHEEL_DOWN), K_MWHEELDOWN);
        assert_eq!(keynum_from_godot_mouse(MouseButton::MIDDLE), K_MOUSE3);
    }

    fn argv(s: &str) -> Vec<String> {
        crate::cmd::tokenize(s)
    }

    #[test]
    fn bind_unbind_bindlist() {
        let mut k = Keys::default();
        assert_eq!(k.bind_f(&argv("bind  b  \"buy ammo\"")), "");
        assert_eq!(k.get_binding(b'b' as i32), "buy ammo");
        assert_eq!(k.bind_f(&argv("bind MOUSE1 +attack")), "");
        assert_eq!(k.bind_f(&argv("bind q say hi there")), "");
        assert_eq!(k.get_binding(b'q' as i32), "say hi there");
        assert_eq!(k.bind_f(&argv("bind b")), "\"b\" = \"buy ammo\"\n");
        assert_eq!(k.bind_f(&argv("bind nokey x")), "\"nokey\" isn't a valid key\n");
        assert_eq!(k.get_key("+ATTACK"), K_MOUSE1);
        assert_eq!(k.bindlist_f(), "b \"buy ammo\"\nq \"say hi there\"\nMOUSE1 \"+attack\"\n");
        assert_eq!(k.write_bindings(), "unbindall\nbind b \"buy ammo\"\nbind q \"say hi there\"\nbind MOUSE1 \"+attack\"\n");
        k.unbind_f(&argv("unbind q"));
        assert_eq!(k.bind_f(&argv("bind q")), "\"q\" is not bound\n");
        k.unbindall_f();
        assert_eq!(k.bindlist_f(), "");
    }

    #[test]
    fn parse_binding_button_commands_carry_key_and_time() {
        let mut k = Keys::default();
        k.set_binding(K_MOUSE1, "+attack; weapnext");
        assert_eq!(k.key_down_event(K_MOUSE1, 100, 0), KeyAction::Text(vec![format!("+attack {K_MOUSE1} 100\n"), "weapnext\n".into()]));
        assert_eq!(k.key_up_event(K_MOUSE1, 150, 0), vec![format!("-attack {K_MOUSE1} 150\n")]);
        // with the ui catching keys: no press commands, releases still run
        assert_eq!(k.key_down_event(K_MOUSE1, 200, KEYCATCH_UI), KeyAction::Text(vec![]));
        assert_eq!(k.key_up_event(K_MOUSE1, 250, KEYCATCH_UI), vec![format!("-attack {K_MOUSE1} 250\n")]);
        k.set_binding(K_ESCAPE, "togglemenu");
        assert_eq!(k.key_down_event(K_ESCAPE, 0, 0), KeyAction::Escape);
        k.key_down_event(K_SHIFT, 0, 0);
        assert_eq!(k.key_down_event(K_ESCAPE, 0, 0), KeyAction::ToggleConsole);
        k.set_binding(b'`' as i32, "toggleconsole");
        k.set_binding(K_F1, "togglemenu");
        assert_eq!(k.parse_binding(K_F1, true, 0, KEYCATCH_UI), vec!["togglemenu\n".to_string()]);
        assert_eq!(k.parse_binding(K_F1, true, 0, KEYCATCH_CONSOLE), Vec::<String>::new());
        // Key_ClearStates releases what is held
        k.key_down_event(K_MOUSE1, 300, 0);
        let up = k.clear_states(0);
        assert!(up.contains(&format!("-attack {K_MOUSE1} 0\n")));
        assert_eq!(k.anykeydown, 0);
    }
}
