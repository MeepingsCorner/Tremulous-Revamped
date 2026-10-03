//! Kernel `trigger` — game/g_trigger.c, g_target.c and g_misc.c: brush
//! triggers, target_* relays, map decoration entities, G_TouchTriggers
//! (g_active.c) and the buildable variant (g_buildable.c).

use super::bg;
use super::combat::{self, DAMAGE_NO_PROTECTION};
use super::qmath::*;
use super::spawn::*;
use super::types::*;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::buildables::BuildableId;
use crate::gen::classes::ClassId;
use crate::gen::enums::{Event, MeansOfDeath as Mod, PmType, SpectatorState, Stat, Team, WeaponState};
use crate::gen::upgrades::UpgradeId;
use crate::gen::weapons::WeaponId;

fn is_client(w: &World, i: usize) -> bool {
    i < MAX_CLIENTS && w.ents[i].client
}
fn stat_team(w: &World, i: usize) -> i32 {
    w.clients[i].ps.stats[Stat::Team as usize]
}
fn time_plus(t: i32, f: f64) -> i32 {
    (t as f64 + f) as i32
}

/// `InitTrigger`.
fn init_trigger(w: &mut World, ent: usize) {
    if w.ents[ent].s_angles != [0.0; 3] {
        let e = &mut w.ents[ent];
        e.movedir = set_movedir(&mut e.s_angles);
    }
    super::mover::set_brush_model(w, ent);
    w.ents[ent].contents = CONTENTS_TRIGGER;
    w.ents[ent].svflags = SVF_NOCLIENT;
}

/// The (wait + random * crandom()) * 1000 re-arm delay of multi triggers.
fn rearm(w: &mut World, ent: usize) {
    if w.ents[ent].wait > 0.0 {
        let c = w.rng.crandom();
        let (wait, random) = (w.ents[ent].wait as f64, w.ents[ent].random as f64);
        w.ents[ent].think = Some(multi_wait);
        w.ents[ent].nextthink = time_plus(w.level.time, (wait + random * c) * 1000.0);
    } else {
        w.ents[ent].touch = None;
        w.ents[ent].nextthink = w.level.time + FRAMETIME;
        w.ents[ent].think = Some(free_entity);
    }
}

/// `G_FreeEntity` as a think function.
pub fn free_entity(w: &mut World, ent: usize) {
    w.free(ent);
}

/// `multi_wait`.
pub fn multi_wait(w: &mut World, ent: usize) {
    w.ents[ent].nextthink = 0;
}

/// `multi_trigger`.
fn multi_trigger(w: &mut World, ent: usize, activator: Option<usize>) {
    w.ents[ent].activator = activator;
    if w.ents[ent].nextthink != 0 {
        return;
    }
    if let Some(a) = activator.filter(|&a| is_client(w, a)) {
        if w.ents[ent].spawnflags & 1 != 0 && stat_team(w, a) != Team::Humans as i32 {
            return;
        }
        if w.ents[ent].spawnflags & 2 != 0 && stat_team(w, a) != Team::Aliens as i32 {
            return;
        }
    }
    let a = w.ents[ent].activator;
    use_targets(w, ent, a);
    rearm(w, ent);
}

pub fn use_multi(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    multi_trigger(w, ent, activator);
}
pub fn touch_multi(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) && w.ents[other].etype != EType::Buildable {
        return;
    }
    multi_trigger(w, ent, Some(other));
}

fn wait_random(w: &mut World, ent: usize, wait_default: &str) {
    w.ents[ent].wait = spawn_float(w, "wait", wait_default).1;
    w.ents[ent].random = spawn_float(w, "random", "0").1;
    let e = &mut w.ents[ent];
    if e.random >= e.wait && e.wait >= 0.0 {
        e.random = e.wait - FRAMETIME as f32;
    }
}

/// `SP_trigger_multiple`.
pub fn sp_trigger_multiple(w: &mut World, ent: usize) {
    wait_random(w, ent, "0.5");
    w.ents[ent].touch = Some(touch_multi);
    w.ents[ent].use_ = Some(use_multi);
    init_trigger(w, ent);
    w.link(ent);
}

pub fn trigger_always_think(w: &mut World, ent: usize) {
    use_targets(w, ent, Some(ent));
    w.free(ent);
}
/// `SP_trigger_always`.
pub fn sp_trigger_always(w: &mut World, ent: usize) {
    w.ents[ent].nextthink = w.level.time + 300;
    w.ents[ent].think = Some(trigger_always_think);
}

/// `trigger_push_touch` (Tremulous does not predict or apply jump pads).
pub fn trigger_push_touch(_w: &mut World, _ent: usize, _other: usize) {}

/// `AimAtTarget`.
pub fn aim_at_target(w: &mut World, ent: usize) {
    let (lo, hi) = w.ents[ent].abs_bounds();
    let origin = scale(&add(&lo, &hi), 0.5);
    let tgt = w.ents[ent].target.clone();
    let Some(t) = pick_target(w, tgt.as_deref()) else {
        w.free(ent);
        return;
    };
    let height = w.ents[t].s_origin[2] - origin[2];
    let gravity = w.level.g_gravity;
    let time = ((height as f64) / (0.5 * gravity as f64)).sqrt() as f32;
    if time == 0.0 {
        w.free(ent);
        return;
    }
    let mut o2 = sub(&w.ents[t].s_origin, &origin);
    o2[2] = 0.0;
    let dist = normalize(&mut o2);
    let forward = dist / time;
    o2 = scale(&o2, forward);
    o2[2] = time * gravity;
    w.ents[ent].origin2 = o2;
}

