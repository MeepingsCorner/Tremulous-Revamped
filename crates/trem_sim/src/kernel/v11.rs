//! Vanilla Tremulous 1.1.0 (protocol 69) adapter (dec040, dec041).
//!
//! The port's client and cgame speak the legacy game (protocol 71). Against a
//! 1.1.0 server the network layer (cl.rs / msg.rs / netchan.rs) decodes the
//! 1.1 wire format into the 1.1 shapes below and calls these functions to get
//! the legacy shapes the rest of the client consumes; outgoing traffic goes
//! the other way. Everything game-specific about 1.1 lives here: enum
//! numbering (weapons, upgrades, classes, buildables, events, means of death,
//! stats, persistant, entity types, eFlags, pm_flags), the playerState and
//! entityState encodings, the configstring layout, the server command texts,
//! the client command texts and usercmd buttons.
//!
//! The 1.1 numbering comes from the 1.1.0 headers through sheets: the
//! classes11 / weapons11 / upgrades11 / buildables11 tables (whose `legacy`
//! column names the legacy row of each 1.1 entry; the hovel and the advanced
//! construction kit are presentation-only legacy rows that no legacy stage
//! allows) and the Event11 / MeansOfDeath11 / EntityType11 / Stat11 / Pers11 /
//! Menu11 / PmType11 enum domains, all checked against the compiler's view of
//! the 1.1.0 headers (re/traces/v11/consts.tsv) by crates/trem_sim/tests/net_v11.rs.
//!
//! Prediction stays exact (kernel::pmove11 runs on the 1.1 playerState): the
//! words of the 1.1 playerState the legacy one has no room for travel in the
//! legacy `ps.misc[]` (unused by the legacy cgame), so `ps_to_11` inverts
//! `ps_from_11` (see `STASH_*`).
//!
//! Reference source: Tremulous 1.1.0 (re/source11, from jkent/tremulous-backport).
use super::msg::EntityState;
use super::qmath::Vec3;
use super::svgame::{self, cs};
use super::types::{PlayerState, UserCmd};
use super::world::GameEvent;
use crate::gen::buildables::BuildableId;
use crate::gen::buildables11::Buildable11Id;
use crate::gen::classes::ClassId;
use crate::gen::classes11::Class11Id;
use crate::gen::constants as k;
use crate::gen::constants11 as k11;
use crate::gen::enums::{Event, Event11, MeansOfDeath as Mod, MeansOfDeath11 as Mod11, Menu11, Pers, Pers11, PmType, PmType11, SpectatorState, Stat, Stat11, Team};
use crate::gen::upgrades::UpgradeId;
use crate::gen::upgrades11::Upgrade11Id;
use crate::gen::weapons::WeaponId;
use crate::gen::weapons11::Weapon11Id;

/// `PROTOCOL_VERSION` of Tremulous 1.1.0.
pub const PROTOCOL_VANILLA: i32 = 69;

/// 1.1.0's `playerState_t` (q_shared.h) as it comes off the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerState11 {
    pub command_time: i32,
    pub pm_type: i32,
    pub bob_cycle: i32,
    pub pm_flags: i32,
    pub pm_time: i32,
    pub origin: Vec3,
    pub velocity: Vec3,
    pub weapon_time: i32,
    pub gravity: i32,
    pub speed: i32,
    pub delta_angles: [i32; 3],
    pub ground_entity_num: i32,
    pub legs_timer: i32,
    pub legs_anim: i32,
    pub torso_timer: i32,
    pub torso_anim: i32,
    pub movement_dir: i32,
    pub grapple_point: Vec3,
    pub e_flags: i32,
    pub event_sequence: i32,
    pub events: [i32; 2],
    pub event_parms: [i32; 2],
    pub external_event: i32,
    pub external_event_parm: i32,
    pub external_event_time: i32,
    pub client_num: i32,
    pub weapon: i32,
    pub weaponstate: i32,
    pub viewangles: Vec3,
    pub viewheight: i32,
    pub damage_event: i32,
    pub damage_yaw: i32,
    pub damage_pitch: i32,
    pub damage_count: i32,
    pub stats: [i32; 16],
    pub persistant: [i32; 16],
    pub powerups: [i32; 16],
    pub ammo: [i32; 16],
    pub generic1: i32,
    pub loop_sound: i32,
    pub jumppad_ent: i32,
    pub ping: i32,
    pub pmove_framecount: i32,
    pub jumppad_frame: i32,
    pub entity_event_sequence: i32,
}

// =====================================================================
// numbering: 1.1 -> legacy and back
// =====================================================================

/// A 1.1 weapon number (`weapon_t`) as the legacy one (`WP_HBUILD2`, which
/// legacy lacks, becomes the presentation-only `ackit` row).
pub fn weapon_from_11(w: i32) -> i32 {
    u8::try_from(w).ok().and_then(Weapon11Id::from_num).map(|x| x.def().legacy.num() as i32).unwrap_or(0)
}
/// A legacy weapon number as the 1.1 one (0 when 1.1 has none).
pub fn weapon_to_11(w: i32) -> i32 {
    Weapon11Id::ALL.iter().find(|x| x.def().legacy.num() as i32 == w).map(|x| x.num() as i32).unwrap_or(0)
}
/// A 1.1 upgrade number (`upgrade_t`) as the legacy one.
pub fn upgrade_from_11(u: i32) -> i32 {
    u8::try_from(u).ok().and_then(Upgrade11Id::from_num).map(|x| x.def().legacy.num() as i32).unwrap_or(0)
}
/// A legacy upgrade number as the 1.1 one.
pub fn upgrade_to_11(u: i32) -> i32 {
    Upgrade11Id::ALL.iter().find(|x| x.def().legacy.num() as i32 == u).map(|x| x.num() as i32).unwrap_or(0)
}
/// A 1.1 class number (`pClass_t`) as the legacy one.
pub fn class_from_11(c: i32) -> i32 {
    u8::try_from(c).ok().and_then(Class11Id::from_num).map(|x| x.def().legacy.num() as i32).unwrap_or(0)
}
/// A legacy class number as the 1.1 one.
pub fn class_to_11(c: i32) -> i32 {
    Class11Id::ALL.iter().find(|x| x.def().legacy.num() as i32 == c).map(|x| x.num() as i32).unwrap_or(0)
}
/// A 1.1 buildable number (`buildable_t`) as the legacy one (`BA_A_HOVEL`
/// becomes the presentation-only `hovel` row).
pub fn buildable_from_11(b: i32) -> i32 {
    u8::try_from(b).ok().and_then(Buildable11Id::from_num).map(|x| x.def().legacy.num() as i32).unwrap_or(0)
}
/// A legacy buildable number as the 1.1 one.
pub fn buildable_to_11(b: i32) -> i32 {
    Buildable11Id::ALL.iter().find(|x| x.def().legacy.num() as i32 == b).map(|x| x.num() as i32).unwrap_or(0)
}
/// The 1.1 table entry of a legacy weapon / buildable / class / upgrade (the
/// 1.1 balance the HUD and menus show in protocol 69 games).
pub fn weapon11_of(w: WeaponId) -> Option<Weapon11Id> {
    Weapon11Id::ALL.into_iter().find(|x| x.def().legacy == w)
}
pub fn buildable11_of(b: BuildableId) -> Option<Buildable11Id> {
    Buildable11Id::ALL.into_iter().find(|x| x.def().legacy == b)
}
pub fn class11_of(c: ClassId) -> Option<Class11Id> {
    Class11Id::ALL.into_iter().find(|x| x.def().legacy == c)
}
pub fn upgrade11_of(u: UpgradeId) -> Option<Upgrade11Id> {
    Upgrade11Id::ALL.into_iter().find(|x| x.def().legacy == u)
}

