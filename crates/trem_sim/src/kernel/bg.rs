//! Kernel `bg` — the logic half of game/bg_misc.c. The data half (class,
//! weapon, upgrade and buildable tables) is generated from the sheets; these
//! helpers only query it.

use super::qmath::*;
use super::types::*;
use crate::gen::buildables::{BuildableDef, BuildableId, BUILDABLES};
use crate::gen::classes::{ClassDef, ClassId, CLASSES};
use crate::gen::enums::{stage, PmType, Stat, Team, Traj, WeaponState};
use crate::gen::upgrades::{UpgradeDef, UpgradeId, UPGRADES};
use crate::gen::weapons::{WeaponDef, WeaponId};

/// `BG_Class`: unknown numbers resolve to the spectator row (nullClass is all
/// zeros in the original; spectator has the same movement-relevant zeros).
#[inline]
pub fn class(num: i32) -> &'static ClassDef {
    ClassId::from_num(num as u8).map(|c| c.def()).unwrap_or(&CLASSES[0])
}
#[inline]
pub fn class_id(num: i32) -> ClassId {
    ClassId::from_num(num as u8).unwrap_or(ClassId::Spectator)
}

static NULL_WEAPON: WeaponDef = WeaponDef {
    id: "none",
    num: 0,
    team: Team::None,
    price: 0,
    stages: 0,
    slots: 0,
    max_ammo: 0,
    max_clips: 0,
    infinite_ammo: false,
    uses_energy: false,
    repeat_rate1: 0,
    repeat_rate2: 0,
    repeat_rate3: 0,
    reload_time: 0,
    knockback_scale: 0.0,
    has_alt_mode: false,
    has_third_mode: false,
    can_zoom: false,
    zoom_fov: 0.0,
    purchasable: false,
    long_ranged: false,
};

/// `BG_Weapon`: WP_NONE and out-of-range map to the zeroed nullWeapon.
#[inline]
pub fn weapon(num: i32) -> &'static WeaponDef {
    WeaponId::from_num(num as u8).filter(|_| num > 0 && num < 256).map(|w| w.def()).unwrap_or(&NULL_WEAPON)
}
#[inline]
pub fn upgrade(num: i32) -> Option<&'static UpgradeDef> {
    UpgradeId::from_num(num as u8).filter(|_| num > 0 && num < 256).map(|u| u.def())
}
#[inline]
pub fn buildable(num: i32) -> Option<&'static BuildableDef> {
    BuildableId::from_num(num as u8).filter(|_| num > 0 && num < 256).map(|b| b.def())
}

pub fn class_has_ability(class_num: i32, ability: u32) -> bool {
    class(class_num).abilities & ability != 0
}

/// `BG_ClassBoundingBox` -> (mins, maxs, crouch maxs, dead mins, dead maxs).
pub fn class_bbox(class_num: i32) -> (Vec3, Vec3, Vec3, Vec3, Vec3) {
    let c = class(class_num);
    (c.mins, c.maxs, c.crouch_maxs, c.dead_mins, c.dead_maxs)
}

pub fn inventory_contains_weapon(weapon: i32, stats: &[i32]) -> bool {
    if stats[Stat::Team as usize] == Team::Humans as i32 && weapon == WeaponId::Blaster as i32 {
        return true;
    }
    stats[Stat::Weapon as usize] == weapon
}
pub fn inventory_contains_upgrade(item: i32, stats: &[i32]) -> bool {
    stats[Stat::Items as usize] & (1 << item) != 0
}
pub fn add_upgrade(item: i32, stats: &mut [i32]) {
    stats[Stat::Items as usize] |= 1 << item;
}
pub fn remove_upgrade(item: i32, stats: &mut [i32]) {
    stats[Stat::Items as usize] &= !(1 << item);
}
pub fn activate_upgrade(item: i32, stats: &mut [i32]) {
    stats[Stat::Activeitems as usize] |= 1 << item;
}
pub fn deactivate_upgrade(item: i32, stats: &mut [i32]) {
    stats[Stat::Activeitems as usize] &= !(1 << item);
}
pub fn upgrade_is_active(item: i32, stats: &[i32]) -> bool {
    stats[Stat::Activeitems as usize] & (1 << item) != 0
}

/// `BG_SlotsForInventory`.
pub fn slots_for_inventory(stats: &[i32]) -> u32 {
    let mut slots = weapon(stats[Stat::Weapon as usize]).slots;
    if stats[Stat::Team as usize] == Team::Humans as i32 {
        slots |= WeaponId::Blaster.def().slots;
    }
    for u in UPGRADES.iter() {
        if inventory_contains_upgrade(u.num as i32, stats) {
            slots |= u.slots;
        }
    }
    slots
}