/// `SP_trigger_push`.
pub fn sp_trigger_push(w: &mut World, ent: usize) {
    init_trigger(w, ent);
    let e = &mut w.ents[ent];
    e.svflags &= !SVF_NOCLIENT;
    e.etype = EType::PushTrigger;
    e.touch = Some(trigger_push_touch);
    e.think = Some(aim_at_target);
    e.nextthink = w.level.time + FRAMETIME;
    w.link(ent);
}

/// `Use_target_push`.
pub fn use_target_push(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let Some(a) = activator.filter(|&a| is_client(w, a)) else { return };
    if w.clients[a].ps.pm_type != PmType::Normal as i32 {
        return;
    }
    w.clients[a].ps.velocity = w.ents[ent].origin2;
}

/// `SP_target_push`.
pub fn sp_target_push(w: &mut World, ent: usize) {
    if w.ents[ent].speed == 0.0 {
        w.ents[ent].speed = 1000.0;
    }
    let e = &mut w.ents[ent];
    let d = set_movedir(&mut e.s_angles);
    e.origin2 = scale(&d, e.speed);
    if e.target.is_some() {
        // r.absmin = r.absmax = s.origin
        e.mins = [0.0; 3];
        e.maxs = [0.0; 3];
        e.origin = e.s_origin;
        e.think = Some(aim_at_target);
        e.nextthink = w.level.time + FRAMETIME;
    }
    e.use_ = Some(use_target_push);
}

/// `trigger_teleporter_touch`.
pub fn trigger_teleporter_touch(w: &mut World, ent: usize, other: usize) {
    if w.ents[ent].e_flags & k::EF_NODRAW != 0 || !is_client(w, other) {
        return;
    }
    if w.clients[other].ps.pm_type == PmType::Dead as i32 {
        return;
    }
    if w.ents[ent].spawnflags & 1 != 0 && w.clients[other].spectator == SpectatorState::Not as i32 {
        return;
    }
    let tgt = w.ents[ent].target.clone();
    let Some(d) = pick_target(w, tgt.as_deref()) else { return };
    let (o, a) = (w.ents[d].s_origin, w.ents[d].s_angles);
    teleport_player(w, other, &o, &a);
}
pub fn trigger_teleporter_use(w: &mut World, ent: usize, _o: Option<usize>, _a: Option<usize>) {
    w.ents[ent].e_flags ^= k::EF_NODRAW;
}
/// `SP_trigger_teleport`.
pub fn sp_trigger_teleport(w: &mut World, ent: usize) {
    init_trigger(w, ent);
    let e = &mut w.ents[ent];
    if e.spawnflags & 1 != 0 {
        e.svflags |= SVF_NOCLIENT;
    } else {
        e.svflags &= !SVF_NOCLIENT;
    }
    if e.spawnflags & 2 != 0 {
        e.e_flags |= k::EF_NODRAW;
    }
    e.etype = EType::TeleportTrigger;
    e.touch = Some(trigger_teleporter_touch);
    e.use_ = Some(trigger_teleporter_use);
    w.link(ent);
}

/// Toggle linked state (hurt_use / trigger_gravity_use / trigger_heal_use).
pub fn toggle_link_use(w: &mut World, ent: usize, _o: Option<usize>, _a: Option<usize>) {
    if w.ents[ent].linked {
        w.unlink(ent);
    } else {
        w.link(ent);
    }
}

/// `hurt_touch`.
pub fn hurt_touch(w: &mut World, ent: usize, other: usize) {
    if !w.ents[other].takedamage || w.ents[ent].timestamp > w.level.time {
        return;
    }
    w.ents[ent].timestamp = w.level.time + if w.ents[ent].spawnflags & 16 != 0 { 1000 } else { FRAMETIME };
    if w.ents[ent].spawnflags & 4 == 0 {
        let o = w.ents[other].origin;
        let idx = w.ents[ent].noise_index;
        w.temp_event(o, Event::GeneralSound, -1, idx, 0);
    }
    let dflags = if w.ents[ent].spawnflags & 8 != 0 { DAMAGE_NO_PROTECTION } else { 0 };
    let d = w.ents[ent].damage;
    combat::damage(w, other, Some(ent), Some(ent), None, None, d, dflags, Mod::TriggerHurt);
}

/// `SP_trigger_hurt`.
pub fn sp_trigger_hurt(w: &mut World, ent: usize) {
    init_trigger(w, ent);
    w.ents[ent].noise_index = w.sound_index("sound/misc/electro.wav");
    let e = &mut w.ents[ent];
    e.touch = Some(hurt_touch);
    if e.damage <= 0 {
        e.damage = 5;
    }
    e.contents = CONTENTS_TRIGGER;
    if e.spawnflags & 2 != 0 {
        e.use_ = Some(toggle_link_use);
    }
    if e.spawnflags & 1 == 0 {
        w.link(ent);
    } else {
        w.unlink(ent);
    }
}