/// `entity_event_t` of 1.1 as the legacy event (EV_LEV4_CHARGE_* are the
/// trample events of legacy).
pub fn event_from_11(ev: Event11) -> Event {
    use Event as L;
    use Event11 as V;
    match ev {
        V::None => L::None,
        V::Footstep => L::Footstep,
        V::FootstepMetal => L::FootstepMetal,
        V::FootstepSquelch => L::FootstepSquelch,
        V::Footsplash => L::Footsplash,
        V::Footwade => L::Footwade,
        V::Swim => L::Swim,
        V::Step4 => L::Step4,
        V::Step8 => L::Step8,
        V::Step12 => L::Step12,
        V::Step16 => L::Step16,
        V::Stepdn4 => L::Stepdn4,
        V::Stepdn8 => L::Stepdn8,
        V::Stepdn12 => L::Stepdn12,
        V::Stepdn16 => L::Stepdn16,
        V::FallShort => L::FallShort,
        V::FallMedium => L::FallMedium,
        V::FallFar => L::FallFar,
        V::Falling => L::Falling,
        V::Jump => L::Jump,
        V::WaterTouch => L::WaterTouch,
        V::WaterLeave => L::WaterLeave,
        V::WaterUnder => L::WaterUnder,
        V::WaterClear => L::WaterClear,
        V::Noammo => L::Noammo,
        V::ChangeWeapon => L::ChangeWeapon,
        V::FireWeapon => L::FireWeapon,
        V::FireWeapon2 => L::FireWeapon2,
        V::FireWeapon3 => L::FireWeapon3,
        V::PlayerRespawn => L::PlayerRespawn,
        V::PlayerTeleportIn => L::PlayerTeleportIn,
        V::PlayerTeleportOut => L::PlayerTeleportOut,
        V::GrenadeBounce => L::GrenadeBounce,
        V::GeneralSound => L::GeneralSound,
        V::GlobalSound => L::GlobalSound,
        V::BulletHitFlesh => L::BulletHitFlesh,
        V::BulletHitWall => L::BulletHitWall,
        V::Shotgun => L::Shotgun,
        V::MissileHit => L::MissileHit,
        V::MissileMiss => L::MissileMiss,
        V::MissileMissMetal => L::MissileMissMetal,
        V::Teslatrail => L::Teslatrail,
        V::Bullet => L::Bullet,
        V::Lev1Grab => L::Lev1Grab,
        V::Lev4ChargePrepare => L::Lev4TramplePrepare,
        V::Lev4ChargeStart => L::Lev4TrampleStart,
        V::Pain => L::Pain,
        V::Death1 => L::Death1,
        V::Death2 => L::Death2,
        V::Death3 => L::Death3,
        V::Obituary => L::Obituary,
        V::GibPlayer => L::GibPlayer,
        V::BuildConstruct => L::BuildConstruct,
        V::BuildDestroy => L::BuildDestroy,
        V::BuildDelay => L::BuildDelay,
        V::BuildRepair => L::BuildRepair,
        V::BuildRepaired => L::BuildRepaired,
        V::HumanBuildableExplosion => L::HumanBuildableExplosion,
        V::AlienBuildableExplosion => L::AlienBuildableExplosion,
        V::AlienAcidtube => L::AlienAcidtube,
        V::MedkitUsed => L::MedkitUsed,
        V::AlienEvolve => L::AlienEvolve,
        V::AlienEvolveFailed => L::AlienEvolveFailed,
        V::DebugLine => L::DebugLine,
        V::Stoploopingsound => L::Stoploopingsound,
        V::Taunt => L::Taunt,
        V::OvermindAttack => L::OvermindAttack,
        V::OvermindDying => L::OvermindDying,
        V::OvermindSpawns => L::OvermindSpawns,
        V::DccAttack => L::DccAttack,
        V::RptuseSound => L::RptuseSound,
    }
}
/// A raw 1.1 event number (EV_EVENT_BITS kept) as the legacy one.
pub fn event_num_from_11(ev: i32) -> i32 {
    let bits = ev & k11::EV_EVENT_BITS;
    let e = Event11::from_raw((ev & !k11::EV_EVENT_BITS).clamp(0, 255) as u8).map(event_from_11).map(|e| e as i32).unwrap_or(0);
    e | bits
}
/// A raw legacy event number (EV_EVENT_BITS kept) as the 1.1 one (0 for
/// the legacy-only events).
pub fn event_num_to_11(ev: i32) -> i32 {
    let bits = ev & k::EV_EVENT_BITS;
    let low = ev & !k::EV_EVENT_BITS;
    let e = Event11::ALL.iter().find(|&&e| event_from_11(e) as i32 == low).map(|&e| e as i32).unwrap_or(0);
    e | bits
}

/// `meansOfDeath_t` of 1.1 as the legacy one (MOD_LEVEL4_CHARGE is legacy's
/// MOD_LEVEL4_TRAMPLE).
pub fn mod_from_11(m: i32) -> i32 {
    use Mod as L;
    use Mod11 as V;
    let Some(v) = Mod11::from_raw(m.clamp(0, 255) as u8) else { return L::Unknown as i32 };
    (match v {
        V::Unknown => L::Unknown,
        V::Shotgun => L::Shotgun,
        V::Blaster => L::Blaster,
        V::Painsaw => L::Painsaw,
        V::Machinegun => L::Machinegun,
        V::Chaingun => L::Chaingun,
        V::Prifle => L::Prifle,
        V::Mdriver => L::Mdriver,
        V::Lasgun => L::Lasgun,
        V::Lcannon => L::Lcannon,
        V::LcannonSplash => L::LcannonSplash,
        V::Flamer => L::Flamer,
        V::FlamerSplash => L::FlamerSplash,
        V::Grenade => L::Grenade,
        V::Water => L::Water,
        V::Slime => L::Slime,
        V::Lava => L::Lava,
        V::Crush => L::Crush,
        V::Telefrag => L::Telefrag,
        V::Falling => L::Falling,
        V::Suicide => L::Suicide,
        V::TargetLaser => L::TargetLaser,
        V::TriggerHurt => L::TriggerHurt,
        V::AbuilderClaw => L::AbuilderClaw,
        V::Level0Bite => L::Level0Bite,
        V::Level1Claw => L::Level1Claw,
        V::Level1Pcloud => L::Level1Pcloud,
        V::Level3Claw => L::Level3Claw,
        V::Level3Pounce => L::Level3Pounce,
        V::Level3Bounceball => L::Level3Bounceball,
        V::Level2Claw => L::Level2Claw,
        V::Level2Zap => L::Level2Zap,
        V::Level4Claw => L::Level4Claw,
        V::Level4Charge => L::Level4Trample,
        V::Slowblob => L::Slowblob,
        V::Poison => L::Poison,
        V::Swarm => L::Swarm,
        V::Hspawn => L::Hspawn,
        V::Teslagen => L::Teslagen,
        V::Mgturret => L::Mgturret,
        V::Reactor => L::Reactor,
        V::Aspawn => L::Aspawn,
        V::Atube => L::Atube,
        V::Overmind => L::Overmind,
    }) as i32
}

