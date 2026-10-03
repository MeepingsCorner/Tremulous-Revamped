//! Kernel `physics` — game/g_physics.c: G_Physics / G_Bounce for buildables
//! and corpses (falling when their floor goes away, bouncing to rest).

use super::bg::{evaluate_trajectory, evaluate_trajectory_delta};
use super::qmath::*;
use super::types::*;
use super::world::{EType, World};
use crate::gen::enums::{contents, Traj};

const PHYSICS_TIME: i32 = 200;

/// `G_Bounce`.
fn bounce(w: &mut World, i: usize, tr: &mut super::cm::Trace) {
    let t = w.level.time;
    let hit_time = w.level.previous_time + ((t - w.level.previous_time) as f32 * tr.fraction) as i32;
    let velocity = evaluate_trajectory_delta(&w.ents[i].pos, hit_time);
    let d = dot(&velocity, &tr.plane.normal);
    let e = &mut w.ents[i];
    e.pos.tr_delta = ma(&velocity, -2.0 * d, &tr.plane.normal);
    let (min_normal, invert) = match (e.etype, e.buildable) {
        (EType::Buildable, Some(b)) => (b.def().min_normal, b.def().invert_normal),
        _ => (0.707f32, false),
    };
    if (tr.plane.normal[2] >= min_normal || (invert && tr.plane.normal[2] <= -min_normal)) && tr.entity_num == ENTITYNUM_WORLD {
        e.pos.tr_delta = scale(&e.pos.tr_delta, e.physics_bounce);
    } else {
        e.pos.tr_delta = scale(&e.pos.tr_delta, 0.3);
    }
    if length(&e.pos.tr_delta) < 10.0 {
        tr.endpos = ma(&tr.endpos, 0.5, &tr.plane.normal);
        let p = tr.endpos;
        super::spawn::set_origin(w, i, p);
        let e = &mut w.ents[i];
        e.ground_entity = tr.entity_num;
        e.origin2 = tr.plane.normal;
        e.pos.tr_delta = [0.0; 3];
        return;
    }
    e.pos.tr_base = e.origin;
    e.origin = add(&e.origin, &tr.plane.normal);
    e.pos.tr_time = t;
}

/// `G_Physics`.
pub fn physics(w: &mut World, i: usize, _msec: i32) {
    let t = w.level.time;
    if w.ents[i].ground_entity == -1 {
        let e = &mut w.ents[i];
        if e.etype == EType::Buildable {
            let traj = e.buildable.map(|b| b.def().traj as i32).unwrap_or(Traj::Gravity as i32);
            if e.pos.tr_type != traj {
                e.pos.tr_type = traj;
                e.pos.tr_time = t;
            }
        } else if e.pos.tr_type != Traj::Gravity as i32 {
            e.pos.tr_type = Traj::Gravity as i32;
            e.pos.tr_time = t;
        }
    }
    let mask = if w.ents[i].clipmask != 0 { w.ents[i].clipmask } else { k::MASK_DEADSOLID };
    if w.ents[i].pos.tr_type == Traj::Stationary as i32 {
        super::world::run_think(w, i);
        if !w.ents[i].inuse {
            return;
        }
        if w.ents[i].next_physics_time < t {
            let e = &w.ents[i];
            let o = ma(&e.origin, -2.0, &e.origin2);
            let tr = w.trace(&e.origin, Some(&e.mins), Some(&e.maxs), &o, i as i32, mask);
            if tr.fraction == 1.0 {
                w.ents[i].ground_entity = -1;
            }
            w.ents[i].next_physics_time = t + PHYSICS_TIME;
        }
        return;
    }
    let origin = evaluate_trajectory(&w.ents[i].pos, t);
    let e = &w.ents[i];
    let mut tr = w.trace(&e.origin, Some(&e.mins), Some(&e.maxs), &origin, i as i32, mask);
    w.ents[i].origin = tr.endpos;
    if tr.startsolid {
        tr.fraction = 0.0;
    }
    w.link(i);
    super::world::run_think(w, i);
    if !w.ents[i].inuse || tr.fraction == 1.0 {
        return;
    }
    let o = w.ents[i].origin;
    if w.point_contents(&o, -1) & contents::NODROP as i32 != 0 {
        w.free(i);
        return;
    }
    bounce(w, i, &mut tr);
}