/// `func_timer_think`.
pub fn func_timer_think(w: &mut World, ent: usize) {
    let a = w.ents[ent].activator;
    use_targets(w, ent, a);
    let c = w.rng.crandom();
    let (wait, random) = (w.ents[ent].wait as f64, w.ents[ent].random as f64);
    w.ents[ent].nextthink = time_plus(w.level.time, 1000.0 * (wait + c * random));
}
pub fn func_timer_use(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    w.ents[ent].activator = activator;
    if w.ents[ent].nextthink != 0 {
        w.ents[ent].nextthink = 0;
        return;
    }
    func_timer_think(w, ent);
}
/// `SP_func_timer`.
pub fn sp_func_timer(w: &mut World, ent: usize) {
    w.ents[ent].random = spawn_float(w, "random", "1").1;
    w.ents[ent].wait = spawn_float(w, "wait", "1").1;
    let e = &mut w.ents[ent];
    e.use_ = Some(func_timer_use);
    e.think = Some(func_timer_think);
    if e.random >= e.wait {
        e.random = e.wait - FRAMETIME as f32;
    }
    if e.spawnflags & 1 != 0 {
        e.nextthink = w.level.time + FRAMETIME;
        e.activator = Some(ent);
    }
    e.svflags = SVF_NOCLIENT;
}

/// `G_Checktrigger_stages`: called when a team reaches a new stage (0-based).
pub fn check_trigger_stages(w: &mut World, team: Team, stage: i32) {
    for i in 1..w.num_entities {
        if !w.ents[i].inuse || w.ents[i].classname != "trigger_stage" {
            continue;
        }
        if team as i32 == w.ents[i].stage_team && stage == w.ents[i].stage_stage {
            if let Some(u) = w.ents[i].use_ {
                u(w, i, Some(i), Some(i));
            }
        }
    }
}

/// The trigger_win pass of LogExit.
pub fn fire_trigger_win(w: &mut World, winner: Team) {
    for i in 1..w.num_entities {
        if !w.ents[i].inuse || w.ents[i].classname != "trigger_win" {
            continue;
        }
        if winner as i32 == w.ents[i].stage_team {
            if let Some(u) = w.ents[i].use_ {
                u(w, i, Some(i), Some(i));
            }
        }
    }
}

pub fn trigger_stage_use(w: &mut World, ent: usize, _o: Option<usize>, _a: Option<usize>) {
    use_targets(w, ent, Some(ent));
}
/// `SP_trigger_stage`.
pub fn sp_trigger_stage(w: &mut World, ent: usize) {
    w.ents[ent].stage_team = spawn_int(w, "team", "0").1;
    w.ents[ent].stage_stage = spawn_int(w, "stage", "0").1;
    w.ents[ent].use_ = Some(trigger_stage_use);
    w.ents[ent].svflags = SVF_NOCLIENT;
}
/// `SP_trigger_win`.
pub fn sp_trigger_win(w: &mut World, ent: usize) {
    w.ents[ent].stage_team = spawn_int(w, "team", "0").1;
    w.ents[ent].use_ = Some(trigger_stage_use);
    w.ents[ent].svflags = SVF_NOCLIENT;
}

/// The NEGATE-aware fire step shared by trigger_buildable/class/equipment.
fn filtered_fire(w: &mut World, ent: usize, activator: usize, matched: bool) {
    let negate = w.ents[ent].e_flags & k::EF_DEAD != 0;
    if matched != negate {
        use_targets(w, ent, Some(activator));
    }
    rearm(w, ent);
}

fn buildable_match(w: &World, ent: usize, a: usize) -> bool {
    let list = &w.ents[ent].b_triggers;
    list.is_empty() || w.ents[a].buildable.is_some_and(|b| list.contains(&(b.num() as i32)))
}
pub fn trigger_buildable_touch(w: &mut World, ent: usize, other: usize) {
    if w.ents[other].etype != EType::Buildable {
        return;
    }
    w.ents[ent].activator = Some(other);
    if w.ents[ent].e_flags & k::EF_NODRAW != 0 || w.ents[ent].nextthink != 0 {
        return;
    }
    let m = buildable_match(w, ent, other);
    filtered_fire(w, ent, other, m);
}
pub fn toggle_nodraw_use(w: &mut World, ent: usize, _o: Option<usize>, _a: Option<usize>) {
    w.ents[ent].e_flags ^= k::EF_NODRAW;
}

fn csv(s: &str) -> Vec<String> {
    s.split(',').map(|t| t.trim_start_matches(' ').to_string()).filter(|t| !t.is_empty()).collect()
}

fn filtered_spawn(w: &mut World, ent: usize, touch: super::sig::Touch) {
    wait_random(w, ent, "0.5");
    let e = &mut w.ents[ent];
    e.touch = Some(touch);
    e.use_ = Some(toggle_nodraw_use);
    if e.spawnflags & 1 != 0 {
        e.e_flags |= k::EF_NODRAW;
    }
    if e.spawnflags & 2 != 0 {
        e.e_flags |= k::EF_DEAD;
    }
    init_trigger(w, ent);
    w.link(ent);
}