/// The legacy `ET_*` of a 1.1 `entityType_t` (legacy inserts ET_LOCATION
/// after ET_BUILDABLE); `ET_EVENTS + event` maps its event too.
pub fn etype_from_11(t: i32) -> i32 {
    let ev11 = crate::gen::enums::EntityType11::Events as i32;
    if t >= ev11 {
        return svgame::ET_EVENTS + event_num_from_11(t - ev11);
    }
    if t >= crate::gen::enums::EntityType11::Missile as i32 {
        t + 1
    } else {
        t
    }
}

/// 1.1 `pm_flags` as legacy's (legacy inserts PMF_TIME_WATERJUMP at 128,
/// so 1.1's 256.. move down one bit; PMF_SPRINTHELD has no 1.1 source).
pub fn pm_flags_from_11(f: i32) -> i32 {
    (f & 0xff) | ((f & !0xff) >> 1)
}
/// Legacy `pm_flags` as 1.1's.
pub fn pm_flags_to_11(f: i32) -> i32 {
    (f & 0x7f) | ((f & 0x7fff80) << 1)
}
/// 1.1's `pmtype_t` as legacy's (PM_SPINTERMISSION is legacy's intermission).
pub fn pm_type_from_11(t: i32) -> i32 {
    if t == PmType11::Spintermission as i32 {
        PmType::Intermission as i32
    } else {
        t
    }
}

/// The SS_* bits of 1.1's STAT_STATE as legacy's, plus the legacy eFlags
/// that carry what 1.1 keeps there (SS_WALLCLIMBINGCEILING is legacy's
/// EF_WALLCLIMBCEILING, SS_POISONCLOUDED its EF_POISONCLOUDED).
pub fn stat_state_from_11(ss: i32) -> (i32, i32) {
    let pairs = [
        (k11::SS_WALLCLIMBING, k::SS_WALLCLIMBING),
        (k11::SS_CREEPSLOWED, k::SS_CREEPSLOWED),
        (k11::SS_SPEEDBOOST, k::SS_SPEEDBOOST),
        (k11::SS_GRABBED, k::SS_GRABBED),
        (k11::SS_BLOBLOCKED, k::SS_BLOBLOCKED),
        (k11::SS_POISONED, k::SS_POISONED),
        (k11::SS_BOOSTED, k::SS_BOOSTED),
        (k11::SS_SLOWLOCKED, k::SS_SLOWLOCKED),
        (k11::SS_MEDKIT_ACTIVE, k::SS_HEALING_ACTIVE),
        (k11::SS_CHARGING, k::SS_CHARGING),
    ];
    let mut out = 0;
    for (a, b) in pairs {
        if ss & a != 0 {
            out |= b;
        }
    }
    let mut ef = 0;
    if ss & k11::SS_WALLCLIMBINGCEILING != 0 {
        ef |= k::EF_WALLCLIMBCEILING;
    }
    if ss & k11::SS_POISONCLOUDED != 0 {
        ef |= k::EF_POISONCLOUDED;
    }
    (out, ef)
}

/// 1.1 eFlags of a player, missile or other non-structure entity as legacy's:
/// bits 0..12 agree; EF_TALK / EF_VOTED / EF_TEAMVOTED / EF_REAL_LIGHT have
/// no legacy counterpart (legacy's 0x2000 is EF_POISONCLOUDED) and
/// EF_BLOBLOCKED moves.
pub fn eflags_from_11(ef: i32) -> i32 {
    let mut out = ef & 0x1fff;
    if ef & k11::EF_CONNECTION != 0 {
        out |= k::EF_CONNECTION;
    }
    if ef & k11::EF_BLOBLOCKED != 0 {
        out |= k::EF_BLOBLOCKED;
    }
    out
}

// =====================================================================
// playerState
// =====================================================================

/// Legacy `ps.misc[]` words holding the 1.1 playerState words the legacy
/// one has no room for: raw eFlags, STAT_WEAPONS / STAT_WEAPONS2 /
/// STAT_SLOTS / STAT_STATE / STAT_MISC / STAT_BUILDABLE / STAT_BOOSTTIME,
/// PERS_TEAM / PERS_RANK / PERS_BANK, jumppad_ent, pm_flags | pm_type << 24,
/// and up to three (weapon + 1) << 16 | packed ammo words of the weapons held.
pub const STASH_EFLAGS: usize = 0;
pub const STASH_WEAPONS: usize = 1;
pub const STASH_WEAPONS2: usize = 2;
pub const STASH_SLOTS: usize = 3;
pub const STASH_STATE: usize = 4;
pub const STASH_MISC: usize = 5;
pub const STASH_BUILDABLE: usize = 6;
pub const STASH_BOOSTTIME: usize = 7;
pub const STASH_PERS_TEAM: usize = 8;
pub const STASH_PERS_RANK: usize = 9;
pub const STASH_PERS_BANK: usize = 10;
pub const STASH_JUMPPAD: usize = 11;
pub const STASH_PMOVE: usize = 12;
pub const STASH_AMMO: usize = 13;
/// Ammo words kept: a human holds the blaster and one other weapon.
const STASH_AMMO_SLOTS: usize = 3;

/// `TEAM_SPECTATOR` (1.1 team_t).
const TEAM_SPECTATOR11: i32 = 1;

/// The legacy STAT_MISC of a 1.1 one: the pounce, trample and lucifer cannon
/// charges are rescaled to the legacy units the HUD and view draw (1.1 keeps
/// the pounce speed, the charge time out of LEVEL4_CHARGE_TIME and the
/// lcannon charge out of LCANNON_TOTAL_CHARGE); build timers are milliseconds
/// in both.
pub fn stat_misc_from_11(weapon11: i32, state11: i32, misc: i32) -> i32 {
    let w = u8::try_from(weapon11).ok().and_then(Weapon11Id::from_num);
    match w {
        Some(Weapon11Id::Level3) => misc * k::LEVEL3_POUNCE_TIME / k11::LEVEL3_POUNCE_SPEED,
        Some(Weapon11Id::Level3upg) => misc * k::LEVEL3_POUNCE_TIME_UPG / k11::LEVEL3_POUNCE_UPG_SPEED,
        Some(Weapon11Id::Level4) if state11 & k11::SS_CHARGING == 0 => misc * k::LEVEL4_TRAMPLE_CHARGE_MAX / k11::LEVEL4_CHARGE_TIME,
        Some(Weapon11Id::Lcannon) => misc * k::LCANNON_CHARGE_TIME_MAX / k11::LCANNON_TOTAL_CHARGE,
        _ => misc,
    }
}

/// STAT_BUILDABLE (buildable | SB_VALID_TOGGLEBIT) of 1.1 as legacy's.
pub fn stat_buildable_from_11(b: i32) -> i32 {
    let valid = if b & k11::SB_VALID_TOGGLEBIT != 0 { k::SB_VALID_TOGGLEBIT } else { 0 };
    buildable_from_11(b & !k11::SB_VALID_TOGGLEBIT) | valid
}

/// The weapons of a 1.1 inventory (STAT_WEAPONS / STAT_WEAPONS2).
pub fn weapons11_held(stats: &[i32; 16]) -> Vec<i32> {
    (1..32).filter(|&w| super::bg11::inventory_contains_weapon(w, stats)).collect()
}

