//! The server log (kernel `log`, dec031): `G_LogPrintf` lines checked
//! against the C format strings (g_client.c, g_cmds.c, g_combat.c,
//! g_buildable.c, g_team.c, g_main.c, g_admin.c), the `%3i:%i%i` clock,
//! colour stripping, g_logFileSync buffering, the dedicated console echo,
//! LogExit / G_SendGameStat and the admin execution log.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use trem_sim::gen::buildables::BuildableId;
use trem_sim::gen::enums::{MeansOfDeath, SpectatorState, Team};
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::world::{Fs, World};
use trem_sim::kernel::{admin, bsp, buildable, client, combat, log, svcmds, team};

#[derive(Clone, Default)]
struct MemFs(Rc<RefCell<BTreeMap<String, String>>>);

impl Fs for MemFs {
    fn read(&self, path: &str) -> Option<String> {
        self.0.borrow().get(path).cloned()
    }
    fn write(&mut self, path: &str, text: &str) -> bool {
        self.0.borrow_mut().insert(path.to_string(), text.to_string());
        true
    }
    fn list(&self, _dir: &str, _ext: &str) -> Vec<String> {
        Vec::new()
    }
}

impl MemFs {
    fn log(&self) -> String {
        self.read("games.log").unwrap_or_default()
    }
    /// The log lines without their time stamp.
    fn lines(&self) -> Vec<String> {
        self.log().lines().map(|l| l.get(7..).unwrap_or("").to_string()).collect()
    }
}

/// A world on the sheet arena with an open log (G_InitGame).
fn world(fs: &MemFs) -> World {
    let b = bsp::from_sheet(MapId::Arena);
    let mut w = World::from_bsp_with(&b, "arena", Some(Box::new(fs.clone())), None);
    w.real_time = 1_790_000_000; // 2026-09-21 14:13:20 UTC
    // lines reach the file at once (the tests read it between frames)
    w.set_cvar("g_logFileSync", "1");
    log::init_game(&mut w);
    w
}

fn spawn(w: &mut World, name: &str, team: &str, class: &str) -> usize {
    let i = w.connect(name, false).unwrap();
    client::command(w, i, &format!("team {team}")).unwrap();
    client::command(w, i, &format!("class {class}")).unwrap();
    for _ in 0..200 {
        w.frame(16);
        if w.clients[i].spectator == SpectatorState::Not as i32 {
            return i;
        }
    }
    panic!("{name} never spawned");
}

fn has(fs: &MemFs, line: &str) -> bool {
    fs.lines().iter().any(|l| l == line)
}

#[test]
fn clock_prefix_is_the_c_format() {
    // "%3i:%i%i "
    assert_eq!(log::log_time(0, 0), "  0:00 ");
    assert_eq!(log::log_time(61_000, 0), "  1:01 ");
    assert_eq!(log::log_time(10 * 60_000 + 59_999, 0), " 10:59 ");
    assert_eq!(log::log_time(123 * 60_000, 0), "123:00 ");
    assert_eq!(log::log_time(5_000, 2_000), "  0:03 ");
    assert_eq!(log::real_time(1_790_000_000), (2026, 9, 21, 14, 13, 20));
    assert_eq!(log::real_time(0), (1970, 1, 1, 0, 0, 0));
}

#[test]
fn mod_names_match_g_combat() {
    // modNames[] in g_combat.c, in order
    let c = [
        "MOD_UNKNOWN", "MOD_SHOTGUN", "MOD_BLASTER", "MOD_PAINSAW", "MOD_MACHINEGUN", "MOD_CHAINGUN", "MOD_PRIFLE", "MOD_MDRIVER", "MOD_LASGUN", "MOD_LCANNON",
        "MOD_LCANNON_SPLASH", "MOD_FLAMER", "MOD_FLAMER_SPLASH", "MOD_GRENADE", "MOD_WATER", "MOD_SLIME", "MOD_LAVA", "MOD_CRUSH", "MOD_TELEFRAG", "MOD_FALLING",
        "MOD_SUICIDE", "MOD_TARGET_LASER", "MOD_TRIGGER_HURT", "MOD_ABUILDER_CLAW", "MOD_LEVEL0_BITE", "MOD_LEVEL1_CLAW", "MOD_LEVEL1_PCLOUD", "MOD_LEVEL3_CLAW",
        "MOD_LEVEL3_POUNCE", "MOD_LEVEL3_BOUNCEBALL", "MOD_LEVEL2_CLAW", "MOD_LEVEL2_ZAP", "MOD_LEVEL4_CLAW", "MOD_LEVEL4_TRAMPLE", "MOD_LEVEL4_CRUSH",
        "MOD_SLOWBLOB", "MOD_POISON", "MOD_SWARM", "MOD_HSPAWN", "MOD_TESLAGEN", "MOD_MGTURRET", "MOD_REACTOR", "MOD_ASPAWN", "MOD_ATUBE", "MOD_OVERMIND",
        "MOD_DECONSTRUCT", "MOD_REPLACE", "MOD_NOCREEP",
    ];
    for (i, n) in c.iter().enumerate() {
        assert_eq!(log::mod_name(i as i32), *n);
    }
    // fall back on the number
    assert_eq!(log::mod_name(c.len() as i32), c.len().to_string());
    assert_eq!(log::mod_name(-1), "-1");
}