/// `SP_trigger_buildable`.
pub fn sp_trigger_buildable(w: &mut World, ent: usize) {
    let list = spawn_string(w, "buildables", "").1;
    w.ents[ent].b_triggers = csv(&list).iter().filter_map(|n| BuildableId::from_id(&n.to_ascii_lowercase())).map(|b| b.num() as i32).collect();
    filtered_spawn(w, ent, trigger_buildable_touch);
}

pub fn trigger_class_touch(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) || stat_team(w, other) != Team::Aliens as i32 {
        return;
    }
    if w.ents[ent].e_flags & k::EF_NODRAW != 0 {
        return;
    }
    w.ents[ent].activator = Some(other);
    if w.ents[ent].nextthink != 0 {
        return;
    }
    let list = &w.ents[ent].c_triggers;
    let m = list.is_empty() || list.contains(&w.clients[other].ps.stats[Stat::Class as usize]);
    filtered_fire(w, ent, other, m);
}
/// `SP_trigger_class`.
pub fn sp_trigger_class(w: &mut World, ent: usize) {
    let list = spawn_string(w, "classes", "").1;
    w.ents[ent].c_triggers = csv(&list).iter().filter_map(|n| ClassId::from_id(&n.to_ascii_lowercase())).map(|c| c.num() as i32).collect();
    filtered_spawn(w, ent, trigger_class_touch);
}

pub fn trigger_equipment_touch(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) || stat_team(w, other) != Team::Humans as i32 {
        return;
    }
    if w.ents[ent].e_flags & k::EF_NODRAW != 0 {
        return;
    }
    w.ents[ent].activator = Some(other);
    if w.ents[ent].nextthink != 0 {
        return;
    }
    let (wl, ul) = (&w.ents[ent].w_triggers, &w.ents[ent].u_triggers);
    let stats = &w.clients[other].ps.stats;
    let m = (wl.is_empty() && ul.is_empty()) || wl.iter().any(|&x| bg::inventory_contains_weapon(x, stats)) || ul.iter().any(|&u| bg::inventory_contains_upgrade(u, stats));
    filtered_fire(w, ent, other, m);
}
/// `SP_trigger_equipment`.
pub fn sp_trigger_equipment(w: &mut World, ent: usize) {
    let list = spawn_string(w, "equipment", "").1;
    let names = csv(&list);
    w.ents[ent].w_triggers = names.iter().filter_map(|n| WeaponId::from_id(&n.to_ascii_lowercase())).map(|x| x.num() as i32).collect();
    w.ents[ent].u_triggers = names.iter().filter_map(|n| UpgradeId::from_id(&n.to_ascii_lowercase())).map(|x| x.num() as i32).collect();
    filtered_spawn(w, ent, trigger_equipment_touch);
}

pub fn trigger_gravity_touch(w: &mut World, ent: usize, other: usize) {
    if is_client(w, other) {
        w.clients[other].ps.gravity = w.ents[ent].trigger_gravity;
    }
}
/// `SP_trigger_gravity`.
pub fn sp_trigger_gravity(w: &mut World, ent: usize) {
    w.ents[ent].trigger_gravity = spawn_int(w, "gravity", "800").1;
    w.ents[ent].touch = Some(trigger_gravity_touch);
    w.ents[ent].use_ = Some(toggle_link_use);
    init_trigger(w, ent);
    w.link(ent);
}

pub fn trigger_heal_touch(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) || w.ents[ent].timestamp > w.level.time {
        return;
    }
    w.ents[ent].timestamp = w.level.time + if w.ents[ent].spawnflags & 2 != 0 { 1000 } else { FRAMETIME };
    let max = w.clients[other].ps.stats[Stat::MaxHealth as usize];
    w.ents[other].health += w.ents[ent].damage;
    if w.ents[other].health > max {
        w.ents[other].health = max;
    }
    w.clients[other].ps.stats[Stat::Health as usize] = w.ents[other].health;
}
/// `SP_trigger_heal`.
pub fn sp_trigger_heal(w: &mut World, ent: usize) {
    w.ents[ent].damage = spawn_int(w, "heal", "5").1;
    if w.ents[ent].damage <= 0 {
        w.ents[ent].damage = 1;
    }
    w.ents[ent].touch = Some(trigger_heal_touch);
    w.ents[ent].use_ = Some(toggle_link_use);
    init_trigger(w, ent);
    if w.ents[ent].spawnflags & 1 == 0 {
        w.link(ent);
    } else {
        w.unlink(ent);
    }
}

