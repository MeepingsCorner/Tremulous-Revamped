//! Kernel `client` — port of game/g_active.c (ClientThink_real, timers,
//! events, impacts, end frame), the spawning half of game/g_client.c
//! (ClientSpawn, respawn, corpses, spawn point selection), the spawn queues
//! of g_main.c, G_LeaveTeam / G_ChangeTeam (g_team.c) and the gameplay
//! commands of g_cmds.c (class, buy, sell, build, itemact/toggle, kill,
//! deconstruct). G_TriggerMenu becomes `Client::message`; the unlagged hooks of
//! ClientThink_real call unlagged.rs (dec033); votes and admin live in
//! their own modules.

use super::bg;
use super::combat::{self, DAMAGE_NO_LOCDAMAGE};
use super::pmove::Pmove;
use super::qmath::*;
use super::types::*;
use super::world::{Client, EType, World, MAX_CLIENTS};
use crate::gen::buildables::BuildableId;
use crate::gen::classes::ClassId;
use crate::gen::enums::{contents, Event, MeansOfDeath as Mod, PlayerAnim, NonSegAnim, Pers, PmType, SpectatorState, Stat, Team, WeaponMode, WeaponState};
use crate::gen::upgrades::UpgradeId;
use crate::gen::weapons::WeaponId;
use std::collections::VecDeque;

const USE_OBJECT_RANGE: f32 = 64.0;
const SHOVE: f32 = 0.0; // g_shove default
const UP_VEL: f32 = 150.0;
const F_VEL: f32 = 50.0;

#[inline]
fn st(w: &World, i: usize, s: Stat) -> i32 {
    w.clients[i].ps.stats[s as usize]
}

/// G_TriggerMenu / print: queue a line for the presentation layer.
pub fn notify(w: &mut World, i: usize, text: impl Into<String>) {
    let t = w.level.time;
    w.clients[i].message = Some((t, text.into()));
}

// ===================================================================== frame

/// `client_think` system: `G_RunClient` for every connected client whose
/// commands are synchronous (bots and the single-process player: each
/// command runs to level.time). Networked clients run `client_think` as
/// their usercmds arrive (dec033).
pub fn think_all(w: &mut World) {
    for i in 0..MAX_CLIENTS {
        if !w.clients[i].connected || w.clients[i].async_cmds {
            continue;
        }
        w.clients[i].cmd.server_time = w.level.time;
        w.clients[i].last_cmd_time = w.level.time;
        client_think_real(w, i);
    }
}

/// `ClientThink`: a usercmd from the server (`SV_ClientThink`). Marks the
/// time it arrived (EF_CONNECTION) and runs `ClientThink_real` at once
/// (g_synchronousClients 0).
pub fn client_think(w: &mut World, i: usize, cmd: UserCmd) {
    if !w.clients[i].connected {
        return;
    }
    w.clients[i].cmd = cmd;
    // mark the time we got info, so we can display the phone jack if we
    // don't get any for a while
    w.clients[i].last_cmd_time = w.level.time;
    client_think_real(w, i);
}

/// `G_ClientIsLagging`: the server's ping estimate reached 999.
pub fn client_is_lagging(w: &World, i: usize) -> bool {
    w.clients[i].ps.ping >= 999
}

/// `BG_PlayerStateToEntityState` (the fields the game layer reads back).
pub fn ps_to_es(w: &mut World, i: usize, snap: bool) {
    let ps = w.clients[i].ps;
    let e = &mut w.ents[i];
    e.etype = if ps.pm_type == PmType::Intermission as i32 || ps.pm_type == PmType::Spectator as i32 || ps.pm_type == PmType::Freeze as i32 && e.etype != EType::Player {
        EType::General
    } else {
        EType::Player
    };
    let mut o = ps.origin;
    if snap {
        for c in o.iter_mut() {
            *c = (*c as f64).round_ties_even() as f32;
        }
    }
    e.pos.tr_base = o;
    e.pos.tr_delta = ps.velocity;
    e.angles = ps.viewangles;
    e.angles2 = [0.0; 3];
    e.e_flags = ps.e_flags;
    if ps.stats[Stat::Health as usize] <= 0 {
        e.e_flags |= k::EF_DEAD;
    } else {
        e.e_flags &= !k::EF_DEAD;
    }
    e.weapon = ps.weapon;
    e.generic1 = ps.generic1;
    e.legs_anim = ps.legs_anim;
    e.torso_anim = ps.torso_anim;
    e.ground_entity = ps.ground_entity_num;
    e.origin2 = ps.grapple_point;
    e.loop_sound = ps.loop_sound;
}

/// `ClientThink_real`.
pub fn client_think_real(w: &mut World, i: usize) {
    let t = w.level.time;
    {
        let c = &mut w.clients[i];
        if c.cmd.server_time > t + 200 {
            c.cmd.server_time = t + 200;
        }
        if c.cmd.server_time < t - 1000 {
            c.cmd.server_time = t - 1000;
        }
    }
    let mut msec = w.clients[i].cmd.server_time - w.clients[i].ps.command_time;
    if msec < 1 && w.clients[i].spectator != SpectatorState::Follow as i32 {
        return;
    }
    if msec > 200 {
        msec = 200;
    }
    w.clients[i].unlagged.time = w.clients[i].cmd.server_time;
    if w.level.intermission_time != 0 {
        // ClientIntermissionThink
        let c = &mut w.clients[i];
        c.ps.e_flags &= !(k::EF_FIRING | k::EF_FIRING2);
        c.oldbuttons = c.buttons;
        c.buttons = c.cmd.buttons;
        if c.buttons & (k::BUTTON_ATTACK | k::BUTTON_USE_HOLDABLE) & (c.oldbuttons ^ c.buttons) != 0 {
            c.ready_to_exit = true;
        }
        return;
    }
    if w.clients[i].spectator != SpectatorState::Not as i32 {
        if w.clients[i].spectator == SpectatorState::Scoreboard as i32 {
            return;
        }
        spectator_think(w, i);
        return;
    }
    super::admin::namelog_update_score(w, i);
    // check for inactivity timer, but never drop the local client
    if !inactivity_timer(w, i) {
        return;
    }

    // calculate where ent is currently seeing all the other active clients
    let ut = w.clients[i].unlagged.time;
    super::unlagged::calc(w, ut, i);

    let has_up = |w: &World, u: UpgradeId| bg::inventory_contains_upgrade(u.num() as i32, &w.clients[i].ps.stats);
    let up_active = |w: &World, u: UpgradeId| bg::upgrade_is_active(u.num() as i32, &w.clients[i].ps.stats);

    // pm_type
    {
        let jet = has_up(w, UpgradeId::Jetpack) && up_active(w, UpgradeId::Jetpack);
        let c = &mut w.clients[i];
        let s = c.ps.stats[Stat::State as usize];
        c.ps.pm_type = if c.noclip {
            PmType::Noclip
        } else if c.ps.stats[Stat::Health as usize] <= 0 {
            PmType::Dead
        } else if s & (k::SS_BLOBLOCKED | k::SS_GRABBED) != 0 {
            PmType::Grabbed
        } else if jet {
            PmType::Jetpack
        } else {
            PmType::Normal
        } as i32;
        let st_ = &mut c.ps.stats[Stat::State as usize];
        if *st_ & k::SS_GRABBED != 0 && c.grab_expiry_time < t {
            *st_ &= !k::SS_GRABBED;
        }
        if *st_ & k::SS_BLOBLOCKED != 0 && c.last_lock_time + k::LOCKBLOB_LOCKTIME < t {
            *st_ &= !k::SS_BLOBLOCKED;
        }
        if *st_ & k::SS_SLOWLOCKED != 0 && c.last_slow_time + k::ABUILDER_BLOB_TIME < t {
            *st_ &= !k::SS_SLOWLOCKED;
        }
        *st_ &= !k::SS_BOOSTEDWARNING;
        if *st_ & k::SS_BOOSTED != 0 {
            if t - c.boosted_time >= k::BOOST_TIME {
                *st_ &= !k::SS_BOOSTED;
            } else if t - c.boosted_time >= k::BOOST_WARN_TIME {
                *st_ |= k::SS_BOOSTEDWARNING;
            }
        }
    }
    if w.clients[i].ps.e_flags & k::EF_POISONCLOUDED != 0 && poison_cloud_time(&w.clients[i].ps) - t + w.clients[i].last_poison_clouded_time <= 0 {
        w.clients[i].ps.e_flags &= !k::EF_POISONCLOUDED;
    }
    {
        let c = &mut w.clients[i];
        if c.ps.stats[Stat::State as usize] & k::SS_POISONED != 0 && c.last_poison_time + k::ALIEN_POISON_TIME < t {
            c.ps.stats[Stat::State as usize] &= !k::SS_POISONED;
        }
        c.ps.gravity = w.level.g_gravity as i32;
    }

    // medkit
    if has_up(w, UpgradeId::Medkit) && up_active(w, UpgradeId::Medkit) {
        let mk = UpgradeId::Medkit.num() as i32;
        let c = &mut w.clients[i];
        let s = c.ps.stats[Stat::State as usize];
        if s & k::SS_HEALING_2X != 0 || (c.ps.stats[Stat::Health as usize] == c.ps.stats[Stat::MaxHealth as usize] && s & k::SS_POISONED == 0) {
            bg::deactivate_upgrade(mk, &mut c.ps.stats);
        } else if c.ps.stats[Stat::Health as usize] > 0 {
            bg::deactivate_upgrade(mk, &mut c.ps.stats);
            bg::remove_upgrade(mk, &mut c.ps.stats);
            c.ps.stats[Stat::State as usize] &= !k::SS_POISONED;
            c.poison_immunity_time = t + k::MEDKIT_POISON_IMMUNITY_TIME;
            c.ps.stats[Stat::State as usize] |= k::SS_HEALING_2X;
            c.last_medkit_time = t;
            c.medkit_health_to_restore = c.ps.stats[Stat::MaxHealth as usize] - c.ps.stats[Stat::Health as usize];
            c.medkit_increment_time = t + k::MEDKIT_STARTUP_TIME / k::MEDKIT_STARTUP_SPEED;
            w.add_event(i, Event::MedkitUsed, 0);
        }
    }

    regenerate(w, i);

    // grenade: "M-M-M-M-MONSTER HACK"
    if has_up(w, UpgradeId::Gren) && up_active(w, UpgradeId::Gren) {
        let g = UpgradeId::Gren.num() as i32;
        bg::deactivate_upgrade(g, &mut w.clients[i].ps.stats);
        bg::remove_upgrade(g, &mut w.clients[i].ps.stats);
        let last = w.ents[i].weapon;
        w.ents[i].weapon = WeaponId::Grenade.num() as i32;
        super::weapon::fire_weapon(w, i, 1);
        w.ents[i].weapon = last;
    }

    {
        let gs = w.level.g_speed;
        let c = &mut w.clients[i];
        c.ps.speed = if c.ps.pm_type == PmType::Noclip as i32 { 600 } else { (gs * bg::class(c.ps.stats[Stat::Class as usize]).speed) as i32 };
        if c.last_creep_slow_time + k::CREEP_TIMEOUT < t {
            c.ps.stats[Stat::State as usize] &= !k::SS_CREEPSLOWED;
        }
    }
    if has_up(w, UpgradeId::Jetpack) && up_active(w, UpgradeId::Jetpack) {
        if w.ents[i].last_damage_time + k::JETPACK_DISABLE_TIME > t && w.rng.random() > k::JETPACK_DISABLE_CHANCE {
            w.clients[i].ps.pm_type = PmType::Normal as i32;
        }
        if w.reactor().is_none() {
            bg::deactivate_upgrade(UpgradeId::Jetpack.num() as i32, &mut w.clients[i].ps.stats);
        }
    }

    // ---------------------------------------------------------------- Pmove
    let old_seq = w.clients[i].ps.event_sequence;
    w.clients[i].pmext.fall_velocity = 0.0;
    let mut ps = w.clients[i].ps;
    let mut pmext = w.clients[i].pmext;
    let mut rng = std::mem::take(&mut w.rng);
    let cmd = w.clients[i].cmd;
    let (mins, maxs, numtouch, touch);
    w.clients[i].unlagged.old_origin = w.clients[i].ps.origin;
    // moved from after Pmove (as in the original)
    if !w.clients[i].noclip {
        w.rng = rng;
        super::trigger::touch_triggers(w, i);
        rng = std::mem::take(&mut w.rng);
        ps = w.clients[i].ps;
    }
    let (waterlevel, watertype);
    {
        let mut pm = Pmove::new(&mut ps, &mut pmext, cmd, &*w, &mut rng);
        pm.tracemask = if pm.ps.pm_type == PmType::Dead as i32 { k::MASK_DEADSOLID } else { k::MASK_PLAYERSOLID };
        pm.pmove();
        mins = pm.mins;
        maxs = pm.maxs;
        numtouch = pm.numtouch;
        touch = pm.touchents;
        waterlevel = pm.waterlevel;
        watertype = pm.watertype;
    }
    w.rng = rng;
    w.ents[i].waterlevel = waterlevel;
    w.ents[i].watertype = watertype;
    w.clients[i].ps = ps;
    w.clients[i].pmext = pmext;
    super::unlagged::detect_collisions(w, i);
    if w.clients[i].ps.event_sequence != old_seq {
        w.ents[i].event_time = t;
    }
    ps_to_es(w, i, true);

    // per-weapon post-move checks
    match WeaponId::from_num(w.clients[i].ps.weapon as u8) {
        Some(WeaponId::Level0) => {
            if !super::weapon::check_venom_attack(w, i) {
                w.clients[i].ps.weaponstate = WeaponState::Ready as i32;
            } else {
                w.clients[i].ps.generic1 = WeaponMode::Primary as i32;
                w.add_event(i, Event::FireWeapon, 0);
            }
        }
        Some(WeaponId::Level1) | Some(WeaponId::Level1upg) => super::weapon::check_grab_attack(w, i),
        Some(WeaponId::Level3) | Some(WeaponId::Level3upg) => {
            if !super::weapon::check_pounce_attack(w, i) {
                w.clients[i].ps.weaponstate = WeaponState::Ready as i32;
            } else {
                w.clients[i].ps.generic1 = WeaponMode::Secondary as i32;
                w.add_event(i, Event::FireWeapon2, 0);
            }
        }
        Some(WeaponId::Level4) => {
            let c = &mut w.clients[i];
            if c.ps.pm_flags & k::PMF_CHARGE == 0 && c.trample_pos != 0 {
                c.trample_pos = 0;
                c.trample_hit = [0; 20];
            }
        }
        Some(WeaponId::Ckit) => super::weapon::check_ckit_repair(w, i),
        _ => {}
    }

    {
        let o = w.ents[i].pos.tr_base;
        let e = &mut w.ents[i];
        e.origin = o;
        e.mins = mins;
        e.maxs = maxs;
    }
    client_impacts(w, i, &touch[..numtouch]);
    client_events(w, i, old_seq);
    w.link(i);
    let o = w.clients[i].ps.origin;
    w.ents[i].origin = o;
    if w.clients[i].ps.event_sequence != old_seq {
        w.ents[i].event_time = t;
    }
    if st(w, i, Stat::Health) <= 0 {
        return;
    }

    {
        let c = &mut w.clients[i];
        c.oldbuttons = c.buttons;
        c.buttons = c.cmd.buttons;
        c.latched_buttons |= c.buttons & !c.oldbuttons;
    }
    if w.clients[i].buttons & k::BUTTON_USE_EVOLVE != 0 && w.clients[i].oldbuttons & k::BUTTON_USE_EVOLVE == 0 {
        use_object(w, i);
    }
    let team = Team::from_raw(w.clients[i].ps.stats[Stat::Team as usize] as u8).unwrap_or(Team::None);
    let o = w.clients[i].ps.origin;
    let bp = super::buildable::get_build_points(w, o, team).max(0);
    let marked = super::buildable::get_marked_build_points(w, o, team);
    w.clients[i].ps.persistant[Pers::Bp as usize] = bp;
    w.clients[i].ps.persistant[Pers::Markedbp as usize] = marked;

    timer_actions(w, i, msec);

    if w.clients[i].suicide_time > 0 && w.clients[i].suicide_time < t {
        w.ents[i].flags &= !combat::FL_GODMODE;
        w.ents[i].health = 0;
        w.clients[i].ps.stats[Stat::Health as usize] = 0;
        combat::player_die(w, i, i, i, 100000, Mod::Suicide as i32);
        w.clients[i].suicide_time = 0;
    }
}

