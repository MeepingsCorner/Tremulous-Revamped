//! Kernel `svcmds` — game/g_svcmds.c: commands of the server console
//! (`ConsoleCommand` and the `Svcmd_*` functions). Output goes to
//! `World::console` (G_Printf), which the caller drains.
//!
//! The host's console is treated as a dedicated server console (the
//! `dedicated` commands — say, chat, a, m, cp, say_team, listmaps — are
//! available, dec026): there is no engine-level console chat in this port.
//! Map rotation commands (mapRotation, advanceMapRotation, stopMapRotation)
//! are run by the host, which owns the rotation state (dec022), as are the
//! engine's own map / devmap / map_restart / set.

use super::admin::{self, team_from_string};
use super::cmds::{self, concat, SayMode};
use super::world::{EType, World, CONSOLE, MAX_CLIENTS};
use crate::gen::enums::Team;

/// `Cmd_TokenizeString`: whitespace-separated, "quoted" arguments kept whole.
pub fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i] <= b' ' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        // a // comment ends the line
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            break;
        }
        let mut tok = Vec::new();
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                tok.push(b[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < b.len() && b[i] > b' ' {
                tok.push(b[i]);
                i += 1;
            }
        }
        out.push(String::from_utf8_lossy(&tok).into_owned());
    }
    out
}

fn printf(w: &mut World, s: &str) {
    w.console.push_str(s);
}

/// `ClientForString`.
fn client_for_string(w: &mut World, s: &str) -> Option<usize> {
    match super::client::client_number_from_string(w, s) {
        Ok(i) => Some(i),
        Err(e) => {
            printf(w, &e);
            None
        }
    }
}

/// `Svcmd_EntityList_f`.
pub fn entity_list(w: &mut World, _a: &[&str]) {
    let mut s = String::new();
    for e in 0..w.num_entities {
        let ent = &w.ents[e];
        if !ent.inuse {
            continue;
        }
        s.push_str(&format!("{e:>3}:"));
        let t = match ent.etype {
            EType::General => "ET_GENERAL          ",
            EType::Player => "ET_PLAYER           ",
            EType::Buildable => "ET_BUILDABLE        ",
            EType::Location => "ET_LOCATION         ",
            EType::Missile => "ET_MISSILE          ",
            EType::Mover => "ET_MOVER            ",
            EType::Portal => "ET_PORTAL           ",
            EType::Speaker => "ET_SPEAKER          ",
            EType::PushTrigger => "ET_PUSH_TRIGGER     ",
            EType::TeleportTrigger => "ET_TELEPORT_TRIGGER ",
            EType::Invisible => "ET_INVISIBLE        ",
            EType::Corpse => "ET_CORPSE           ",
            EType::ParticleSystem => "ET_PARTICLE_SYSTEM  ",
            EType::AnimMapObj => "ET_ANIMMAPOBJ       ",
            EType::ModelDoor => "ET_MODELDOOR        ",
            EType::LightFlare => "ET_LIGHTFLARE       ",
        };
        s.push_str(t);
        s.push_str(ent.classname);
        s.push('\n');
    }
    printf(w, &s);
}

/// The "ip" userinfo key: the host's player is "localhost", bots "bot".
fn client_ip(w: &World, i: usize) -> &'static str {
    if w.clients[i].local_client { "localhost" } else { "bot" }
}

/// `Svcmd_Status_f`.
pub fn status(w: &mut World, _a: &[&str]) {
    let mut s = String::from("slot score ping address               rate     name\n---- ----- ---- -------               ----     ----\n");
    for i in 0..MAX_CLIENTS {
        let c = &w.clients[i];
        if !c.connected {
            continue;
        }
        s.push_str(&format!("{:<4} {:<5} {:<4} {:<21} {:<8} {}\n", i, c.ps.persistant[crate::gen::enums::Pers::Score as usize], c.ps.ping, client_ip(w, i), 25000, c.name));
    }
    printf(w, &s);
}

/// `Svcmd_ForceTeam_f`.
pub fn force_team(w: &mut World, a: &[&str]) {
    if a.len() != 3 {
        printf(w, "usage: forceteam <player> <team>\n");
        return;
    }
    let Some(cl) = client_for_string(w, a[1]) else { return };
    let Some(team) = team_from_string(a[2]) else {
        printf(w, &format!("forceteam: invalid team \"{}\"\n", a[2]));
        return;
    };
    super::client::change_team(w, cl, team);
}