pub fn trigger_ammo_touch(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) || stat_team(w, other) != Team::Humans as i32 || w.ents[ent].timestamp > w.level.time {
        return;
    }
    if w.clients[other].ps.weaponstate != WeaponState::Ready as i32 {
        return;
    }
    let wpn = w.clients[other].ps.stats[Stat::Weapon as usize];
    let wd = bg::weapon(wpn);
    if wd.uses_energy && w.ents[ent].spawnflags & 2 != 0 {
        return;
    }
    if !wd.uses_energy && w.ents[ent].spawnflags & 4 != 0 {
        return;
    }
    w.ents[ent].timestamp = w.level.time + if w.ents[ent].spawnflags & 1 != 0 { 1000 } else { FRAMETIME };
    let (max_ammo, max_clips) = (wd.max_ammo, wd.max_clips);
    let add = w.ents[ent].damage;
    let ps = &mut w.clients[other].ps;
    if ps.ammo + add > max_ammo {
        if ps.clips < max_clips {
            ps.clips += 1;
            ps.ammo = 1;
        } else {
            ps.ammo = max_ammo;
        }
    } else {
        ps.ammo += add;
    }
}
/// `SP_trigger_ammo`.
pub fn sp_trigger_ammo(w: &mut World, ent: usize) {
    w.ents[ent].damage = spawn_int(w, "ammo", "1").1;
    if w.ents[ent].damage <= 0 {
        w.ents[ent].damage = 1;
    }
    w.ents[ent].touch = Some(trigger_ammo_touch);
    init_trigger(w, ent);
    w.link(ent);
}

// ============================================================ g_target.c

pub fn think_target_delay(w: &mut World, ent: usize) {
    let a = w.ents[ent].activator;
    use_targets(w, ent, a);
}
pub fn use_target_delay(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let c = w.rng.crandom();
    let (wait, random) = (w.ents[ent].wait as f64, w.ents[ent].random as f64);
    w.ents[ent].nextthink = time_plus(w.level.time, (wait + random * c) * 1000.0);
    w.ents[ent].think = Some(think_target_delay);
    w.ents[ent].activator = activator;
}
/// `SP_target_delay`.
pub fn sp_target_delay(w: &mut World, ent: usize) {
    let (p, d) = spawn_float(w, "delay", "0");
    w.ents[ent].wait = d;
    if !p {
        w.ents[ent].wait = spawn_float(w, "wait", "1").1;
    }
    if w.ents[ent].wait == 0.0 {
        w.ents[ent].wait = 1.0;
    }
    w.ents[ent].use_ = Some(use_target_delay);
}

pub fn use_target_score(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let Some(a) = activator else { return };
    if is_client(w, a) {
        let c = w.ents[ent].count;
        combat::add_score(w, a, c);
    }
}
/// `SP_target_score`.
pub fn sp_target_score(w: &mut World, ent: usize) {
    if w.ents[ent].count == 0 {
        w.ents[ent].count = 1;
    }
    w.ents[ent].use_ = Some(use_target_score);
}

/// `Use_Target_Print`: centre-print queued for the presentation layer.
pub fn use_target_print(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let msg = w.ents[ent].message.clone().unwrap_or_default();
    let t = w.level.time;
    let sf = w.ents[ent].spawnflags;
    if sf & 4 != 0 {
        if let Some(a) = activator.filter(|&a| is_client(w, a)) {
            w.center_prints.push((t, None, Some(a), msg));
        }
        return;
    }
    if sf & 3 != 0 {
        if sf & 1 != 0 {
            w.center_prints.push((t, Some(Team::Humans), None, msg.clone()));
        }
        if sf & 2 != 0 {
            w.center_prints.push((t, Some(Team::Aliens), None, msg));
        }
        return;
    }
    w.center_prints.push((t, None, None, msg));
}
/// `SP_target_print`.
pub fn sp_target_print(w: &mut World, ent: usize) {
    w.ents[ent].use_ = Some(use_target_print);
}

/// `Use_Target_Speaker`.
pub fn use_target_speaker(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let idx = w.ents[ent].noise_index;
    let sf = w.ents[ent].spawnflags;
    if sf & 3 != 0 {
        w.ents[ent].loop_sound = if w.ents[ent].loop_sound != 0 { 0 } else { idx };
    } else if sf & 8 != 0 && activator.is_some() {
        w.add_event(activator.unwrap(), Event::GeneralSound, idx);
    } else if sf & 4 != 0 {
        w.add_event(ent, Event::GlobalSound, idx);
    } else {
        w.add_event(ent, Event::GeneralSound, idx);
    }
}
/// `SP_target_speaker`.
pub fn sp_target_speaker(w: &mut World, ent: usize) {
    w.ents[ent].wait = spawn_float(w, "wait", "0").1;
    w.ents[ent].random = spawn_float(w, "random", "0").1;
    let (p, s) = spawn_string(w, "noise", "NOSOUND");
    if !p {
        // G_Error( "target_speaker without a noise key" ): drop the entity
        w.free(ent);
        return;
    }
    if s.starts_with('*') {
        w.ents[ent].spawnflags |= 8;
    }
    let buffer = if !s.contains(".wav") { format!("{s}.wav") } else { s };
    let idx = w.sound_index(&buffer);
    let e = &mut w.ents[ent];
    e.noise_index = idx;
    e.etype = EType::Speaker;
    e.event_parm = idx;
    e.frame = (e.wait * 10.0) as i32;
    e.s_client_num = (e.random * 10.0) as i32;
    if e.spawnflags & 1 != 0 {
        e.loop_sound = idx;
    }
    e.use_ = Some(use_target_speaker);
    if e.spawnflags & 4 != 0 {
        e.svflags |= SVF_BROADCAST;
    }
    e.pos.tr_base = e.s_origin;
    w.link(ent);
}

