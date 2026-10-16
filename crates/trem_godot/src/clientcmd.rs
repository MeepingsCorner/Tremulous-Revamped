//! The client's command interpreter: qcommon `Cmd_ExecuteString` over the
//! command buffer (`Cbuf_Execute`) with the commands the client registers
//! (cmd.c `Cmd_Exec_f`, `Cmd_Vstr_f`, `Cmd_Echo_f`, `Cmd_Wait_f`; cvar.c;
//! cl_keys.c binds; cl_input.c kbuttons; cl_console.c `Con_ToggleConsole_f`
//! / `Con_ToggleMenu_f`; cl_main.c `CL_ForwardToServer_f`), then
//! `Cvar_Command`, `CL_GameCommand` (cgame `CG_ConsoleCommand`),
//! `SV_GameCommand` (the listen server's console), `UI_ConsoleCommand`
//! (ui_atoms.c `UI_MessageMode_f`, `UI_Menu_f`, `UI_CloseMenus_f`,
//! `UI_Me_f`) and finally `CL_ForwardCommandToServer`. Startup runs
//! `Com_ExecuteCfg` (default.cfg, autogen.cfg, autoexec.cfg) and the
//! config is written back like `Com_WriteConfiguration`.
//!
//! What lives on the Godot side (menus, console, scoreboard, port debug
//! helpers) is reached through `ClientHost::host`.

use crate::cgcmds::{self, WeaponSelect};
use crate::cmd::{args_from, tokenize, Cbuf, Cvars, CVAR_ARCHIVE};
use crate::input::{self, ClInput, FrameIn, InputCvars};
use crate::keys::{KeyAction, Keys};
use trem_sim::kernel::types::{PlayerState, UserCmd};

/// qcommon.h Q3CONFIG_CFG (legacy): the archived binds and cvars.
pub const CONFIG_CFG: &str = "autogen.cfg";

/// The port's additions, run after default.cfg (`exec port_default.cfg`)
/// on a first start (afterwards autogen.cfg holds every bind):
///
/// * x and z: legacy has no `boost` / `+zoom` commands (the 1.1.0
///   default.cfg binds them); legacy's ui_shared.c `g_bindings` defaults
///   (`Controls_SetDefaults`) put sprint (+button8) on x and dodge
///   (+button6) on z, so those two keys get them;
/// * keys default.cfg leaves unbound: port debug helpers (bots, restart,
///   map menu, help), the third-person view (cg_thirdPerson) and the
///   VoIP microphone toggle (k, `voip_toggle`).
pub const PORT_DEFAULT_CFG: &str = "\
// port additions (not in the original default.cfg)
bind  x             +button8
bind  z             +button6
bind  v             \"toggle cg_thirdPerson\"
bind  F3            \"port_addbot aliens\"
bind  F4            \"port_addbot humans\"
bind  F5            \"port_removebot aliens\"
bind  F6            \"port_removebot humans\"
bind  F7            port_restart
bind  F8            port_maps
bind  F9            port_help
bind  F10           port_gfx
bind  g             port_vision
bind  k             voip_toggle
";

/// Binds added to PORT_DEFAULT_CFG after its first release, with the
/// `port_bindsVersion` that brought each: a config written before then
/// (autogen.cfg starts with "unbindall") gets them once, on keys it left
/// unbound.
const PORT_LATER_BINDS: &[(i32, &str, &str)] = &[(1, "k", "voip_toggle")];

/// The 1.1.0 data's default.cfg (data-1.1.0.pk3), verbatim: used when no
/// game data is on the search path, so the sheet arena gets the same keys.
pub const BUILTIN_DEFAULT_CFG: &str = "\
//
// KEY BINDINGS
//

unbindall

bind  -             sizedown
bind  _             sizedown
bind  =             sizeup
bind  +             sizeup

//
// WEAPONS/UPGRADES
//
bind  MOUSE1        +attack
bind  MOUSE2        +button5

bind  [             weapprev
bind  ]             weapnext
bind  mwheelup      weapprev
bind  mwheeldown    weapnext
bind  ENTER         +button2
bind  MOUSE3        +button2


//
// CHARACTER CONTROLS
//

bind  CTRL          +attack

bind  ALT           +strafe
bind  SHIFT         +speed
bind  x             boost

bind  DEL           +lookdown
bind  PGDN          +lookup
bind  END           centerview

bind  c             +movedown
bind  SPACE         +moveup


bind  UPARROW       +forward
bind  DOWNARROW     +back
bind  LEFTARROW     +left
bind  RIGHTARROW    +right
bind  w             +forward
bind  a             +moveleft
bind  s             +back
bind  d             +moveright