/// The 1.1 playerState as the legacy cgame reads it.
pub fn ps_from_11(p: &PlayerState11) -> PlayerState {
    let st = |s: Stat11| p.stats[s as usize];
    let pe = |s: Pers11| p.persistant[s as usize];
    let (state, ef_extra) = stat_state_from_11(st(Stat11::State));
    let mut s = PlayerState {
        command_time: p.command_time,
        pm_type: pm_type_from_11(p.pm_type),
        bob_cycle: p.bob_cycle,
        pm_flags: pm_flags_from_11(p.pm_flags),
        pm_time: p.pm_time,
        origin: p.origin,
        velocity: p.velocity,
        weapon_time: p.weapon_time,
        gravity: p.gravity,
        speed: p.speed,
        delta_angles: p.delta_angles,
        ground_entity_num: p.ground_entity_num,
        legs_timer: p.legs_timer,
        legs_anim: p.legs_anim,
        torso_timer: p.torso_timer,
        torso_anim: p.torso_anim,
        movement_dir: p.movement_dir,
        grapple_point: p.grapple_point,
        e_flags: eflags_from_11(p.e_flags) | ef_extra,
        event_sequence: p.event_sequence,
        events: [event_num_from_11(p.events[0]), event_num_from_11(p.events[1])],
        event_parms: p.event_parms,
        external_event: event_num_from_11(p.external_event),
        external_event_parm: p.external_event_parm,
        external_event_time: p.external_event_time,
        client_num: p.client_num,
        weapon: weapon_from_11(p.weapon),
        weaponstate: p.weaponstate,
        viewangles: p.viewangles,
        viewheight: p.viewheight,
        damage_event: p.damage_event,
        damage_yaw: p.damage_yaw,
        damage_pitch: p.damage_pitch,
        damage_count: p.damage_count,
        generic1: p.generic1,
        loop_sound: p.loop_sound,
        ping: p.ping,
        pmove_framecount: p.pmove_framecount,
        jumppad_frame: p.jumppad_frame,
        entity_event_sequence: p.entity_event_sequence,
        ..Default::default()
    };
    // events whose parm is a means of death
    for i in 0..2 {
        if p.events[i] & !k11::EV_EVENT_BITS == Event11::Obituary as i32 {
            s.event_parms[i] = mod_from_11(p.event_parms[i]);
        }
    }
    let team = st(Stat11::Pteam);
    let held = weapons11_held(&p.stats);
    // legacy STAT_WEAPON: the weapon held besides the humans' blaster
    let primary = held.iter().copied().find(|&w| w != Weapon11Id::Blaster as i32).unwrap_or(0);
    let ls = &mut s.stats;
    ls[Stat::Health as usize] = st(Stat11::Health);
    ls[Stat::Items as usize] = (1..16).filter(|&u| super::bg11::inventory_contains_upgrade(u, &p.stats)).fold(0, |m, u| m | 1 << upgrade_from_11(u));
    ls[Stat::Activeitems as usize] = (1..16).filter(|&u| super::bg11::upgrade_is_active(u, &p.stats)).fold(0, |m, u| m | 1 << upgrade_from_11(u));
    ls[Stat::Weapon as usize] = weapon_from_11(primary);
    ls[Stat::MaxHealth as usize] = st(Stat11::MaxHealth);
    ls[Stat::Class as usize] = class_from_11(st(Stat11::Pclass));
    ls[Stat::Team as usize] = team;
    ls[Stat::Stamina as usize] = st(Stat11::Stamina);
    ls[Stat::State as usize] = state;
    ls[Stat::Misc as usize] = stat_misc_from_11(p.weapon, st(Stat11::State), st(Stat11::Misc));
    ls[Stat::Buildable as usize] = stat_buildable_from_11(st(Stat11::Buildable));
    ls[Stat::Falldist as usize] = st(Stat11::Falldist);
    ls[Stat::Viewlock as usize] = st(Stat11::Viewlock);
    let lp = &mut s.persistant;
    lp[Pers::Score as usize] = pe(Pers11::Score);
    lp[Pers::Hits as usize] = pe(Pers11::Hits);
    lp[Pers::Specstate as usize] = if pe(Pers11::Team) == TEAM_SPECTATOR11 {
        if p.pm_flags & k11::PMF_FOLLOW != 0 {
            SpectatorState::Follow as i32
        } else {
            SpectatorState::Free as i32
        }
    } else {
        SpectatorState::Not as i32
    };
    lp[Pers::SpawnCount as usize] = pe(Pers11::SpawnCount);
    lp[Pers::Attacker as usize] = pe(Pers11::Attacker);
    lp[Pers::Killed as usize] = pe(Pers11::Killed);
    lp[Pers::State as usize] = pe(Pers11::State);
    // 1.1 aliens count frags, legacy aliens ALIEN_CREDITS_PER_KILL each
    lp[Pers::Credit as usize] = if team == Team::Aliens as i32 { pe(Pers11::Credit) * k::ALIEN_CREDITS_PER_KILL } else { pe(Pers11::Credit) };
    lp[Pers::Queuepos as usize] = pe(Pers11::Queuepos);
    lp[Pers::Newweapon as usize] = weapon_from_11(pe(Pers11::Newweapon));
    let (ammo, clips) = ammo_of_11(p, p.weapon, &held);
    s.ammo = ammo;
    s.clips = clips;
    // the stash (ps_to_11)
    let m = &mut s.misc;
    m[STASH_EFLAGS] = p.e_flags;
    m[STASH_WEAPONS] = st(Stat11::Weapons);
    m[STASH_WEAPONS2] = st(Stat11::Weapons2);
    m[STASH_SLOTS] = st(Stat11::Slots);
    m[STASH_STATE] = st(Stat11::State);
    m[STASH_MISC] = st(Stat11::Misc);
    m[STASH_BUILDABLE] = st(Stat11::Buildable);
    m[STASH_BOOSTTIME] = st(Stat11::Boosttime);
    m[STASH_PERS_TEAM] = pe(Pers11::Team);
    m[STASH_PERS_RANK] = pe(Pers11::Rank);
    m[STASH_PERS_BANK] = pe(Pers11::Bank);
    m[STASH_JUMPPAD] = p.jumppad_ent;
    m[STASH_PMOVE] = (p.pm_flags & 0xffffff) | (p.pm_type << 24);
    for (i, &w) in held.iter().take(STASH_AMMO_SLOTS).enumerate() {
        let (a, c) = ammo_of_11(p, w, &held);
        m[STASH_AMMO + i] = ((w + 1) << 16) | (a | c << 12);
    }
    s
}

/// Ammo and clips of `weapon` in a 1.1 playerState. Stock 1.1 keeps the
/// packed word at `ammo[weapon]`; some 1.1-protocol servers (newer game
/// logic behind the 1.1 compatibility layer) put the current weapon's word
/// in another slot. Without this the prediction sees an empty rifle and
/// plays EV_NOAMMO instead of the fire sound, and the real shot arrives
/// late with the snapshot. When the weapon's own slot is empty, take a word
/// from a slot no held weapon owns.
fn ammo_of_11(p: &PlayerState11, weapon: i32, held: &[i32]) -> (i32, i32) {
    let (a, c) = super::bg11::unpack_ammo_array(weapon, &p.ammo, &p.powerups);
    if a != 0 || c != 0 || weapon != p.weapon || weapon <= 0 {
        return (a, c);
    }
    match (0..16).find(|&i| p.ammo[i] != 0 && !held.contains(&(i as i32))) {
        Some(i) => (p.ammo[i] & 0x0fff, (p.ammo[i] >> 12) & 0x0f),
        None => (a, c),
    }
}

