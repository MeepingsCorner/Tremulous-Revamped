//! Kernel `mover` — game/g_mover.c: the pusher (G_MoverPush and friends),
//! binary movers (doors, rotating doors, model doors, plats, buttons),
//! trains, and the continuous movers (static, rotating, bobbing, pendulum).
//!
//! Entity mapping: `origin`/`angles` are r.currentOrigin/r.currentAngles,
//! `pos`/`apos` the s.pos/s.apos trajectories, `s_origin`/`s_angles` the
//! spawn values.

use super::bg::{evaluate_trajectory, Trajectory};
use super::combat;
use super::qmath::*;
use super::spawn::*;
use super::types::*;
use super::world::{EType, MoverState, World, MAX_CLIENTS};
use crate::gen::enums::{Event, MeansOfDeath as Mod, SpectatorState, Team, Traj};

const TRAIN_START_OFF: i32 = 1;
const TRAIN_BLOCK_STOPS: i32 = 2;

struct Pushed {
    ent: usize,
    origin: Vec3,
    angles: Vec3,
    deltayaw: i32,
}

/// `C int = int + float` truncation used by nextthink assignments.
fn time_plus(t: i32, f: f32) -> i32 {
    (t as f32 + f) as i32
}

fn is_client(w: &World, i: usize) -> bool {
    i < MAX_CLIENTS && w.ents[i].client
}

/// `G_TestEntityPosition`: the entity number it is stuck in, if any.
pub fn test_entity_position(w: &World, i: usize) -> Option<usize> {
    let e = &w.ents[i];
    let mask = if e.clipmask != 0 { e.clipmask } else { k::MASK_SOLID };
    let o = if is_client(w, i) { w.clients[i].ps.origin } else { e.pos.tr_base };
    let tr = w.trace(&o, Some(&e.mins), Some(&e.maxs), &o, i as i32, mask);
    if tr.startsolid {
        Some(tr.entity_num.max(0) as usize)
    } else {
        None
    }
}

/// `G_CreateRotationMatrix`.
fn rotation_matrix(angles: &Vec3) -> [Vec3; 3] {
    let (f, r, u) = angle_vectors(angles);
    [f, neg(&r), u]
}
fn transpose(m: &[Vec3; 3]) -> [Vec3; 3] {
    [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]]
}
fn rotate_point(p: &Vec3, m: &[Vec3; 3]) -> Vec3 {
    [dot(&m[0], p), dot(&m[1], p), dot(&m[2], p)]
}

/// `G_TryPushingEntity`.
fn try_pushing_entity(w: &mut World, check: usize, pusher: usize, mv: &Vec3, amove: &Vec3, pushed: &mut Vec<Pushed>) -> bool {
    if w.ents[pusher].e_flags & k::EF_MOVER_STOP != 0 && w.ents[check].ground_entity != pusher as i32 {
        return false;
    }
    if w.ents[check].etype == EType::Buildable && w.ents[check].ground_entity != pusher as i32 {
        return false;
    }
    let client = is_client(w, check);
    let mut rec = Pushed { ent: check, origin: w.ents[check].pos.tr_base, angles: w.ents[check].apos.tr_base, deltayaw: 0 };
    if client {
        rec.deltayaw = w.clients[check].ps.delta_angles[YAW];
        rec.origin = w.clients[check].ps.origin;
    }
    pushed.push(rec);

    let tr = rotation_matrix(amove);
    let matrix = transpose(&tr);
    let base = if client { w.clients[check].ps.origin } else { w.ents[check].pos.tr_base };
    let org = sub(&base, &w.ents[pusher].origin);
    let org2 = rotate_point(&org, &matrix);
    let move2 = sub(&org2, &org);
    {
        let e = &mut w.ents[check];
        e.pos.tr_base = add(&e.pos.tr_base, mv);
        e.pos.tr_base = add(&e.pos.tr_base, &move2);
    }
    if client {
        let c = &mut w.clients[check];
        c.ps.origin = add(&c.ps.origin, mv);
        c.ps.origin = add(&c.ps.origin, &move2);
        c.ps.delta_angles[YAW] += angle2short_f(amove[YAW]);
    }
    if w.ents[check].ground_entity != pusher as i32 {
        w.ents[check].ground_entity = -1;
    }
    if test_entity_position(w, check).is_none() {
        let o = if client { w.clients[check].ps.origin } else { w.ents[check].pos.tr_base };
        w.ents[check].origin = o;
        w.link(check);
        return true;
    }
    // if it is ok to leave in the old position, do it
    let last = pushed.last().unwrap();
    let (lo, la) = (last.origin, last.angles);
    w.ents[check].pos.tr_base = lo;
    if client {
        w.clients[check].ps.origin = lo;
    }
    w.ents[check].apos.tr_base = la;
    if test_entity_position(w, check).is_none() {
        w.ents[check].ground_entity = -1;
        pushed.pop();
        return true;
    }
    false
}