#[test]
fn init_connect_say_and_shutdown_lines() {
    let fs = MemFs::default();
    let mut w = world(&fs);
    let log0 = fs.log();
    let first: Vec<&str> = log0.lines().collect();
    assert_eq!(first[0], "  0:00 ------------------------------------------------------------");
    assert!(first[1].starts_with("  0:00 InitGame: \\version\\tremulous 1.1.0\\") && first[1].ends_with("\\mapname\\arena"), "{}", first[1]);
    assert_eq!(first[2], "  0:00 RealTime: 2026/09/21 14:13:20");

    let i = w.connect("^1Red^7Name", false).unwrap();
    // ClientConnect: %i [%s] (%s) "%s^7" "%c%s%c^7" (decoloured, DECOLOR_OFF keeps the colours)
    let guid = w.clients[i].guid.clone();
    assert!(has(&fs, &format!("ClientConnect: {i} [] ({guid}) \"RedName\" \"^1Red^7Name\"")), "{:#?}", fs.lines());
    // the host's player is an admin: AdminAuth, then ClientBegin
    assert!(fs.lines().iter().any(|l| l.starts_with(&format!("AdminAuth: {i} \"RedName\" \""))), "{:#?}", fs.lines());
    assert!(has(&fs, &format!("ClientBegin: {i}")));
    client::command(&mut w, i, "say hello ^3world").unwrap();
    // Say: %d "%s^7": ^2%s
    assert!(has(&fs, &format!("Say: {i} \"RedName\": hello world")), "{:#?}", fs.lines());
    client::command(&mut w, i, "team humans").unwrap();
    // ChangeTeam: %d %s: %s^7 switched teams
    assert!(has(&fs, &format!("ChangeTeam: {i} human: RedName switched teams")));
    client::command(&mut w, i, "say_team hi team").unwrap();
    assert!(has(&fs, &format!("SayTeam: {i} \"RedName\": hi team")));
    // console say_team (g_svcmds.c) and privmsg
    svcmds::console_command(&mut w, "say_team h from the console");
    assert!(has(&fs, "SayTeam: -1 \"console\": from the console"), "{:#?}", fs.lines());
    let j = w.connect("Other", false).unwrap();
    client::command(&mut w, i, "m Other psst").unwrap();
    // %s: %d "%s^7" "%s": ^%c%s
    assert!(has(&fs, &format!("PrivMsg: {i} \"RedName\" \"Other\": psst")), "{:#?}", fs.lines());
    // a rename: ClientRename: %i [%s] (%s) "%s^7" -> "%s^7" "%c%s%c^7"
    let gj = w.clients[j].guid.clone();
    client::userinfo_changed(&mut w, j, "^2Green", true);
    assert!(has(&fs, &format!("ClientRename: {j} [] ({gj}) \"Other\" -> \"Green\" \"^2Green\"")), "{:#?}", fs.lines());
    w.disconnect(j);
    assert!(has(&fs, &format!("ClientDisconnect: {j} [] ({gj}) \"Green\"")));
    log::shutdown_game(&mut w);
    let l = fs.lines();
    assert_eq!(l[l.len() - 2], "ShutdownGame:");
    assert_eq!(l[l.len() - 1], "------------------------------------------------------------");
    // nothing after shutdown
    client::command(&mut w, i, "say late").unwrap();
    w.frame(16);
    assert!(!fs.log().contains("late"));
}