pub fn target_teleporter_use(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let Some(a) = activator.filter(|&a| is_client(w, a)) else { return };
    let tgt = w.ents[ent].target.clone();
    let Some(d) = pick_target(w, tgt.as_deref()) else { return };
    let (o, an) = (w.ents[d].s_origin, w.ents[d].s_angles);
    teleport_player(w, a, &o, &an);
}
/// `SP_target_teleporter`.
pub fn sp_target_teleporter(w: &mut World, ent: usize) {
    w.ents[ent].use_ = Some(target_teleporter_use);
}

/// `target_relay_use`.
pub fn target_relay_use(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let sf = w.ents[ent].spawnflags;
    if let Some(a) = activator.filter(|&a| is_client(w, a)) {
        if sf & 1 != 0 && stat_team(w, a) != Team::Humans as i32 {
            return;
        }
        if sf & 2 != 0 && stat_team(w, a) != Team::Aliens as i32 {
            return;
        }
    }
    if sf & 4 != 0 {
        let tgt = w.ents[ent].target.clone();
        if let Some(t) = pick_target(w, tgt.as_deref()) {
            if let Some(u) = w.ents[t].use_ {
                u(w, t, Some(ent), activator);
            }
        }
        return;
    }
    use_targets(w, ent, activator);
}
/// `SP_target_relay`.
pub fn sp_target_relay(w: &mut World, ent: usize) {
    w.ents[ent].use_ = Some(target_relay_use);
}

pub fn target_kill_use(w: &mut World, _ent: usize, _o: Option<usize>, activator: Option<usize>) {
    if let Some(a) = activator {
        combat::damage(w, a, None, None, None, None, 100000, DAMAGE_NO_PROTECTION, Mod::Telefrag);
    }
}
/// `SP_target_kill`.
pub fn sp_target_kill(w: &mut World, ent: usize) {
    w.ents[ent].use_ = Some(target_kill_use);
}

/// `SP_target_position` / `SP_info_notnull`.
pub fn sp_target_position(w: &mut World, ent: usize) {
    let o = w.ents[ent].s_origin;
    set_origin(w, ent, o);
}

/// `SP_target_location`.
pub fn sp_target_location(w: &mut World, ent: usize) {
    w.ents[ent].etype = EType::Location;
    w.ents[ent].svflags = SVF_BROADCAST;
    w.link(ent);
    if w.location_names.len() >= 64 {
        return;
    }
    let mut msg = w.ents[ent].message.clone().unwrap_or_default();
    if w.ents[ent].count != 0 {
        let c = w.ents[ent].count.clamp(0, 7);
        w.ents[ent].count = c;
        msg = format!("^{}{}^7", c, msg);
    }
    w.location_names.push(msg);
    w.ents[ent].next_train = w.level.location_head;
    w.ents[ent].generic1 = w.location_names.len() as i32 - 1;
    w.level.location_head = Some(ent);
    let o = w.ents[ent].s_origin;
    set_origin(w, ent, o);
}

/// `target_rumble_think`.
pub fn target_rumble_think(w: &mut World, ent: usize) {
    if w.ents[ent].last_move_time < w.level.time {
        // level.time + 0.5 truncated back into an int
        w.ents[ent].last_move_time = w.level.time;
    }
    let speed = w.ents[ent].speed;
    for i in 0..w.num_entities {
        if !w.ents[i].inuse || !is_client(w, i) {
            continue;
        }
        if w.clients[i].ps.ground_entity_num == ENTITYNUM_NONE {
            continue;
        }
        w.clients[i].ps.ground_entity_num = ENTITYNUM_NONE;
        let (c1, c2) = (w.rng.crandom(), w.rng.crandom());
        let v = &mut w.clients[i].ps.velocity;
        v[0] = (v[0] as f64 + c1 * 150.0) as f32;
        v[1] = (v[1] as f64 + c2 * 150.0) as f32;
        v[2] = speed;
    }
    if w.level.time < w.ents[ent].timestamp {
        w.ents[ent].nextthink = w.level.time + FRAMETIME;
    }
}
pub fn target_rumble_use(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let e = &mut w.ents[ent];
    e.timestamp = w.level.time + e.count * FRAMETIME;
    e.nextthink = w.level.time + FRAMETIME;
    e.activator = activator;
    e.last_move_time = 0;
}
/// `SP_target_rumble`.
pub fn sp_target_rumble(w: &mut World, ent: usize) {
    let e = &mut w.ents[ent];
    if e.count == 0 {
        e.count = 10;
    }
    if e.speed == 0.0 {
        e.speed = 100.0;
    }
    e.think = Some(target_rumble_think);
    e.use_ = Some(target_rumble_use);
}

pub fn target_alien_win_use(w: &mut World, _e: usize, _o: Option<usize>, _a: Option<usize>) {
    if !w.level.uncond_human_win {
        w.level.uncond_alien_win = true;
    }
}
pub fn target_human_win_use(w: &mut World, _e: usize, _o: Option<usize>, _a: Option<usize>) {
    if !w.level.uncond_alien_win {
        w.level.uncond_human_win = true;
    }
}
pub fn sp_target_alien_win(w: &mut World, ent: usize) {
    w.ents[ent].use_ = Some(target_alien_win_use);
}
pub fn sp_target_human_win(w: &mut World, ent: usize) {
    w.ents[ent].use_ = Some(target_human_win_use);
}