/// `G_MoverPush`: Err(obstacle) when blocked.
fn mover_push(w: &mut World, pusher: usize, mv: &Vec3, amove: &Vec3, pushed: &mut Vec<Pushed>) -> Result<(), usize> {
    let (mut mins, mut maxs) = ([0.0f32; 3], [0.0f32; 3]);
    let (mut tmins, mut tmaxs);
    {
        let p = &w.ents[pusher];
        let ca = p.angles;
        if ca[0] != 0.0 || ca[1] != 0.0 || ca[2] != 0.0 || amove[0] != 0.0 || amove[1] != 0.0 || amove[2] != 0.0 {
            let radius = super::world::radius_from_bounds(&p.mins, &p.maxs);
            tmins = [0.0f32; 3];
            tmaxs = [0.0f32; 3];
            for i in 0..3 {
                mins[i] = p.origin[i] + mv[i] - radius;
                maxs[i] = p.origin[i] + mv[i] + radius;
                tmins[i] = mins[i] - mv[i];
                tmaxs[i] = maxs[i] - mv[i];
            }
        } else {
            let (amin, amax) = p.abs_bounds();
            for i in 0..3 {
                mins[i] = amin[i] + mv[i];
                maxs[i] = amax[i] + mv[i];
            }
            tmins = amin;
            tmaxs = amax;
            for i in 0..3 {
                if mv[i] > 0.0 {
                    tmaxs[i] += mv[i];
                } else {
                    tmins[i] += mv[i];
                }
            }
        }
    }
    w.unlink(pusher);
    let list = w.entities_in_box(&tmins, &tmaxs);
    {
        let p = &mut w.ents[pusher];
        p.origin = add(&p.origin, mv);
        p.angles = add(&p.angles, amove);
    }
    w.link(pusher);

    for check in list {
        let et = w.ents[check].etype;
        if et != EType::Buildable && et != EType::Corpse && et != EType::Player {
            continue;
        }
        if w.ents[check].ground_entity != pusher as i32 {
            let (cmin, cmax) = w.ents[check].abs_bounds();
            if cmin[0] >= maxs[0] || cmin[1] >= maxs[1] || cmin[2] >= maxs[2] || cmax[0] <= mins[0] || cmax[1] <= mins[1] || cmax[2] <= mins[2] {
                continue;
            }
            if test_entity_position(w, check).is_none() {
                continue;
            }
        }
        if try_pushing_entity(w, check, pusher, mv, amove, pushed) {
            continue;
        }
        if w.ents[pusher].pos.tr_type == Traj::Sine as i32 || w.ents[pusher].apos.tr_type == Traj::Sine as i32 {
            combat::damage(w, check, Some(pusher), Some(pusher), None, None, 99999, 0, Mod::Crush);
            continue;
        }
        for p in pushed.iter().rev() {
            w.ents[p.ent].pos.tr_base = p.origin;
            w.ents[p.ent].apos.tr_base = p.angles;
            if is_client(w, p.ent) {
                w.clients[p.ent].ps.delta_angles[YAW] = p.deltayaw;
                w.clients[p.ent].ps.origin = p.origin;
            }
            w.link(p.ent);
        }
        return Err(check);
    }
    Ok(())
}

fn team_parts(w: &World, ent: usize) -> Vec<usize> {
    let mut v = vec![ent];
    let mut p = w.ents[ent].teamchain;
    while let Some(i) = p {
        if v.contains(&i) {
            break;
        }
        v.push(i);
        p = w.ents[i].teamchain;
    }
    v
}

/// `G_MoverTeam`.
fn mover_team(w: &mut World, ent: usize) {
    let parts = team_parts(w, ent);
    let mut pushed = Vec::new();
    let t = w.level.time;
    let mut blocked: Option<usize> = None;
    for &part in &parts {
        let origin = evaluate_trajectory(&w.ents[part].pos, t);
        let angles = evaluate_trajectory(&w.ents[part].apos, t);
        let mv = sub(&origin, &w.ents[part].origin);
        let amove = sub(&angles, &w.ents[part].angles);
        if let Err(o) = mover_push(w, part, &mv, &amove, &mut pushed) {
            blocked = Some(o);
            break;
        }
    }
    if let Some(obstacle) = blocked {
        let dt = t - w.level.previous_time;
        for &part in &parts {
            let e = &mut w.ents[part];
            e.pos.tr_time += dt;
            e.apos.tr_time += dt;
            e.origin = evaluate_trajectory(&e.pos, t);
            e.angles = evaluate_trajectory(&e.apos, t);
            w.link(part);
        }
        if let Some(b) = w.ents[ent].blocked {
            b(w, ent, obstacle);
        }
        return;
    }
    for &part in &parts {
        let e = &w.ents[part];
        if e.pos.tr_type == Traj::LinearStop as i32 && t >= e.pos.tr_time + e.pos.tr_duration {
            if let Some(r) = w.ents[part].reached {
                r(w, part);
            }
        }
        let e = &w.ents[part];
        if e.apos.tr_type == Traj::LinearStop as i32 && t >= e.apos.tr_time + e.apos.tr_duration {
            if let Some(r) = w.ents[part].reached {
                r(w, part);
            }
        }
    }
}

/// `G_RunMover`.
pub fn run_mover(w: &mut World, ent: usize) {
    if w.ents[ent].flags & FL_TEAMSLAVE != 0 {
        return;
    }
    let e = &w.ents[ent];
    if (e.pos.tr_type != Traj::Stationary as i32 || e.apos.tr_type != Traj::Stationary as i32) && e.mover_state < MoverState::ModelPos1 {
        mover_team(w, ent);
    }
    super::world::run_think(w, ent);
}

// ------------------------------------------------------- binary movers

/// `SetMoverState`.
pub fn set_mover_state(w: &mut World, ent: usize, state: MoverState, time: i32) {
    use MoverState::*;
    let t = w.level.time;
    let e = &mut w.ents[ent];
    e.mover_state = state;
    e.pos.tr_time = time;
    e.apos.tr_time = time;
    let rate = |tr: &Trajectory| (1000.0f64 / tr.tr_duration as f64) as f32;
    match state {
        Pos1 => {
            e.pos.tr_base = e.pos1;
            e.pos.tr_type = Traj::Stationary as i32;
        }
        Pos2 => {
            e.pos.tr_base = e.pos2;
            e.pos.tr_type = Traj::Stationary as i32;
        }
        OneTo2 => {
            e.pos.tr_base = e.pos1;
            let d = sub(&e.pos2, &e.pos1);
            e.pos.tr_delta = scale(&d, rate(&e.pos));
            e.pos.tr_type = Traj::LinearStop as i32;
        }
        TwoTo1 => {
            e.pos.tr_base = e.pos2;
            let d = sub(&e.pos1, &e.pos2);
            e.pos.tr_delta = scale(&d, rate(&e.pos));
            e.pos.tr_type = Traj::LinearStop as i32;
        }
        RotatorPos1 => {
            e.apos.tr_base = e.pos1;
            e.apos.tr_type = Traj::Stationary as i32;
        }
        RotatorPos2 => {
            e.apos.tr_base = e.pos2;
            e.apos.tr_type = Traj::Stationary as i32;
        }
        Rotator1To2 => {
            e.apos.tr_base = e.pos1;
            let d = sub(&e.pos2, &e.pos1);
            e.apos.tr_delta = scale(&d, rate(&e.apos));
            e.apos.tr_type = Traj::LinearStop as i32;
        }
        Rotator2To1 => {
            e.apos.tr_base = e.pos2;
            let d = sub(&e.pos1, &e.pos2);
            e.apos.tr_delta = scale(&d, rate(&e.apos));
            e.apos.tr_type = Traj::LinearStop as i32;
        }
        _ => {}
    }
    if state >= Pos1 && state <= TwoTo1 {
        e.origin = evaluate_trajectory(&e.pos, t);
    }
    if state >= RotatorPos1 && state <= Rotator2To1 {
        e.angles = evaluate_trajectory(&e.apos, t);
    }
    w.link(ent);
}

