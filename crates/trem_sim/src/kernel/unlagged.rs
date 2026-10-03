//! Kernel `unlagged` — the unlagged system of game/g_active.c (dec033,
//! supersedes the unlagged part of dec027): `G_UnlaggedStore` keeps the
//! clients' boxes every frame (the `unlagged_store` system after
//! ClientEndFrame), `G_UnlaggedCalc` interpolates them back to the time the
//! shooter's usercmd was issued (cmd.serverTime), `G_UnlaggedOn` /
//! `G_UnlaggedOff` move the other players there for the hit traces of
//! g_weapon.c and back, `G_UnlaggedDetectCollisions` drops players the
//! shooter's own movement ran into, `G_UnlaggedClear` forgets a client's
//! history on spawn and teleport.
//!
//! In a single process every usercmd runs at level.time, so the newest
//! marker is never older than the command and Calc stops at once: the
//! rewind is the identity there, as before.

use super::qmath::*;
use super::world::{World, MAX_CLIENTS};
use crate::gen::enums::contents;

/// `MAX_UNLAGGED_MARKERS`: 10 frames of the C's sv_fps 20 are 500 ms of
/// history; the port's server frames are 16 ms (sv_fps 60, dec033), so it
/// keeps 30 markers to cover the same 500 ms.
pub const MAX_UNLAGGED_MARKERS: usize = 30;

/// `unlagged_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Unlagged {
    pub mins: Vec3,
    pub maxs: Vec3,
    pub origin: Vec3,
    pub used: bool,
}

/// A client's unlagged state (`unlaggedHist`, `unlaggedBackup`,
/// `unlaggedCalc`, `unlaggedTime`, `oldOrigin` of gclient_t).
#[derive(Clone, Debug, Default)]
pub struct ClientUnlagged {
    pub hist: [Unlagged; MAX_UNLAGGED_MARKERS],
    pub backup: Unlagged,
    pub calc: Unlagged,
    pub time: i32,
    pub old_origin: Vec3,
}

fn body_linked(w: &World, i: usize) -> bool {
    w.ents[i].inuse && w.ents[i].linked && w.ents[i].contents & contents::BODY as i32 != 0 && w.clients[i].connected
}

/// `G_UnlaggedStore` (the `unlagged_store` system).
pub fn store(w: &mut World) {
    if !w.level.unlagged {
        return;
    }
    w.level.unlagged_index = (w.level.unlagged_index + 1) % MAX_UNLAGGED_MARKERS;
    let idx = w.level.unlagged_index;
    w.level.unlagged_times[idx] = w.level.time;
    for i in 0..MAX_CLIENTS {
        let used = body_linked(w, i);
        let e = &w.ents[i];
        let save = Unlagged { mins: e.mins, maxs: e.maxs, origin: e.pos.tr_base, used };
        w.clients[i].unlagged.hist[idx] = if used { save } else { Unlagged::default() };
    }
}

/// `G_UnlaggedClear`: forget the history (death, spawn, teleport).
pub fn clear(w: &mut World, i: usize) {
    for h in w.clients[i].unlagged.hist.iter_mut() {
        h.used = false;
    }
}

/// `G_UnlaggedCalc`: where every other client was at `time`.
pub fn calc(w: &mut World, time: i32, rewind: usize) {
    if !w.level.unlagged {
        return;
    }
    // clear any calculated values from a previous run
    for i in 0..MAX_CLIENTS {
        w.clients[i].unlagged.calc.used = false;
    }
    let mut start = w.level.unlagged_index;
    let mut stop: Option<usize> = None;
    let mut lerp = 0.5f32;
    let mut i = 0;
    while i < MAX_UNLAGGED_MARKERS {
        if w.level.unlagged_times[start] <= time {
            break;
        }
        stop = Some(start);
        start = if start == 0 { MAX_UNLAGGED_MARKERS - 1 } else { start - 1 };
        i += 1;
    }
    if i == MAX_UNLAGGED_MARKERS {
        // if we searched all markers and the oldest one still isn't old enough
        // just use the oldest marker with no lerping
        lerp = 0.0;
    }
    // client is on the current frame, no need for unlagged
    let Some(stop) = stop else { return };
    // lerp between two markers
    let frame_msec = w.level.unlagged_times[stop] - w.level.unlagged_times[start];
    if frame_msec > 0 {
        lerp = (time - w.level.unlagged_times[start]) as f32 / frame_msec as f32;
    }
    for i in 0..MAX_CLIENTS {
        if i == rewind || !body_linked(w, i) {
            continue;
        }
        let u = &mut w.clients[i].unlagged;
        let (a, b) = (u.hist[start], u.hist[stop]);
        if !a.used || !b.used {
            continue;
        }
        // between two unlagged markers
        u.calc = Unlagged { mins: lerp3(lerp, &a.mins, &b.mins), maxs: lerp3(lerp, &a.maxs, &b.maxs), origin: lerp3(lerp, &a.origin, &b.origin), used: true };
    }
}