/// `BG_PlayerPoisonCloudTime`.
pub fn poison_cloud_time(ps: &PlayerState) -> i32 {
    let mut t = k::LEVEL1_PCLOUD_TIME;
    if bg::inventory_contains_upgrade(UpgradeId::Bsuit.num() as i32, &ps.stats) {
        t -= k::BSUIT_PCLOUD_PROTECTION;
    }
    if bg::inventory_contains_upgrade(UpgradeId::Helmet.num() as i32, &ps.stats) {
        t -= k::HELMET_PCLOUD_PROTECTION;
    }
    if bg::inventory_contains_upgrade(UpgradeId::Larmour.num() as i32, &ps.stats) {
        t -= k::LIGHTARMOUR_PCLOUD_PROTECTION;
    }
    t
}

/// The alien regeneration block of ClientThink_real.
fn regenerate(w: &mut World, i: usize) {
    let t = w.level.time;
    if w.level.surrender_team == Some(w.clients[i].team) || !(w.ents[i].next_regen_time >= 0 && w.ents[i].next_regen_time < t) {
        return;
    }
    let regen_rate = bg::class(st(w, i, Stat::Class)).regen_rate;
    if w.ents[i].health <= 0 || regen_rate == 0.0 {
        w.ents[i].next_regen_time = -1;
        return;
    }
    let origin = w.clients[i].ps.origin;
    let r = k::REGEN_BOOST_RANGE;
    let mut modifier = 1.0f32;
    let my_team = w.clients[i].team;
    let below_max = w.ents[i].health < st(w, i, Stat::MaxHealth);
    for b in w.entities_in_box(&[origin[0] - r, origin[1] - r, origin[2] - r], &[origin[0] + r, origin[1] + r, origin[2] + r]) {
        if distance(&origin, &w.ents[b].origin) > r {
            continue;
        }
        let e = &w.ents[b];
        if modifier < k::BOOSTER_REGEN_MOD && e.etype == EType::Buildable && e.buildable == Some(BuildableId::Booster) && e.spawned && e.health > 0 && e.powered {
            modifier = k::BOOSTER_REGEN_MOD;
            continue;
        }
        if e.etype == EType::Player && b < MAX_CLIENTS && e.client && w.clients[b].team == my_team && e.health > 0 {
            let class = ClassId::from_num(w.clients[b].ps.stats[Stat::Class as usize] as u8);
            let mut did = false;
            if class == Some(ClassId::Level1) && modifier < k::LEVEL1_REGEN_MOD {
                modifier = k::LEVEL1_REGEN_MOD;
                did = true;
            } else if class == Some(ClassId::Level1upg) && modifier < k::LEVEL1_UPG_REGEN_MOD {
                modifier = k::LEVEL1_UPG_REGEN_MOD;
                did = true;
            }
            if did && below_max {
                w.clients[b].has_healed = true;
            }
        }
    }
    let creep = super::buildable::find_creep(w, i);
    let c = &mut w.clients[i];
    let s = &mut c.ps.stats[Stat::State as usize];
    *s |= k::SS_HEALING_ACTIVE;
    *s &= !(k::SS_HEALING_2X | k::SS_HEALING_3X);
    if modifier == 1.0 && !creep {
        *s &= !k::SS_HEALING_ACTIVE;
        modifier *= k::ALIEN_REGEN_NOCREEP_MOD;
    } else if modifier >= 3.0 {
        *s |= k::SS_HEALING_3X;
    } else if modifier >= 2.0 {
        *s |= k::SS_HEALING_2X;
    }
    let interval = (1000.0f32 / (regen_rate * modifier)) as i32;
    let interval = interval.max(1);
    let e = &mut w.ents[i];
    let count = 1 + (t - e.next_regen_time) / interval;
    e.health += count;
    e.next_regen_time += count * interval;
    let maxh = w.clients[i].ps.stats[Stat::MaxHealth as usize];
    if w.ents[i].health >= maxh {
        w.ents[i].health = maxh;
        w.ents[i].credits.iter_mut().for_each(|c| *c = 0);
    }
}

/// `SpectatorThink`.
fn spectator_think(w: &mut World, i: usize) {
    let c = &mut w.clients[i];
    c.oldbuttons = c.buttons;
    c.buttons = c.cmd.buttons;
    let attack1 = c.buttons & k::BUTTON_ATTACK != 0 && c.oldbuttons & k::BUTTON_ATTACK == 0;
    let team = c.team;
    // following only while the target is not itself spectating
    let mut following = c.spectator == SpectatorState::Follow as i32;
    if following {
        let n = c.spectator_client;
        if n < 0 || n as usize >= MAX_CLIENTS || w.clients[n as usize].spectator != SpectatorState::Not as i32 {
            following = false;
        }
    }
    let mut queued = search_queue(w, team, i);
    if attack1 && queued {
        if w.clients[i].spectator == SpectatorState::Follow as i32 {
            stop_following(w, i);
        }
        remove_from_queue(w, team, i);
        let c = &mut w.clients[i];
        c.class_selection = None;
        c.ps.stats[Stat::Class as usize] = ClassId::Spectator.num() as i32;
        c.ps.pm_flags &= !k::PMF_QUEUED;
        queued = false;
    } else if attack1 {
        if w.clients[i].spectator == SpectatorState::Follow as i32 {
            stop_following(w, i);
        }
        let msg = match team {
            Team::None => "Choose a team",
            Team::Aliens => "Choose a class to spawn as",
            Team::Humans => "Choose an item to spawn with",
        };
        notify(w, i, msg);
    }
    if following {
        return;
    }
    let c = &mut w.clients[i];
    c.ps.pm_type = if c.spectator == SpectatorState::Locked as i32 || c.spectator == SpectatorState::Follow as i32 {
        PmType::Freeze
    } else if c.noclip {
        PmType::Noclip
    } else {
        PmType::Spectator
    } as i32;
    if queued {
        c.ps.pm_flags |= k::PMF_QUEUED;
    }
    c.ps.speed = 600; // pers.flySpeed default
    c.ps.stats[Stat::Stamina as usize] = 0;
    c.ps.stats[Stat::Misc as usize] = 0;
    c.ps.stats[Stat::Buildable as usize] = 0;
    c.ps.stats[Stat::Class as usize] = ClassId::Spectator.num() as i32;
    c.ps.weapon = 0;

    let mut ps = c.ps;
    let mut pmext = c.pmext;
    let cmd = c.cmd;
    let mut rng = std::mem::take(&mut w.rng);
    {
        let mut pm = Pmove::new(&mut ps, &mut pmext, cmd, &*w, &mut rng);
        pm.tracemask = k::MASK_DEADSOLID;
        pm.pmove();
    }
    w.rng = rng;
    let c = &mut w.clients[i];
    c.ps = ps;
    c.pmext = pmext;
    w.ents[i].origin = ps.origin;
    super::trigger::touch_triggers(w, i);
    w.unlink(i);
    let ps = w.clients[i].ps;
    if ps.pm_flags & k::PMF_QUEUED != 0 {
        let pos = queue_pos(w, team, i);
        let spawns = if team == Team::Aliens { w.level.num_alien_spawns } else { w.level.num_human_spawns };
        let c = &mut w.clients[i];
        c.ps.persistant[Pers::Queuepos as usize] = pos;
        c.ps.persistant[Pers::Spawns as usize] = spawns;
    }
}

fn client_mass(w: &World, i: usize) -> i32 {
    let c = &w.clients[i];
    match c.team {
        Team::Aliens => c.class_selection.map(|cl| cl.def().health).unwrap_or(0),
        Team::Humans => {
            if bg::inventory_contains_upgrade(UpgradeId::Bsuit.num() as i32, &c.ps.stats) {
                200
            } else {
                100
            }
        }
        Team::None => 0,
    }
}

/// `ClientShove` (g_shove defaults to 0, so this is a no-op unless tuned).
fn client_shove(w: &mut World, i: usize, v: usize) {
    let cmd = w.clients[i].cmd;
    if cmd.rightmove == 0 && cmd.forwardmove == 0 && cmd.upmove == 0 {
        return;
    }
    if !w.on_same_team(i, v) && w.clients[v].ps.ground_entity_num != i as i32 {
        return;
    }
    let (em, vm) = (client_mass(w, i), client_mass(w, v));
    if vm <= 0 || em <= 0 {
        return;
    }
    let force = (SHOVE * em as f32 / vm as f32).clamp(0.0, 150.0);
    let mut dir = sub(&w.ents[v].origin, &w.ents[i].origin);
    normalize(&mut dir);
    let push = scale(&dir, force);
    let c = &mut w.clients[v];
    c.ps.velocity = add(&c.ps.velocity, &push);
    if c.ps.pm_time == 0 {
        c.ps.pm_time = ((force * 2.0 + 0.5) as i32).clamp(50, 200);
        c.ps.pm_flags |= k::PMF_TIME_KNOCKBACK;
    }
}

/// `ClientImpacts`.
fn client_impacts(w: &mut World, i: usize, touch: &[i32]) {
    for &o in touch {
        if o < 0 || o as usize >= w.ents.len() {
            continue;
        }
        let o = o as usize;
        // see G_UnlaggedDetectCollisions(), this is the inverse of that:
        // a player blocking our server-side Pmove keeps its real position
        if o < MAX_CLIENTS && w.ents[o].client && w.clients[o].unlagged.calc.used {
            w.clients[o].unlagged.calc.used = false;
        }
        if w.clients[i].ps.weapon == WeaponId::Level4.num() as i32 {
            super::weapon::charge_attack(w, i, o);
            super::weapon::crush_attack(w, i, o);
        }
        if o < MAX_CLIENTS && w.ents[o].client {
            client_shove(w, i, o);
        }
        if let Some(t) = w.ents[o].touch {
            t(w, o, i);
        }
    }
}

/// `ClientEvents`: falling damage and weapon fire.
fn client_events(w: &mut World, i: usize, old_seq: i32) {
    let mut seq = old_seq;
    let cur = w.clients[i].ps.event_sequence;
    if seq < cur - MAX_PS_EVENTS as i32 {
        seq = cur - MAX_PS_EVENTS as i32;
    }
    let class = st(w, i, Stat::Class);
    for s in seq..cur {
        let ev = w.clients[i].ps.events[(s & (MAX_PS_EVENTS as i32 - 1)) as usize];
        // presentation copy of every predictable event
        let parm = w.clients[i].ps.event_parms[(s & (MAX_PS_EVENTS as i32 - 1)) as usize];
        if let Some(e) = Event::from_raw(ev as u8) {
            w.add_predictable_event(i, e, parm);
        }
        match Event::from_raw(ev as u8) {
            Some(Event::FallMedium) | Some(Event::FallFar) => {
                if w.ents[i].etype != EType::Player {
                    continue;
                }
                let fd = ((st(w, i, Stat::Falldist) as f32 - k::MIN_FALL_DISTANCE) / (k::MAX_FALL_DISTANCE - k::MIN_FALL_DISTANCE)).clamp(0.0, 1.0);
                let cd = bg::class(class);
                let dmg = (cd.health as f32 * cd.fall_damage * fd) as i32;
                let (mut mins, ..) = bg::class_bbox(class);
                mins[0] = 0.0;
                mins[1] = 0.0;
                let point = add(&w.clients[i].ps.origin, &mins);
                w.clients[i].pain_debounce_time = w.level.time + 200;
                combat::damage(w, i, None, None, Some([0.0, 0.0, 1.0]), Some(point), dmg, DAMAGE_NO_LOCDAMAGE, Mod::Falling);
            }
            Some(Event::FireWeapon) => super::weapon::fire_weapon(w, i, 1),
            Some(Event::FireWeapon2) => super::weapon::fire_weapon(w, i, 2),
            Some(Event::FireWeapon3) => super::weapon::fire_weapon(w, i, 3),
            _ => {}
        }
    }
}

