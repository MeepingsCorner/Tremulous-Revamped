//! Kernel `buildable` — port of game/g_buildable.c: power and creep networks,
//! the per-buildable think/die behaviours named by buildable_behaviors.tsv,
//! G_BuildableThink, G_CanBuild / G_Build, spawn-point checks.

use super::bg::{self, Trajectory};
use super::combat::{self, FL_NOTARGET};
use super::qmath::*;
use super::types::*;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::buildable_behaviors::for_target;
use crate::gen::buildables::BuildableId;
use crate::gen::classes::ClassId;
use crate::gen::enums::{contents, surface, BuildableAnim, Event, MeansOfDeath as Mod, PmType, Stat, Team};
use crate::gen::upgrades::UpgradeId;
use crate::gen::weapons::WeaponId;

pub const POWER_REFRESH_TIME: i32 = 2000;
pub const MAX_ALIEN_BBOX: f32 = 25.0;
pub const M_ROOT3: f32 = 1.732050808;

fn is(w: &World, i: usize, b: BuildableId) -> bool {
    w.ents[i].inuse && w.ents[i].etype == EType::Buildable && w.ents[i].buildable == Some(b)
}
fn buildables(w: &World) -> Vec<usize> {
    (MAX_CLIENTS..w.num_entities).filter(|&i| w.ents[i].inuse && w.ents[i].etype == EType::Buildable).collect()
}

/// `ANIM_FORCEBIT`: play the one-shot even if the idle animation changed.
pub const ANIM_FORCEBIT: i32 = 0x40;

/// `G_SetBuildableAnim`: one-shot animation in legsAnim, toggled once per frame.
pub fn set_anim_ex(w: &mut World, i: usize, a: BuildableAnim, force: bool) {
    let t = w.level.time;
    let e = &mut w.ents[i];
    let mut local = a as i32 | (e.legs_anim & k::ANIM_TOGGLEBIT);
    if force {
        local |= ANIM_FORCEBIT;
    }
    if e.anim_time != t {
        e.anim_time = t;
        local ^= k::ANIM_TOGGLEBIT;
    }
    e.legs_anim = local;
}

/// `G_SetIdleBuildableAnim`: the looping animation in torsoAnim.
pub fn set_idle_anim(w: &mut World, i: usize, a: BuildableAnim) {
    w.ents[i].torso_anim = a as i32;
}

/// The call sites' (anim, force, idle) combinations from g_buildable.c.
pub fn set_anim(w: &mut World, i: usize, a: BuildableAnim) {
    let force = matches!(a, BuildableAnim::Destroy1 | BuildableAnim::Spawn1 | BuildableAnim::Construct1 | BuildableAnim::Construct2 | BuildableAnim::Attack2);
    set_anim_ex(w, i, a, force);
    match a {
        BuildableAnim::Destroy1 => set_idle_anim(w, i, BuildableAnim::Destroyed),
        BuildableAnim::Construct2 => set_idle_anim(w, i, BuildableAnim::Idle1),
        _ => {}
    }
}

/// `G_FindPower`: attempt to find power for `this` (a real buildable, or a
/// BA_NONE dummy when `kind` is None) at `origin`; returns the provider.
fn find_power_for(w: &World, this: Option<usize>, origin: Vec3, kind: Option<BuildableId>, search_unspawned: bool) -> Option<usize> {
    let mut closest = None;
    let mut min_distance = k::REPEATER_BASESIZE + 1;
    for i in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[i];
        if e.etype != EType::Buildable || !e.inuse {
            continue;
        }
        let src =matches!(e.buildable, Some(BuildableId::Reactor) | Some(BuildableId::Repeater));
        if !(src && (search_unspawned || e.spawned) && e.powered && e.health > 0) {
            continue;
        }
        let distance = length(&sub(&origin, &e.origin)) as i32;
        let zone_cost = |base: i32| -> i32 {
            let mut bp = base;
            for j in MAX_CLIENTS..w.num_entities {
                let e2 = &w.ents[j];
                if e2.etype != EType::Buildable || !e2.inuse || Some(j) == this {
                    continue;
                }
                if e2.parent_node == Some(i) {
                    bp -= e2.buildable.map(|b| b.def().build_points).unwrap_or(0);
                }
            }
            bp
        };
        if e.buildable == Some(BuildableId::Reactor) && distance <= k::REACTOR_BASESIZE {
            match kind {
                Some(kb) => {
                    let bp = zone_cost(w.level.human_bp) - w.level.human_bp_queue - kb.def().build_points;
                    if bp >= 0 {
                        return Some(i);
                    }
                    // a buildable can still be built if it shares BP from two zones (TODO upstream)
                }
                None => return Some(i),
            }
        } else if distance < min_distance {
            match kind {
                Some(kb) => {
                    let mut bp = zone_cost(w.level.repeater_bp);
                    if e.uses_bp_zone && w.level.bp_zones.get(e.bp_zone).is_some_and(|z| z.active) {
                        bp -= w.level.bp_zones[e.bp_zone].queued;
                    }
                    bp -= kb.def().build_points;
                    if bp >= 0 {
                        closest = Some(i);
                        min_distance = distance;
                    }
                }
                None => {
                    closest = Some(i);
                    min_distance = distance;
                }
            }
        }
    }
    closest
}

/// `G_FindPower`.
pub fn find_power(w: &mut World, this: usize, search_unspawned: bool) -> bool {
    if w.ents[this].team != Team::Humans {
        return false;
    }
    match w.ents[this].buildable {
        Some(BuildableId::Reactor) => {
            w.ents[this].parent_node = Some(this);
            return true;
        }
        Some(BuildableId::Repeater) => {
            w.ents[this].parent_node = w.reactor();
            return w.ents[this].parent_node.is_some();
        }
        _ => {}
    }
    let p = find_power_for(w, Some(this), w.ents[this].origin, w.ents[this].buildable, search_unspawned);
    w.ents[this].parent_node = p;
    p.is_some()
}

/// `G_PowerEntityForPoint`.
pub fn power_entity_for_point(w: &World, origin: Vec3) -> Option<usize> {
    find_power_for(w, None, origin, None, false)
}

/// `G_PowerEntityForEntity` (without updating parentNode: callers in the
/// original only read the result).
pub fn power_entity_for_entity(w: &World, this: usize) -> Option<usize> {
    if w.ents[this].team != Team::Humans {
        return None;
    }
    match w.ents[this].buildable {
        Some(BuildableId::Reactor) => Some(this),
        Some(BuildableId::Repeater) => w.reactor(),
        _ => find_power_for(w, Some(this), w.ents[this].origin, w.ents[this].buildable, false),
    }
}

/// `G_IsPowered`.
pub fn is_powered(w: &World, origin: Vec3) -> Option<BuildableId> {
    power_entity_for_point(w, origin).and_then(|i| w.ents[i].buildable)
}

/// `G_TimeTilSuddenDeath`.
pub fn time_til_sudden_death(w: &World) -> i32 {
    const SUDDENDEATHWARNING: i32 = 60000;
    let l = &w.level;
    if (l.sudden_death_time == 0 && l.sudden_death_begin_time == 0) || l.sudden_death_begin_time < 0 {
        return SUDDENDEATHWARNING + 1;
    }
    l.sudden_death_begin_time - (l.time - l.start_time)
}

/// `G_GetBuildPoints`.
pub fn get_build_points(w: &World, pos: Vec3, team: Team) -> i32 {
    if time_til_sudden_death(w) <= 0 {
        return 0;
    }
    match team {
        Team::Aliens => w.level.alien_bp_left,
        Team::Humans => {
            let p = power_entity_for_point(w, pos);
            if let Some(pp) = p {
                let e = &w.ents[pp];
                if e.buildable == Some(BuildableId::Reactor) {
                    return w.level.human_bp_left;
                }
                if e.buildable == Some(BuildableId::Repeater) && e.uses_bp_zone {
                    if let Some(z) = w.level.bp_zones.get(e.bp_zone).filter(|z| z.active) {
                        return z.total - z.queued;
                    }
                }
            }
            w.level.human_bp_left
        }
        Team::None => 0,
    }
}

/// `G_GetMarkedBuildPoints`.
pub fn get_marked_build_points(w: &World, pos: Vec3, team: Team) -> i32 {
    if time_til_sudden_death(w) <= 0 || w.level.mark_deconstruct == 0 {
        return 0;
    }
    let here = if team == Team::Humans { power_entity_for_point(w, pos) } else { None };
    let mut sum = 0;
    for i in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[i];
        if e.etype != EType::Buildable {
            continue;
        }
        if team == Team::Humans && !matches!(e.buildable, Some(BuildableId::Reactor) | Some(BuildableId::Repeater)) && e.parent_node != here {
            continue;
        }
        if !e.inuse || e.health <= 0 || e.team != team {
            continue;
        }
        if e.deconstruct {
            sum += e.buildable.map(|b| b.def().build_points).unwrap_or(0);
        }
    }
    sum
}

/// `G_InPowerZone`.
pub fn in_power_zone(w: &World, this: usize) -> Option<usize> {
    let o = w.ents[this].origin;
    for i in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[i];
        if e.etype != EType::Buildable || i == this || !e.spawned || e.health <= 0 {
            continue;
        }
        if matches!(e.buildable, Some(BuildableId::Reactor) | Some(BuildableId::Repeater)) && e.powered {
            let distance = length(&sub(&o, &e.origin)) as i32;
            if e.buildable == Some(BuildableId::Reactor) && distance <= k::REACTOR_BASESIZE {
                return Some(i);
            }
            if e.buildable == Some(BuildableId::Repeater) && distance <= k::REPEATER_BASESIZE {
                return Some(i);
            }
        }
    }
    None
}

/// `G_FindDCC`: how many powered DCCs control this human structure.
pub fn find_dcc(w: &World, i: usize) -> i32 {
    if w.ents[i].team != Team::Humans {
        return 0;
    }
    let o = w.ents[i].origin;
    (MAX_CLIENTS..w.num_entities).filter(|&j| is(w, j, BuildableId::Dcc) && w.ents[j].spawned && (length(&sub(&o, &w.ents[j].origin)) as i32) < k::DC_RANGE && w.ents[j].powered).count() as i32
}

/// `G_IsDCCBuilt`.
pub fn is_dcc_built(w: &World) -> bool {
    (MAX_CLIENTS..w.num_entities).any(|i| {
        let e = &w.ents[i];
        e.etype == EType::Buildable && e.buildable == Some(BuildableId::Dcc) && e.spawned && e.health > 0
    })
}

