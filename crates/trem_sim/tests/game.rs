//! Game-rule tests over the full World (client/combat/weapon/buildable/team
//! kernels). These are behavioural checks against the original rules, not
//! golden traces: g_*.c has no standalone oracle (see dec014).

use sha2::{Digest, Sha256};
use trem_sim::gen::buildables::BuildableId;
use trem_sim::gen::classes::ClassId;
use trem_sim::gen::enums::{SpectatorState, Stat, Team};
use trem_sim::gen::maps::MapId;
use trem_sim::gen::upgrades::UpgradeId;
use trem_sim::gen::weapons::WeaponId;
use trem_sim::kernel::{bg, bot, client, world::World};

fn run_until_spawned(w: &mut World, i: usize) {
    for _ in 0..200 {
        w.frame(16);
        if w.clients[i].spectator == SpectatorState::Not as i32 {
            return;
        }
    }
    panic!("client {i} never spawned");
}

fn player(w: &mut World, team: &str, class: &str) -> usize {
    let i = w.connect("player", false).unwrap();
    client::command(w, i, &format!("team {team}")).unwrap();
    client::command(w, i, &format!("class {class}")).unwrap();
    run_until_spawned(w, i);
    i
}

fn teleport(w: &mut World, i: usize, o: [f32; 3]) {
    w.clients[i].ps.origin = o;
    w.clients[i].ps.velocity = [0.0; 3];
    w.ents[i].origin = o;
}

