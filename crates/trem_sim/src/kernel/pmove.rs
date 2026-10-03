//! Kernel `pmove` — port of game/bg_pmove.c (Tremulous 1.2 movement).
//!
//! Function names mirror the original (`pm_walk_move` = PM_WalkMove) so the
//! symbols sheet maps 1:1. Arithmetic follows C promotion rules exactly
//! (see qmath.rs); the golden-trace harness (re/harness/pmove_ref.c) is the
//! judge of that claim.

use super::bg;
use super::cm::Trace;
use super::qmath::*;
use super::rng::{Rng, RAND_MAX};
use super::types::*;
use crate::gen::enums::{Event, NonSegAnim as Nspa, Pers, PlayerAnim as A, PmType, SpectatorState, Stat, Team, WeaponAnim as Wa, WeaponMode, WeaponState};
use crate::gen::enums::{ability, contents, surface};
use crate::gen::upgrades::UpgradeId;
use crate::gen::weapons::WeaponId;

pub const MIN_WALK_NORMAL: f32 = 0.7;
pub const STEPSIZE: i32 = 18;
pub const TIMER_LAND: i32 = 130;
pub const TIMER_GESTURE: i32 = 34 * 66 + 50;
pub const TIMER_ATTACK: i32 = 500;
pub const OVERCLIP: f32 = 1.001;
pub const FALLING_THRESHOLD: f32 = -900.0;
pub const ANIM_TOGGLEBIT: i32 = 0x80;
pub const MAXTOUCH: usize = 32;

pub const PM_STOPSPEED: f32 = 100.0;
pub const PM_DUCK_SCALE: f32 = 0.25;
pub const PM_SWIM_SCALE: f32 = 0.50;
pub const PM_ACCELERATE: f32 = 10.0;
pub const PM_WATERACCELERATE: f32 = 4.0;
pub const PM_FLYACCELERATE: f32 = 4.0;
pub const PM_FRICTION: f32 = 6.0;
pub const PM_WATERFRICTION: f32 = 1.0;
pub const PM_FLIGHTFRICTION: f32 = 6.0;
pub const PM_SPECTATORFRICTION: f32 = 5.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Pml {
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub frametime: f32,
    pub msec: i32,
    pub walking: bool,
    pub ground_plane: bool,
    pub ladder: bool,
    pub ground_trace: Trace,
    pub impact_speed: f32,
    pub previous_origin: Vec3,
    pub previous_velocity: Vec3,
    pub previous_waterlevel: i32,
}

pub struct Pmove<'a, W: PmWorld> {
    pub ps: &'a mut PlayerState,
    pub pmext: &'a mut PmoveExt,
    pub cmd: UserCmd,
    pub tracemask: i32,
    pub debug_level: i32,
    pub no_footsteps: bool,
    pub auto_weapon_hit: [bool; 32],
    pub framecount: i32,
    pub numtouch: usize,
    pub touchents: [i32; MAXTOUCH],
    pub mins: Vec3,
    pub maxs: Vec3,
    pub watertype: i32,
    pub waterlevel: i32,
    pub xyspeed: f32,
    pub pmove_fixed: bool,
    pub pmove_msec: i32,
    pub world: &'a W,
    pub rng: &'a mut Rng,
    pub pml: Pml,
}

const ST_STATE: usize = Stat::State as usize;
const ST_MISC: usize = Stat::Misc as usize;
const ST_CLASS: usize = Stat::Class as usize;
const ST_TEAM: usize = Stat::Team as usize;
const ST_STAMINA: usize = Stat::Stamina as usize;
const ST_HEALTH: usize = Stat::Health as usize;
const P_STATE: usize = Pers::State as usize;

impl<'a, W: PmWorld> Pmove<'a, W> {
    pub fn new(ps: &'a mut PlayerState, pmext: &'a mut PmoveExt, cmd: UserCmd, world: &'a W, rng: &'a mut Rng) -> Self {
        Pmove {
            ps,
            pmext,
            cmd,
            tracemask: k::MASK_PLAYERSOLID,
            debug_level: 0,
            no_footsteps: false,
            auto_weapon_hit: [false; 32],
            framecount: 0,
            numtouch: 0,
            touchents: [0; MAXTOUCH],
            mins: [0.0; 3],
            maxs: [0.0; 3],
            watertype: 0,
            waterlevel: 0,
            xyspeed: 0.0,
            pmove_fixed: false,
            pmove_msec: 8,
            world,
            rng,
            pml: Pml::default(),
        }
    }