/// The 1.1 playerState of a legacy one made by `ps_from_11` (what the 1.1
/// prediction runs on): `ps_from_11(&ps_to_11(&ps_from_11(&p)))` is
/// `ps_from_11(&p)`. Only the ammo of the weapons held survives.
pub fn ps_to_11(s: &PlayerState) -> PlayerState11 {
    let m = &s.misc;
    let mut p = PlayerState11 {
        command_time: s.command_time,
        pm_type: m[STASH_PMOVE] >> 24,
        bob_cycle: s.bob_cycle,
        pm_flags: m[STASH_PMOVE] & 0xffffff,
        pm_time: s.pm_time,
        origin: s.origin,
        velocity: s.velocity,
        weapon_time: s.weapon_time,
        gravity: s.gravity,
        speed: s.speed,
        delta_angles: s.delta_angles,
        ground_entity_num: s.ground_entity_num,
        legs_timer: s.legs_timer,
        legs_anim: s.legs_anim,
        torso_timer: s.torso_timer,
        torso_anim: s.torso_anim,
        movement_dir: s.movement_dir,
        grapple_point: s.grapple_point,
        e_flags: m[STASH_EFLAGS],
        event_sequence: s.event_sequence,
        events: [event_num_to_11(s.events[0]), event_num_to_11(s.events[1])],
        event_parms: s.event_parms,
        external_event: event_num_to_11(s.external_event),
        external_event_parm: s.external_event_parm,
        external_event_time: s.external_event_time,
        client_num: s.client_num,
        weapon: weapon_to_11(s.weapon),
        weaponstate: s.weaponstate,
        viewangles: s.viewangles,
        viewheight: s.viewheight,
        damage_event: s.damage_event,
        damage_yaw: s.damage_yaw,
        damage_pitch: s.damage_pitch,
        damage_count: s.damage_count,
        generic1: s.generic1,
        loop_sound: s.loop_sound,
        jumppad_ent: m[STASH_JUMPPAD],
        ping: s.ping,
        pmove_framecount: s.pmove_framecount,
        jumppad_frame: s.jumppad_frame,
        entity_event_sequence: s.entity_event_sequence,
        ..Default::default()
    };
    for i in 0..2 {
        if p.events[i] & !k11::EV_EVENT_BITS == Event11::Obituary as i32 {
            p.event_parms[i] = Mod11::ALL.iter().find(|&&x| mod_from_11(x as i32) == s.event_parms[i]).map(|&x| x as i32).unwrap_or(0);
        }
    }
    let ls = |x: Stat| s.stats[x as usize];
    let st = &mut p.stats;
    st[Stat11::Health as usize] = ls(Stat::Health);
    st[Stat11::Items as usize] = (1..16).filter(|&u| ls(Stat::Items) & (1 << u) != 0).fold(0, |a, u| a | 1 << upgrade_to_11(u));
    st[Stat11::Slots as usize] = m[STASH_SLOTS];
    st[Stat11::Activeitems as usize] = (1..16).filter(|&u| ls(Stat::Activeitems) & (1 << u) != 0).fold(0, |a, u| a | 1 << upgrade_to_11(u));
    st[Stat11::Weapons as usize] = m[STASH_WEAPONS];
    st[Stat11::Weapons2 as usize] = m[STASH_WEAPONS2];
    st[Stat11::MaxHealth as usize] = ls(Stat::MaxHealth);
    st[Stat11::Pclass as usize] = class_to_11(ls(Stat::Class));
    st[Stat11::Pteam as usize] = ls(Stat::Team);
    st[Stat11::Stamina as usize] = ls(Stat::Stamina);
    st[Stat11::State as usize] = m[STASH_STATE];
    st[Stat11::Misc as usize] = m[STASH_MISC];
    st[Stat11::Buildable as usize] = m[STASH_BUILDABLE];
    st[Stat11::Boosttime as usize] = m[STASH_BOOSTTIME];
    st[Stat11::Falldist as usize] = ls(Stat::Falldist);
    st[Stat11::Viewlock as usize] = ls(Stat::Viewlock);
    let lp = |x: Pers| s.persistant[x as usize];
    let pe = &mut p.persistant;
    pe[Pers11::Score as usize] = lp(Pers::Score);
    pe[Pers11::Hits as usize] = lp(Pers::Hits);
    pe[Pers11::Rank as usize] = m[STASH_PERS_RANK];
    pe[Pers11::Team as usize] = m[STASH_PERS_TEAM];
    pe[Pers11::SpawnCount as usize] = lp(Pers::SpawnCount);
    pe[Pers11::Attacker as usize] = lp(Pers::Attacker);
    pe[Pers11::Killed as usize] = lp(Pers::Killed);
    pe[Pers11::State as usize] = lp(Pers::State);
    pe[Pers11::Credit as usize] = if ls(Stat::Team) == Team::Aliens as i32 { lp(Pers::Credit) / k::ALIEN_CREDITS_PER_KILL } else { lp(Pers::Credit) };
    pe[Pers11::Bank as usize] = m[STASH_PERS_BANK];
    pe[Pers11::Queuepos as usize] = lp(Pers::Queuepos);
    pe[Pers11::Newweapon as usize] = weapon_to_11(lp(Pers::Newweapon));
    for i in 0..STASH_AMMO_SLOTS {
        let v = m[STASH_AMMO + i];
        if v != 0 {
            let w = (v >> 16) - 1;
            let word = v & 0xffff;
            super::bg11::pack_ammo_array(w, &mut p.ammo, &mut p.powerups, word & 0x0fff, (word >> 12) & 0x0f);
        }
    }
    p
}

// =====================================================================
// entityState
// =====================================================================

/// A structure's 1.1 `generic1` (health scaled to B_HEALTH_BITS bits and the
/// B_*_TOGGLEBIT flags) as the legacy encoding: hit points of the legacy
/// buildable (the presentation divides by its health) in `generic1` and the
/// EF_B_SPAWNED / EF_B_POWERED flags in eFlags.
pub fn buildable_health_from_11(legacy: Option<BuildableId>, generic1: i32) -> (i32, i32) {
    let scaled = generic1 & ((1 << k11::B_HEALTH_BITS) - 1);
    let max = legacy.map(|b| b.def().health).unwrap_or(0);
    let hp = (scaled as f32 / k11::B_HEALTH_SCALE * max as f32).ceil() as i32;
    let mut ef = 0;
    if generic1 & k11::B_SPAWNED_TOGGLEBIT != 0 {
        ef |= k::EF_B_SPAWNED;
    }
    if generic1 & k11::B_POWERED_TOGGLEBIT != 0 {
        ef |= k::EF_B_POWERED;
    }
    (hp, ef)
}