/// The BUTTON_USE_EVOLVE block: use a structure in front, or evolve menu.
fn use_object(w: &mut World, i: usize) {
    let ps = w.clients[i].ps;
    let (view, _, _) = angle_vectors(&ps.viewangles);
    let point = ma(&ps.origin, USE_OBJECT_RANGE, &view);
    let tr = w.trace(&ps.origin, None, None, &point, i as i32, k::MASK_SHOT);
    let team = w.clients[i].team;
    // traceEnt->buildableTeam == STAT_TEAM && traceEnt->use
    let usable = |w: &World, e: usize| -> bool { e < w.ents.len() && w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].team == team && w.ents[e].use_.is_some() };
    let mut target = if tr.entity_num >= 0 && usable(w, tr.entity_num as usize) { Some(tr.entity_num as usize) } else { None };
    if target.is_none() {
        // no entity in front of player - do a small area search
        let r = USE_OBJECT_RANGE;
        let o = ps.origin;
        target = w.entities_in_box(&[o[0] - r, o[1] - r, o[2] - r], &[o[0] + r, o[1] + r, o[2] + r]).into_iter().find(|&e| usable(w, e));
    }
    if let Some(e) = target {
        // other and activator are the same in this context
        let f = w.ents[e].use_.unwrap();
        f(w, e, Some(i), Some(i));
        return;
    }
    if team == Team::Aliens {
        let class = bg::class_id(ps.stats[Stat::Class as usize]);
        if bg::alien_can_evolve(class, w.clients[i].credit, w.level.alien_stage) {
            notify(w, i, "Evolve: choose a class");
            w.clients[i].wants_menu = true;
        } else {
            w.add_event(i, Event::AlienEvolveFailed, 0);
        }
    }
}

/// The `use` callbacks of usable structures (armoury -> menu, reactor and
/// repeater -> energy ammo, telenode/egg -> nothing interesting).

/// `ClientTimerActions`.
fn timer_actions(w: &mut World, i: usize, msec: i32) {
    let t = w.level.time;
    let cmd = w.clients[i].cmd;
    let (af, ar) = ((cmd.forwardmove as i32).abs(), (cmd.rightmove as i32).abs());
    let stopped = af == 0 && ar == 0;
    let walking = !stopped && af <= 64 && ar <= 64;
    let crouched = cmd.upmove <= 0 && w.clients[i].ps.pm_flags & k::PMF_DUCKED != 0;
    {
        let c = &mut w.clients[i];
        c.time100 += msec;
        c.time1000 += msec;
        c.time10000 += msec;
    }
    while w.clients[i].time100 >= 100 {
        w.clients[i].time100 -= 100;
        let weapon = player_weapon(&w.clients[i].ps);
        {
            let c = &mut w.clients[i];
            let sprint = c.ps.stats[Stat::State as usize] & k::SS_SPEEDBOOST != 0 && c.buttons & k::BUTTON_WALKING == 0;
            let s = &mut c.ps.stats[Stat::Stamina as usize];
            if stopped || c.ps.pm_type == PmType::Jetpack as i32 {
                *s += k::STAMINA_STOP_RESTORE;
            } else if sprint {
                *s -= k::STAMINA_SPRINT_TAKE;
            } else if walking || crouched {
                *s += k::STAMINA_WALK_RESTORE;
            }
            *s = (*s).clamp(-k::STAMINA_MAX, k::STAMINA_MAX);
        }
        let builder = weapon == WeaponId::Abuild as i32 || weapon == WeaponId::Abuildupg as i32 || st(w, i, Stat::Weapon) == WeaponId::Ckit as i32;
        if builder {
            let m = &mut w.clients[i].ps.stats[Stat::Misc as usize];
            if *m > 0 {
                *m -= 100;
            }
            if *m < 0 {
                *m = 0;
            }
        }
        if weapon == WeaponId::Abuild as i32 || weapon == WeaponId::Abuildupg as i32 || weapon == WeaponId::Ckit as i32 {
            let sb = st(w, i, Stat::Buildable);
            let b = sb & !k::SB_VALID_TOGGLEBIT;
            if b > 0 {
                let ok = BuildableId::from_num(b as u8).map(|bid| super::buildable::can_build(w, i, bid).is_ok()).unwrap_or(false);
                let s = &mut w.clients[i].ps.stats[Stat::Buildable as usize];
                if ok {
                    *s |= k::SB_VALID_TOGGLEBIT;
                } else {
                    *s &= !k::SB_VALID_TOGGLEBIT;
                }
            }
            w.clients[i].ps.misc = [0; 16];
        }
        if w.clients[i].team == Team::Humans && st(w, i, Stat::State) & k::SS_HEALING_2X != 0 {
            let remaining = k::MEDKIT_STARTUP_TIME - (t - w.clients[i].last_medkit_time);
            let can = w.ents[i].health < st(w, i, Stat::MaxHealth) && w.clients[i].medkit_health_to_restore != 0 && w.clients[i].ps.pm_type != PmType::Dead as i32;
            if remaining < 0 {
                if can {
                    w.clients[i].medkit_health_to_restore -= 1;
                    w.ents[i].health += 1;
                } else {
                    w.clients[i].ps.stats[Stat::State as usize] &= !k::SS_HEALING_2X;
                }
            } else if can {
                if t > w.clients[i].medkit_increment_time {
                    w.clients[i].medkit_health_to_restore -= 1;
                    w.ents[i].health += 1;
                    w.clients[i].medkit_increment_time = t + remaining / k::MEDKIT_STARTUP_SPEED;
                }
            } else {
                w.clients[i].ps.stats[Stat::State as usize] &= !k::SS_HEALING_2X;
            }
        }
    }
    while w.clients[i].time1000 >= 1000 {
        w.clients[i].time1000 -= 1000;
        if st(w, i, Stat::State) & k::SS_POISONED != 0 {
            let stats = w.clients[i].ps.stats;
            let mut dmg = k::ALIEN_POISON_DMG;
            if bg::inventory_contains_upgrade(UpgradeId::Bsuit.num() as i32, &stats) {
                dmg -= k::BSUIT_POISON_PROTECTION;
            }
            if bg::inventory_contains_upgrade(UpgradeId::Helmet.num() as i32, &stats) {
                dmg -= k::HELMET_POISON_PROTECTION;
            }
            if bg::inventory_contains_upgrade(UpgradeId::Larmour.num() as i32, &stats) {
                dmg -= k::LIGHTARMOUR_POISON_PROTECTION;
            }
            let pc = w.clients[i].last_poison_client;
            let pc = if pc >= 0 { Some(pc as usize) } else { None };
            combat::damage(w, i, pc, pc, None, None, dmg, 0, Mod::Poison);
        }
        // turn off life support when a team admits defeat
        let st_team = w.clients[i].team;
        if st_team != Team::None && w.level.surrender_team == Some(st_team) {
            let dmg = if st_team == Team::Aliens { bg::class(st(w, i, Stat::Class)).regen_rate as i32 } else { 5 };
            combat::damage(w, i, None, None, None, None, dmg, combat::DAMAGE_NO_ARMOR, Mod::Suicide);
        }
        w.clients[i].alive_seconds += 1;
        let period = w.level.freekill_period;
        if period > 0 && w.clients[i].alive_seconds % period == 0 {
            match w.clients[i].team {
                Team::Aliens => combat::add_credit(w, i, k::FREEKILL_ALIEN, true),
                Team::Humans => combat::add_credit(w, i, k::FREEKILL_HUMAN, true),
                _ => {}
            }
        }
    }
    while w.clients[i].time10000 >= 10000 {
        w.clients[i].time10000 -= 10000;
        let wp = w.clients[i].ps.weapon;
        if wp == WeaponId::Abuild as i32 || wp == WeaponId::Abuildupg as i32 {
            combat::add_score(w, i, k::ALIEN_BUILDER_SCOREINC);
        } else if wp == WeaponId::Ckit as i32 {
            combat::add_score(w, i, k::HUMAN_BUILDER_SCOREINC);
        }
        if w.clients[i].has_healed {
            if wp == WeaponId::Level1 as i32 {
                combat::add_score(w, i, k::LEVEL1_REGEN_SCOREINC);
            } else if wp == WeaponId::Level1upg as i32 {
                combat::add_score(w, i, k::LEVEL1_UPG_REGEN_SCOREINC);
            }
            w.clients[i].has_healed = false;
        }
    }
    if w.clients[i].ps.weapon == WeaponId::Level3upg as i32 {
        if w.clients[i].ps.ammo < WeaponId::Level3upg.def().max_ammo {
            if w.ents[i].timestamp + k::LEVEL3_BOUNCEBALL_REGEN < t {
                w.clients[i].ps.ammo += 1;
                w.ents[i].timestamp = t;
            }
        } else {
            w.ents[i].timestamp = t;
        }
    }
}

/// `BG_GetPlayerWeapon`.
pub fn player_weapon(ps: &PlayerState) -> i32 {
    let n = ps.persistant[Pers::Newweapon as usize];
    if n != 0 { n } else { ps.weapon }
}

/// `client_end_frame` system.
pub fn end_frame_all(w: &mut World) {
    for i in 0..MAX_CLIENTS {
        if w.clients[i].connected {
            end_frame(w, i);
        }
    }
}

/// `ClientEndFrame`.
pub fn end_frame(w: &mut World, i: usize) {
    if w.clients[i].spectator != SpectatorState::Not as i32 {
        spectator_client_end_frame(w, i);
        return;
    }
    if w.level.intermission_time != 0 {
        return;
    }
    // burn from lava, etc
    world_effects(w, i);
    damage_feedback(w, i);
    if w.clients[i].ps.stats[Stat::Health as usize] != w.ents[i].health {
        let h = w.ents[i].health;
        w.clients[i].ps.stats[Stat::Health as usize] = h;
    }
    if st(w, i, Stat::Health) <= 0 && w.level.time >= w.clients[i].respawn_time {
        respawn(w, i);
    }
    // G_SetClientSound
    let fry = w.ents[i].waterlevel != 0 && w.ents[i].watertype & (contents::LAVA as i32 | contents::SLIME as i32) != 0;
    w.clients[i].ps.loop_sound = if fry { w.sound_index("sound/misc/fry.wav") } else { 0 };
    ps_to_es(w, i, true);
}

/// `SpectatorClientEndFrame`: a follower sees the followed player's state.
fn spectator_client_end_frame(w: &mut World, i: usize) {
    if w.clients[i].spectator != SpectatorState::Follow as i32 {
        return;
    }
    let n = w.clients[i].spectator_client;
    if n < 0 || n as usize >= MAX_CLIENTS || !w.clients[n as usize].connected {
        return;
    }
    let score = w.clients[i].ps.persistant[Pers::Score as usize];
    let ping = w.clients[i].ps.ping;
    let mut ps = w.clients[n as usize].ps;
    ps.persistant[Pers::Score as usize] = score;
    ps.ping = ping;
    ps.pm_flags |= k::PMF_FOLLOW;
    ps.pm_flags &= !k::PMF_QUEUED;
    w.clients[i].ps = ps;
}

// ========================================================== intermission

/// `FindIntermissionPoint`: info_player_intermission (looking at its
/// target), else a player spawn.
pub fn find_intermission_point(w: &mut World) {
    let spot = (0..w.num_entities).find(|&e| w.ents[e].inuse && w.ents[e].classname == "info_player_intermission");
    if let Some(e) = spot {
        w.level.intermission_origin = w.ents[e].s_origin;
        w.level.intermission_angle = w.ents[e].s_angles;
        let tgt = w.ents[e].target.clone();
        if let Some(t) = super::spawn::pick_target(w, tgt.as_deref()) {
            let dir = sub(&w.ents[t].s_origin, &w.level.intermission_origin);
            w.level.intermission_angle = vectoangles(&dir);
        }
    } else if let Some((o, a)) = select_spawn_point(w, &[0.0; 3]) {
        // the map creator forgot to put in an intermission point...
        w.level.intermission_origin = o;
        w.level.intermission_angle = a;
    } else {
        let (o, a) = w.intermission_point(Team::None);
        w.level.intermission_origin = o;
        w.level.intermission_angle = a;
    }
}

/// `MoveClientToIntermission`.
pub fn move_client_to_intermission(w: &mut World, i: usize) {
    if w.clients[i].spectator == SpectatorState::Follow as i32 {
        stop_following(w, i);
    }
    let (o, a) = (w.level.intermission_origin, w.level.intermission_angle);
    w.ents[i].origin = o;
    let c = &mut w.clients[i];
    c.ps.origin = o;
    c.ps.viewangles = a;
    c.ps.pm_type = PmType::Intermission as i32;
    c.ps.e_flags = 0;
    c.ps.velocity = [0.0; 3];
    w.ents[i].etype = EType::General;
    w.ents[i].e_flags = 0;
    w.ents[i].loop_sound = 0;
    w.ents[i].contents = 0;
    w.unlink(i);
}