/// `Svcmd_LayoutSave_f`.
pub fn layout_save(w: &mut World, a: &[&str]) {
    if a.len() != 2 {
        printf(w, "usage: layoutsave <name>\n");
        return;
    }
    // sanitize name (MAX_QPATH - 4)
    let str2: String = a[1].chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64 - 4 - 1).collect();
    if str2.is_empty() {
        printf(w, &format!("layoutsave: invalid name \"{}\"\n", a[1]));
        return;
    }
    super::layout::layout_save(w, &str2);
}

/// `Svcmd_LayoutLoad_f`: set g_layouts and map_restart.
pub fn layout_load(w: &mut World, a: &[&str]) {
    if a.len() < 2 {
        printf(w, "usage: layoutload <name> ...\n");
        return;
    }
    let mut s = concat(a, 1);
    s.truncate(255);
    w.exec.push(format!("set g_layouts \"{s}\""));
    w.exec.push("map_restart".into());
}

/// `Svcmd_AdmitDefeat_f`.
pub fn admit_defeat(w: &mut World, a: &[&str]) {
    if a.len() != 2 {
        printf(w, "admitdefeat: must provide a team\n");
        return;
    }
    match team_from_string(a[1]) {
        Some(t @ (Team::Aliens | Team::Humans)) => super::vote::admit_defeat(w, t),
        _ => printf(w, "admitdefeat: invalid team\n"),
    }
}

/// `Svcmd_TeamWin_f` (alienWin / humanWin).
pub fn team_win(w: &mut World, a: &[&str]) {
    match team_from_string(a[0]) {
        Some(Team::Aliens) => super::vote::base_self_destruct(w, Team::Humans),
        Some(Team::Humans) => super::vote::base_self_destruct(w, Team::Aliens),
        _ => {}
    }
}

/// `Svcmd_Evacuation_f`.
pub fn evacuation(w: &mut World, _a: &[&str]) {
    super::vote::evacuation(w);
}

/// `Svcmd_TeamMessage_f` (console say_team <team> <message>).
pub fn team_message(w: &mut World, a: &[&str]) {
    if a.len() < 3 {
        printf(w, "usage: say_team <team> <message>\n");
        return;
    }
    let Some(team) = team_from_string(a[1]) else {
        printf(w, &format!("say_team: invalid team \"{}\"\n", a[1].chars().next().unwrap_or(' ')));
        return;
    };
    let msg = concat(a, 2);
    let t = w.level.time;
    for i in 0..MAX_CLIENTS {
        if w.clients[i].connected && (w.clients[i].team == team || (w.clients[i].team == Team::None && admin::permission(w, Some(i), admin::ADMF_SPEC_ALLCHAT))) {
            w.chat.push((t, i, CONSOLE, SayMode::Team, format!("console: {msg}")));
        }
    }
    super::log::log_printf(w, &format!("SayTeam: -1 \"console\": {msg}\n"));
}

/// `Svcmd_CenterPrint_f`.
pub fn center_print(w: &mut World, a: &[&str]) {
    if a.len() < 2 {
        printf(w, "usage: cp <message>\n");
        return;
    }
    let t = w.level.time;
    w.center_prints.push((t, None, None, concat(a, 1)));
}

/// `Svcmd_EjectClient_f`: drop a client (never the host's own player).
pub fn eject_client(w: &mut World, a: &[&str]) {
    if a.len() < 2 {
        printf(w, "usage: eject <player|-1> <reason>\n");
        return;
    }
    let reason = concat(a, 2);
    if super::spawn::c_atoi(a[1]) == -1 {
        for i in 0..MAX_CLIENTS {
            if w.clients[i].connected && !w.clients[i].local_client {
                admin::drop_client(w, i, &reason);
            }
        }
    } else {
        let Some(cl) = client_for_string(w, a[1]) else { return };
        if w.clients[cl].local_client {
            printf(w, "eject: cannot eject local clients\n");
            return;
        }
        admin::drop_client(w, cl, &reason);
    }
}