/// A 1.1 entityState (powerups arrive in `misc`) as the legacy cgame reads
/// it. Temporary event entities are re-encoded as the port's event entities
/// (`svgame::event_state`, read back by `svgame::event_from_state`); an
/// event carried on a regular entity (`s.event`) keeps its fields with the
/// event renumbered, for `entity_event_from_11`.
pub fn es_from_11(e: &EntityState) -> EntityState {
    let mut s = *e;
    let et11 = e.e_type;
    s.e_type = etype_from_11(et11);
    s.event = event_num_from_11(e.event);
    s.weapon_anim = 0;
    use crate::gen::enums::EntityType11 as T;
    let events11 = T::Events as i32;
    if et11 >= events11 {
        let ev = Event11::from_raw((et11 - events11).clamp(0, 255) as u8).unwrap_or(Event11::None);
        // the 1.1 fields with legacy numbers
        let mut m = *e;
        m.weapon = weapon_from_11(e.weapon);
        let predictable = e.e_flags & k11::EF_PLAYER_EVENT != 0;
        let entity = if predictable { e.other_entity_num } else { -1 };
        let mut ge = game_event_11(&m, event_from_11(ev), entity, &e.pos.tr_base);
        ge.predictable = predictable;
        return svgame::event_state(&ge, e.number);
    }
    match et11 {
        x if x == T::Player as i32 => {
            s.weapon = weapon_from_11(e.weapon);
            s.e_flags = eflags_from_11(e.e_flags);
            // team | class << 8
            s.misc = (e.misc & 0xff) | (class_from_11((e.misc >> 8) & 0xff) << 8);
            s.modelindex = (1..16).filter(|&u| e.modelindex & (1 << u) != 0).fold(0, |a, u| a | 1 << upgrade_from_11(u));
            s.modelindex2 = (1..16).filter(|&u| e.modelindex2 & (1 << u) != 0).fold(0, |a, u| a | 1 << upgrade_from_11(u));
        }
        x if x == T::Buildable as i32 => {
            let b = buildable_from_11(e.modelindex);
            s.modelindex = b;
            let id = BuildableId::from_num(b.clamp(0, 255) as u8);
            let (hp, ef) = buildable_health_from_11(id, e.generic1);
            s.generic1 = hp;
            s.e_flags = (e.e_flags & (k::EF_DEAD | k::EF_TELEPORT_BIT | k::EF_NODRAW)) | ef;
            s.weapon = weapon_from_11(e.weapon);
        }
        x if x == T::Corpse as i32 => {
            // 1.1 corpses carry the class in clientNum; legacy's in generic1
            // and the team in modelindex2
            let class = class_from_11(e.client_num);
            s.generic1 = class;
            s.modelindex2 = ClassId::from_num(class.clamp(0, 255) as u8).map(|c| c.def().team as i32).unwrap_or(0);
            s.e_flags = eflags_from_11(e.e_flags);
            s.misc = 0;
        }
        x if x == T::Missile as i32 => {
            s.weapon = weapon_from_11(e.weapon);
            s.e_flags = eflags_from_11(e.e_flags);
        }
        _ => {
            s.e_flags = eflags_from_11(e.e_flags);
            s.weapon = weapon_from_11(e.weapon);
        }
    }
    s
}

/// What a 1.1 event means in the port's `GameEvent` terms, from the fields
/// 1.1's cg_event.c reads (`es` with legacy weapon numbers): `entity` is the
/// entity the event belongs to (-1 for a free-standing temp entity),
/// `origin` the event position.
pub fn game_event_11(es: &EntityState, ev: Event, entity: i32, origin: &Vec3) -> GameEvent {
    let ent = |n: i32| if n == k::ENTITYNUM_NONE { -1 } else { n };
    let mut g = GameEvent { time: es.time, event: ev, origin: *origin, origin2: es.origin2, entity, other: -1, parm: es.event_parm, weapon: es.weapon, generic1: es.generic1, charge: 0, predictable: false };
    match ev {
        // CG_MissileHitPlayer( weapon, generic1, position, dir, otherEntityNum )
        Event::MissileHit => g.other = ent(es.other_entity_num),
        // CG_Bullet( trBase, otherEntityNum (the shooter), dir / target )
        Event::BulletHitWall | Event::BulletHitFlesh | Event::Shotgun => g.other = ent(es.other_entity_num),
        // source = generic1, target = clientNum
        Event::Teslatrail => {
            g.other = es.client_num;
            g.parm = es.generic1;
            g.weapon = WeaponId::Teslagen as i32;
        }
        // victim = otherEntityNum, attacker = otherEntityNum2, eventParm = MOD
        Event::Obituary => {
            g.weapon = es.other_entity_num;
            g.other = es.other_entity_num2;
            g.parm = mod_from_11(es.event_parm);
        }
        _ => {}
    }
    g
}

/// `CG_CheckEvents` for an event carried on a regular entity of a 1.1
/// snapshot (`s.event` changed; `s` from `es_from_11`): the GameEvent at the
/// entity's position.
pub fn entity_event_from_11(s: &EntityState, number: usize, lerp_origin: &Vec3) -> Option<GameEvent> {
    let ev = Event::from_raw((s.event & !k::EV_EVENT_BITS).clamp(0, 255) as u8)?;
    if ev == Event::None {
        return None;
    }
    // (the obituary's means of death is still the 1.1 one in eventParm)
    Some(game_event_11(s, ev, number as i32, lerp_origin))
}

// =====================================================================
// configstrings
// =====================================================================

/// 1.1 `CS_*` (bg_public.h / q_shared.h of 1.1.0).
pub mod cs11 {
    pub const SERVERINFO: usize = 0;
    pub const SYSTEMINFO: usize = 1;
    pub const MUSIC: usize = 2;
    pub const MESSAGE: usize = 3;
    pub const MOTD: usize = 4;
    pub const WARMUP: usize = 5;
    pub const SCORES1: usize = 6;
    pub const SCORES2: usize = 7;
    pub const VOTE_TIME: usize = 8;
    pub const VOTE_STRING: usize = 9;
    pub const VOTE_YES: usize = 10;
    pub const VOTE_NO: usize = 11;
    pub const TEAMVOTE_TIME: usize = 12;
    pub const TEAMVOTE_STRING: usize = 14;
    pub const TEAMVOTE_YES: usize = 16;
    pub const TEAMVOTE_NO: usize = 18;
    pub const GAME_VERSION: usize = 20;
    pub const LEVEL_START_TIME: usize = 21;
    pub const INTERMISSION: usize = 22;
    pub const FLAGSTATUS: usize = 23;
    pub const SHADERSTATE: usize = 24;
    pub const BOTINFO: usize = 25;
    pub const CLIENTS_READY: usize = 26;
    pub const BUILDPOINTS: usize = 28;
    pub const STAGES: usize = 29;
    pub const SPAWNS: usize = 30;
    pub const MODELS: usize = 33;
    pub const SOUNDS: usize = MODELS + 256;
    pub const SHADERS: usize = SOUNDS + 256;
    pub const PARTICLE_SYSTEMS: usize = SHADERS + 64;
    pub const PLAYERS: usize = PARTICLE_SYSTEMS + 64;
    pub const PRECACHES: usize = PLAYERS + 64;
    pub const LOCATIONS: usize = PRECACHES + 64;
    pub const MAX: usize = LOCATIONS + 64;
}

/// Where the 1.1 configstrings legacy has no slot for land: past legacy's
/// CS_MAX (MAX_CONFIGSTRINGS is 1024 in both), for the cgame of a protocol
/// 69 game: "alienBP alienBPTotal humanBP humanBPTotal humanBPPowered",
/// the raw CS_STAGES "alienStage humanStage alienKills humanKills
/// alienNext humanNext", "numAlienSpawns numHumanSpawns" and CS_SCORES1/2.
pub const CS11_BUILDPOINTS: usize = cs::MAX;
pub const CS11_STAGES: usize = cs::MAX + 1;
pub const CS11_SPAWNS: usize = cs::MAX + 2;
pub const CS11_SCORES1: usize = cs::MAX + 3;
pub const CS11_SCORES2: usize = cs::MAX + 4;