/// `BeginIntermission`.
pub fn begin_intermission(w: &mut World) {
    if w.level.intermission_time != 0 {
        return;
    }
    w.level.intermission_time = w.level.time;
    super::vote::clear_votes(w);
    find_intermission_point(w);
    for i in 0..MAX_CLIENTS {
        if !w.clients[i].connected {
            continue;
        }
        if w.ents[i].health <= 0 && w.clients[i].spectator == SpectatorState::Not as i32 {
            respawn(w, i);
        }
        move_client_to_intermission(w, i);
    }
}

/// `ExitLevel`: map_restart (no g_nextMap / rotation in this port); the host
/// sees `level.exited` and reloads.
pub fn exit_level(w: &mut World) {
    w.level.intermission_time = 0;
    w.level.exited = true;
    for c in w.clients.iter_mut().filter(|c| c.connected) {
        c.ps.persistant[Pers::Score as usize] = 0;
    }
}

/// `CheckIntermissionExit`.
pub fn check_intermission_exit(w: &mut World) {
    let connected = w.clients.iter().filter(|c| c.connected).count();
    if connected == 0 {
        exit_level(w);
        return;
    }
    let (mut ready, mut not_ready) = (0, 0);
    for c in w.clients.iter().filter(|c| c.connected && c.team != Team::None) {
        // bots always ready up
        if c.ready_to_exit || c.is_bot {
            ready += 1;
        } else {
            not_ready += 1;
        }
    }
    let l = &mut w.level;
    if l.time < l.intermission_time + 5000 {
        return;
    }
    if l.time > l.intermission_time + 60000 {
        exit_level(w);
        return;
    }
    if ready == 0 && not_ready > 0 {
        l.ready_to_exit = false;
        return;
    }
    if not_ready == 0 {
        exit_level(w);
        return;
    }
    if !l.ready_to_exit {
        l.ready_to_exit = true;
        l.exit_time = l.time;
    }
    if l.time < l.exit_time + 30000 {
        return;
    }
    exit_level(w);
}

// ============================================================ following

/// `G_SelectAlienLockSpawnPoint` / `G_SelectHumanLockSpawnPoint`.
fn lock_spawn_point(w: &mut World, team: Team) -> (Vec3, Vec3) {
    match team {
        Team::None => select_spectator_spawn_point(w),
        _ => select_lock_spawn_point(w, team),
    }
}

fn first_ent(w: &World, classname: &str) -> Option<usize> {
    (0..w.num_entities).find(|&e| w.ents[e].inuse && w.ents[e].classname == classname)
}

/// `SpotWouldTelefrag`: any client inside a player box at the spot.
pub fn spot_would_telefrag(w: &World, spot: usize) -> bool {
    let o = w.ents[spot].s_origin;
    let (mins, maxs) = ([o[0] - 15.0, o[1] - 15.0, o[2] - 24.0], [o[0] + 15.0, o[1] + 15.0, o[2] + 32.0]);
    w.entities_in_box(&mins, &maxs).into_iter().any(|e| e < MAX_CLIENTS && w.ents[e].client)
}

/// `G_SelectRandomFurthestSpawnPoint` / `G_SelectSpawnPoint`: a random
/// info_player_deathmatch among the furthest half from `avoid`.
pub fn select_spawn_point(w: &mut World, avoid: &Vec3) -> Option<(Vec3, Vec3)> {
    let mut list: Vec<(f32, usize)> = Vec::new();
    for e in 0..w.num_entities {
        if !w.ents[e].inuse || w.ents[e].classname != "info_player_deathmatch" || spot_would_telefrag(w, e) {
            continue;
        }
        let dist = length(&sub(&w.ents[e].s_origin, avoid));
        // insertion into a list sorted by distance, furthest first (max 64)
        let pos = list.iter().position(|&(d, _)| dist > d).unwrap_or(list.len());
        if pos < 64 {
            list.insert(pos, (dist, e));
            list.truncate(64);
        }
    }
    let spot = if list.is_empty() {
        first_ent(w, "info_player_deathmatch")?
    } else {
        let rnd = (w.rng.random() * (list.len() / 2) as f32) as usize;
        list[rnd].1
    };
    let mut o = w.ents[spot].s_origin;
    o[2] += 9.0;
    Some((o, w.ents[spot].s_angles))
}

/// `G_SelectSpectatorSpawnPoint`: the intermission point.
pub fn select_spectator_spawn_point(w: &mut World) -> (Vec3, Vec3) {
    find_intermission_point(w);
    (w.level.intermission_origin, w.level.intermission_angle)
}

/// `G_SelectAlienLockSpawnPoint` / `G_SelectHumanLockSpawnPoint`.
pub fn select_lock_spawn_point(w: &mut World, team: Team) -> (Vec3, Vec3) {
    let name = if team == Team::Aliens { "info_alien_intermission" } else { "info_human_intermission" };
    match first_ent(w, name) {
        Some(e) => (w.ents[e].s_origin, w.ents[e].s_angles),
        None if w.num_entities > MAX_CLIENTS && first_ent(w, "info_player_deathmatch").is_some() => select_spectator_spawn_point(w),
        // sheet-authored worlds keep placements as spots, not entities
        None => w.intermission_point(team),
    }
}

/// `G_StopFromFollowing`: stops any other clients from following this one.
pub fn stop_from_following(w: &mut World, i: usize) {
    for f in 0..MAX_CLIENTS {
        if w.clients[f].connected && w.clients[f].spectator == SpectatorState::Follow as i32 && w.clients[f].spectator_client == w.clients[i].ps.client_num {
            if !follow_new_client(w, f, 1) {
                stop_following(w, f);
            }
        }
    }
}

/// `G_StopFollowing`: drop to free (or team-locked) spectating.
pub fn stop_following(w: &mut World, i: usize) {
    let team = w.clients[i].team;
    w.clients[i].ps.stats[Stat::Team as usize] = team as i32;
    if team == Team::None {
        w.clients[i].spectator = SpectatorState::Free as i32;
    } else {
        w.clients[i].spectator = SpectatorState::Locked as i32;
        let (o, a) = lock_spawn_point(w, team);
        super::spawn::set_origin(w, i, o);
        w.clients[i].ps.origin = o;
        set_view_angle(w, i, a);
    }
    let credit = w.clients[i].credit;
    let c = &mut w.clients[i];
    c.ps.persistant[Pers::Specstate as usize] = c.spectator;
    c.spectator_client = -1;
    c.ps.pm_flags &= !k::PMF_FOLLOW;
    c.ps.ground_entity_num = ENTITYNUM_NONE;
    c.ps.stats[Stat::State as usize] = 0;
    c.ps.stats[Stat::Viewlock as usize] = 0;
    c.ps.e_flags &= !(k::EF_WALLCLIMB | k::EF_WALLCLIMBCEILING);
    c.ps.viewangles[PITCH] = 0.0;
    c.ps.client_num = i as i32;
    c.ps.persistant[Pers::Credit as usize] = credit;
    super::team::calculate_ranks(w);
}

/// `G_FollowLockView`: the followed player is in the spawn queue, so watch
/// from that team's lock position.
pub fn follow_lock_view(w: &mut World, i: usize) {
    let n = w.clients[i].spectator_client;
    let team = w.clients[i].team;
    {
        let c = &mut w.clients[i];
        c.spectator = SpectatorState::Follow as i32;
        c.ps.persistant[Pers::Specstate as usize] = c.spectator;
        c.ps.client_num = n;
        c.ps.pm_flags &= !k::PMF_FOLLOW;
        c.ps.stats[Stat::Team as usize] = team as i32;
        c.ps.stats[Stat::State as usize] &= !k::SS_WALLCLIMBING;
        c.ps.stats[Stat::Viewlock as usize] = 0;
        c.ps.e_flags &= !(k::EF_WALLCLIMB | k::EF_WALLCLIMBCEILING);
        c.ps.e_flags ^= k::EF_TELEPORT_BIT;
        c.ps.viewangles[PITCH] = 0.0;
    }
    let ft = if n >= 0 && (n as usize) < MAX_CLIENTS { w.clients[n as usize].team } else { Team::None };
    // the C leaves spawn_origin uninitialised for a spectator target; use the
    // spectator point instead
    let (o, a) = lock_spawn_point(w, ft);
    super::spawn::set_origin(w, i, o);
    w.clients[i].ps.origin = o;
    set_view_angle(w, i, a);
}

/// `G_FollowNewClient`: cycle to the next followable client in `dir`.
pub fn follow_new_client(w: &mut World, i: usize, dir: i32) -> bool {
    let dir = dir.clamp(-1, 1);
    if dir == 0 {
        return true;
    }
    if w.clients[i].spectator == SpectatorState::Not as i32 {
        return false;
    }
    let max = MAX_CLIENTS as i32;
    let mut clientnum = w.clients[i].spectator_client;
    let mut original = clientnum;
    let mut select_any = false;
    if clientnum < 0 || clientnum >= max {
        clientnum = 0;
        original = 0;
        select_any = true;
    }
    loop {
        clientnum += dir;
        if clientnum >= max {
            clientnum = 0;
        }
        if clientnum < 0 {
            clientnum = max - 1;
        }
        let n = clientnum as usize;
        let ok = n != i
            && !(clientnum == original && !select_any)
            && w.clients[n].connected
            && w.clients[n].team != Team::None
            && (w.clients[i].sticky_spec || w.clients[n].spectator == SpectatorState::Not as i32)
            && (w.clients[i].team == Team::None || w.clients[n].team == w.clients[i].team);
        if ok {
            w.clients[i].spectator_client = clientnum;
            w.clients[i].spectator = SpectatorState::Follow as i32;
            if w.clients[n].spectator != SpectatorState::Not as i32 {
                follow_lock_view(w, i);
            }
            return true;
        }
        if clientnum == original {
            return false;
        }
    }
}

/// `G_ToggleFollow`.
pub fn toggle_follow(w: &mut World, i: usize) {
    if w.clients[i].spectator == SpectatorState::Follow as i32 {
        stop_following(w, i);
    } else {
        follow_new_client(w, i, 1);
    }
}

/// `G_ClientNumberFromString`: a slot number or a unique (sub)name match.
pub fn client_number_from_string(w: &World, s: &str) -> Result<usize, String> {
    if s.is_empty() {
        return Err("no player name or slot # provided\n".into());
    }
    // numeric values are just slot numbers
    if s.bytes().all(|c| c.is_ascii_digit()) {
        let i = super::spawn::c_atoi(s);
        if i < 0 || i as usize >= MAX_CLIENTS {
            return Err(String::new());
        }
        if !w.clients[i as usize].connected {
            return Err("no player connected in that slot #\n".into());
        }
        return Ok(i as usize);
    }
    let s2 = super::cmds::sanitise_string(s);
    if s2.is_empty() {
        return Err("no player name provided\n".into());
    }
    let mut err = String::from("more than one player name matches. be more specific or use the slot #:\n");
    let (mut found, mut m) = (0, 0);
    for i in 0..MAX_CLIENTS {
        if !w.clients[i].connected {
            continue;
        }
        let n2 = super::cmds::sanitise_string(&w.clients[i].name);
        if n2 == s2 {
            return Ok(i);
        }
        if n2.contains(&s2) {
            err.push_str(&format!("{i:<2} - {}^7\n", w.clients[i].name));
            found += 1;
            m = i;
        }
    }
    if found == 1 {
        return Ok(m);
    }
    if found == 0 {
        return Err("no connected player by that name or slot #\n".into());
    }
    Err(err)
}

/// `Cmd_Follow_f`.
pub fn cmd_follow(w: &mut World, i: usize, arg: Option<&str>) -> Result<(), String> {
    if w.clients[i].spectator == SpectatorState::Not as i32 {
        return Ok(());
    }
    let Some(arg) = arg else {
        toggle_follow(w, i);
        return Ok(());
    };
    let n = client_number_from_string(w, arg).map_err(|e| format!("follow: {e}"))?;
    if n == i {
        return Ok(());
    }
    if !w.clients[i].sticky_spec && w.clients[n].spectator != SpectatorState::Not as i32 {
        return Ok(());
    }
    if w.clients[i].team != Team::None && w.clients[n].team != w.clients[i].team {
        return Ok(());
    }
    w.clients[i].spectator = SpectatorState::Follow as i32;
    w.clients[i].spectator_client = n as i32;
    Ok(())
}

/// `P_WorldEffects`: drowning, lava and slime.
fn world_effects(w: &mut World, i: usize) {
    let t = w.level.time;
    if w.clients[i].noclip {
        w.clients[i].air_out_time = t + 12000; // don't need air
        return;
    }
    let waterlevel = w.ents[i].waterlevel;
    if waterlevel == 3 {
        if w.clients[i].air_out_time < t {
            // drown!
            w.clients[i].air_out_time += 1000;
            if w.ents[i].health > 0 {
                w.ents[i].damage = (w.ents[i].damage + 2).min(15);
                let snd = if w.ents[i].health <= w.ents[i].damage {
                    "*drown.wav"
                } else if w.rng.rand() < super::rng::RAND_MAX / 2 + 1 {
                    "sound/player/gurp1.wav"
                } else {
                    "sound/player/gurp2.wav"
                };
                let idx = w.sound_index(snd);
                w.add_event(i, Event::GeneralSound, idx);
                w.clients[i].pain_debounce_time = t + 200;
                let d = w.ents[i].damage;
                combat::damage(w, i, None, None, None, None, d, combat::DAMAGE_NO_ARMOR, Mod::Water);
            }
        }
    } else {
        w.clients[i].air_out_time = t + 12000;
        w.ents[i].damage = 2;
    }
    let wt = w.ents[i].watertype;
    if waterlevel != 0 && wt & (contents::LAVA as i32 | contents::SLIME as i32) != 0 && w.ents[i].health > 0 && w.clients[i].pain_debounce_time <= t {
        if wt & contents::LAVA as i32 != 0 {
            combat::damage(w, i, None, None, None, None, 30 * waterlevel, 0, Mod::Lava);
        }
        if wt & contents::SLIME as i32 != 0 {
            combat::damage(w, i, None, None, None, None, 10 * waterlevel, 0, Mod::Slime);
        }
    }
}

