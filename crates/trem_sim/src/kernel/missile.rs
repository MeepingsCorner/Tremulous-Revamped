//! Kernel `missile` — port of game/g_missile.c. Each fire_* constructor is a
//! row of missiles.tsv; `fire` builds the entity from the row, `run_missile`
//! and `impact` are G_RunMissile / G_MissileImpact.

use super::bg::{self, Trajectory};
use super::combat::{self, DAMAGE_NO_LOCDAMAGE};
use super::qmath::*;
use super::types::*;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::enums::{contents, surface, Event, MeansOfDeath as Mod, MissileImpact, Stat, Team, Traj};
use crate::gen::missiles::MissileId;

pub const MISSILE_PRESTEP_TIME: i32 = 50;

/// Generic `fire_*`. `lc` overrides (damage, radius, speed) for the lucifer cannon.
pub fn fire(w: &mut World, owner: usize, m: MissileId, start: Vec3, dir: Vec3, lc: Option<(i32, i32, i32)>) -> usize {
    let d = m.def();
    let mut dir = dir;
    normalize(&mut dir);
    let b = w.spawn();
    let t = w.level.time;
    let (damage, radius, speed) = match lc {
        Some((dmg, r, s)) => (dmg, r, s as f32),
        None => (d.damage, d.splash_radius, d.speed),
    };
    let splash = if lc.is_some() { damage / 2 } else { d.splash_damage };
    let mut delta = if d.lag != 0.0 && owner < MAX_CLIENTS {
        let pvel = scale(&w.clients[owner].ps.velocity, d.lag);
        ma(&pvel, speed, &dir)
    } else {
        scale(&dir, speed)
    };
    for c in delta.iter_mut() {
        *c = (*c as i32) as f32; // SnapVector
    }
    let generic1 = w.ents[owner].generic1;
    let e = &mut w.ents[b];
    e.classname = d.id;
    e.etype = EType::Missile;
    e.missile = Some(m);
    e.point_against_world = d.point_against_world;
    e.weapon = d.weapon.num() as i32;
    e.generic1 = generic1;
    e.owner = owner as i32;
    e.damage = damage;
    e.splash_damage = splash;
    e.splash_radius = radius;
    e.method_of_death = d.r#mod as i32;
    e.splash_mod = d.splash_mod as i32;
    e.clipmask = k::MASK_SHOT;
    e.mins = [-d.size; 3];
    e.maxs = [d.size; 3];
    e.e_flags = if d.bounce_half { k::EF_BOUNCE_HALF } else { 0 };
    if d.impact == MissileImpact::Hive {
        e.e_flags |= k::EF_BOUNCE | k::EF_NO_BOUNCE_SOUND;
    }
    e.pos = Trajectory { tr_type: d.traj as i32, tr_time: t - MISSILE_PRESTEP_TIME, tr_duration: 0, tr_base: start, tr_delta: delta };
    e.origin = start;
    if let Some((dmg, ..)) = lc {
        // pass the missile charge through
        let charge = (dmg - k::LCANNON_SECONDARY_DAMAGE) as f32 / k::LCANNON_DAMAGE as f32;
        e.torso_anim = ((charge * 255.0) as i32).max(0);
    }
    let lifetime = if lc.map(|x| x.0 == k::LCANNON_DAMAGE).unwrap_or(false) { 0 } else { d.lifetime };
    e.nextthink = t + lifetime;
    e.think = Some(explode_missile);
    e.linked = true;
    if d.impact == MissileImpact::Hive {
        // fire_hive: a homing swarm that returns to its hive
        let target = w.ents[owner].target_ent;
        let e = &mut w.ents[b];
        e.parent = Some(owner);
        e.target_ent = target;
        e.timestamp = t + k::HIVE_LIFETIME;
        e.nextthink = t + k::HIVE_DIR_CHANGE_PERIOD;
        e.think = Some(hive_search_and_destroy);
    }
    b
}