#[test]
fn log_file_sync_and_dedicated_echo() {
    let fs = MemFs::default();
    let mut w = world(&fs);
    w.set_cvar("g_logFileSync", "0");
    let i = w.connect("P", false).unwrap();
    // g_logFileSync 0: buffered until the end of the frame
    client::command(&mut w, i, "say one").unwrap();
    assert!(!fs.log().contains("Say: 0 \"P\": one"));
    w.frame(16);
    assert!(fs.log().contains("Say: 0 \"P\": one"));
    // g_logFileSync 1: at once
    assert!(w.set_cvar("g_logFileSync", "1"));
    client::command(&mut w, i, "say two").unwrap();
    assert!(fs.log().contains("Say: 0 \"P\": two"));
    // dedicated: echoed to the server console without the clock (G_UnEscapeString keeps colours)
    assert!(w.set_cvar("dedicated", "1"));
    w.console.clear();
    client::command(&mut w, i, "say three").unwrap();
    assert!(w.console.contains("Say: 0 \"P^7\": ^2three\n"), "{:?}", w.console);
    // no g_logFile: not logging to disk
    let fs2 = MemFs::default();
    let b = bsp::from_sheet(MapId::Arena);
    let mut w2 = World::from_bsp_with(&b, "arena", Some(Box::new(fs2.clone())), None);
    w2.set_cvar("g_logFile", "");
    log::init_game(&mut w2);
    assert!(w2.console.contains("Not logging to disk"));
    assert!(fs2.read("games.log").is_none());
}

#[test]
fn kill_construct_deconstruct_stage_and_exit_lines() {
    let fs = MemFs::default();
    let mut w = world(&fs);
    let h = spawn(&mut w, "Marine", "humans", "ckit");
    let a = spawn(&mut w, "Xeno", "aliens", "level0");
    // Die: %d %d %s: %s^7 killed %s
    combat::damage(&mut w, a, Some(h), Some(h), None, None, 1000, 0, MeansOfDeath::Shotgun);
    assert!(has(&fs, &format!("Die: {h} {a} MOD_SHOTGUN: Marine killed Xeno")), "{:#?}", fs.lines());
    // a world kill
    let h2 = spawn(&mut w, "Rookie", "humans", "rifle");
    combat::damage(&mut w, h2, None, None, None, None, 1000, 0, MeansOfDeath::Falling);
    assert!(fs.lines().iter().any(|l| l.ends_with(&format!(" {h2} MOD_FALLING: <world> killed Rookie"))), "{:#?}", fs.lines());
    // Construct: %d %d %s%s: %s^7 is building %s%s%s
    let o = w.clients[h].ps.origin;
    let e = buildable::build(&mut w, Some(h), BuildableId::Medistat, [o[0] + 300.0, o[1], o[2]], [0.0, 0.0, 1.0], [0.0; 3]);
    assert!(has(&fs, &format!("Construct: {h} {e} medistat: Marine is building Medistation")), "{:#?}", fs.lines());
    // Deconstruct (G_LogDestruction): ^3Deconstruct: %d %d %s %s: %s %s by %s
    combat::damage(&mut w, e, Some(h), Some(h), None, None, 10000, 0, MeansOfDeath::Deconstruct);
    assert!(has(&fs, &format!("Deconstruct: {h} {e} medistat MOD_DECONSTRUCT: Medistation deconstructed by Marine")), "{:#?}", fs.lines());
    // Stage: A 2: Aliens reached Stage 2
    w.level.alien_credits = 1_000_000;
    team::calculate_stages(&mut w);
    assert!(has(&fs, "Stage: A 2: Aliens reached Stage 2"));
    assert!(has(&fs, "Stage: A 3: Aliens reached Stage 3"));
    assert_eq!(w.level.alien_stage2_time, w.level.time);
    // LogExit: Exit: %s, score: %i  ping: %i  client: %i %s, then the game stat
    w.level.uncond_human_win = true;
    w.frame(16);
    let lines = fs.lines();
    assert!(lines.iter().any(|l| l == "Exit: Humans win."), "{lines:#?}");
    assert!(lines.iter().any(|l| l.starts_with("score: ") && l.contains(&format!("  ping: 0  client: {h} Marine"))), "{lines:#?}");
    let stat = lines.iter().find(|l| l.starts_with("GameStat: ")).expect("gamestat line");
    // "%s %s T:%c A:%f H:%f M:%s D:%d SD:%d AS:%d AS2T:%d AS3T:%d HS:%d HS2T:%d HS3T:%d CL:%d" + " \"%s\" %c %d %d %d" per client
    let re_head = format!("GameStat: tremulous 1.1.0 main T:H A:");
    assert!(stat.starts_with(&re_head), "{stat}");
    assert!(stat.contains(" M:arena D:") && stat.contains(" AS:2 AS2T:") && stat.contains(" HS:0 HS2T:0 HS3T:0 CL:3"), "{stat}");
    assert!(stat.contains(" \"Marine\" H ") && stat.contains(" \"Xeno\" A "), "{stat}");
    let a_field = stat.split(" A:").nth(1).unwrap().split(' ').next().unwrap();
    assert_eq!(a_field.split('.').nth(1).map(|d| d.len()), Some(6), "%f has six decimals: {a_field}");
}