/// The legacy index of a 1.1 configstring index (None: no counterpart).
pub fn configstring_index_from_11(index: usize) -> Option<usize> {
    use cs11 as v;
    Some(match index {
        v::SERVERINFO => cs::SERVERINFO,
        v::SYSTEMINFO => cs::SYSTEMINFO,
        v::MUSIC => cs::MUSIC,
        v::MESSAGE => cs::MESSAGE,
        v::MOTD => cs::MOTD,
        v::WARMUP => cs::WARMUP,
        v::SCORES1 => CS11_SCORES1,
        v::SCORES2 => CS11_SCORES2,
        // the global vote is legacy's vote 0 (TEAM_NONE)
        v::VOTE_TIME => cs::VOTE_TIME,
        v::VOTE_STRING => cs::VOTE_STRING,
        v::VOTE_YES => cs::VOTE_YES,
        v::VOTE_NO => cs::VOTE_NO,
        // team votes: 1.1 cs_offset 0 = humans, 1 = aliens; legacy + team
        i if (v::TEAMVOTE_TIME..v::TEAMVOTE_TIME + 2).contains(&i) => cs::VOTE_TIME + team_vote_slot(i - v::TEAMVOTE_TIME),
        i if (v::TEAMVOTE_STRING..v::TEAMVOTE_STRING + 2).contains(&i) => cs::VOTE_STRING + team_vote_slot(i - v::TEAMVOTE_STRING),
        i if (v::TEAMVOTE_YES..v::TEAMVOTE_YES + 2).contains(&i) => cs::VOTE_YES + team_vote_slot(i - v::TEAMVOTE_YES),
        i if (v::TEAMVOTE_NO..v::TEAMVOTE_NO + 2).contains(&i) => cs::VOTE_NO + team_vote_slot(i - v::TEAMVOTE_NO),
        v::GAME_VERSION => cs::GAME_VERSION,
        v::LEVEL_START_TIME => cs::LEVEL_START_TIME,
        v::INTERMISSION => cs::INTERMISSION,
        v::SHADERSTATE => cs::SHADERSTATE,
        v::BOTINFO => cs::BOTINFO,
        v::CLIENTS_READY => cs::CLIENTS_READY,
        v::BUILDPOINTS => CS11_BUILDPOINTS,
        v::STAGES => CS11_STAGES,
        v::SPAWNS => CS11_SPAWNS,
        i if (v::MODELS..v::SOUNDS).contains(&i) => cs::MODELS + (i - v::MODELS),
        i if (v::SOUNDS..v::SHADERS).contains(&i) => cs::SOUNDS + (i - v::SOUNDS),
        i if (v::SHADERS..v::PARTICLE_SYSTEMS).contains(&i) => cs::SHADERS + (i - v::SHADERS),
        i if (v::PARTICLE_SYSTEMS..v::PLAYERS).contains(&i) => cs::PARTICLE_SYSTEMS + (i - v::PARTICLE_SYSTEMS),
        i if (v::PLAYERS..v::PRECACHES).contains(&i) => cs::PLAYERS + (i - v::PLAYERS),
        i if (v::LOCATIONS..v::MAX).contains(&i) => cs::LOCATIONS + (i - v::LOCATIONS),
        _ => return None,
    })
}
fn team_vote_slot(cs_offset: usize) -> usize {
    if cs_offset == 0 {
        Team::Humans as usize
    } else {
        Team::Aliens as usize
    }
}

/// A configstring of the 1.1 gamestate: the legacy index and text, or None
/// when legacy has no counterpart. CS_CLIENTS_READY's "%d" bitmask becomes
/// legacy's client list string; CS_STAGES keeps its 1.1 text in CS11_STAGES
/// (`configstrings_from_11` also writes legacy's two stage strings).
pub fn configstring_from_11(index: usize, s: &str) -> Option<(usize, String)> {
    let li = configstring_index_from_11(index)?;
    let v = if index == cs11::CLIENTS_READY {
        let mask = super::spawn::c_atoi(s) as u32;
        svgame::client_list_string(|i| i < 32 && mask & (1 << i) != 0)
    } else {
        s.to_string()
    };
    Some((li, v))
}

/// Every legacy configstring a 1.1 one sets: `configstring_from_11` plus,
/// for CS_STAGES, legacy's CS_ALIEN_STAGES / CS_HUMAN_STAGES
/// ("stage kills nextThreshold", from g_main.c's "%d %d %d %d %d %d").
pub fn configstrings_from_11(index: usize, s: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = configstring_from_11(index, s).into_iter().collect();
    if index == cs11::STAGES {
        if let Some((a, h)) = stages_from_11(s) {
            out.push((cs::ALIEN_STAGES, a));
            out.push((cs::HUMAN_STAGES, h));
        }
    }
    out
}

/// 1.1's CS_STAGES text as legacy's (alien, human) stage strings.
pub fn stages_from_11(s: &str) -> Option<(String, String)> {
    let v: Vec<i32> = s.split_whitespace().map(super::spawn::c_atoi).collect();
    if v.len() < 6 {
        return None;
    }
    Some((format!("{} {} {}", v[0], v[2], v[4]), format!("{} {} {}", v[1], v[3], v[5])))
}

/// 1.1's CS_BUILDPOINTS: (alien BP, alien total, human BP, human total,
/// human BP powered).
pub fn buildpoints_from_11(s: &str) -> (i32, i32, i32, i32, i32) {
    let v: Vec<i32> = s.split_whitespace().map(super::spawn::c_atoi).collect();
    let g = |i: usize| v.get(i).copied().unwrap_or(0);
    (g(0), g(1), g(2), g(3), g(4))
}

// =====================================================================
// server commands
// =====================================================================

/// `Cmd_TokenizeString` of a server command (quotes group, // comments end).
pub fn tokenize(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = cmd.as_bytes();
    let mut i = 0;
    loop {
        while i < b.len() && b[i] <= b' ' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            break;
        }
        if b[i] == b'"' {
            i += 1;
            let st = i;
            while i < b.len() && b[i] != b'"' {
                i += 1;
            }
            out.push(String::from_utf8_lossy(&b[st..i]).into_owned());
            i += 1;
            continue;
        }
        let st = i;
        while i < b.len() && b[i] > b' ' && b[i] != b'"' {
            i += 1;
        }
        out.push(String::from_utf8_lossy(&b[st..i]).into_owned());
    }
    out
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "'"))
}

/// What 1.1's `CG_Menu` does for a `servermenu` id, as the port's server
/// notice (`servermenu 0 "<text>" <wants_menu>`, whose texts the port's
/// menus key on, cf. menus.rs `cg_menu`): the text the port's game prints
/// where it has the same refusal, else 1.1's own short message.
pub fn menu_notice_11(menu: Menu11) -> (&'static str, bool) {
    use Menu11 as M;
    match menu {
        M::Team => ("Choose a team", false),
        M::ATeamfull => ("The alien team has too many players. Please wait until slots become available or join the human team.", false),
        M::HTeamfull => ("The human team has too many players. Please wait until slots become available or join the alien team.", false),
        M::AClass => ("Choose a class to spawn as", false),
        M::ABuild | M::HBuild => (super::weapon::BUILD_MENU_NOTICE, true),
        M::AInfest => ("Evolve: choose a class", true),
        M::AHovelOccupied => ("This Hovel is occupied by another builder", false),
        M::AHovelBlocked => ("The exit to this Hovel is currently blocked", false),
        M::ANoeroom => ("Not enough room here to evolve", false),
        M::ATooclose => ("Too close to the enemy to evolve", false),
        M::ANoovmndEvolve => ("There is no Overmind", false),
        M::ASpwnwarn => ("This spawn will not be controlled by an Overmind", false),
        M::AOvermind => ("There can only be one Overmind", false),
        M::ANoassert => ("The Overmind cannot control any more structures", false),
        M::ANocreep => ("You need creep to build here", false),
        M::ANoovmnd => ("There is no Overmind", false),
        M::ANoroom | M::HNoroom => ("There is no room to build here", false),
        M::ANormal | M::HNormal => ("Cannot build on this surface", false),
        M::AHovel => ("There can only be one Hovel", false),
        M::AHovelExit => ("The exit to this Hovel would always be blocked", false),
        M::HSpawn => ("Choose an item to spawn with", false),
        M::HArmoury => ("Armoury", true),
        M::HNoslots => ("You have no room for that item", false),
        M::HNofunds => ("You don't have enough credits", false),
        M::HItemheld => ("You already hold this item", false),
        M::HRepeater => ("There is no power here", false),
        M::HNopower => ("There is not enough power left", false),
        M::HNotpowered => ("This Armoury is not powered", false),
        M::HNodcc => ("A Defence Computer is required", false),
        M::HReactor => ("There can only be one Reactor", false),
        M::HTnodewarn => ("This Telenode will not be powered", false),
        M::HRptwarn => ("This Repeater will not be powered", false),
        M::HRptwarn2 => ("There is already power here", false),
    }
}