/// `P_DamageFeedback`.
fn damage_feedback(w: &mut World, i: usize) {
    if !bg::pm_live(w.clients[i].ps.pm_type) {
        return;
    }
    let count = w.clients[i].damage_flash;
    if count == 0 {
        return;
    }
    // world damage (falling, slime, etc) uses a special code
    // to make the blend blob centered instead of positional
    if w.clients[i].damage_from_world {
        w.clients[i].ps.damage_pitch = 255;
        w.clients[i].ps.damage_yaw = 255;
        w.clients[i].damage_from_world = false;
    } else {
        let angles = vectoangles(&w.clients[i].damage_from);
        w.clients[i].ps.damage_pitch = (angles[0] / 360.0 * 256.0) as i32;
        w.clients[i].ps.damage_yaw = (angles[1] / 360.0 * 256.0) as i32;
    }
    let t = w.level.time;
    if t > w.clients[i].pain_debounce_time {
        w.clients[i].pain_debounce_time = t + 700;
        let h = w.ents[i].health.min(255);
        w.add_event(i, Event::Pain, h);
        w.clients[i].ps.damage_event += 1;
    }
    let c = &mut w.clients[i];
    c.ps.damage_count = count.min(255);
    c.damage_flash = 0;
}

// =================================================================== spawning

/// `BodySink`.
fn body_sink(w: &mut World, b: usize) {
    let t = w.level.time;
    if !w.ents[b].active {
        w.ents[b].active = true;
        w.ents[b].killed_by = MAX_CLIENTS as i32;
        w.ents[b].timestamp = t;
    }
    if t - w.ents[b].timestamp > 6500 {
        w.free(b);
        return;
    }
    w.ents[b].nextthink = t + 100;
    w.ents[b].pos.tr_base[2] -= 1.0;
    w.ents[b].origin[2] -= 1.0;
}

/// `SpawnCorpse`.
fn spawn_corpse(w: &mut World, i: usize) {
    let origin = w.ents[i].origin;
    w.unlink(i);
    if w.point_contents(&origin, -1) & contents::NODROP as i32 != 0 {
        return;
    }
    let t = w.level.time;
    let ps = w.clients[i].ps;
    let nonseg = ps.persistant[Pers::State as usize] & k::PS_NONSEGMODEL != 0;
    let legs = w.ents[i].legs_anim & !k::ANIM_TOGGLEBIT;
    let b = w.spawn();
    let (_, _, _, dmins, dmaxs) = bg::class_bbox(ps.stats[Stat::Class as usize]);
    let v_diff = dmins[2] - w.ents[i].mins[2];
    let dest = [origin[0], origin[1], origin[2] - v_diff];
    // body->clipmask is still 0 here: this trace hits nothing (as in the original)
    let tr = w.trace(&origin, Some(&dmins), Some(&dmaxs), &dest, b as i32, 0);
    let anim = if !nonseg {
        (match PlayerAnim::from_raw(legs as u8) {
            Some(PlayerAnim::BothDeath1) | Some(PlayerAnim::BothDead1) => PlayerAnim::BothDead1,
            Some(PlayerAnim::BothDeath2) | Some(PlayerAnim::BothDead2) => PlayerAnim::BothDead2,
            _ => PlayerAnim::BothDead3,
        }) as i32
    } else {
        (match NonSegAnim::from_raw(legs as u8) {
            Some(NonSegAnim::Death1) | Some(NonSegAnim::Dead1) => NonSegAnim::Dead1,
            Some(NonSegAnim::Death2) | Some(NonSegAnim::Dead2) => NonSegAnim::Dead2,
            _ => NonSegAnim::Dead3,
        }) as i32
    };
    let health = ps.stats[Stat::Health as usize];
    let human = ps.stats[Stat::Team as usize] == Team::Humans as i32;
    let body_angles = w.ents[i].angles;
    let e = &mut w.ents[b];
    e.angles = body_angles;
    e.etype = EType::Corpse;
    e.e_flags = k::EF_DEAD;
    e.timestamp = t;
    e.contents = contents::CORPSE as i32;
    e.classname = if human { "humanCorpse" } else { "alienCorpse" };
    e.team = if human { Team::Humans } else { Team::Aliens };
    e.generic1 = ps.stats[Stat::Class as usize];
    e.think = Some(body_sink);
    e.nextthink = t + 20000;
    e.legs_anim = anim;
    e.torso_anim = anim;
    e.takedamage = false;
    e.health = health;
    e.mins = dmins;
    e.maxs = dmaxs;
    e.clipmask = 0;
    w.ents[i].health = 0;
    w.set_origin(b, tr.endpos);
    let e = &mut w.ents[b];
    e.pos.tr_type = crate::gen::enums::Traj::Gravity as i32;
    e.pos.tr_time = t;
    e.pos.tr_delta = ps.velocity;
    e.linked = true;
}

/// `G_SetClientViewAngle`.
pub fn set_view_angle(w: &mut World, i: usize, angle: Vec3) {
    let c = &mut w.clients[i];
    for k_ in 0..3 {
        let cmd_angle = angle2short_f(angle[k_]);
        c.ps.delta_angles[k_] = cmd_angle - c.cmd.angles[k_];
    }
    c.ps.viewangles = angle;
    w.ents[i].angles = angle;
}

/// `respawn`.
pub fn respawn(w: &mut World, i: usize) {
    spawn_corpse(w, i);
    w.clients[i].class_selection = None;
    client_spawn(w, i, None, None, None);
    // stop any following clients that don't have sticky spec on
    for f in 0..MAX_CLIENTS {
        if w.clients[f].connected && w.clients[f].spectator == SpectatorState::Follow as i32 && w.clients[f].spectator_client == i as i32 {
            if !w.clients[f].sticky_spec {
                if !follow_new_client(w, f, 1) {
                    stop_following(w, f);
                }
            } else {
                follow_lock_view(w, f);
            }
        }
    }
}

/// `G_SelectSpawnBuildable`.
fn select_spawn_buildable(w: &World, preference: &Vec3, b: BuildableId) -> Option<usize> {
    let mut spot: Option<usize> = None;
    for s in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[s];
        if !e.inuse || e.etype != EType::Buildable || e.buildable != Some(b) {
            continue;
        }
        if !e.spawned || e.health <= 0 || e.ground_entity == 0 || e.client_spawn_time > 0 {
            continue;
        }
        if super::buildable::check_spawn_point(w, s as i32, e.origin, e.origin2, b).is_err() {
            continue;
        }
        if spot.map(|sp| distance(preference, &e.origin) < distance(preference, &w.ents[sp].origin)).unwrap_or(true) {
            spot = Some(s);
        }
    }
    spot
}

/// `G_SelectTremulousSpawnPoint` -> (spawn ent, origin, angles).
pub fn select_tremulous_spawn_point(w: &World, team: Team, preference: &Vec3) -> Option<(usize, Vec3, Vec3)> {
    let b = match team {
        Team::Aliens if w.level.num_alien_spawns > 0 => BuildableId::Eggpod,
        Team::Humans if w.level.num_human_spawns > 0 => BuildableId::Telenode,
        _ => return None,
    };
    let spot = select_spawn_buildable(w, preference, b)?;
    let e = &w.ents[spot];
    let origin = super::buildable::check_spawn_point(w, spot as i32, e.origin, e.origin2, b).ok()?;
    let mut angles = e.angles;
    angles[ROLL] = 0.0;
    Some((spot, origin, angles))
}

/// `ClientSpawn`. `spawn == Some(i)` means evolving in place.
pub fn client_spawn(w: &mut World, i: usize, spawn: Option<usize>, origin: Option<Vec3>, angles: Option<Vec3>) {
    let t = w.level.time;
    super::unlagged::clear(w, i);
    let team = w.clients[i].team;
    {
        let c = &mut w.clients[i];
        // if client is dead and following teammate, stop following before spawning
        if c.spectator_client != -1 {
            c.spectator_client = -1;
            c.spectator = SpectatorState::Free as i32;
        }
        if c.class_selection.is_none() && team == Team::None {
            c.spectator = SpectatorState::Free as i32;
        } else if c.class_selection.is_none() {
            c.spectator = SpectatorState::Locked as i32;
        }
    }
    if w.clients[i].spectator == SpectatorState::Follow as i32 {
        stop_following(w, i);
    }
    let mut spawn_origin = origin.unwrap_or([0.0; 3]);
    let mut spawn_angles = angles.unwrap_or([0.0; 3]);
    let spectating = w.clients[i].spectator != SpectatorState::Not as i32;
    let mut spawn_point = None;
    if spectating {
        let (o, a) = lock_spawn_point(w, team);
        spawn_origin = o;
        spawn_angles = a;
    } else {
        let Some(sp) = spawn else { return };
        spawn_point = Some(sp);
        if sp != i {
            super::buildable::set_anim(w, sp, crate::gen::enums::BuildableAnim::Spawn1);
            w.ents[sp].client_spawn_time = if w.ents[sp].team == Team::Aliens { k::ALIEN_SPAWN_REPEAT_TIME } else { k::HUMAN_SPAWN_REPEAT_TIME };
        }
    }

    // clear everything but the persistant data
    let flags = (w.clients[i].ps.e_flags & k::EF_TELEPORT_BIT) ^ k::EF_TELEPORT_BIT;
    let old = std::mem::take(&mut w.clients[i]);
    let class = old.class_selection;
    let mut c = Client {
        connected: true,
        name: old.name,
        is_bot: old.is_bot,
        team: old.team,
        class_selection: old.class_selection,
        human_item: old.human_item,
        credit: old.credit,
        score: old.score,
        kills: old.kills,
        deaths: old.deaths,
        spectator: old.spectator,
        cmd: old.cmd,
        last_death_location: old.last_death_location,
        evolve_health_fraction: old.evolve_health_fraction,
        message: old.message,
        last_cmd_time: old.last_cmd_time,
        sticky_spec: old.sticky_spec,
        spectator_client: -1,
        // the rest of clientPersistant_t survives ClientSpawn too
        alive_seconds: old.alive_seconds,
        team_change_time: old.team_change_time,
        enter_time: old.enter_time,
        vote: old.vote,
        voted: old.voted,
        ignored: old.ignored,
        guid: old.guid,
        local_client: old.local_client,
        async_cmds: old.async_cmds,
        ip: old.ip,
        use_unlagged: old.use_unlagged,
        team_info: old.team_info,
        admin: old.admin,
        namelog: old.namelog,
        flood_demerits: old.flood_demerits,
        flood_time: old.flood_time,
        // clientSession_t
        restart_team: old.restart_team,
        spectator_time: old.spectator_time,
        inactivity_time: w.level.time + w.level.inactivity * 1000,
        ..Default::default()
    };
    c.ps.ping = old.ps.ping;
    c.ps.persistant = old.ps.persistant;
    c.ps.event_sequence = old.ps.event_sequence;
    c.ps.persistant[Pers::SpawnCount as usize] += 1;
    c.ps.persistant[Pers::Specstate as usize] = c.spectator;
    c.ps.e_flags = flags;
    c.ps.client_num = i as i32;
    c.last_poison_client = -1;
    let class_num = class.map(|c| c.num() as i32).unwrap_or(0);
    // ClientUserinfoChanged: G_NonSegModel(model) -> PS_NONSEGMODEL (classes.nonsegmented)
    if bg::class(class_num).nonsegmented {
        c.ps.persistant[Pers::State as usize] |= k::PS_NONSEGMODEL;
    } else {
        c.ps.persistant[Pers::State as usize] &= !k::PS_NONSEGMODEL;
    }
    c.ps.stats[Stat::MaxHealth as usize] = if !spectating { bg::class(class_num).health } else { 100 };
    let weapon = if class == Some(ClassId::HumanBase) || class == Some(ClassId::HumanBsuit) {
        bg::add_upgrade(UpgradeId::Medkit.num() as i32, &mut c.ps.stats);
        c.human_item.map(|w| w.num() as i32).unwrap_or(WeaponId::Rifle.num() as i32)
    } else if !spectating {
        bg::class(class_num).start_weapon.map(|w| w.num() as i32).unwrap_or(0)
    } else {
        0
    };
    let wd = bg::weapon(weapon);
    c.ps.stats[Stat::Weapon as usize] = weapon;
    c.ps.ammo = wd.max_ammo;
    c.ps.clips = wd.max_clips;
    c.ps.persistant[Pers::Newweapon as usize] = 0;
    c.ps.stats[Stat::Class as usize] = class_num;
    c.ps.stats[Stat::Team as usize] = team as i32;
    c.ps.stats[Stat::Buildable as usize] = 0;
    c.ps.stats[Stat::State as usize] = 0;
    c.ps.grapple_point = [0.0, 0.0, 1.0];
    c.ps.stats[Stat::Health as usize] = c.ps.stats[Stat::MaxHealth as usize];
    c.ps.stats[Stat::Stamina as usize] = k::STAMINA_MAX;
    c.ps.origin = spawn_origin;
    c.ps.gravity = w.level.g_gravity as i32;
    c.ps.viewheight = bg::class(class_num).viewheight;
    w.clients[i] = c;

    {
        let (mins, maxs, ..) = bg::class_bbox(class_num);
        let credits = std::mem::take(&mut w.ents[i].credits);
        let e = &mut w.ents[i];
        *e = super::world::Entity { credits, ..Default::default() };
        e.credits.iter_mut().for_each(|x| *x = 0);
        if e.credits.len() != MAX_CLIENTS {
            e.credits = vec![0; MAX_CLIENTS];
        }
        e.inuse = true;
        e.client = true;
        e.etype = EType::Player;
        e.classname = "player";
        e.ground_entity = ENTITYNUM_NONE;
        e.owner = ENTITYNUM_NONE;
        e.killed_by = ENTITYNUM_NONE;
        e.takedamage = !spectating;
        e.contents = if spectating { 0 } else { contents::BODY as i32 };
        e.clipmask = k::MASK_PLAYERSOLID;
        e.mins = mins;
        e.maxs = maxs;
        e.team = team;
        e.health = w.clients[i].ps.stats[Stat::Health as usize];
        e.next_regen_time = t;
    }
    if spawn == Some(i) {
        let f = w.clients[i].evolve_health_fraction;
        w.ents[i].health = (w.ents[i].health as f32 * f) as i32;
        let h = &mut w.clients[i].ps.stats[Stat::Health as usize];
        *h = (*h as f32 * f) as i32;
    }
    w.set_origin(i, spawn_origin);

    if !spectating && team == Team::Aliens {
        if spawn == Some(i) {
            add_predictable_event(&mut w.clients[i].ps, Event::AlienEvolve as i32, dir_to_byte(&[0.0, 0.0, 1.0]));
        } else {
            spawn_angles[YAW] += 180.0;
            spawn_angles[YAW] = angle_normalize360(spawn_angles[YAW]);
            if let Some(sp) = spawn_point {
                let o2 = w.ents[sp].origin2;
                if o2[2] > 0.0 {
                    let (fwd, _, _) = angle_vectors(&spawn_angles);
                    let mut dir = add(&o2, &scale(&fwd, F_VEL));
                    normalize(&mut dir);
                    w.clients[i].ps.velocity = scale(&dir, UP_VEL);
                }
            }
            add_predictable_event(&mut w.clients[i].ps, Event::PlayerRespawn as i32, 0);
        }
    } else if !spectating && team == Team::Humans {
        spawn_angles[YAW] += 180.0;
        spawn_angles[YAW] = angle_normalize360(spawn_angles[YAW]);
    }
    w.clients[i].ps.pm_flags |= k::PMF_RESPAWNED;
    set_view_angle(w, i, spawn_angles);
    if !spectating {
        w.link(i);
        if team == Team::Humans {
            force_weapon_change(w, i, weapon);
        }
        w.clients[i].ps.weaponstate = WeaponState::Ready as i32;
    } else {
        w.unlink(i);
    }
    {
        let c = &mut w.clients[i];
        c.ps.pm_flags |= k::PMF_TIME_KNOCKBACK;
        c.ps.pm_time = 100;
        c.respawn_time = t;
        c.ps.torso_anim = PlayerAnim::TorsoStand as i32;
        c.ps.legs_anim = PlayerAnim::LegsIdle as i32;
        c.ps.weapon = c.ps.stats[Stat::Weapon as usize];
        c.ps.command_time = t - 100;
        c.cmd.server_time = t;
    }
    client_think_real(w, i);
    if !spectating {
        ps_to_es(w, i, true);
        let o = w.clients[i].ps.origin;
        w.ents[i].origin = o;
        w.link(i);
    }
    w.ents[i].evaluate_acceleration = true;
    super::team::calculate_ranks(w);
    end_frame(w, i);
    ps_to_es(w, i, true);
}