    // ------------------------------------------------------------- helpers
    #[inline]
    fn trace(&self, start: &Vec3, end: &Vec3, mask: i32) -> Trace {
        self.world.trace(start, Some(&self.mins), Some(&self.maxs), end, self.ps.client_num, mask)
    }
    #[inline]
    fn class(&self) -> &'static crate::gen::classes::ClassDef {
        bg::class(self.ps.stats[ST_CLASS])
    }
    #[inline]
    fn has_ability(&self, a: u32) -> bool {
        bg::class_has_ability(self.ps.stats[ST_CLASS], a)
    }
    #[inline]
    fn nonseg(&self) -> bool {
        self.ps.persistant[P_STATE] & k::PS_NONSEGMODEL != 0
    }
    #[inline]
    fn wp(&self, w: WeaponId) -> bool {
        self.ps.weapon == w as i32
    }

    pub fn pm_add_event(&mut self, ev: Event) {
        add_predictable_event(self.ps, ev as i32, 0);
    }
    fn pm_add_event_raw(&mut self, ev: i32) {
        add_predictable_event(self.ps, ev, 0);
    }

    pub fn pm_add_touch_ent(&mut self, ent: i32) {
        if ent == ENTITYNUM_WORLD || self.numtouch == MAXTOUCH {
            return;
        }
        if self.touchents[..self.numtouch].contains(&ent) {
            return;
        }
        self.touchents[self.numtouch] = ent;
        self.numtouch += 1;
    }

    fn pm_start_torso_anim(&mut self, anim: i32) {
        if bg::pm_paralyzed(self.ps.pm_type) {
            return;
        }
        self.ps.torso_anim = ((self.ps.torso_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | anim;
    }
    fn pm_start_weapon_anim(&mut self, anim: i32) {
        if bg::pm_paralyzed(self.ps.pm_type) {
            return;
        }
        self.ps.weapon_anim = ((self.ps.weapon_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | anim;
    }
    fn pm_start_legs_anim(&mut self, anim: i32) {
        if bg::pm_paralyzed(self.ps.pm_type) {
            return;
        }
        if !self.nonseg() {
            if self.ps.legs_timer > 0 {
                return;
            }
        } else if self.ps.torso_timer > 0 {
            return;
        }
        self.ps.legs_anim = ((self.ps.legs_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | anim;
    }
    fn pm_continue_legs_anim(&mut self, anim: i32) {
        if (self.ps.legs_anim & !ANIM_TOGGLEBIT) == anim {
            return;
        }
        if !self.nonseg() {
            if self.ps.legs_timer > 0 {
                return;
            }
        } else if self.ps.torso_timer > 0 {
            return;
        }
        self.pm_start_legs_anim(anim);
    }
    fn pm_continue_torso_anim(&mut self, anim: i32) {
        if (self.ps.torso_anim & !ANIM_TOGGLEBIT) == anim {
            return;
        }
        if self.ps.torso_timer > 0 {
            return;
        }
        self.pm_start_torso_anim(anim);
    }
    fn pm_continue_weapon_anim(&mut self, anim: i32) {
        if (self.ps.weapon_anim & !ANIM_TOGGLEBIT) == anim {
            return;
        }
        self.pm_start_weapon_anim(anim);
    }
    fn pm_force_legs_anim(&mut self, anim: i32) {
        if !self.nonseg() {
            self.ps.legs_timer = 0;
        } else {
            self.ps.torso_timer = 0;
        }
        self.pm_start_legs_anim(anim);
    }
    /// The forward/back jump animation pattern repeated throughout the original.
    fn pm_jump_anim(&mut self) {
        if self.cmd.forwardmove >= 0 {
            let a = if !self.nonseg() { A::LegsJump as i32 } else { Nspa::Jump as i32 };
            self.pm_force_legs_anim(a);
            self.ps.pm_flags &= !k::PMF_BACKWARDS_JUMP;
        } else {
            let a = if !self.nonseg() { A::LegsJumpb as i32 } else { Nspa::Jumpback as i32 };
            self.pm_force_legs_anim(a);
            self.ps.pm_flags |= k::PMF_BACKWARDS_JUMP;
        }
    }

    /// `PM_ClipVelocity`.
    pub fn clip_velocity(input: &Vec3, normal: &Vec3, overbounce: f32) -> Vec3 {
        let mut backoff = dot(input, normal);
        if backoff < 0.0 {
            backoff *= overbounce;
        } else {
            backoff /= overbounce;
        }
        let mut out = [0.0f32; 3];
        for i in 0..3 {
            let change = normal[i] * backoff;
            out[i] = input[i] - change;
        }
        out
    }

    /// `PM_Friction`.
    fn pm_friction(&mut self) {
        let mut vec = self.ps.velocity;
        if self.pml.walking && self.ps.stats[ST_STATE] & k::SS_WALLCLIMBING == 0 {
            vec[2] = 0.0;
        }
        let speed = length(&vec);
        if speed < 1.0 {
            self.ps.velocity[0] = 0.0;
            self.ps.velocity[1] = 0.0;
            return;
        }
        let mut drop = 0.0f32;
        if self.waterlevel <= 1
            && (self.pml.walking || self.pml.ladder)
            && self.pml.ground_trace.surface_flags & surface::SLICK as i32 == 0
            && self.ps.pm_flags & k::PMF_TIME_KNOCKBACK == 0
        {
            let c = self.class();
            let control = if speed < c.stop_speed { c.stop_speed } else { speed };
            drop += control * c.friction * self.pml.frametime;
        }
        if self.waterlevel != 0 {
            drop += speed * PM_WATERFRICTION * self.waterlevel as f32 * self.pml.frametime;
        }
        if self.ps.pm_type == PmType::Jetpack as i32 {
            drop += speed * PM_FLIGHTFRICTION * self.pml.frametime;
        }
        if self.ps.pm_type == PmType::Spectator as i32 {
            drop += speed * PM_SPECTATORFRICTION * self.pml.frametime;
        }
        let mut newspeed = speed - drop;
        if newspeed < 0.0 {
            newspeed = 0.0;
        }
        newspeed /= speed;
        self.ps.velocity[0] *= newspeed;
        self.ps.velocity[1] *= newspeed;
        self.ps.velocity[2] *= newspeed;
    }

    /// `PM_Accelerate` (q2 style).
    fn pm_accelerate(&mut self, wishdir: &Vec3, wishspeed: f32, accel: f32) {
        let currentspeed = dot(&self.ps.velocity, wishdir);
        let addspeed = wishspeed - currentspeed;
        if addspeed <= 0.0 {
            return;
        }
        let mut accelspeed = accel * self.pml.frametime * wishspeed;
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        for i in 0..3 {
            self.ps.velocity[i] += accelspeed * wishdir[i];
        }
    }

    /// `PM_CmdScale` (mutates `cmd` like the original's pointer argument).
    fn pm_cmd_scale(&mut self, cmd: &mut UserCmd) -> f32 {
        let mut modifier: f32 = 1.0;
        if self.ps.stats[ST_TEAM] == Team::Humans as i32 && self.ps.pm_type == PmType::Normal as i32 {
            let was_sprinting = self.ps.stats[ST_STATE] & k::SS_SPEEDBOOST != 0;
            let mut sprint = was_sprinting;
            if self.ps.persistant[P_STATE] & k::PS_SPRINTTOGGLE != 0 {
                if cmd.buttons & k::BUTTON_SPRINT != 0 && self.ps.pm_flags & k::PMF_SPRINTHELD == 0 {
                    sprint = !sprint;
                    self.ps.pm_flags |= k::PMF_SPRINTHELD;
                } else if self.ps.pm_flags & k::PMF_SPRINTHELD != 0 && cmd.buttons & k::BUTTON_SPRINT == 0 {
                    self.ps.pm_flags &= !k::PMF_SPRINTHELD;
                }
            } else {
                sprint = cmd.buttons & k::BUTTON_SPRINT != 0;
            }
            if sprint {
                self.ps.stats[ST_STATE] |= k::SS_SPEEDBOOST;
            } else if was_sprinting && !sprint {
                self.ps.stats[ST_STATE] &= !k::SS_SPEEDBOOST;
            }
            if sprint && cmd.buttons & k::BUTTON_WALKING == 0 {
                modifier *= k::HUMAN_SPRINT_MODIFIER;
            } else {
                modifier *= k::HUMAN_JOG_MODIFIER;
            }
            if cmd.forwardmove < 0 {
                modifier *= k::HUMAN_BACK_MODIFIER;
            } else if cmd.rightmove != 0 {
                modifier *= k::HUMAN_SIDE_MODIFIER;
            }
            if self.ps.stats[ST_STAMINA] < k::STAMINA_SLOW_LEVEL + k::STAMINA_JUMP_TAKE {
                cmd.upmove = 0;
            }
            if self.ps.stats[ST_STAMINA] <= k::STAMINA_SLOW_LEVEL {
                modifier *= (self.ps.stats[ST_STAMINA] + k::STAMINA_MAX) as f32 / (k::STAMINA_SLOW_LEVEL + k::STAMINA_MAX) as f32;
            }
            let armoured = bg::inventory_contains_upgrade(UpgradeId::Larmour as i32, &self.ps.stats) || bg::inventory_contains_upgrade(UpgradeId::Bsuit as i32, &self.ps.stats);
            if self.ps.stats[ST_STATE] & k::SS_CREEPSLOWED != 0 {
                modifier *= if armoured { k::CREEP_ARMOUR_MODIFIER } else { k::CREEP_MODIFIER };
            }
            if self.ps.e_flags & k::EF_POISONCLOUDED != 0 {
                modifier *= if armoured { k::PCLOUD_ARMOUR_MODIFIER } else { k::PCLOUD_MODIFIER };
            }
        }
        if self.wp(WeaponId::Level4) && self.ps.pm_flags & k::PMF_CHARGE != 0 {
            modifier *= 1.0f32 + (self.ps.stats[ST_MISC] as f32 * (k::LEVEL4_TRAMPLE_SPEED - 1.0f32) / k::LEVEL4_TRAMPLE_DURATION as f32);
        }
        if (self.wp(WeaponId::Level3) || self.wp(WeaponId::Level3upg)) && cmd.buttons & k::BUTTON_ATTACK2 != 0 {
            modifier *= k::LEVEL3_POUNCE_SPEED_MOD;
        }
        if self.ps.stats[ST_STATE] & k::SS_SLOWLOCKED != 0 {
            modifier *= k::ABUILDER_BLOB_SPEED_MOD;
        }
        if self.ps.pm_type == PmType::Grabbed as i32 {
            modifier = 0.0;
        }
        if self.ps.pm_type != PmType::Spectator as i32 && self.ps.pm_type != PmType::Noclip as i32 {
            if self.class().jump_magnitude == 0.0 {
                cmd.upmove = 0;
            }
            if self.ps.pm_flags & k::PMF_DUCKED == 0 && self.ps.pm_type != PmType::Jetpack as i32 && cmd.upmove < 0 {
                cmd.upmove = 0;
            }
        }
        let fm = cmd.forwardmove as i32;
        let rm = cmd.rightmove as i32;
        let um = cmd.upmove as i32;
        let mut max = fm.abs();
        if rm.abs() > max {
            max = rm.abs();
        }
        if um.abs() > max {
            max = um.abs();
        }
        if max == 0 {
            return 0.0;
        }
        let total = ((fm * fm + rm * rm + um * um) as f64).sqrt() as f32;
        ((self.ps.speed as f32 * max as f32) as f64 / (127.0 * total as f64) * modifier as f64) as f32
    }

    /// `PM_SetMovementDir`.
    fn pm_set_movement_dir(&mut self) {
        let (f, r) = (self.cmd.forwardmove, self.cmd.rightmove);
        if f != 0 || r != 0 {
            self.ps.movement_dir = match (r, f) {
                (0, f) if f > 0 => 0,
                (r, f) if r < 0 && f > 0 => 1,
                (r, 0) if r < 0 => 2,
                (r, f) if r < 0 && f < 0 => 3,
                (0, f) if f < 0 => 4,
                (r, f) if r > 0 && f < 0 => 5,
                (r, 0) if r > 0 => 6,
                (r, f) if r > 0 && f > 0 => 7,
                _ => self.ps.movement_dir,
            };
        } else if self.ps.movement_dir == 2 {
            self.ps.movement_dir = 1;
        } else if self.ps.movement_dir == 6 {
            self.ps.movement_dir = 7;
        }
    }

    /// `PM_CheckCharge`.
    fn pm_check_charge(&mut self) {
        if !self.wp(WeaponId::Level4) {
            return;
        }
        if self.cmd.buttons & k::BUTTON_ATTACK2 != 0 && self.ps.stats[ST_STATE] & k::SS_CHARGING == 0 {
            self.ps.pm_flags &= !k::PMF_CHARGE;
            return;
        }
        if self.ps.stats[ST_MISC] > 0 {
            self.ps.pm_flags |= k::PMF_CHARGE;
        } else {
            self.ps.pm_flags &= !k::PMF_CHARGE;
        }
    }

    /// `PM_CheckPounce`.
    fn pm_check_pounce(&mut self) -> bool {
        if !self.wp(WeaponId::Level3) && !self.wp(WeaponId::Level3upg) {
            return false;
        }
        if self.ps.ground_entity_num != ENTITYNUM_NONE && self.ps.pm_flags & k::PMF_CHARGE != 0 {
            self.ps.pm_flags &= !k::PMF_CHARGE;
            self.ps.weapon_time += k::LEVEL3_POUNCE_REPEAT;
            return false;
        }
        if self.cmd.buttons & k::BUTTON_ATTACK2 != 0 {
            self.ps.pm_flags &= !k::PMF_CHARGE;
            return false;
        }
        if self.ps.pm_flags & k::PMF_CHARGE != 0 || self.ps.stats[ST_MISC] < k::LEVEL3_POUNCE_TIME_MIN || self.ps.ground_entity_num == ENTITYNUM_NONE {
            return false;
        }
        self.pml.ground_plane = false;
        self.pml.walking = false;
        self.ps.pm_flags |= k::PMF_CHARGE;
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        let jump_magnitude = if self.wp(WeaponId::Level3) {
            self.ps.stats[ST_MISC] * k::LEVEL3_POUNCE_JUMP_MAG / k::LEVEL3_POUNCE_TIME
        } else {
            self.ps.stats[ST_MISC] * k::LEVEL3_POUNCE_JUMP_MAG_UPG / k::LEVEL3_POUNCE_TIME_UPG
        };
        self.ps.velocity = ma(&self.ps.velocity, jump_magnitude as f32, &self.pml.forward);
        self.pm_add_event(Event::Jump);
        self.pm_jump_anim();
        self.pmext.pounce_payload = self.ps.stats[ST_MISC];
        self.ps.stats[ST_MISC] = 0;
        true
    }

    /// `PM_CheckWallJump`.
    fn pm_check_wall_jump(&mut self) -> bool {
        let ref_normal: Vec3 = [0.0, 0.0, 1.0];
        let normal_fraction: f32 = 1.5;
        let cmd_fraction: f32 = 1.0;
        let up_fraction: f32 = 1.5;
        if !self.has_ability(ability::WALLJUMPER) {
            return false;
        }
        let mut movedir = project_point_on_plane(&self.pml.forward, &ref_normal);
        normalize(&mut movedir);
        if self.cmd.forwardmove < 0 {
            movedir = neg(&movedir);
        }
        if self.cmd.rightmove != 0 {
            movedir = self.pml.right;
            if self.cmd.rightmove < 0 {
                movedir = neg(&movedir);
            }
        }
        let point = ma(&self.ps.origin, 0.25, &movedir);
        let trace = self.trace(&self.ps.origin.clone(), &point, self.tracemask);
        if trace.fraction < 1.0 && trace.surface_flags & (surface::SKY | surface::SLICK) as i32 == 0 && trace.plane.normal[2] < MIN_WALK_NORMAL {
            if !compare(&trace.plane.normal, &self.ps.grapple_point) {
                self.ps.grapple_point = trace.plane.normal;
            }
        } else {
            return false;
        }
        if self.ps.pm_flags & k::PMF_RESPAWNED != 0 {
            return false;
        }
        if self.cmd.upmove < 10 {
            return false;
        }
        if self.ps.pm_flags & k::PMF_TIME_WALLJUMP != 0 {
            return false;
        }
        if self.ps.pm_flags & k::PMF_JUMP_HELD != 0 && self.ps.grapple_point[2] == 1.0 {
            self.cmd.upmove = 0;
            return false;
        }
        self.ps.pm_flags |= k::PMF_TIME_WALLJUMP;
        self.ps.pm_time = 200;
        self.pml.ground_plane = false;
        self.pml.walking = false;
        self.ps.pm_flags |= k::PMF_JUMP_HELD;
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        let forward = project_point_on_plane(&self.pml.forward, &self.ps.grapple_point);
        let right = project_point_on_plane(&self.pml.right, &self.ps.grapple_point);
        let mut dir = scale(&self.ps.grapple_point, normal_fraction);
        if self.cmd.forwardmove > 0 {
            dir = ma(&dir, cmd_fraction, &forward);
        } else if self.cmd.forwardmove < 0 {
            dir = ma(&dir, -cmd_fraction, &forward);
        }
        if self.cmd.rightmove > 0 {
            dir = ma(&dir, cmd_fraction, &right);
        } else if self.cmd.rightmove < 0 {
            dir = ma(&dir, -cmd_fraction, &right);
        }
        dir = ma(&dir, up_fraction, &ref_normal);
        normalize(&mut dir);
        self.ps.velocity = ma(&self.ps.velocity, self.class().jump_magnitude, &dir);
        if length(&self.ps.velocity) > k::LEVEL2_WALLJUMP_MAXSPEED {
            normalize(&mut self.ps.velocity);
            self.ps.velocity = scale(&self.ps.velocity, k::LEVEL2_WALLJUMP_MAXSPEED);
        }
        self.pm_add_event(Event::Jump);
        self.pm_jump_anim();
        true
    }

    /// `PM_CheckJump`.
    fn pm_check_jump(&mut self) -> bool {
        if self.ps.ground_entity_num == ENTITYNUM_NONE {
            return false;
        }
        if self.class().jump_magnitude == 0.0 {
            return false;
        }
        if (self.wp(WeaponId::Level3) || self.wp(WeaponId::Level3upg)) && self.ps.stats[ST_MISC] > 0 {
            return false;
        }
        if self.wp(WeaponId::Level4) && self.ps.stats[ST_MISC] > 0 {
            return false;
        }
        let human = self.ps.stats[ST_TEAM] == Team::Humans as i32;
        if human && self.ps.stats[ST_STAMINA] < k::STAMINA_SLOW_LEVEL + k::STAMINA_JUMP_TAKE {
            return false;
        }
        if human && self.ps.pm_time != 0 {
            return false;
        }
        if self.ps.pm_flags & k::PMF_RESPAWNED != 0 {
            return false;
        }
        if self.cmd.upmove < 10 {
            return false;
        }
        if self.ps.pm_type == PmType::Grabbed as i32 {
            self.cmd.upmove = 0;
            return false;
        }
        if self.ps.pm_flags & k::PMF_JUMP_HELD != 0 {
            self.cmd.upmove = 0;
            return false;
        }
        if self.has_ability(ability::WALLJUMPER) {
            self.ps.pm_flags |= k::PMF_TIME_WALLJUMP;
            self.ps.pm_time = 200;
        }
        self.pml.ground_plane = false;
        self.pml.walking = false;
        self.ps.pm_flags |= k::PMF_JUMP_HELD;
        if human {
            self.ps.stats[ST_STAMINA] -= k::STAMINA_JUMP_TAKE;
        }
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        let normal = bg::client_normal(self.ps);
        if self.ps.velocity[2] < 0.0 {
            self.ps.velocity[2] = 0.0;
        }
        self.ps.velocity = ma(&self.ps.velocity, self.class().jump_magnitude, &normal);
        self.pm_add_event(Event::Jump);
        self.pm_jump_anim();
        true
    }

    /// `PM_CheckWaterJump`.
    fn pm_check_water_jump(&mut self) -> bool {
        if self.ps.pm_time != 0 {
            return false;
        }
        if self.waterlevel != 2 {
            return false;
        }
        let mut flatforward = [self.pml.forward[0], self.pml.forward[1], 0.0];
        normalize(&mut flatforward);
        let mut spot = ma(&self.ps.origin, 30.0, &flatforward);
        spot[2] += 4.0;
        let cont = self.world.point_contents(&spot, self.ps.client_num);
        if cont & contents::SOLID as i32 == 0 {
            return false;
        }
        spot[2] += 16.0;
        let cont = self.world.point_contents(&spot, self.ps.client_num);
        if cont != 0 {
            return false;
        }
        self.ps.velocity = scale(&self.pml.forward, 200.0);
        self.ps.velocity[2] = 350.0;
        self.ps.pm_flags |= k::PMF_TIME_WATERJUMP;
        self.ps.pm_time = 2000;
        true
    }

    /// `PM_CheckDodge`.
    fn pm_check_dodge(&mut self) -> bool {
        if self.ps.stats[ST_TEAM] != Team::Humans as i32 {
            return false;
        }
        if self.ps.pm_flags & k::PMF_CHARGE != 0 && self.ps.ground_entity_num != ENTITYNUM_NONE {
            self.ps.pm_flags = (self.ps.pm_flags & !k::PMF_CHARGE) | k::PMF_TIME_LAND;
            self.ps.pm_time = k::HUMAN_DODGE_TIMEOUT;
        }
        if self.ps.pm_type != PmType::Normal as i32 || self.ps.stats[ST_STAMINA] < k::STAMINA_SLOW_LEVEL + k::STAMINA_DODGE_TAKE || self.ps.pm_flags & k::PMF_DUCKED != 0 {
            return false;
        }
        if self.cmd.forwardmove > 0 {
            return false;
        }
        if self.ps.pm_flags & (k::PMF_TIME_LAND | k::PMF_CHARGE) != 0 || self.ps.ground_entity_num == ENTITYNUM_NONE || self.cmd.buttons & k::BUTTON_DODGE == 0 {
            return false;
        }
        if (self.cmd.rightmove == 0 && self.cmd.forwardmove == 0) || self.cmd.upmove != 0 {
            return false;
        }
        let (_, mut right, _) = angle_vectors(&self.ps.viewangles);
        let mut forward = [-right[1], right[0], 0.0];
        let mut jump = self.class().jump_magnitude;
        if self.cmd.rightmove != 0 && self.cmd.forwardmove != 0 {
            jump = (jump as f64 * (0.5f32 as f64 * std::f64::consts::SQRT_2)) as f32;
        }
        let side_modifier = if self.ps.stats[ST_STATE] & k::SS_SLOWLOCKED != 0 || self.ps.stats[ST_STATE] & k::SS_CREEPSLOWED != 0 || self.ps.e_flags & k::EF_POISONCLOUDED != 0 {
            k::HUMAN_DODGE_SLOWED_MODIFIER
        } else {
            k::HUMAN_DODGE_SIDE_MODIFIER
        };
        let mut velocity = [0.0f32; 3];
        if self.cmd.rightmove != 0 {
            if self.cmd.rightmove < 0 {
                right = neg(&right);
            }
            velocity = ma(&velocity, jump * side_modifier, &right);
        }
        if self.cmd.forwardmove != 0 {
            if self.cmd.forwardmove < 0 {
                forward = neg(&forward);
            }
            velocity = ma(&velocity, jump * side_modifier, &forward);
        }
        velocity[2] = jump * k::HUMAN_DODGE_UP_MODIFIER;
        for i in 0..3 {
            if (velocity[i] < 0.0 && self.ps.velocity[i] > velocity[i]) || (velocity[i] > 0.0 && self.ps.velocity[i] < velocity[i]) {
                self.ps.velocity[i] = velocity[i];
            }
        }
        self.pml.ground_plane = false;
        self.pml.walking = false;
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        self.ps.pm_flags |= k::PMF_CHARGE;
        self.ps.stats[ST_STAMINA] -= k::STAMINA_DODGE_TAKE;
        self.ps.legs_anim = ((self.ps.legs_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | A::LegsJump as i32;
        self.pm_add_event(Event::Jump);
        true
    }

    /// `PM_WaterJumpMove`.
    fn pm_water_jump_move(&mut self) {
        self.pm_step_slide_move(true, false);
        self.ps.velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
        if self.ps.velocity[2] < 0.0 {
            self.ps.pm_flags &= !k::PMF_ALL_TIMES;
            self.ps.pm_time = 0;
        }
    }

    /// `PM_WaterMove`.
    fn pm_water_move(&mut self) {
        if self.pm_check_water_jump() {
            self.pm_water_jump_move();
            return;
        }
        self.pm_friction();
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.cmd = cmd;
        let mut wishvel = [0.0f32; 3];
        if scale_ == 0.0 {
            wishvel = [0.0, 0.0, -60.0];
        } else {
            for i in 0..3 {
                wishvel[i] = scale_ * self.pml.forward[i] * self.cmd.forwardmove as f32 + scale_ * self.pml.right[i] * self.cmd.rightmove as f32;
            }
            wishvel[2] += scale_ * self.cmd.upmove as f32;
        }
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        if wishspeed > self.ps.speed as f32 * PM_SWIM_SCALE {
            wishspeed = self.ps.speed as f32 * PM_SWIM_SCALE;
        }
        self.pm_accelerate(&wishdir, wishspeed, PM_WATERACCELERATE);
        if self.pml.ground_plane && dot(&self.ps.velocity, &self.pml.ground_trace.plane.normal) < 0.0 {
            let vel = length(&self.ps.velocity);
            self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &self.pml.ground_trace.plane.normal, OVERCLIP);
            normalize(&mut self.ps.velocity);
            self.ps.velocity = scale(&self.ps.velocity, vel);
        }
        self.pm_slide_move(false);
    }

    /// `PM_JetPackMove`.
    fn pm_jet_pack_move(&mut self) {
        self.pm_friction();
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.cmd = cmd;
        let mut wishvel = [0.0f32; 3];
        for i in 0..2 {
            wishvel[i] = scale_ * self.pml.forward[i] * self.cmd.forwardmove as f32 + scale_ * self.pml.right[i] * self.cmd.rightmove as f32;
        }
        wishvel[2] = if (self.cmd.upmove as f32) > 0.0 {
            k::JETPACK_FLOAT_SPEED
        } else if (self.cmd.upmove as f32) < 0.0 {
            -k::JETPACK_SINK_SPEED
        } else {
            0.0
        };
        let mut wishdir = wishvel;
        let wishspeed = normalize(&mut wishdir);
        self.pm_accelerate(&wishdir, wishspeed, PM_FLYACCELERATE);
        self.pm_step_slide_move(false, false);
        let a = if !self.nonseg() { A::LegsLand as i32 } else { Nspa::Land as i32 };
        self.pm_continue_legs_anim(a);
    }

    /// `PM_FlyMove`.
    fn pm_fly_move(&mut self) {
        self.pm_friction();
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.cmd = cmd;
        let mut wishvel = [0.0f32; 3];
        if scale_ != 0.0 {
            for i in 0..3 {
                wishvel[i] = scale_ * self.pml.forward[i] * self.cmd.forwardmove as f32 + scale_ * self.pml.right[i] * self.cmd.rightmove as f32;
            }
            wishvel[2] += scale_ * self.cmd.upmove as f32;
        }
        let mut wishdir = wishvel;
        let wishspeed = normalize(&mut wishdir);
        self.pm_accelerate(&wishdir, wishspeed, PM_FLYACCELERATE);
        self.pm_step_slide_move(false, false);
    }

    /// `PM_AirMove`.
    fn pm_air_move(&mut self) {
        self.pm_check_wall_jump();
        self.pm_friction();
        let fmove = self.cmd.forwardmove as f32;
        let smove = self.cmd.rightmove as f32;
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.pm_set_movement_dir();
        self.pml.forward[2] = 0.0;
        self.pml.right[2] = 0.0;
        normalize(&mut self.pml.forward);
        normalize(&mut self.pml.right);
        let mut wishvel = [0.0f32; 3];
        for i in 0..2 {
            wishvel[i] = self.pml.forward[i] * fmove + self.pml.right[i] * smove;
        }
        wishvel[2] = 0.0;
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        wishspeed *= scale_;
        let aa = self.class().air_acceleration;
        self.pm_accelerate(&wishdir, wishspeed, aa);
        if self.pml.ground_plane {
            self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &self.pml.ground_trace.plane.normal, OVERCLIP);
        }
        self.pm_step_slide_move(true, false);
    }

    fn water_scale_clamp(&self, wishspeed: &mut f32) {
        if self.waterlevel != 0 {
            let mut water_scale = (self.waterlevel as f64 / 3.0) as f32;
            water_scale = (1.0 - (1.0 - PM_SWIM_SCALE as f64) * water_scale as f64) as f32;
            if *wishspeed > self.ps.speed as f32 * water_scale {
                *wishspeed = self.ps.speed as f32 * water_scale;
            }
        }
    }

    /// `PM_ClimbMove`.
    fn pm_climb_move(&mut self) {
        if self.waterlevel > 2 && dot(&self.pml.forward, &self.pml.ground_trace.plane.normal) > 0.0 {
            self.pm_water_move();
            return;
        }
        if self.pm_check_jump() || self.pm_check_pounce() {
            if self.waterlevel > 1 {
                self.pm_water_move();
            } else {
                self.pm_air_move();
            }
            return;
        }
        self.pm_friction();
        let fmove = self.cmd.forwardmove as f32;
        let smove = self.cmd.rightmove as f32;
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.pm_set_movement_dir();
        let n = self.pml.ground_trace.plane.normal;
        self.pml.forward = Self::clip_velocity(&self.pml.forward, &n, OVERCLIP);
        self.pml.right = Self::clip_velocity(&self.pml.right, &n, OVERCLIP);
        normalize(&mut self.pml.forward);
        normalize(&mut self.pml.right);
        let mut wishvel = [0.0f32; 3];
        for i in 0..3 {
            wishvel[i] = self.pml.forward[i] * fmove + self.pml.right[i] * smove;
        }
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        wishspeed *= scale_;
        if self.ps.pm_flags & k::PMF_DUCKED != 0 && wishspeed > self.ps.speed as f32 * PM_DUCK_SCALE {
            wishspeed = self.ps.speed as f32 * PM_DUCK_SCALE;
        }
        self.water_scale_clamp(&mut wishspeed);
        let slick = self.pml.ground_trace.surface_flags & surface::SLICK as i32 != 0 || self.ps.pm_flags & k::PMF_TIME_KNOCKBACK != 0;
        let accelerate = if slick { self.class().air_acceleration } else { self.class().acceleration };
        self.pm_accelerate(&wishdir, wishspeed, accelerate);
        if slick {
            self.ps.velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
        }
        let vel = length(&self.ps.velocity);
        self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &n, OVERCLIP);
        normalize(&mut self.ps.velocity);
        self.ps.velocity = scale(&self.ps.velocity, vel);
        if self.ps.velocity[0] == 0.0 && self.ps.velocity[1] == 0.0 && self.ps.velocity[2] == 0.0 {
            return;
        }
        self.pm_step_slide_move(false, false);
    }

    /// `PM_WalkMove`.
    fn pm_walk_move(&mut self) {
        if self.waterlevel > 2 && dot(&self.pml.forward, &self.pml.ground_trace.plane.normal) > 0.0 {
            self.pm_water_move();
            return;
        }
        if self.pm_check_jump() || self.pm_check_pounce() {
            if self.waterlevel > 1 {
                self.pm_water_move();
            } else {
                self.pm_air_move();
            }
            return;
        }
        self.pm_check_charge();
        self.pm_friction();
        let fmove = self.cmd.forwardmove as f32;
        let smove = self.cmd.rightmove as f32;
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.pm_set_movement_dir();
        self.pml.forward[2] = 0.0;
        self.pml.right[2] = 0.0;
        let n = self.pml.ground_trace.plane.normal;
        self.pml.forward = Self::clip_velocity(&self.pml.forward, &n, OVERCLIP);
        self.pml.right = Self::clip_velocity(&self.pml.right, &n, OVERCLIP);
        normalize(&mut self.pml.forward);
        normalize(&mut self.pml.right);
        let mut wishvel = [0.0f32; 3];
        for i in 0..3 {
            wishvel[i] = self.pml.forward[i] * fmove + self.pml.right[i] * smove;
        }
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        wishspeed *= scale_;
        if self.ps.pm_flags & k::PMF_DUCKED != 0 && wishspeed > self.ps.speed as f32 * PM_DUCK_SCALE {
            wishspeed = self.ps.speed as f32 * PM_DUCK_SCALE;
        }
        self.water_scale_clamp(&mut wishspeed);
        let slick = self.pml.ground_trace.surface_flags & surface::SLICK as i32 != 0 || self.ps.pm_flags & k::PMF_TIME_KNOCKBACK != 0;
        let accelerate = if slick { self.class().air_acceleration } else { self.class().acceleration };
        self.pm_accelerate(&wishdir, wishspeed, accelerate);
        if slick {
            self.ps.velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
        }
        self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &n, OVERCLIP);
        if self.ps.velocity[0] == 0.0 && self.ps.velocity[1] == 0.0 {
            return;
        }
        self.pm_step_slide_move(false, false);
    }

    /// `PM_LadderMove`.
    fn pm_ladder_move(&mut self) {
        self.pm_friction();
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.cmd = cmd;
        let mut wishvel = [0.0f32; 3];
        for i in 0..3 {
            wishvel[i] = scale_ * self.pml.forward[i] * self.cmd.forwardmove as f32 + scale_ * self.pml.right[i] * self.cmd.rightmove as f32;
        }
        wishvel[2] += scale_ * self.cmd.upmove as f32;
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        if wishspeed > self.ps.speed as f32 * PM_SWIM_SCALE {
            wishspeed = self.ps.speed as f32 * PM_SWIM_SCALE;
        }
        self.pm_accelerate(&wishdir, wishspeed, PM_ACCELERATE);
        if self.pml.ground_plane && dot(&self.ps.velocity, &self.pml.ground_trace.plane.normal) < 0.0 {
            let vel = length(&self.ps.velocity);
            self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &self.pml.ground_trace.plane.normal, OVERCLIP);
            normalize(&mut self.ps.velocity);
            self.ps.velocity = scale(&self.ps.velocity, vel);
        }
        self.pm_slide_move(false);
    }

    /// `PM_CheckLadder`.
    fn pm_check_ladder(&mut self) {
        if !self.has_ability(ability::CANUSELADDERS) {
            self.pml.ladder = false;
            return;
        }
        let mut forward = self.pml.forward;
        forward[2] = 0.0;
        let end = ma(&self.ps.origin, 1.0, &forward);
        let trace = self.trace(&self.ps.origin.clone(), &end, k::MASK_PLAYERSOLID);
        self.pml.ladder = trace.fraction < 1.0 && trace.surface_flags & surface::LADDER as i32 != 0;
    }

    /// `PM_DeadMove`.
    fn pm_dead_move(&mut self) {
        if !self.pml.walking {
            return;
        }
        let mut forward = length(&self.ps.velocity);
        forward -= 20.0;
        if forward <= 0.0 {
            self.ps.velocity = [0.0; 3];
        } else {
            normalize(&mut self.ps.velocity);
            self.ps.velocity = scale(&self.ps.velocity, forward);
        }
    }

    /// `PM_NoclipMove`.
    fn pm_noclip_move(&mut self) {
        let speed = length(&self.ps.velocity);
        if speed < 1.0 {
            self.ps.velocity = VEC3_ORIGIN;
        } else {
            let mut drop = 0.0f32;
            let friction = (PM_FRICTION as f64 * 1.5) as f32;
            let control = if speed < PM_STOPSPEED { PM_STOPSPEED } else { speed };
            drop += control * friction * self.pml.frametime;
            let mut newspeed = speed - drop;
            if newspeed < 0.0 {
                newspeed = 0.0;
            }
            newspeed /= speed;
            self.ps.velocity = scale(&self.ps.velocity, newspeed);
        }
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.cmd = cmd;
        let fmove = self.cmd.forwardmove as f32;
        let smove = self.cmd.rightmove as f32;
        let mut wishvel = [0.0f32; 3];
        for i in 0..3 {
            wishvel[i] = self.pml.forward[i] * fmove + self.pml.right[i] * smove;
        }
        wishvel[2] += self.cmd.upmove as f32;
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        wishspeed *= scale_;
        self.pm_accelerate(&wishdir, wishspeed, PM_ACCELERATE);
        self.ps.origin = ma(&self.ps.origin, self.pml.frametime, &self.ps.velocity);
    }

    /// `PM_FootstepForSurface`.
    fn pm_footstep_for_surface(&self) -> i32 {
        if self.ps.stats[ST_STATE] & k::SS_CREEPSLOWED != 0 {
            return Event::FootstepSquelch as i32;
        }
        if self.pml.ground_trace.surface_flags & surface::NOSTEPS as i32 != 0 {
            return 0;
        }
        if self.pml.ground_trace.surface_flags & surface::METALSTEPS as i32 != 0 {
            return Event::FootstepMetal as i32;
        }
        Event::Footstep as i32
    }

    /// `PM_CrashLand`.
    fn pm_crash_land(&mut self) {
        if self.ps.pm_flags & k::PMF_BACKWARDS_JUMP != 0 {
            let a = if !self.nonseg() { A::LegsLandb as i32 } else { Nspa::Landback as i32 };
            self.pm_force_legs_anim(a);
        } else {
            let a = if !self.nonseg() { A::LegsLand as i32 } else { Nspa::Land as i32 };
            self.pm_force_legs_anim(a);
        }
        if !self.nonseg() {
            self.ps.legs_timer = TIMER_LAND;
        } else {
            self.ps.torso_timer = TIMER_LAND;
        }
        let dist = self.ps.origin[2] - self.pml.previous_origin[2];
        let vel = self.pml.previous_velocity[2];
        let acc = -(self.ps.gravity as f32);
        let a = acc / 2.0;
        let b = vel;
        let c = -dist;
        let den = b * b - 4.0 * a * c;
        if den < 0.0 {
            return;
        }
        let t = ((-b as f64 - (den as f64).sqrt()) / (2.0f32 * a) as f64) as f32;
        let mut delta = vel + t * acc;
        delta = (delta as f64 * delta as f64 * 0.0001) as f32;
        if self.waterlevel == 3 {
            return;
        }
        if self.waterlevel == 2 {
            delta = (delta as f64 * 0.25) as f32;
        }
        if self.waterlevel == 1 {
            delta = (delta as f64 * 0.5) as f32;
        }
        if delta < 1.0 {
            return;
        }
        if self.pml.ground_trace.surface_flags & surface::NODAMAGE as i32 == 0 {
            self.ps.stats[Stat::Falldist as usize] = delta as i32;
            if delta > k::AVG_FALL_DISTANCE {
                if bg::pm_live(self.ps.pm_type) {
                    self.pm_add_event(Event::FallFar);
                }
            } else if delta > k::MIN_FALL_DISTANCE {
                if bg::pm_live(self.ps.pm_type) {
                    self.pm_add_event(Event::FallMedium);
                }
            } else if delta > 7.0 {
                self.pm_add_event(Event::FallShort);
            } else {
                let ev = self.pm_footstep_for_surface();
                self.pm_add_event_raw(ev);
            }
        }
        self.ps.bob_cycle = 0;
    }

    /// `PM_CorrectAllSolid`.
    fn pm_correct_all_solid(&mut self, trace: &mut Trace) -> bool {
        for i in -1..=1i32 {
            for j in -1..=1i32 {
                for kk in -1..=1i32 {
                    let mut point = self.ps.origin;
                    point[0] += i as f32;
                    point[1] += j as f32;
                    point[2] += kk as f32;
                    *trace = self.trace(&point, &point, self.tracemask);
                    if !trace.allsolid {
                        let point = [self.ps.origin[0], self.ps.origin[1], (self.ps.origin[2] as f64 - 0.25) as f32];
                        *trace = self.trace(&self.ps.origin.clone(), &point, self.tracemask);
                        self.pml.ground_trace = *trace;
                        return true;
                    }
                }
            }
        }
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        self.pml.ground_plane = false;
        self.pml.walking = false;
        false
    }

    /// `PM_GroundTraceMissed`.
    fn pm_ground_trace_missed(&mut self) {
        if self.ps.ground_entity_num != ENTITYNUM_NONE {
            let mut point = self.ps.origin;
            point[2] -= 64.0;
            let trace = self.world.trace(&self.ps.origin, None, None, &point, self.ps.client_num, self.tracemask);
            if trace.fraction == 1.0 {
                self.pm_jump_anim();
            }
        }
        if self.has_ability(ability::TAKESFALLDAMAGE) && self.ps.velocity[2] < FALLING_THRESHOLD && self.pml.previous_velocity[2] >= FALLING_THRESHOLD {
            self.pm_add_event(Event::Falling);
        }
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        self.pml.ground_plane = false;
        self.pml.walking = false;
    }

    /// Ceiling-to-floor delta correction shared by GroundTrace and GroundClimbTrace.
    fn pm_ceiling_to_floor_correction(&mut self) {
        let (forward, _, _) = angle_vectors(&self.ps.viewangles);
        let rotated = rotate_point_around_vector(&self.ps.grapple_point, &forward, 180.0);
        let angles = vectoangles(&rotated);
        self.ps.delta_angles[YAW] -= angle2short_f(angles[YAW] - self.ps.viewangles[YAW]);
    }

    /// `PM_GroundClimbTrace`.
    fn pm_ground_climb_trace(&mut self) {
        let ref_normal: Vec3 = [0.0, 0.0, 1.0];
        let ceiling_normal: Vec3 = [0.0, 0.0, -1.0];
        let horizontal: Vec3 = [1.0, 0.0, 0.0];
        let eps: f32 = 0.000001;
        let surf_normal = bg::client_normal(self.ps);
        let mut movedir = project_point_on_plane(&self.pml.forward, &surf_normal);
        normalize(&mut movedir);
        let lookdir = movedir;
        if self.cmd.forwardmove < 0 {
            movedir = neg(&movedir);
        }
        if self.cmd.rightmove != 0 {
            movedir = self.pml.right;
            if self.cmd.rightmove < 0 {
                movedir = neg(&movedir);
            }
        }
        let mut trace = Trace::default();
        let mut i = 0;
        while i <= 4 {
            let origin = self.ps.origin;
            match i {
                0 => {
                    if self.pm_predict_step_move() {
                        i += 1;
                        continue;
                    }
                    let point = ma(&origin, 0.25, &movedir);
                    trace = self.trace(&origin, &point, self.tracemask);
                }
                1 => {
                    let point = ma(&origin, -0.25, &surf_normal);
                    trace = self.trace(&origin, &point, self.tracemask & !contents::BODY as i32);
                }
                2 => {
                    if self.pml.ground_plane && self.pm_predict_step_move() {
                        let point = ma(&origin, -STEPSIZE as f32, &surf_normal);
                        trace = self.trace(&origin, &point, self.tracemask);
                    } else {
                        i += 1;
                        continue;
                    }
                }
                3 => {
                    if self.pml.ground_plane {
                        let point = ma(&origin, -16.0, &surf_normal);
                        let point = ma(&point, -16.0, &movedir);
                        trace = self.trace(&origin, &point, self.tracemask);
                    } else {
                        i += 1;
                        continue;
                    }
                }
                _ => {
                    let point = [origin[0], origin[1], origin[2] - 0.25];
                    trace = self.trace(&origin, &point, self.tracemask);
                }
            }

            if trace.fraction < 1.0 && trace.surface_flags & (surface::SKY | surface::SLICK) as i32 == 0 && !(trace.entity_num != ENTITYNUM_WORLD && i != 4) {
                if i == 2 || i == 3 {
                    if i == 2 {
                        self.pm_step_event(&self.ps.origin.clone(), &trace.endpos, &surf_normal);
                    }
                    self.ps.origin = trace.endpos;
                }
                let mut trace_cross_surf = cross(&trace.plane.normal, &surf_normal);
                normalize(&mut trace_cross_surf);
                let mut trace_cross_ref = cross(&trace.plane.normal, &ref_normal);
                normalize(&mut trace_cross_ref);
                let mut surf_cross_ref = cross(&surf_normal, &ref_normal);
                normalize(&mut surf_cross_ref);

                let trace_dot_surf = dot(&trace.plane.normal, &surf_normal);
                let mut trace_ang_surf = rad2deg((trace_dot_surf as f64).acos()) as f32;
                if trace_ang_surf > 180.0 {
                    trace_ang_surf -= 180.0;
                }
                let trace_dot_ref = dot(&trace.plane.normal, &ref_normal);
                let mut trace_ang_ref = rad2deg((trace_dot_ref as f64).acos()) as f32;
                if trace_ang_ref > 180.0 {
                    trace_ang_ref -= 180.0;
                }
                let surf_dot_ref = dot(&surf_normal, &ref_normal);
                let mut surf_ang_ref = rad2deg((surf_dot_ref as f64).acos()) as f32;
                if surf_ang_ref > 180.0 {
                    surf_ang_ref -= 180.0;
                }

                if !compare_epsilon(&trace.plane.normal, &surf_normal, eps) {
                    if !compare_epsilon(&trace.plane.normal, &ref_normal, eps)
                        && !compare_epsilon(&surf_normal, &ref_normal, eps)
                        && !compare_epsilon(&trace.plane.normal, &ceiling_normal, eps)
                        && !compare_epsilon(&surf_normal, &ceiling_normal, eps)
                    {
                        let ref_to_trace = rotate_point_around_vector(&trace_cross_ref, &horizontal, -trace_ang_ref);
                        let temp_vec = rotate_point_around_vector(&surf_cross_ref, &horizontal, -surf_ang_ref);
                        let ref_to_surf_to_trace = rotate_point_around_vector(&trace_cross_surf, &temp_vec, -trace_ang_surf);
                        let rtt_dot = dot(&ref_to_trace, &ref_to_surf_to_trace);
                        let mut rtt_ang = angle2short_d(rad2deg((rtt_dot as f64).acos()));
                        if rtt_ang > 32768 {
                            rtt_ang -= 32768;
                        }
                        let mut tv = cross(&ref_to_trace, &ref_to_surf_to_trace);
                        normalize(&mut tv);
                        if dot(&trace.plane.normal, &tv) > 0.0 {
                            rtt_ang = -rtt_ang;
                        }
                        self.ps.delta_angles[YAW] -= rtt_ang;
                    }
                    let mut abc = cross(&trace_cross_surf, &surf_normal);
                    normalize(&mut abc);
                    let d = dot(&abc, &self.ps.origin);
                    let point = add(&self.ps.origin, &lookdir);
                    if (abc[0] * point[0] + abc[1] * point[1] + abc[2] * point[2] - d) > 0.0 {
                        trace_ang_surf = -trace_ang_surf;
                    }
                    let mut ld_dot_tcs = dot(&lookdir, &trace_cross_surf);
                    if ld_dot_tcs < 0.0 {
                        trace_cross_surf = neg(&trace_cross_surf);
                        ld_dot_tcs = dot(&lookdir, &trace_cross_surf);
                    }
                    trace_ang_surf *= 1.0f32 - ld_dot_tcs;
                    if self.ps.persistant[P_STATE] & k::PS_WALLCLIMBINGFOLLOW == 0 {
                        self.ps.delta_angles[PITCH] -= angle2short_f(trace_ang_surf);
                    }
                    if compare_epsilon(&trace.plane.normal, &ceiling_normal, eps) {
                        let mut gp = cross(&surf_normal, &trace.plane.normal);
                        normalize(&mut gp);
                        self.ps.grapple_point = gp;
                        self.ps.e_flags |= k::EF_WALLCLIMBCEILING;
                    }
                    if compare_epsilon(&surf_normal, &ceiling_normal, eps) {
                        let to_angles = vectoangles(&trace.plane.normal);
                        let surf_angles = vectoangles(&self.ps.grapple_point);
                        self.ps.delta_angles[1] -= angle2short_f(((surf_angles[1] - to_angles[1]) * 2.0) - 180.0);
                    }
                }
                self.pml.ground_trace = trace;
                self.ps.e_flags |= k::EF_WALLCLIMB;
                if !compare_epsilon(&trace.plane.normal, &ceiling_normal, eps) {
                    self.ps.grapple_point = trace.plane.normal;
                    self.ps.e_flags &= !k::EF_WALLCLIMBCEILING;
                }
                break;
            } else if trace.allsolid && !self.pm_correct_all_solid(&mut trace) {
                return;
            }
            i += 1;
        }

        if trace.fraction >= 1.0 {
            self.pm_ground_trace_missed();
            self.pml.ground_plane = false;
            self.pml.walking = false;
            self.ps.e_flags &= !k::EF_WALLCLIMB;
            if self.ps.e_flags & k::EF_WALLCLIMBCEILING != 0 {
                self.pm_ceiling_to_floor_correction();
            }
            self.ps.e_flags &= !k::EF_WALLCLIMBCEILING;
            self.ps.grapple_point = ref_normal;
            return;
        }
        self.pml.ground_plane = true;
        self.pml.walking = true;
        if self.ps.pm_flags & k::PMF_TIME_WATERJUMP != 0 {
            self.ps.pm_flags &= !k::PMF_TIME_WATERJUMP;
            self.ps.pm_time = 0;
        }
        self.ps.ground_entity_num = trace.entity_num;
        self.pm_add_touch_ent(trace.entity_num);
    }

    /// `PM_GroundTrace`.
    fn pm_ground_trace(&mut self) {
        let ref_normal: Vec3 = [0.0, 0.0, 1.0];
        if self.has_ability(ability::WALLCLIMBER) {
            if self.ps.persistant[P_STATE] & k::PS_WALLCLIMBINGTOGGLE != 0 {
                if self.cmd.upmove < 0 && self.ps.pm_flags & k::PMF_CROUCH_HELD == 0 {
                    if self.ps.stats[ST_STATE] & k::SS_WALLCLIMBING == 0 {
                        self.ps.stats[ST_STATE] |= k::SS_WALLCLIMBING;
                    } else {
                        self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBING;
                    }
                    self.ps.pm_flags |= k::PMF_CROUCH_HELD;
                } else if self.cmd.upmove >= 0 {
                    self.ps.pm_flags &= !k::PMF_CROUCH_HELD;
                }
            } else if self.cmd.upmove < 0 {
                self.ps.stats[ST_STATE] |= k::SS_WALLCLIMBING;
            } else {
                self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBING;
            }
            if self.ps.pm_type == PmType::Dead as i32 {
                self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBING;
            }
            if self.ps.stats[ST_STATE] & k::SS_WALLCLIMBING != 0 {
                self.pm_ground_climb_trace();
                return;
            }
            if self.ps.e_flags & k::EF_WALLCLIMBCEILING != 0 {
                self.pm_ceiling_to_floor_correction();
            }
        }
        self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBING;
        self.ps.e_flags &= !(k::EF_WALLCLIMB | k::EF_WALLCLIMBCEILING);

        let point = [self.ps.origin[0], self.ps.origin[1], self.ps.origin[2] - 0.25];
        let mut trace = self.trace(&self.ps.origin.clone(), &point, self.tracemask);
        self.pml.ground_trace = trace;
        if trace.allsolid && !self.pm_correct_all_solid(&mut trace) {
            return;
        }
        self.ps.grapple_point = ref_normal;

        if trace.fraction == 1.0 {
            let mut stepped_down = false;
            if self.pml.ground_plane && self.pm_predict_step_move() {
                let point = [self.ps.origin[0], self.ps.origin[1], self.ps.origin[2] - STEPSIZE as f32];
                trace = self.trace(&self.ps.origin.clone(), &point, self.tracemask);
                if trace.fraction < 1.0 {
                    self.pm_step_event(&self.ps.origin.clone(), &trace.endpos, &ref_normal);
                    self.ps.origin = trace.endpos;
                    stepped_down = true;
                }
            }
            if !stepped_down {
                self.pm_ground_trace_missed();
                self.pml.ground_plane = false;
                self.pml.walking = false;
                return;
            }
        }

        if self.ps.velocity[2] > 0.0 && dot(&self.ps.velocity, &trace.plane.normal) > 10.0 {
            self.pm_jump_anim();
            self.ps.ground_entity_num = ENTITYNUM_NONE;
            self.pml.ground_plane = false;
            self.pml.walking = false;
            return;
        }
        if trace.plane.normal[2] < MIN_WALK_NORMAL {
            self.ps.ground_entity_num = ENTITYNUM_NONE;
            self.pml.ground_plane = true;
            self.pml.walking = false;
            return;
        }
        self.pml.ground_plane = true;
        self.pml.walking = true;
        if self.ps.pm_flags & k::PMF_TIME_WATERJUMP != 0 {
            self.ps.pm_flags &= !k::PMF_TIME_WATERJUMP;
            self.ps.pm_time = 0;
        }
        if self.ps.ground_entity_num == ENTITYNUM_NONE {
            self.pmext.fall_velocity = self.pml.previous_velocity[2];
            if self.has_ability(ability::TAKESFALLDAMAGE) {
                self.pm_crash_land();
            }
        }
        self.ps.ground_entity_num = trace.entity_num;
        self.pm_add_touch_ent(trace.entity_num);
    }

    /// `PM_SetWaterLevel`.
    fn pm_set_water_level(&mut self) {
        self.waterlevel = 0;
        self.watertype = 0;
        let mut point = [self.ps.origin[0], self.ps.origin[1], self.ps.origin[2] + k::MINS_Z as f32 + 1.0];
        let cont = self.world.point_contents(&point, self.ps.client_num);
        if cont & k::MASK_WATER != 0 {
            let sample2 = self.ps.viewheight - k::MINS_Z;
            let sample1 = sample2 / 2;
            self.watertype = cont;
            self.waterlevel = 1;
            point[2] = self.ps.origin[2] + k::MINS_Z as f32 + sample1 as f32;
            let cont = self.world.point_contents(&point, self.ps.client_num);
            if cont & k::MASK_WATER != 0 {
                self.waterlevel = 2;
                point[2] = self.ps.origin[2] + k::MINS_Z as f32 + sample2 as f32;
                let cont = self.world.point_contents(&point, self.ps.client_num);
                if cont & k::MASK_WATER != 0 {
                    self.waterlevel = 3;
                }
            }
        }
    }

    /// `PM_SetViewheight`.
    fn pm_set_viewheight(&mut self) {
        let c = self.class();
        self.ps.viewheight = if self.ps.pm_flags & k::PMF_DUCKED != 0 { c.crouch_viewheight } else { c.viewheight };
    }

    /// `PM_CheckDuck`.
    fn pm_check_duck(&mut self) {
        let (pmins, pmaxs, pcmaxs, _, _) = bg::class_bbox(self.ps.stats[ST_CLASS]);
        self.mins[0] = pmins[0];
        self.mins[1] = pmins[1];
        self.maxs[0] = pmaxs[0];
        self.maxs[1] = pmaxs[1];
        self.mins[2] = pmins[2];
        if self.ps.pm_type == PmType::Dead as i32 {
            self.maxs[2] = -8.0;
            self.ps.viewheight = (pmins[2] + k::DEAD_VIEWHEIGHT as f32) as i32;
            return;
        }
        if self.cmd.upmove < 0 && !compare(&pmaxs, &pcmaxs) && self.ps.pm_type != PmType::Jetpack as i32 {
            self.ps.pm_flags |= k::PMF_DUCKED;
        } else if self.ps.pm_flags & k::PMF_DUCKED != 0 {
            self.maxs[2] = pmaxs[2];
            let o = self.ps.origin;
            let trace = self.trace(&o, &o, self.tracemask);
            if !trace.allsolid {
                self.ps.pm_flags &= !k::PMF_DUCKED;
            }
        }
        self.maxs[2] = if self.ps.pm_flags & k::PMF_DUCKED != 0 { pcmaxs[2] } else { pmaxs[2] };
        self.pm_set_viewheight();
    }

    /// `PM_Footsteps`.
    fn pm_footsteps(&mut self) {
        let v = self.ps.velocity;
        if self.has_ability(ability::WALLCLIMBER) && self.pml.ground_plane {
            self.xyspeed = ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt() as f32;
        } else {
            self.xyspeed = ((v[0] * v[0] + v[1] * v[1]) as f64).sqrt() as f32;
        }
        let ns = self.nonseg();
        if self.ps.ground_entity_num == ENTITYNUM_NONE {
            if self.waterlevel > 1 {
                self.pm_continue_legs_anim(if !ns { A::LegsSwim as i32 } else { Nspa::Swim as i32 });
            }
            return;
        }
        let (f, r) = (self.cmd.forwardmove, self.cmd.rightmove);
        if f == 0 && r == 0 {
            if self.xyspeed < 5.0 {
                self.ps.bob_cycle = 0;
                if self.ps.pm_flags & k::PMF_DUCKED != 0 {
                    self.pm_continue_legs_anim(if !ns { A::LegsIdlecr as i32 } else { Nspa::Stand as i32 });
                } else {
                    self.pm_continue_legs_anim(if !ns { A::LegsIdle as i32 } else { Nspa::Stand as i32 });
                }
            }
            return;
        }
        let side_anim = |walk: bool, back: bool| -> i32 {
            if r > 0 && f == 0 {
                if walk { Nspa::Walkright as i32 } else { Nspa::Runright as i32 }
            } else if r < 0 && f == 0 {
                if walk { Nspa::Walkleft as i32 } else { Nspa::Runleft as i32 }
            } else if back {
                if walk { Nspa::Walkback as i32 } else { Nspa::Runback as i32 }
            } else if walk {
                Nspa::Walk as i32
            } else {
                Nspa::Run as i32
            }
        };
        let mut footstep = false;
        let mut bobmove: f32;
        let backwards = self.ps.pm_flags & k::PMF_BACKWARDS_RUN != 0;
        if self.ps.pm_flags & k::PMF_DUCKED != 0 {
            bobmove = 0.5;
            let a = if backwards {
                if !ns { A::LegsBackcr as i32 } else { side_anim(true, true) }
            } else if !ns {
                A::LegsWalkcr as i32
            } else {
                side_anim(true, false)
            };
            self.pm_continue_legs_anim(a);
        } else if self.cmd.buttons & k::BUTTON_WALKING == 0 {
            bobmove = 0.4;
            let a = if self.wp(WeaponId::Level4) && self.ps.pm_flags & k::PMF_CHARGE != 0 {
                Nspa::Charge as i32
            } else if backwards {
                if !ns { A::LegsBack as i32 } else { side_anim(false, true) }
            } else if !ns {
                A::LegsRun as i32
            } else {
                side_anim(false, false)
            };
            self.pm_continue_legs_anim(a);
            footstep = true;
        } else {
            bobmove = 0.3;
            let a = if backwards {
                if !ns { A::LegsBackwalk as i32 } else { side_anim(true, true) }
            } else if !ns {
                A::LegsWalk as i32
            } else {
                side_anim(true, false)
            };
            self.pm_continue_legs_anim(a);
        }
        bobmove *= self.class().bob_cycle;
        if self.ps.stats[ST_STATE] & k::SS_SPEEDBOOST != 0 {
            bobmove *= k::HUMAN_SPRINT_MODIFIER;
        }
        let old = self.ps.bob_cycle;
        self.ps.bob_cycle = ((old as f32 + bobmove * self.pml.msec as f32) as i32) & 255;
        if ((old + 64) ^ (self.ps.bob_cycle + 64)) & 128 != 0 {
            match self.waterlevel {
                0 => {
                    if footstep && !self.no_footsteps {
                        let ev = self.pm_footstep_for_surface();
                        self.pm_add_event_raw(ev);
                    }
                }
                1 => self.pm_add_event(Event::Footsplash),
                2 => self.pm_add_event(Event::Swim),
                _ => {}
            }
        }
    }

    /// `PM_WaterEvents`.
    fn pm_water_events(&mut self) {
        let prev = self.pml.previous_waterlevel;
        if prev == 0 && self.waterlevel != 0 {
            self.pm_add_event(Event::WaterTouch);
        }
        if prev != 0 && self.waterlevel == 0 {
            self.pm_add_event(Event::WaterLeave);
        }
        if prev != 3 && self.waterlevel == 3 {
            self.pm_add_event(Event::WaterUnder);
        }
        if prev == 3 && self.waterlevel != 3 {
            self.pm_add_event(Event::WaterClear);
        }
    }

    /// `PM_BeginWeaponChange`.
    fn pm_begin_weapon_change(&mut self, weapon: i32) {
        if weapon <= 0 || weapon >= WeaponId::COUNT as i32 + 1 {
            return;
        }
        if !bg::inventory_contains_weapon(weapon, &self.ps.stats) {
            return;
        }
        if self.ps.weaponstate == WeaponState::Dropping as i32 {
            return;
        }
        self.ps.pm_flags &= !k::PMF_WEAPON_RELOAD;
        if self.ps.weaponstate == WeaponState::Reloading as i32 {
            self.ps.weapon_time = 0;
        }
        if self.wp(WeaponId::Lcannon) {
            self.ps.stats[ST_MISC] = 0;
        }
        self.ps.weaponstate = WeaponState::Dropping as i32;
        self.ps.weapon_time += 200;
        self.ps.persistant[Pers::Newweapon as usize] = weapon;
        self.ps.stats[Stat::Buildable as usize] = 0;
        if !self.nonseg() {
            self.pm_start_torso_anim(A::TorsoDrop as i32);
            self.pm_start_weapon_anim(Wa::Drop as i32);
        }
    }

    /// `PM_FinishWeaponChange`.
    fn pm_finish_weapon_change(&mut self) {
        self.pm_add_event(Event::ChangeWeapon);
        let mut weapon = self.ps.persistant[Pers::Newweapon as usize];
        if weapon < 0 || weapon >= WeaponId::COUNT as i32 + 1 {
            weapon = 0;
        }
        if !bg::inventory_contains_weapon(weapon, &self.ps.stats) {
            weapon = 0;
        }
        self.ps.weapon = weapon;
        self.ps.weaponstate = WeaponState::Raising as i32;
        self.ps.weapon_time += 250;
        if !self.nonseg() {
            self.pm_start_torso_anim(A::TorsoRaise as i32);
            self.pm_start_weapon_anim(Wa::Raise as i32);
        }
    }

    /// `PM_TorsoAnimation`.
    fn pm_torso_animation(&mut self) {
        if self.ps.weaponstate == WeaponState::Ready as i32 {
            if !self.nonseg() {
                let a = if self.wp(WeaponId::Blaster) { A::TorsoStand2 } else { A::TorsoStand };
                self.pm_continue_torso_anim(a as i32);
            }
            self.pm_continue_weapon_anim(Wa::Idle as i32);
        }
    }

    /// `PM_Weapon`.
    fn pm_weapon(&mut self) {
        let mut add_time = 200;
        let mut attack1 = self.cmd.buttons & k::BUTTON_ATTACK != 0;
        let mut attack2 = self.cmd.buttons & k::BUTTON_ATTACK2 != 0;
        let mut attack3 = self.cmd.buttons & k::BUTTON_USE_HOLDABLE != 0;
        let msec = self.pml.msec;

        if self.ps.persistant[Pers::Specstate as usize] != SpectatorState::Not as i32 {
            return;
        }
        if self.ps.stats[ST_HEALTH] <= 0 {
            self.ps.weapon = 0;
            return;
        }
        if self.wp(WeaponId::Level3) || self.wp(WeaponId::Level3upg) {
            let max = if self.wp(WeaponId::Level3) { k::LEVEL3_POUNCE_TIME } else { k::LEVEL3_POUNCE_TIME_UPG };
            if self.cmd.buttons & k::BUTTON_ATTACK2 != 0 {
                self.ps.stats[ST_MISC] += msec;
            } else {
                self.ps.stats[ST_MISC] -= msec;
            }
            if self.ps.stats[ST_MISC] > max {
                self.ps.stats[ST_MISC] = max;
            } else if self.ps.stats[ST_MISC] < 0 {
                self.ps.stats[ST_MISC] = 0;
            }
        }
        if self.wp(WeaponId::Level4) {
            if self.ps.stats[ST_STATE] & k::SS_CHARGING == 0 {
                if self.ps.stats[ST_MISC] < k::LEVEL4_TRAMPLE_CHARGE_TRIGGER && self.cmd.buttons & k::BUTTON_ATTACK2 != 0 {
                    self.ps.stats[ST_STATE] &= !k::SS_CHARGING;
                    if self.cmd.forwardmove > 0 {
                        let mut charge = msec;
                        let (mut dir, _, _) = angle_vectors(&self.ps.viewangles);
                        let mut vel = self.ps.velocity;
                        vel[2] = 0.0;
                        dir[2] = 0.0;
                        normalize(&mut vel);
                        normalize(&mut dir);
                        charge = (charge as f32 * dot(&dir, &vel)) as i32;
                        self.ps.stats[ST_MISC] += charge;
                    } else {
                        self.ps.stats[ST_MISC] = 0;
                    }
                } else if self.ps.stats[ST_STATE] & k::SS_CHARGING == 0 {
                    if self.ps.stats[ST_MISC] > k::LEVEL4_TRAMPLE_CHARGE_MIN {
                        if self.ps.stats[ST_MISC] > k::LEVEL4_TRAMPLE_CHARGE_MAX {
                            self.ps.stats[ST_MISC] = k::LEVEL4_TRAMPLE_CHARGE_MAX;
                        }
                        self.ps.stats[ST_MISC] = self.ps.stats[ST_MISC] * k::LEVEL4_TRAMPLE_DURATION / k::LEVEL4_TRAMPLE_CHARGE_MAX;
                        self.ps.stats[ST_STATE] |= k::SS_CHARGING;
                        self.pm_add_event(Event::Lev4TrampleStart);
                    } else {
                        self.ps.stats[ST_MISC] -= msec;
                    }
                }
            } else {
                if self.ps.stats[ST_MISC] < k::LEVEL4_TRAMPLE_CHARGE_MIN {
                    self.ps.stats[ST_MISC] = 0;
                } else {
                    self.ps.stats[ST_MISC] -= msec;
                }
                if length(&self.ps.velocity) < 64.0 || self.cmd.rightmove != 0 {
                    self.ps.stats[ST_MISC] -= k::LEVEL4_TRAMPLE_STOP_PENALTY * msec;
                }
            }
            if self.ps.stats[ST_MISC] <= 0 || self.cmd.forwardmove <= 0 {
                self.ps.stats[ST_MISC] = 0;
                self.ps.stats[ST_STATE] &= !k::SS_CHARGING;
            }
        }
        self.ps.e_flags &= !k::EF_WARN_CHARGE;
        if self.wp(WeaponId::Lcannon) {
            if self.ps.weapon_time == 0 && self.ps.weaponstate != WeaponState::NeedsReset as i32 && self.cmd.buttons & k::BUTTON_ATTACK != 0 {
                self.ps.stats[ST_MISC] += msec;
                if self.ps.stats[ST_MISC] >= k::LCANNON_CHARGE_TIME_MAX {
                    self.ps.stats[ST_MISC] = k::LCANNON_CHARGE_TIME_MAX;
                }
                if self.ps.stats[ST_MISC] > self.ps.ammo * k::LCANNON_CHARGE_TIME_MAX / k::LCANNON_CHARGE_AMMO {
                    self.ps.stats[ST_MISC] = self.ps.ammo * k::LCANNON_CHARGE_TIME_MAX / k::LCANNON_CHARGE_AMMO;
                }
            }
            if self.ps.stats[ST_MISC] > k::LCANNON_CHARGE_TIME_WARN {
                self.ps.e_flags |= k::EF_WARN_CHARGE;
            }
        }
        if self.ps.pm_flags & k::PMF_RESPAWNED != 0 {
            return;
        }
        if (self.wp(WeaponId::Level3) || self.wp(WeaponId::Level3upg)) && self.cmd.buttons & k::BUTTON_ATTACK != 0 && self.ps.pm_flags & k::PMF_CHARGE != 0 {
            return;
        }
        if self.ps.weapon_time > 0 {
            self.ps.weapon_time -= msec;
        }
        if self.ps.weapon_time < 0 {
            self.ps.weapon_time = 0;
        }
        if self.ps.stats[ST_STATE] & k::SS_CHARGING != 0 {
            return;
        }
        if bg::player_can_change_weapon(self.ps) {
            if self.cmd.buttons & k::BUTTON_USE_HOLDABLE != 0 {
                if self.ps.pm_flags & k::PMF_USE_ITEM_HELD == 0 {
                    let w = self.cmd.weapon as i32;
                    if w < 32 {
                        if self.ps.weapon != w {
                            self.pm_begin_weapon_change(w);
                        }
                    } else if bg::inventory_contains_upgrade(w - 32, &self.ps.stats) {
                        if bg::upgrade_is_active(w - 32, &self.ps.stats) {
                            bg::deactivate_upgrade(w - 32, &mut self.ps.stats);
                        } else {
                            bg::activate_upgrade(w - 32, &mut self.ps.stats);
                        }
                    }
                    self.ps.pm_flags |= k::PMF_USE_ITEM_HELD;
                }
            } else {
                self.ps.pm_flags &= !k::PMF_USE_ITEM_HELD;
            }
            if self.ps.pm_flags & k::PMF_WEAPON_SWITCH != 0 {
                self.ps.pm_flags &= !k::PMF_WEAPON_SWITCH;
                if self.ps.weapon != 0 {
                    let nw = self.ps.persistant[Pers::Newweapon as usize];
                    self.pm_begin_weapon_change(nw);
                } else {
                    self.pm_finish_weapon_change();
                }
            }
        }
        if self.ps.weapon_time > 0 {
            return;
        }
        if self.ps.weaponstate == WeaponState::Dropping as i32 {
            self.pm_finish_weapon_change();
            return;
        }
        if self.ps.weaponstate == WeaponState::Raising as i32 {
            self.ps.weaponstate = WeaponState::Ready as i32;
            if !self.nonseg() {
                let a = if self.wp(WeaponId::Blaster) { A::TorsoStand2 } else { A::TorsoStand };
                self.pm_continue_torso_anim(a as i32);
            }
            self.pm_continue_weapon_anim(Wa::Idle as i32);
            return;
        }
        let wdef = bg::weapon(self.ps.weapon);
        if self.ps.ammo == 0 && self.ps.clips == 0 && !wdef.infinite_ammo {
            if attack1 || (wdef.has_alt_mode && attack2) || (wdef.has_third_mode && attack3) {
                self.pm_add_event(Event::Noammo);
                self.ps.weapon_time += 500;
            }
            if self.ps.weaponstate == WeaponState::Firing as i32 {
                self.ps.weaponstate = WeaponState::Ready as i32;
            }
            return;
        }
        if self.ps.weaponstate == WeaponState::Reloading as i32 {
            self.ps.clips -= 1;
            self.ps.ammo = wdef.max_ammo;
            if wdef.uses_energy && bg::inventory_contains_upgrade(UpgradeId::Battpack as i32, &self.ps.stats) {
                self.ps.ammo = (self.ps.ammo as f32 * k::BATTPACK_MODIFIER) as i32;
            }
            self.ps.weaponstate = WeaponState::Raising as i32;
            self.pm_start_torso_anim(A::TorsoRaise as i32);
            self.ps.weapon_time += 250;
            return;
        }
        if !wdef.infinite_ammo && (self.ps.ammo <= 0 || self.ps.pm_flags & k::PMF_WEAPON_RELOAD != 0) && self.ps.clips > 0 {
            self.ps.pm_flags &= !k::PMF_WEAPON_RELOAD;
            self.ps.weaponstate = WeaponState::Reloading as i32;
            self.pm_start_torso_anim(A::TorsoDrop as i32);
            self.pm_start_weapon_anim(Wa::Reload as i32);
            self.ps.weapon_time += wdef.reload_time;
            return;
        }
        let w = WeaponId::from_num(self.ps.weapon as u8);
        match w {
            Some(WeaponId::Level0) => return,
            Some(WeaponId::Level3) | Some(WeaponId::Level3upg) => {
                if !attack1 && !attack2 && !attack3 {
                    return;
                }
            }
            Some(WeaponId::Lcannon) => {
                attack3 = false;
                if self.ps.weaponstate == WeaponState::NeedsReset as i32 {
                    if attack1 {
                        return;
                    }
                    self.ps.weaponstate = WeaponState::Ready as i32;
                }
                if attack1 || self.ps.stats[ST_MISC] > 0 {
                    attack2 = false;
                }
                if (attack1 || self.ps.stats[ST_MISC] == 0) && !attack2 {
                    self.ps.weapon_time = 0;
                    if self.ps.stats[ST_MISC] < k::LCANNON_CHARGE_TIME_MAX {
                        self.ps.weaponstate = WeaponState::Ready as i32;
                        return;
                    }
                    self.ps.weaponstate = WeaponState::NeedsReset as i32;
                }
                if self.ps.stats[ST_MISC] > k::LCANNON_CHARGE_TIME_MIN {
                    attack1 = true;
                    attack2 = false;
                } else if self.ps.stats[ST_MISC] > 0 {
                    self.ps.stats[ST_MISC] = 0;
                    self.ps.weapon_time = 0;
                    self.ps.weaponstate = WeaponState::Ready as i32;
                    return;
                } else if !attack2 {
                    self.ps.weapon_time = 0;
                    self.ps.weaponstate = WeaponState::Ready as i32;
                    return;
                }
            }
            Some(WeaponId::Mdriver) => {
                attack2 = false;
                attack3 = false;
                if !attack1 {
                    self.ps.weapon_time = 0;
                    self.ps.weaponstate = WeaponState::Ready as i32;
                    return;
                }
            }
            _ => {
                if !attack1 && !attack2 && !attack3 {
                    self.ps.weapon_time = 0;
                    self.ps.weaponstate = WeaponState::Ready as i32;
                    return;
                }
            }
        }
        if attack3 {
            if wdef.has_third_mode {
                if self.wp(WeaponId::Level3upg) && self.ps.ammo == 0 {
                    self.ps.weapon_time += 200;
                    return;
                }
                self.ps.generic1 = WeaponMode::Tertiary as i32;
                self.pm_add_event(Event::FireWeapon3);
                add_time = wdef.repeat_rate3;
            } else {
                self.ps.weapon_time = 0;
                self.ps.weaponstate = WeaponState::Ready as i32;
                self.ps.generic1 = WeaponMode::Notfiring as i32;
                return;
            }
        } else if attack2 {
            if wdef.has_alt_mode {
                self.ps.generic1 = WeaponMode::Secondary as i32;
                self.pm_add_event(Event::FireWeapon2);
                add_time = wdef.repeat_rate2;
            } else {
                self.ps.weapon_time = 0;
                self.ps.weaponstate = WeaponState::Ready as i32;
                self.ps.generic1 = WeaponMode::Notfiring as i32;
                return;
            }
        } else if attack1 {
            self.ps.generic1 = WeaponMode::Primary as i32;
            self.pm_add_event(Event::FireWeapon);
            add_time = wdef.repeat_rate1;
        }
        if (self.ps.weapon as usize) < 32 && self.auto_weapon_hit[self.ps.weapon as usize] {
            match w {
                Some(WeaponId::Level0) => {
                    self.ps.generic1 = WeaponMode::Primary as i32;
                    self.pm_add_event(Event::FireWeapon);
                    add_time = wdef.repeat_rate1;
                }
                Some(WeaponId::Level3) | Some(WeaponId::Level3upg) => {
                    self.ps.generic1 = WeaponMode::Secondary as i32;
                    self.pm_add_event(Event::FireWeapon2);
                    add_time = wdef.repeat_rate2;
                }
                _ => {}
            }
        }
        if !self.nonseg() {
            match w {
                Some(WeaponId::Flamer) => {
                    if self.ps.weaponstate == WeaponState::Ready as i32 {
                        self.pm_start_torso_anim(A::TorsoAttack as i32);
                        self.pm_start_weapon_anim(Wa::Attack1 as i32);
                    }
                }
                Some(WeaponId::Blaster) => {
                    self.pm_start_torso_anim(A::TorsoAttack2 as i32);
                    self.pm_start_weapon_anim(Wa::Attack1 as i32);
                }
                _ => {
                    self.pm_start_torso_anim(A::TorsoAttack as i32);
                    self.pm_start_weapon_anim(Wa::Attack1 as i32);
                }
            }
        } else {
            let mut num = self.rng.rand();
            match w {
                Some(WeaponId::Level1upg) | Some(WeaponId::Level1) => {
                    if attack1 {
                        num /= RAND_MAX / 6 + 1;
                        self.pm_force_legs_anim(Nspa::Attack1 as i32);
                        self.pm_start_weapon_anim(Wa::Attack1 as i32 + num);
                    }
                }
                Some(WeaponId::Level2upg) | Some(WeaponId::Level2) => {
                    // case fall-through in the original: level2upg's attack2, then level2's attack1
                    if w == Some(WeaponId::Level2upg) && attack2 {
                        self.pm_force_legs_anim(Nspa::Attack2 as i32);
                        self.pm_start_weapon_anim(Wa::Attack7 as i32);
                    }
                    if attack1 {
                        num /= RAND_MAX / 6 + 1;
                        self.pm_force_legs_anim(Nspa::Attack1 as i32);
                        self.pm_start_weapon_anim(Wa::Attack1 as i32 + num);
                    }
                }
                Some(WeaponId::Level4) => {
                    num /= RAND_MAX / 3 + 1;
                    self.pm_force_legs_anim(Nspa::Attack1 as i32 + num);
                    self.pm_start_weapon_anim(Wa::Attack1 as i32 + num);
                }
                _ => {
                    if attack1 {
                        self.pm_force_legs_anim(Nspa::Attack1 as i32);
                        self.pm_start_weapon_anim(Wa::Attack1 as i32);
                    } else if attack2 {
                        self.pm_force_legs_anim(Nspa::Attack2 as i32);
                        self.pm_start_weapon_anim(Wa::Attack2 as i32);
                    } else if attack3 {
                        self.pm_force_legs_anim(Nspa::Attack3 as i32);
                        self.pm_start_weapon_anim(Wa::Attack3 as i32);
                    }
                }
            }
            self.ps.torso_timer = TIMER_ATTACK;
        }
        if self.ps.weaponstate != WeaponState::NeedsReset as i32 {
            self.ps.weaponstate = WeaponState::Firing as i32;
        }
        if !wdef.infinite_ammo || (self.wp(WeaponId::Level3upg) && attack3) {
            if self.wp(WeaponId::Lcannon) && attack1 && !attack2 {
                self.ps.ammo -= (self.ps.stats[ST_MISC] * k::LCANNON_CHARGE_AMMO + k::LCANNON_CHARGE_TIME_MAX - 1) / k::LCANNON_CHARGE_TIME_MAX;
            } else {
                self.ps.ammo -= 1;
            }
            if self.ps.ammo < 0 {
                self.ps.ammo = 0;
            }
        }
        if self.wp(WeaponId::Chaingun) {
            let at = add_time as f32;
            if self.ps.pm_flags & k::PMF_DUCKED != 0 || bg::inventory_contains_upgrade(UpgradeId::Bsuit as i32, &self.ps.stats) {
                let r1 = self.rng.random();
                self.ps.delta_angles[PITCH] -= angle2short_d(((r1 as f64 * 0.5) - 0.125) * (30.0f32 / at) as f64);
                let r2 = self.rng.random();
                self.ps.delta_angles[YAW] -= angle2short_d(((r2 as f64 * 0.5) - 0.25) * (30.0f64 / at as f64));
            } else {
                let r1 = self.rng.random();
                self.ps.delta_angles[PITCH] -= angle2short_d(((r1 as f64 * 8.0) - 2.0) * (30.0f64 / at as f64));
                let r2 = self.rng.random();
                self.ps.delta_angles[YAW] -= angle2short_d(((r2 as f64 * 8.0) - 4.0) * (30.0f64 / at as f64));
            }
        }
        self.ps.weapon_time += add_time;
    }

    /// `PM_Animate`.
    fn pm_animate(&mut self) {
        if bg::pm_paralyzed(self.ps.pm_type) {
            return;
        }
        if self.cmd.buttons & k::BUTTON_GESTURE != 0 {
            if self.ps.taunt_timer > 0 {
                return;
            }
            if self.ps.torso_timer == 0 {
                if !self.nonseg() {
                    self.pm_start_torso_anim(A::TorsoGesture as i32);
                } else {
                    self.pm_force_legs_anim(Nspa::Gesture as i32);
                }
                self.ps.torso_timer = TIMER_GESTURE;
                self.ps.taunt_timer = TIMER_GESTURE;
                self.pm_add_event(Event::Taunt);
            }
        }
    }

    /// `PM_DropTimers`.
    fn pm_drop_timers(&mut self) {
        let msec = self.pml.msec;
        if self.ps.pm_time != 0 {
            if msec >= self.ps.pm_time {
                self.ps.pm_flags &= !k::PMF_ALL_TIMES;
                self.ps.pm_time = 0;
            } else {
                self.ps.pm_time -= msec;
            }
        }
        for t in [&mut self.ps.legs_timer, &mut self.ps.torso_timer, &mut self.ps.taunt_timer] {
            if *t > 0 {
                *t -= msec;
                if *t < 0 {
                    *t = 0;
                }
            }
        }
    }

    /// `PmoveSingle`.
    pub fn pmove_single(&mut self) {
        self.numtouch = 0;
        self.watertype = 0;
        self.waterlevel = 0;
        if self.ps.stats[ST_HEALTH] <= 0 {
            self.tracemask &= !(contents::BODY as i32);
        }
        if (self.cmd.forwardmove as i32).abs() > 64 || (self.cmd.rightmove as i32).abs() > 64 {
            self.cmd.buttons &= !k::BUTTON_WALKING;
        }
        let can_fire = self.ps.pm_flags & k::PMF_RESPAWNED == 0
            && self.ps.pm_type != PmType::Intermission as i32
            && ((self.ps.ammo > 0 || self.ps.clips > 0) || bg::weapon(self.ps.weapon).infinite_ammo);
        for (button, flag) in [(k::BUTTON_ATTACK, k::EF_FIRING), (k::BUTTON_ATTACK2, k::EF_FIRING2), (k::BUTTON_USE_HOLDABLE, k::EF_FIRING3)] {
            if can_fire && self.cmd.buttons & button != 0 {
                self.ps.e_flags |= flag;
            } else {
                self.ps.e_flags &= !flag;
            }
        }
        if self.ps.stats[ST_HEALTH] > 0 && self.cmd.buttons & (k::BUTTON_ATTACK | k::BUTTON_USE_HOLDABLE) == 0 {
            self.ps.pm_flags &= !k::PMF_RESPAWNED;
        }
        if self.cmd.buttons & k::BUTTON_TALK != 0 {
            self.cmd.buttons = k::BUTTON_TALK;
            self.cmd.forwardmove = 0;
            self.cmd.rightmove = 0;
            if self.cmd.upmove > 0 {
                self.cmd.upmove = 0;
            }
        }
        self.pml = Pml::default();
        self.pml.msec = (self.cmd.server_time - self.ps.command_time).clamp(1, 200);
        self.ps.command_time = self.cmd.server_time;
        self.pml.previous_origin = self.ps.origin;
        self.pml.previous_velocity = self.ps.velocity;
        self.pml.frametime = (self.pml.msec as f64 * 0.001) as f32;
        let (f, r, u) = angle_vectors(&self.ps.viewangles);
        self.pml.forward = f;
        self.pml.right = r;
        self.pml.up = u;
        if self.cmd.upmove < 10 {
            self.ps.pm_flags &= !k::PMF_JUMP_HELD;
        }
        if self.cmd.forwardmove < 0 {
            self.ps.pm_flags |= k::PMF_BACKWARDS_RUN;
        } else if self.cmd.forwardmove > 0 || (self.cmd.forwardmove == 0 && self.cmd.rightmove != 0) {
            self.ps.pm_flags &= !k::PMF_BACKWARDS_RUN;
        }
        if bg::pm_paralyzed(self.ps.pm_type) {
            self.cmd.forwardmove = 0;
            self.cmd.rightmove = 0;
            self.cmd.upmove = 0;
        }
        if self.ps.pm_type == PmType::Spectator as i32 {
            update_view_angles(self.ps, &self.cmd);
            self.pm_check_duck();
            self.pm_fly_move();
            self.pm_drop_timers();
            return;
        }
        if self.ps.pm_type == PmType::Noclip as i32 {
            update_view_angles(self.ps, &self.cmd);
            self.pm_noclip_move();
            self.pm_set_viewheight();
            self.pm_weapon();
            self.pm_drop_timers();
            return;
        }
        if self.ps.pm_type == PmType::Freeze as i32 || self.ps.pm_type == PmType::Intermission as i32 {
            return;
        }
        self.pm_set_water_level();
        self.pml.previous_waterlevel = self.waterlevel;
        self.pm_check_duck();
        self.pm_check_ladder();
        self.pm_ground_trace();
        update_view_angles(self.ps, &self.cmd);
        if self.ps.pm_type == PmType::Dead as i32 || self.ps.pm_type == PmType::Grabbed as i32 {
            self.pm_dead_move();
        }
        self.pm_drop_timers();
        self.pm_check_dodge();
        if self.ps.pm_type == PmType::Jetpack as i32 {
            self.pm_jet_pack_move();
        } else if self.ps.pm_flags & k::PMF_TIME_WATERJUMP != 0 {
            self.pm_water_jump_move();
        } else if self.waterlevel > 1 {
            self.pm_water_move();
        } else if self.pml.ladder {
            self.pm_ladder_move();
        } else if self.pml.walking {
            if self.has_ability(ability::WALLCLIMBER) && self.ps.stats[ST_STATE] & k::SS_WALLCLIMBING != 0 {
                self.pm_climb_move();
            } else {
                self.pm_walk_move();
            }
        } else {
            self.pm_air_move();
        }
        self.pm_animate();
        self.pm_ground_trace();
        update_view_angles(self.ps, &self.cmd);
        self.pm_set_water_level();
        self.pm_weapon();
        self.pm_torso_animation();
        self.pm_footsteps();
        self.pm_water_events();
        // trap_SnapVector: round-half-even (dec009)
        for i in 0..3 {
            self.ps.velocity[i] = self.ps.velocity[i].round_ties_even();
        }
    }

    /// `Pmove`: chops long moves into <= 66 ms (or pmove_msec) steps.
    pub fn pmove(&mut self) {
        let final_time = self.cmd.server_time;
        if final_time < self.ps.command_time {
            return;
        }
        if final_time > self.ps.command_time + 1000 {
            self.ps.command_time = final_time - 1000;
        }
        self.ps.pmove_framecount = (self.ps.pmove_framecount + 1) & ((1 << 6) - 1);
        while self.ps.command_time != final_time {
            let mut msec = final_time - self.ps.command_time;
            if self.pmove_fixed {
                if msec > self.pmove_msec {
                    msec = self.pmove_msec;
                }
            } else if msec > 66 {
                msec = 66;
            }
            self.cmd.server_time = self.ps.command_time + msec;
            self.pmove_single();
            if self.ps.pm_flags & k::PMF_JUMP_HELD != 0 {
                self.cmd.upmove = 20;
            }
        }
    }
}

/// `PM_UpdateViewAngles`.
pub fn update_view_angles(ps: &mut PlayerState, cmd: &UserCmd) {
    if ps.pm_type == PmType::Intermission as i32 {
        return;
    }
    if ps.pm_type != PmType::Spectator as i32 && ps.stats[ST_HEALTH] <= 0 {
        return;
    }
    let mut temp = [0i16; 3];
    let mut tempang = [0.0f32; 3];
    for i in 0..3 {
        if i == ROLL {
            temp[i] = ps.delta_angles[i] as i16;
        } else {
            temp[i] = (cmd.angles[i].wrapping_add(ps.delta_angles[i])) as i16;
        }
        if i == PITCH {
            if temp[i] > 16000 {
                ps.delta_angles[i] = 16000 - cmd.angles[i];
                temp[i] = 16000;
            } else if temp[i] < -16000 {
                ps.delta_angles[i] = -16000 - cmd.angles[i];
                temp[i] = -16000;
            }
        }
        tempang[i] = short2angle(temp[i] as i32) as f32;
    }
    let axis = angles_to_axis(&tempang);
    let rotaxis = if ps.stats[ST_STATE] & k::SS_WALLCLIMBING == 0 {
        axis
    } else {
        bg::rotate_axis(&ps.grapple_point, &axis, false, ps.e_flags & k::EF_WALLCLIMBCEILING != 0).unwrap_or(axis)
    };
    let mut tempang = axis_to_angles(&rotaxis);
    for a in tempang.iter_mut() {
        while *a > 180.0 {
            *a -= 360.0;
        }
        while *a < -180.0 {
            *a += 360.0;
        }
    }
    ps.viewangles = tempang;
    if ps.pm_type == PmType::Grabbed as i32 && !bg::inventory_contains_upgrade(UpgradeId::Bsuit as i32, &ps.stats) {
        let dir = byte_to_dir(ps.stats[Stat::Viewlock as usize]);
        let angles = vectoangles(&dir);
        for i in 0..3 {
            let mut diff = angle_subtract(ps.viewangles[i], angles[i]);
            while diff > 180.0 {
                diff -= 360.0;
            }
            while diff < -180.0 {
                diff += 360.0;
            }
            let fd = (diff as f64).abs();
            if diff < -90.0 {
                ps.delta_angles[i] += angle2short_d(fd - 90.0f32 as f64);
            } else if diff > 90.0 {
                ps.delta_angles[i] -= angle2short_d(fd - 90.0f32 as f64);
            }
            if diff < 0.0 {
                ps.delta_angles[i] += angle2short_d(fd * 0.05f32 as f64);
            } else if diff > 0.0 {
                ps.delta_angles[i] -= angle2short_d(fd * 0.05f32 as f64);
            }
        }
    }
}