/// `G_FindCreep`.
pub fn find_creep(w: &mut World, this: usize) -> bool {
    if w.ents[this].ground_entity == -1 {
        return true;
    }
    let need_new = this < MAX_CLIENTS || w.ents[this].parent_node.map(|p| !w.ents[p].inuse || w.ents[p].health <= 0).unwrap_or(true);
    if need_new {
        let found = creep_source(w, w.ents[this].origin);
        if let Some((s, d)) = found {
            if d <= k::CREEP_BASESIZE {
                if this >= MAX_CLIENTS {
                    w.ents[this].parent_node = Some(s);
                }
                return true;
            }
        }
        return false;
    }
    this >= MAX_CLIENTS
}
fn creep_source(w: &World, origin: Vec3) -> Option<(usize, i32)> {
    let mut best = None;
    let mut min_distance = 10000;
    for i in buildables(w) {
        let e = &w.ents[i];
        if matches!(e.buildable, Some(BuildableId::Eggpod) | Some(BuildableId::Overmind)) && e.spawned && e.health > 0 {
            let d = length(&sub(&origin, &e.origin)) as i32;
            if d < min_distance {
                min_distance = d;
                best = Some((i, d));
            }
        }
    }
    best
}
pub fn is_creep_here(w: &World, origin: Vec3) -> bool {
    creep_source(w, origin).map(|(_, d)| d <= k::CREEP_BASESIZE).unwrap_or(false)
}

/// `G_CreepSlow`.
fn creep_slow(w: &mut World, this: usize) {
    let Some(b) = w.ents[this].buildable else { return };
    let s = b.def().creep_size as f32;
    let o = w.ents[this].origin;
    for e in w.entities_in_box(&[o[0] - s, o[1] - s, o[2] - s], &[o[0] + s, o[1] + s, o[2] + s]) {
        if w.ents[e].flags & FL_NOTARGET != 0 {
            continue;
        }
        if e < MAX_CLIENTS && w.ents[e].client && w.clients[e].ps.stat(Stat::Team) == Team::Humans as i32 && w.clients[e].ps.ground_entity_num != ENTITYNUM_NONE {
            w.clients[e].ps.stats[Stat::State as usize] |= k::SS_CREEPSLOWED;
            w.clients[e].last_creep_slow_time = w.level.time;
        }
    }
}

fn null_die(_: &mut World, _: usize, _: usize, _: usize, _: i32, _: i32) {}

/// `G_QueueValue`: the share of the BP the enemy earned by damaging it.
pub fn queue_value(w: &World, this: usize) -> i32 {
    let mut damage_total = 0i32;
    let mut fraction = 0.0f64;
    let team = w.ents[this].team;
    for i in 0..MAX_CLIENTS {
        let c = w.ents[this].credits.get(i).copied().unwrap_or(0);
        damage_total += c;
        if team != w.clients[i].team {
            fraction += c as f64;
        }
    }
    if damage_total > 0 {
        fraction /= damage_total as f64;
    } else {
        fraction = 1.0;
    }
    (fraction * w.ents[this].buildable.map(|b| b.def().build_points).unwrap_or(0) as f64) as i32
}

/// `G_QueueBuildPoints`.
pub fn queue_build_points(w: &mut World, this: usize) {
    let qp = queue_value(w, this);
    if qp == 0 {
        return;
    }
    let t = w.level.time;
    match w.ents[this].team {
        Team::Aliens => {
            if w.level.alien_bp_queue == 0 {
                w.level.alien_next_queue_time = t + w.level.alien_queue_time;
            }
            w.level.alien_bp_queue += qp;
        }
        Team::Humans => {
            let Some(p) = power_entity_for_entity(w, this) else { return };
            match w.ents[p].buildable {
                Some(BuildableId::Reactor) => {
                    let l = &mut w.level;
                    let nqt = super::team::next_queue_time(l.human_bp_queue, l.human_bp, l.human_queue_time);
                    if l.human_bp_queue == 0 || t + nqt < l.human_next_queue_time {
                        l.human_next_queue_time = t + nqt;
                    }
                    l.human_bp_queue += qp;
                }
                Some(BuildableId::Repeater) => {
                    let (uses, zi) = (w.ents[p].uses_bp_zone, w.ents[p].bp_zone);
                    let rqt = w.level.repeater_queue_time;
                    if let Some(z) = w.level.bp_zones.get_mut(zi).filter(|z| uses && z.active) {
                        let nqt = super::team::next_queue_time(z.queued, z.total, rqt);
                        if z.queued == 0 || t + nqt < z.next_queue_time {
                            z.next_queue_time = t + nqt;
                        }
                        z.queued += qp;
                    }
                }
                _ => {}
            }
        }
        Team::None => {}
    }
}

/// `CREEP_SCALEDOWN_TIME`.
const CREEP_SCALEDOWN_TIME: i32 = 3000;

/// `AGeneric_CreepRecede`.
pub fn a_generic_creep_recede(w: &mut World, this: usize) {
    if w.ents[this].e_flags & k::EF_DEAD == 0 {
        w.ents[this].e_flags |= k::EF_DEAD;
        queue_build_points(w, this);
        combat::reward_attackers(w, this);
        w.add_event(this, Event::BuildDestroy, 0);
        // es->time < 0: the creep recedes from this moment (cg_buildable.c CG_Creep)
        let t = w.level.time;
        let e = &mut w.ents[this];
        e.s_time = if e.spawned {
            -t
        } else {
            let bt = e.buildable.map(|b| b.def().build_time).unwrap_or(1) as f32;
            -(t - (CREEP_SCALEDOWN_TIME as f32 * (1.0 - (t - e.build_time) as f32 / bt)) as i32)
        };
    }
    if w.ents[this].timestamp + 10000 > w.level.time {
        w.ents[this].nextthink = w.level.time + 500;
    } else {
        w.free(this);
    }
}

/// `AGeneric_Blast`.
pub fn a_generic_blast(w: &mut World, this: usize) {
    let dir = w.ents[this].origin2;
    let (o, kb, sd, sr, sm) = (w.ents[this].pos.tr_base, w.ents[this].killed_by, w.ents[this].splash_damage, w.ents[this].splash_radius, w.ents[this].splash_mod);
    let attacker = if kb >= 0 && kb < ENTITYNUM_WORLD { Some(kb as usize) } else { None };
    combat::radius_damage(w, o, attacker, sd as f32, sr as f32, Some(this), super::missile::mod_of(sm), Some(Team::Aliens));
    w.ents[this].e_flags |= k::EF_NODRAW;
    w.add_event(this, Event::AlienBuildableExplosion, dir_to_byte(&dir));
    w.ents[this].timestamp = w.level.time;
    w.ents[this].think = Some(a_generic_creep_recede);
    w.ents[this].nextthink = w.level.time + 500;
    w.ents[this].contents = 0;
}

/// `AGeneric_Die`.
pub fn a_generic_die(w: &mut World, this: usize, _inflictor: usize, attacker: usize, _damage: i32, mod_: i32) {
    set_anim(w, this, BuildableAnim::Destroy1);
    let e = &mut w.ents[this];
    e.die = Some(null_die);
    e.killed_by = attacker as i32;
    e.think = Some(a_generic_blast);
    e.e_flags &= !k::EF_FIRING;
    e.powered = false;
    e.nextthink = if e.spawned { w.level.time + 5000 } else { w.level.time };
    log_destruction(w, this, attacker, mod_);
}

/// `AGeneric_CreepCheck`.
fn a_generic_creep_check(w: &mut World, this: usize) {
    let spawn = w.ents[this].parent_node;
    if !find_creep(w, this) {
        let attacker = spawn.and_then(|s| {
            let kb = w.ents[s].killed_by;
            if kb >= 0 && kb < ENTITYNUM_WORLD { Some(kb as usize) } else { None }
        });
        let h = w.ents[this].health;
        combat::damage(w, this, None, attacker, None, None, h, 0, Mod::Nocreep);
        return;
    }
    creep_slow(w, this);
}

/// `AGeneric_Think`.
pub fn a_generic_think(w: &mut World, this: usize) {
    w.ents[this].powered = w.overmind().is_some();
    let nt = w.ents[this].buildable.map(|b| b.def().nextthink).unwrap_or(100);
    w.ents[this].nextthink = w.level.time + nt;
    a_generic_creep_check(w, this);
}

/// `G_CheckSpawnPoint`: returns the blocking entity, or Ok(spawn origin).
pub fn check_spawn_point(w: &World, spawn_num: i32, origin: Vec3, normal: Vec3, spawn: BuildableId) -> Result<Vec3, i32> {
    let d = spawn.def();
    let (cmins, cmaxs, local) = match spawn {
        BuildableId::Eggpod => {
            let disp = (d.maxs[2] + MAX_ALIEN_BBOX) * M_ROOT3;
            ([-MAX_ALIEN_BBOX; 3], [MAX_ALIEN_BBOX; 3], ma(&origin, disp, &normal))
        }
        BuildableId::Telenode => {
            let c = ClassId::HumanBase.def();
            let mut l = origin;
            l[2] += d.maxs[2] + (c.mins[2] as f64).abs() as f32 + 1.0;
            (c.mins, c.maxs, l)
        }
        _ => return Err(ENTITYNUM_WORLD),
    };
    let tr = w.trace(&origin, None, None, &local, spawn_num, k::MASK_SHOT);
    if tr.entity_num != ENTITYNUM_NONE {
        return Err(tr.entity_num);
    }
    let tr = w.trace(&local, Some(&cmins), Some(&cmaxs), &local, ENTITYNUM_NONE, k::MASK_PLAYERSOLID);
    if tr.entity_num != ENTITYNUM_NONE {
        return Err(tr.entity_num);
    }
    Ok(local)
}

fn spawn_blocked_handling(w: &mut World, this: usize, kind: BuildableId) -> bool {
    if !w.ents[this].spawned || w.ents[this].ground_entity == ENTITYNUM_NONE {
        return false;
    }
    if let Err(blocker) = check_spawn_point(w, this as i32, w.ents[this].origin, w.ents[this].origin2, kind) {
        let b = blocker as usize;
        if blocker >= 0 && b < w.ents.len() && w.ents[b].etype == EType::Buildable {
            combat::damage(w, b, None, None, None, None, 10000, 0, Mod::Suicide);
            set_anim(w, this, BuildableAnim::Spawn1);
        } else if blocker == ENTITYNUM_WORLD {
            combat::damage(w, this, None, None, None, None, 10000, 0, Mod::Suicide);
            return true;
        } else if blocker >= 0 && b < w.ents.len() && w.ents[b].etype == EType::Corpse {
            w.free(b);
        }
    }
    false
}

/// `ASpawn_Think`.
pub fn a_spawn_think(w: &mut World, this: usize) {
    if spawn_blocked_handling(w, this, BuildableId::Eggpod) {
        return;
    }
    creep_slow(w, this);
    w.ents[this].nextthink = w.level.time + BuildableId::Eggpod.def().nextthink;
}