// ================================================================ spawn queue

fn queue(w: &mut World, team: Team) -> Option<&mut VecDeque<usize>> {
    match team {
        Team::Aliens => Some(&mut w.level.alien_spawn_queue),
        Team::Humans => Some(&mut w.level.human_spawn_queue),
        Team::None => None,
    }
}
fn search_queue(w: &mut World, team: Team, i: usize) -> bool {
    queue(w, team).map(|q| q.contains(&i)).unwrap_or(false)
}
fn push_queue(w: &mut World, team: Team, i: usize) -> bool {
    match queue(w, team) {
        Some(q) if !q.contains(&i) && q.len() < MAX_CLIENTS => {
            q.push_back(i);
            true
        }
        _ => false,
    }
}
fn remove_from_queue(w: &mut World, team: Team, i: usize) {
    if let Some(q) = queue(w, team) {
        q.retain(|&x| x != i);
    }
    w.clients[i].ps.persistant[Pers::Queuepos as usize] = 0;
}
fn queue_pos(w: &mut World, team: Team, i: usize) -> i32 {
    queue(w, team).and_then(|q| q.iter().position(|&x| x == i)).map(|p| p as i32 + 1).unwrap_or(-1)
}

/// `spawn_clients` system: G_SpawnClients for both teams.
pub fn spawn_queues(w: &mut World) {
    if !super::team::warmup_over(w) {
        return;
    }
    for team in [Team::Aliens, Team::Humans] {
        let spawns = if team == Team::Aliens { w.level.num_alien_spawns } else { w.level.num_human_spawns };
        let Some(&head) = queue(w, team).and_then(|q| q.front()) else { continue };
        if spawns <= 0 {
            continue;
        }
        let pref = w.clients[head].last_death_location;
        if let Some((sp, o, a)) = select_tremulous_spawn_point(w, team, &pref) {
            queue(w, team).unwrap().pop_front();
            stop_following(w, head);
            w.clients[head].spectator = SpectatorState::Not as i32;
            w.clients[head].ps.pm_flags &= !k::PMF_QUEUED;
            client_spawn(w, head, Some(sp), Some(o), Some(a));
        }
    }
}

// ===================================================================== teams

/// `G_LeaveTeam`.
pub fn leave_team(w: &mut World, i: usize) {
    let team = w.clients[i].team;
    if team == Team::None {
        if w.clients[i].spectator == SpectatorState::Follow as i32 {
            stop_following(w, i);
        }
        return;
    }
    remove_from_queue(w, team, i);
    // stop any following clients
    stop_from_following(w, i);
    super::vote::vote(w, i, team, false);
    super::weapon::clear_player_zap_effects(w, i);
    w.clients[i].suicide_time = 0;
    for e in 0..w.num_entities {
        if !w.ents[e].inuse {
            continue;
        }
        if e < MAX_CLIENTS && w.ents[e].client && w.clients[e].connected {
            let c = &mut w.clients[e];
            if c.ps.stats[Stat::State as usize] & k::SS_POISONED != 0 && c.last_poison_client == i as i32 {
                c.ps.stats[Stat::State as usize] &= !k::SS_POISONED;
            }
        } else if w.ents[e].etype == EType::Missile && w.ents[e].owner == i as i32 {
            w.free(e);
        }
    }
}

/// `G_ChangeTeam` (Cmd_Team_f without the balance/vote checks).
pub fn change_team(w: &mut World, i: usize, new_team: Team) {
    let old = w.clients[i].team;
    if old == new_team {
        return;
    }
    leave_team(w, i);
    // a bot stays on the team it was moved to (putteam / forceteam)
    if w.clients[i].is_bot {
        w.bots.states[i].team = new_team;
    }
    w.clients[i].team_change_time = w.level.time;
    w.clients[i].team = new_team;
    w.clients[i].class_selection = None;
    w.clients[i].spectator = SpectatorState::Free as i32;
    client_spawn(w, i, None, None, None);
    let c = &mut w.clients[i];
    if old == Team::Humans && new_team == Team::Aliens {
        c.credit = (c.credit as f32 * k::ALIEN_MAX_CREDITS as f32 / k::HUMAN_MAX_CREDITS as f32 + 0.5) as i32;
    } else if old == Team::Aliens && new_team == Team::Humans {
        c.credit = (c.credit as f32 * k::HUMAN_MAX_CREDITS as f32 / k::ALIEN_MAX_CREDITS as f32 + 0.5) as i32;
    }
    c.ps.persistant[Pers::Credit as usize] = c.credit;
    let line = format!("ChangeTeam: {i} {}: {}^7 switched teams\n", super::log::team_name(new_team), w.clients[i].name);
    super::log::log_printf(w, &line);
    super::team::calculate_ranks(w);
}

// ================================================================== commands

/// `G_ForceWeaponChange`.
pub fn force_weapon_change(w: &mut World, i: usize, weapon: i32) {
    let ps = &mut w.clients[i].ps;
    if ps.weaponstate == WeaponState::Reloading as i32 {
        ps.torso_anim = ((ps.torso_anim & k::ANIM_TOGGLEBIT) ^ k::ANIM_TOGGLEBIT) | PlayerAnim::TorsoRaise as i32;
        ps.weapon_time = 250;
        ps.weaponstate = WeaponState::Ready as i32;
    }
    ps.persistant[Pers::Newweapon as usize] = if weapon == WeaponId::Blaster as i32 { WeaponId::Blaster as i32 } else { ps.stats[Stat::Weapon as usize] };
    ps.generic1 = WeaponMode::Notfiring as i32;
    ps.pm_flags |= k::PMF_WEAPON_SWITCH;
}

/// `G_GiveClientMaxAmmo`.
pub fn give_max_ammo(w: &mut World, i: usize, energy_only: bool) {
    let weapon = st(w, i, Stat::Weapon);
    let wd = bg::weapon(weapon);
    let ps = w.clients[i].ps;
    if wd.infinite_ammo || (energy_only && !wd.uses_energy) || bg::weapon_is_full(weapon, &ps.stats, ps.ammo, ps.clips) {
        return;
    }
    let mut max = wd.max_ammo;
    if wd.uses_energy && bg::inventory_contains_upgrade(UpgradeId::Battpack.num() as i32, &ps.stats) {
        max = (max as f32 * k::BATTPACK_MODIFIER) as i32;
    }
    w.clients[i].ps.ammo = max;
    w.clients[i].ps.clips = wd.max_clips;
    let cur = w.clients[i].ps.weapon;
    force_weapon_change(w, i, cur);
    if wd.uses_energy {
        w.add_event(i, Event::RptuseSound, 0);
    }
}

/// `G_RoomForClassChange`.
pub fn room_for_class_change(w: &World, i: usize, class: ClassId) -> Option<Vec3> {
    let (fmins, fmaxs, ..) = bg::class_bbox(st(w, i, Stat::Class));
    let (tmins, tmaxs, ..) = bg::class_bbox(class.num() as i32);
    let mut new_origin = w.clients[i].ps.origin;
    let mut growth = tmaxs[0] - fmaxs[0];
    if tmaxs[1] - fmaxs[1] > growth {
        growth = tmaxs[1] - fmaxs[1];
    }
    if tmins[0] - fmins[0] > -growth {
        growth = -(tmins[0] - fmins[0]);
    }
    if tmins[1] - fmins[1] > -growth {
        growth = -(tmins[1] - fmins[1]);
    }
    let nudge = if growth > 0.0 { growth * 2.0 } else { 0.0 };
    new_origin[2] -= tmins[2] - fmins[2];
    let mut temp = new_origin;
    temp[2] += nudge;
    let tr = w.trace(&new_origin, Some(&tmins), Some(&tmaxs), &temp, i as i32, k::MASK_PLAYERSOLID);
    let mut temp = new_origin;
    temp[2] += nudge * tr.fraction;
    let tr = w.trace(&temp, Some(&tmins), Some(&tmaxs), &new_origin, i as i32, k::MASK_PLAYERSOLID);
    new_origin = tr.endpos;
    let tr = w.trace(&new_origin, Some(&tmins), Some(&tmaxs), &new_origin, i as i32, k::MASK_PLAYERSOLID);
    if !tr.startsolid && tr.fraction == 1.0 { Some(new_origin) } else { None }
}

/// `Cmd_Team_f`: "aliens" | "humans" | "spectate".
pub fn cmd_team(w: &mut World, i: usize, name: &str) -> Result<(), String> {
    let oldteam = w.clients[i].team;
    let t = w.level.time;
    super::team::calculate_ranks(w);
    let (mut aliens, mut humans) = (w.level.num_alien_clients, w.level.num_human_clients);
    match oldteam {
        Team::Aliens => aliens -= 1,
        Team::Humans => humans -= 1,
        Team::None => {}
    }
    let mut force = super::admin::permission(w, Some(i), super::admin::ADMF_FORCETEAMCHANGE);
    let c = &w.clients[i];
    // stop team join spam
    if c.team_change_time != 0 && t - c.team_change_time < 1000 {
        return Ok(());
    }
    // stop switching teams for gameplay exploit reasons
    if !force && !w.level.cheats && c.alive_seconds != 0 && t - c.team_change_time < 30000 {
        return Err(format!("You must wait another {} seconds before changing teams again", ((30000 - (t - c.team_change_time)) as f32 / 1000.0) as i32));
    }
    if name.is_empty() {
        return Err(format!("team: {}", match oldteam { Team::Aliens => "aliens", Team::Humans => "humans", Team::None => "spectate" }));
    }
    let (alock, hlock) = (w.level.alien_team_locked, w.level.human_team_locked);
    let team = if name.eq_ignore_ascii_case("auto") {
        if hlock && alock {
            Team::None
        } else if hlock || humans > aliens {
            Team::Aliens
        } else if alock || aliens > humans {
            Team::Humans
        } else if w.rng.rand() / (super::rng::RAND_MAX / 2 + 1) == 0 {
            Team::Aliens
        } else {
            Team::Humans
        }
    } else {
        // G_TeamFromString: first character
        match name.chars().next().map(|ch| ch.to_ascii_lowercase()) {
            Some('0') | Some('s') => Team::None,
            Some('1') | Some('a') => {
                if alock {
                    return Err("The alien team is locked".into());
                } else if hlock {
                    force = true;
                }
                if !force && w.level.team_force_balance && aliens > humans {
                    return Err("The alien team has too many players. Please wait until slots become available or join the human team".into());
                }
                Team::Aliens
            }
            Some('2') | Some('h') => {
                if hlock {
                    return Err("The human team is locked".into());
                } else if alock {
                    force = true;
                }
                if !force && w.level.team_force_balance && humans > aliens {
                    return Err("The human team has too many players. Please wait until slots become available or join the alien team".into());
                }
                Team::Humans
            }
            _ => return Err(format!("Unknown team: {name}")),
        }
    };
    if oldteam == team {
        return Ok(());
    }
    change_team(w, i, team);
    Ok(())
}