pub fn target_hurt_use(w: &mut World, ent: usize, _o: Option<usize>, activator: Option<usize>) {
    let Some(a) = activator else { return };
    if !w.ents[a].takedamage {
        return;
    }
    let d = w.ents[ent].damage;
    combat::damage(w, a, Some(ent), Some(ent), None, None, d, 0, Mod::TriggerHurt);
}
/// `SP_target_hurt`.
pub fn sp_target_hurt(w: &mut World, ent: usize) {
    if w.ents[ent].damage == 0 {
        w.ents[ent].damage = 5;
    }
    w.ents[ent].use_ = Some(target_hurt_use);
}

// ============================================================== g_misc.c

/// `SP_info_null` / `SP_light` / `SP_misc_model` / func_group: removed.
pub fn sp_free(w: &mut World, ent: usize) {
    w.free(ent);
}
/// `SP_misc_teleporter_dest`: positional only.
pub fn sp_nothing(_w: &mut World, _ent: usize) {}

/// `locateCamera`.
pub fn locate_camera(w: &mut World, ent: usize) {
    let tgt = w.ents[ent].target.clone();
    let Some(owner) = pick_target(w, tgt.as_deref()) else {
        w.free(ent);
        return;
    };
    w.ents[ent].owner_num = owner as i32;
    let osf = w.ents[owner].spawnflags;
    if osf & 1 != 0 {
        w.ents[ent].frame = 25;
    } else if osf & 2 != 0 {
        w.ents[ent].frame = 75;
    }
    w.ents[ent].misc = if osf & 4 != 0 { 0 } else { 1 };
    w.ents[ent].s_client_num = w.ents[owner].s_client_num;
    w.ents[ent].origin2 = w.ents[owner].s_origin;
    let otgt = w.ents[owner].target.clone();
    let dir = match pick_target(w, otgt.as_deref()) {
        Some(t) => {
            let mut d = sub(&w.ents[t].s_origin, &w.ents[owner].s_origin);
            normalize(&mut d);
            d
        }
        None => {
            let mut a = w.ents[owner].s_angles;
            set_movedir(&mut a)
        }
    };
    w.ents[ent].event_parm = dir_to_byte(&dir);
}
/// `SP_misc_portal_surface`.
pub fn sp_misc_portal_surface(w: &mut World, ent: usize) {
    w.ents[ent].mins = [0.0; 3];
    w.ents[ent].maxs = [0.0; 3];
    w.link(ent);
    w.ents[ent].svflags = SVF_PORTAL;
    w.ents[ent].etype = EType::Portal;
    if w.ents[ent].target.is_none() {
        w.ents[ent].origin2 = w.ents[ent].s_origin;
    } else {
        w.ents[ent].think = Some(locate_camera);
        w.ents[ent].nextthink = w.level.time + 100;
    }
}
/// `SP_misc_portal_camera`.
pub fn sp_misc_portal_camera(w: &mut World, ent: usize) {
    w.ents[ent].mins = [0.0; 3];
    w.ents[ent].maxs = [0.0; 3];
    w.link(ent);
    let roll = spawn_float(w, "roll", "0").1;
    w.ents[ent].s_client_num = (roll / 360.0f32 * 256.0) as i32;
}

pub fn toggle_particle_system(w: &mut World, ent: usize) {
    w.ents[ent].e_flags ^= k::EF_NODRAW;
    w.ents[ent].nextthink = 0;
}
pub fn use_particle_system(w: &mut World, ent: usize, _o: Option<usize>, _a: Option<usize>) {
    toggle_particle_system(w, ent);
    if w.ents[ent].wait > 0.0 {
        w.ents[ent].think = Some(toggle_particle_system);
        w.ents[ent].nextthink = w.level.time + (w.ents[ent].wait * 1000.0) as i32;
    }
}
/// `SP_misc_particle_system`.
pub fn sp_misc_particle_system(w: &mut World, ent: usize) {
    let o = w.ents[ent].s_origin;
    set_origin(w, ent, o);
    let s = spawn_string(w, "psName", "").1;
    w.ents[ent].wait = spawn_float(w, "wait", "0").1;
    let idx = w.particle_index(&s);
    let e = &mut w.ents[ent];
    e.model_index = idx;
    if e.spawnflags & 1 != 0 {
        e.e_flags |= k::EF_NODRAW;
    }
    e.use_ = Some(use_particle_system);
    e.etype = EType::ParticleSystem;
    w.link(ent);
}