/// `MatchTeam`.
pub fn match_team(w: &mut World, leader: usize, state: MoverState, time: i32) {
    for s in team_parts(w, leader) {
        set_mover_state(w, s, state, time);
    }
}

fn sound(w: &mut World, ent: usize, idx: i32) {
    if idx != 0 {
        w.add_event(ent, Event::GeneralSound, idx);
    }
}

/// `ReturnToPos1`.
pub fn return_to_pos1(w: &mut World, ent: usize) {
    let t = w.level.time;
    match_team(w, ent, MoverState::TwoTo1, t);
    w.ents[ent].loop_sound = w.ents[ent].sound_loop;
    let s = w.ents[ent].sound2to1;
    sound(w, ent, s);
}

/// `ReturnToApos1`.
pub fn return_to_apos1(w: &mut World, ent: usize) {
    let t = w.level.time;
    match_team(w, ent, MoverState::Rotator2To1, t);
    w.ents[ent].loop_sound = w.ents[ent].sound_loop;
    let s = w.ents[ent].sound2to1;
    sound(w, ent, s);
}

/// `Think_ClosedModelDoor`.
pub fn think_closed_model_door(w: &mut World, ent: usize) {
    let s = w.ents[ent].sound_pos1;
    sound(w, ent, s);
    // close areaportals
    if w.ents[ent].teammaster.is_none_or(|m| m == ent) {
        w.adjust_area_portal_state(ent, false);
    }
    w.ents[ent].mover_state = MoverState::ModelPos1;
}

/// `Think_CloseModelDoor`.
pub fn think_close_model_door(w: &mut World, ent: usize) {
    let Some(cb) = w.ents[ent].clip_brush else { return };
    let (lo, hi) = w.ents[cb].abs_bounds();
    let list = w.entities_in_box(&lo, &hi);
    w.link(cb);
    let mut can_close = true;
    for check in list {
        let et = w.ents[check].etype;
        if et != EType::Buildable && et != EType::Corpse && et != EType::Player {
            continue;
        }
        if test_entity_position(w, check).is_some() {
            can_close = false;
        }
    }
    if !can_close {
        w.unlink(cb);
        w.ents[ent].nextthink = time_plus(w.level.time, w.ents[ent].wait);
        return;
    }
    w.ents[ent].legs_anim = 0;
    let s = w.ents[ent].sound2to1;
    sound(w, ent, s);
    w.ents[ent].mover_state = MoverState::Model2To1;
    w.ents[ent].think = Some(think_closed_model_door);
    w.ents[ent].nextthink = time_plus(w.level.time, w.ents[ent].speed);
}

/// `Think_OpenModelDoor`.
pub fn think_open_model_door(w: &mut World, ent: usize) {
    if let Some(cb) = w.ents[ent].clip_brush {
        w.unlink(cb);
    }
    w.ents[ent].loop_sound = w.ents[ent].sound_loop;
    let s = w.ents[ent].sound_pos2;
    sound(w, ent, s);
    w.ents[ent].mover_state = MoverState::ModelPos2;
    w.ents[ent].think = Some(think_close_model_door);
    w.ents[ent].nextthink = time_plus(w.level.time, w.ents[ent].wait);
    if w.ents[ent].activator.is_none() {
        w.ents[ent].activator = Some(ent);
    }
    let a = w.ents[ent].activator;
    use_targets(w, ent, a);
}

/// `Reached_BinaryMover`.
pub fn reached_binary_mover(w: &mut World, ent: usize) {
    use MoverState::*;
    w.ents[ent].loop_sound = w.ents[ent].sound_loop;
    let t = w.level.time;
    match w.ents[ent].mover_state {
        OneTo2 | Rotator1To2 => {
            let rot = w.ents[ent].mover_state == Rotator1To2;
            set_mover_state(w, ent, if rot { RotatorPos2 } else { Pos2 }, t);
            let s = w.ents[ent].sound_pos2;
            sound(w, ent, s);
            w.ents[ent].think = Some(if rot { return_to_apos1 } else { return_to_pos1 });
            w.ents[ent].nextthink = time_plus(t, w.ents[ent].wait);
            if w.ents[ent].activator.is_none() {
                w.ents[ent].activator = Some(ent);
            }
            let a = w.ents[ent].activator;
            use_targets(w, ent, a);
        }
        TwoTo1 | Rotator2To1 => {
            let rot = w.ents[ent].mover_state == Rotator2To1;
            set_mover_state(w, ent, if rot { RotatorPos1 } else { Pos1 }, t);
            let s = w.ents[ent].sound_pos1;
            sound(w, ent, s);
            // close areaportals
            if w.ents[ent].teammaster.is_none_or(|m| m == ent) {
                w.adjust_area_portal_state(ent, false);
            }
        }
        _ => {} // G_Error( "Reached_BinaryMover: bad moverState" )
    }
}

/// `Use_BinaryMover`.
pub fn use_binary_mover(w: &mut World, ent: usize, other: Option<usize>, activator: Option<usize>) {
    use MoverState::*;
    if w.ents[ent].targetname.is_some() && other.is_some_and(|o| is_client(w, o)) {
        return;
    }
    if w.ents[ent].flags & FL_TEAMSLAVE != 0 {
        if let Some(m) = w.ents[ent].teammaster {
            if m != ent {
                use_binary_mover(w, m, other, activator);
            }
        }
        return;
    }
    w.ents[ent].activator = activator;
    let t = w.level.time;
    let state = w.ents[ent].mover_state;
    match state {
        Pos1 | RotatorPos1 => {
            match_team(w, ent, if state == Pos1 { OneTo2 } else { Rotator1To2 }, t + 50);
            let s = w.ents[ent].sound1to2;
            sound(w, ent, s);
            w.ents[ent].loop_sound = w.ents[ent].sound_loop;
            // open areaportal
            if w.ents[ent].teammaster.is_none_or(|m| m == ent) {
                w.adjust_area_portal_state(ent, true);
            }
        }
        Pos2 | RotatorPos2 | ModelPos2 => {
            w.ents[ent].nextthink = time_plus(t, w.ents[ent].wait);
        }
        TwoTo1 | OneTo2 | Rotator2To1 | Rotator1To2 => {
            let rot = matches!(state, Rotator2To1 | Rotator1To2);
            let tr = if rot { w.ents[ent].apos } else { w.ents[ent].pos };
            let total = tr.tr_duration;
            let partial = (t - tr.tr_time).min(total);
            let (next, snd) = match state {
                TwoTo1 => (OneTo2, w.ents[ent].sound1to2),
                OneTo2 => (TwoTo1, w.ents[ent].sound2to1),
                Rotator2To1 => (Rotator1To2, w.ents[ent].sound1to2),
                _ => (Rotator2To1, w.ents[ent].sound2to1),
            };
            match_team(w, ent, next, t - (total - partial));
            sound(w, ent, snd);
        }
        ModelPos1 => {
            w.ents[ent].legs_anim = 1;
            w.ents[ent].think = Some(think_open_model_door);
            w.ents[ent].nextthink = time_plus(t, w.ents[ent].speed);
            let s = w.ents[ent].sound1to2;
            sound(w, ent, s);
            w.ents[ent].loop_sound = w.ents[ent].sound_loop;
            // open areaportal
            if w.ents[ent].teammaster.is_none_or(|m| m == ent) {
                w.adjust_area_portal_state(ent, true);
            }
            w.ents[ent].mover_state = Model1To2;
        }
        _ => {}
    }
}

