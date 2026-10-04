//! Kernel `weapon` — port of game/g_weapon.c. The FireWeapon switch is the
//! weapon_behaviors.tsv fn table; per-weapon constants (range, width, damage,
//! spread, means of death, missile row) are columns of that sheet, so one
//! kernel function serves many rows (MDD §7.4). The hit traces run with
//! the other players moved to where the shooter saw them (`G_UnlaggedOn` /
//! `G_UnlaggedOff`, unlagged.rs, dec033).

use super::bg;
use super::combat::{self, DAMAGE_NO_KNOCKBACK, DAMAGE_NO_LOCDAMAGE};
use super::qmath::*;
use super::types::*;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::enums::{surface, Event, MeansOfDeath as Mod, Stat, Team, WeaponMode};
use crate::gen::weapon_behaviors::{for_target, WeaponBehaviorDef};
use crate::gen::weapons::WeaponId;

#[derive(Clone, Copy, Debug)]
pub struct Aim {
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub muzzle: Vec3,
}

/// `CalcMuzzlePoint` + the AngleVectors preamble of FireWeapon.
pub fn aim(w: &World, ent: usize) -> Aim {
    if ent < MAX_CLIENTS && w.ents[ent].client {
        let ps = &w.clients[ent].ps;
        let (f, r, u) = angle_vectors(&ps.viewangles);
        let normal = bg::client_normal(ps);
        let mut m = ma(&ps.origin, ps.viewheight as f32, &normal);
        m = ma(&m, 1.0, &f);
        for c in m.iter_mut() {
            *c = (*c as i32) as f32; // SnapVector truncates
        }
        Aim { forward: f, right: r, up: u, muzzle: m }
    } else {
        let e = &w.ents[ent];
        let (f, r, u) = angle_vectors(&e.turret_aim);
        Aim { forward: f, right: r, up: u, muzzle: e.pos.tr_base }
    }
}

fn behavior(w: &World, ent: usize) -> Option<&'static WeaponBehaviorDef> {
    WeaponId::from_num(w.ents[ent].weapon as u8).and_then(for_target)
}

/// `FireWeapon`, `FireWeapon2`, `FireWeapon3` via the fn table.
pub fn fire_weapon(w: &mut World, ent: usize, mode: i32) {
    let Some(b) = behavior(w, ent) else { return };
    let f = match mode {
        1 => b.fire1,
        2 => b.fire2,
        _ => b.fire3,
    };
    if let Some(f) = f {
        f(w, ent);
    }
}

pub fn noop(_: &mut World, _: usize) {}

fn snap_vector_towards(v: &mut Vec3, to: &Vec3) {
    for i in 0..3 {
        if v[i] >= 0.0 {
            v[i] = ((v[i] + if to[i] <= v[i] { 0.0 } else { 1.0 }) as i32) as f32;
        } else {
            v[i] = ((v[i] + if to[i] <= v[i] { -1.0 } else { 0.0 }) as i32) as f32;
        }
    }
}

/// `G_WideTrace`: box vs bodies, then line of sight vs world.
fn wide_trace(w: &mut World, ent: usize, a: &Aim, range: f32, width: f32, height: f32) -> (super::cm::Trace, Option<usize>) {
    if ent >= MAX_CLIENTS || !w.ents[ent].client {
        return (super::cm::Trace::default(), None);
    }
    super::unlagged::on(w, ent, Some(&a.muzzle), range + width);
    let r = wide_trace_body(w, ent, a, range, width, height);
    super::unlagged::off(w);
    r
}
fn wide_trace_body(w: &World, ent: usize, a: &Aim, range: f32, width: f32, height: f32) -> (super::cm::Trace, Option<usize>) {
    let mins = [-width, -width, -height];
    let maxs = [width, width, height];
    let end = ma(&a.muzzle, range, &a.forward);
    let mut tr = w.trace(&a.muzzle, Some(&mins), Some(&maxs), &end, ent as i32, crate::gen::enums::contents::BODY as i32);
    let mut target = if tr.entity_num != ENTITYNUM_NONE { Some(tr.entity_num as usize) } else { None };
    let range2 = distance(&a.muzzle, &tr.endpos) + width;
    let end = ma(&a.muzzle, range2, &a.forward);
    tr = w.trace(&a.muzzle, None, None, &end, ent as i32, crate::gen::enums::contents::SOLID as i32);
    if tr.entity_num != ENTITYNUM_NONE {
        target = Some(tr.entity_num as usize);
    }
    (tr, target)
}