pub fn use_anim_model(w: &mut World, ent: usize, _o: Option<usize>, _a: Option<usize>) {
    let e = &mut w.ents[ent];
    if e.spawnflags & 1 != 0 {
        e.e_flags ^= k::EF_NODRAW;
    } else {
        e.e_flags ^= k::EF_MOVER_STOP;
    }
}
/// `SP_misc_anim_model`.
pub fn sp_misc_anim_model(w: &mut World, ent: usize) {
    let model = w.ents[ent].model.clone().unwrap_or_default();
    let idx = w.model_index(&model);
    let e = &mut w.ents[ent];
    e.misc = e.animation[0] as i32;
    e.weapon = e.animation[1] as i32;
    e.torso_anim = e.animation[2] as i32;
    e.legs_anim = e.animation[3] as i32;
    e.angles2[0] = e.pos2[0];
    e.model_index = idx;
    e.use_ = Some(use_anim_model);
    e.etype = EType::AnimMapObj;
    if e.spawnflags & 2 != 0 {
        e.e_flags |= k::EF_MOVER_STOP;
    }
    w.link(ent);
}

pub fn use_light_flare(w: &mut World, ent: usize, _o: Option<usize>, _a: Option<usize>) {
    w.ents[ent].e_flags ^= k::EF_NODRAW;
}

/// `findEmptySpot`.
fn find_empty_spot(w: &World, origin: &Vec3, radius: f32) -> Vec3 {
    let mut total = [0.0f32; 3];
    for i in -1..=1 {
        for j in -1..=1 {
            for k in -1..=1 {
                let mut delta = [i as f32 * radius, j as f32 * radius, k as f32 * radius];
                let test = add(origin, &delta);
                let tr = w.trace(&test, None, None, &test, -1, k::MASK_SOLID);
                if !tr.allsolid {
                    let tr = w.trace(&test, None, None, origin, -1, k::MASK_SOLID);
                    delta = scale(&delta, tr.fraction);
                    total = add(&total, &delta);
                }
            }
        }
    }
    normalize(&mut total);
    total = scale(&total, radius);
    add(origin, &total)
}

/// `SP_misc_light_flare`.
pub fn sp_misc_light_flare(w: &mut World, ent: usize) {
    let shader = w.ents[ent].target_shader_name.clone().unwrap_or_default();
    let idx = w.shader_index(&shader);
    w.ents[ent].etype = EType::LightFlare;
    w.ents[ent].model_index = idx;
    w.ents[ent].origin2 = w.ents[ent].pos2;
    let o = w.ents[ent].s_origin;
    w.ents[ent].angles2 = find_empty_spot(w, &o, 8.0);
    w.ents[ent].use_ = Some(use_light_flare);
    w.ents[ent].speed = spawn_float(w, "speed", "200").1;
    w.ents[ent].s_time = w.ents[ent].speed as i32;
    w.ents[ent].generic1 = spawn_int(w, "mindist", "0").1;
    if w.ents[ent].spawnflags & 1 != 0 {
        w.ents[ent].e_flags |= k::EF_NODRAW;
    }
    w.link(ent);
}

// ======================================================= touch dispatch

/// `G_TouchTriggers` (g_active.c).
pub fn touch_triggers(w: &mut World, ent: usize) {
    if !is_client(w, ent) || w.clients[ent].ps.stats[Stat::Health as usize] <= 0 {
        return;
    }
    let class = w.clients[ent].ps.stats[Stat::Class as usize];
    let (pmins, pmaxs, ..) = bg::class_bbox(class);
    let o = w.clients[ent].ps.origin;
    let range = [10.0f32; 3];
    let mins = sub(&add(&o, &pmins), &range);
    let maxs = add(&add(&o, &pmaxs), &range);
    let list = w.entities_in_box(&mins, &maxs);
    let mins = add(&o, &w.ents[ent].mins);
    let maxs = add(&o, &w.ents[ent].maxs);
    for hit in list {
        if w.ents[hit].touch.is_none() && w.ents[ent].touch.is_none() {
            continue;
        }
        if w.ents[hit].contents & CONTENTS_TRIGGER == 0 {
            continue;
        }
        if w.clients[ent].spectator != SpectatorState::Not as i32 {
            let is_door_trigger = w.ents[hit].classname == "door_trigger";
            if w.ents[hit].etype != EType::TeleportTrigger && !is_door_trigger {
                super::mover::manual_trigger_spectator(w, hit, ent);
                continue;
            }
        }
        if !w.entity_contact(&mins, &maxs, hit) {
            continue;
        }
        if let Some(t) = w.ents[hit].touch {
            t(w, hit, ent);
        }
    }
}

/// The trigger part of G_BuildableThink (g_buildable.c G_BuildableTouchTriggers).
pub fn buildable_touch_triggers(w: &mut World, ent: usize) {
    if w.ents[ent].health <= 0 || !w.ents[ent].spawned {
        return;
    }
    let Some(b) = w.ents[ent].buildable else { return };
    let d = b.def();
    let o = w.ents[ent].origin;
    let range = [10.0f32; 3];
    let list = w.entities_in_box(&sub(&add(&o, &d.mins), &range), &add(&add(&o, &d.maxs), &range));
    let (mins, maxs) = (add(&o, &d.mins), add(&o, &d.maxs));
    for hit in list {
        if w.ents[hit].touch.is_none() || w.ents[hit].contents & CONTENTS_TRIGGER == 0 {
            continue;
        }
        if !w.entity_contact(&mins, &maxs, hit) {
            continue;
        }
        if let Some(t) = w.ents[hit].touch {
            t(w, hit, ent);
        }
    }
}