/// `trap_SetBrushModel`.
pub fn set_brush_model(w: &mut World, ent: usize) {
    let n = w.ents[ent].model.as_deref().and_then(|m| m.strip_prefix('*')).map(c_atoi).unwrap_or(0).max(0) as usize;
    let m = w.cm.models.get(n).copied().unwrap_or_default();
    let e = &mut w.ents[ent];
    e.model_index = n as i32;
    e.mins = m.mins;
    e.maxs = m.maxs;
    e.bmodel = true;
    e.contents = -1;
    w.link(ent);
}

/// The "noise", "light" and "color" keys InitMover/InitRotator/door_model share.
fn mover_keys(w: &mut World, ent: usize) {
    if let Some(m2) = w.ents[ent].model2.clone() {
        let idx = w.model_index(&m2);
        w.ents[ent].model_index2 = idx;
    }
    let (p, s) = spawn_string(w, "noise", "100");
    if p {
        let idx = w.sound_index(&s);
        w.ents[ent].loop_sound = idx;
    }
    let (light_set, light) = spawn_float(w, "light", "100");
    let (color_set, color) = spawn_vector(w, "color", "1 1 1");
    if light_set || color_set {
        let r = ((color[0] * 255.0) as i32).min(255);
        let g = ((color[1] * 255.0) as i32).min(255);
        let b = ((color[2] * 255.0) as i32).min(255);
        let i = ((light / 4.0) as i32).min(255);
        w.ents[ent].constant_light = r | (g << 8) | (b << 16) | (i << 24);
    }
}

/// `InitMover`.
pub fn init_mover(w: &mut World, ent: usize) {
    mover_keys(w, ent);
    let e = &mut w.ents[ent];
    e.use_ = Some(use_binary_mover);
    e.reached = Some(reached_binary_mover);
    e.mover_state = MoverState::Pos1;
    e.svflags = SVF_USE_CURRENT_ORIGIN;
    e.etype = EType::Mover;
    e.origin = e.pos1;
    w.link(ent);
    let e = &mut w.ents[ent];
    e.pos.tr_type = Traj::Stationary as i32;
    e.pos.tr_base = e.pos1;
    let mv = sub(&e.pos2, &e.pos1);
    let distance = length(&mv);
    if e.speed == 0.0 {
        e.speed = 100.0;
    }
    e.pos.tr_delta = scale(&mv, e.speed);
    e.pos.tr_duration = (distance * 1000.0 / e.speed) as i32;
    if e.pos.tr_duration <= 0 {
        e.pos.tr_duration = 1;
    }
}

/// `InitRotator`.
pub fn init_rotator(w: &mut World, ent: usize) {
    mover_keys(w, ent);
    let e = &mut w.ents[ent];
    e.use_ = Some(use_binary_mover);
    e.reached = Some(reached_binary_mover);
    e.mover_state = MoverState::RotatorPos1;
    e.svflags = SVF_USE_CURRENT_ORIGIN;
    e.etype = EType::Mover;
    e.angles = e.pos1;
    w.link(ent);
    let e = &mut w.ents[ent];
    e.apos.tr_type = Traj::Stationary as i32;
    e.apos.tr_base = e.pos1;
    let mv = sub(&e.pos2, &e.pos1);
    let angle = length(&mv);
    if e.speed == 0.0 {
        e.speed = 120.0;
    }
    e.apos.tr_delta = scale(&mv, e.speed);
    e.apos.tr_duration = (angle * 1000.0 / e.speed) as i32;
    if e.apos.tr_duration <= 0 {
        e.apos.tr_duration = 1;
    }
}

// ---------------------------------------------------------------- doors

/// `Blocked_Door`.
pub fn blocked_door(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) && w.ents[other].etype != EType::Buildable {
        w.free(other);
        return;
    }
    if w.ents[ent].damage != 0 {
        let d = w.ents[ent].damage;
        combat::damage(w, other, Some(ent), Some(ent), None, None, d, 0, Mod::Crush);
    }
    if w.ents[ent].spawnflags & 4 != 0 {
        return;
    }
    use_binary_mover(w, ent, Some(ent), Some(other));
}

/// `Touch_DoorTriggerSpectator` over an absolute box (trigger or hull).
fn door_trigger_spectator(w: &mut World, absmin: Vec3, absmax: Vec3, axis: usize, other: usize) {
    let mut origin = [0.0f32; 3];
    let mut dir = [0.0f32; 3];
    let so = w.ents[other].origin;
    if ((so[axis] - absmax[axis]) as f64).abs() < ((so[axis] - absmin[axis]) as f64).abs() {
        origin[axis] = absmin[axis] - 20.0;
        dir[axis] = -1.0;
    } else {
        origin[axis] = absmax[axis] + 20.0;
        dir[axis] = 1.0;
    }
    for i in 0..3 {
        if i != axis {
            origin[i] = ((absmin[i] + absmax[i]) as f64 * 0.5) as f32;
        }
    }
    let angles = vectoangles(&dir);
    teleport_player(w, other, &origin, &angles);
}

fn opening(s: MoverState) -> bool {
    use MoverState::*;
    matches!(s, OneTo2 | Pos2 | Rotator1To2 | RotatorPos2 | Model1To2 | ModelPos2)
}