/// `BG_WeaponIsFull`.
pub fn weapon_is_full(w: i32, stats: &[i32], ammo: i32, clips: i32) -> bool {
    let mut max_ammo = weapon(w).max_ammo;
    let max_clips = weapon(w).max_clips;
    if inventory_contains_upgrade(UpgradeId::Battpack as i32, stats) {
        max_ammo = (max_ammo as f32 * k::BATTPACK_MODIFIER) as i32;
    }
    max_ammo == ammo && max_clips == clips
}

/// `BG_PlayerCanChangeWeapon`.
pub fn player_can_change_weapon(ps: &PlayerState) -> bool {
    if ps.weapon == WeaponId::Lcannon as i32 && ps.stat(Stat::Misc) > k::LCANNON_CHARGE_TIME_MIN {
        return false;
    }
    ps.weapon_time <= 0 || ps.weaponstate != WeaponState::Firing as i32
}

/// `BG_GetClientNormal`.
pub fn client_normal(ps: &PlayerState) -> Vec3 {
    if ps.stat(Stat::State) & k::SS_WALLCLIMBING != 0 {
        if ps.e_flags & k::EF_WALLCLIMBCEILING != 0 {
            [0.0, 0.0, -1.0]
        } else {
            ps.grapple_point
        }
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// `BG_GetClientViewOrigin`.
pub fn client_view_origin(ps: &PlayerState) -> Vec3 {
    let n = client_normal(ps);
    ma(&ps.origin, ps.viewheight as f32, &n)
}

/// `BG_RotateAxis`.
pub fn rotate_axis(surf_normal: &Vec3, in_axis: &[Vec3; 3], inverse: bool, ceiling: bool) -> Option<[Vec3; 3]> {
    let ref_normal: Vec3 = [0.0, 0.0, 1.0];
    let ceiling_normal: Vec3 = [0.0, 0.0, -1.0];
    let (local_normal, x_normal);
    if ceiling {
        local_normal = ceiling_normal;
        x_normal = *surf_normal;
    } else {
        local_normal = *surf_normal;
        let mut x = cross(&local_normal, &ref_normal);
        normalize(&mut x);
        x_normal = x;
    }
    if length(&x_normal) != 0.0 {
        let mut rot_angle = rad2deg((dot(&local_normal, &ref_normal) as f64).acos()) as f32;
        if inverse {
            rot_angle = -rot_angle;
        }
        let _ = angle_normalize180(rot_angle); // result discarded in the original
        Some([
            rotate_point_around_vector(&x_normal, &in_axis[0], -rot_angle),
            rotate_point_around_vector(&x_normal, &in_axis[1], -rot_angle),
            rotate_point_around_vector(&x_normal, &in_axis[2], -rot_angle),
        ])
    } else {
        None
    }
}

/// `BG_GetValueOfPlayer`.
pub fn value_of_player(ps: &PlayerState) -> i32 {
    let mut worth = class(ps.stat(Stat::Class)).value;
    if ps.stat(Stat::Team) == Team::Humans as i32 {
        for u in UPGRADES.iter() {
            if inventory_contains_upgrade(u.num as i32, &ps.stats) {
                worth += u.price;
            }
        }
        if ps.stat(Stat::Weapon) != 0 {
            worth += weapon(ps.stat(Stat::Weapon)).price;
        }
    }
    worth
}

pub fn class_allowed_in_stage(c: ClassId, stage_num: i32) -> bool {
    c.def().stages & (1 << stage_num) != 0
}
pub fn weapon_allowed_in_stage(w: WeaponId, stage_num: i32) -> bool {
    w.def().stages & (1 << stage_num) != 0
}
pub fn upgrade_allowed_in_stage(u: UpgradeId, stage_num: i32) -> bool {
    u.def().stages & (1 << stage_num) != 0
}
pub fn buildable_allowed_in_stage(b: BuildableId, stage_num: i32) -> bool {
    b.def().stages & (1 << stage_num) != 0
}
pub const ALL_STAGES: u32 = stage::S1 | stage::S2 | stage::S3;

/// `BG_ClassCanEvolveFromTo`: total credit cost (credits are evos x
/// ALIEN_CREDITS_PER_KILL) along the cheapest evolve path, or -1.
pub fn class_can_evolve_from_to(from: ClassId, to: ClassId, credits: i32, stage_num: i32, cost: i32) -> i32 {
    if credits < cost || from == ClassId::Spectator || to == ClassId::Spectator || from == to {
        return -1;
    }
    let mut best = credits + 1;
    for &thru in from.def().evolves_to {
        if !class_allowed_in_stage(thru, stage_num) || !class_is_allowed(thru) {
            continue;
        }
        let evolve_cost = thru.def().cost * k::ALIEN_CREDITS_PER_KILL;
        let value = if thru == to { cost + evolve_cost } else { class_can_evolve_from_to(thru, to, credits, stage_num, cost + evolve_cost) };
        if value >= 0 && value < best {
            best = value;
        }
    }
    if best <= credits {
        best
    } else {
        -1
    }
}

/// `BG_AlienCanEvolve`.
pub fn alien_can_evolve(c: ClassId, credits: i32, stage_num: i32) -> bool {
    c.def().evolves_to.iter().any(|&t| class_allowed_in_stage(t, stage_num) && class_is_allowed(t) && credits >= t.def().cost * k::ALIEN_CREDITS_PER_KILL)
}

/// `BG_EvaluateTrajectory`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trajectory {
    pub tr_type: i32,
    pub tr_time: i32,
    pub tr_duration: i32,
    pub tr_base: Vec3,
    pub tr_delta: Vec3,
}

pub fn evaluate_trajectory(tr: &Trajectory, mut at_time: i32) -> Vec3 {
    let t = Traj::from_raw(tr.tr_type as u8).unwrap_or(Traj::Stationary);
    match t {
        Traj::Stationary | Traj::Interpolate => tr.tr_base,
        Traj::Linear => {
            let dt = ((at_time - tr.tr_time) as f64 * 0.001) as f32;
            ma(&tr.tr_base, dt, &tr.tr_delta)
        }
        Traj::Sine => {
            let dt = (at_time - tr.tr_time) as f32 / tr.tr_duration as f32;
            let phase = ((dt as f64 * M_PI * 2.0).sin()) as f32;
            ma(&tr.tr_base, phase, &tr.tr_delta)
        }
        Traj::LinearStop => {
            if at_time > tr.tr_time + tr.tr_duration {
                at_time = tr.tr_time + tr.tr_duration;
            }
            let mut dt = ((at_time - tr.tr_time) as f64 * 0.001) as f32;
            if dt < 0.0 {
                dt = 0.0;
            }
            ma(&tr.tr_base, dt, &tr.tr_delta)
        }
        Traj::Gravity => {
            let dt = ((at_time - tr.tr_time) as f64 * 0.001) as f32;
            let mut r = ma(&tr.tr_base, dt, &tr.tr_delta);
            r[2] = (r[2] as f64 - 0.5 * k::DEFAULT_GRAVITY as f64 * dt as f64 * dt as f64) as f32;
            r
        }
        Traj::Buoyancy => {
            let dt = ((at_time - tr.tr_time) as f64 * 0.001) as f32;
            let mut r = ma(&tr.tr_base, dt, &tr.tr_delta);
            r[2] = (r[2] as f64 + 0.5 * k::DEFAULT_GRAVITY as f64 * dt as f64 * dt as f64) as f32;
            r
        }
    }
}

pub fn evaluate_trajectory_delta(tr: &Trajectory, at_time: i32) -> Vec3 {
    let t = Traj::from_raw(tr.tr_type as u8).unwrap_or(Traj::Stationary);
    match t {
        Traj::Stationary | Traj::Interpolate => [0.0; 3],
        Traj::Linear => tr.tr_delta,
        Traj::Sine => {
            let dt = (at_time - tr.tr_time) as f32 / tr.tr_duration as f32;
            let mut phase = ((dt as f64 * M_PI * 2.0).cos()) as f32;
            phase = (phase as f64 * 0.5) as f32;
            scale(&tr.tr_delta, phase)
        }
        Traj::LinearStop => {
            if at_time > tr.tr_time + tr.tr_duration {
                [0.0; 3]
            } else {
                tr.tr_delta
            }
        }
        Traj::Gravity => {
            let dt = ((at_time - tr.tr_time) as f64 * 0.001) as f32;
            let mut r = tr.tr_delta;
            r[2] -= k::DEFAULT_GRAVITY as f32 * dt;
            r
        }
        Traj::Buoyancy => {
            let dt = ((at_time - tr.tr_time) as f64 * 0.001) as f32;
            let mut r = tr.tr_delta;
            r[2] += k::DEFAULT_GRAVITY as f32 * dt;
            r
        }
    }
}

/// `BG_PositionBuildableRelativeToPlayer` -> (origin, angles, trace).
pub fn position_buildable_relative_to_player<W: PmWorld>(ps: &PlayerState, mins: &Vec3, maxs: &Vec3, w: &W) -> (Vec3, Vec3, super::cm::Trace) {
    let player_normal = client_normal(ps);
    let angles = ps.viewangles;
    let build_dist = class(ps.stat(Stat::Class)).build_dist;
    let (f, _, _) = angle_vectors(&angles);
    let mut forward = project_point_on_plane(&f, &player_normal);
    normalize(&mut forward);
    let mut entity_origin = ma(&ps.origin, build_dist, &forward);
    let mut target_origin = entity_origin;
    entity_origin = ma(&entity_origin, 32.0, &player_normal);
    target_origin = ma(&target_origin, -128.0, &player_normal);
    let tr = w.trace(&entity_origin, Some(mins), Some(maxs), &target_origin, ps.client_num, k::MASK_DEADSOLID);
    let out = ma(&tr.endpos, 0.1, &player_normal);
    (out, vectoangles(&forward), tr)
}

pub fn pm_paralyzed(t: i32) -> bool {
    t == PmType::Dead as i32 || t == PmType::Freeze as i32 || t == PmType::Intermission as i32
}
pub fn pm_live(t: i32) -> bool {
    t == PmType::Normal as i32 || t == PmType::Jetpack as i32 || t == PmType::Grabbed as i32
}

pub fn buildables() -> &'static [BuildableDef] {
    &BUILDABLES
}