#[test]
fn votes_and_admin_exec_lines() {
    let fs = MemFs::default();
    let mut w = world(&fs);
    let i = w.connect("Host", false).unwrap();
    let j = w.connect("Guest", false).unwrap();
    w.level.allow_vote = true;
    client::command(&mut w, j, "team aliens").unwrap();
    client::command(&mut w, i, "team aliens").unwrap();
    w.frame(16); // (a vote time of 0 means no vote)
    client::command(&mut w, j, "callvote map_restart").unwrap();
    // %s: %d "%s^7": %s with level.voteString
    assert!(has(&fs, &format!("CallVote: {j} \"Guest\": map_restart")), "{:#?}", fs.lines());
    let _ = client::command(&mut w, i, "vote yes");
    for _ in 0..2000 {
        w.frame(16);
    }
    // EndVote: %s %s %d %d %d
    let end = fs.lines().into_iter().find(|l| l.starts_with("EndVote: ")).unwrap_or_else(|| panic!("{:#?}", fs.lines()));
    let f: Vec<&str> = end.split(' ').collect();
    assert!(f.len() == 6 && f[1] == "global" && (f[2] == "pass" || f[2] == "fail") && f[3..].iter().all(|n| n.parse::<i32>().is_ok()), "{end}");
    // admin commands: AdminExec: %s: %d "%s^7" "%s^7" [%d] (%s): %s[: admin_log ...]
    assert!(admin::cmd_check(&mut w, Some(i), &["lock", "a"]));
    let gi = w.clients[i].guid.clone();
    let lvl = w.admin.admins[w.clients[i].admin.unwrap()].level;
    assert!(has(&fs, &format!("AdminExec: ok: {i} \"Host\" \"Host\" [{lvl}] ({gi}): lock: alien")), "{:#?}", fs.lines());
    // a failed command logs its arguments (ConcatArgsPrintable quotes the spaced one)
    assert!(admin::cmd_check(&mut w, Some(i), &["putteam", "nobody here", "h"]));
    assert!(has(&fs, &format!("AdminExec: fail: {i} \"Host\" \"Host\" [{lvl}] ({gi}): putteam: \"nobody here\" h")), "{:#?}", fs.lines());
    // a guest without the permission
    assert!(admin::cmd_check(&mut w, Some(j), &["kick", "Host", "bye"]));
    let gj = w.clients[j].guid.clone();
    assert!(has(&fs, &format!("AdminExec: fail: {j} \"Guest\" \"Guest\" [0] ({gj}): kick: Host bye")), "{:#?}", fs.lines());
    // the console
    assert!(admin::cmd_check(&mut w, None, &["putteam", "Guest", "h"]));
    assert!(has(&fs, &format!("AdminExec: ok: -1 \"console\" \"console\" [0] (): putteam: {j} ({gj}) \"Guest\"")), "{:#?}", fs.lines());
    // silent commands are not logged
    let n = fs.lines().len();
    assert!(admin::cmd_check(&mut w, Some(i), &["listplayers"]));
    w.frame(16);
    assert_eq!(fs.lines().iter().skip(n).filter(|l| l.starts_with("AdminExec")).count(), 0);
    // G_AdminMessage: %s: %d "%s^7": ^6%s
    svcmds::console_command(&mut w, "a hello admins");
    assert!(has(&fs, "AdminMsg: -1 \"console\": hello admins"), "{:#?}", fs.lines());
    let _ = Team::None;
}