//
// MISC
//

bind  t             messagemode
bind  y             messagemode2
bind  u             messagemode3
bind  i             messagemode4

bind  z             +zoom
bind  r             reload
bind  b             \"buy ammo\"
bind  m             \"itemact medkit\"
bind  q             +button7
bind  e             deconstruct
bind  f             +button3
bind  h             scoresUp
bind  n             scoresDown

bind  \\             +mlook

//
// CLIENT ENVIRONMENT COMMANDS
//

bind  PAUSE         pause
bind  ESCAPE        togglemenu
bind  ~             toggleconsole
bind  `             toggleconsole

bind  TAB           +scores

bind  F1            \"vote yes\"
bind  F2            \"vote no\"


//
//  DEVELOPER KEYS
//
bind  F11           screenshotJPEG
";

/// Commands the Godot side runs (`ClientHost::host`): cl_console.c,
/// cl_main.c / common.c engine commands, the cgame scoreboard commands,
/// ui_atoms.c's menu commands and the port's debug helpers.
const HOST_COMMANDS: &[&str] = &[
    // cl_console.c
    "toggleconsole",
    "togglemenu",
    "clear",
    // common.c / cl_main.c
    "quit",
    "disconnect",
    // renderer
    "screenshot",
    "screenshotJPEG",
    // cl_console.c Con_Dump_f
    "condump",
    // port debug helpers (PORT_DEFAULT_CFG)
    "port_addbot",
    "port_removebot",
    "port_restart",
    "port_maps",
    "port_help",
    // the graphics modes (gfx.rs), the FMOD sound path (fmodaudio.rs)
    "port_gfx",
    // the enhanced flashlight (humans) / night vision (aliens)
    "port_vision",
    "s_fmodinfo",
];

/// ui_atoms.c `commands[]` handled by the menus (`UI_ConsoleCommand`).
const UI_HOST_COMMANDS: &[&str] = &["closemenus", "menu", "messagemode", "messagemode2"];

/// What the client needs from the rest of the program.
pub trait ClientHost {
    /// cg.snap->ps (None before the first snapshot).
    fn snap_ps(&self) -> Option<PlayerState>;
    /// cg.time
    fn cg_time(&self) -> i32;
    /// `FS_ReadFile` (home path, then the pk3s).
    fn read_file(&self, path: &str) -> Option<String>;
    /// `SV_GameCommand` would take this command word (listen server console).
    fn is_server_command(&self, word: &str) -> bool;
    /// Run a line on the server console; its output.
    fn server_console(&mut self, line: &str) -> String;
    /// A `set` of a cvar the server owns: true when it took it.
    fn server_set(&mut self, name: &str, value: &str) -> bool;
    /// `CL_ForwardCommandToServer` (a client command); error text or "".
    fn forward_to_server(&mut self, line: &str) -> String;
    /// A command for the Godot side (menus, console, port helpers).
    fn host(&mut self, argv: &[String]);
    /// `CG_Viewpos_f` text.
    fn viewpos(&self) -> String;
    /// `CG_ClientList_f` text.
    fn client_list(&self) -> String;
    /// The engine's network commands (connect, reconnect, localservers,
    /// globalservers, ping, serverstatus, rcon, heartbeat, serverinfo,
    /// systeminfo, killserver; dec033): Some(output) when it ran one.
    fn engine_command(&mut self, _argv: &[String]) -> Option<String> {
        None
    }
}

/// The client: keys, cvars, command buffer, input and cg.weaponSelect.
#[derive(Debug)]
pub struct Client {
    pub keys: Keys,
    pub cvars: Cvars,
    pub cbuf: Cbuf,
    pub input: ClInput,
    pub wsel: WeaponSelect,
    /// `Key_GetCatcher()`
    pub catcher: i32,
    /// Com_Printf text since the last `take_output`
    out: String,
    /// `in_restart` ran (`IN_Restart`: the joystick is opened again)
    pub in_restart: bool,
    /// the port's microphone toggle (`voip_toggle`): on until toggled off
    /// or until CL_CaptureVoip refuses it (the host turns it off then)
    pub voip_toggle: bool,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// `CL_Init` / `CL_InitInput` / `CG_RegisterCvars`: the client cvars.
    pub fn new() -> Self {
        let mut c = Client { keys: Keys::default(), cvars: Cvars::default(), cbuf: Cbuf::default(), input: ClInput::default(), wsel: WeaponSelect::default(), catcher: 0, out: String::new(), in_restart: false, voip_toggle: false };
        input::register_cvars(&mut c.cvars);
        crate::userinfo::register_cvars(&mut c.cvars);
        crate::joystick::register_cvars(&mut c.cvars);
        c.cvars.get("cg_viewsize", "100", CVAR_ARCHIVE);
        // CVAR_CHEAT in cg_main.c; the port lets the player toggle it
        c.cvars.get("cg_thirdPerson", "0", 0);
        // cl_console.c Con_Init; cg_main.c's debugging cvars (CVAR_CHEAT)
        c.cvars.get("scr_conspeed", "3", 0);
        // dec084: 0 = auto (whole-number scale by height), 1 = the 1.1 8x16 console, 2-4 fixed
        c.cvars.get("con_scale", "0", CVAR_ARCHIVE);
        for n in ["cg_debugevents", "cg_drawBBOX", "cg_drawSurfNormal"] {
            c.cvars.get(n, "0", 0);
        }
        c.cvars.modified_flags = 0;
        c
    }

    /// The Com_Printf text so far.
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    fn print(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// `Com_ExecuteCfg` (with the port's additions after default.cfg),
    /// then `cvar_modifiedFlags &= ~CVAR_ARCHIVE` like Com_Init.
    pub fn startup(&mut self, host: &mut dyn ClientHost) {
        self.cvars.get("port_bindsVersion", "0", CVAR_ARCHIVE);
        for f in ["default.cfg", "port_default.cfg", CONFIG_CFG, "autoexec.cfg"] {
            if f == CONFIG_CFG {
                // port_default.cfg had every port bind; a config written
                // before port_bindsVersion existed sets it back to 0
                let latest = PORT_LATER_BINDS.iter().map(|b| b.0).max().unwrap_or(0);
                self.cvars.set("port_bindsVersion", &latest.to_string());
                if host.read_file(CONFIG_CFG).is_some_and(|t| !t.contains("port_bindsVersion")) {
                    self.cvars.set("port_bindsVersion", "0");
                }
            }
            self.cbuf.add_text(&format!("exec {f}\n"));
            self.execute(host);
        }
        // the later port binds an older config lacks, on keys it left free
        let have = self.cvars.integer("port_bindsVersion");
        let mut migrated = false;
        for &(version, key, cmd) in PORT_LATER_BINDS.iter().filter(|b| b.0 > have) {
            let k = crate::keys::key_string_to_keynum(key);
            if k >= 0 && self.keys.get_binding(k).is_empty() {
                self.keys.set_binding(k, cmd);
            }
            self.cvars.set("port_bindsVersion", &version.to_string());
            migrated = true;
        }
        self.keys.modified = migrated;
        self.cvars.modified_flags &= !CVAR_ARCHIVE;
    }

    /// `Com_WriteConfiguration`'s check: binds or archived cvars changed.
    pub fn config_modified(&self) -> bool {
        self.keys.modified || self.cvars.modified_flags & CVAR_ARCHIVE != 0
    }

    /// `Com_WriteConfigToFile`'s text (and the modified flags cleared).
    pub fn write_config(&mut self) -> String {
        self.keys.modified = false;
        self.cvars.modified_flags &= !CVAR_ARCHIVE;
        format!("// generated by tremulous, do not modify\n{}{}", self.keys.write_bindings(), self.cvars.write_variables())
    }

    /// `Cbuf_AddText`.
    pub fn add_text(&mut self, t: &str) {
        self.cbuf.add_text(t);
    }

    /// `Cbuf_Execute`.
    pub fn execute(&mut self, host: &mut dyn ClientHost) {
        let mut comments = (false, false);
        // (a vstr that runs itself would never end)
        for _ in 0..4096 {
            let Some(line) = self.cbuf.next_line(&mut comments) else { return };
            self.execute_string(&line, host);
        }
        self.print("Cbuf_Execute: too many commands, buffer cleared\n");
        self.cbuf = Cbuf::default();
    }

    /// `Key_SetCatcher`: a changed catcher clears every key (`Key_ClearStates`).
    pub fn set_catcher(&mut self, catcher: i32, host: &mut dyn ClientHost) {
        if catcher != self.catcher {
            for t in self.keys.clear_states(catcher) {
                self.cbuf.add_text(&t);
            }
            self.execute(host);
        }
        self.catcher = catcher;
    }

    /// `CL_KeyEvent` for a key number; the bound commands run at once.
    pub fn key_event(&mut self, key: i32, down: bool, time: u32, host: &mut dyn ClientHost) {
        let catcher = self.catcher;
        if down {
            match self.keys.key_down_event(key, time, catcher) {
                KeyAction::ToggleConsole => {
                    // Con_ToggleConsole_f + Key_ClearStates
                    for t in self.keys.clear_states(catcher) {
                        self.cbuf.add_text(&t);
                    }
                    self.execute(host);
                    host.host(&["toggleconsole".to_string()]);
                    return;
                }
                KeyAction::Escape => {
                    host.host(&["escape".to_string()]);
                    return;
                }
                KeyAction::Text(t) => {
                    for l in t {
                        self.cbuf.add_text(&l);
                    }
                }
            }
        } else {
            for l in self.keys.key_up_event(key, time, catcher) {
                self.cbuf.add_text(&l);
            }
        }
        self.execute(host);
    }

    /// `CL_MouseEvent`: view movement only without a key catcher.
    pub fn mouse_event(&mut self, dx: f32, dy: f32) {
        if self.catcher == 0 {
            self.input.mouse_event(dx, dy);
        }
    }

    /// `CL_CreateNewCommands` + `CL_CreateCmd` for this frame.
    pub fn create_cmd(&mut self, com_frame_time: u32, cgame_sensitivity: f32, server_time: i32) -> UserCmd {
        self.input.new_frame(com_frame_time);
        let cv = InputCvars::from_cvars(&self.cvars);
        let fr = FrameIn { catcher: self.catcher, anykeydown: self.keys.anykeydown > 0, weapon: self.wsel.select.clamp(0, 255) as u8, cgame_sensitivity, server_time };
        let fm = self.input.frame_msec;
        self.input.create_cmd(&cv, &fr, fm)
    }

    /// The client's registered command names (`cmd_functions`, what
    /// `Cmd_CommandCompletion` walks): cmd.c, cvar.c, cl_keys.c, cl_input.c
    /// (`CL_InitInput`), cl_console.c, cl_main.c, the ui's and the cgame's
    /// (`CG_InitConsoleCommands`, with the game commands it adds so they
    /// complete before being forwarded).
    pub fn command_names(&self) -> Vec<String> {
        let mut out: Vec<String> = [
            "exec", "execq", "vstr", "echo", "wait", "print", "toggle", "set", "seta", "setu", "sets", "reset", "cvarlist", "bind", "unbind", "unbindall", "bindlist", "writeconfig", "cmd", "rcon", "in_restart", "+mlook", "-mlook", "centerview", "me",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for b in ["moveup", "movedown", "left", "right", "forward", "back", "lookup", "lookdown", "strafe", "moveleft", "moveright", "speed", "attack"] {
            out.push(format!("+{b}"));
            out.push(format!("-{b}"));
        }
        for i in 0..15 {
            out.push(format!("+button{i}"));
            out.push(format!("-button{i}"));
        }
        // wt-b: cl_input.c +/-voiprecord, cl_main.c video / stopvideo
        out.extend(["+voiprecord", "-voiprecord", "voip_toggle", "voip_refreshdevices", "video", "stopvideo"].map(String::from));
        out.extend(HOST_COMMANDS.iter().chain(UI_HOST_COMMANDS).chain(cgcmds::CG_COMMANDS).chain(cgcmds::CG_FORWARDED_COMMANDS).map(|s| s.to_string()));
        out
    }

    /// `Cmd_ExecuteString`.
    pub fn execute_string(&mut self, text: &str, host: &mut dyn ClientHost) {
        let argv = tokenize(text);
        let Some(a0) = argv.first() else { return };
        let c = a0.to_ascii_lowercase();
        let arg = |n: usize| argv.get(n).cloned().unwrap_or_default();
        // registered command functions
        match c.as_str() {
            "exec" | "execq" => return self.exec_f(&argv, host),
            "vstr" => {
                // Cmd_Vstr_f
                if argv.len() != 2 {
                    return self.print("vstr <variablename> : execute a variable command\n");
                }
                let v = self.cvars.string(&argv[1]);
                return self.cbuf.insert_text(&v);
            }
            "echo" => return self.print(&format!("{}\n", args_from(&argv, 1))),
            "wait" => {
                // Cmd_Wait_f
                let n = if argv.len() == 2 { trem_sim::kernel::spawn::c_atoi(&argv[1]) } else { 1 };
                self.cbuf.wait = if n < 0 { 1 } else { n };
                return;
            }
            "print" => {
                let s = match self.cvars.find(&arg(1)) {
                    Some(v) if argv.len() == 2 => Cvars::print(v),
                    None if argv.len() == 2 => format!("Cvar {} does not exist.\n", arg(1)),
                    _ => "usage: print <variable>\n".into(),
                };
                return self.print(&s);
            }
            "toggle" => {
                let s = self.cvars.toggle_f(&argv);
                return self.print(&s);
            }
            "set" | "seta" | "setu" | "sets" => {
                // a cvar of the server (g_*, timelimit, cg view cvars, ...)
                // goes to the server console, everything else is the client's
                if argv.len() >= 3 && self.cvars.find(&argv[1]).is_none() && host.server_set(&argv[1], &args_from(&argv, 2)) {
                    return;
                }
                let s = self.cvars.set_f(&argv);
                return self.print(&s);
            }
            "reset" => {
                let s = self.cvars.reset_f(&argv);
                return self.print(&s);
            }
            "cvarlist" => {
                let s = self.cvars.list_f();
                return self.print(&s);
            }
            "bind" => {
                let s = self.keys.bind_f(&argv);
                return self.print(&s);
            }
            "unbind" => {
                let s = self.keys.unbind_f(&argv);
                return self.print(&s);
            }
            "unbindall" => return self.keys.unbindall_f(),
            "bindlist" => {
                let s = self.keys.bindlist_f();
                return self.print(&s);
            }
            "writeconfig" => {
                // Com_WriteConfig_f: the host writes on the next frame
                self.keys.modified = true;
                return;
            }
            "cmd" => {
                // CL_ForwardToServer_f
                if argv.len() > 1 {
                    let e = host.forward_to_server(&args_from(&argv, 1));
                    self.print_err(&e);
                }
                return;
            }
            "rcon" => {
                // CL_Rcon_f over the network when connected or rconAddress is set
                if let Some(out) = host.engine_command(&argv) {
                    return self.print(&out);
                }
                // else this is the listen server
                let r = host.server_console(&args_from(&argv, 1));
                self.print_err(&r);
                return;
            }
            "in_restart" => {
                self.in_restart = true;
                return self.input.clear();
            }
            _ => {}
        }
        // wt-b VoIP: cl_input.c IN_VoipRecordDown / IN_VoipRecordUp
        if c == "+voiprecord" || c == "-voiprecord" {
            self.cvars.set("cl_voipSend", trem_sim::kernel::voip::voip_record(c.starts_with('+')));
            return;
        }
        // the port's microphone toggle (k): +voiprecord that stays down
        // (its own state: CL_CaptureVoip turns cl_voipSend back to 0 when
        // it will not capture, so the cvar cannot say what the key did)
        if c == "voip_toggle" {
            let on = !self.voip_toggle;
            self.voip_toggle = on;
            self.cvars.set("cl_voipSend", trem_sim::kernel::voip::voip_record(on));
            return self.print(if on { "VoIP: microphone on\n" } else { "VoIP: microphone off\n" });
        }
        // cl_input.c commands
        let delta_pitch = host.snap_ps().map(|p| p.delta_angles[0]).unwrap_or(0);
        let freelook = self.cvars.integer("cl_freelook");
        if self.input.command(&argv, freelook, delta_pitch) {
            return;
        }
        if HOST_COMMANDS.iter().any(|h| h.eq_ignore_ascii_case(&c)) {
            return host.host(&argv);
        }
        // the engine's network commands
        if let Some(out) = host.engine_command(&argv) {
            return self.print(&out);
        }
        // check cvars
        if let Some(s) = self.cvars.command(&argv) {
            return self.print(&s);
        }
        // check client game commands
        if cgcmds::is_cg_command(&c) {
            return self.cg_console_command(&c, &argv, host);
        }
        // check server game commands
        if host.is_server_command(&c) {
            let r = host.server_console(text.trim());
            return self.print_err(&r);
        }
        // check ui commands
        if UI_HOST_COMMANDS.iter().any(|h| h.eq_ignore_ascii_case(&c)) {
            return host.host(&argv);
        }
        if c == "me" {
            // UI_Me_f
            return self.cbuf.add_text(&format!("say \"/me {}\"", args_from(&argv, 1)));
        }
        // CL_ForwardCommandToServer: ignore key up commands
        if c.starts_with('-') {
            return;
        }
        if c.starts_with('+') {
            return self.print(&format!("Unknown command \"{a0}^7\"\n"));
        }
        let e = host.forward_to_server(text.trim());
        self.print_err(&e);
    }

    fn print_err(&mut self, e: &str) {
        if !e.is_empty() {
            self.print(&format!("{}\n", e.trim_end()));
        }
    }

    /// `Cmd_Exec_f` (".cfg" appended when the name has no extension).
    fn exec_f(&mut self, argv: &[String], host: &mut dyn ClientHost) {
        let quiet = argv[0].eq_ignore_ascii_case("execq");
        if argv.len() != 2 {
            return self.print(&format!("exec{} <filename> : execute a script file{}\n", if quiet { "q" } else { "" }, if quiet { " without notification" } else { "" }));
        }
        let mut name = argv[1].clone();
        // COM_DefaultExtension
        if !name.rsplit(['/', '\\']).next().unwrap_or("").contains('.') {
            name.push_str(".cfg");
        }
        match host.read_file(&name) {
            None => self.print(&format!("couldn't exec {name}\n")),
            Some(t) => {
                if !quiet {
                    self.print(&format!("execing {name}\n"));
                }
                self.cbuf.insert_text(&t);
            }
        }
    }

    /// `CG_ConsoleCommand`.
    fn cg_console_command(&mut self, c: &str, argv: &[String], host: &mut dyn ClientHost) {
        let time = host.cg_time();
        match c {
            "weapnext" | "weapprev" => {
                // !cg.snap: nothing
                let Some(ps) = host.snap_ps() else { return };
                let follow = if c == "weapnext" { self.wsel.next_weapon(&ps, time) } else { self.wsel.prev_weapon(&ps, time) };
                if let Some(cmd) = follow {
                    // trap_SendClientCommand
                    let e = host.forward_to_server(cmd);
                    self.print_err(&e);
                }
            }
            "weapon" => {
                if let Some(ps) = host.snap_ps() {
                    self.wsel.weapon_f(&ps, time, argv.get(1).map(|s| s.as_str()).unwrap_or(""));
                }
            }
            "sizeup" | "sizedown" => {
                let v = cgcmds::size_step(self.cvars.integer("cg_viewsize"), c == "sizeup");
                self.cvars.set("cg_viewsize", &v.to_string());
            }
            "viewpos" => {
                let s = host.viewpos();
                self.print(&s);
            }
            "clientlist" => {
                let s = host.client_list();
                self.print(&s);
            }
            "ui_menu" => {
                // CG_UIMenu_f: "menu <name>"
                self.cbuf.insert_text(&format!("menu {}", argv.get(1).map(|s| s.as_str()).unwrap_or("")));
            }
            // +scores / -scores / scoresUp / scoresDown: the scoreboard menu
            _ => host.host(argv),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::*;
    use trem_sim::gen::enums::{Stat, Team};
    use trem_sim::gen::weapons::WeaponId;

    #[derive(Default)]
    struct Host {
        files: Vec<(String, String)>,
        forwarded: Vec<String>,
        hosted: Vec<String>,
        server: Vec<String>,
        ps: Option<PlayerState>,
    }

    impl ClientHost for Host {
        fn snap_ps(&self) -> Option<PlayerState> {
            self.ps
        }
        fn cg_time(&self) -> i32 {
            1000
        }
        fn read_file(&self, path: &str) -> Option<String> {
            if path.eq_ignore_ascii_case("default.cfg") {
                return Some(BUILTIN_DEFAULT_CFG.into());
            }
            if path.eq_ignore_ascii_case("port_default.cfg") {
                return Some(PORT_DEFAULT_CFG.into());
            }
            self.files.iter().find(|(p, _)| p.eq_ignore_ascii_case(path)).map(|(_, t)| t.clone())
        }
        fn is_server_command(&self, word: &str) -> bool {
            word.starts_with("g_") || word == "status"
        }
        fn server_console(&mut self, line: &str) -> String {
            self.server.push(line.to_string());
            String::new()
        }
        fn server_set(&mut self, name: &str, value: &str) -> bool {
            if name.starts_with("g_") {
                self.server.push(format!("set {name} {value}"));
                return true;
            }
            false
        }
        fn forward_to_server(&mut self, line: &str) -> String {
            self.forwarded.push(line.to_string());
            String::new()
        }
        fn host(&mut self, argv: &[String]) {
            self.hosted.push(argv.join(" "));
        }
        fn viewpos(&self) -> String {
            "(1 2 3) : 90\n".into()
        }
        fn client_list(&self) -> String {
            String::new()
        }
    }

    fn started() -> (Client, Host) {
        let mut c = Client::new();
        let mut h = Host::default();
        c.startup(&mut h);
        let out = c.take_output();
        assert_eq!(out, "execing default.cfg\nexecing port_default.cfg\ncouldn't exec autogen.cfg\ncouldn't exec autoexec.cfg\n");
        (c, h)
    }

    #[test]
    fn startup_binds_default_cfg_and_port_keys() {
        let (c, h) = started();
        let k = |s: &str| c.keys.get_binding(key_string_to_keynum(s)).to_string();
        assert_eq!(k("MOUSE1"), "+attack");
        assert_eq!(k("MOUSE2"), "+button5");
        assert_eq!(k("w"), "+forward");
        assert_eq!(k("b"), "buy ammo");
        assert_eq!(k("\\"), "+mlook");
        assert_eq!(k("TAB"), "+scores");
        assert_eq!(k("x"), "+button8");
        assert_eq!(k("z"), "+button6");
        assert_eq!(k("F1"), "vote yes");
        assert_eq!(k("F7"), "port_restart");
        assert_eq!(k("g"), "port_vision");
        assert!(!c.config_modified(), "Com_Init clears the archive flag");
        assert!(h.forwarded.is_empty());
    }

    #[test]
    fn keys_run_binds_buttons_and_commands() {
        let (mut c, mut h) = started();
        c.key_event(b'w' as i32, true, 10, &mut h);
        assert!(c.input.forward.active);
        assert_eq!(c.input.forward.down[0], b'w' as i32);
        c.key_event(K_UPARROW, true, 12, &mut h);
        c.key_event(b'w' as i32, false, 20, &mut h);
        assert!(c.input.forward.active);
        c.key_event(K_UPARROW, false, 30, &mut h);
        assert!(!c.input.forward.active);
        c.key_event(b'r' as i32, true, 40, &mut h);
        c.key_event(b'r' as i32, false, 41, &mut h);
        c.key_event(b'b' as i32, true, 42, &mut h);
        assert_eq!(h.forwarded, vec!["reload", "buy ammo"]);
        c.key_event(b't' as i32, true, 50, &mut h);
        c.key_event(K_TAB, true, 51, &mut h);
        c.key_event(K_TAB, false, 52, &mut h);
        assert_eq!(h.hosted, vec!["messagemode", format!("+scores {K_TAB} 51").as_str(), format!("-scores {K_TAB} 52").as_str()]);
        // u: messagemode3 is not a legacy command: forwarded to the server
        c.key_event(b'u' as i32, true, 53, &mut h);
        assert_eq!(h.forwarded.last().unwrap(), "messagemode3");
        // +button5 (MOUSE2) -> BUTTON_ATTACK2; F (+button3) gesture
        c.key_event(K_MOUSE2, true, 60, &mut h);
        let cmd = c.create_cmd(70, 1.0, 0);
        assert_eq!(cmd.buttons & 32, 32);
        // the ui takes the keys: the held button is released
        c.set_catcher(KEYCATCH_UI, &mut h);
        assert!(!c.input.buttons[5].active);
        c.key_event(b'r' as i32, true, 80, &mut h);
        assert_eq!(h.forwarded.last().unwrap(), "messagemode3", "no commands under a catcher");
        // console key and Escape
        h.hosted.clear();
        c.set_catcher(0, &mut h);
        c.key_event(K_CONSOLE, true, 90, &mut h);
        c.key_event(K_ESCAPE, true, 91, &mut h);
        assert_eq!(h.hosted, vec!["toggleconsole", "escape"]);
        // +zoom does not exist in legacy
        c.key_event(b'f' as i32, true, 92, &mut h);
        assert!(c.input.buttons[3].active);
        c.add_text("+zoom 1 2\n");
        c.execute(&mut h);
        assert_eq!(c.take_output().lines().last().unwrap(), "Unknown command \"+zoom^7\"");
    }

    #[test]
    fn exec_vstr_set_and_server_commands() {
        let (mut c, mut h) = started();
        h.files.push(("my.cfg".into(), "seta sensitivity 2; bind k kill\nset g_cheats 1\nset foo \"say hi; echo x\"".into()));
        c.add_text("exec my\nvstr foo\nstatus\ng_speed 300\n");
        c.execute(&mut h);
        assert_eq!(c.cvars.value("sensitivity"), 2.0);
        assert_eq!(c.keys.get_binding(b'k' as i32), "kill");
        assert!(c.config_modified());
        assert_eq!(h.server, vec!["set g_cheats 1", "status", "g_speed 300"]);
        assert_eq!(h.forwarded, vec!["say hi"]);
        let out = c.take_output();
        assert!(out.starts_with("execing my.cfg\n"), "{out}");
        assert!(out.ends_with("x\n"), "{out}");
        let cfg = c.write_config();
        assert!(cfg.starts_with("// generated by tremulous, do not modify\nunbindall\n"));
        assert!(cfg.contains("bind k \"kill\"\n"));
        assert!(cfg.contains("seta sensitivity \"2\"\n"));
        assert!(!c.config_modified());
        // writing it and executing it gives the same binds back
        let mut c2 = Client::new();
        h.files.push(("autogen.cfg".into(), cfg));
        c2.startup(&mut h);
        assert_eq!(c2.keys.bindlist_f(), c.keys.bindlist_f());
        assert_eq!(c2.cvars.value("sensitivity"), 2.0);
        // toggle and wait
        c.add_text("toggle cg_thirdPerson; wait; toggle cg_thirdPerson\n");
        c.execute(&mut h);
        assert_eq!(c.cvars.integer("cg_thirdPerson"), 1);
        c.execute(&mut h);
        assert_eq!(c.cvars.integer("cg_thirdPerson"), 0);
        c.add_text("sizedown; sizedown\n");
        c.execute(&mut h);
        assert_eq!(c.cvars.integer("cg_viewsize"), 80);
        c.add_text("viewpos\nme waves\n");
        c.execute(&mut h);
        assert_eq!(c.take_output(), "(1 2 3) : 90\n");
        assert_eq!(h.forwarded.last().unwrap(), "say \"/me waves\"");
    }

    #[test]
    fn weapon_commands_drive_the_usercmd_weapon() {
        let (mut c, mut h) = started();
        let mut ps = PlayerState::default();
        ps.stats[Stat::Team as usize] = Team::Humans as i32;
        ps.stats[Stat::Health as usize] = 100;
        ps.stats[Stat::Weapon as usize] = WeaponId::Rifle as i32;
        ps.weapon = WeaponId::Rifle as i32;
        h.ps = Some(ps);
        c.wsel.respawn(&ps, 0);
        c.key_event(K_MWHEELDOWN, true, 1, &mut h);
        c.key_event(K_MWHEELDOWN, false, 1, &mut h);
        // rifle -> (no upgrades) -> blaster
        assert_eq!(c.wsel.select, WeaponId::Blaster as i32);
        assert_eq!(c.wsel.time, 1000);
        c.key_event(K_ENTER, true, 2, &mut h);
        let cmd = c.create_cmd(16, 1.0, 0);
        assert_eq!(cmd.weapon as i32, WeaponId::Blaster as i32);
        assert_eq!(cmd.buttons & 4, 4, "+button2 = BUTTON_USE_HOLDABLE");
        c.add_text("weapon 10\n");
        c.execute(&mut h);
        assert_eq!(c.wsel.select, WeaponId::Rifle as i32);
    }

    #[test]
    fn voip_toggle_on_k() {
        let k = crate::keys::key_string_to_keynum("k");
        // a first start binds it from port_default.cfg
        let (mut c, mut h) = started();
        assert_eq!(c.keys.get_binding(k), "voip_toggle");
        assert!(!c.config_modified());
        // each press flips the microphone
        c.add_text("voip_toggle\n");
        c.execute(&mut h);
        assert_eq!(c.cvars.integer("cl_voipSend"), 1);
        assert_eq!(c.take_output(), "VoIP: microphone on\n");
        // CL_CaptureVoip may set cl_voipSend back to 0: the next press is still "off"
        c.cvars.set("cl_voipSend", "0");
        c.add_text("voip_toggle\n");
        c.execute(&mut h);
        assert_eq!(c.cvars.integer("cl_voipSend"), 0);
        assert_eq!(c.take_output(), "VoIP: microphone off\n");
        // what it writes keeps the bind and the version
        let cfg = c.write_config();
        assert!(cfg.contains("bind k \"voip_toggle\"") && cfg.contains("port_bindsVersion"), "{cfg}");

        let start = |autogen: &str| {
            let mut c = Client::new();
            let mut h = Host { files: vec![(CONFIG_CFG.into(), autogen.into())], ..Default::default() };
            c.startup(&mut h);
            c
        };
        // a config from before the bind: k gets it once, and is saved
        let c = start("unbindall\nbind w \"+forward\"\n");
        assert_eq!(c.keys.get_binding(k), "voip_toggle");
        assert!(c.config_modified());
        assert_eq!(c.cvars.integer("port_bindsVersion"), 1);
        // ... but a k the player bound stays theirs
        let c = start("unbindall\nbind k \"say hi\"\n");
        assert_eq!(c.keys.get_binding(k), "say hi");
        // and a k unbound after the bind came stays unbound
        let c = start("unbindall\nbind w \"+forward\"\nseta port_bindsVersion \"1\"\n");
        assert_eq!(c.keys.get_binding(k), "");
        assert!(!c.config_modified());
    }
}