/// `Cmd_Class_f`: spawn selection while spectating, evolution while alive.
pub fn cmd_class(w: &mut World, i: usize, name: &str) -> Result<(), String> {
    let team = w.clients[i].team;
    if w.clients[i].spectator != SpectatorState::Not as i32 {
        if w.clients[i].spectator == SpectatorState::Follow as i32 {
            stop_following(w, i);
        }
        match team {
            Team::Aliens => {
                let class = ClassId::from_id(name).ok_or("Unknown class")?;
                if !matches!(class, ClassId::Builder | ClassId::Builderupg | ClassId::Level0) {
                    return Err("You cannot spawn with that class".into());
                }
                if !bg::class_is_allowed(class) {
                    return Err("This class is not allowed".into());
                }
                if !bg::class_allowed_in_stage(class, w.level.alien_stage) {
                    return Err("This class is not yet available".into());
                }
                if push_queue(w, team, i) {
                    w.clients[i].class_selection = Some(class);
                    w.clients[i].ps.stats[Stat::Class as usize] = class.num() as i32;
                }
            }
            Team::Humans => {
                // set the item to spawn with (BG_WeaponIsAllowed)
                let item = match name {
                    "rifle" if bg::weapon_is_allowed(WeaponId::Rifle) => WeaponId::Rifle,
                    "ckit" if bg::weapon_is_allowed(WeaponId::Ckit) => WeaponId::Ckit,
                    _ => return Err("Unknown starting item".into()),
                };
                w.clients[i].human_item = Some(item);
                if push_queue(w, team, i) {
                    w.clients[i].class_selection = Some(ClassId::HumanBase);
                    w.clients[i].ps.stats[Stat::Class as usize] = ClassId::HumanBase.num() as i32;
                }
            }
            Team::None => return Err("Join a team first".into()),
        }
        return Ok(());
    }
    if w.ents[i].health <= 0 {
        return Ok(());
    }
    if team != Team::Aliens {
        return Err("You must be dead to use the class command".into());
    }
    let new_class = ClassId::from_id(name).ok_or("Unknown class")?;
    let Some(current) = w.clients[i].class_selection else { return Ok(()) };
    if w.overmind().is_none() {
        return Err("There is no Overmind".into());
    }
    let r = k::ALIENSENSE_RANGE * 0.5 / super::buildable::M_ROOT3;
    let o = w.clients[i].ps.origin;
    for e in w.entities_in_box(&[o[0] - r, o[1] - r, o[2] - r], &[o[0] + r, o[1] + r, o[2] + r]) {
        let x = &w.ents[e];
        if (e < MAX_CLIENTS && x.client && w.clients[e].ps.stats[Stat::Team as usize] == Team::Humans as i32) || (x.etype == EType::Buildable && x.team == Team::Humans && x.powered) {
            return Err("Too close to the enemy to evolve".into());
        }
    }
    if w.clients[i].ps.e_flags & k::EF_WALLCLIMB != 0 {
        return Err("You cannot evolve while wallwalking".into());
    }
    if matches!(current, ClassId::Builder | ClassId::Builderupg) && st(w, i, Stat::Misc) > 0 {
        return Err("You cannot evolve until build timer expires".into());
    }
    let cost = bg::class_can_evolve_from_to(current, new_class, w.clients[i].credit, w.level.alien_stage, 0);
    let Some(infest) = room_for_class_change(w, i, new_class) else { return Err("Not enough room here to evolve".into()) };
    if cost < 0 {
        return Err(format!("You cannot evolve to {}", new_class.id()));
    }
    let f = (st(w, i, Stat::Health) as f32 / current.def().health as f32).clamp(0.0, 1.0);
    w.clients[i].evolve_health_fraction = f;
    combat::add_credit(w, i, -cost, true);
    w.clients[i].class_selection = Some(new_class);
    let old_vel = w.clients[i].ps.velocity;
    let boosted = if st(w, i, Stat::State) & k::SS_BOOSTED != 0 { Some(w.clients[i].boosted_time) } else { None };
    let angles = w.ents[i].angles;
    client_spawn(w, i, Some(i), Some(infest), Some(angles));
    w.clients[i].ps.velocity = old_vel;
    if let Some(bt) = boosted {
        w.clients[i].boosted_time = bt;
        w.clients[i].ps.stats[Stat::State as usize] |= k::SS_BOOSTED;
    }
    Ok(())
}

/// `Cmd_Buy_f`.
pub fn cmd_buy(w: &mut World, i: usize, name: &str) -> Result<(), String> {
    let origin = w.clients[i].ps.origin;
    let weapon = WeaponId::from_id(name);
    let upgrade = UpgradeId::from_id(name);
    let cur_w = bg::weapon(st(w, i, Stat::Weapon));
    let energy_only = if super::buildable::buildable_range(w, origin, 100.0, BuildableId::Arm) {
        false
    } else if upgrade == Some(UpgradeId::Ammo) && cur_w.uses_energy && (super::buildable::buildable_range(w, origin, 100.0, BuildableId::Reactor) || super::buildable::buildable_range(w, origin, 100.0, BuildableId::Repeater)) {
        true
    } else {
        return Err(if upgrade == Some(UpgradeId::Ammo) && cur_w.uses_energy { "You can't buy energy ammo here".into() } else { "You must be near a powered Armoury".into() });
    };
    let credit = w.clients[i].credit;
    let stats = w.clients[i].ps.stats;
    if let Some(wp) = weapon {
        let d = wp.def();
        if bg::inventory_contains_weapon(wp.num() as i32, &stats) {
            return Err("You already hold this item".into());
        }
        if d.team != Team::Humans || !d.purchasable || !bg::weapon_allowed_in_stage(wp, w.level.human_stage) || !bg::weapon_is_allowed(wp) {
            return Err("You can't buy this item".into());
        }
        if d.price > credit {
            return Err("You don't have enough credits".into());
        }
        if d.slots & bg::slots_for_inventory(&stats) != 0 {
            return Err("You have no room for that item".into());
        }
        if !bg::player_can_change_weapon(&w.clients[i].ps) {
            return Err("You can't change weapons right now".into());
        }
        let c = &mut w.clients[i];
        c.ps.stats[Stat::Weapon as usize] = wp.num() as i32;
        c.ps.ammo = d.max_ammo;
        c.ps.clips = d.max_clips;
        if d.uses_energy && bg::inventory_contains_upgrade(UpgradeId::Battpack.num() as i32, &c.ps.stats) {
            c.ps.ammo = (c.ps.ammo as f32 * k::BATTPACK_MODIFIER) as i32;
        }
        force_weapon_change(w, i, wp.num() as i32);
        w.clients[i].ps.stats[Stat::Misc as usize] = 0;
        combat::add_credit(w, i, -d.price, false);
    } else if let Some(up) = upgrade {
        let d = up.def();
        if bg::inventory_contains_upgrade(up.num() as i32, &stats) {
            return Err("You already hold this item".into());
        }
        if d.price > credit {
            return Err("You don't have enough credits".into());
        }
        if d.slots & bg::slots_for_inventory(&stats) != 0 {
            return Err("You have no room for that item".into());
        }
        if d.team != Team::Humans || !d.purchasable || !bg::upgrade_allowed_in_stage(up, w.level.human_stage) || !bg::upgrade_is_allowed(up) {
            return Err("You can't buy this item".into());
        }
        if up == UpgradeId::Ammo {
            give_max_ammo(w, i, energy_only);
        } else {
            if up == UpgradeId::Bsuit {
                let Some(o) = room_for_class_change(w, i, ClassId::HumanBsuit) else { return Err("Not enough room here to put on a Battle Suit".into()) };
                let c = &mut w.clients[i];
                c.ps.origin = o;
                c.ps.stats[Stat::Class as usize] = ClassId::HumanBsuit.num() as i32;
                c.class_selection = Some(ClassId::HumanBsuit);
                c.ps.e_flags ^= k::EF_TELEPORT_BIT;
            }
            bg::add_upgrade(up.num() as i32, &mut w.clients[i].ps.stats);
        }
        if up == UpgradeId::Battpack {
            give_max_ammo(w, i, true);
        }
        combat::add_credit(w, i, -d.price, false);
    } else {
        return Err(format!("Unknown item {name}"));
    }
    Ok(())
}

/// `Cmd_Sell_f`.
pub fn cmd_sell(w: &mut World, i: usize, name: &str) -> Result<(), String> {
    let origin = w.clients[i].ps.origin;
    if !super::buildable::buildable_range(w, origin, 100.0, BuildableId::Arm) {
        return Err("You must be near a powered Armoury".into());
    }
    let weapon = if name.starts_with("weapon") { WeaponId::from_num(st(w, i, Stat::Weapon) as u8) } else { WeaponId::from_id(name) };
    let upgrade = UpgradeId::from_id(name);
    if let Some(wp) = weapon {
        let selected = player_weapon(&w.clients[i].ps);
        if !bg::player_can_change_weapon(&w.clients[i].ps) {
            return Err("You can't change weapons right now".into());
        }
        if !wp.def().purchasable {
            return Err("You can't sell this weapon".into());
        }
        if bg::inventory_contains_weapon(wp.num() as i32, &w.clients[i].ps.stats) {
            if wp == WeaponId::Ckit && st(w, i, Stat::Misc) > 0 {
                return Err("You cannot sell until build timer expires".into());
            }
            w.clients[i].ps.stats[Stat::Weapon as usize] = 0;
            w.clients[i].ps.stats[Stat::Buildable as usize] = 0;
            combat::add_credit(w, i, wp.def().price, false);
        }
        if wp.num() as i32 == selected {
            force_weapon_change(w, i, WeaponId::Blaster as i32);
        }
    } else if let Some(up) = upgrade {
        sell_upgrade(w, i, up)?;
    } else if name == "upgrades" {
        for up in UpgradeId::ALL {
            if bg::inventory_contains_upgrade(up.num() as i32, &w.clients[i].ps.stats) && up.def().purchasable {
                let _ = sell_upgrade(w, i, up);
            }
        }
    } else {
        return Err(format!("Unknown item {name}"));
    }
    Ok(())
}

fn sell_upgrade(w: &mut World, i: usize, up: UpgradeId) -> Result<(), String> {
    if !up.def().purchasable {
        return Err("You can't sell this item".into());
    }
    if !bg::inventory_contains_upgrade(up.num() as i32, &w.clients[i].ps.stats) {
        return Ok(());
    }
    if up == UpgradeId::Bsuit {
        let Some(o) = room_for_class_change(w, i, ClassId::HumanBase) else { return Err("Not enough room here to take off your Battle Suit".into()) };
        let c = &mut w.clients[i];
        c.ps.origin = o;
        c.ps.stats[Stat::Class as usize] = ClassId::HumanBase.num() as i32;
        c.class_selection = Some(ClassId::HumanBase);
        c.ps.e_flags ^= k::EF_TELEPORT_BIT;
    }
    bg::remove_upgrade(up.num() as i32, &mut w.clients[i].ps.stats);
    if up == UpgradeId::Battpack {
        give_max_ammo(w, i, true);
    }
    combat::add_credit(w, i, up.def().price, false);
    Ok(())
}