/// `AHive_SearchAndDestroy`.
pub fn hive_search_and_destroy(w: &mut World, i: usize) {
    let t = w.level.time;
    if t > w.ents[i].timestamp {
        let o = w.ents[i].origin;
        let e = &mut w.ents[i];
        e.pos.tr_base = o;
        e.pos.tr_type = Traj::Stationary as i32;
        e.pos.tr_time = t;
        e.think = Some(explode_missile);
        e.nextthink = t + 50;
        if let Some(p) = e.parent {
            w.ents[p].active = false;
        }
        return;
    }
    let me = w.ents[i].origin;
    let Some(mut target) = w.ents[i].target_ent.filter(|&x| x < w.ents.len()) else {
        w.ents[i].nextthink = t + k::HIVE_DIR_CHANGE_PERIOD;
        return;
    };
    let mut nearest = distance_squared(&me, &w.ents[target].origin);
    for c in 0..MAX_CLIENTS {
        let e = &w.ents[c];
        if e.flags & super::combat::FL_NOTARGET != 0 || !e.client || e.health <= 0 || w.clients[c].ps.stat(Stat::Team) != Team::Humans as i32 {
            continue;
        }
        let d = distance_squared(&e.origin, &me);
        if nearest > d {
            let (mn, mx, own, cm) = (w.ents[i].mins, w.ents[i].maxs, w.ents[i].owner, w.ents[i].clipmask);
            let tr = w.trace(&me, Some(&mn), Some(&mx), &e.origin, own, cm);
            if tr.entity_num != ENTITYNUM_WORLD {
                nearest = d;
                target = c;
            }
        }
    }
    w.ents[i].target_ent = Some(target);
    let mut dir = sub(&w.ents[target].origin, &me);
    normalize(&mut dir);
    let mut delta = scale(&dir, k::HIVE_SPEED);
    for c in delta.iter_mut() {
        *c = (*c as i32) as f32; // SnapVector
    }
    let e = &mut w.ents[i];
    e.pos.tr_delta = delta;
    e.pos.tr_base = me;
    e.pos.tr_time = t;
    e.nextthink = t + k::HIVE_DIR_CHANGE_PERIOD;
}

