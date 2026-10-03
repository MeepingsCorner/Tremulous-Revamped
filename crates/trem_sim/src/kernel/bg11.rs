//! Kernel `bg11` — the Tremulous 1.1.0 half of game/bg_misc.c the protocol 69
//! client needs (dec041): the `BG_Find*ForClass` / `BG_Find*ForWeapon`
//! lookups over the classes11 / weapons11 / upgrades11 / buildables11 sheets
//! (bg_misc.c's tables with data-1.1.0.pk3's overrides, dumped by
//! re/harness/v11_ref), with the C's fall-through values for unknown numbers,
//! and the 1.1 inventory and ammo packing (`BG_UnpackAmmoArray`,
//! `BG_PackAmmoArray`, `BG_InventoryContainsWeapon`, ...) over the 1.1
//! playerState's `stats[]`, `ammo[]` and `powerups[]`.
//!
//! Reference source: re/source11/src/game/bg_misc.c.

use crate::gen::buildables11::{Buildable11Id, Buildables11Def};
use crate::gen::classes11::{Class11Id, Classes11Def};
use crate::gen::enums::Stat11;
use crate::gen::upgrades11::Upgrade11Id;
use crate::gen::weapons11::{Weapon11Id, Weapons11Def};

use super::qmath::Vec3;

const ST_ITEMS: usize = Stat11::Items as usize;
const ST_SLOTS: usize = Stat11::Slots as usize;
const ST_ACTIVEITEMS: usize = Stat11::Activeitems as usize;
const ST_WEAPONS: usize = Stat11::Weapons as usize;
const ST_WEAPONS2: usize = Stat11::Weapons2 as usize;

/// The `bg_classList` entry of `pclass` (None: the lookups fall through).
pub fn class(pclass: i32) -> Option<&'static Classes11Def> {
    u8::try_from(pclass).ok().and_then(Class11Id::from_num).map(|c| c.def())
}
/// `BG_FindSpeedForClass`.
pub fn speed(pclass: i32) -> f32 {
    class(pclass).map(|c| c.speed).unwrap_or(1.0)
}
/// `BG_FindAccelerationForClass`.
pub fn acceleration(pclass: i32) -> f32 {
    class(pclass).map(|c| c.acceleration).unwrap_or(10.0)
}
/// `BG_FindAirAccelerationForClass`.
pub fn air_acceleration(pclass: i32) -> f32 {
    class(pclass).map(|c| c.air_acceleration).unwrap_or(1.0)
}
/// `BG_FindFrictionForClass`.
pub fn friction(pclass: i32) -> f32 {
    class(pclass).map(|c| c.friction).unwrap_or(6.0)
}
/// `BG_FindStopSpeedForClass`.
pub fn stop_speed(pclass: i32) -> f32 {
    class(pclass).map(|c| c.stop_speed).unwrap_or(100.0)
}
/// `BG_FindJumpMagnitudeForClass`.
pub fn jump_magnitude(pclass: i32) -> f32 {
    class(pclass).map(|c| c.jump_magnitude).unwrap_or(270.0)
}
/// `BG_FindBobCycleForClass`.
pub fn bob_cycle(pclass: i32) -> f32 {
    class(pclass).map(|c| c.bob_cycle).unwrap_or(1.0)
}
/// `BG_FindHealthForClass`.
pub fn health(pclass: i32) -> i32 {
    class(pclass).map(|c| c.health).unwrap_or(100)
}
/// `BG_ClassHasAbility`.
pub fn class_has_ability(pclass: i32, ability: u32) -> bool {
    class(pclass).is_some_and(|c| c.abilities & ability != 0)
}
/// `BG_FindBBoxForClass` (mins, maxs, crouch maxs, dead mins, dead maxs);
/// an unknown class gets `bg_classList[ 0 ]`'s.
pub fn class_bbox(pclass: i32) -> (Vec3, Vec3, Vec3, Vec3, Vec3) {
    let c = class(pclass).unwrap_or(Class11Id::Spectator.def());
    (c.mins, c.maxs, c.crouch_maxs, c.dead_mins, c.dead_maxs)
}
/// `BG_FindViewheightForClass` (viewheight, crouch viewheight).
pub fn class_viewheight(pclass: i32) -> (i32, i32) {
    let c = class(pclass).unwrap_or(Class11Id::Spectator.def());
    (c.viewheight, c.crouch_viewheight)
}