/// `Cmd_Build_f`: select a blueprint (the build itself is the fire button).
pub fn cmd_build(w: &mut World, i: usize, name: &str) -> Result<(), String> {
    if w.level.surrender_team == Some(w.clients[i].team) {
        return Err("Building is denied to you, as your team has admitted defeat".into());
    }
    let b = BuildableId::from_id(name).ok_or_else(|| format!("Unknown structure {name}"))?;
    let d = b.def();
    let team = w.clients[i].team;
    let stage_ = if team == Team::Aliens { w.level.alien_stage } else { w.level.human_stage };
    if !d.build_weapons.iter().any(|x| x.num() as i32 == w.clients[i].ps.weapon) || !bg::buildable_is_allowed(b) || !bg::buildable_allowed_in_stage(b, stage_) {
        return Err("You cannot build that structure".into());
    }
    w.clients[i].ps.stats[Stat::Buildable as usize] = 0;
    use super::buildable::BuildError as E;
    match super::buildable::can_build(w, i, b) {
        Ok(_) => w.clients[i].ps.stats[Stat::Buildable as usize] = k::SB_VALID_TOGGLEBIT | b.num() as i32,
        Err(E::Normal | E::NoCreep | E::NoRoom | E::NoOvermind | E::NoPower | E::RptNoReactor | E::RptPowerHere) => {
            w.clients[i].ps.stats[Stat::Buildable as usize] = b.num() as i32;
        }
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// `Cmd_ActivateItem_f` / `Cmd_ToggleItem_f` merged: toggles an upgrade or
/// switches between the blaster and the primary weapon.
pub fn cmd_toggle_item(w: &mut World, i: usize, name: &str) -> Result<(), String> {
    if let Some(wp) = WeaponId::from_id(name) {
        if !bg::inventory_contains_weapon(wp.num() as i32, &w.clients[i].ps.stats) && wp != WeaponId::Blaster {
            return Err(format!("You don't have the {name}"));
        }
        if !bg::player_can_change_weapon(&w.clients[i].ps) {
            return Ok(());
        }
        let target = if w.clients[i].ps.weapon != WeaponId::Blaster as i32 { WeaponId::Blaster as i32 } else { 0 };
        force_weapon_change(w, i, target);
        return Ok(());
    }
    let up = UpgradeId::from_id(name).ok_or_else(|| format!("You don't have the {name}"))?;
    let n = up.num() as i32;
    let stats = &mut w.clients[i].ps.stats;
    if !bg::inventory_contains_upgrade(n, stats) {
        return Err(format!("You don't have the {name}"));
    }
    if bg::upgrade_is_active(n, stats) {
        bg::deactivate_upgrade(n, stats);
    } else {
        bg::activate_upgrade(n, stats);
    }
    Ok(())
}

/// `Cmd_Kill_f` (no cheats: 20 second timer, toggles).
pub fn cmd_kill(w: &mut World, i: usize) -> Result<(), String> {
    let t = w.level.time;
    let c = &mut w.clients[i];
    if c.suicide_time == 0 {
        c.suicide_time = t + 20000;
        Ok(())
    } else {
        c.suicide_time = 0;
        Err("Suicide cancelled".into())
    }
}

/// `Cmd_Destroy_f` (deconstruct the structure in front, no marking).
pub fn cmd_deconstruct(w: &mut World, i: usize, destroy: bool) -> Result<(), String> {
    use super::buildable as bld;
    let ps = w.clients[i].ps;
    let view = bg::client_view_origin(&ps);
    let (fwd, _, _) = angle_vectors(&ps.viewangles);
    let end = ma(&view, 100.0, &fwd);
    let tr = w.trace(&view, None, None, &end, i as i32, k::MASK_PLAYERSOLID);
    let e = tr.entity_num;
    let builder = matches!(WeaponId::from_num(ps.weapon as u8), Some(WeaponId::Abuild) | Some(WeaponId::Abuildupg) | Some(WeaponId::Ckit));
    if !(tr.fraction < 1.0 && e >= 0 && (e as usize) < w.ents.len()) {
        return Err("Nothing to deconstruct".into());
    }
    let e = e as usize;
    let team = w.clients[i].team;
    if w.ents[e].etype != EType::Buildable || w.ents[e].team != team || !builder {
        return Err("Nothing to deconstruct".into());
    }
    let mark = w.level.mark_deconstruct;
    // always let the builder prevent the explosion
    if w.ents[e].health <= 0 {
        bld::queue_build_points(w, e);
        combat::reward_attackers(w, e);
        w.free(e);
        return Ok(());
    }
    // cancel deconstruction (unmark)
    if !destroy && mark != 0 && w.ents[e].deconstruct {
        w.ents[e].deconstruct = false;
        return Ok(());
    }
    let b = w.ents[e].buildable.unwrap();
    let last_spawn = (team == Team::Aliens && b == BuildableId::Eggpod && w.level.num_alien_spawns <= 1) || (team == Team::Humans && b == BuildableId::Telenode && w.level.num_human_spawns <= 1);
    if last_spawn && !w.level.cheats && mark == 0 {
        return Err("You may not deconstruct the last spawn".into());
    }
    if bld::time_til_sudden_death(w) <= 0 {
        return Err("Building is not allowed during Sudden Death".into());
    }
    let powered_ok = |w: &mut World| team != Team::Humans || bld::find_power(w, e, true);
    if (mark == 0 || !powered_ok(w)) && st(w, i, Stat::Misc) > 0 {
        w.add_event(i, Event::BuildDelay, i as i32);
        return Err("Build timer is still running".into());
    }
    if destroy {
        let h = w.ents[e].health;
        combat::damage(w, e, Some(i), Some(i), Some(fwd), Some(tr.endpos), h, 0, Mod::Suicide);
    } else if mark != 0 && (powered_ok(w) || last_spawn) {
        w.ents[e].deconstruct = true;
        w.ents[e].deconstruct_time = w.level.time;
    } else {
        if !w.level.cheats {
            w.clients[i].ps.stats[Stat::Misc as usize] += b.def().build_time / 4;
        }
        let h = w.ents[e].health;
        combat::damage(w, e, Some(i), Some(i), Some(fwd), Some(tr.endpos), h, 0, Mod::Deconstruct);
        w.free(e);
    }
    Ok(())
}

/// `Cmd_Reload_f`.
pub fn cmd_reload(w: &mut World, i: usize) -> Result<(), String> {
    let ps = &mut w.clients[i].ps;
    let wd = bg::weapon(ps.weapon);
    if wd.infinite_ammo || ps.clips <= 0 {
        return Ok(());
    }
    let mut max = wd.max_ammo;
    if wd.uses_energy && bg::inventory_contains_upgrade(UpgradeId::Battpack.num() as i32, &ps.stats) {
        max = (max as f32 * k::BATTPACK_MODIFIER) as i32;
    }
    if ps.ammo >= max {
        return Ok(());
    }
    if ps.weaponstate != WeaponState::Reloading as i32 {
        ps.pm_flags |= k::PMF_WEAPON_RELOAD;
    }
    Ok(())
}

/// `ClientCommand` dispatch for the gameplay subset ("buy rifle", "class level1", ...).
pub fn command(w: &mut World, i: usize, line: &str) -> Result<(), String> {
    super::cmds::client_command(w, i, line)
}

// ============================================================ names, idling

/// `BG_LoadEmoticons` (game side): the names of `emoticons/<name>_<width>x1.tga`
/// in the host's file system, into `level.emoticons` (G_InitGame). The
/// Tremulous 1.1 data ships none, so names keep "[...]" as typed then.
pub fn load_emoticons(w: &mut World) -> usize {
    const MAX_EMOTICONS: usize = 64;
    const MAX_EMOTICON_NAME_LEN: usize = 16;
    let files = w.fs.as_ref().map(|f| f.list("emoticons", "x1.tga")).unwrap_or_default();
    let mut out = Vec::new();
    for f in files.iter() {
        if out.len() >= MAX_EMOTICONS {
            break;
        }
        let b = f.as_bytes();
        let n = b.len();
        if n < 9 || b[n - 8] != b'_' || !(b'1'..=b'9').contains(&b[n - 7]) {
            w.console.push_str(&format!("^3skipping invalidly named emoticon \"{f}\"\n"));
            continue;
        }
        if n - 8 > MAX_EMOTICON_NAME_LEN {
            w.console.push_str(&format!("^3emoticon file name \"{f}\" too long (>{})\n", MAX_EMOTICON_NAME_LEN + 8));
            continue;
        }
        if !w.fs.as_ref().is_some_and(|fs| fs.exists(&format!("emoticons/{f}"))) {
            w.console.push_str(&format!("^3could not open \"emoticons/{f}\"\n"));
            continue;
        }
        out.push(f[..n - 8].to_string());
    }
    w.console.push_str(&format!("Loaded {} of {} emoticons (MAX_EMOTICONS is {MAX_EMOTICONS})\n", out.len(), files.len()));
    w.emoticons = out;
    w.emoticons.len()
}

/// `G_IsEmoticon`: "[name]" (or "[[name]" escaped) for a loaded emoticon.
fn is_emoticon(w: &World, s: &[u8]) -> Option<bool> {
    let mut p = 0;
    if s.first() != Some(&b'[') {
        return None;
    }
    p += 1;
    let mut escape = false;
    if s.get(p) == Some(&b'[') {
        escape = true;
        p += 1;
    }
    let mut name = String::new();
    // MAX_EMOTICON_NAME_LEN 16
    while p < s.len() && name.len() < 15 {
        if s[p] == b']' {
            return w.emoticons.iter().any(|e| e.eq_ignore_ascii_case(&name)).then_some(escape);
        }
        name.push(s[p] as char);
        p += 1;
    }
    None
}

/// `G_ClientCleanName`: strip leading/control characters, limit spaces,
/// keep colour codes, reject comment starters and empty names.
pub fn clean_name(w: &World, input: &str) -> String {
    let b = input.as_bytes();
    let out_size = super::cmds::MAX_NAME_LENGTH - 1;
    let mut out: Vec<u8> = Vec::new();
    let (mut len, mut colorless_len, mut spaces) = (0usize, 0usize, 0);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        // don't allow leading spaces
        if colorless_len == 0 && c == b' ' {
            i += 1;
            continue;
        }
        // don't allow nonprinting characters or (dead) console keys
        if c < b' ' || c > b'}' || c == b'`' {
            i += 1;
            continue;
        }
        if super::cmds::is_color_string(b, i) {
            i += 1;
            // make sure room in dest for both chars
            if len as isize > out_size as isize - 2 {
                break;
            }
            out.push(b'^');
            out.push(b[i]);
            len += 2;
            i += 1;
            continue;
        } else if !w.level.emoticons_allowed_in_names {
            if let Some(escaped) = is_emoticon(w, &b[i..]) {
                if len as isize > out_size as isize - 2 {
                    break;
                }
                out.extend_from_slice(b"[[");
                len += 2;
                if escaped {
                    i += 1;
                }
                i += 1;
                continue;
            }
        }
        // don't allow too many consecutive spaces
        if c == b' ' {
            spaces += 1;
            if spaces > 3 {
                i += 1;
                continue;
            }
        } else {
            spaces = 0;
        }
        if len as isize > out_size as isize - 1 {
            break;
        }
        out.push(c);
        colorless_len += 1;
        len += 1;
        i += 1;
    }
    let p = String::from_utf8_lossy(&out).into_owned();
    let invalid = p.len() >= 12 && p[..12].eq_ignore_ascii_case("[skipnotify]") || p.contains("//") || p.contains("/*") || p.is_empty() || colorless_len == 0;
    if invalid {
        return "UnnamedPlayer".into();
    }
    p
}

/// The name part of `ClientUserinfoChanged`: clean, rate-limit, admin name
/// check, censor, namelog. `force` is forceName (admin rename).
pub fn userinfo_changed(w: &mut World, i: usize, name: &str, force: bool) {
    let oldname = w.clients[i].name.clone();
    let newname = clean_name(w, name);
    if oldname == newname {
        return;
    }
    let t = w.level.time;
    let nl = w.clients[i].namelog;
    let (nct, nc, muted) = nl.map(|n| (w.namelogs[n].name_change_time, w.namelogs[n].name_changes, w.namelogs[n].muted)).unwrap_or((0, 0, false));
    let mut revert = None;
    if !force && nct != 0 && t - nct <= w.level.min_name_change_period * 1000 {
        revert = Some(format!("Name change spam protection (g_minNameChangePeriod = {})", w.level.min_name_change_period));
    } else if !force && w.level.max_name_changes > 0 && nc >= w.level.max_name_changes {
        revert = Some(format!("Maximum name changes reached (g_maxNameChanges = {})", w.level.max_name_changes));
    } else if !force && muted {
        revert = Some("You cannot change your name while you are muted".into());
    } else if let Err(e) = super::admin::name_check(w, i, &newname) {
        revert = Some(e);
    }
    if let Some(msg) = revert {
        notify(w, i, msg);
        w.clients[i].name = if oldname.is_empty() { "UnnamedPlayer".into() } else { oldname };
    } else {
        w.clients[i].name = super::cmds::censor_string(w, &newname, Some(i));
        // pers.connected == CON_CONNECTED: not while connecting
        if !force && !oldname.is_empty() {
            if let Some(n) = nl {
                w.namelogs[n].name_change_time = t;
                w.namelogs[n].name_changes += 1;
            }
        }
        if !oldname.is_empty() {
            // pers.ip.str (empty for the host's player and bots)
            let (name, guid, ip) = (w.clients[i].name.clone(), w.clients[i].guid.clone(), w.clients[i].ip.str.clone());
            let line = format!("ClientRename: {i} [{ip}] ({guid}) \"{oldname}^7\" -> \"{name}^7\" \"{}{name}{}^7\"\n", super::cmds::DECOLOR_OFF as char, super::cmds::DECOLOR_ON as char);
            super::log::log_printf(w, &line);
        }
    }
    super::admin::namelog_update_name(w, i);
}

/// `ClientInactivityTimer`: false when the client was dropped.
pub fn inactivity_timer(w: &mut World, i: usize) -> bool {
    let t = w.level.time;
    let inact = w.level.inactivity;
    let c = &mut w.clients[i];
    if inact == 0 {
        // give everyone some time, so if the operator sets g_inactivity during
        // gameplay, everyone isn't kicked
        c.inactivity_time = t + 60 * 1000;
        c.inactivity_warning = false;
    } else if c.cmd.forwardmove != 0 || c.cmd.rightmove != 0 || c.cmd.upmove != 0 || c.cmd.buttons & k::BUTTON_ATTACK != 0 {
        c.inactivity_time = t + inact * 1000;
        c.inactivity_warning = false;
    } else if !c.local_client {
        let (it, warned) = (c.inactivity_time, c.inactivity_warning);
        if t > it && !super::admin::permission(w, Some(i), super::admin::ADMF_ACTIVITY) {
            super::admin::drop_client(w, i, "Dropped due to inactivity");
            return false;
        }
        if t > it - 10000 && !warned && !super::admin::permission(w, Some(i), super::admin::ADMF_ACTIVITY) {
            w.clients[i].inactivity_warning = true;
            w.center_prints.push((t, None, Some(i), "Ten seconds until inactivity drop!".into()));
        }
    }
    true
}