/// `Svcmd_DumpUser_f`. The userinfo has the keys the port knows; like the
/// original loop, the last pair is not printed.
pub fn dump_user(w: &mut World, a: &[&str]) {
    if a.len() != 2 {
        printf(w, "usage: dumpuser <player>\n");
        return;
    }
    let Some(cl) = client_for_string(w, a[1]) else { return };
    let c = &w.clients[cl];
    let pairs = [("name", c.name.clone()), ("ip", client_ip(w, cl).to_string()), ("cg_stickySpec", (c.sticky_spec as i32).to_string()), ("rate", "25000".into())];
    let mut s = String::from("userinfo\n--------\n");
    for (k, v) in &pairs[..pairs.len() - 1] {
        s.push_str(&format!("{k:<20}{v}\n"));
    }
    printf(w, &s);
}

/// `Svcmd_Pr_f`: print to one client (or -1: everyone).
pub fn pr(w: &mut World, a: &[&str]) {
    if a.len() < 3 {
        printf(w, "usage: <clientnum|-1> <message>\n");
        return;
    }
    let cl = super::spawn::c_atoi(a[1].get(..3).unwrap_or(a[1]));
    if cl >= MAX_CLIENTS as i32 || cl < -1 {
        printf(w, &format!("invalid clientnum {cl}\n"));
        return;
    }
    let t = w.level.time;
    w.prints.push((t, None, (cl >= 0).then_some(cl as usize), concat(a, 2)));
}

/// `G_PrintSpawnQueue` (the port's queue is a deque: front 0).
fn print_spawn_queue(w: &mut World, team: Team) {
    let q = if team == Team::Aliens { &w.level.alien_spawn_queue } else { &w.level.human_spawn_queue };
    let len = q.len();
    let back = (len as i32 - 1).rem_euclid(MAX_CLIENTS as i32);
    let mut s = format!("l:{len} f:0 b:{back}    :");
    for c in q.iter() {
        s.push_str(&format!("{c}:"));
    }
    s.push('\n');
    printf(w, &s);
}

/// `Svcmd_PrintQueue_f`.
pub fn print_queue(w: &mut World, a: &[&str]) {
    if a.len() != 2 {
        printf(w, "usage: printqueue <team>\n");
        return;
    }
    match team_from_string(a[1]) {
        Some(t @ (Team::Aliens | Team::Humans)) => print_spawn_queue(w, t),
        _ => printf(w, "unknown team\n"),
    }
}

/// `Svcmd_MessageWrapper`: a, m, say, chat from the console.
pub fn message_wrapper(w: &mut World, a: &[&str]) {
    match a[0].to_ascii_lowercase().as_str() {
        "a" => cmds::admin_message_cmd(w, None, a),
        "m" => cmds::private_message_from(w, None, a),
        "say" => cmds::say_all(w, CONSOLE, SayMode::All, &concat(a, 1)),
        "chat" => cmds::say_all(w, CONSOLE, SayMode::Raw, &concat(a, 1)),
        _ => {}
    }
}

/// `Svcmd_ListMapsWrapper`: Cmd_ListMaps_f( NULL ).
pub fn list_maps(w: &mut World, a: &[&str]) {
    let search = a.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    let mut maps: Vec<String> = w.level.map_list.iter().filter(|m| m.to_lowercase().contains(&search)).cloned().collect();
    // `SortMaps`: strcmp order
    maps.sort();
    let s = format!("Listing {} maps:\n{}\n", maps.len(), maps.join(" "));
    printf(w, &s);
}

/// `Svcmd_SuddenDeath_f`.
pub fn sudden_death(w: &mut World, a: &[&str]) {
    let secs = a.get(1).map(|s| super::spawn::c_atoi(s.get(..4).unwrap_or(s))).unwrap_or(0);
    super::vote::sudden_death(w, secs);
}

/// `G_LoadCensors` as a console command.
pub fn load_censors(w: &mut World, _a: &[&str]) {
    cmds::load_censors(w);
}

/// `BG_MemoryInfo`: the port has no BG_Alloc zone.
pub fn game_memory(w: &mut World, _a: &[&str]) {
    printf(w, "game_memory: Rust allocator (no BG_Alloc zone)\n");
}

type Svcmd = fn(&mut World, &[&str]);