/// The `bg_weapons` entry of `weapon`.
pub fn weapon(weapon: i32) -> Option<&'static Weapons11Def> {
    u8::try_from(weapon).ok().and_then(Weapon11Id::from_num).map(|w| w.def())
}
/// `BG_FindAmmoForWeapon` (None: the C leaves its outputs untouched).
pub fn ammo_for_weapon(w: i32) -> Option<(i32, i32)> {
    weapon(w).map(|d| (d.max_ammo, d.max_clips))
}
/// `BG_FindInfinteAmmoForWeapon`.
pub fn infinite_ammo(w: i32) -> bool {
    weapon(w).is_some_and(|d| d.infinite_ammo)
}
/// `BG_FindUsesEnergyForWeapon`.
pub fn uses_energy(w: i32) -> bool {
    weapon(w).is_some_and(|d| d.uses_energy)
}
/// `BG_FindRepeatRate1ForWeapon`.
pub fn repeat_rate1(w: i32) -> i32 {
    weapon(w).map(|d| d.repeat_rate1).unwrap_or(1000)
}
/// `BG_FindRepeatRate2ForWeapon`.
pub fn repeat_rate2(w: i32) -> i32 {
    weapon(w).map(|d| d.repeat_rate2).unwrap_or(1000)
}
/// `BG_FindRepeatRate3ForWeapon`.
pub fn repeat_rate3(w: i32) -> i32 {
    weapon(w).map(|d| d.repeat_rate3).unwrap_or(1000)
}
/// `BG_FindReloadTimeForWeapon`.
pub fn reload_time(w: i32) -> i32 {
    weapon(w).map(|d| d.reload_time).unwrap_or(1000)
}
/// `BG_WeaponHasAltMode`.
pub fn has_alt_mode(w: i32) -> bool {
    weapon(w).is_some_and(|d| d.has_alt_mode)
}
/// `BG_WeaponHasThirdMode`.
pub fn has_third_mode(w: i32) -> bool {
    weapon(w).is_some_and(|d| d.has_third_mode)
}
/// `BG_FindSlotsForWeapon`.
pub fn weapon_slots(w: i32) -> i32 {
    weapon(w).map(|d| d.slots as i32).unwrap_or(0)
}
/// `BG_FindSlotsForUpgrade`.
pub fn upgrade_slots(u: i32) -> i32 {
    u8::try_from(u).ok().and_then(Upgrade11Id::from_num).map(|u| u.def().slots as i32).unwrap_or(0)
}
/// The `bg_buildableList` entry of `bclass`.
pub fn buildable(bclass: i32) -> Option<&'static Buildables11Def> {
    u8::try_from(bclass).ok().and_then(Buildable11Id::from_num).map(|b| b.def())
}