/// A 1.1 server command (`svc_serverCommand` text) as legacy text(s).
pub fn server_command_from_11(cmd: &str) -> Vec<String> {
    let argv = tokenize(cmd);
    let a = |i: usize| argv.get(i).map(|s| s.as_str()).unwrap_or("");
    let num = |i: usize| super::spawn::c_atoi(a(i));
    match a(0) {
        // CG_Chat: 1.1 sends the whole line ("name: text"); legacy chat
        // carries the sender, the mode and the line
        "chat" | "tchat" => {
            let line: String = a(1).chars().filter(|&c| c != '\x19').collect();
            let mode = if a(0) == "tchat" { 1 } else { 0 };
            vec![format!("chat -1 {mode} {}", quoted(&line))]
        }
        "print" => {
            let mut out = vec![cmd.to_string()];
            // LogExit's announcement: legacy's cgame reads the winner from CS_WINNER
            match a(1).trim_end() {
                "Humans win" => out.push(format!("cs {} \"Humans Win\"", cs::WINNER)),
                "Aliens win" => out.push(format!("cs {} \"Aliens Win\"", cs::WINNER)),
                _ => {}
            }
            out
        }
        "cs" => {
            let i = num(1).max(0) as usize;
            configstrings_from_11(i, a(2)).into_iter().map(|(li, v)| format!("cs {li} {}", quoted(&v))).collect()
        }
        // "scores count alienKills humanKills (client score ping time weapon upgrade)*"
        // -> legacy "scores alienKills humanKills (...)*"
        "scores" => {
            let n = num(1).max(0) as usize;
            let mut s = format!("scores {} {}", num(2), num(3));
            for i in 0..n {
                let b = 4 + i * 6;
                if argv.len() < b + 6 {
                    break;
                }
                s.push_str(&format!(" {} {} {} {} {} {}", num(b), num(b + 1), num(b + 2), num(b + 3), weapon_from_11(num(b + 4)), upgrade_from_11(num(b + 5))));
            }
            vec![s]
        }
        // "tinfo count (client location health armor weapon team|class<<8)*" ->
        // legacy: humans "(client location health weapon upgrade)*", aliens
        // "(client location health class)*" (only team mates are listed)
        "tinfo" => {
            let n = num(1).max(0) as usize;
            let mut s = String::from("tinfo");
            for i in 0..n {
                let b = 2 + i * 6;
                if argv.len() < b + 6 {
                    break;
                }
                let powerups = num(b + 5);
                if powerups & 0xff == Team::Humans as i32 {
                    s.push_str(&format!(" {} {} {} {} {}", num(b), num(b + 1), num(b + 2), weapon_from_11(num(b + 4)), 0));
                } else {
                    s.push_str(&format!(" {} {} {} {}", num(b), num(b + 1), num(b + 2), class_from_11((powerups >> 8) & 0xff)));
                }
            }
            vec![s]
        }
        "servermenu" => {
            let Some(m) = Menu11::from_raw(num(1).clamp(0, 255) as u8) else { return vec![] };
            let (text, wants) = menu_notice_11(m);
            vec![format!("servermenu 0 {} {}", quoted(text), wants as i32)]
        }
        "map_restart" => vec![cmd.to_string(), format!("cs {} \"\"", cs::WINNER)],
        // the ptrc (persistent team reconnection code) dialogue, the client's
        // weapon selection hint and shader remaps have no legacy cgame side
        "weaponswitch" | "ptrcrequest" | "ptrcissue" | "ptrcconfirm" | "remapShader" | "remapshader" => vec![],
        _ => vec![cmd.to_string()],
    }
}

// =====================================================================
// client commands and usercmds
// =====================================================================

/// A legacy client command as the 1.1 server expects it (None: not sent).
/// The commands keep their names (g_cmds.c of both games share them); the
/// item names of class / build / buy / sell / itemact... are the same ids
/// in both tables. Legacy-only commands 1.1 has no handler for are dropped.
pub fn client_command_to_11(cmd: &str) -> Option<String> {
    let argv = tokenize(cmd);
    let c = argv.first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    match c.as_str() {
        // legacy-only commands (g_cmds.c of 1.1 has no handler)
        "say_area" | "vsay" | "vsay_team" | "vsay_local" | "m" | "mt" | "ignore" | "unignore" | "listmaps" | "damage" | "test" => None,
        // "team spectate|aliens|humans|auto": 1.1 also says "spectate"
        "team" => Some(match argv.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("spec") | Some("spectator") | Some("spectators") => "team spectate".to_string(),
            _ => cmd.to_string(),
        }),
        // "deconstruct marked" is legacy's g_markDeconstruct form
        "deconstruct" => Some("deconstruct".to_string()),
        _ => Some(cmd.to_string()),
    }
}

/// A legacy usercmd as the 1.1 server expects it: the buttons 1.1 does not
/// have (BUTTON_DODGE, BUTTON_SPRINT: 1.1's BUTTON_NEGATIVE / GUARDBASE)
/// cleared, BUTTON_USE_EVOLVE is 1.1's BUTTON_GETFLAG (the "use" button),
/// and the weapon byte renumbered (> 32 selects upgrade weapon - 32).
pub fn usercmd_to_11(c: &UserCmd) -> UserCmd {
    let mut u = *c;
    u.buttons = c.buttons & !(k::BUTTON_DODGE | k::BUTTON_SPRINT);
    if c.buttons & k::BUTTON_USE_EVOLVE != 0 {
        u.buttons |= k11::BUTTON_GETFLAG;
    }
    let w = c.weapon as i32;
    u.weapon = if w > 32 { (32 + upgrade_to_11(w - 32)) as u8 } else { weapon_to_11(w) as u8 };
    u
}

/// 1.1.0 sprints by the "boost" client command (Cmd_Boost_f sets
/// SS_SPEEDBOOST; g_active.c clears it when the player stops running) where
/// legacy holds BUTTON_SPRINT: the command to send for a usercmd of a
/// protocol 69 game whose sprint button just went down.
pub fn boost_command(previous_buttons: i32, buttons: i32) -> Option<&'static str> {
    (buttons & k::BUTTON_SPRINT != 0 && previous_buttons & k::BUTTON_SPRINT == 0).then_some("boost")
}