/// `AOvermind_Think`.
pub fn a_overmind_think(w: &mut World, this: usize) {
    let t = w.level.time;
    if w.ents[this].spawned && w.ents[this].health > 0 {
        let (o, sd, sr) = (w.ents[this].pos.tr_base, w.ents[this].splash_damage, w.ents[this].splash_radius);
        if combat::radius_damage(w, o, Some(this), sd as f32, sr as f32, Some(this), Mod::Overmind, Some(Team::Aliens)) {
            w.ents[this].timestamp = t;
            set_anim(w, this, BuildableAnim::Attack1);
        }
        if w.level.num_alien_spawns > 0 {
            w.level.overmind_muted = false;
        }
        if !w.level.overmind_muted && w.level.num_alien_spawns <= 0 && t > w.ents[this].overmind_timers[0] {
            w.ents[this].overmind_timers[0] = t + 30000;
            w.temp_event(o, Event::OvermindSpawns, -1, 0, 0);
        }
        if (w.ents[this].health as f32) < (k::OVERMIND_HEALTH as f32 / 10.0) && t > w.ents[this].overmind_timers[1] {
            w.ents[this].overmind_timers[1] = t + 5000;
            w.temp_event(o, Event::OvermindDying, -1, 0, 0);
        }
        if w.ents[this].health < w.ents[this].last_health && t > w.ents[this].overmind_timers[2] {
            w.ents[this].overmind_timers[2] = t + 10000;
            w.temp_event(o, Event::OvermindAttack, -1, 0, 0);
        }
        w.ents[this].last_health = w.ents[this].health;
    } else {
        w.ents[this].overmind_timers[0] = t + 30000;
    }
    creep_slow(w, this);
    w.ents[this].nextthink = t + BuildableId::Overmind.def().nextthink;
}

/// `AAcidTube_Think`.
pub fn a_acid_tube_think(w: &mut World, this: usize) {
    a_generic_think(w, this);
    if !w.ents[this].inuse || !(w.ents[this].spawned && w.ents[this].health > 0 && w.ents[this].powered) {
        return;
    }
    let r = k::ACIDTUBE_RANGE;
    let o = w.ents[this].origin;
    for e in w.entities_in_box(&[o[0] - r, o[1] - r, o[2] - r], &[o[0] + r, o[1] + r, o[2] + r]) {
        if w.ents[e].flags & FL_NOTARGET != 0 || !w.visible(this, e, contents::SOLID as i32) {
            continue;
        }
        if e < MAX_CLIENTS && w.ents[e].client && w.clients[e].ps.stat(Stat::Team) == Team::Humans as i32 {
            if w.level.time >= w.ents[this].timestamp + k::ACIDTUBE_REPEAT_ANIM {
                w.ents[this].timestamp = w.level.time;
                set_anim(w, this, BuildableAnim::Attack1);
                let n = w.ents[this].origin2;
                w.add_event(this, Event::AlienAcidtube, dir_to_byte(&n));
            }
            let base = w.ents[this].pos.tr_base;
            combat::radius_damage(w, base, Some(this), k::ACIDTUBE_DAMAGE as f32, k::ACIDTUBE_RANGE, Some(this), Mod::Atube, Some(Team::Aliens));
            w.ents[this].nextthink = w.level.time + k::ACIDTUBE_REPEAT;
            return;
        }
    }
}

/// `ABooster_Touch`.
pub fn a_booster_touch(w: &mut World, this: usize, other: usize) {
    if !w.ents[this].spawned || !w.ents[this].powered || w.ents[this].health <= 0 {
        return;
    }
    if !(other < MAX_CLIENTS && w.ents[other].client) {
        return;
    }
    if w.clients[other].ps.stat(Stat::Team) == Team::Humans as i32 || w.ents[other].flags & FL_NOTARGET != 0 {
        return;
    }
    w.clients[other].ps.stats[Stat::State as usize] |= k::SS_BOOSTED;
    w.clients[other].boosted_time = w.level.time;
}

/// `AGeneric_Pain`.
pub fn a_generic_pain(w: &mut World, this: usize, _attacker: usize, _damage: i32) {
    if w.ents[this].health <= 0 {
        return;
    }
    set_anim_ex(w, this, BuildableAnim::Pain1, false);
}

/// `ABarricade_Pain`.
pub fn a_barricade_pain(w: &mut World, this: usize, _attacker: usize, _damage: i32) {
    if w.ents[this].health <= 0 {
        return;
    }
    let a = if w.ents[this].shrunk_time == 0 { BuildableAnim::Pain1 } else { BuildableAnim::Pain2 };
    set_anim_ex(w, this, a, false);
}

/// `ABarricade_Shrink`.
pub fn a_barricade_shrink(w: &mut World, this: usize, mut shrink: bool) {
    let t = w.level.time;
    if !w.ents[this].spawned || w.ents[this].health <= 0 {
        shrink = true;
    }
    if shrink && w.ents[this].shrunk_time != 0 {
        w.ents[this].shrunk_time = t;
        let anim = w.ents[this].torso_anim & !(ANIM_FORCEBIT | k::ANIM_TOGGLEBIT);
        if w.ents[this].spawned && w.ents[this].health > 0 && anim != BuildableAnim::Destroyed as i32 {
            set_idle_anim(w, this, BuildableAnim::Destroyed);
            set_anim_ex(w, this, BuildableAnim::Attack1, true);
        }
        return;
    }
    if !shrink && (w.ents[this].shrunk_time == 0 || t < w.ents[this].shrunk_time + k::BARRICADE_SHRINKTIMEOUT) {
        return;
    }
    let d = BuildableId::Barricade.def();
    w.ents[this].mins = d.mins;
    w.ents[this].maxs = d.maxs;
    if shrink {
        w.ents[this].maxs[2] = ((w.ents[this].maxs[2] as f64 * k::BARRICADE_SHRINKPROP as f64) as i32) as f32;
        w.ents[this].shrunk_time = t;
        if w.ents[this].spawned && w.ents[this].health > 0 {
            set_anim_ex(w, this, BuildableAnim::Attack1, true);
            set_idle_anim(w, this, BuildableAnim::Destroyed);
        }
    } else {
        let (o, mn, mx) = (w.ents[this].origin, w.ents[this].mins, w.ents[this].maxs);
        let tr = w.trace(&o, Some(&mn), Some(&mx), &o, this as i32, k::MASK_PLAYERSOLID);
        if tr.startsolid || tr.fraction < 1.0 {
            w.ents[this].maxs[2] = ((w.ents[this].maxs[2] as f64 * k::BARRICADE_SHRINKPROP as f64) as i32) as f32;
            return;
        }
        w.ents[this].shrunk_time = 0;
        let anim = w.ents[this].legs_anim & !(ANIM_FORCEBIT | k::ANIM_TOGGLEBIT);
        if w.ents[this].spawned && w.ents[this].health > 0 && anim != BuildableAnim::Construct1 as i32 && anim != BuildableAnim::Construct2 as i32 {
            set_idle_anim(w, this, d.idle_anim);
            set_anim_ex(w, this, BuildableAnim::Attack2, true);
        }
    }
    if w.ents[this].spawned {
        w.link(this);
    }
}

/// `ABarricade_Die`.
pub fn a_barricade_die(w: &mut World, this: usize, inflictor: usize, attacker: usize, damage: i32, mod_: i32) {
    a_generic_die(w, this, inflictor, attacker, damage, mod_);
    a_barricade_shrink(w, this, true);
}

/// `ABarricade_Think`.
pub fn a_barricade_think(w: &mut World, this: usize) {
    a_generic_think(w, this);
    if !w.ents[this].inuse {
        return;
    }
    let p = w.ents[this].powered;
    a_barricade_shrink(w, this, !p);
}

/// `ABarricade_Touch`: aliens high enough to pass over make it shrink.
pub fn a_barricade_touch(w: &mut World, this: usize, other: usize) {
    if !(other < MAX_CLIENTS && w.ents[other].client) || w.clients[other].team != Team::Aliens {
        return;
    }
    let client_z = (w.ents[other].origin[2] + w.ents[other].mins[2]) as i32;
    let min_z = (w.ents[this].origin[2] - 18.0) as i32 + (w.ents[this].maxs[2] as f64 * k::BARRICADE_SHRINKPROP as f64) as i32;
    if client_z < min_z {
        return;
    }
    a_barricade_shrink(w, this, true);
}

/// `TRAPPER_ACCURACY`: lower is better.
const TRAPPER_ACCURACY: i32 = 10;

/// `ATrapper_FireOnEnemy`: lead the target (velocity, acceleration, jerk) by
/// bisecting the lock blob's flight time.
fn a_trapper_fire_on_enemy(w: &mut World, this: usize, firespeed: i32) {
    let Some(enemy) = w.ents[this].enemy else { return };
    let mut distance_to_target = w.ents[this].buildable.map(|b| b.def().turret_range).unwrap_or(0) as f32;
    let class_speed = bg::class(w.clients[enemy].ps.stat(Stat::Class)).speed;
    let ls = k::LOCKBLOB_SPEED;
    let mut low_msec = 0i32;
    let mut high_msec = (((distance_to_target * ls) + (distance_to_target * class_speed)) / (ls * ls) * 1000.0) as i32;
    let half_acc = scale(&w.ents[enemy].acceleration, 1.0 / 2.0);
    let third_jerk = scale(&w.ents[enemy].jerk, 1.0 / 3.0);
    let base = w.ents[enemy].pos.tr_base;
    let delta = w.ents[enemy].pos.tr_delta;
    let me = w.ents[this].pos.tr_base;
    let mut dir = [0.0f32; 3];
    // highMsec and lowMsec can only move toward one another
    while high_msec - low_msec > TRAPPER_ACCURACY {
        let partition = (high_msec + low_msec) / 2;
        let time = partition as f32 / 1000.0;
        let projectile_distance = ls * time;
        dir = ma(&base, time, &delta);
        dir = ma(&dir, time * time, &half_acc);
        dir = ma(&dir, time * time * time, &third_jerk);
        dir = sub(&dir, &me);
        distance_to_target = length(&dir);
        if projectile_distance < distance_to_target {
            low_msec = partition;
        } else if projectile_distance > distance_to_target {
            high_msec = partition;
        } else {
            break; // unlikely to happen
        }
    }
    normalize(&mut dir);
    w.ents[this].turret_aim = vectoangles(&dir);
    // fire at target
    super::weapon::fire_weapon(w, this, 1);
    set_anim(w, this, BuildableAnim::Attack1);
    w.ents[this].count = w.level.time + firespeed;
}

/// `ATrapper_CheckTarget`.
fn a_trapper_check_target(w: &World, this: usize, target: Option<usize>, range: i32) -> bool {
    let Some(t) = target else { return false };
    let e = &w.ents[t];
    if !e.inuse || t == this || !(t < MAX_CLIENTS && e.client) || e.flags & FL_NOTARGET != 0 {
        return false;
    }
    let c = &w.clients[t];
    if c.ps.stat(Stat::Team) == Team::Aliens as i32 || c.spectator != crate::gen::enums::SpectatorState::Not as i32 || e.health <= 0 || c.ps.stat(Stat::State) & k::SS_BLOBLOCKED != 0 {
        return false;
    }
    let mut d = sub(&e.origin, &w.ents[this].origin);
    if length(&d) > range as f32 {
        return false;
    }
    // only allow a narrow field of "vision"
    normalize(&mut d);
    if dot(&d, &w.ents[this].origin2) < k::LOCKBLOB_DOT {
        return false;
    }
    let tr = w.trace(&w.ents[this].pos.tr_base, None, None, &e.pos.tr_base, this as i32, k::MASK_SHOT);
    tr.contents & contents::SOLID as i32 == 0
}