/// The `svcmds[]` table (map rotation commands are the host's).
const SVCMDS: &[(&str, Svcmd)] = &[
    ("a", message_wrapper),
    ("admitDefeat", admit_defeat),
    ("alienWin", team_win),
    ("chat", message_wrapper),
    ("cp", center_print),
    ("dumpuser", dump_user),
    ("eject", eject_client),
    ("entityList", entity_list),
    ("evacuation", evacuation),
    ("forceTeam", force_team),
    ("game_memory", game_memory),
    ("humanWin", team_win),
    ("layoutLoad", layout_load),
    ("layoutSave", layout_save),
    ("listmaps", list_maps),
    ("loadcensors", load_censors),
    ("m", message_wrapper),
    ("pr", pr),
    ("printqueue", print_queue),
    ("say", message_wrapper),
    ("say_team", team_message),
    ("status", status),
    ("suddendeath", sudden_death),
];

/// The `svcmds[]` names with their `dedicated` flag, in the C table's
/// order (the map rotation commands run in the host here).
const SVCMD_NAMES: &[(&str, bool)] = &[
    ("a", true),
    ("admitDefeat", false),
    ("advanceMapRotation", false),
    ("alienWin", false),
    ("chat", true),
    ("cp", true),
    ("dumpuser", false),
    ("eject", false),
    ("entityList", false),
    ("evacuation", false),
    ("forceTeam", false),
    ("game_memory", false),
    ("humanWin", false),
    ("layoutLoad", false),
    ("layoutSave", false),
    ("listmaps", true),
    ("loadcensors", false),
    ("m", true),
    ("mapRotation", false),
    ("pr", false),
    ("printqueue", false),
    ("say", true),
    ("say_team", true),
    ("status", false),
    ("stopMapRotation", false),
    ("suddendeath", false),
];

/// `G_RegisterCommands`: the server console commands the engine learns
/// (`trap_AddCommand`, for its tab completion): the svcmds (the dedicated
/// ones only on a dedicated server) and `G_admin_register_cmds`' admin
/// command keywords. `G_UnregisterCommands` / `G_admin_unregister_cmds`
/// remove the same list again (the host drops it with the World).
pub fn register_commands(w: &World) -> Vec<String> {
    let mut out: Vec<String> = SVCMD_NAMES.iter().filter(|(_, ded)| !*ded || w.level.dedicated).map(|(n, _)| n.to_string()).collect();
    out.extend(crate::gen::admin_commands::ADMIN_COMMANDS.iter().map(|d| d.id.to_string()));
    out
}

/// Is `cmd` one of the `svcmds[]` (not an admin command)?
pub fn is_svcmd(cmd: &str) -> bool {
    SVCMDS.iter().any(|(n, _)| n.eq_ignore_ascii_case(cmd))
}

/// Is `cmd` a game console command (svcmd or admin command)?
pub fn is_console_command(w: &World, cmd: &str) -> bool {
    SVCMDS.iter().any(|(n, _)| n.eq_ignore_ascii_case(cmd)) || admin::admin_command_exists(w, cmd)
}

/// `ConsoleCommand`: run one server console line. Returns false when it is
/// neither an svcmd nor an admin command (the host handles it); output is
/// in `World::console`.
pub fn console_command(w: &mut World, line: &str) -> bool {
    let toks = tokenize(line);
    let argv: Vec<&str> = toks.iter().map(|s| s.as_str()).collect();
    let Some(&cmd) = argv.first() else { return false };
    match SVCMDS.iter().find(|(n, _)| n.eq_ignore_ascii_case(cmd)) {
        Some((_, f)) => {
            f(w, &argv);
            true
        }
        // see if this is an admin command
        None => admin::cmd_check(w, None, &argv),
    }
}

/// `Cbuf_Execute` for `trap_SendConsoleCommand( EXEC_APPEND )`: run the
/// queued lines the game handles itself; the rest (map, map_restart, set
/// ...) are returned for the host's console, in order.
pub fn run_exec(w: &mut World) -> Vec<String> {
    let mut host = Vec::new();
    for _ in 0..64 {
        if w.exec.is_empty() {
            break;
        }
        for text in std::mem::take(&mut w.exec) {
            for line in text.split([';', '\n']).map(str::trim).filter(|l| !l.is_empty()) {
                if !console_command(w, line) {
                    host.push(line.to_string());
                }
            }
        }
    }
    host
}