/// `manualTriggerSpectator`: a spectator touching a trigger_multiple skips
/// the closest func_door it targets.
pub fn manual_trigger_spectator(w: &mut World, trigger: usize, player: usize) {
    if w.ents[trigger].classname != "trigger_multiple" {
        return;
    }
    let Some(target) = w.ents[trigger].target.clone() else { return };
    let mut doors = Vec::new();
    let mut t = None;
    while let Some(i) = find_targetname(w, t, &target) {
        t = Some(i);
        if w.ents[i].classname == "func_door" {
            doors.push(i);
        }
        if !w.ents[trigger].inuse {
            return;
        }
    }
    let po = w.ents[player].origin;
    let mut closest = None;
    let mut best = f32::INFINITY;
    for d in doors {
        let dist = distance(&po, &w.ents[d].origin);
        if dist < best {
            best = dist;
            closest = Some(d);
        }
    }
    let Some(door) = closest else { return };
    if opening(w.ents[door].mover_state) {
        return;
    }
    let (mut mins, mut maxs) = w.ents[door].abs_bounds();
    for o in team_parts(w, door).into_iter().skip(1) {
        let (a, b) = w.ents[o].abs_bounds();
        add_point_to_bounds(&a, &mut mins, &mut maxs);
        add_point_to_bounds(&b, &mut mins, &mut maxs);
    }
    let mut best = 0;
    for i in 1..3 {
        if maxs[i] - mins[i] < maxs[best] - mins[best] {
            best = i;
        }
    }
    maxs[best] += 60.0;
    mins[best] -= 60.0;
    door_trigger_spectator(w, mins, maxs, best, player);
}

/// `Touch_DoorTrigger`.
pub fn touch_door_trigger(w: &mut World, ent: usize, other: usize) {
    use MoverState::*;
    if w.ents[other].etype == EType::Buildable {
        return;
    }
    let Some(parent) = w.ents[ent].parent else { return };
    let ps = w.ents[parent].mover_state;
    if is_client(w, other) && w.clients[other].spectator != SpectatorState::Not as i32 {
        if !matches!(ps, OneTo2 | Pos2 | Rotator1To2 | RotatorPos2) {
            let (lo, hi) = w.ents[ent].abs_bounds();
            let axis = w.ents[ent].count.clamp(0, 2) as usize;
            door_trigger_spectator(w, lo, hi, axis, other);
        }
    } else if !matches!(ps, OneTo2 | Rotator1To2 | Rotator2To1) {
        use_binary_mover(w, parent, Some(ent), Some(other));
    }
}

/// `Think_SpawnNewDoorTrigger`.
pub fn think_spawn_new_door_trigger(w: &mut World, ent: usize) {
    let (mut mins, mut maxs) = w.ents[ent].abs_bounds();
    for o in team_parts(w, ent).into_iter().skip(1) {
        let (a, b) = w.ents[o].abs_bounds();
        add_point_to_bounds(&a, &mut mins, &mut maxs);
        add_point_to_bounds(&b, &mut mins, &mut maxs);
    }
    let mut best = 0;
    for i in 1..3 {
        if maxs[i] - mins[i] < maxs[best] - mins[best] {
            best = i;
        }
    }
    maxs[best] += 60.0;
    mins[best] -= 60.0;
    let o = w.spawn();
    let e = &mut w.ents[o];
    e.classname = "door_trigger";
    e.mins = mins;
    e.maxs = maxs;
    e.parent = Some(ent);
    e.contents = CONTENTS_TRIGGER;
    e.touch = Some(touch_door_trigger);
    e.count = best as i32;
    w.link(o);
    if w.ents[ent].mover_state < MoverState::ModelPos1 {
        let (s, t) = (w.ents[ent].mover_state, w.level.time);
        match_team(w, ent, s, t);
    }
}

/// `Think_MatchTeam`.
pub fn think_match_team(w: &mut World, ent: usize) {
    let (s, t) = (w.ents[ent].mover_state, w.level.time);
    match_team(w, ent, s, t);
}

fn door_sounds(w: &mut World, ent: usize, strt: &str, end: &str) {
    let s = spawn_string(w, "sound2to1", strt).1;
    w.ents[ent].sound2to1 = w.sound_index(&s);
    let s = spawn_string(w, "sound1to2", strt).1;
    w.ents[ent].sound1to2 = w.sound_index(&s);
    let s = spawn_string(w, "soundPos2", end).1;
    w.ents[ent].sound_pos2 = w.sound_index(&s);
    let s = spawn_string(w, "soundPos1", end).1;
    w.ents[ent].sound_pos1 = w.sound_index(&s);
}

fn door_think_setup(w: &mut World, ent: usize, rotating: bool) {
    w.ents[ent].nextthink = w.level.time + FRAMETIME;
    if w.ents[ent].flags & FL_TEAMSLAVE == 0 {
        let health = spawn_int(w, "health", "0").1;
        if health != 0 {
            w.ents[ent].takedamage = true;
        }
        let _ = rotating;
        w.ents[ent].think = Some(if w.ents[ent].targetname.is_some() || health != 0 { think_match_team } else { think_spawn_new_door_trigger });
    }
}

/// `SP_func_door`.
pub fn sp_func_door(w: &mut World, ent: usize) {
    door_sounds(w, ent, "sound/movers/doors/dr1_strt.wav", "sound/movers/doors/dr1_end.wav");
    w.ents[ent].blocked = Some(blocked_door);
    if w.ents[ent].speed == 0.0 {
        w.ents[ent].speed = 400.0;
    }
    if w.ents[ent].wait == 0.0 {
        w.ents[ent].wait = 2.0;
    }
    w.ents[ent].wait *= 1000.0;
    let lip = spawn_float(w, "lip", "8").1;
    w.ents[ent].damage = spawn_int(w, "dmg", "2").1;
    w.ents[ent].pos1 = w.ents[ent].s_origin;
    set_brush_model(w, ent);
    let e = &mut w.ents[ent];
    e.movedir = set_movedir(&mut e.s_angles);
    let abs_movedir = [e.movedir[0].abs(), e.movedir[1].abs(), e.movedir[2].abs()];
    let size = sub(&e.maxs, &e.mins);
    let distance = dot(&abs_movedir, &size) - lip;
    e.pos2 = ma(&e.pos1, distance, &e.movedir);
    if e.spawnflags & 1 != 0 {
        let temp = e.pos2;
        e.pos2 = e.s_origin;
        e.pos1 = temp;
    }
    init_mover(w, ent);
    door_think_setup(w, ent, false);
}

