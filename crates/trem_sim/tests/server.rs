//! Server-side subsystems (dec026): layouts and the build log
//! (g_buildable.c), the admin system (g_admin.c, g_namelog.c), server
//! console commands (g_svcmds.c), name cleaning, inactivity, session data.
//! Behavioural checks against the original rules (dec014).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use trem_sim::gen::buildables::BuildableId;
use trem_sim::gen::enums::{SpectatorState, Team};
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::world::{EType, Fs, World};
use trem_sim::kernel::{admin, bot, bsp, client, layout, session, svcmds};

/// An in-memory `trap_FS` the test can look into after the World owns it.
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
    fn list(&self, dir: &str, ext: &str) -> Vec<String> {
        let pre = format!("{dir}/");
        self.0.borrow().keys().filter_map(|k| k.strip_prefix(&pre)).filter(|k| !k.contains('/') && k.ends_with(ext)).map(|k| k.to_string()).collect()
    }
}

fn run_until_spawned(w: &mut World, i: usize) {
    for _ in 0..200 {
        w.frame(16);
        if w.clients[i].spectator == SpectatorState::Not as i32 {
            return;
        }
    }
    panic!("client {i} never spawned");
}

fn join(w: &mut World, name: &str, team: &str, class: &str) -> usize {
    let i = w.connect(name, false).unwrap();
    client::command(w, i, &format!("team {team}")).unwrap();
    client::command(w, i, &format!("class {class}")).unwrap();
    run_until_spawned(w, i);
    i
}

fn console(w: &mut World, line: &str) -> String {
    w.console.clear();
    assert!(svcmds::console_command(w, line), "{line} is a console command");
    std::mem::take(&mut w.console)
}

fn buildables(w: &World) -> Vec<(BuildableId, [f32; 3])> {
    (0..w.num_entities).filter(|&i| w.ents[i].inuse && w.ents[i].etype == EType::Buildable).map(|i| (w.ents[i].buildable.unwrap(), w.ents[i].pos.tr_base)).collect()
}

fn arena_world(fs: &MemFs, layouts: Option<(&str, bool)>) -> World {
    let b = bsp::from_sheet(MapId::Arena);
    World::from_bsp_with(&b, "arena", Some(Box::new(fs.clone())), layouts)
}