/// `BG_UnpackAmmoArray`: (ammo, clips) of `weapon` from `ps.ammo` (weapons
/// 0..15) and `ps.powerups` (16..31).
pub fn unpack_ammo_array(weapon: i32, ps_ammo: &[i32; 16], ps_ammo2: &[i32; 16]) -> (i32, i32) {
    let w = (weapon & 31) as usize;
    let v = if w <= 15 { ps_ammo[w] } else { ps_ammo2[w - 16] };
    (v & 0x0FFF, (v >> 12) & 0x0F)
}
/// `BG_PackAmmoArray`.
pub fn pack_ammo_array(weapon: i32, ps_ammo: &mut [i32; 16], ps_ammo2: &mut [i32; 16], ammo: i32, clips: i32) {
    let value = ammo | (clips << 12);
    if weapon <= 15 {
        ps_ammo[(weapon & 15) as usize] = value;
    } else {
        ps_ammo2[((weapon - 16) & 15) as usize] = value;
    }
}
fn weapon_list(stats: &[i32]) -> i32 {
    (stats[ST_WEAPONS] & 0x0000FFFF) | ((stats[ST_WEAPONS2] << 16) & 0xFFFF0000u32 as i32)
}
/// `BG_InventoryContainsWeapon`.
pub fn inventory_contains_weapon(weapon: i32, stats: &[i32]) -> bool {
    weapon_list(stats) & 1i32.wrapping_shl(weapon as u32) != 0
}
/// `BG_AddWeaponToInventory`.
pub fn add_weapon_to_inventory(weapon: i32, stats: &mut [i32]) {
    let list = weapon_list(stats) | 1i32.wrapping_shl(weapon as u32);
    stats[ST_WEAPONS] = list & 0x0000FFFF;
    stats[ST_WEAPONS2] = ((list as u32 & 0xFFFF0000) >> 16) as i32;
    stats[ST_SLOTS] |= weapon_slots(weapon);
}
/// `BG_RemoveWeaponFromInventory`.
pub fn remove_weapon_from_inventory(weapon: i32, stats: &mut [i32]) {
    let list = weapon_list(stats) & !1i32.wrapping_shl(weapon as u32);
    stats[ST_WEAPONS] = list & 0x0000FFFF;
    stats[ST_WEAPONS2] = ((list as u32 & 0xFFFF0000) >> 16) as i32;
    stats[ST_SLOTS] &= !weapon_slots(weapon);
}
/// `BG_AddUpgradeToInventory`.
pub fn add_upgrade_to_inventory(item: i32, stats: &mut [i32]) {
    stats[ST_ITEMS] |= 1i32.wrapping_shl(item as u32);
    stats[ST_SLOTS] |= upgrade_slots(item);
}
/// `BG_RemoveUpgradeFromInventory`.
pub fn remove_upgrade_from_inventory(item: i32, stats: &mut [i32]) {
    stats[ST_ITEMS] &= !1i32.wrapping_shl(item as u32);
    stats[ST_SLOTS] &= !upgrade_slots(item);
}
/// `BG_InventoryContainsUpgrade`.
pub fn inventory_contains_upgrade(item: i32, stats: &[i32]) -> bool {
    stats[ST_ITEMS] & 1i32.wrapping_shl(item as u32) != 0
}
/// `BG_ActivateUpgrade`.
pub fn activate_upgrade(item: i32, stats: &mut [i32]) {
    stats[ST_ACTIVEITEMS] |= 1i32.wrapping_shl(item as u32);
}
/// `BG_DeactivateUpgrade`.
pub fn deactivate_upgrade(item: i32, stats: &mut [i32]) {
    stats[ST_ACTIVEITEMS] &= !1i32.wrapping_shl(item as u32);
}
/// `BG_UpgradeIsActive`.
pub fn upgrade_is_active(item: i32, stats: &[i32]) -> bool {
    stats[ST_ACTIVEITEMS] & 1i32.wrapping_shl(item as u32) != 0
}

/// `BG_FindStagesForClass` / `ForWeapon` / `ForUpgrade` / `ForBuildable`:
/// whether a `stages` bitmask (1 << S1 | ...) allows `stage` (0-based).
pub fn allowed_in_stage(stages: u32, stage: i32) -> bool {
    (0..3).contains(&stage) && stages & (1 << stage) != 0
}

/// `BG_FindCostOfClass`.
pub fn cost_of_class(pclass: i32) -> i32 {
    class(pclass).map(|c| c.cost).unwrap_or(0)
}
/// `BG_FindValueOfClass`.
pub fn value_of_class(pclass: i32) -> i32 {
    class(pclass).map(|c| c.value).unwrap_or(0)
}

/// `BG_ClassCanEvolveFromTo`: the frag cost of evolving from `fclass` to
/// `tclass` with `credits` frags (`num` accumulated so far), or -1. The
/// sheet lists only the non-PCL_NONE children; a PCL_NONE child can never
/// match nor recurse to a result in the C either.
pub fn class_can_evolve_from_to(fclass: i32, tclass: i32, credits: i32, num: i32) -> i32 {
    let cost = cost_of_class(tclass);
    // base case
    if credits < cost {
        return -1;
    }
    if fclass == 0 || tclass == 0 {
        return -1;
    }
    let Some(from) = class(fclass) else { return -1 };
    if from.children.iter().any(|c| c.num() as i32 == tclass) {
        return num + cost;
    }
    for c in from.children {
        let cost = cost_of_class(c.num() as i32);
        let sub = class_can_evolve_from_to(c.num() as i32, tclass, credits - cost, num + cost);
        if sub >= 0 {
            return sub;
        }
    }
    -1
}