/// bg_misc.c `eventnames[]` (`BG_EventName`, cg_debugEvents).
pub const EVENT_NAMES: [&str; 74] = [
    "EV_NONE", "EV_FOOTSTEP", "EV_FOOTSTEP_METAL", "EV_FOOTSTEP_SQUELCH", "EV_FOOTSPLASH", "EV_FOOTWADE", "EV_SWIM", "EV_STEP_4", "EV_STEP_8", "EV_STEP_12", "EV_STEP_16", "EV_STEPDN_4", "EV_STEPDN_8", "EV_STEPDN_12", "EV_STEPDN_16", "EV_FALL_SHORT", "EV_FALL_MEDIUM", "EV_FALL_FAR", "EV_FALLING", "EV_JUMP", "EV_WATER_TOUCH", "EV_WATER_LEAVE", "EV_WATER_UNDER", "EV_WATER_CLEAR", "EV_NOAMMO", "EV_CHANGE_WEAPON", "EV_FIRE_WEAPON", "EV_FIRE_WEAPON2", "EV_FIRE_WEAPON3", "EV_PLAYER_RESPAWN", "EV_PLAYER_TELEPORT_IN", "EV_PLAYER_TELEPORT_OUT", "EV_GRENADE_BOUNCE", "EV_GENERAL_SOUND", "EV_GLOBAL_SOUND", "EV_BULLET_HIT_FLESH", "EV_BULLET_HIT_WALL", "EV_SHOTGUN", "EV_MASS_DRIVER", "EV_MISSILE_HIT", "EV_MISSILE_MISS", "EV_MISSILE_MISS_METAL", "EV_TESLATRAIL", "EV_BULLET", "EV_LEV1_GRAB", "EV_LEV4_TRAMPLE_PREPARE", "EV_LEV4_TRAMPLE_START", "EV_PAIN", "EV_DEATH1", "EV_DEATH2", "EV_DEATH3", "EV_OBITUARY", "EV_GIB_PLAYER", "EV_BUILD_CONSTRUCT", "EV_BUILD_DESTROY", "EV_BUILD_DELAY", "EV_BUILD_REPAIR", "EV_BUILD_REPAIRED", "EV_HUMAN_BUILDABLE_EXPLOSION", "EV_ALIEN_BUILDABLE_EXPLOSION", "EV_ALIEN_ACIDTUBE", "EV_MEDKIT_USED", "EV_ALIEN_EVOLVE", "EV_ALIEN_EVOLVE_FAILED", "EV_DEBUG_LINE", "EV_STOPLOOPINGSOUND", "EV_TAUNT", "EV_OVERMIND_ATTACK", "EV_OVERMIND_DYING", "EV_OVERMIND_SPAWNS", "EV_DCC_ATTACK", "EV_MGTURRET_SPINUP", "EV_RPTUSE_SOUND", "EV_LEV2_ZAP",
];