fn state_hash(w: &World) -> String {
    let mut h = Sha256::new();
    for c in w.clients.iter().filter(|c| c.connected) {
        h.update(format!("{:?}{:?}{}{}{}", c.ps.origin, c.ps.stats, c.credit, c.score, c.kills));
    }
    for e in w.ents.iter().filter(|e| e.inuse) {
        h.update(format!("{:?}{}{:?}", e.origin, e.health, e.buildable));
    }
    hex(&h.finalize())
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn map_buildables_spawn_and_count() {
    let mut w = World::new(MapId::Arena);
    w.frame(16);
    assert_eq!(w.level.num_alien_spawns, 3);
    assert_eq!(w.level.num_human_spawns, 3);
    assert!(w.overmind().is_some() && w.reactor().is_some());
}

#[test]
fn spawn_queue_puts_players_on_spawns() {
    let mut w = World::new(MapId::Arena);
    let h = player(&mut w, "humans", "rifle");
    assert_eq!(w.clients[h].ps.stats[Stat::Class as usize], ClassId::HumanBase.num() as i32);
    assert_eq!(w.clients[h].ps.weapon, WeaponId::Rifle as i32);
    assert!(bg::inventory_contains_upgrade(UpgradeId::Medkit.num() as i32, &w.clients[h].ps.stats));
    assert_eq!(w.ents[h].health, 100);
    let a = player(&mut w, "aliens", "level0");
    assert_eq!(w.clients[a].ps.stats[Stat::Class as usize], ClassId::Level0.num() as i32);
    assert_eq!(w.ents[a].health, ClassId::Level0.def().health);
    assert!(w.clients[a].ps.origin[0] < 0.0 && w.clients[h].ps.origin[0] > 0.0);
}

#[test]
fn humans_buy_and_sell_at_the_armoury() {
    let mut w = World::new(MapId::Arena);
    let h = player(&mut w, "humans", "rifle");
    assert!(client::command(&mut w, h, "buy shotgun").is_err(), "far from armoury");
    let arm = w.find_buildable(BuildableId::Arm).unwrap();
    let mut o = w.ents[arm].origin;
    o[0] -= 60.0;
    o[2] += 20.0;
    teleport(&mut w, h, o);
    w.frame(16);
    assert!(client::command(&mut w, h, "buy shotgun").is_err(), "no credits");
    trem_sim::kernel::combat::add_credit(&mut w, h, 400, false);
    client::command(&mut w, h, "sell weapon").unwrap();
    client::command(&mut w, h, "buy shotgun").unwrap();
    assert_eq!(w.clients[h].ps.stats[Stat::Weapon as usize], WeaponId::Shotgun as i32);
    assert_eq!(w.clients[h].credit, 400 - WeaponId::Shotgun.def().price);
    assert!(client::command(&mut w, h, "buy helmet").is_err(), "helmet is stage 2");
    client::command(&mut w, h, "buy larmour").unwrap();
    assert!(bg::inventory_contains_upgrade(UpgradeId::Larmour.num() as i32, &w.clients[h].ps.stats));
    client::command(&mut w, h, "sell larmour").unwrap();
    assert_eq!(w.clients[h].credit, 400 - WeaponId::Shotgun.def().price);
    for _ in 0..60 {
        w.frame(16);
    }
    assert_eq!(w.clients[h].ps.weapon, WeaponId::Shotgun as i32, "PM_Weapon raised the new weapon");
}

#[test]
fn aliens_evolve_with_enough_credits() {
    let mut w = World::new(MapId::Arena);
    let a = player(&mut w, "aliens", "level0");
    assert!(client::command(&mut w, a, "class level1").is_err());
    let cost = ClassId::Level1.def().cost * trem_sim::gen::constants::ALIEN_CREDITS_PER_KILL;
    trem_sim::kernel::combat::add_credit(&mut w, a, cost, true);
    for _ in 0..30 {
        w.frame(16);
    }
    client::command(&mut w, a, "class level1").unwrap();
    assert_eq!(w.clients[a].ps.stats[Stat::Class as usize], ClassId::Level1.num() as i32);
    assert_eq!(w.clients[a].credit, 0);
    assert_eq!(w.clients[a].ps.weapon, WeaponId::Level1 as i32);
}

#[test]
fn builders_place_structures_with_build_points() {
    let mut w = World::new(MapId::Arena);
    let b = player(&mut w, "aliens", "builder");
    for _ in 0..60 {
        w.frame(16);
    }
    let before = (64..w.num_entities).filter(|&e| w.ents[e].inuse && w.ents[e].buildable == Some(BuildableId::Barricade)).count();
    // face away from the overmind, onto open creep
    teleport(&mut w, b, [-950.0, -60.0, 40.0]);
    w.frame(16);
    client::command(&mut w, b, "build barricade").unwrap();
    let mut placed = false;
    for f in 0..120 {
        w.clients[b].cmd.buttons = if f % 2 == 0 { trem_sim::gen::constants::BUTTON_ATTACK } else { 0 };
        w.frame(16);
        let n = (64..w.num_entities).filter(|&e| w.ents[e].inuse && w.ents[e].buildable == Some(BuildableId::Barricade)).count();
        if n > before {
            placed = true;
            break;
        }
    }
    assert!(placed, "barricade built: {:?}", w.clients[b].message);
    assert!(w.clients[b].ps.stats[Stat::Misc as usize] > 0, "build timer started");
}

#[test]
fn rifle_fire_damages_and_kills() {
    let mut w = World::new(MapId::Arena);
    let h = player(&mut w, "humans", "rifle");
    let a = player(&mut w, "aliens", "level0");
    teleport(&mut w, h, [-300.0, -300.0, 40.0]);
    teleport(&mut w, a, [-100.0, -300.0, 40.0]);
    for _ in 0..40 {
        teleport(&mut w, a, [-100.0, -300.0, 40.0]);
        w.frame(16);
    }
    // aim the human at the dretch's centre
    let eye = bg::client_view_origin(&w.clients[h].ps);
    let dir = trem_sim::kernel::qmath::sub(&w.ents[a].origin, &eye);
    let ang = trem_sim::kernel::qmath::vectoangles(&dir);
    client::set_view_angle(&mut w, h, ang);
    let start = w.ents[a].health;
    let mut killed = false;
    for _ in 0..400 {
        w.clients[h].cmd.buttons = trem_sim::gen::constants::BUTTON_ATTACK;
        teleport(&mut w, a, [-100.0, -300.0, 40.0]);
        w.frame(16);
        if w.clients[a].ps.stats[Stat::Health as usize] <= 0 || w.clients[a].deaths > 0 {
            killed = true;
            break;
        }
    }
    assert!(killed, "dretch died (hp {} of {start})", w.ents[a].health);
    assert_eq!(w.clients[h].kills, 1);
    assert!(w.clients[h].credit > 0, "kill paid credits");
}

#[test]
fn bot_match_is_deterministic_and_plays() {
    let run = || {
        let mut w = World::new(MapId::Arena);
        for k in 0..3 {
            bot::add_bot(&mut w, &format!("a{k}"), Team::Aliens);
            bot::add_bot(&mut w, &format!("h{k}"), Team::Humans);
        }
        let mut deaths = 0;
        for _ in 0..(45_000 / 16) {
            w.frame(16);
            deaths += w.drain_events().iter().filter(|e| e.event == trem_sim::gen::enums::Event::Obituary).count();
            if w.level.intermission_time != 0 {
                break;
            }
        }
        (state_hash(&w), deaths, w.level.time)
    };
    let (h1, d1, t1) = run();
    let (h2, d2, t2) = run();
    assert_eq!((h1, d1, t1), (h2, d2, t2));
    assert!(d1 > 0, "bots fought");
}

/// G_CalculateBuildPoints: what is left is the pool minus every standing
/// structure of the team (reactor-zone structures for humans).
#[test]
fn build_points_account_for_standing_structures() {
    let mut w = World::new(MapId::Arena);
    for _ in 0..10 {
        w.frame(16);
    }
    let cost = |t: Team| -> i32 { (64..w.num_entities).filter(|&e| w.ents[e].inuse && w.ents[e].team == t && w.ents[e].buildable.is_some() && w.ents[e].health > 0).map(|e| w.ents[e].buildable.unwrap().def().build_points).sum() };
    assert_eq!(w.level.alien_bp_left, (w.level.alien_bp - cost(Team::Aliens)).max(0));
    assert!(w.level.human_bp_left <= w.level.human_bp);
}

/// Cmd_Destroy with g_markDeconstruct: "deconstruct" marks, again unmarks,
/// "destroy" blows it up.
#[test]
fn deconstruct_marks_and_unmarks() {
    let mut w = World::new(MapId::Arena);
    let b = player(&mut w, "aliens", "builder");
    for _ in 0..60 {
        w.frame(16);
    }
    // stand in front of an acid tube / egg and look at it
    let target = (64..w.num_entities).find(|&e| w.ents[e].inuse && w.ents[e].team == Team::Aliens && matches!(w.ents[e].buildable, Some(BuildableId::AcidTube) | Some(BuildableId::Eggpod))).unwrap();
    let to = w.ents[target].origin;
    let from = [to[0] - 60.0, to[1], to[2] + 10.0];
    teleport(&mut w, b, from);
    w.clients[b].ps.viewangles = [0.0, 0.0, 0.0];
    w.clients[b].ps.stats[Stat::Misc as usize] = 0;
    client::command(&mut w, b, "deconstruct").unwrap();
    assert!(w.ents[target].deconstruct, "marked");
    assert!(w.ents[target].inuse && w.ents[target].health > 0);
    let marked = trem_sim::kernel::buildable::get_marked_build_points(&w, from, Team::Aliens);
    assert_eq!(marked, w.ents[target].buildable.unwrap().def().build_points);
    client::command(&mut w, b, "deconstruct").unwrap();
    assert!(!w.ents[target].deconstruct, "unmarked");
    // destroy is CMD_CHEAT
    assert!(client::command(&mut w, b, "destroy").is_err());
    w.level.cheats = true;
    client::command(&mut w, b, "destroy").unwrap();
    assert!(w.ents[target].health <= 0, "destroyed");
}

#[test]
fn spectators_follow_and_cycle_players() {
    let mut w = World::new(MapId::Arena);
    let a = player(&mut w, "humans", "rifle");
    let b = player(&mut w, "humans", "rifle");
    let o = w.clients[a].ps.origin;
    teleport(&mut w, b, [o[0] + 64.0, o[1], o[2]]);
    let s = w.connect("spec", false).unwrap();
    w.frame(16);
    assert_eq!(w.clients[s].spectator, SpectatorState::Free as i32);
    // follow with no target picks the first followable client
    client::command(&mut w, s, "follow").unwrap();
    assert_eq!(w.clients[s].spectator, SpectatorState::Follow as i32);
    let first = w.clients[s].spectator_client as usize;
    assert!(first == a || first == b);
    w.frame(16);
    // SpectatorClientEndFrame: our view is the followed player's state
    assert_ne!(w.clients[s].ps.pm_flags & trem_sim::kernel::types::k::PMF_FOLLOW, 0);
    assert_eq!(w.clients[s].ps.client_num, first as i32);
    assert_eq!(w.clients[s].ps.origin, w.clients[first].ps.origin);
    // cycling moves to the other player and back
    client::command(&mut w, s, "follownext").unwrap();
    let second = w.clients[s].spectator_client as usize;
    assert_ne!(second, first);
    client::command(&mut w, s, "followprev").unwrap();
    assert_eq!(w.clients[s].spectator_client as usize, first);
    // followed player leaves the team: the follower moves on to the other one
    // (Cmd_Team_f: 30 s lock after playing unless cheats are on)
    w.level.cheats = true;
    for _ in 0..70 {
        w.frame(16);
    }
    client::command(&mut w, first, "team spectate").unwrap();
    assert_eq!(w.clients[s].spectator_client as usize, second);
    // toggling stops following and gives our own view back
    client::command(&mut w, s, "follow").unwrap();
    w.frame(16);
    assert_eq!(w.clients[s].spectator, SpectatorState::Free as i32);
    assert_eq!(w.clients[s].spectator_client, -1);
    assert_eq!(w.clients[s].ps.client_num, s as i32);
    assert_eq!(w.clients[s].ps.pm_flags & trem_sim::kernel::types::k::PMF_FOLLOW, 0);
}

#[test]
fn win_goes_to_intermission_then_exits_when_ready() {
    let mut w = World::new(MapId::Arena);
    let a = player(&mut w, "humans", "rifle");
    let b = w.connect("bot", true).unwrap();
    client::command(&mut w, b, "team aliens").unwrap();
    w.level.uncond_human_win = true;
    w.frame(16);
    assert!(w.level.intermission_queued != 0 && w.level.winner == Some(Team::Humans));
    for _ in 0..(10000 / 16) {
        w.frame(16);
        if w.level.intermission_time != 0 {
            break;
        }
    }
    assert!(w.level.intermission_time != 0, "intermission began");
    assert_eq!(w.clients[a].ps.origin, w.level.intermission_origin);
    assert_eq!(w.clients[a].ps.pm_type, trem_sim::gen::enums::PmType::Intermission as i32);
    // not before five seconds, even when everyone is ready
    w.clients[a].cmd.buttons = trem_sim::kernel::types::k::BUTTON_ATTACK;
    for _ in 0..10 {
        w.clients[a].cmd.server_time = w.level.time + 16;
        w.frame(16);
    }
    assert!(w.clients[a].ready_to_exit);
    assert!(!w.level.exited);
    for _ in 0..(6000 / 16) {
        w.frame(16);
    }
    assert!(w.level.exited, "everyone ready after 5s -> ExitLevel");
}

#[test]
fn client_commands_are_gated_by_their_flags() {
    let mut w = World::new(MapId::Arena);
    let s = w.connect("spec", false).unwrap();
    // the host's player is the top admin (dec026): ALLFLAGS includes
    // SPECALLCHAT; drop it to level 0 for the team chat checks below
    assert!(trem_sim::kernel::svcmds::console_command(&mut w, &format!("setlevel {s} 0")));
    // CMD_TEAM / CMD_HUMAN / CMD_SPEC / CMD_CHEAT
    assert!(client::command(&mut w, s, "kill").is_err());
    assert!(client::command(&mut w, s, "buy rifle").is_err());
    assert!(client::command(&mut w, s, "god").is_err());
    assert!(client::command(&mut w, s, "nosuchcommand").is_err());
    // noclip is CMD_CHEAT_TEAM: fine for spectators without cheats
    client::command(&mut w, s, "noclip").unwrap();
    assert!(w.clients[s].noclip);
    let h = player(&mut w, "humans", "rifle");
    assert!(client::command(&mut w, h, "follow").is_err(), "follow is CMD_SPEC");
    assert!(client::command(&mut w, h, "give all").is_err());
    w.level.cheats = true;
    client::command(&mut w, h, "god").unwrap();
    let hp = w.ents[h].health;
    trem_sim::kernel::combat::damage(&mut w, h, None, None, None, None, 50, 0, trem_sim::gen::enums::MeansOfDeath::TargetLaser);
    assert_eq!(w.ents[h].health, hp, "godmode");
    client::command(&mut w, h, "give funds 100").unwrap();
    assert!(w.clients[h].credit >= 100);
    // chat: say reaches everyone, say_team only team mates (spectators are a team)
    client::command(&mut w, h, "say hello all").unwrap();
    assert!(w.chat.iter().any(|c| c.1 == s && c.4.ends_with("hello all")));
    w.chat.clear();
    client::command(&mut w, h, "say_team humans only").unwrap();
    assert!(w.chat.iter().all(|c| c.1 == h));
    client::command(&mut w, s, "ignore player").unwrap();
    w.chat.clear();
    client::command(&mut w, h, "say again").unwrap();
    assert!(!w.chat.iter().any(|c| c.1 == s), "ignored");
}

#[test]
fn team_auto_balances_and_spam_is_ignored() {
    let mut w = World::new(MapId::Arena);
    let _h = player(&mut w, "humans", "rifle");
    let a = w.connect("auto", false).unwrap();
    client::command(&mut w, a, "team auto").unwrap();
    assert_eq!(w.clients[a].team, Team::Aliens, "auto joins the smaller team");
    // a second change within a second is ignored (team join spam)
    client::command(&mut w, a, "team humans").unwrap();
    assert_eq!(w.clients[a].team, Team::Aliens);
    for _ in 0..70 {
        w.frame(16);
    }
    client::command(&mut w, a, "team humans").unwrap();
    assert_eq!(w.clients[a].team, Team::Humans);
}

#[test]
fn warmup_holds_spawning_until_it_is_over() {
    // g_doWarmup 0 (the default): CS_WARMUP "-1", spawning at once
    let mut w = World::new(MapId::Arena);
    trem_sim::kernel::team::init_warmup(&mut w);
    assert_eq!(w.level.cs_warmup, -1);
    assert!(trem_sim::kernel::team::warmup_over(&w));
    // g_doWarmup 1, g_warmup 3: nobody spawns for three seconds
    let mut w = World::new(MapId::Arena);
    assert!(w.set_cvar("g_doWarmup", "1"));
    assert!(w.set_cvar("g_warmup", "3"));
    trem_sim::kernel::team::init_warmup(&mut w);
    assert_eq!(w.level.cs_warmup, w.level.time + 3000);
    let i = w.connect("player", false).unwrap();
    client::command(&mut w, i, "team humans").unwrap();
    client::command(&mut w, i, "class rifle").unwrap();
    while w.level.time < w.level.warmup_time - 16 {
        w.frame(16);
        assert_ne!(w.clients[i].spectator, SpectatorState::Not as i32, "spawned during the warmup at {}", w.level.time);
    }
    run_until_spawned(&mut w, i);
    assert!(w.level.time >= w.level.warmup_time);
}

#[test]
fn disabled_game_elements_are_refused() {
    let mut w = World::new(MapId::Arena);
    // SP_worldspawn's disabledEquipment / disabledClasses / disabledBuildables
    let warnings = bg::init_allowed_game_elements("shotgun, larmour,bogus", " level1", "mgturret");
    assert_eq!(warnings, vec!["^3WARNING: unknown equipment bogus".to_string()]);
    assert!(!bg::weapon_is_allowed(WeaponId::Shotgun) && bg::weapon_is_allowed(WeaponId::Rifle));
    assert!(!bg::upgrade_is_allowed(UpgradeId::Larmour) && bg::upgrade_is_allowed(UpgradeId::Helmet));
    assert!(!bg::class_is_allowed(ClassId::Level1) && bg::class_is_allowed(ClassId::Level0));
    assert!(!bg::buildable_is_allowed(BuildableId::Mgturret) && bg::buildable_is_allowed(BuildableId::Telenode));
    // evolution routes through a disabled class are closed (BG_ClassCanEvolveFromTo)
    assert_eq!(bg::class_can_evolve_from_to(ClassId::Level0, ClassId::Level1, 10_000, 2, 0), -1);
    assert!(!bg::alien_can_evolve(ClassId::Level0, 10_000, 0) || ClassId::Level0.def().evolves_to.iter().any(|&c| c != ClassId::Level1 && bg::class_allowed_in_stage(c, 0)));
    let h = player(&mut w, "humans", "rifle");
    let arm = w.find_buildable(BuildableId::Arm).unwrap();
    let mut o = w.ents[arm].origin;
    o[0] -= 60.0;
    o[2] += 20.0;
    teleport(&mut w, h, o);
    w.frame(16);
    trem_sim::kernel::combat::add_credit(&mut w, h, 400, false);
    client::command(&mut w, h, "sell weapon").unwrap();
    assert_eq!(client::command(&mut w, h, "buy shotgun").unwrap_err(), "You can't buy this item");
    assert_eq!(client::command(&mut w, h, "buy larmour").unwrap_err(), "You can't buy this item");
    // a new game starts with nothing disabled
    let w2 = World::new(MapId::Arena);
    drop(w2);
    assert!(bg::weapon_is_allowed(WeaponId::Shotgun));
}

/// buildFire with nothing selected: G_TriggerMenu(MN_A_BUILD / MN_H_BUILD)
/// asks the presentation for the build menu (granger and construction kit),
/// then `build <name>` + fire places the structure (human side: a medistat
/// in the reactor's power zone).
#[test]
fn builders_open_the_build_menu_and_build() {
    use trem_sim::kernel::weapon::BUILD_MENU_NOTICE;
    let mut w = World::new(MapId::Arena);
    let a = player(&mut w, "aliens", "builder");
    let h = player(&mut w, "humans", "ckit");
    for _ in 0..60 {
        w.frame(16);
    }
    for &p in &[a, h] {
        w.clients[p].wants_menu = false;
        w.clients[p].message = None;
        w.clients[p].cmd.buttons = trem_sim::gen::constants::BUTTON_ATTACK;
        w.frame(16);
        w.clients[p].cmd.buttons = 0;
        w.frame(16);
        assert!(w.clients[p].wants_menu, "fire with no blueprint asks for the build menu");
        assert_eq!(w.clients[p].message.as_ref().map(|m| m.1.as_str()), Some(BUILD_MENU_NOTICE));
    }
    // the human builds next to the reactor
    let reactor = (64..w.num_entities).find(|&e| w.ents[e].inuse && w.ents[e].buildable == Some(BuildableId::Reactor)).expect("arena has a reactor");
    let ro = w.ents[reactor].origin;
    let count = |w: &World| (64..w.num_entities).filter(|&e| w.ents[e].inuse && w.ents[e].buildable == Some(BuildableId::Medistat)).count();
    let before = count(&w);
    let mut placed = false;
    'outer: for (dx, dy) in [(200.0, 0.0), (-200.0, 0.0), (0.0, 200.0), (0.0, -200.0)] {
        teleport(&mut w, h, [ro[0] + dx, ro[1] + dy, ro[2] + 20.0]);
        let yaw = if dx > 0.0 { 0.0 } else if dx < 0.0 { 180.0 } else if dy > 0.0 { 90.0 } else { -90.0 };
        client::set_view_angle(&mut w, h, [0.0, yaw, 0.0]);
        w.frame(16);
        w.clients[h].ps.stats[Stat::Misc as usize] = 0;
        if client::command(&mut w, h, "build medistat").is_err() {
            continue;
        }
        for f in 0..60 {
            w.clients[h].cmd.buttons = if f % 2 == 0 { trem_sim::gen::constants::BUTTON_ATTACK } else { 0 };
            w.frame(16);
            if count(&w) > before {
                placed = true;
                break 'outer;
            }
        }
    }
    assert!(placed, "medistat built: {:?}", w.clients[h].message);
}