/// 1.1.0's q_math.c `VectorNormalize`: unlike ioquake3's (qmath.rs) it takes
/// the square root first and divides by it (`ilength = 1/length`), which
/// rounds differently.
pub fn vector_normalize(v: &mut Vec3) -> f32 {
    let mut length = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    length = (length as f64).sqrt() as f32;
    if length != 0.0 {
        let ilength = 1.0f32 / length;
        v[0] *= ilength;
        v[1] *= ilength;
        v[2] *= ilength;
    }
    length
}
/// 1.1.0's q_math.c `VectorNormalize2`.
pub fn vector_normalize2(v: &Vec3, out: &mut Vec3) -> f32 {
    let mut length = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    length = (length as f64).sqrt() as f32;
    if length != 0.0 {
        let ilength = 1.0f32 / length;
        *out = [v[0] * ilength, v[1] * ilength, v[2] * ilength];
    } else {
        *out = [0.0; 3];
    }
    length
}
/// 1.1.0's `BG_RotateAxis` (the legacy one with 1.1's `VectorNormalize`).
pub fn rotate_axis(surf_normal: &Vec3, in_axis: &[Vec3; 3], inverse: bool, ceiling: bool) -> Option<[Vec3; 3]> {
    use super::qmath::{cross, dot, length, rad2deg, rotate_point_around_vector};
    let ref_normal: Vec3 = [0.0, 0.0, 1.0];
    let ceiling_normal: Vec3 = [0.0, 0.0, -1.0];
    let (local_normal, x_normal);
    if ceiling {
        local_normal = ceiling_normal;
        x_normal = *surf_normal;
    } else {
        local_normal = *surf_normal;
        let mut x = cross(&local_normal, &ref_normal);
        vector_normalize(&mut x);
        x_normal = x;
    }
    // can't rotate with no rotation vector
    if length(&x_normal) != 0.0 {
        let mut rot_angle = rad2deg((dot(&local_normal, &ref_normal) as f64).acos()) as f32;
        if inverse {
            rot_angle = -rot_angle;
        }
        // AngleNormalize180( rotAngle ): result discarded in the original
        Some([
            rotate_point_around_vector(&x_normal, &in_axis[0], -rot_angle),
            rotate_point_around_vector(&x_normal, &in_axis[1], -rot_angle),
            rotate_point_around_vector(&x_normal, &in_axis[2], -rot_angle),
        ])
    } else {
        None
    }
}

/// 1.1.0's `BG_PositionBuildableRelativeToPlayer` for the legacy-side
/// playerState of a protocol 69 game (`v11::ps_from_11`): the 1.1 build
/// distance, `MASK_PLAYERSOLID` and 1.1's VectorNormalize -> (origin,
/// angles, trace).
pub fn position_buildable_relative_to_player<W: super::types::PmWorld>(ps: &super::types::PlayerState, mins: &Vec3, maxs: &Vec3, w: &W) -> (Vec3, Vec3, super::cm::Trace) {
    use super::qmath::{angle_vectors, ma, project_point_on_plane, vectoangles};
    // SS_WALLCLIMBING / SS_WALLCLIMBINGCEILING are legacy's SS_WALLCLIMBING /
    // EF_WALLCLIMBCEILING there
    let player_normal = super::bg::client_normal(ps);
    let build_dist = class(super::v11::class_to_11(ps.stats[crate::gen::enums::Stat::Class as usize])).map(|c| c.build_dist).unwrap_or(0.0);
    let (f, _, _) = angle_vectors(&ps.viewangles);
    let mut forward = project_point_on_plane(&f, &player_normal);
    vector_normalize(&mut forward);
    let mut entity_origin = ma(&ps.origin, build_dist, &forward);
    let mut target_origin = entity_origin;
    // so buildings can be placed facing slopes
    entity_origin = ma(&entity_origin, 32.0, &player_normal);
    // so buildings drop to floor
    target_origin = ma(&target_origin, -128.0, &player_normal);
    let tr = w.trace(&entity_origin, Some(mins), Some(maxs), &target_origin, ps.client_num, crate::gen::constants11::MASK_PLAYERSOLID);
    let out = ma(&tr.endpos, 0.1, &player_normal);
    (out, vectoangles(&forward), tr)
}