#[test]
fn layouts_save_list_select_and_load() {
    let fs = MemFs::default();
    let mut w = arena_world(&fs, None);
    let original = buildables(&w);
    assert!(!original.is_empty());
    // one more structure than the map has, then layoutsave (name sanitised)
    let o = original.iter().find(|b| b.0 == BuildableId::Reactor).unwrap().1;
    trem_sim::kernel::buildable::spawn_from_map(&mut w, BuildableId::Medistat, [o[0] + 200.0, o[1], o[2]], [0.0; 3]);
    let out = console(&mut w, "layoutsave my!layout");
    assert!(out.contains("layouts/arena/mylayout.dat"), "{out}");
    let text = fs.read("layouts/arena/mylayout.dat").unwrap();
    assert_eq!(text.lines().count(), original.len() + 1);
    assert!(text.lines().all(|l| l.split(' ').count() == 13));
    assert!(text.lines().any(|l| l.starts_with("medistat ")));

    // G_LayoutList: *BUILTIN* + the file
    let (n, list) = layout::layout_list(&mut w, "arena");
    assert_eq!((n, list.as_str()), (2, "*BUILTIN* mylayout "));
    let out = console(&mut w, "listlayouts");
    assert!(out.contains("2 layouts found for 'arena'") && out.contains(" mylayout"), "{out}");

    // g_layouts "mylayout": the map's structures are replaced by the file's
    let w2 = arena_world(&fs, Some(("mylayout", true)));
    assert_eq!(w2.level.layout, "mylayout");
    let loaded = buildables(&w2);
    assert_eq!(loaded.len(), original.len() + 1);
    assert_eq!(loaded.iter().filter(|b| b.0 == BuildableId::Medistat).count(), original.iter().filter(|b| b.0 == BuildableId::Medistat).count() + 1);
    // the saved positions are where they rest again (layout round trip)
    let again = layout::layout_text(&w2);
    let sorted = |t: &str| {
        let mut v: Vec<String> = t.lines().map(|s| s.to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(sorted(&again), sorted(&text));

    // *BUILTIN* keeps the map; an unknown name falls back to the map default
    assert_eq!(buildables(&arena_world(&fs, Some(("*BUILTIN*", true)))).len(), original.len());
    let w4 = arena_world(&fs, Some(("nosuch", true)));
    assert!(w4.level.layout.is_empty());
    assert_eq!(buildables(&w4).len(), original.len());

    // layoutload: g_layouts + map_restart for the host
    console(&mut w, "layoutLoad mylayout");
    assert_eq!(svcmds::run_exec(&mut w), vec!["set g_layouts \"mylayout\"".to_string(), "map_restart".to_string()]);
}

#[test]
fn layout_text_keeps_sscanf_semantics() {
    let mut w = World::new(MapId::Arena);
    let n0 = buildables(&w).len();
    let o = buildables(&w).iter().find(|b| b.0 == BuildableId::Reactor).unwrap().1;
    let (x, y, z) = (o[0] + 200.0, o[1], o[2] + 8.0);
    // a bad name is skipped, a short line repeats the previous values (sscanf),
    // a final line without '\n' is ignored
    let text = format!("nosuch 0 0 0 0 0 0 0 0 0 0 0 0\nmedistat {x} {y} {z} 0 90 0 0 0 0 0 0 0\nmedistat {x} {}\nreactor 0 0 0", y + 120.0);
    layout::layout_load_text(&mut w, &text);
    let now = buildables(&w);
    assert_eq!(now.len(), n0 + 2, "{}", w.console);
    assert!(now.iter().any(|b| b.0 == BuildableId::Medistat && (b.1[0] - x).abs() < 0.01 && (b.1[1] - y - 120.0).abs() < 0.01));
}

#[test]
fn admin_levels_permissions_and_config_round_trip() {
    let fs = MemFs::default();
    let mut w = World::new(MapId::Arena);
    w.fs = Some(Box::new(fs.clone()));
    // no admin.dat yet: the default levels
    assert!(!admin::readconfig(&mut w, None));
    assert_eq!(w.admin.levels.len(), 6);
    let host = w.connect("Host", false).unwrap();
    let b = bot::add_bot(&mut w, "Xeno1", Team::Aliens).unwrap();
    assert!(w.clients[host].local_client && !w.clients[b].local_client);
    // the host's player became a level 5 admin and admin.dat was written
    let a = w.clients[host].admin.expect("host is an admin");
    assert_eq!(w.admin.admins[a].level, 5);
    let dat = fs.read("admin.dat").unwrap();
    assert!(dat.contains("[level]\nlevel   = 5\nname    = ^1Server Operator\nflags   = ALLFLAGS -IMMUTABLE -INCOGNITO\n"), "{dat}");
    assert!(dat.contains("[admin]\nname    = Host\nguid    = LOCALHOST\nlevel   = 5\n"), "{dat}");
    // ALLFLAGS minus the denied ones; level 0 only has the basics
    assert!(admin::permission(&w, Some(host), "kick") && admin::permission(&w, Some(host), admin::ADMF_SPEC_ALLCHAT));
    assert!(!admin::permission(&w, Some(host), admin::ADMF_IMMUTABLE));
    assert!(admin::permission(&w, Some(b), "time") && !admin::permission(&w, Some(b), "kick"));
    assert!(admin::permission(&w, None, "anything"), "the console always wins");

    // a bot (level 0) is refused, the host may
    w.prints.clear();
    assert!(admin::cmd_check(&mut w, Some(b), &["kick", "Host", "x"]));
    assert!(w.prints.iter().any(|p| p.2 == Some(b) && p.3.contains("permission denied")));
    assert!(!admin::cmd_check(&mut w, Some(b), &["nosuchcmd"]));

    // setlevel from the console, then readconfig restores the mapping
    let out = console(&mut w, "setlevel Xeno1 3");
    assert!(out.contains("was given level 3 admin rights by console"), "{out}");
    let mut text = fs.read("admin.dat").unwrap();
    text.push_str("[command]\ncommand = hello\nexec    = cp hello world\ndesc    = say hello\nflag    = hello\n\n");
    fs.0.borrow_mut().insert("admin.dat".into(), text);
    assert!(admin::readconfig(&mut w, None));
    assert_eq!(w.admin.admins.len(), 2);
    assert_eq!(w.admin.admins[w.clients[b].admin.unwrap()].level, 3);
    assert!(admin::permission(&w, Some(b), "kick") && admin::permission(&w, Some(b), admin::ADMF_ADMINCHAT));
    // custom [command]: runs its exec line through the console buffer
    assert!(admin::cmd_check(&mut w, Some(host), &["hello"]));
    assert_eq!(w.exec, vec!["cp hello world".to_string()]);
    assert!(svcmds::run_exec(&mut w).is_empty());
    assert!(w.center_prints.iter().any(|c| c.3 == "hello world"));
    // a level 3 admin may not touch the level 5 host
    w.prints.clear();
    admin::cmd_check(&mut w, Some(b), &["putteam", "Host", "a"]);
    assert!(w.prints.iter().any(|p| p.3.contains("higher")));
    assert_eq!(w.clients[host].team, Team::None);
    // adminhelp lists what the caller may use
    let out = console(&mut w, "adminhelp kick");
    assert!(out.contains("Syntax: kick [name|slot#] (reason)"), "{out}");
}

#[test]
fn admin_commands_act_on_bots() {
    let fs = MemFs::default();
    let mut w = World::new(MapId::Arena);
    w.fs = Some(Box::new(fs.clone()));
    w.real_time = 1_800_000_000;
    let host = join(&mut w, "Host", "humans", "rifle");
    let x = bot::add_bot(&mut w, "Xeno1", Team::Aliens).unwrap();
    let m = bot::add_bot(&mut w, "Marine1", Team::Humans).unwrap();
    // putteam / lock
    client::command(&mut w, host, "putteam Marine1 a").unwrap();
    assert_eq!(w.clients[m].team, Team::Aliens);
    w.frame(16);
    assert_eq!(w.clients[m].team, Team::Aliens, "the bot stays where it was put");
    client::command(&mut w, host, "lock h").unwrap();
    assert!(w.level.human_team_locked);
    let late = w.connect("Late", false).unwrap();
    assert!(client::command(&mut w, late, "team humans").is_err());
    client::command(&mut w, host, "unlock h").unwrap();
    // mute: CMD_MESSAGE commands are dropped silently
    client::command(&mut w, host, "mute Late").unwrap();
    w.chat.clear();
    client::command(&mut w, late, "say hi").unwrap();
    assert!(w.chat.is_empty());
    client::command(&mut w, host, "unmute Late").unwrap();
    client::command(&mut w, late, "say hi").unwrap();
    assert!(!w.chat.is_empty());
    // denybuild
    client::command(&mut w, host, "denybuild Late").unwrap();
    client::command(&mut w, late, "team aliens").unwrap();
    client::command(&mut w, late, "class builder").unwrap();
    run_until_spawned(&mut w, late);
    assert!(client::command(&mut w, late, "build eggpod").unwrap_err().contains("lost faith"));
    // "!cmd" in chat runs the admin command (kick: a 2 minute ban)
    client::command(&mut w, host, "say !kick Xeno1 bye").unwrap();
    assert!(!w.clients[x].connected);
    assert_eq!(w.admin.bans.len(), 1);
    assert_eq!(w.admin.bans[0].expires, 1_800_000_000 + 120);
    assert!(fs.read("admin.dat").unwrap().contains("[ban]\nname    = Xeno1\nguid    = BOT_XENO1\n"));
    assert!(bot::add_bot(&mut w, "Xeno1", Team::Aliens).is_none(), "banned");
    let out = console(&mut w, "showbans");
    assert!(out.contains("Xeno1") && out.contains("bye"), "{out}");
    console(&mut w, "unban 1");
    assert!(bot::add_bot(&mut w, "Xeno1", Team::Aliens).is_some());
    // the host can't be kicked
    let out = console(&mut w, "kick Host x");
    assert!(out.contains("disconnecting the host"), "{out}");
    // rename: G_admin_name_check refuses a name in use
    let out = console(&mut w, "rename Late Host");
    assert!(out.contains("already in use"), "{out}");
    console(&mut w, "rename Late ^1Newbie");
    assert_eq!(w.clients[late].name, "^1Newbie");
    // listplayers shows levels and the M/B marks
    let out = console(&mut w, "listplayers");
    assert!(out.contains("Server Operator") && out.contains("Newbie"), "{out}");
    // pause stops level.time
    let t = w.level.time;
    console(&mut w, "pause");
    for _ in 0..100 {
        w.frame(16);
    }
    assert_eq!(w.level.time, t);
    console(&mut w, "pause");
    w.frame(16);
    assert_eq!(w.level.time, t + 16);
    // changemap / restart go to the host's console
    w.level.map_list = vec!["atcs".into()];
    console(&mut w, "changemap atcs");
    console(&mut w, "restart switchteams");
    assert_eq!(svcmds::run_exec(&mut w), vec!["map atcs".to_string(), "map_restart".to_string()]);
    assert_eq!(w.clients[host].restart_team, Team::Aliens);
}

#[test]
fn build_log_and_revert() {
    let mut w = World::new(MapId::Arena);
    let host = join(&mut w, "Host", "aliens", "builder");
    let n0 = buildables(&w).len();
    // build an egg in front of the builder
    w.level.cheats = true;
    client::command(&mut w, host, "build eggpod").unwrap();
    let r = trem_sim::kernel::buildable::build_if_valid(&mut w, host, BuildableId::Eggpod);
    let egg = r.expect("valid spot");
    assert_eq!(buildables(&w).len(), n0 + 1);
    assert_eq!(w.level.build_id, 1);
    assert_eq!(w.ents[egg].built_by, w.clients[host].namelog);
    let out = console(&mut w, "buildlog");
    assert!(out.contains("Egg") && out.contains("built") && out.contains("by Host"), "{out}");
    // destroy an existing structure (team kill), then revert both changes
    let other = (0..w.num_entities).find(|&i| w.ents[i].inuse && w.ents[i].buildable == Some(BuildableId::Booster)).or_else(|| (0..w.num_entities).find(|&i| w.ents[i].inuse && w.ents[i].buildable == Some(BuildableId::Eggpod) && i != egg)).unwrap();
    let kind = w.ents[other].buildable.unwrap();
    let pos = w.ents[other].pos.tr_base;
    trem_sim::kernel::combat::damage(&mut w, other, None, Some(host), None, None, 100000, 0, trem_sim::gen::enums::MeansOfDeath::Suicide);
    assert_eq!(w.level.build_id, 2);
    assert_eq!(w.level.build_log[1].fate, layout::BuildFate::Teamkill);
    for _ in 0..400 {
        w.frame(16);
    }
    assert!(!w.ents[other].inuse || w.ents[other].health <= 0);
    let out = console(&mut w, "revert 64");
    assert!(out.contains("reverted 2 changes"), "{out}");
    assert_eq!(w.level.build_id, 0);
    assert!(!w.ents[egg].inuse || w.ents[egg].buildable != Some(BuildableId::Eggpod) || w.ents[egg].etype != EType::Buildable, "the new egg is gone");
    for _ in 0..20 {
        w.frame(16);
    }
    assert!(buildables(&w).iter().any(|b| b.0 == kind && (b.1[0] - pos[0]).abs() < 1.0 && (b.1[1] - pos[1]).abs() < 1.0), "the destroyed structure is back");
}

#[test]
fn server_console_commands() {
    let mut w = World::new(MapId::Arena);
    let host = join(&mut w, "Host", "humans", "rifle");
    let x = bot::add_bot(&mut w, "Xeno1", Team::Aliens).unwrap();
    assert!(console(&mut w, "entityList").contains("ET_BUILDABLE        "));
    let st = console(&mut w, "status");
    assert!(st.starts_with("slot score ping address") && st.contains("localhost") && st.contains("Xeno1"), "{st}");
    console(&mut w, "forceTeam Xeno1 h");
    assert_eq!(w.clients[x].team, Team::Humans);
    assert!(console(&mut w, "forceTeam Xeno1 q").contains("invalid team"));
    let du = console(&mut w, "dumpuser Host");
    assert!(du.contains("name                Host"), "{du}");
    console(&mut w, "pr -1 hello everyone");
    assert!(w.prints.iter().any(|p| p.3 == "hello everyone" && p.2.is_none()));
    w.chat.clear();
    console(&mut w, "say from the console");
    assert!(w.chat.iter().any(|c| c.1 == host && c.4 == "console: from the console"));
    console(&mut w, "say_team h team only");
    assert!(w.chat.iter().any(|c| c.1 == host && c.4.ends_with("team only")));
    assert!(console(&mut w, "printqueue a").starts_with("l:"));
    assert!(console(&mut w, "eject Host").contains("cannot eject local clients"));
    console(&mut w, "eject Xeno1 bye");
    assert!(!w.clients[x].connected);
    console(&mut w, "admitDefeat a");
    assert_eq!(w.level.surrender_team, Some(Team::Aliens));
    assert!(w.prints.iter().any(|p| p.3 == "Alien team has admitted defeat"));
    assert!(!svcmds::console_command(&mut w, "map_restart"), "an engine command: the host's");
    assert!(svcmds::is_console_command(&w, "layoutsave") && svcmds::is_console_command(&w, "kick"));
}

#[test]
fn client_names_are_cleaned_and_checked() {
    let w = World::new(MapId::Arena);
    assert_eq!(client::clean_name(&w, "   Bob"), "Bob");
    assert_eq!(client::clean_name(&w, "a     b"), "a   b", "at most three spaces in a row");
    assert_eq!(client::clean_name(&w, "^1Red^7Name"), "^1Red^7Name");
    assert_eq!(client::clean_name(&w, "bad\x01`name"), "badname");
    assert_eq!(client::clean_name(&w, "x//y"), "UnnamedPlayer");
    assert_eq!(client::clean_name(&w, "[skipnotify]me"), "UnnamedPlayer");
    assert_eq!(client::clean_name(&w, "^1^2"), "UnnamedPlayer");
    assert_eq!(client::clean_name(&w, &"n".repeat(40)).len(), 31);
    let mut w = World::new(MapId::Arena);
    let a = w.connect("  Alice", false).unwrap();
    assert_eq!(w.clients[a].name, "Alice");
    // duplicates, leading digits and "console" are refused (G_admin_name_check)
    let b = w.connect("alice", false).unwrap();
    assert_eq!(w.clients[b].name, "UnnamedPlayer");
    let c = w.connect("1st", false).unwrap();
    assert_eq!(w.clients[c].name, "UnnamedPlayer");
    let d = w.connect("Console", false).unwrap();
    assert_eq!(w.clients[d].name, "UnnamedPlayer");
    assert_eq!(trem_sim::kernel::cmds::sanitise_string("^1Mr. X-1"), "mrx1");
    assert_eq!(trem_sim::kernel::cmds::decolor_string("^1a^^b^7c"), "a^c");
    assert_eq!(trem_sim::kernel::cmds::client_numbers_from_string(&w, "unnamed", 64), vec![b, c, d]);
}

#[test]
fn inactivity_drops_idle_non_local_clients() {
    let mut w = World::new(MapId::Arena);
    w.set_cvar("g_inactivity", "10");
    let host = join(&mut w, "Host", "humans", "rifle");
    let idle = join(&mut w, "Idle", "humans", "rifle");
    // the host's player is never dropped; the other one is warned then dropped
    for _ in 0..(11000 / 16) {
        w.frame(16);
    }
    assert!(w.center_prints.iter().any(|c| c.2 == Some(idle) && c.3.starts_with("Ten seconds")));
    for _ in 0..(10000 / 16) {
        w.frame(16);
    }
    assert!(!w.clients[idle].connected);
    assert!(w.clients[host].connected);
    // g_inactivity 0 (default): nobody is dropped
    let mut w = World::new(MapId::Arena);
    let _h = join(&mut w, "Host", "humans", "rifle");
    let i2 = join(&mut w, "Idle", "humans", "rifle");
    for _ in 0..(120000 / 16) {
        w.frame(16);
    }
    assert!(w.clients[i2].connected);
}

#[test]
fn votes_on_players_run_admin_commands() {
    let mut w = World::new(MapId::Arena);
    let host = join(&mut w, "Host", "humans", "rifle");
    let x = bot::add_bot(&mut w, "Xeno1", Team::Aliens).unwrap();
    assert!(client::command(&mut w, host, "callvote mute").unwrap_err().contains("no target"));
    client::command(&mut w, host, "callvote mute Xeno1 spam").unwrap();
    client::command(&mut w, x, "vote yes").unwrap();
    for _ in 0..10 {
        w.frame(16);
    }
    assert!(svcmds::run_exec(&mut w).is_empty());
    let n = w.clients[x].namelog.unwrap();
    assert!(w.namelogs[n].muted);
    // the host is immune (ALLFLAGS includes IMMUNITY)
    let y = bot::add_bot(&mut w, "Xeno2", Team::Aliens).unwrap();
    assert!(client::command(&mut w, y, "callvote kick Host because").unwrap_err().contains("immune"));
}

#[test]
fn session_data_survives_a_map_change() {
    let mut w = World::new(MapId::Arena);
    let host = join(&mut w, "Host", "humans", "rifle");
    let b = bot::add_bot(&mut w, "Xeno1", Team::Aliens).unwrap();
    client::command(&mut w, host, "ignore Xeno1").unwrap();
    console(&mut w, "restart keepteams");
    let sess = session::write_session_data(&w);
    assert!(sess[host].is_some() && sess[5].is_none());
    // the next World: the host reconnects in the same slot and gets its team back
    let mut w2 = World::new(MapId::Arena);
    let h2 = w2.connect("Host", false).unwrap();
    let _b2 = bot::add_bot(&mut w2, "Xeno1", Team::Aliens).unwrap();
    assert_eq!(h2, host);
    session::read_session_data(&mut w2, h2, sess[host].as_ref().unwrap());
    assert_eq!(w2.clients[h2].team, Team::Humans);
    assert_eq!(w2.clients[h2].ignored, vec![b]);
    assert_eq!(session::ClientSession::from_cvar(&sess[host].clone().unwrap()).to_cvar(), sess[host].clone().unwrap());
}

/// `BG_LoadEmoticons` (game side): valid `<name>_<width>x1.tga` names only;
/// a loaded emoticon may appear in a name (`G_IsEmoticon`).
#[test]
fn emoticons_load_from_the_file_system() {
    let fs = MemFs::default();
    for f in ["emoticons/smile_1x1.tga", "emoticons/wide_2x1.tga", "emoticons/bad_0x1.tga", "emoticons/x1.tga"] {
        fs.clone().write(f, "tga");
    }
    let mut w = arena_world(&fs, None);
    w.console.clear();
    assert_eq!(client::load_emoticons(&mut w), 2);
    assert_eq!(w.emoticons, vec!["smile".to_string(), "wide".to_string()]);
    assert!(w.console.contains("skipping invalidly named emoticon \"bad_0x1.tga\""), "{}", w.console);
    assert!(w.console.contains("Loaded 2 of 4 emoticons (MAX_EMOTICONS is 64)"), "{}", w.console);
}

#[test]
fn ip_bans_and_namelog_addresses() {
    use trem_sim::kernel::world::ConnectInfo;
    let fs = MemFs::default();
    let mut w = arena_world(&fs, None);
    w.real_time = 1_800_000_000;
    let net = |name: &str, guid: &str, ip: &str| ConnectInfo { name: name.into(), guid: guid.into(), ip: ip.into(), use_unlagged: true, ..Default::default() };
    // G_AddressParse / G_AddressCompare
    let (ok, a) = admin::address_parse("10.1.2.3/16");
    assert!(ok && a.mask == 16 && !a.ipv6);
    assert!(admin::address_compare(&a, &admin::address_parse("10.1.200.9").1));
    assert!(!admin::address_compare(&a, &admin::address_parse("10.2.0.1").1));
    let (ok6, a6) = admin::address_parse("2001:db8::1/32");
    assert!(ok6 && a6.ipv6 && a6.mask == 32);
    assert!(admin::address_compare(&a6, &admin::address_parse("2001:db8:ffff::7").1));
    assert!(!admin::address_parse("localhost").0);
    let alice = w.connect_ex(None, &net("Alice", "0123456789ABCDEF0123456789ABCDEF", "10.0.0.5"), false).unwrap();
    assert_eq!(w.clients[alice].ip.str, "10.0.0.5");
    // namelog keeps the address; namelog searches by IP
    let out = console(&mut w, "namelog 10.0.0.5");
    assert!(out.contains("10.0.0.5") && out.contains("Alice"), "{out}");
    let out = console(&mut w, "namelog 10.9.9.9");
    assert!(!out.contains("Alice"), "{out}");
    // a CIDR ban from the console (ADMF_CAN_IP_BAN: masks 1-32) drops her
    let out = console(&mut w, "ban 10.0.0.0/24 1h spam");
    assert!(out.contains("banned") || w.prints.iter().any(|p| p.3.contains("has been banned")), "{out}");
    assert!(!w.clients[alice].connected, "the ban dropped the matching client");
    assert_eq!(w.admin.bans.len(), 1);
    assert_eq!(w.admin.bans[0].ip.str, "10.0.0.0/24");
    assert!(w.server_commands.iter().any(|(to, c)| *to == Some(alice) && c.starts_with("disconnect \"You have been banned")));
    assert!(fs.read("admin.dat").unwrap().contains("ip      = 10.0.0.0/24\n"));
    // another GUID from the same network is refused, another network is not
    let r = w.connect_ex(None, &net("Mallory", "FEDCBA9876543210FEDCBA9876543210", "10.0.0.77"), false);
    assert!(r.unwrap_err().starts_with("You have been banned by console"));
    let bob = w.connect_ex(None, &net("Bob", "FEDCBA9876543210FEDCBA9876543210", "10.0.1.77"), false).unwrap();
    // showbans by IP (ban_matchip both ways)
    let out = console(&mut w, "showbans 10.0.0.200");
    assert!(out.contains("10.0.0.0/24") && out.contains("spam"), "{out}");
    let out = console(&mut w, "showbans 192.168.0.1");
    assert!(!out.contains("spam"), "{out}");
    // adjustban narrows the mask: /32 removes it from the text
    console(&mut w, "adjustban 1 /32");
    assert_eq!(w.admin.bans[0].ip.str, "10.0.0.0");
    assert_eq!(w.admin.bans[0].ip.mask, 32);
    // a name ban bans every address the player used
    console(&mut w, "ban Bob 1h cheating");
    assert!(!w.clients[bob].connected);
    assert!(w.admin.bans.iter().any(|b| b.ip.str == "10.0.1.77" && b.guid == "FEDCBA9876543210FEDCBA9876543210"));
    // the ban file round-trips the addresses
    let mut w2 = arena_world(&fs, None);
    w2.real_time = 1_800_000_000;
    admin::readconfig(&mut w2, None);
    assert!(w2.admin.bans.iter().any(|b| b.ip.str == "10.0.1.77" && admin::address_compare(&b.ip, &admin::address_parse("10.0.1.77").1)));
}

#[test]
fn userinfo_name_changes_follow_the_rename_rules() {
    let mut w = World::new(MapId::Arena);
    let i = w.connect("Alpha", false).unwrap();
    assert_eq!(w.clients[i].name, "Alpha");
    let msg = |w: &World| w.clients[i].message.as_ref().map(|m| m.1.clone()).unwrap_or_default();
    let advance = |w: &mut World, ms: i32| {
        let end = w.level.time + ms;
        while w.level.time < end {
            w.frame(50);
        }
    };
    advance(&mut w, 1000);
    // CL_CheckUserinfo -> ClientUserinfoChanged
    client::userinfo_changed(&mut w, i, "Beta", false);
    assert_eq!(w.clients[i].name, "Beta");
    let nl = w.clients[i].namelog.unwrap();
    assert!(w.namelogs[nl].names.iter().any(|n| n == "Alpha") && w.namelogs[nl].names.iter().any(|n| n == "Beta"));
    // within g_minNameChangePeriod (5 s): reverted with the C's print
    client::userinfo_changed(&mut w, i, "Gamma", false);
    assert_eq!(w.clients[i].name, "Beta");
    assert_eq!(msg(&w), "Name change spam protection (g_minNameChangePeriod = 5)");
    // the same name (after cleaning) is no change at all
    client::userinfo_changed(&mut w, i, "  Beta", false);
    assert_eq!(w.clients[i].name, "Beta");
    // g_maxNameChanges 5: the sixth change is refused
    for n in ["Gamma", "Delta", "Epsilon", "Zeta"] {
        advance(&mut w, 6000);
        client::userinfo_changed(&mut w, i, n, false);
        assert_eq!(w.clients[i].name, n);
    }
    advance(&mut w, 6000);
    client::userinfo_changed(&mut w, i, "Eta", false);
    assert_eq!(w.clients[i].name, "Zeta");
    assert_eq!(msg(&w), "Maximum name changes reached (g_maxNameChanges = 5)");
    // an admin rename (forceName) is not limited
    client::userinfo_changed(&mut w, i, "Theta", true);
    assert_eq!(w.clients[i].name, "Theta");
    // muted players keep their name
    let mut w = World::new(MapId::Arena);
    let j = w.connect("Quiet", false).unwrap();
    advance(&mut w, 1000);
    let nl = w.clients[j].namelog.unwrap();
    w.namelogs[nl].muted = true;
    client::userinfo_changed(&mut w, j, "Loud", false);
    assert_eq!(w.clients[j].name, "Quiet");
    assert_eq!(w.clients[j].message.as_ref().unwrap().1, "You cannot change your name while you are muted");
}