/// `ATrapper_FindEnemy`: scan every entity from a random start.
fn a_trapper_find_enemy(w: &mut World, this: usize, range: i32) {
    let n = w.num_entities as i32;
    let start = w.rng.rand() / (super::rng::RAND_MAX / n + 1);
    for i in start..n + start {
        let t = (i % n) as usize;
        if a_trapper_check_target(w, this, Some(t), range) {
            w.ents[this].enemy = Some(t);
            return;
        }
    }
    w.ents[this].enemy = None;
}

/// `ATrapper_Think`.
pub fn a_trapper_think(w: &mut World, this: usize) {
    let d = BuildableId::Trapper.def();
    let (range, firespeed) = (d.turret_range, d.turret_fire_speed);
    a_generic_think(w, this);
    if w.ents[this].spawned && w.ents[this].powered {
        // if the current target is not valid find a new one
        if !a_trapper_check_target(w, this, w.ents[this].enemy, range) {
            a_trapper_find_enemy(w, this, range);
        }
        if w.ents[this].enemy.is_none() {
            return;
        }
        if w.ents[this].count < w.level.time {
            a_trapper_fire_on_enemy(w, this, firespeed);
        }
    }
}

/// `AHive_CheckTarget`: fires the swarm when `enemy` is a visible human.
fn a_hive_check_target(w: &mut World, this: usize, enemy: usize) -> bool {
    if w.ents[enemy].health <= 0 || !(enemy < MAX_CLIENTS && w.ents[enemy].client) || w.clients[enemy].ps.stat(Stat::Team) != Team::Humans as i32 {
        return false;
    }
    if w.ents[enemy].flags & FL_NOTARGET != 0 {
        return false;
    }
    let tip = ma(&w.ents[this].pos.tr_base, w.ents[this].maxs[2], &w.ents[this].origin2);
    if distance(&tip, &w.ents[enemy].origin) > k::HIVE_SENSE_RANGE {
        return false;
    }
    let eb = w.ents[enemy].pos.tr_base;
    let tr = w.trace(&tip, None, None, &eb, this as i32, k::MASK_SHOT);
    if tr.fraction < 1.0 && tr.entity_num != enemy as i32 {
        return false;
    }
    w.ents[this].active = true;
    w.ents[this].target_ent = Some(enemy);
    w.ents[this].timestamp = w.level.time + k::HIVE_REPEAT;
    let mut dir = sub(&eb, &w.ents[this].pos.tr_base);
    normalize(&mut dir);
    w.ents[this].turret_aim = vectoangles(&dir);
    w.ents[this].weapon = WeaponId::Hive as i32;
    super::weapon::fire_weapon(w, this, 1);
    set_anim_ex(w, this, BuildableAnim::Attack1, false);
    true
}

/// `AHive_Think`.
pub fn a_hive_think(w: &mut World, this: usize) {
    a_generic_think(w, this);
    if !w.ents[this].inuse {
        return;
    }
    if w.ents[this].timestamp < w.level.time {
        w.ents[this].active = false;
    }
    if w.ents[this].spawned && !w.ents[this].active && w.ents[this].powered {
        let r = k::HIVE_SENSE_RANGE;
        let o = w.ents[this].origin;
        let list = w.entities_in_box(&[o[0] - r, o[1] - r, o[2] - r], &[o[0] + r, o[1] + r, o[2] + r]);
        if list.is_empty() {
            return;
        }
        let n = list.len() as i32;
        let start = (w.rng.rand() / (super::rng::RAND_MAX / n + 1)) as usize;
        for k_ in start..list.len() + start {
            if a_hive_check_target(w, this, list[k_ % list.len()]) {
                return;
            }
        }
    }
}

/// `AHive_Pain`.
pub fn a_hive_pain(w: &mut World, this: usize, attacker: usize, _damage: i32) {
    if w.ents[this].spawned && w.ents[this].powered && !w.ents[this].active {
        a_hive_check_target(w, this, attacker);
    }
    set_anim_ex(w, this, BuildableAnim::Pain1, false);
}

/// `G_SuicideIfNoPower`.
fn suicide_if_no_power(w: &mut World, this: usize) -> bool {
    if w.ents[this].team != Team::Humans {
        return false;
    }
    if !w.ents[this].powered {
        if w.ents[this].count == 0 {
            w.ents[this].count = w.level.time;
        } else if w.level.time - w.ents[this].count >= k::HUMAN_BUILDABLE_INACTIVE_TIME {
            let h = w.ents[this].health;
            combat::damage(w, this, None, None, None, None, h, 0, Mod::Nocreep);
            return true;
        }
    } else {
        w.ents[this].count = 0;
    }
    false
}

/// `HSpawn_Disappear`.
pub fn h_spawn_disappear(w: &mut World, this: usize) {
    w.ents[this].e_flags |= k::EF_NODRAW;
    w.ents[this].timestamp = w.level.time;
    queue_build_points(w, this);
    combat::reward_attackers(w, this);
    w.free(this);
}

/// `HSpawn_Blast` (also the generic human buildable explosion).
pub fn h_spawn_blast(w: &mut World, this: usize) {
    w.ents[this].timestamp = w.level.time;
    let (o, kb, sd, sr, sm) = (w.ents[this].pos.tr_base, w.ents[this].killed_by, w.ents[this].splash_damage, w.ents[this].splash_radius, w.ents[this].splash_mod);
    let attacker = if kb >= 0 && kb < ENTITYNUM_WORLD { Some(kb as usize) } else { None };
    combat::radius_damage(w, o, attacker, sd as f32, sr as f32, Some(this), super::missile::mod_of(sm), None);
    queue_build_points(w, this);
    combat::reward_attackers(w, this);
    w.add_event(this, Event::HumanBuildableExplosion, dir_to_byte(&[0.0, 0.0, 1.0]));
    w.ents[this].etype = EType::General;
    w.ents[this].free_after_event = true;
    w.ents[this].event_time = w.level.time;
    w.ents[this].contents = 0;
}

/// `HSpawn_Die` (used for every human buildable here; see sheet).
pub fn h_spawn_die(w: &mut World, this: usize, _inflictor: usize, attacker: usize, _damage: i32, mod_: i32) {
    set_anim(w, this, BuildableAnim::Destroy1);
    let t = w.level.time;
    let e = &mut w.ents[this];
    e.die = Some(null_die);
    e.killed_by = attacker as i32;
    e.powered = false;
    e.e_flags &= !k::EF_FIRING;
    if e.spawned {
        e.think = Some(h_spawn_blast);
        e.nextthink = t + k::HUMAN_DETONATION_DELAY;
    } else {
        e.think = Some(h_spawn_disappear);
        e.nextthink = t;
    }
    log_destruction(w, this, attacker, mod_);
}

/// `HSpawn_Think`.
pub fn h_spawn_think(w: &mut World, this: usize) {
    let p = find_power(w, this, false);
    w.ents[this].powered = p;
    if suicide_if_no_power(w, this) {
        return;
    }
    if spawn_blocked_handling(w, this, BuildableId::Telenode) {
        return;
    }
    w.ents[this].nextthink = w.level.time + BuildableId::Telenode.def().nextthink;
}

/// `HRepeater_Think`: power, suicide inside another zone, claim a BP zone.
pub fn h_repeater_think(w: &mut World, this: usize) {
    let p = find_power(w, this, false);
    w.ents[this].powered = p;
    if let Some(_pe) = in_power_zone(w, this) {
        let h = w.ents[this].health;
        combat::damage(w, this, None, None, None, None, h, 0, Mod::Suicide);
        return;
    }
    idle_power_state(w, this);
    let needs = w.ents[this].spawned && (!w.ents[this].uses_bp_zone || !w.level.bp_zones.get(w.ents[this].bp_zone).is_some_and(|z| z.active));
    if needs {
        let (t, rbp) = (w.level.time, w.level.repeater_bp);
        if let Some(zi) = w.level.bp_zones.iter().position(|z| !z.active) {
            w.level.bp_zones[zi] = super::world::BpZone { active: true, total: rbp, queued: 0, next_queue_time: t };
            w.ents[this].bp_zone = zi;
            w.ents[this].uses_bp_zone = true;
        }
    }
    w.ents[this].nextthink = w.level.time + POWER_REFRESH_TIME;
}

/// `HRepeater_Die`.
pub fn h_repeater_die(w: &mut World, this: usize, inflictor: usize, attacker: usize, damage: i32, mod_: i32) {
    h_spawn_die(w, this, inflictor, attacker, damage, mod_);
    if w.ents[this].uses_bp_zone {
        let zi = w.ents[this].bp_zone;
        if let Some(z) = w.level.bp_zones.get_mut(zi) {
            z.active = false;
        }
        w.ents[this].uses_bp_zone = false;
    }
}

/// `G_IdlePowerState`: unpowered structures idle in BANIM_IDLE3.
pub fn idle_power_state(w: &mut World, this: usize) {
    let Some(b) = w.ents[this].buildable else { return };
    if w.ents[this].powered {
        if w.ents[this].torso_anim == BuildableAnim::Idle3 as i32 {
            set_idle_anim(w, this, b.def().idle_anim);
        }
    } else if w.ents[this].torso_anim != BuildableAnim::Idle3 as i32 {
        set_idle_anim(w, this, BuildableAnim::Idle3);
    }
}

/// `HReactor_Think`.
pub fn h_reactor_think(w: &mut World, this: usize) {
    let (range, dmg) = if w.ents[this].dcc != 0 { (k::REACTOR_ATTACK_DCC_RANGE, k::REACTOR_ATTACK_DCC_DAMAGE) } else { (k::REACTOR_ATTACK_RANGE, k::REACTOR_ATTACK_DAMAGE) };
    w.ents[this].powered = true;
    if w.ents[this].spawned && w.ents[this].health > 0 {
        let o = w.ents[this].origin;
        let mut fired = false;
        for e in w.entities_in_box(&[o[0] - range, o[1] - range, o[2] - range], &[o[0] + range, o[1] + range, o[2] + range]) {
            if !(e < MAX_CLIENTS && w.ents[e].client && w.clients[e].ps.stat(Stat::Team) == Team::Aliens as i32) || w.ents[e].flags & FL_NOTARGET != 0 {
                continue;
            }
            let eo = w.ents[e].origin;
            w.temp_event(eo, Event::Teslatrail, e as i32, this as i32, 0);
            fired = true;
        }
        if fired {
            w.ents[this].timestamp = w.level.time;
            let base = w.ents[this].pos.tr_base;
            combat::radius_damage(w, base, Some(this), dmg as f32, range, Some(this), Mod::Reactor, Some(Team::Humans));
        }
    }
    w.ents[this].nextthink = w.level.time + k::REACTOR_ATTACK_REPEAT;
}