/// `SP_func_door_rotating`.
pub fn sp_func_door_rotating(w: &mut World, ent: usize) {
    door_sounds(w, ent, "sound/movers/doors/dr1_strt.wav", "sound/movers/doors/dr1_end.wav");
    w.ents[ent].blocked = Some(blocked_door);
    let e = &mut w.ents[ent];
    if e.speed == 0.0 {
        e.speed = 120.0;
    }
    if e.speed < 0.0 {
        e.speed *= -1.0;
        e.spawnflags |= 8;
    }
    if e.wait == 0.0 {
        e.wait = 2.0;
    }
    e.wait *= 1000.0;
    e.movedir = [0.0; 3];
    e.s_angles = [0.0; 3];
    if e.spawnflags & 32 != 0 {
        e.movedir[2] = 1.0;
    } else if e.spawnflags & 64 != 0 {
        e.movedir[0] = 1.0;
    } else {
        e.movedir[1] = 1.0;
    }
    if e.spawnflags & 8 != 0 {
        e.movedir = neg(&e.movedir);
    }
    if e.rotator_angle == 0.0 {
        e.rotator_angle = 90.0;
    }
    e.pos1 = e.s_angles;
    set_brush_model(w, ent);
    let e = &mut w.ents[ent];
    e.pos2 = ma(&e.pos1, e.rotator_angle, &e.movedir);
    if e.spawnflags & 1 != 0 {
        let temp = e.pos2;
        e.pos2 = e.s_angles;
        e.pos1 = temp;
        e.movedir = neg(&e.movedir);
    }
    e.pos.tr_base = e.s_origin;
    e.origin = e.pos.tr_base;
    init_rotator(w, ent);
    door_think_setup(w, ent, true);
}

/// `SP_func_door_model`.
pub fn sp_func_door_model(w: &mut World, ent: usize) {
    door_sounds(w, ent, "sound/movers/doors/dr1_strt.wav", "sound/movers/doors/dr1_end.wav");
    let e = &mut w.ents[ent];
    if e.speed == 0.0 {
        e.speed = 200.0;
    }
    if e.wait <= 0.0 {
        e.wait = 2.0;
    }
    e.wait *= 1000.0;
    let cb = w.spawn();
    w.ents[ent].clip_brush = Some(cb);
    w.ents[cb].model = w.ents[ent].model.clone();
    w.ents[cb].classname = "door_model_clip";
    set_brush_model(w, cb);
    w.ents[cb].etype = EType::Invisible;
    w.link(cb);
    let (cmin, cmax) = (w.ents[cb].mins, w.ents[cb].maxs);
    w.ents[ent].mins = cmin;
    w.ents[ent].maxs = cmax;
    let o = spawn_vector(w, "modelOrigin", "0 0 0").1;
    w.ents[ent].s_origin = o;
    let sc = spawn_vector(w, "scale", "1 1 1").1;
    w.ents[ent].origin2 = sc;
    if let Some(m2) = w.ents[ent].model2.clone() {
        let idx = w.model_index(&m2);
        w.ents[ent].model_index = idx;
    }
    let (p, s) = spawn_string(w, "noise", "100");
    if p {
        let idx = w.sound_index(&s);
        w.ents[ent].loop_sound = idx;
    }
    let (light_set, light) = spawn_float(w, "light", "100");
    let (color_set, color) = spawn_vector(w, "color", "1 1 1");
    if light_set || color_set {
        let r = ((color[0] * 255.0) as i32).min(255);
        let g = ((color[1] * 255.0) as i32).min(255);
        let b = ((color[2] * 255.0) as i32).min(255);
        let i = ((light / 4.0) as i32).min(255);
        w.ents[ent].constant_light = r | (g << 8) | (b << 16) | (i << 24);
    }
    let e = &mut w.ents[ent];
    e.use_ = Some(use_binary_mover);
    e.mover_state = MoverState::ModelPos1;
    e.etype = EType::ModelDoor;
    e.pos = Trajectory { tr_type: Traj::Stationary as i32, tr_time: 0, tr_duration: 0, tr_base: e.s_origin, tr_delta: [0.0; 3] };
    e.apos = Trajectory { tr_type: Traj::Stationary as i32, tr_time: 0, tr_duration: 0, tr_base: e.s_angles, tr_delta: [0.0; 3] };
    e.misc = e.animation[0] as i32;
    e.weapon = (e.animation[1] as i32).abs();
    if e.weapon == 0 {
        e.weapon = 1;
    }
    e.torso_anim = (e.weapon as f32 * (1000.0f32 / e.speed)) as i32;
    w.link(ent);
    if w.ents[ent].flags & FL_TEAMSLAVE == 0 {
        let health = spawn_int(w, "health", "0").1;
        if health != 0 {
            w.ents[ent].takedamage = true;
        }
        if !(w.ents[ent].targetname.is_some() || health != 0) {
            w.ents[ent].nextthink = w.level.time + FRAMETIME;
            w.ents[ent].think = Some(think_spawn_new_door_trigger);
        }
    }
}

// ------------------------------------------------------------------ plat

/// `Touch_Plat`.
pub fn touch_plat(w: &mut World, ent: usize, other: usize) {
    if w.ents[ent].spawnflags & 1 != 0 {
        return;
    }
    if !is_client(w, other) || w.clients[other].ps.stats[crate::gen::enums::Stat::Health as usize] <= 0 {
        return;
    }
    if w.ents[ent].mover_state == MoverState::Pos2 {
        w.ents[ent].nextthink = w.level.time + 1000;
    }
}

/// `Touch_PlatCenterTrigger`.
pub fn touch_plat_center_trigger(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) {
        return;
    }
    let Some(p) = w.ents[ent].parent else { return };
    if w.ents[p].mover_state == MoverState::Pos1 {
        use_binary_mover(w, p, Some(ent), Some(other));
    }
}