/// `BG_EventName`.
pub fn event_name(num: i32) -> &'static str {
    usize::try_from(num).ok().and_then(|n| EVENT_NAMES.get(n)).copied().unwrap_or("UNKNOWN")
}

/// bg_misc.c `BG_TeamName`.
pub fn team_name(team: i32) -> &'static str {
    match team {
        0 => "spectator",
        1 => "alien",
        2 => "human",
        _ => "<team>",
    }
}

/// bg_misc.c `gameElements_t`: what the map disables (worldspawn's
/// disabledEquipment / disabledClasses / disabledBuildables, which
/// SP_worldspawn copies into g_disabled*).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameElements {
    pub buildables: Vec<BuildableId>,
    pub classes: Vec<ClassId>,
    pub weapons: Vec<WeaponId>,
    pub upgrades: Vec<UpgradeId>,
}

thread_local! {
    /// `bg_disabledGameElements` (a static of bg_misc.c: one game per thread here).
    static DISABLED: std::cell::RefCell<GameElements> = std::cell::RefCell::new(GameElements::default());
}

/// The entries of a comma separated list, leading spaces stripped (the
/// `BG_ParseCSV*List` loop).
fn csv_entries(string: &str) -> Vec<String> {
    if string.is_empty() {
        return Vec::new();
    }
    // Q_strncpyz( buffer, string, MAX_STRING_CHARS )
    let b: String = string.chars().take(1023).collect();
    b.split(',').map(|q| q.trim_start_matches(' ').to_string()).collect()
}