fn blood(w: &mut World, attacker: usize, victim: usize, at: Vec3) {
    if w.ents[victim].health > 0 {
        let wpn = w.ents[attacker].weapon;
        let _ = wpn;
        w.temp_event_from(at, Event::MissileHit, victim as i32, 0, attacker);
    }
}

/// `meleeAttack` (claws, bites): range/width/height/damage/mod from the sheet.
pub fn melee_attack(w: &mut World, ent: usize) {
    let Some(b) = behavior(w, ent) else { return };
    let a = aim(w, ent);
    let (tr, target) = wide_trace(w, ent, &a, b.range.unwrap_or(64.0), b.width.unwrap_or(0.0), b.height.unwrap_or(0.0));
    let Some(t) = target else { return };
    if !w.ents[t].takedamage {
        return;
    }
    blood(w, ent, t, tr.endpos);
    combat::damage(w, t, Some(ent), Some(ent), Some(a.forward), Some(tr.endpos), b.damage.unwrap_or(0), DAMAGE_NO_KNOCKBACK, b.r#mod.unwrap_or(Mod::Unknown));
}

/// `bulletFire` (rifle, chaingun, mg turret).
pub fn bullet_fire(w: &mut World, ent: usize) {
    let Some(b) = behavior(w, ent) else { return };
    let a = aim(w, ent);
    let spread = b.spread.unwrap_or(0) as f32;
    let mut r = (w.rng.random() as f64 * M_PI * 2.0f32 as f64) as f32;
    let u = ((r as f64).sin() * w.rng.crandom() * spread as f64 * 16.0) as f32;
    r = ((r as f64).cos() * w.rng.crandom() * spread as f64 * 16.0) as f32;
    let mut end = ma(&a.muzzle, 8192.0 * 16.0, &a.forward);
    end = ma(&end, r, &a.right);
    end = ma(&end, u, &a.up);
    // don't use unlagged if this is not a client (e.g. turret)
    let client = ent < MAX_CLIENTS && w.ents[ent].client;
    if client {
        super::unlagged::on(w, ent, Some(&a.muzzle), 8192.0 * 16.0);
    }
    let mut tr = w.trace(&a.muzzle, None, None, &end, ent as i32, k::MASK_SHOT);
    if client {
        super::unlagged::off(w);
    }
    if tr.surface_flags & surface::NOIMPACT as i32 != 0 {
        return;
    }
    let t = tr.entity_num as usize;
    snap_vector_towards(&mut tr.endpos, &a.muzzle);
    let flesh = t < w.ents.len() && w.ents[t].takedamage && matches!(w.ents[t].etype, EType::Player | EType::Buildable);
    if flesh {
        w.temp_event_from(tr.endpos, Event::BulletHitFlesh, ent as i32, t as i32, ent);
    } else {
        let parm = dir_to_byte(&tr.plane.normal);
        w.temp_event_from(tr.endpos, Event::BulletHitWall, ent as i32, parm, ent);
    }
    if t < w.ents.len() && w.ents[t].takedamage {
        combat::damage(w, t, Some(ent), Some(ent), Some(a.forward), Some(tr.endpos), b.damage.unwrap_or(0), 0, b.r#mod.unwrap_or(Mod::Unknown));
    }
}

/// `shotgunFire` + `ShotgunPattern`.
pub fn shotgun_fire(w: &mut World, ent: usize) {
    let Some(b) = behavior(w, ent) else { return };
    let a = aim(w, ent);
    let mut origin2 = scale(&a.forward, 4096.0);
    for c in origin2.iter_mut() {
        *c = (*c as i32) as f32;
    }
    let mut seed = w.rng.rand() / (super::rng::RAND_MAX / 0x100 + 1);
    w.temp_event_from(a.muzzle, Event::Shotgun, ent as i32, seed, ent);
    if let Some(ev) = w.events.last_mut() {
        ev.origin2 = origin2;
    }
    let mut forward = [0.0; 3];
    normalize2(&origin2, &mut forward);
    let right = perpendicular_vector(&forward);
    let up = cross(&forward, &right);
    let spread = b.spread.unwrap_or(0) as f32;
    // ShotgunPattern: the pellets trace with unlagged on
    let mut hits = Vec::new();
    super::unlagged::on(w, ent, Some(&a.muzzle), k::SHOTGUN_RANGE as f32);
    for _ in 0..k::SHOTGUN_PELLETS {
        let r = q_crandom(&mut seed) * spread * 16.0;
        let u = q_crandom(&mut seed) * spread * 16.0;
        let mut end = ma(&a.muzzle, b.range.unwrap_or(k::SHOTGUN_RANGE as f32), &forward);
        end = ma(&end, r, &right);
        end = ma(&end, u, &up);
        let tr = w.trace(&a.muzzle, None, None, &end, ent as i32, k::MASK_SHOT);
        let t = tr.entity_num as usize;
        if tr.surface_flags & surface::NOIMPACT as i32 == 0 && t < w.ents.len() && w.ents[t].takedamage {
            hits.push((t, tr.endpos));
        }
    }
    // the C damages inside the pattern loop, with unlagged on; G_Damage
    // reads the unlagged calc itself, so applying it after Off is the same
    // except for the trace order of later pellets against a body killed by
    // an earlier one (the body stays in place for this frame either way)
    super::unlagged::off(w);
    for (t, at) in hits {
        if w.ents[t].takedamage {
            combat::damage(w, t, Some(ent), Some(ent), Some(forward), Some(at), b.damage.unwrap_or(0), 0, Mod::Shotgun);
        }
    }
}

pub fn q_crandom(seed: &mut i32) -> f32 {
    *seed = seed.wrapping_mul(69069).wrapping_add(1);
    let r = (*seed & 0xffff) as f32 / 0x10000 as f32;
    (2.0 * (r as f64 - 0.5)) as f32
}

/// `massDriverFire` / `lasGunFire`: instant hit along the view axis.
pub fn instant_fire(w: &mut World, ent: usize) {
    let Some(b) = behavior(w, ent) else { return };
    let a = aim(w, ent);
    let end = ma(&a.muzzle, 8192.0 * 16.0, &a.forward);
    super::unlagged::on(w, ent, Some(&a.muzzle), 8192.0 * 16.0);
    let mut tr = w.trace(&a.muzzle, None, None, &end, ent as i32, k::MASK_SHOT);
    super::unlagged::off(w);
    if tr.surface_flags & surface::NOIMPACT as i32 != 0 {
        return;
    }
    let t = tr.entity_num as usize;
    snap_vector_towards(&mut tr.endpos, &a.muzzle);
    if t < w.ents.len() && w.ents[t].takedamage && matches!(w.ents[t].etype, EType::Player | EType::Buildable) {
        blood(w, ent, t, tr.endpos);
    } else {
        let parm = dir_to_byte(&tr.plane.normal);
        w.temp_event_from(tr.endpos, Event::MissileMiss, ent as i32, parm, ent);
    }
    if t < w.ents.len() && w.ents[t].takedamage {
        combat::damage(w, t, Some(ent), Some(ent), Some(a.forward), Some(tr.endpos), b.damage.unwrap_or(0), 0, b.r#mod.unwrap_or(Mod::Unknown));
    }
}

/// `painSawFire`.
pub fn pain_saw_fire(w: &mut World, ent: usize) {
    let Some(b) = behavior(w, ent) else { return };
    let a = aim(w, ent);
    let (mut tr, target) = wide_trace(w, ent, &a, b.range.unwrap_or(64.0), b.width.unwrap_or(0.0), b.height.unwrap_or(8.0));
    let Some(t) = target else { return };
    if !w.ents[t].takedamage {
        return;
    }
    tr.endpos[2] -= 5.0;
    blood(w, ent, t, tr.endpos);
    combat::damage(w, t, Some(ent), Some(ent), Some(a.forward), Some(tr.endpos), b.damage.unwrap_or(0), DAMAGE_NO_KNOCKBACK, Mod::Painsaw);
}

/// Projectile weapons: missile row named by the behaviour row.
pub fn missile_fire(w: &mut World, ent: usize) {
    let Some(b) = behavior(w, ent) else { return };
    let Some(m) = b.missile else { return };
    let a = aim(w, ent);
    let start = if m == crate::gen::missiles::MissileId::Hive { ma(&a.muzzle, w.ents[ent].maxs[2], &w.ents[ent].origin2) } else { a.muzzle };
    super::missile::fire(w, ent, m, start, a.forward, None);
}

/// `flamerFire`: muzzle corrected down and to the hand.
pub fn flamer_fire(w: &mut World, ent: usize) {
    let a = aim(w, ent);
    let mut origin = ma(&a.muzzle, -7.0, &a.up);
    origin = ma(&origin, 4.5, &a.right);
    super::missile::fire(w, ent, crate::gen::missiles::MissileId::Flame, origin, a.forward, None);
}

/// `LCChargeFire(ent, qfalse)`.
pub fn lcannon_fire(w: &mut World, ent: usize) {
    lc_charge_fire(w, ent, false)
}
/// `LCChargeFire(ent, qtrue)`.
pub fn lcannon_fire2(w: &mut World, ent: usize) {
    lc_charge_fire(w, ent, true)
}
fn lc_charge_fire(w: &mut World, ent: usize, secondary: bool) {
    let a = aim(w, ent);
    let misc = w.clients[ent].ps.stat(Stat::Misc);
    let p = if secondary && misc <= 0 {
        (k::LCANNON_SECONDARY_DAMAGE, k::LCANNON_SECONDARY_RADIUS, k::LCANNON_SECONDARY_SPEED)
    } else {
        (misc * k::LCANNON_DAMAGE / k::LCANNON_CHARGE_TIME_MAX, k::LCANNON_RADIUS, k::LCANNON_SPEED)
    };
    super::missile::fire(w, ent, crate::gen::missiles::MissileId::Lcannon, a.muzzle, a.forward, Some(p));
    w.clients[ent].ps.stats[Stat::Misc as usize] = 0;
}

/// `teslaFire` (enemy chosen by HTeslaGen_Think).
pub fn tesla_fire(w: &mut World, ent: usize) {
    let Some(enemy) = w.ents[ent].enemy else { return };
    let origin = ma(&w.ents[ent].pos.tr_base, w.ents[ent].maxs[2], &w.ents[ent].origin2);
    let mut target = w.ents[enemy].origin;
    target[2] += w.ents[enemy].maxs[2];
    let tr = w.trace(&origin, None, None, &target, ent as i32, k::MASK_SHOT);
    if tr.entity_num != enemy as i32 {
        return;
    }
    w.ents[ent].e_flags |= k::EF_FIRING;
    if w.ents[enemy].takedamage {
        let dir = sub(&target, &origin);
        combat::damage(w, enemy, Some(ent), Some(ent), Some(dir), Some(tr.endpos), k::TESLAGEN_DMG, 0, Mod::Teslagen);
    }
    w.temp_event(tr.endpos, Event::Teslatrail, enemy as i32, ent as i32, 0);
}

/// `buildFire`: place the selected buildable (STAT_BUILDABLE) if valid.
pub fn build_fire(w: &mut World, ent: usize) {
    let sel = w.clients[ent].ps.stat(Stat::Buildable) & !k::SB_VALID_TOGGLEBIT;
    if sel > 0 {
        if w.clients[ent].ps.stat(Stat::Misc) > 0 {
            w.add_event(ent, Event::BuildDelay, ent as i32);
            return;
        }
        if let Some(b) = crate::gen::buildables::BuildableId::from_num(sel as u8) {
            match super::buildable::build_if_valid(w, ent, b) {
                Ok(_) => {
                    if !w.level.cheats {
                        w.clients[ent].ps.stats[Stat::Misc as usize] += b.def().build_time;
                    }
                    w.clients[ent].ps.stats[Stat::Buildable as usize] = 0;
                }
                Err(reason) => w.clients[ent].message = Some((w.level.time, reason.to_string())),
            }
        }
        return;
    }
    // G_TriggerMenu( ent->client->ps.clientNum, menu ): MN_A_BUILD /
    // MN_H_BUILD, the build menu of the builder's team
    w.clients[ent].wants_menu = true;
    super::client::notify(w, ent, BUILD_MENU_NOTICE);
}

/// The notice `build_fire` sends with G_TriggerMenu(MN_A_BUILD / MN_H_BUILD)
/// (the presentation opens tremulous_alienbuild / tremulous_humanbuild).
pub const BUILD_MENU_NOTICE: &str = "Choose a structure to build";

/// `cancelBuildFire`: clear the ghost, or (ckit) repair a friendly buildable.
pub fn cancel_build_fire(w: &mut World, ent: usize) {
    if w.clients[ent].ps.stat(Stat::Buildable) & !k::SB_VALID_TOGGLEBIT > 0 {
        w.clients[ent].ps.stats[Stat::Buildable as usize] = 0;
        return;
    }
    // (the ckit repairs from ClientThink, CheckCkitRepair: not here)
    if w.clients[ent].ps.weapon != WeaponId::Ckit as i32 {
        // granger claw swipe
        let a = aim(w, ent);
        let (tr, target) = wide_trace(w, ent, &a, k::ABUILDER_CLAW_RANGE, k::ABUILDER_CLAW_WIDTH, k::ABUILDER_CLAW_WIDTH);
        if let Some(t) = target {
            if w.ents[t].takedamage {
                combat::damage(w, t, Some(ent), Some(ent), Some(a.forward), Some(tr.endpos), k::ABUILDER_CLAW_DMG, DAMAGE_NO_KNOCKBACK, Mod::AbuilderClaw);
            }
        }
    }
}

/// `CheckCkitRepair` (from ClientThink while the ckit is held): a damaged
/// human structure in the crosshair within 100 units gains HBUILD_HEALRATE,
/// then the ckit waits its repeat rate (HBUILD_REPEAT): 18 health a second,
/// not every frame. The repair events go on the builder.
pub fn check_ckit_repair(w: &mut World, ent: usize) {
    let ps = &w.clients[ent].ps;
    if ps.weapon_time > 0 || ps.stat(Stat::Misc) > 0 {
        return;
    }
    let a = aim(w, ent);
    let view = bg::client_view_origin(&w.clients[ent].ps);
    let end = ma(&view, 100.0, &a.forward);
    let tr = w.trace(&view, None, None, &end, ent as i32, k::MASK_PLAYERSOLID);
    let t = tr.entity_num as usize;
    if tr.fraction >= 1.0 || t >= w.ents.len() || !w.ents[t].spawned || w.ents[t].health <= 0 || w.ents[t].etype != EType::Buildable || w.ents[t].team != Team::Humans {
        return;
    }
    let Some(b) = w.ents[t].buildable else { return };
    let full = b.def().health;
    if w.ents[t].health < full {
        w.ents[t].health += k::HBUILD_HEALRATE;
        if w.ents[t].health >= full {
            w.ents[t].health = full;
            w.add_event(ent, Event::BuildRepaired, 0);
        } else {
            w.add_event(ent, Event::BuildRepair, 0);
        }
        let rate = bg::weapon(w.clients[ent].ps.weapon).repeat_rate1;
        w.clients[ent].ps.weapon_time += rate;
    }
}

/// `CheckVenomAttack` (dretch auto-bite), called from ClientThink.
pub fn check_venom_attack(w: &mut World, ent: usize) -> bool {
    if w.clients[ent].ps.weapon_time != 0 {
        return false;
    }
    let a = aim(w, ent);
    let (tr, target) = wide_trace(w, ent, &a, k::LEVEL0_BITE_RANGE, k::LEVEL0_BITE_WIDTH, k::LEVEL0_BITE_WIDTH);
    let Some(t) = target else { return false };
    let te = &w.ents[t];
    if !te.takedamage || te.health <= 0 {
        return false;
    }
    if te.etype == EType::Buildable && (te.spawned || te.team == Team::Aliens) {
        return false;
    }
    if t < MAX_CLIENTS && te.client && (w.clients[t].ps.stat(Stat::Team) == Team::Aliens as i32 || w.clients[t].ps.stat(Stat::Health) <= 0) {
        return false;
    }
    blood(w, ent, t, tr.endpos);
    combat::damage(w, t, Some(ent), Some(ent), Some(a.forward), Some(tr.endpos), k::LEVEL0_BITE_DMG, DAMAGE_NO_KNOCKBACK, Mod::Level0Bite);
    w.clients[ent].ps.weapon_time += k::LEVEL0_BITE_REPEAT;
    true
}

/// `CheckGrabAttack` (basilisk grab).
pub fn check_grab_attack(w: &mut World, ent: usize) {
    let a = aim(w, ent);
    let range = if w.clients[ent].ps.weapon == WeaponId::Level1 as i32 { k::LEVEL1_GRAB_RANGE } else { k::LEVEL1_GRAB_U_RANGE };
    let end = ma(&a.muzzle, range, &a.forward);
    let tr = w.trace(&a.muzzle, None, None, &end, ent as i32, k::MASK_SHOT);
    if tr.surface_flags & surface::NOIMPACT as i32 != 0 {
        return;
    }
    let t = tr.entity_num as usize;
    if t >= MAX_CLIENTS || !w.ents[t].takedamage || !w.ents[t].client {
        return;
    }
    if w.clients[t].ps.stat(Stat::Team) == Team::Aliens as i32 || w.clients[t].ps.stat(Stat::Health) <= 0 {
        return;
    }
    if w.clients[t].ps.stat(Stat::State) & k::SS_GRABBED == 0 {
        let (dir, _, _) = angle_vectors(&w.clients[t].ps.viewangles);
        w.clients[t].ps.stats[Stat::Viewlock as usize] = dir_to_byte(&dir);
        add_predictable_event(&mut w.clients[ent].ps, Event::Lev1Grab as i32, 0);
    }
    w.clients[t].ps.stats[Stat::State as usize] |= k::SS_GRABBED;
    let time = if w.clients[ent].ps.weapon == WeaponId::Level1 as i32 { k::LEVEL1_GRAB_TIME } else { k::LEVEL1_GRAB_U_TIME };
    w.clients[t].grab_expiry_time = w.level.time + time;
}

/// `poisonCloud` (advanced basilisk alt fire).
pub fn poison_cloud(w: &mut World, ent: usize) {
    let r = k::LEVEL1_PCLOUD_RANGE;
    let o = w.clients[ent].ps.origin;
    let mins = [o[0] - r, o[1] - r, o[2] - r];
    let maxs = [o[0] + r, o[1] + r, o[2] + r];
    super::unlagged::on(w, ent, Some(&o), r);
    let hits = w.entities_in_box(&mins, &maxs);
    let mut clouded = Vec::new();
    for h in hits {
        if h >= MAX_CLIENTS || !w.ents[h].client || w.clients[h].ps.stat(Stat::Team) != Team::Humans as i32 {
            continue;
        }
        if !w.visible(ent, h, crate::gen::enums::contents::SOLID as i32) {
            continue;
        }
        clouded.push(h);
    }
    super::unlagged::off(w);
    for h in clouded {
        w.clients[h].ps.e_flags |= k::EF_POISONCLOUDED;
        w.clients[h].last_poison_clouded_time = w.level.time;
    }
}

/// `G_FindZapChainTargets`: living humans and human structures within chain
/// range of the first target with world line of sight.
fn find_zap_chain_targets(w: &World, zap: &mut super::world::Zap) {
    let ent = zap.targets[0];
    let r = k::LEVEL2_AREAZAP_CHAIN_RANGE;
    let o = w.ents[ent].origin;
    for enemy in w.entities_in_box(&[o[0] - r, o[1] - r, o[2] - r], &[o[0] + r, o[1] + r, o[2] + r]) {
        if enemy == ent || (enemy < MAX_CLIENTS && w.ents[enemy].client && w.clients[enemy].noclip) {
            continue;
        }
        let d = distance(&o, &w.ents[enemy].origin);
        let human = (enemy < MAX_CLIENTS && w.ents[enemy].client && w.clients[enemy].ps.stat(Stat::Team) == Team::Humans as i32)
            || (w.ents[enemy].etype == EType::Buildable && w.ents[enemy].buildable.is_some_and(|b| b.def().team == Team::Humans));
        if human && w.ents[enemy].health > 0 && d <= r {
            let eo = w.ents[enemy].origin;
            let tr = w.trace(&o, None, None, &eo, ent as i32, crate::gen::enums::contents::SOLID as i32);
            if tr.entity_num == ENTITYNUM_NONE {
                zap.targets.push(enemy);
                zap.distances.push(d);
                if zap.targets.len() >= k::LEVEL2_AREAZAP_MAX_TARGETS as usize {
                    return;
                }
            }
        }
    }
}

/// `G_CreateNewZap`.
fn create_new_zap(w: &mut World, creator: usize, target: usize, forward: Vec3) {
    if w.zaps.len() >= MAX_CLIENTS {
        return;
    }
    let mut zap = super::world::Zap { creator, targets: vec![target], distances: vec![0.0], time_to_live: k::LEVEL2_AREAZAP_TIME };
    if w.ents[target].health > 0 {
        let to = w.ents[target].origin;
        combat::damage(w, target, Some(creator), Some(creator), Some(forward), Some(to), k::LEVEL2_AREAZAP_DMG, DAMAGE_NO_KNOCKBACK | DAMAGE_NO_LOCDAMAGE, Mod::Level2Zap);
        find_zap_chain_targets(w, &mut zap);
        for i in 1..zap.targets.len() {
            let falloff = (zap.distances[i] as f64 / k::LEVEL2_AREAZAP_CHAIN_RANGE as f64).powf(k::LEVEL2_AREAZAP_CHAIN_FALLOFF as f64);
            let dmg = (k::LEVEL2_AREAZAP_DMG as f64 * (1.0 - falloff) + 1.0) as i32;
            let to = w.ents[target].origin;
            let tg = zap.targets[i];
            combat::damage(w, tg, Some(target), Some(creator), Some(forward), Some(to), dmg, DAMAGE_NO_KNOCKBACK | DAMAGE_NO_LOCDAMAGE, Mod::Level2Zap);
        }
    }
    w.zaps.push(zap);
}

/// `areaZapFire` (advanced marauder).
pub fn area_zap_fire(w: &mut World, ent: usize) {
    let a = aim(w, ent);
    let (_, target) = wide_trace(w, ent, &a, k::LEVEL2_AREAZAP_RANGE, k::LEVEL2_AREAZAP_WIDTH, k::LEVEL2_AREAZAP_WIDTH);
    let Some(t) = target else { return };
    let human = (t < MAX_CLIENTS && w.ents[t].client && w.clients[t].ps.stat(Stat::Team) == Team::Humans as i32) || (w.ents[t].etype == EType::Buildable && w.ents[t].buildable.is_some_and(|b| b.def().team == Team::Humans));
    if human {
        create_new_zap(w, ent, t, a.forward);
    }
}

/// `G_UpdateZaps` system: zap beams live LEVEL2_AREAZAP_TIME and lose
/// targets that disappear.
pub fn update_zaps(w: &mut World) {
    if !super::team::warmup_over(w) {
        return;
    }
    let msec = w.level.msec;
    let ents = &w.ents;
    w.zaps.retain_mut(|z| {
        z.time_to_live -= msec;
        if z.time_to_live <= 0 || !ents[z.targets[0]].inuse {
            return false;
        }
        let mut j = 1;
        while j < z.targets.len() {
            if !ents[z.targets[j]].inuse {
                z.targets.swap_remove(j);
                z.distances.swap_remove(j);
            } else {
                j += 1;
            }
        }
        true
    });
}

/// `G_ClearPlayerZapEffects` (G_LeaveTeam, TeleportPlayer).
pub fn clear_player_zap_effects(w: &mut World, player: usize) {
    w.zaps.retain_mut(|z| {
        if z.creator == player || z.targets[0] == player {
            return false;
        }
        let mut j = 1;
        while j < z.targets.len() {
            if z.targets[j] == player {
                z.targets.swap_remove(j);
                z.distances.swap_remove(j);
            } else {
                j += 1;
            }
        }
        true
    });
}

/// `CheckPounceAttack`.
pub fn check_pounce_attack(w: &mut World, ent: usize) -> bool {
    if w.clients[ent].pmext.pounce_payload <= 0 {
        return false;
    }
    let payload = w.clients[ent].pmext.pounce_payload;
    if w.clients[ent].ps.pm_flags & k::PMF_CHARGE == 0 {
        w.clients[ent].pmext.pounce_payload = 0;
    }
    let a = aim(w, ent);
    let lvl3 = w.clients[ent].ps.weapon == WeaponId::Level3 as i32;
    let range = if lvl3 { k::LEVEL3_POUNCE_RANGE } else { k::LEVEL3_POUNCE_UPG_RANGE };
    let (tr, target) = wide_trace(w, ent, &a, range, k::LEVEL3_POUNCE_WIDTH, k::LEVEL3_POUNCE_WIDTH);
    let Some(t) = target else { return false };
    if !w.ents[t].takedamage {
        return false;
    }
    blood(w, ent, t, tr.endpos);
    let time_max = if lvl3 { k::LEVEL3_POUNCE_TIME } else { k::LEVEL3_POUNCE_TIME_UPG };
    let dmg = payload * k::LEVEL3_POUNCE_DMG / time_max;
    w.clients[ent].pmext.pounce_payload = 0;
    combat::damage(w, t, Some(ent), Some(ent), Some(a.forward), Some(tr.endpos), dmg, DAMAGE_NO_LOCDAMAGE, Mod::Level3Pounce);
    true
}

/// `G_ChargeAttack` (tyrant trample), from ClientImpacts.
pub fn charge_attack(w: &mut World, ent: usize, victim: usize) {
    let ps = &w.clients[ent].ps;
    if ps.stat(Stat::Misc) <= 0 || ps.stat(Stat::State) & k::SS_CHARGING == 0 || ps.weapon_time != 0 {
        return;
    }
    let mut forward = sub(&w.ents[victim].origin, &w.ents[ent].origin);
    normalize(&mut forward);
    if !w.ents[victim].takedamage {
        return;
    }
    if !(victim < MAX_CLIENTS && w.ents[victim].client) {
        let c = &mut w.clients[ent];
        if c.trample_hit.contains(&(victim as i32)) {
            return;
        }
        c.trample_hit[c.trample_pos % 20] = victim as i32;
        c.trample_pos += 1;
    }
    let dmg = k::LEVEL4_TRAMPLE_DMG * w.clients[ent].ps.stat(Stat::Misc) / k::LEVEL4_TRAMPLE_DURATION;
    let o = w.ents[victim].origin;
    combat::damage(w, victim, Some(ent), Some(ent), Some(forward), Some(o), dmg, DAMAGE_NO_LOCDAMAGE, Mod::Level4Trample);
    w.clients[ent].ps.weapon_time += k::LEVEL4_TRAMPLE_REPEAT;
}

/// `G_CrushAttack` (tyrant landing on someone).
pub fn crush_attack(w: &mut World, ent: usize, victim: usize) {
    let ps = &w.clients[ent].ps;
    if !w.ents[victim].takedamage || ps.origin[2] + w.ents[ent].mins[2] < w.ents[victim].origin[2] + w.ents[victim].maxs[2] {
        return;
    }
    if victim < MAX_CLIENTS && w.ents[victim].client && w.clients[victim].ps.ground_entity_num == ENTITYNUM_NONE {
        return;
    }
    let jump = bg::class(ps.stat(Stat::Class)).jump_magnitude;
    let mut dmg = ((w.clients[ent].pmext.fall_velocity + jump) * -k::LEVEL4_CRUSH_DAMAGE_PER_V) as i32;
    if dmg < 0 {
        dmg = 0;
    }
    if victim < MAX_CLIENTS && w.ents[victim].client && w.clients[ent].last_crush_time + k::LEVEL4_CRUSH_REPEAT < w.level.time {
        w.clients[ent].last_crush_time = w.level.time;
        dmg += k::LEVEL4_CRUSH_DAMAGE;
    }
    if dmg < 1 {
        return;
    }
    let dir = sub(&w.ents[victim].origin, &w.clients[ent].ps.origin);
    let o = w.ents[victim].origin;
    combat::damage(w, victim, Some(ent), Some(ent), Some(dir), Some(o), dmg, DAMAGE_NO_LOCDAMAGE, Mod::Level4Crush);
}

pub fn weapon_mode_name(m: i32) -> &'static str {
    match m {
        x if x == WeaponMode::Primary as i32 => "primary",
        x if x == WeaponMode::Secondary as i32 => "secondary",
        _ => "tertiary",
    }
}