/// `SpawnPlatTrigger`.
fn spawn_plat_trigger(w: &mut World, ent: usize) {
    let t = w.spawn();
    let (pos1, mins, maxs) = (w.ents[ent].pos1, w.ents[ent].mins, w.ents[ent].maxs);
    let mut tmin = [pos1[0] + mins[0] + 33.0, pos1[1] + mins[1] + 33.0, pos1[2] + mins[2]];
    let mut tmax = [pos1[0] + maxs[0] - 33.0, pos1[1] + maxs[1] - 33.0, pos1[2] + maxs[2] + 8.0];
    for i in 0..2 {
        if tmax[i] <= tmin[i] {
            tmin[i] = pos1[i] + ((mins[i] + maxs[i]) as f64 * 0.5) as f32;
            tmax[i] = tmin[i] + 1.0;
        }
    }
    let e = &mut w.ents[t];
    e.classname = "plat_trigger";
    e.touch = Some(touch_plat_center_trigger);
    e.contents = CONTENTS_TRIGGER;
    e.parent = Some(ent);
    e.mins = tmin;
    e.maxs = tmax;
    w.link(t);
}

/// `SP_func_plat`.
pub fn sp_func_plat(w: &mut World, ent: usize) {
    door_sounds(w, ent, "sound/movers/plats/pt1_strt.wav", "sound/movers/plats/pt1_end.wav");
    w.ents[ent].s_angles = [0.0; 3];
    w.ents[ent].speed = spawn_float(w, "speed", "200").1;
    w.ents[ent].damage = spawn_int(w, "dmg", "2").1;
    w.ents[ent].wait = spawn_float(w, "wait", "1").1;
    let lip = spawn_float(w, "lip", "8").1;
    w.ents[ent].wait = 1000.0;
    set_brush_model(w, ent);
    let (hp, mut height) = spawn_float(w, "height", "0");
    if !hp {
        height = (w.ents[ent].maxs[2] - w.ents[ent].mins[2]) - lip;
    }
    let e = &mut w.ents[ent];
    e.pos2 = e.s_origin;
    e.pos1 = e.pos2;
    e.pos1[2] -= height;
    init_mover(w, ent);
    let e = &mut w.ents[ent];
    e.touch = Some(touch_plat);
    e.blocked = Some(blocked_door);
    e.parent = Some(ent);
    if e.targetname.is_none() {
        spawn_plat_trigger(w, ent);
    }
}

// ---------------------------------------------------------------- button

/// `Touch_Button`.
pub fn touch_button(w: &mut World, ent: usize, other: usize) {
    if !is_client(w, other) {
        return;
    }
    if w.ents[ent].mover_state == MoverState::Pos1 {
        use_binary_mover(w, ent, Some(other), Some(other));
    }
}

/// `SP_func_button`.
pub fn sp_func_button(w: &mut World, ent: usize) {
    let s = spawn_string(w, "sound1to2", "sound/movers/switches/button1.wav").1;
    w.ents[ent].sound1to2 = w.sound_index(&s);
    let e = &mut w.ents[ent];
    if e.speed == 0.0 {
        e.speed = 40.0;
    }
    if e.wait == 0.0 {
        e.wait = 1.0;
    }
    e.wait *= 1000.0;
    e.pos1 = e.s_origin;
    set_brush_model(w, ent);
    let lip = spawn_float(w, "lip", "4").1;
    let e = &mut w.ents[ent];
    e.movedir = set_movedir(&mut e.s_angles);
    let a = [e.movedir[0].abs(), e.movedir[1].abs(), e.movedir[2].abs()];
    let size = sub(&e.maxs, &e.mins);
    let distance = a[0] * size[0] + a[1] * size[1] + a[2] * size[2] - lip;
    e.pos2 = ma(&e.pos1, distance, &e.movedir);
    if e.health != 0 {
        e.takedamage = true;
    } else {
        e.touch = Some(touch_button);
    }
    init_mover(w, ent);
}

// ----------------------------------------------------------------- train

/// `Think_BeginMoving`.
pub fn think_begin_moving(w: &mut World, ent: usize) {
    w.ents[ent].pos.tr_time = w.level.time;
    w.ents[ent].pos.tr_type = Traj::LinearStop as i32;
}

/// `Reached_Train`.
pub fn reached_train(w: &mut World, ent: usize) {
    let Some(next) = w.ents[ent].next_train else { return };
    let Some(nn) = w.ents[next].next_train else { return };
    use_targets(w, next, None);
    w.ents[ent].next_train = Some(nn);
    w.ents[ent].pos1 = w.ents[next].s_origin;
    w.ents[ent].pos2 = w.ents[nn].s_origin;
    let mut speed = if w.ents[next].speed != 0.0 { w.ents[next].speed } else { w.ents[ent].speed };
    if speed < 1.0 {
        speed = 1.0;
    }
    w.ents[ent].last_speed = speed;
    let mv = sub(&w.ents[ent].pos2, &w.ents[ent].pos1);
    let len = length(&mv);
    w.ents[ent].pos.tr_duration = (len * 1000.0 / speed) as i32;
    w.ents[ent].svflags &= !SVF_NOCLIENT;
    if w.ents[ent].pos.tr_duration < 1 {
        w.ents[ent].pos.tr_duration = 1;
        w.ents[ent].svflags |= SVF_NOCLIENT;
    }
    w.ents[ent].loop_sound = w.ents[next].sound_loop;
    let t = w.level.time;
    set_mover_state(w, ent, MoverState::OneTo2, t);
    if w.ents[ent].spawnflags & TRAIN_START_OFF != 0 {
        w.ents[ent].pos.tr_type = Traj::Stationary as i32;
        return;
    }
    if w.ents[next].wait != 0.0 {
        w.ents[ent].nextthink = time_plus(t, w.ents[next].wait * 1000.0);
        w.ents[ent].think = Some(think_begin_moving);
        w.ents[ent].pos.tr_type = Traj::Stationary as i32;
    }
}

fn start_train(w: &mut World, ent: usize) {
    let mv = sub(&w.ents[ent].pos2, &w.ents[ent].pos1);
    w.ents[ent].pos.tr_duration = (length(&mv) * 1000.0 / w.ents[ent].last_speed) as i32;
    let t = w.level.time;
    set_mover_state(w, ent, MoverState::OneTo2, t);
    w.ents[ent].spawnflags &= !TRAIN_START_OFF;
}

fn stop_train(w: &mut World, ent: usize) {
    let t = w.level.time;
    let o = evaluate_trajectory(&w.ents[ent].pos, t);
    w.ents[ent].pos1 = o;
    set_mover_state(w, ent, MoverState::Pos1, t);
    w.ents[ent].spawnflags |= TRAIN_START_OFF;
}

/// `Use_Train`.
pub fn use_train(w: &mut World, ent: usize, _other: Option<usize>, _activator: Option<usize>) {
    if w.ents[ent].spawnflags & TRAIN_START_OFF != 0 {
        start_train(w, ent);
    } else {
        stop_train(w, ent);
    }
}