/// `HArmoury_Think` / `HDCC_Think`.
pub fn h_armoury_think(w: &mut World, this: usize) {
    w.ents[this].nextthink = w.level.time + POWER_REFRESH_TIME;
    let p = find_power(w, this, false);
    w.ents[this].powered = p;
    suicide_if_no_power(w, this);
}

/// `HArmoury_Activate`: a human uses an armoury.
pub fn h_armoury_activate(w: &mut World, this: usize, _other: Option<usize>, activator: Option<usize>) {
    let Some(a) = activator.filter(|&a| a < MAX_CLIENTS && w.ents[a].client) else { return };
    if !w.ents[this].spawned {
        return;
    }
    // only humans can activate this
    if w.clients[a].ps.stat(Stat::Team) != Team::Humans as i32 {
        return;
    }
    // if this is powered then call the armoury menu
    if w.ents[this].powered {
        w.clients[a].wants_menu = true;
        super::client::notify(w, a, "Armoury");
    } else {
        super::client::notify(w, a, "This Armoury is not powered");
    }
}

/// `HRepeater_Use` (reactor and repeater): refill energy weapons.
pub fn h_repeater_use(w: &mut World, this: usize, other: Option<usize>, _activator: Option<usize>) {
    if w.ents[this].health <= 0 || !w.ents[this].spawned {
        return;
    }
    if let Some(o) = other.filter(|&o| o < MAX_CLIENTS && w.ents[o].client) {
        super::client::give_max_ammo(w, o, true);
    }
}

/// `HMedistat_Die`: clear the target's healing flag, then HSpawn_Die.
pub fn h_medistat_die(w: &mut World, this: usize, inflictor: usize, attacker: usize, damage: i32, mod_: i32) {
    if let Some(en) = w.ents[this].enemy.filter(|&e| e < MAX_CLIENTS && w.ents[e].client) {
        w.clients[en].ps.stats[Stat::State as usize] &= !k::SS_HEALING_ACTIVE;
    }
    h_spawn_die(w, this, inflictor, attacker, damage, mod_);
}

/// `HMedistat_Think`.
pub fn h_medistat_think(w: &mut World, this: usize) {
    let t = w.level.time;
    w.ents[this].nextthink = t + BuildableId::Medistat.def().nextthink;
    let p = find_power(w, this, false);
    w.ents[this].powered = p;
    if suicide_if_no_power(w, this) {
        return;
    }
    idle_power_state(w, this);
    if let Some(en) = w.ents[this].enemy {
        if en < MAX_CLIENTS {
            w.clients[en].ps.stats[Stat::State as usize] &= !k::SS_HEALING_ACTIVE;
        }
    }
    if !w.ents[this].powered {
        w.ents[this].active = false;
        w.ents[this].enemy = None;
        w.ents[this].nextthink = t + POWER_REFRESH_TIME;
        return;
    }
    if !w.ents[this].spawned {
        return;
    }
    let mut maxs = add(&w.ents[this].origin, &w.ents[this].maxs);
    let mut mins = add(&w.ents[this].origin, &w.ents[this].mins);
    mins[2] += (w.ents[this].mins[2] as f64).abs() as f32 + w.ents[this].maxs[2];
    maxs[2] += 60.0;
    let list = w.entities_in_box(&mins, &maxs);
    let mut occupied = false;
    for &p in &list {
        if p >= MAX_CLIENTS || !w.ents[p].client {
            continue;
        }
        w.clients[p].ps.stats[Stat::State as usize] &= !k::SS_POISONED;
        if w.ents[this].enemy == Some(p) && w.clients[p].ps.stat(Stat::Team) == Team::Humans as i32 && w.ents[p].health < w.clients[p].ps.stat(Stat::MaxHealth) && bg::pm_live(w.clients[p].ps.pm_type) {
            occupied = true;
            w.clients[p].ps.stats[Stat::State as usize] |= k::SS_HEALING_ACTIVE;
        }
    }
    if !occupied {
        w.ents[this].enemy = None;
        for &p in &list {
            if p >= MAX_CLIENTS || !w.ents[p].client || w.clients[p].ps.stat(Stat::Team) != Team::Humans as i32 {
                continue;
            }
            let ps = &w.clients[p].ps;
            if (w.ents[p].health < ps.stat(Stat::MaxHealth) || ps.stat(Stat::Stamina) < k::STAMINA_MAX) && bg::pm_live(ps.pm_type) {
                w.ents[this].enemy = Some(p);
                if !w.ents[this].active {
                    set_anim(w, this, BuildableAnim::Attack1);
                    w.ents[this].active = true;
                    w.clients[p].ps.stats[Stat::State as usize] |= k::SS_HEALING_ACTIVE;
                }
            } else if !bg::inventory_contains_upgrade(UpgradeId::Medkit as i32, &ps.stats) {
                bg::add_upgrade(UpgradeId::Medkit as i32, &mut w.clients[p].ps.stats);
            }
        }
    }
    match w.ents[this].enemy {
        None if w.ents[this].active => {
            set_anim(w, this, BuildableAnim::Construct2);
            w.ents[this].active = false;
        }
        Some(p) if p < MAX_CLIENTS => {
            let c = &mut w.clients[p];
            if c.ps.stat(Stat::Stamina) < k::STAMINA_MAX {
                c.ps.stats[Stat::Stamina as usize] += k::STAMINA_MEDISTAT_RESTORE;
            }
            if c.ps.stat(Stat::Stamina) > k::STAMINA_MAX {
                c.ps.stats[Stat::Stamina as usize] = k::STAMINA_MAX;
            }
            w.ents[p].health += 1;
            let mh = w.clients[p].ps.stat(Stat::MaxHealth);
            if w.ents[p].health >= mh {
                w.ents[p].health = mh;
                if !bg::inventory_contains_upgrade(UpgradeId::Medkit as i32, &w.clients[p].ps.stats) {
                    bg::add_upgrade(UpgradeId::Medkit as i32, &mut w.clients[p].ps.stats);
                }
            }
            w.clients[p].ps.stats[Stat::Health as usize] = w.ents[p].health;
        }
        _ => {}
    }
}

fn turret_check_target(w: &World, this: usize, t: Option<usize>, los: bool) -> bool {
    let Some(t) = t else { return false };
    if w.ents[t].health <= 0 || !(t < MAX_CLIENTS && w.ents[t].client) || w.clients[t].team != Team::Aliens || w.ents[t].flags & FL_NOTARGET != 0 {
        return false;
    }
    if !los {
        return true;
    }
    // Accept target if we can line-trace to it
    let mut dir = sub(&w.ents[t].pos.tr_base, &w.ents[this].pos.tr_base);
    normalize(&mut dir);
    let end = ma(&w.ents[this].pos.tr_base, k::MGTURRET_RANGE, &dir);
    let tr = w.trace(&w.ents[this].pos.tr_base, None, None, &end, this as i32, k::MASK_SHOT);
    tr.entity_num == t as i32
}

/// `HMGTurret_TrackEnemy`.
fn turret_track_enemy(w: &mut World, this: usize) -> bool {
    let Some(en) = w.ents[this].enemy else { return false };
    let ref_normal: Vec3 = [0.0, 0.0, 1.0];
    let mut dir_to_target = sub(&w.ents[en].pos.tr_base, &w.ents[this].pos.tr_base);
    normalize(&mut dir_to_target);
    let o2 = w.ents[this].origin2;
    let mut x_normal = cross(&o2, &ref_normal);
    normalize(&mut x_normal);
    let rot_angle = rad2deg((dot(&o2, &ref_normal) as f64).acos()) as f32;
    let dtt_adj = rotate_point_around_vector(&x_normal, &dir_to_target, rot_angle);
    let angle_to_target = vectoangles(&dtt_adj);
    let a2 = &mut w.ents[this].angles2;
    let diff_p = angle_subtract(a2[PITCH], angle_to_target[PITCH]);
    let diff_y = angle_subtract(a2[YAW], angle_to_target[YAW]);
    let sp = k::MGTURRET_ANGULARSPEED as f32;
    if diff_p < 0.0 && diff_p < -sp {
        a2[PITCH] += sp;
    } else if diff_p > 0.0 && diff_p > sp {
        a2[PITCH] -= sp;
    } else {
        a2[PITCH] = angle_to_target[PITCH];
    }
    let mut temp = (a2[PITCH] as f64).abs() as f32;
    if temp > 180.0 {
        temp -= 360.0;
    }
    if temp < -(k::MGTURRET_VERTICALCAP as f32) {
        a2[PITCH] = -360.0 + k::MGTURRET_VERTICALCAP as f32;
    }
    if diff_y < 0.0 && diff_y < -sp {
        a2[YAW] += sp;
    } else if diff_y > 0.0 && diff_y > sp {
        a2[YAW] -= sp;
    } else {
        a2[YAW] = angle_to_target[YAW];
    }
    let (f, _, _) = angle_vectors(&w.ents[this].angles2);
    let dt = rotate_point_around_vector(&x_normal, &f, -rot_angle);
    w.ents[this].turret_aim = vectoangles(&dt);
    (diff_y as i32).abs() - k::MGTURRET_ANGULARSPEED <= k::MGTURRET_ACCURACY_TO_FIRE && (diff_p as i32).abs() - k::MGTURRET_ANGULARSPEED <= k::MGTURRET_ACCURACY_TO_FIRE
}

const MGT_STATE_INACTIVE: i32 = 0;
const MGT_STATE_DROP: i32 = 1;
const MGT_STATE_RISE: i32 = 2;
const MGT_STATE_ACTIVE: i32 = 3;

/// `HMGTurret_State`: raise or lower the barrel towards `state` (the C keeps
/// the state in `waterlevel` and the drop speed in `speed`).
fn mg_turret_state(w: &mut World, this: usize, state: i32) -> bool {
    let e = &mut w.ents[this];
    if e.waterlevel == state {
        return false;
    }
    let angle = angle_normalize180(e.angles2[PITCH]);
    let cap = k::MGTURRET_VERTICALCAP as f32;
    if state == MGT_STATE_INACTIVE {
        if angle < cap {
            if e.waterlevel != MGT_STATE_DROP {
                e.speed = 0.25;
                e.waterlevel = MGT_STATE_DROP;
            } else {
                e.speed *= 1.25;
            }
            e.angles2[PITCH] = cap.min(angle + e.speed);
            return true;
        } else {
            e.waterlevel = MGT_STATE_INACTIVE;
        }
    } else if state == MGT_STATE_ACTIVE {
        if e.enemy.is_none() && angle > 0.0 {
            e.waterlevel = MGT_STATE_RISE;
            e.angles2[PITCH] = 0.0f32.max(angle - k::MGTURRET_ANGULARSPEED as f32 * 0.5);
        } else {
            e.waterlevel = MGT_STATE_ACTIVE;
        }
    }
    false
}