/// `BG_ParseCSVEquipmentList`: weapons and upgrades by name (unknown names
/// warned about), at most `WP_NUM_WEAPONS - 1` / `UP_NUM_UPGRADES - 1`.
pub fn parse_csv_equipment_list(string: &str, warnings: &mut Vec<String>) -> (Vec<WeaponId>, Vec<UpgradeId>) {
    let (wsize, usize_) = (WeaponId::ALL.len() + 1, UpgradeId::ALL.len() + 1);
    let (mut weapons, mut upgrades) = (Vec::new(), Vec::new());
    for q in csv_entries(string) {
        let l = q.to_ascii_lowercase();
        if let Some(w) = WeaponId::from_id(&l) {
            weapons.push(w);
        } else if let Some(u) = UpgradeId::from_id(&l) {
            upgrades.push(u);
        } else {
            warnings.push(format!("^3WARNING: unknown equipment {q}"));
        }
        if weapons.len() == wsize - 1 || upgrades.len() == usize_ - 1 {
            break;
        }
    }
    (weapons, upgrades)
}

/// `BG_ParseCSVClassList`.
pub fn parse_csv_class_list(string: &str, warnings: &mut Vec<String>) -> Vec<ClassId> {
    let size = ClassId::ALL.len();
    let mut out = Vec::new();
    for q in csv_entries(string) {
        if out.len() >= size - 1 {
            break;
        }
        match ClassId::from_id(&q.to_ascii_lowercase()).filter(|&c| c != ClassId::Spectator) {
            Some(c) => out.push(c),
            None => warnings.push(format!("^3WARNING: unknown class {q}")),
        }
    }
    out
}

/// `BG_ParseCSVBuildableList`.
pub fn parse_csv_buildable_list(string: &str, warnings: &mut Vec<String>) -> Vec<BuildableId> {
    let size = BuildableId::ALL.len() + 1;
    let mut out = Vec::new();
    for q in csv_entries(string) {
        if out.len() >= size - 1 {
            break;
        }
        match BuildableId::from_id(&q.to_ascii_lowercase()) {
            Some(b) => out.push(b),
            None => warnings.push(format!("^3WARNING: unknown buildable {q}")),
        }
    }
    out
}

/// `BG_InitAllowedGameElements` from g_disabledEquipment,
/// g_disabledClasses and g_disabledBuildables; returns the warnings.
pub fn init_allowed_game_elements(equipment: &str, classes: &str, buildables: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let (weapons, upgrades) = parse_csv_equipment_list(equipment, &mut warnings);
    let e = GameElements { weapons, upgrades, classes: parse_csv_class_list(classes, &mut warnings), buildables: parse_csv_buildable_list(buildables, &mut warnings) };
    DISABLED.with(|d| *d.borrow_mut() = e);
    warnings
}

/// `BG_WeaponIsAllowed`.
pub fn weapon_is_allowed(w: WeaponId) -> bool {
    DISABLED.with(|d| !d.borrow().weapons.contains(&w))
}

/// `BG_UpgradeIsAllowed`.
pub fn upgrade_is_allowed(u: UpgradeId) -> bool {
    DISABLED.with(|d| !d.borrow().upgrades.contains(&u))
}

/// `BG_ClassIsAllowed`.
pub fn class_is_allowed(c: ClassId) -> bool {
    DISABLED.with(|d| !d.borrow().classes.contains(&c))
}

/// `BG_BuildableIsAllowed`.
pub fn buildable_is_allowed(b: BuildableId) -> bool {
    DISABLED.with(|d| !d.borrow().buildables.contains(&b))
}