/// `VectorLerp2`.
fn lerp3(f: f32, a: &Vec3, b: &Vec3) -> Vec3 {
    [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1]), a[2] + f * (b[2] - a[2])]
}

/// `G_UnlaggedOff`: everyone back where they are.
pub fn off(w: &mut World) {
    if !w.level.unlagged {
        return;
    }
    for i in 0..MAX_CLIENTS {
        let b = w.clients[i].unlagged.backup;
        if !b.used {
            continue;
        }
        let e = &mut w.ents[i];
        e.mins = b.mins;
        e.maxs = b.maxs;
        e.origin = b.origin;
        w.clients[i].unlagged.backup.used = false;
        w.link(i);
    }
}

/// `G_UnlaggedOn`: move the clients near `muzzle` (all with None) to their
/// calculated positions for the attacker's traces.
pub fn on(w: &mut World, attacker: usize, muzzle: Option<&Vec3>, range: f32) {
    if !w.level.unlagged || attacker >= MAX_CLIENTS || !w.clients[attacker].use_unlagged {
        return;
    }
    for i in 0..MAX_CLIENTS {
        let calc = w.clients[i].unlagged.calc;
        if !calc.used || w.clients[i].unlagged.backup.used || !body_linked(w, i) {
            continue;
        }
        if w.ents[i].origin == calc.origin {
            continue;
        }
        if let Some(m) = muzzle {
            let r1 = distance(&calc.origin, &calc.maxs);
            let r2 = distance(&calc.origin, &calc.mins);
            if distance(m, &calc.origin) > range + r1.max(r2) {
                continue;
            }
        }
        // create a backup of the real positions
        let e = &w.ents[i];
        w.clients[i].unlagged.backup = Unlagged { mins: e.mins, maxs: e.maxs, origin: e.origin, used: true };
        // move the client to the calculated unlagged position
        let e = &mut w.ents[i];
        e.mins = calc.mins;
        e.maxs = calc.maxs;
        e.origin = calc.origin;
        w.link(i);
    }
}

/// `G_UnlaggedDetectCollisions` (after Pmove and G_UnlaggedCalc).
pub fn detect_collisions(w: &mut World, i: usize) {
    if !w.level.unlagged || !w.clients[i].use_unlagged {
        return;
    }
    let calc = w.clients[i].unlagged.calc;
    let old = w.clients[i].unlagged.old_origin;
    let now = w.clients[i].ps.origin;
    // if the client isn't moving, this is not necessary
    if old == now {
        return;
    }
    let mut range = distance(&old, &now);
    // increase the range by the player's largest possible radius since it's
    // the players bounding box that collides, not their origin
    let r1 = distance(&calc.origin, &calc.mins);
    let r2 = distance(&calc.origin, &calc.maxs);
    range += r1.max(r2);
    on(w, i, Some(&old), range);
    let (mins, maxs) = (w.ents[i].mins, w.ents[i].maxs);
    let tr = w.trace(&old, Some(&mins), Some(&maxs), &now, i as i32, super::types::k::MASK_PLAYERSOLID);
    if tr.entity_num >= 0 && (tr.entity_num as usize) < MAX_CLIENTS {
        w.clients[tr.entity_num as usize].unlagged.calc.used = false;
    }
    off(w);
}
