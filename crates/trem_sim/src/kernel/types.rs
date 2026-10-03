//! Kernel glue `types` — playerState_t, usercmd_t, pmoveExt_t and the index
//! helpers over the generated enums (bg_public.h / q_shared.h shapes).

use super::qmath::Vec3;
pub use crate::gen::constants as k;
use crate::gen::enums::{Pers, Stat};

pub const MAX_STATS: usize = k::MAX_STATS as usize;
pub const MAX_PERSISTANT: usize = k::MAX_PERSISTANT as usize;
pub const MAX_PS_EVENTS: usize = k::MAX_PS_EVENTS as usize;
pub const ENTITYNUM_NONE: i32 = k::ENTITYNUM_NONE;
pub const ENTITYNUM_WORLD: i32 = k::ENTITYNUM_WORLD;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerState {
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
    pub taunt_timer: i32,
    pub weapon_anim: i32,
    pub movement_dir: i32,
    pub grapple_point: Vec3,
    pub e_flags: i32,
    pub event_sequence: i32,
    pub events: [i32; MAX_PS_EVENTS],
    pub event_parms: [i32; MAX_PS_EVENTS],
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
    pub stats: [i32; MAX_STATS],
    pub persistant: [i32; MAX_PERSISTANT],
    pub misc: [i32; 16],
    pub ammo: i32,
    pub clips: i32,
    pub generic1: i32,
    pub loop_sound: i32,
    pub other_entity_num: i32,
    pub ping: i32,
    pub pmove_framecount: i32,
    pub jumppad_frame: i32,
    pub entity_event_sequence: i32,
}

impl PlayerState {
    #[inline]
    pub fn stat(&self, s: Stat) -> i32 {
        self.stats[s as usize]
    }
    #[inline]
    pub fn stat_mut(&mut self, s: Stat) -> &mut i32 {
        &mut self.stats[s as usize]
    }
    #[inline]
    pub fn pers(&self, p: Pers) -> i32 {
        self.persistant[p as usize]
    }
    #[inline]
    pub fn pers_mut(&mut self, p: Pers) -> &mut i32 {
        &mut self.persistant[p as usize]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserCmd {
    pub server_time: i32,
    pub angles: [i32; 3],
    pub buttons: i32,
    pub weapon: u8,
    pub forwardmove: i8,
    pub rightmove: i8,
    pub upmove: i8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PmoveExt {
    pub pounce_payload: i32,
    pub fall_velocity: f32,
}

/// World callbacks of `pmove_t` (trace / pointcontents). Generic, not `dyn`:
/// the server and the parity harness monomorphise their own copy (MDD §9.3).
pub trait PmWorld {
    fn trace(&self, start: &Vec3, mins: Option<&Vec3>, maxs: Option<&Vec3>, end: &Vec3, pass_entity: i32, mask: i32) -> super::cm::Trace;
    fn point_contents(&self, p: &Vec3, pass_entity: i32) -> i32;
}

/// `BG_AddPredictableEventToPlayerstate`.
pub fn add_predictable_event(ps: &mut PlayerState, ev: i32, parm: i32) {
    let i = (ps.event_sequence & (MAX_PS_EVENTS as i32 - 1)) as usize;
    ps.events[i] = ev;
    ps.event_parms[i] = parm;
    ps.event_sequence += 1;
}