/// `Think_SetupTrainTargets`.
pub fn think_setup_train_targets(w: &mut World, ent: usize) {
    let Some(target) = w.ents[ent].target.clone() else { return };
    let Some(first) = find_targetname(w, None, &target) else { return };
    w.ents[ent].next_train = Some(first);
    let mut start: Option<usize> = None;
    let mut path = first;
    while Some(path) != start {
        if start.is_none() {
            start = Some(path);
        }
        let Some(pt) = w.ents[path].target.clone() else { return };
        let mut next = None;
        loop {
            next = find_targetname(w, next, &pt);
            match next {
                None => return,
                Some(n) if w.ents[n].classname == "path_corner" => break,
                _ => {}
            }
        }
        let n = next.unwrap();
        w.ents[path].next_train = Some(n);
        path = n;
    }
    reached_train(w, ent);
}

/// `SP_path_corner`.
pub fn sp_path_corner(w: &mut World, ent: usize) {
    if w.ents[ent].targetname.is_none() {
        w.free(ent);
    }
}

/// `Blocked_Train`.
pub fn blocked_train(w: &mut World, ent: usize, other: usize) {
    if w.ents[ent].spawnflags & TRAIN_BLOCK_STOPS != 0 {
        stop_train(w, ent);
        return;
    }
    if !is_client(w, other) {
        combat::damage(w, other, Some(ent), Some(ent), None, None, 10000, 0, Mod::Crush);
        if w.ents[other].etype == EType::Buildable && w.ents[other].spawned {
            let o = w.ents[other].origin;
            match w.ents[other].team {
                Team::Aliens => {
                    let dir = w.ents[other].origin2;
                    w.temp_event(o, Event::AlienBuildableExplosion, -1, dir_to_byte(&dir), 0);
                }
                Team::Humans => w.temp_event(o, Event::HumanBuildableExplosion, -1, dir_to_byte(&[0.0, 0.0, 1.0]), 0),
                Team::None => {}
            }
        }
        w.free(other);
        return;
    }
    combat::damage(w, other, Some(ent), Some(ent), None, None, 10000, 0, Mod::Crush);
}

/// `SP_func_train`.
pub fn sp_func_train(w: &mut World, ent: usize) {
    let e = &mut w.ents[ent];
    e.s_angles = [0.0; 3];
    if e.spawnflags & TRAIN_BLOCK_STOPS != 0 {
        e.damage = 0;
    } else if e.damage == 0 {
        e.damage = 2;
    }
    if e.speed == 0.0 {
        e.speed = 100.0;
    }
    if e.target.is_none() {
        w.free(ent);
        return;
    }
    set_brush_model(w, ent);
    init_mover(w, ent);
    let e = &mut w.ents[ent];
    e.reached = Some(reached_train);
    e.use_ = Some(use_train);
    e.blocked = Some(blocked_train);
    e.nextthink = w.level.time + FRAMETIME;
    e.think = Some(think_setup_train_targets);
}

// ---------------------------------------------------- continuous movers

/// `SP_func_static`.
pub fn sp_func_static(w: &mut World, ent: usize) {
    set_brush_model(w, ent);
    init_mover(w, ent);
    let o = w.ents[ent].s_origin;
    w.ents[ent].pos.tr_base = o;
    w.ents[ent].origin = o;
}

/// `SP_func_rotating`.
pub fn sp_func_rotating(w: &mut World, ent: usize) {
    let e = &mut w.ents[ent];
    if e.speed == 0.0 {
        e.speed = 100.0;
    }
    e.apos.tr_type = Traj::Linear as i32;
    if e.spawnflags & 4 != 0 {
        e.apos.tr_delta[2] = e.speed;
    } else if e.spawnflags & 8 != 0 {
        e.apos.tr_delta[0] = e.speed;
    } else {
        e.apos.tr_delta[1] = e.speed;
    }
    if e.damage == 0 {
        e.damage = 2;
    }
    set_brush_model(w, ent);
    init_mover(w, ent);
    let e = &mut w.ents[ent];
    e.pos.tr_base = e.s_origin;
    e.origin = e.pos.tr_base;
    e.angles = e.apos.tr_base;
    w.link(ent);
}

/// `SP_func_bobbing`.
pub fn sp_func_bobbing(w: &mut World, ent: usize) {
    w.ents[ent].speed = spawn_float(w, "speed", "4").1;
    let height = spawn_float(w, "height", "32").1;
    w.ents[ent].damage = spawn_int(w, "dmg", "2").1;
    let phase = spawn_float(w, "phase", "0").1;
    set_brush_model(w, ent);
    init_mover(w, ent);
    let e = &mut w.ents[ent];
    e.pos.tr_base = e.s_origin;
    e.origin = e.s_origin;
    e.pos.tr_duration = (e.speed * 1000.0) as i32;
    e.pos.tr_time = (e.pos.tr_duration as f32 * phase) as i32;
    e.pos.tr_type = Traj::Sine as i32;
    if e.spawnflags & 1 != 0 {
        e.pos.tr_delta[0] = height;
    } else if e.spawnflags & 2 != 0 {
        e.pos.tr_delta[1] = height;
    } else {
        e.pos.tr_delta[2] = height;
    }
}

/// `SP_func_pendulum`.
pub fn sp_func_pendulum(w: &mut World, ent: usize) {
    let speed = spawn_float(w, "speed", "30").1;
    w.ents[ent].damage = spawn_int(w, "dmg", "2").1;
    let phase = spawn_float(w, "phase", "0").1;
    set_brush_model(w, ent);
    let mut len = w.ents[ent].mins[2].abs();
    if len < 8.0 {
        len = 8.0;
    }
    let freq = (1.0 / (M_PI * 2.0) * ((w.level.g_gravity / (3.0 * len)) as f64).sqrt()) as f32;
    w.ents[ent].pos.tr_duration = (1000.0f32 / freq) as i32;
    init_mover(w, ent);
    let e = &mut w.ents[ent];
    e.pos.tr_base = e.s_origin;
    e.origin = e.s_origin;
    e.apos.tr_base = e.s_angles;
    e.apos.tr_duration = (1000.0f32 / freq) as i32;
    e.apos.tr_time = (e.apos.tr_duration as f32 * phase) as i32;
    e.apos.tr_type = Traj::Sine as i32;
    e.apos.tr_delta[2] = speed;
}