/// `HMGTurret_Think`.
pub fn h_mg_turret_think(w: &mut World, this: usize) {
    let t = w.level.time;
    w.ents[this].nextthink = t + BuildableId::Mgturret.def().nextthink;
    w.ents[this].e_flags &= !k::EF_FIRING;
    let p = find_power(w, this, false);
    w.ents[this].powered = p;
    if suicide_if_no_power(w, this) {
        return;
    }
    idle_power_state(w, this);
    if !w.ents[this].powered {
        // if power loss drop turret
        if w.ents[this].spawned && mg_turret_state(w, this, MGT_STATE_INACTIVE) {
            return;
        }
        w.ents[this].nextthink = t + POWER_REFRESH_TIME;
        return;
    }
    if !w.ents[this].spawned {
        return;
    }
    if !turret_check_target(w, this, w.ents[this].enemy, true) {
        w.ents[this].active = false;
        w.ents[this].turret_spinup = -1;
        // HMGTurret_FindEnemy
        w.ents[this].enemy = None;
        let r = k::MGTURRET_RANGE;
        let o = w.ents[this].origin;
        let list = w.entities_in_box(&[o[0] - r, o[1] - r, o[2] - r], &[o[0] + r, o[1] + r, o[2] + r]);
        if !list.is_empty() {
            let n = list.len() as i32;
            let start = w.rng.rand() / (super::rng::RAND_MAX / n + 1);
            for i in start..n + start {
                let cand = list[(i % n) as usize];
                if turret_check_target(w, this, Some(cand), true) {
                    w.ents[this].enemy = Some(cand);
                    break;
                }
            }
        }
    }
    // if newly powered raise turret
    mg_turret_state(w, this, MGT_STATE_ACTIVE);
    if w.ents[this].enemy.is_none() {
        return;
    }
    if !turret_track_enemy(w, this) {
        w.ents[this].active = false;
        w.ents[this].turret_spinup = -1;
        return;
    }
    if !w.ents[this].active && w.ents[this].timestamp < t {
        w.ents[this].active = true;
        w.ents[this].turret_spinup = t + k::MGTURRET_SPINUP_TIME;
        w.add_event(this, Event::MgturretSpinup, 0);
    }
    if !w.ents[this].active || w.ents[this].turret_spinup > t || w.ents[this].timestamp > t {
        return;
    }
    super::weapon::fire_weapon(w, this, 1);
    w.ents[this].e_flags |= k::EF_FIRING;
    w.ents[this].timestamp = t + BuildableId::Mgturret.def().turret_fire_speed;
    w.add_event(this, Event::FireWeapon, 0);
    set_anim(w, this, BuildableAnim::Attack1);
}

/// `HTeslaGen_Think`.
pub fn h_tesla_gen_think(w: &mut World, this: usize) {
    let t = w.level.time;
    w.ents[this].nextthink = t + BuildableId::Tesla.def().nextthink;
    let p = find_power(w, this, false);
    w.ents[this].powered = p;
    if suicide_if_no_power(w, this) {
        return;
    }
    idle_power_state(w, this);
    if !w.ents[this].powered {
        w.ents[this].e_flags &= !k::EF_FIRING;
        w.ents[this].nextthink = t + POWER_REFRESH_TIME;
        return;
    }
    if w.ents[this].spawned && w.ents[this].timestamp < t {
        w.ents[this].e_flags &= !k::EF_FIRING;
        let origin = ma(&w.ents[this].origin, w.ents[this].maxs[2], &w.ents[this].origin2);
        let r = k::TESLAGEN_RANGE as f32;
        w.ents[this].weapon = WeaponId::Teslagen as i32;
        for e in w.entities_in_box(&[origin[0] - r, origin[1] - r, origin[2] - r], &[origin[0] + r, origin[1] + r, origin[2] + r]) {
            if w.ents[e].flags & FL_NOTARGET != 0 {
                continue;
            }
            if e < MAX_CLIENTS && w.ents[e].client && w.ents[e].health > 0 && w.clients[e].team == Team::Aliens && distance(&origin, &w.ents[e].origin) <= r {
                w.ents[this].enemy = Some(e);
                super::weapon::fire_weapon(w, this, 1);
            }
        }
        w.ents[this].enemy = None;
        if w.ents[this].e_flags & k::EF_FIRING != 0 {
            w.add_event(this, Event::FireWeapon, 0);
            w.ents[this].timestamp = t + k::TESLAGEN_REPEAT;
        }
    }
}

/// `G_BuildableThink`.
pub fn buildable_think(w: &mut World, i: usize, msec: i32) {
    let Some(b) = w.ents[i].buildable else { return };
    let d = b.def();
    let t = w.level.time;
    if !w.ents[i].spawned && w.ents[i].health > 0 && w.ents[i].build_time + d.build_time < t {
        w.ents[i].spawned = true;
        if b == BuildableId::Overmind {
            w.center_prints.push((t, Some(Team::Aliens), None, "The Overmind has awakened!".into()));
        }
    }
    w.ents[i].time1000 += msec;
    if w.ents[i].time1000 >= 1000 {
        w.ents[i].time1000 -= 1000;
        let e = &mut w.ents[i];
        if !e.spawned && e.health > 0 {
            e.health += (d.health as f32 / (d.build_time as f32 * 0.001)).ceil() as i32;
        } else if e.health > 0 && e.health < d.health {
            if e.team == Team::Aliens && d.regen_rate != 0 && e.last_damage_time + k::ALIEN_REGEN_DAMAGE_TIME < t {
                e.health += d.regen_rate;
            } else if e.team == Team::Humans && e.dcc != 0 && e.last_damage_time + k::HUMAN_REGEN_DAMAGE_TIME < t {
                e.health += k::DC_HEALRATE * e.dcc;
            }
        }
        if e.health >= d.health {
            e.health = d.health;
            e.credits.iter_mut().for_each(|c| *c = 0);
        }
    }
    let e = &mut w.ents[i];
    if e.client_spawn_time > 0 {
        e.client_spawn_time = (e.client_spawn_time - msec).max(0);
    }
    w.ents[i].dcc = find_dcc(w, i);
    let e = &mut w.ents[i];
    e.generic1 = e.health.max(0);
    e.e_flags &= !(k::EF_B_POWERED | k::EF_B_SPAWNED | k::EF_B_MARKED);
    if e.powered {
        e.e_flags |= k::EF_B_POWERED;
    }
    if e.spawned {
        e.e_flags |= k::EF_B_SPAWNED;
    }
    if e.deconstruct {
        e.e_flags |= k::EF_B_MARKED;
    }
    // G_BuildableTouchTriggers runs from run_entities; then G_Physics
    super::trigger::buildable_touch_triggers(w, i);
    if w.ents[i].inuse && w.ents[i].etype == EType::Buildable {
        super::physics::physics(w, i, msec);
    }
}

/// `G_CanBuild` result (IBE_*), with the original messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    NoOvermind,
    NoCreep,
    NoAlienBp,
    OneOvermind,
    Normal,
    NoRoom,
    NoPower,
    NoHumanBp,
    NoDcc,
    OneReactor,
    RptNoReactor,
    RptPowerHere,
    Permission,
    LastSpawn,
}
impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            BuildError::NoOvermind => "There is no Overmind",
            BuildError::NoCreep => "You need creep to build here",
            BuildError::NoAlienBp => "The Overmind cannot control any more structures",
            BuildError::OneOvermind => "There can only be one Overmind",
            BuildError::Normal => "Cannot build on this surface",
            BuildError::NoRoom => "There is no room to build here",
            BuildError::NoPower => "There is no power here",
            BuildError::NoHumanBp => "There is not enough power left",
            BuildError::NoDcc => "A Defence Computer is required",
            BuildError::OneReactor => "There can only be one Reactor",
            BuildError::RptNoReactor => "There is no Reactor for this Repeater",
            BuildError::RptPowerHere => "There is already power here",
            BuildError::Permission => "Building is not allowed here",
            BuildError::LastSpawn => "This action would remove your last spawn",
        })
    }
}

/// Build points left for `team` where `ent` stands (PERS_BP).
pub fn bp_available(w: &World, team: Team) -> i32 {
    match team {
        Team::Aliens => w.level.alien_bp_left,
        Team::Humans => w.level.human_bp_left,
        _ => 0,
    }
}

/// `G_BuildablesIntersect`.
fn buildables_intersect(a: BuildableId, oa: &Vec3, b: BuildableId, ob: &Vec3) -> bool {
    let (da, db) = (a.def(), b.def());
    bounds_intersect(&add(&da.mins, oa), &add(&da.maxs, oa), &add(&db.mins, ob), &add(&db.maxs, ob))
}

/// `G_ClearDeconMarks`.
pub fn clear_decon_marks(w: &mut World) {
    for i in MAX_CLIENTS..w.num_entities {
        if w.ents[i].inuse && w.ents[i].etype == EType::Buildable {
            w.ents[i].deconstruct = false;
        }
    }
}

/// `G_FreeMarkedBuildables`: returns the readable list of what was removed.
pub fn free_marked_buildables(w: &mut World, deconner: Option<usize>) -> String {
    free_marked_buildables_nums(w, deconner).0
}

/// `G_FreeMarkedBuildables` with its `nums` list (" 12 34", the entity
/// numbers removed, for the Construct log line).
pub fn free_marked_buildables_nums(w: &mut World, deconner: Option<usize>) -> (String, String) {
    let mut nums = String::new();
    if w.level.mark_deconstruct == 0 {
        return (String::new(), nums);
    }
    let list: Vec<usize> = w.level.marked_buildables.iter().take(w.level.num_for_removal).copied().collect();
    let mut counts: Vec<(BuildableId, i32)> = Vec::new();
    for e in list {
        let Some(b) = w.ents[e].buildable else { continue };
        let h = w.ents[e].health;
        combat::damage(w, e, None, deconner, None, None, h, 0, Mod::Replace);
        nums.push_str(&format!(" {e}"));
        match counts.iter_mut().find(|c| c.0 == b) {
            Some(c) => c.1 += 1,
            None => counts.push((b, 1)),
        }
        w.free(e);
    }
    counts.sort_by_key(|c| c.0.num());
    let n = counts.len();
    let mut out = String::new();
    for (k_, (b, c)) in counts.iter().enumerate() {
        if k_ > 0 {
            if k_ == n - 1 {
                out.push_str(&format!("{} and ", if n > 2 { "," } else { "" }));
            } else {
                out.push_str(", ");
            }
        }
        out.push_str(b.cold().display_name);
        if *c > 1 {
            out.push_str(&format!(" ({c}x)"));
        }
    }
    (out, nums)
}