fn distance_squared(a: &Vec3, b: &Vec3) -> f32 {
    let d = sub(a, b);
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// `G_ExplodeMissile`.
pub fn explode_missile(w: &mut World, i: usize) {
    let mut origin = bg::evaluate_trajectory(&w.ents[i].pos, w.level.time);
    for c in origin.iter_mut() {
        *c = (*c as i32) as f32;
    }
    w.set_origin(i, origin);
    w.ents[i].etype = EType::General;
    let wpn = w.ents[i].weapon;
    if wpn != crate::gen::weapons::WeaponId::Lockblob as i32 && wpn != crate::gen::weapons::WeaponId::Flamer as i32 {
        w.temp_event_from(origin, Event::MissileMiss, i as i32, dir_to_byte(&[0.0, 0.0, 1.0]), i);
    }
    w.ents[i].free_after_event = true;
    w.ents[i].event_time = w.level.time;
    if w.ents[i].splash_damage != 0 {
        let owner = w.ents[i].owner;
        let (sd, sr, sm) = (w.ents[i].splash_damage, w.ents[i].splash_radius, w.ents[i].splash_mod);
        combat::radius_damage(w, origin, owner_opt(owner), sd as f32, sr as f32, Some(i), mod_of(sm), None);
    }
}

fn owner_opt(o: i32) -> Option<usize> {
    if o >= 0 && o < ENTITYNUM_WORLD { Some(o as usize) } else { None }
}
pub fn mod_of(m: i32) -> Mod {
    Mod::from_raw(m as u8).unwrap_or(Mod::Unknown)
}

/// `G_BounceMissile`.
fn bounce(w: &mut World, i: usize, tr: &super::cm::Trace) {
    let hit_time = w.level.previous_time + ((w.level.time - w.level.previous_time) as f32 * tr.fraction) as i32;
    let velocity = bg::evaluate_trajectory_delta(&w.ents[i].pos, hit_time);
    let d = dot(&velocity, &tr.plane.normal);
    let e = &mut w.ents[i];
    e.pos.tr_delta = ma(&velocity, -2.0 * d, &tr.plane.normal);
    if e.e_flags & k::EF_BOUNCE_HALF != 0 {
        e.pos.tr_delta = scale(&e.pos.tr_delta, 0.65);
        if tr.plane.normal[2] > 0.2 && length(&e.pos.tr_delta) < 40.0 {
            let p = tr.endpos;
            w.set_origin(i, p);
            return;
        }
    }
    let e = &mut w.ents[i];
    e.origin = add(&e.origin, &tr.plane.normal);
    e.pos.tr_base = e.origin;
    e.pos.tr_time = w.level.time;
}

/// `G_MissileImpact`.
fn impact(w: &mut World, i: usize, tr: &super::cm::Trace) {
    let other = tr.entity_num as usize;
    let attacker = w.ents[i].owner;
    let kind = w.ents[i].missile.map(|m| m.def().impact).unwrap_or(MissileImpact::Normal);
    let other_takes = other < w.ents.len() && w.ents[other].takedamage;
    if !other_takes && w.ents[i].e_flags & (k::EF_BOUNCE | k::EF_BOUNCE_HALF) != 0 || kind == MissileImpact::Bounce {
        bounce(w, i, tr);
        if w.ents[i].e_flags & k::EF_NO_BOUNCE_SOUND == 0 {
            w.add_event(i, Event::GrenadeBounce, 0);
        }
        return;
    }
    let human = other < MAX_CLIENTS && w.ents[other].client && w.clients[other].ps.stat(Stat::Team) == Team::Humans as i32;
    let mut return_after = false;
    match kind {
        MissileImpact::Lockblob if human => {
            let c = &mut w.clients[other];
            c.ps.stats[Stat::State as usize] |= k::SS_BLOBLOCKED;
            c.last_lock_time = w.level.time;
            let (dir, _, _) = angle_vectors(&c.ps.viewangles);
            c.ps.stats[Stat::Viewlock as usize] = dir_to_byte(&dir);
        }
        MissileImpact::Slowblob if human => {
            let c = &mut w.clients[other];
            c.ps.stats[Stat::State as usize] |= k::SS_SLOWLOCKED;
            c.last_slow_time = w.level.time;
            let (dir, _, _) = angle_vectors(&c.ps.viewangles);
            c.ps.stats[Stat::Viewlock as usize] = dir_to_byte(&dir);
        }
        MissileImpact::Hive => {
            if w.ents[other].etype == EType::Buildable && w.ents[other].buildable == Some(crate::gen::buildables::BuildableId::Hive) {
                if let Some(p) = w.ents[i].parent {
                    w.ents[p].active = false;
                }
                w.free(i);
                return;
            }
            w.ents[i].owner = other as i32;
            w.ents[i].think = Some(explode_missile);
            w.ents[i].nextthink = w.level.time + 100;
            if human {
                return_after = true;
            } else {
                return;
            }
        }
        _ => {}
    }
    if other_takes && w.ents[i].damage != 0 {
        let mut velocity = bg::evaluate_trajectory_delta(&w.ents[i].pos, w.level.time);
        if length(&velocity) == 0.0 {
            velocity[2] = 1.0;
        }
        let (dmg, m, o) = (w.ents[i].damage, w.ents[i].method_of_death, w.ents[i].origin);
        combat::damage(w, other, Some(i), owner_opt(attacker), Some(velocity), Some(o), dmg, DAMAGE_NO_LOCDAMAGE, mod_of(m));
    }
    if return_after {
        return;
    }
    let parm = dir_to_byte(&tr.plane.normal);
    let ev = if other_takes && other < w.ents.len() && matches!(w.ents[other].etype, EType::Player | EType::Buildable) {
        Event::MissileHit
    } else if tr.surface_flags & surface::METALSTEPS as i32 != 0 {
        Event::MissileMissMetal
    } else {
        Event::MissileMiss
    };
    let wpn = w.ents[i].weapon;
    let mut endpos = tr.endpos;
    let base = w.ents[i].pos.tr_base;
    for c in 0..3 {
        endpos[c] = if endpos[c] >= 0.0 { ((endpos[c] + if base[c] <= endpos[c] { 0.0 } else { 1.0 }) as i32) as f32 } else { ((endpos[c] + if base[c] <= endpos[c] { -1.0 } else { 0.0 }) as i32) as f32 };
    }
    let _ = wpn;
    w.temp_event_from(endpos, ev, other as i32, parm, i);
    w.ents[i].free_after_event = true;
    w.ents[i].event_time = w.level.time;
    w.ents[i].etype = EType::General;
    w.set_origin(i, endpos);
    if w.ents[i].splash_damage != 0 {
        let (sd, sr, sm) = (w.ents[i].splash_damage, w.ents[i].splash_radius, w.ents[i].splash_mod);
        combat::radius_damage(w, endpos, owner_opt(attacker), sd as f32, sr as f32, Some(other), mod_of(sm), None);
    }
}

/// `G_RunMissile`.
pub fn run_missile(w: &mut World, i: usize) {
    let origin = bg::evaluate_trajectory(&w.ents[i].pos, w.level.time);
    let pass = w.ents[i].owner;
    let (cur, mins, maxs, mask) = (w.ents[i].origin, w.ents[i].mins, w.ents[i].maxs, w.ents[i].clipmask);
    let mut tr = w.trace(&cur, Some(&mins), Some(&maxs), &origin, pass, mask);
    if tr.startsolid || tr.allsolid {
        tr.fraction = 0.0;
        tr.endpos = cur;
    }
    let mut hit = false;
    if tr.fraction < 1.0 {
        if !w.ents[i].point_against_world || tr.contents & contents::BODY as i32 != 0 {
            hit = true;
        } else {
            tr = w.trace(&cur, None, None, &origin, pass, mask);
            if tr.fraction < 1.0 || tr.contents & contents::BODY as i32 != 0 {
                hit = true;
            } else {
                tr = w.trace(&cur, Some(&mins), Some(&maxs), &origin, pass, contents::BODY as i32);
                if tr.fraction < 1.0 {
                    hit = true;
                }
            }
        }
    }
    w.ents[i].origin = tr.endpos;
    if hit {
        if tr.surface_flags & surface::NOIMPACT as i32 != 0 {
            w.free(i);
            return;
        }
        impact(w, i, &tr);
        if w.ents[i].etype != EType::Missile {
            return;
        }
    }
    w.ents[i].linked = true;
    super::world::run_think(w, i);
    let _ = Traj::Linear;
}