/// `G_SufficientBPAvailable`: fills level.marked_buildables / num_for_removal.
fn sufficient_bp_available(w: &mut World, b: BuildableId, origin: Vec3) -> Result<(), BuildError> {
    let d = b.def();
    let team = d.team;
    let mut build_points = d.build_points;
    w.level.num_for_removal = 0;
    w.level.marked_buildables.clear();
    let (remaining_bp, remaining_spawns, bp_error, spawn, core) = match team {
        Team::Aliens => (get_build_points(w, origin, team), w.level.num_alien_spawns, BuildError::NoAlienBp, BuildableId::Eggpod, BuildableId::Overmind),
        _ => {
            let r = if matches!(b, BuildableId::Reactor | BuildableId::Repeater) { w.level.human_bp_left } else { get_build_points(w, origin, team) };
            (r, w.level.num_human_spawns, BuildError::NoHumanBp, BuildableId::Telenode, BuildableId::Reactor)
        }
    };
    let ents: Vec<usize> = (MAX_CLIENTS..w.num_entities).filter(|&i| w.ents[i].etype == EType::Buildable).collect();
    if w.level.mark_deconstruct == 0 {
        if remaining_bp - build_points < 0 {
            return Err(bp_error);
        }
        for &i in &ents {
            if let Some(eb) = w.ents[i].buildable {
                if buildables_intersect(b, &origin, eb, &w.ents[i].origin) {
                    return Err(BuildError::NoRoom);
                }
            }
        }
        return Ok(());
    }
    build_points -= remaining_bp;
    let here = power_entity_for_point(w, origin);
    let mut marked: Vec<usize> = Vec::new();
    let mut points_yielded = 0;
    let mut collision_count = 0;
    let mut repeater_in_range_count = 0;
    let mut num_for_removal = 0usize;
    for &i in &ents {
        let e = &w.ents[i];
        let Some(eb) = e.buildable else { continue };
        let eo = e.origin;
        let collision = buildables_intersect(b, &origin, eb, &eo);
        if collision {
            if w.level.mark_deconstruct == 1 {
                return Err(BuildError::NoRoom);
            }
            if w.level.mark_deconstruct == 2 && eb != b {
                return Err(BuildError::NoRoom);
            }
            collision_count += 1;
        }
        let repeater_in_range = b == BuildableId::Repeater && eb == b && distance(&eo, &origin) < k::REPEATER_BASESIZE as f32;
        if repeater_in_range {
            repeater_in_range_count += 1;
        }
        if team == Team::Humans && !matches!(b, BuildableId::Reactor | BuildableId::Repeater) && e.parent_node != here {
            continue;
        }
        if !e.inuse || e.health <= 0 || e.team != team {
            continue;
        }
        if eb == core && b != core {
            continue;
        }
        if team == Team::Humans && here == Some(i) && b != BuildableId::Repeater && b != core {
            continue;
        }
        if !collision && !e.powered {
            continue;
        }
        if e.deconstruct {
            marked.push(i);
            if collision || repeater_in_range {
                if collision {
                    collision_count -= 1;
                }
                if repeater_in_range {
                    repeater_in_range_count -= 1;
                }
                if e.powered {
                    points_yielded += eb.def().build_points;
                }
                num_for_removal += 1;
            } else if eb.def().unique_test && eb == b {
                if e.powered {
                    points_yielded += eb.def().build_points;
                }
                num_for_removal += 1;
            }
        }
    }
    let num_required = num_for_removal;
    if build_points > 0 && marked.is_empty() {
        return Err(bp_error);
    }
    if collision_count > 0 {
        return Err(BuildError::NoRoom);
    }
    if repeater_in_range_count > 0 {
        return Err(BuildError::RptPowerHere);
    }
    // G_CompareBuildablesForRemoval (stable sort: qsort order of equal keys is unspecified)
    let precedence = |x: BuildableId| -> i32 {
        use BuildableId::*;
        [Barricade, AcidTube, Trapper, Hive, Booster, Eggpod, Overmind, Mgturret, Tesla, Dcc, Medistat, Arm, Telenode, Repeater, Reactor].iter().position(|&p| p == x).map(|p| p as i32 + 1).unwrap_or(0)
    };
    let (na, nh) = (w.level.num_alien_spawns, w.level.num_human_spawns);
    {
        let wr = &*w;
        marked.sort_by(|&a, &bb| {
            let (ea, eb) = (&wr.ents[a], &wr.ents[bb]);
            let (ba, bb_) = (ea.buildable.unwrap(), eb.buildable.unwrap());
            let am = buildables_intersect(b, &origin, ba, &ea.origin);
            let bm = buildables_intersect(b, &origin, bb_, &eb.origin);
            if am != bm {
                return if am { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
            }
            if matches!(b, BuildableId::Eggpod | BuildableId::Telenode) {
                let last = |x: BuildableId| (x == BuildableId::Eggpod && na == 1) || (x == BuildableId::Telenode && nh == 1);
                if last(ba) {
                    return std::cmp::Ordering::Greater;
                }
                if last(bb_) {
                    return std::cmp::Ordering::Less;
                }
            }
            let (am, bm) = (ba == b, bb_ == b);
            if am != bm {
                return if am { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
            }
            if ba == bb_ {
                let (am, bm) = (here == Some(a), here == Some(bb));
                if am != bm {
                    return if am { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
                }
                return ea.deconstruct_time.cmp(&eb.deconstruct_time);
            }
            precedence(ba).cmp(&precedence(bb_))
        });
    }
    while points_yielded < build_points && num_for_removal < marked.len() {
        let e = &w.ents[marked[num_for_removal]];
        if e.powered {
            points_yielded += e.buildable.map(|x| x.def().build_points).unwrap_or(0);
        }
        num_for_removal += 1;
    }
    let mut changed = true;
    while changed && num_for_removal > 1 && num_for_removal > num_required {
        changed = false;
        let e = &w.ents[marked[num_required]];
        let un = if e.powered { e.buildable.map(|x| x.def().build_points).unwrap_or(0) } else { 0 };
        if points_yielded - un >= build_points {
            points_yielded -= un;
            marked.remove(num_required);
            num_for_removal -= 1;
            changed = true;
        }
    }
    let spawn_count = marked.iter().take(num_for_removal).filter(|&&i| w.ents[i].buildable == Some(spawn)).count() as i32;
    w.level.marked_buildables = marked;
    w.level.num_for_removal = num_for_removal;
    if !w.level.cheats && remaining_spawns > 0 && remaining_spawns - spawn_count < 1 {
        return Err(BuildError::LastSpawn);
    }
    if points_yielded < build_points {
        return Err(bp_error);
    }
    Ok(())
}

/// `G_CanBuild` -> (origin, normal, angles). Leaves the structures that
/// would be replaced in level.marked_buildables[..num_for_removal].
pub fn can_build(w: &mut World, ent: usize, b: BuildableId) -> Result<(Vec3, Vec3, Vec3), BuildError> {
    let ps = w.clients[ent].ps;
    let d = b.def();
    // G_SetBuildableLinkState( qfalse ): buildables do not interact with these traces
    let (origin, angles, tr1, tr2, tr3, contents) = {
        let world_only = WorldNoBuildables(w);
        let (origin, angles, tr1) = bg::position_buildable_relative_to_player(&ps, &d.mins, &d.maxs, &world_only);
        let tr2 = world_only.trace(&origin, Some(&d.mins), Some(&d.maxs), &origin, ent as i32, k::MASK_PLAYERSOLID);
        let tr3 = world_only.trace(&ps.origin, None, None, &origin, ent as i32, k::MASK_PLAYERSOLID);
        let contents = world_only.point_contents(&origin, -1);
        (origin, angles, tr1, tr2, tr3, contents)
    };
    let normal = tr1.plane.normal;
    let mut reason = None;
    if !(normal[2] >= d.min_normal || (d.invert_normal && normal[2] <= -d.min_normal)) {
        reason = Some(BuildError::Normal);
    }
    if tr1.entity_num != ENTITYNUM_WORLD {
        reason = Some(BuildError::Normal);
    }
    if let Err(r) = sufficient_bp_available(w, b, origin) {
        reason = Some(r);
    }
    if ps.stat(Stat::Team) == Team::Aliens as i32 {
        if b != BuildableId::Overmind && w.overmind().is_none() {
            reason = Some(BuildError::NoOvermind);
        }
        if d.creep_test && !is_creep_here(w, origin) {
            reason = Some(BuildError::NoCreep);
        }
        if tr1.surface_flags & surface::NOALIENBUILD as i32 != 0 || contents & contents::NOALIENBUILD as i32 != 0 {
            reason = Some(BuildError::Permission);
        }
    } else if ps.stat(Stat::Team) == Team::Humans as i32 {
        let powered = is_powered(w, origin);
        if powered.is_none() && b != BuildableId::Reactor && b != BuildableId::Repeater {
            reason = Some(BuildError::NoPower);
        }
        if d.dcc_test && !is_dcc_built(w) {
            reason = Some(BuildError::NoDcc);
        }
        if b == BuildableId::Repeater {
            if w.reactor().is_none() {
                reason = Some(BuildError::RptNoReactor);
            } else if w.level.mark_deconstruct != 0 && powered == Some(BuildableId::Reactor) {
                reason = Some(BuildError::RptPowerHere);
            } else if w.level.mark_deconstruct == 0 && powered.is_some() {
                reason = Some(BuildError::RptPowerHere);
            }
        }
        if tr1.surface_flags & surface::NOHUMANBUILD as i32 != 0 || contents & contents::NOHUMANBUILD as i32 != 0 {
            reason = Some(BuildError::Permission);
        }
    }
    if tr1.surface_flags & surface::NOBUILD as i32 != 0 || contents & contents::NOBUILD as i32 != 0 {
        reason = Some(BuildError::Permission);
    }
    if d.unique_test {
        // G_FindBuildable: the first of its kind, dead or alive
        let existing = (MAX_CLIENTS..w.num_entities).find(|&i| w.ents[i].etype == EType::Buildable && w.ents[i].buildable == Some(b));
        if existing.is_some_and(|i| !w.ents[i].deconstruct) {
            reason = Some(if b == BuildableId::Overmind { BuildError::OneOvermind } else { BuildError::OneReactor });
        }
    }
    if reason.is_none() && matches!(b, BuildableId::Eggpod | BuildableId::Telenode) {
        // G_SetBuildableMarkedLinkState( qfalse ) around the spawn check
        let marked: Vec<usize> = w.level.marked_buildables.iter().take(w.level.num_for_removal).copied().filter(|&i| w.ents[i].linked).collect();
        for &m in &marked {
            w.unlink(m);
        }
        if check_spawn_point(w, ENTITYNUM_NONE, origin, normal, b).is_err() {
            reason = Some(BuildError::Normal);
        }
        for &m in &marked {
            w.link(m);
        }
    }
    if reason.is_none() && (tr2.fraction < 1.0 || tr3.fraction < 1.0) {
        reason = Some(BuildError::NoRoom);
    }
    match reason {
        Some(r) => {
            w.level.num_for_removal = 0;
            Err(r)
        }
        None => Ok((origin, normal, angles)),
    }
}

/// The world as G_SetBuildableLinkState( qfalse ) leaves it: every
/// buildable unlinked.
struct WorldNoBuildables<'a>(&'a World);
impl PmWorld for WorldNoBuildables<'_> {
    fn trace(&self, s: &Vec3, mins: Option<&Vec3>, maxs: Option<&Vec3>, e: &Vec3, pass: i32, mask: i32) -> super::cm::Trace {
        let w = self.0;
        w.trace_ex(s, mins, maxs, e, pass, mask, &|i| w.ents[i].etype == EType::Buildable)
    }
    fn point_contents(&self, p: &Vec3, pass: i32) -> i32 {
        let w = self.0;
        w.point_contents_ex(p, pass, &|i| w.ents[i].etype == EType::Buildable)
    }
}

/// `G_BuildIfValid`.
pub fn build_if_valid(w: &mut World, ent: usize, b: BuildableId) -> Result<usize, BuildError> {
    let (origin, normal, _angles) = can_build(w, ent, b)?;
    let yaw = w.clients[ent].ps.viewangles[YAW];
    Ok(build(w, Some(ent), b, origin, normal, [0.0, yaw, 0.0]))
}

/// `G_Build`.
pub fn build(w: &mut World, builder: Option<usize>, b: BuildableId, origin: Vec3, normal: Vec3, angles: Vec3) -> usize {
    build_ex(w, builder, None, [0.0; 3], b, origin, normal, angles)
}

/// `G_Build` with the builder entity's `builtBy` and `s.angles2` (layout
/// items and build-log reverts have no client).
#[allow(clippy::too_many_arguments)]
pub fn build_ex(w: &mut World, builder: Option<usize>, builder_built_by: Option<usize>, builder_angles2: Vec3, b: BuildableId, origin: Vec3, normal: Vec3, angles: Vec3) -> usize {
    let d = b.def();
    let is_client = builder.is_some_and(|bu| bu < MAX_CLIENTS && w.ents[bu].client);
    let log = if is_client { Some(super::layout::build_log_new(w, builder, super::layout::BuildFate::Construct)) } else { None };
    // Free existing buildables
    let mut readable = String::new();
    let mut buildnums = String::new();
    if let Some(bu) = builder {
        (readable, buildnums) = free_marked_buildables_nums(w, Some(bu));
    }
    let i = w.spawn();
    let t = w.level.time;
    let mut local = origin;
    if builder.is_none() {
        local = ma(&local, 1.0, &normal);
    }
    let beh = for_target(b);
    let e = &mut w.ents[i];
    e.etype = EType::Buildable;
    e.classname = d.id;
    e.buildable = Some(b);
    e.team = d.team;
    e.mins = d.mins;
    e.maxs = d.maxs;
    e.health = 1;
    e.splash_damage = d.splash_damage;
    e.splash_radius = d.splash_radius;
    e.splash_mod = d.means_of_death as i32;
    e.nextthink = d.nextthink;
    e.takedamage = true;
    e.spawned = false;
    e.build_time = t;
    e.s_time = t;
    e.think = beh.map(|x| x.think);
    e.die = beh.and_then(|x| x.die);
    e.pain = beh.and_then(|x| x.pain);
    e.physics_bounce = d.bounce;
    e.touch = beh.and_then(|x| x.touch);
    e.use_ = beh.and_then(|x| x.use_);
    e.weapon = d.turret_proj.map(|x| x.num() as i32).unwrap_or(0);
    e.contents = contents::BODY as i32;
    e.clipmask = k::MASK_PLAYERSOLID;
    e.angles = angles;
    e.origin2 = normal;
    e.turret_aim = angles;
    e.killed_by = ENTITYNUM_NONE;
    let _ = PmType::Normal;
    w.set_origin(i, local);
    // gently nudge the buildable onto the surface :)
    // set turret angles (the builder's angles2 are zero for players)
    let built_by = if is_client { builder.and_then(|bu| w.clients[bu].namelog) } else { builder_built_by };
    let e = &mut w.ents[i];
    e.pos.tr_delta = scale(&normal, -50.0);
    e.angles = [0.0, angles[YAW], angles[ROLL]];
    // set turret angles (VectorCopy( builder->s.angles2 ) then yaw / pitch)
    e.angles2 = [k::MGTURRET_VERTICALCAP as f32, angles[YAW], builder_angles2[ROLL]];
    e.built_by = built_by;
    e.pos.tr_type = d.traj as i32;
    e.pos.tr_time = t;
    w.ents[i].nextthink = t + d.nextthink;
    w.link(i);
    if d.team == Team::Humans {
        let p = find_power(w, i, true);
        w.ents[i].powered = p;
    } else {
        w.ents[i].powered = w.overmind().is_some() || b == BuildableId::Overmind;
    }
    if b == BuildableId::Barricade {
        w.ents[i].shrunk_time = 0;
        a_barricade_shrink(w, i, true);
    }
    set_idle_anim(w, i, d.idle_anim);
    set_anim(w, i, BuildableAnim::Construct1);
    w.add_event(i, Event::BuildConstruct, 0);
    if let Some(bu) = builder.filter(|&bu| bu < MAX_CLIENTS && w.ents[bu].client) {
        let team = w.clients[bu].team;
        let msg = format!("{} built by {}{}{}", b.cold().display_name, w.clients[bu].name, if readable.is_empty() { "" } else { ", replacing " }, readable);
        w.prints.push((w.level.time, Some(team), None, msg));
        let line = format!("Construct: {bu} {i} {}{buildnums}: {}^7 is building {}{}{}\n", d.id, w.clients[bu].name, b.cold().display_name, if readable.is_empty() { "" } else { ", replacing " }, readable);
        super::log::log_printf(w, &line);
    }
    if let Some(l) = log {
        super::layout::build_log_set(w, l, i);
    }
    i
}

/// `G_LogDestruction`: tell the team when one of its own structures is
/// deconstructed or destroyed by a team mate (the build log is not kept).
pub fn log_destruction(w: &mut World, this: usize, actor: usize, mod_: i32) {
    use super::layout::BuildFate;
    let actor_client = actor < MAX_CLIENTS && w.ents[actor].client;
    let fate = if mod_ == Mod::Deconstruct as i32 {
        BuildFate::Deconstruct
    } else if mod_ == Mod::Replace as i32 {
        BuildFate::Replace
    } else if mod_ == Mod::Nocreep as i32 {
        if actor_client { BuildFate::Unpower } else { BuildFate::Auto }
    } else if actor_client {
        if Some(w.clients[actor].team) == w.ents[this].buildable.map(|b| b.def().team) { BuildFate::Teamkill } else { BuildFate::Destroy }
    } else {
        BuildFate::Auto
    };
    super::layout::build_log_auto(w, Some(actor), this, fate);
    // don't log when marked structures are removed
    if mod_ == Mod::Replace as i32 {
        return;
    }
    let is_client = actor < MAX_CLIENTS && w.ents[actor].client;
    if let Some(b) = w.ents[this].buildable {
        let line = format!(
            "^3Deconstruct: {actor} {this} {} {}: {} {} by {}\n",
            b.def().id,
            super::log::mod_name(mod_),
            b.cold().display_name,
            if mod_ == Mod::Deconstruct as i32 { "deconstructed" } else { "destroyed" },
            if is_client { w.clients[actor].name.clone() } else { "<world>".into() }
        );
        super::log::log_printf(w, &line);
    }
    // no-power deaths for humans come after some minutes; don't print them
    if mod_ == Mod::Nocreep as i32 && is_client && w.clients[actor].team == Team::Humans {
        return;
    }
    let Some(b) = w.ents[this].buildable else { return };
    if is_client && w.clients[actor].team == b.def().team {
        let what = if mod_ == Mod::Deconstruct as i32 { "DECONSTRUCTED" } else { "DESTROYED" };
        let msg = format!("{} {what} by {}", b.cold().display_name, w.clients[actor].name);
        let team = w.clients[actor].team;
        w.prints.push((w.level.time, Some(team), None, msg));
    }
}

/// `G_FinishSpawningBuildable` for map-placed buildables (already spawned).
pub fn spawn_from_map(w: &mut World, b: BuildableId, origin: Vec3, angles: Vec3) {
    finish_spawning_buildable(w, b, origin, angles, [0.0; 3], [0.0; 3], None, true);
}

/// `G_FinishSpawningBuildable`: G_Build from a placeholder (origin2 = the
/// surface normal or zero, angles2 = turret angles), fully built, dropped
/// onto the surface. With `force` false a start-solid spot is refused.
#[allow(clippy::too_many_arguments)]
pub fn finish_spawning_buildable(w: &mut World, b: BuildableId, origin: Vec3, angles: Vec3, origin2: Vec3, angles2: Vec3, built_by: Option<usize>, force: bool) -> Option<usize> {
    let normal = if origin2 != [0.0; 3] {
        origin2
    } else if b.def().traj == crate::gen::enums::Traj::Buoyancy {
        [0.0, 0.0, -1.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let i = build_ex(w, None, built_by, angles2, b, origin, normal, angles);
    let d = b.def();
    w.ents[i].spawned = true;
    w.ents[i].health = d.health;
    w.ents[i].e_flags |= k::EF_B_SPAWNED;
    w.ents[i].build_time = -d.build_time;
    // es->time = level.time - buildTime is negative when the level starts at
    // time 0, which the client reads as receding creep: keep it at 0 (grown)
    w.ents[i].s_time = 0;
    let dest = add(&scale(&w.ents[i].origin2, -4096.0), &w.ents[i].origin);
    w.unlink(i);
    let tr = w.trace(&w.ents[i].origin.clone(), Some(&d.mins), Some(&d.maxs), &dest, i as i32, w.ents[i].clipmask);
    if tr.startsolid && !force {
        let o = w.ents[i].origin;
        w.console.push_str(&format!("G_FinishSpawningBuildable: {} startsolid at ({:.0} {:.0} {:.0})\n", d.id, o[0], o[1], o[2]));
        w.free(i);
        return None;
    }
    w.ents[i].origin2 = tr.plane.normal;
    w.ents[i].ground_entity = tr.entity_num;
    w.set_origin(i, tr.endpos);
    w.link(i);
    w.ents[i].powered = d.team == Team::Aliens || find_power(w, i, true) || b == BuildableId::Reactor;
    let _ = Trajectory::default();
    Some(i)
}

/// `G_BuildableRange`.
pub fn buildable_range(w: &World, origin: Vec3, r: f32, b: BuildableId) -> bool {
    w.entities_in_box(&[origin[0] - r, origin[1] - r, origin[2] - r], &[origin[0] + r, origin[1] + r, origin[2] + r])
        .into_iter()
        .any(|i| is(w, i, b) && w.ents[i].spawned && w.ents[i].health > 0 && (w.ents[i].team != Team::Humans || w.ents[i].powered))
}
