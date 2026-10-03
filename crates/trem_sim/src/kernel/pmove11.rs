//! Kernel `pmove11` — Tremulous 1.1.0's game/bg_pmove.c and
//! game/bg_slidemove.c (dec041): the movement a protocol 69 server runs, so
//! the cgame predicts it exactly when connected to one (`Cgame::vanilla`).
//!
//! It works on the 1.1 `playerState_t` (`v11::PlayerState11`: STAT_PCLASS /
//! STAT_PTEAM, the ammo packed in `ammo[]` / `powerups[]`, the weapons in
//! STAT_WEAPONS / STAT_WEAPONS2, SS_WALLCLIMBINGCEILING in STAT_STATE) and the
//! 1.1 class and weapon tables (bg11.rs over the classes11 / weapons11
//! sheets) and 1.1's q_math.c VectorNormalize (bg11.rs). Function names mirror the C (`pm_walk_move` = PM_WalkMove). The
//! golden trace re/harness/pmove11_ref.c (the ORIGINAL 1.1.0 code against the
//! shared collision) over sheets/domain/scenarios11.tsv is the judge:
//! re/traces/arena11.sha256, crates/trem_sim/tests/net_v11.rs.
//!
//! Reference source: re/source11/src/game/bg_pmove.c, bg_slidemove.c.

use super::bg11;
use super::cm::Trace;
use super::qmath::*;
use super::rng::Rng;
use super::types::{PmWorld, UserCmd, ENTITYNUM_NONE, ENTITYNUM_WORLD};
use super::v11::PlayerState11;
use crate::gen::constants11 as k;
use crate::gen::enums::{contents, surface};
use crate::gen::enums::{ability11, Event11, NonSegAnim as Nspa, Pers11, PlayerAnim as A, PmType11, Stat11, WeaponMode, WeaponState};
use crate::gen::upgrades11::Upgrade11Id;
use crate::gen::weapons11::Weapon11Id;

pub use super::pmove::Pml;

/// `MIN_WALK_NORMAL`.
pub const MIN_WALK_NORMAL: f32 = k::MIN_WALK_NORMAL;
/// `STEPSIZE`.
pub const STEPSIZE: i32 = k::STEPSIZE;
/// `OVERCLIP`.
pub const OVERCLIP: f32 = k::OVERCLIP;
/// `ANIM_TOGGLEBIT`.
pub const ANIM_TOGGLEBIT: i32 = k::ANIM_TOGGLEBIT;
/// `MAXTOUCH`.
pub const MAXTOUCH: usize = k::MAXTOUCH as usize;

/// `pm_stopspeed` and the other movement parameters of bg_pmove.c.
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

const ST_HEALTH: usize = Stat11::Health as usize;
const ST_PCLASS: usize = Stat11::Pclass as usize;
const ST_PTEAM: usize = Stat11::Pteam as usize;
const ST_STAMINA: usize = Stat11::Stamina as usize;
const ST_STATE: usize = Stat11::State as usize;
const ST_MISC: usize = Stat11::Misc as usize;
const ST_BUILDABLE: usize = Stat11::Buildable as usize;
const ST_FALLDIST: usize = Stat11::Falldist as usize;
const ST_VIEWLOCK: usize = Stat11::Viewlock as usize;
const P_STATE: usize = Pers11::State as usize;
const P_TEAM: usize = Pers11::Team as usize;
const P_NEWWEAPON: usize = Pers11::Newweapon as usize;
/// `PTE_HUMANS`.
const PTE_HUMANS: i32 = 2;
/// `TEAM_SPECTATOR` (team_t).
const TEAM_SPECTATOR: i32 = 1;

/// `pmove_t` over a 1.1 playerState.
pub struct Pmove11<'a, W: PmWorld> {
    pub ps: &'a mut PlayerState11,
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

#[inline]
fn wp(n: Weapon11Id) -> i32 {
    n as i32
}

impl<'a, W: PmWorld> Pmove11<'a, W> {
    pub fn new(ps: &'a mut PlayerState11, cmd: UserCmd, world: &'a W, rng: &'a mut Rng) -> Self {
        Pmove11 {
            ps,
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
    fn pclass(&self) -> i32 {
        self.ps.stats[ST_PCLASS]
    }
    #[inline]
    fn has_ability(&self, a: u32) -> bool {
        bg11::class_has_ability(self.pclass(), a)
    }
    #[inline]
    fn nonseg(&self) -> bool {
        self.ps.persistant[P_STATE] & k::PS_NONSEGMODEL != 0
    }
    #[inline]
    fn is(&self, w: Weapon11Id) -> bool {
        self.ps.weapon == wp(w)
    }
    #[inline]
    fn state(&self, ss: i32) -> bool {
        self.ps.stats[ST_STATE] & ss != 0
    }

    /// `PM_AddEvent`.
    pub fn pm_add_event(&mut self, ev: i32) {
        add_predictable_event(self.ps, ev, 0);
    }
    fn ev(&mut self, e: Event11) {
        self.pm_add_event(e as i32);
    }

    /// `PM_AddTouchEnt`.
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

    /// `PM_StartTorsoAnim`.
    fn pm_start_torso_anim(&mut self, anim: i32) {
        if self.ps.pm_type >= PmType11::Dead as i32 {
            return;
        }
        self.ps.torso_anim = ((self.ps.torso_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | anim;
    }
    /// `PM_StartLegsAnim`.
    fn pm_start_legs_anim(&mut self, anim: i32) {
        if self.ps.pm_type >= PmType11::Dead as i32 {
            return;
        }
        // legsTimer is clamped too tightly for nonsegmented models
        if !self.nonseg() {
            if self.ps.legs_timer > 0 {
                return;
            }
        } else if self.ps.torso_timer > 0 {
            return;
        }
        self.ps.legs_anim = ((self.ps.legs_anim & ANIM_TOGGLEBIT) ^ ANIM_TOGGLEBIT) | anim;
    }
    /// `PM_ContinueLegsAnim`.
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
    /// `PM_ContinueTorsoAnim`.
    fn pm_continue_torso_anim(&mut self, anim: i32) {
        if (self.ps.torso_anim & !ANIM_TOGGLEBIT) == anim {
            return;
        }
        if self.ps.torso_timer > 0 {
            return;
        }
        self.pm_start_torso_anim(anim);
    }
    /// `PM_ForceLegsAnim`.
    fn pm_force_legs_anim(&mut self, anim: i32) {
        if !self.nonseg() {
            self.ps.legs_timer = 0;
        } else {
            self.ps.torso_timer = 0;
        }
        self.pm_start_legs_anim(anim);
    }
    /// The forward / backward jump animation block repeated in the C.
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
        super::pmove::Pmove::<W>::clip_velocity(input, normal, overbounce)
    }

    /// `PM_Friction`.
    fn pm_friction(&mut self) {
        let mut vec = self.ps.velocity;
        // make sure vertical velocity is NOT set to zero when wall climbing
        if self.pml.walking && !self.state(k::SS_WALLCLIMBING) {
            vec[2] = 0.0;
        }
        let speed = length(&vec);
        if speed < 1.0 {
            self.ps.velocity[0] = 0.0;
            self.ps.velocity[1] = 0.0;
            return;
        }
        let mut drop = 0.0f32;
        // apply ground friction
        if self.waterlevel <= 1
            && (self.pml.walking || self.pml.ladder)
            && self.pml.ground_trace.surface_flags & surface::SLICK as i32 == 0
            && self.ps.pm_flags & k::PMF_TIME_KNOCKBACK == 0
        {
            let stop_speed = bg11::stop_speed(self.pclass());
            let control = if speed < stop_speed { stop_speed } else { speed };
            drop += control * bg11::friction(self.pclass()) * self.pml.frametime;
        }
        if self.waterlevel != 0 {
            drop += speed * PM_WATERFRICTION * self.waterlevel as f32 * self.pml.frametime;
        }
        if self.ps.pm_type == PmType11::Jetpack as i32 {
            drop += speed * PM_FLIGHTFRICTION * self.pml.frametime;
        }
        if self.ps.pm_type == PmType11::Spectator as i32 {
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

    /// `PM_CmdScale` (mutates `cmd` like the C's pointer argument).
    fn pm_cmd_scale(&mut self, cmd: &mut UserCmd) -> f32 {
        let mut modifier: f32 = 1.0;
        if self.ps.stats[ST_PTEAM] == PTE_HUMANS && self.ps.pm_type == PmType11::Normal as i32 {
            if self.state(k::SS_SPEEDBOOST) {
                modifier *= k::HUMAN_SPRINT_MODIFIER;
            } else {
                modifier *= k::HUMAN_JOG_MODIFIER;
            }
            if cmd.forwardmove < 0 {
                // can't run backwards
                modifier *= k::HUMAN_BACK_MODIFIER;
            } else if cmd.rightmove != 0 {
                // can't move that fast sideways
                modifier *= k::HUMAN_SIDE_MODIFIER;
            }
            // must have +ve stamina to jump
            if self.ps.stats[ST_STAMINA] < 0 {
                cmd.upmove = 0;
            }
            // slow down once stamina depletes
            if self.ps.stats[ST_STAMINA] <= -500 {
                modifier *= (self.ps.stats[ST_STAMINA] + 1000) as f32 / 500.0f32;
            }
            if self.state(k::SS_CREEPSLOWED) {
                if bg11::inventory_contains_upgrade(Upgrade11Id::Larmour as i32, &self.ps.stats) || bg11::inventory_contains_upgrade(Upgrade11Id::Bsuit as i32, &self.ps.stats) {
                    modifier *= k::CREEP_ARMOUR_MODIFIER;
                } else {
                    modifier *= k::CREEP_MODIFIER;
                }
            }
        }
        if self.is(Weapon11Id::Level4) && self.ps.pm_flags & k::PMF_CHARGE != 0 {
            modifier *= 1.0f32 + (self.ps.stats[ST_MISC] as f32 / k::LEVEL4_CHARGE_TIME as f32) * (k::LEVEL4_CHARGE_SPEED - 1.0f32);
        }
        // slow player if charging up for a pounce
        if (self.is(Weapon11Id::Level3) || self.is(Weapon11Id::Level3upg)) && cmd.buttons & k::BUTTON_ATTACK2 != 0 {
            modifier *= k::LEVEL3_POUNCE_SPEED_MOD;
        }
        // slow the player if slow locked
        if self.state(k::SS_SLOWLOCKED) {
            modifier *= k::ABUILDER_BLOB_SPEED_MOD;
        }
        if self.ps.pm_type == PmType11::Grabbed as i32 {
            modifier = 0.0;
        }
        if self.ps.pm_type != PmType11::Spectator as i32 && self.ps.pm_type != PmType11::Noclip as i32 {
            if bg11::jump_magnitude(self.pclass()) == 0.0 {
                cmd.upmove = 0;
            }
            // prevent speed distortions for non ducking classes
            if self.ps.pm_flags & k::PMF_DUCKED == 0 && self.ps.pm_type != PmType11::Jetpack as i32 && cmd.upmove < 0 {
                cmd.upmove = 0;
            }
        }
        let (fm, rm, um) = (cmd.forwardmove as i32, cmd.rightmove as i32, cmd.upmove as i32);
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
            if r == 0 && f > 0 {
                self.ps.movement_dir = 0;
            } else if r < 0 && f > 0 {
                self.ps.movement_dir = 1;
            } else if r < 0 && f == 0 {
                self.ps.movement_dir = 2;
            } else if r < 0 && f < 0 {
                self.ps.movement_dir = 3;
            } else if r == 0 && f < 0 {
                self.ps.movement_dir = 4;
            } else if r > 0 && f < 0 {
                self.ps.movement_dir = 5;
            } else if r > 0 && f == 0 {
                self.ps.movement_dir = 6;
            } else if r > 0 && f > 0 {
                self.ps.movement_dir = 7;
            }
        } else if self.ps.movement_dir == 2 {
            self.ps.movement_dir = 1;
        } else if self.ps.movement_dir == 6 {
            self.ps.movement_dir = 7;
        }
    }

    /// `PM_CheckCharge`.
    fn pm_check_charge(&mut self) {
        if !self.is(Weapon11Id::Level4) {
            return;
        }
        if self.cmd.buttons & k::BUTTON_ATTACK2 != 0 && !self.state(k::SS_CHARGING) {
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
        if !self.is(Weapon11Id::Level3) && !self.is(Weapon11Id::Level3upg) {
            return false;
        }
        if self.cmd.buttons & k::BUTTON_ATTACK2 != 0 {
            self.ps.pm_flags &= !k::PMF_CHARGE;
            return false;
        }
        if self.ps.pm_flags & k::PMF_CHARGE != 0 {
            return false;
        }
        if self.ps.stats[ST_MISC] == 0 {
            return false;
        }
        self.pml.ground_plane = false; // jumping away
        self.pml.walking = false;
        self.ps.pm_flags |= k::PMF_CHARGE;
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        self.ps.velocity = ma(&self.ps.velocity, self.ps.stats[ST_MISC] as f32, &self.pml.forward);
        self.ev(Event11::Jump);
        self.pm_jump_anim();
        true
    }

    /// `PM_CheckWallJump`.
    fn pm_check_wall_jump(&mut self) -> bool {
        let ref_normal: Vec3 = [0.0, 0.0, 1.0];
        let normal_fraction: f32 = 1.5;
        let cmd_fraction: f32 = 1.0;
        let up_fraction: f32 = 1.5;
        if self.ps.pm_flags & k::PMF_RESPAWNED != 0 {
            return false; // don't allow jump until all buttons are up
        }
        if self.cmd.upmove < 10 {
            return false; // not holding jump
        }
        if self.ps.pm_flags & k::PMF_TIME_WALLJUMP != 0 {
            return false;
        }
        // must wait for jump to be released
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
        bg11::vector_normalize(&mut dir);
        self.ps.velocity = ma(&self.ps.velocity, bg11::jump_magnitude(self.pclass()), &dir);
        // for a long run of wall jumps the velocity can get pretty large, this caps it
        if length(&self.ps.velocity) > k::LEVEL2_WALLJUMP_MAXSPEED {
            bg11::vector_normalize(&mut self.ps.velocity);
            self.ps.velocity = scale(&self.ps.velocity, k::LEVEL2_WALLJUMP_MAXSPEED);
        }
        self.ev(Event11::Jump);
        self.pm_jump_anim();
        true
    }

    /// `PM_CheckJump`.
    fn pm_check_jump(&mut self) -> bool {
        let pclass = self.pclass();
        if bg11::jump_magnitude(pclass) == 0.0 {
            return false;
        }
        if self.has_ability(ability11::WALLJUMPER) {
            return self.pm_check_wall_jump();
        }
        // can't jump and pounce at the same time
        if (self.is(Weapon11Id::Level3) || self.is(Weapon11Id::Level3upg)) && self.ps.stats[ST_MISC] > 0 {
            return false;
        }
        // can't jump and charge at the same time
        if self.is(Weapon11Id::Level4) && self.ps.stats[ST_MISC] > 0 {
            return false;
        }
        let human = self.ps.stats[ST_PTEAM] == PTE_HUMANS;
        if human && self.ps.stats[ST_STAMINA] < 0 {
            return false;
        }
        if self.ps.pm_flags & k::PMF_RESPAWNED != 0 {
            return false;
        }
        if self.cmd.upmove < 10 {
            return false;
        }
        // can't jump whilst grabbed
        if self.ps.pm_type == PmType11::Grabbed as i32 {
            self.cmd.upmove = 0;
            return false;
        }
        // must wait for jump to be released
        if self.ps.pm_flags & k::PMF_JUMP_HELD != 0 {
            self.cmd.upmove = 0;
            return false;
        }
        self.pml.ground_plane = false;
        self.pml.walking = false;
        self.ps.pm_flags |= k::PMF_JUMP_HELD;
        // take some stamina off
        if human {
            self.ps.stats[ST_STAMINA] -= 500;
        }
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        // jump away from wall
        if self.state(k::SS_WALLCLIMBING) {
            let mut normal: Vec3 = [0.0, 0.0, -1.0];
            if !self.state(k::SS_WALLCLIMBINGCEILING) {
                normal = self.ps.grapple_point;
            }
            self.ps.velocity = ma(&self.ps.velocity, bg11::jump_magnitude(pclass), &normal);
        } else {
            self.ps.velocity[2] = bg11::jump_magnitude(pclass);
        }
        self.ev(Event11::Jump);
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
        bg11::vector_normalize(&mut flatforward);
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
        // jump out of water
        self.ps.velocity = scale(&self.pml.forward, 200.0);
        self.ps.velocity[2] = 350.0;
        self.ps.pm_flags |= k::PMF_TIME_WATERJUMP;
        self.ps.pm_time = 2000;
        true
    }

    /// `PM_WaterJumpMove`.
    fn pm_water_jump_move(&mut self) {
        // waterjump has no control, but falls
        self.pm_step_slide_move(true, false);
        self.ps.velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
        if self.ps.velocity[2] < 0.0 {
            // cancel as soon as we are falling down again
            self.ps.pm_flags &= !k::PMF_ALL_TIMES;
            self.ps.pm_time = 0;
        }
    }

    /// The "make sure we can go up slopes easily" ground plane clip of
    /// PM_WaterMove / PM_LadderMove.
    fn clip_to_ground_keep_speed(&mut self) {
        if self.pml.ground_plane && dot(&self.ps.velocity, &self.pml.ground_trace.plane.normal) < 0.0 {
            let vel = length(&self.ps.velocity);
            self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &self.pml.ground_trace.plane.normal, OVERCLIP);
            bg11::vector_normalize(&mut self.ps.velocity);
            self.ps.velocity = scale(&self.ps.velocity, vel);
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
            wishvel[2] = -60.0; // sink towards bottom
        } else {
            for i in 0..3 {
                wishvel[i] = scale_ * self.pml.forward[i] * self.cmd.forwardmove as f32 + scale_ * self.pml.right[i] * self.cmd.rightmove as f32;
            }
            wishvel[2] += scale_ * self.cmd.upmove as f32;
        }
        let mut wishdir = wishvel;
        let mut wishspeed = bg11::vector_normalize(&mut wishdir);
        if wishspeed > self.ps.speed as f32 * PM_SWIM_SCALE {
            wishspeed = self.ps.speed as f32 * PM_SWIM_SCALE;
        }
        self.pm_accelerate(&wishdir, wishspeed, PM_WATERACCELERATE);
        self.clip_to_ground_keep_speed();
        self.pm_slide_move(false);
    }

    /// `PM_JetPackMove`. The C leaves wishvel[2] uninitialised when upmove is
    /// 0 (undefined behaviour); it is taken as 0 here, as tremulous-legacy
    /// later fixed it.
    fn pm_jet_pack_move(&mut self) {
        self.pm_friction();
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.cmd = cmd;
        let mut wishvel = [0.0f32; 3];
        for i in 0..2 {
            wishvel[i] = scale_ * self.pml.forward[i] * self.cmd.forwardmove as f32 + scale_ * self.pml.right[i] * self.cmd.rightmove as f32;
        }
        if (self.cmd.upmove as f32) > 0.0 {
            wishvel[2] = k::JETPACK_FLOAT_SPEED;
        }
        if (self.cmd.upmove as f32) < 0.0 {
            wishvel[2] = -k::JETPACK_SINK_SPEED;
        }
        let mut wishdir = wishvel;
        let wishspeed = bg11::vector_normalize(&mut wishdir);
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
        let wishspeed = bg11::vector_normalize(&mut wishdir);
        self.pm_accelerate(&wishdir, wishspeed, PM_FLYACCELERATE);
        self.pm_step_slide_move(false, false);
    }

    /// `PM_AirMove`.
    fn pm_air_move(&mut self) {
        self.pm_friction();
        let fmove = self.cmd.forwardmove as f32;
        let smove = self.cmd.rightmove as f32;
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        // set the movementDir so clients can rotate the legs for strafing
        self.pm_set_movement_dir();
        // project moves down to flat plane
        self.pml.forward[2] = 0.0;
        self.pml.right[2] = 0.0;
        bg11::vector_normalize(&mut self.pml.forward);
        bg11::vector_normalize(&mut self.pml.right);
        let mut wishvel = [0.0f32; 3];
        for i in 0..2 {
            wishvel[i] = self.pml.forward[i] * fmove + self.pml.right[i] * smove;
        }
        wishvel[2] = 0.0;
        let mut wishdir = wishvel;
        let mut wishspeed = bg11::vector_normalize(&mut wishdir);
        wishspeed *= scale_;
        // not on ground, so little effect on velocity
        self.pm_accelerate(&wishdir, wishspeed, bg11::air_acceleration(self.pclass()));
        // we may have a ground plane that is very steep, even
        // though we don't have a groundentity: slide along the steep plane
        if self.pml.ground_plane {
            self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &self.pml.ground_trace.plane.normal, OVERCLIP);
        }
        self.pm_step_slide_move(true, false);
    }

    /// The ducking and wading clamps of PM_ClimbMove / PM_WalkMove.
    fn clamp_wishspeed(&self, wishspeed: &mut f32) {
        // clamp the speed lower if ducking
        if self.ps.pm_flags & k::PMF_DUCKED != 0 && *wishspeed > self.ps.speed as f32 * PM_DUCK_SCALE {
            *wishspeed = self.ps.speed as f32 * PM_DUCK_SCALE;
        }
        // clamp the speed lower if wading or walking on the bottom
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
            // begin swimming
            self.pm_water_move();
            return;
        }
        if self.pm_check_jump() || self.pm_check_pounce() {
            // jumped away
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
        // project the forward and right directions onto the ground plane
        let n = self.pml.ground_trace.plane.normal;
        self.pml.forward = Self::clip_velocity(&self.pml.forward, &n, OVERCLIP);
        self.pml.right = Self::clip_velocity(&self.pml.right, &n, OVERCLIP);
        bg11::vector_normalize(&mut self.pml.forward);
        bg11::vector_normalize(&mut self.pml.right);
        let mut wishvel = [0.0f32; 3];
        for i in 0..3 {
            wishvel[i] = self.pml.forward[i] * fmove + self.pml.right[i] * smove;
        }
        let mut wishdir = wishvel;
        let mut wishspeed = bg11::vector_normalize(&mut wishdir);
        wishspeed *= scale_;
        self.clamp_wishspeed(&mut wishspeed);
        // when a player gets hit, they temporarily lose full control
        let slick = self.pml.ground_trace.surface_flags & surface::SLICK as i32 != 0 || self.ps.pm_flags & k::PMF_TIME_KNOCKBACK != 0;
        let accelerate = if slick { bg11::air_acceleration(self.pclass()) } else { bg11::acceleration(self.pclass()) };
        self.pm_accelerate(&wishdir, wishspeed, accelerate);
        if slick {
            self.ps.velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
        }
        let vel = length(&self.ps.velocity);
        // slide along the ground plane
        self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &n, OVERCLIP);
        // don't decrease velocity when going up or down a slope
        bg11::vector_normalize(&mut self.ps.velocity);
        self.ps.velocity = scale(&self.ps.velocity, vel);
        // don't do anything if standing still
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
        // charging
        self.pm_check_charge();
        self.pm_friction();
        let fmove = self.cmd.forwardmove as f32;
        let smove = self.cmd.rightmove as f32;
        let mut cmd = self.cmd;
        let scale_ = self.pm_cmd_scale(&mut cmd);
        self.pm_set_movement_dir();
        // project moves down to flat plane
        self.pml.forward[2] = 0.0;
        self.pml.right[2] = 0.0;
        let n = self.pml.ground_trace.plane.normal;
        self.pml.forward = Self::clip_velocity(&self.pml.forward, &n, OVERCLIP);
        self.pml.right = Self::clip_velocity(&self.pml.right, &n, OVERCLIP);
        bg11::vector_normalize(&mut self.pml.forward);
        bg11::vector_normalize(&mut self.pml.right);
        let mut wishvel = [0.0f32; 3];
        for i in 0..3 {
            wishvel[i] = self.pml.forward[i] * fmove + self.pml.right[i] * smove;
        }
        let mut wishdir = wishvel;
        let mut wishspeed = bg11::vector_normalize(&mut wishdir);
        wishspeed *= scale_;
        self.clamp_wishspeed(&mut wishspeed);
        let slick = self.pml.ground_trace.surface_flags & surface::SLICK as i32 != 0 || self.ps.pm_flags & k::PMF_TIME_KNOCKBACK != 0;
        let accelerate = if slick { bg11::air_acceleration(self.pclass()) } else { bg11::acceleration(self.pclass()) };
        self.pm_accelerate(&wishdir, wishspeed, accelerate);
        if slick {
            self.ps.velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
        }
        let vel = length(&self.ps.velocity);
        // slide along the ground plane
        self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &n, OVERCLIP);
        // don't decrease velocity when going up or down a slope
        bg11::vector_normalize(&mut self.ps.velocity);
        self.ps.velocity = scale(&self.ps.velocity, vel);
        // don't do anything if standing still
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
        let mut wishspeed = bg11::vector_normalize(&mut wishdir);
        if wishspeed > self.ps.speed as f32 * PM_SWIM_SCALE {
            wishspeed = self.ps.speed as f32 * PM_SWIM_SCALE;
        }
        self.pm_accelerate(&wishdir, wishspeed, PM_ACCELERATE);
        // slanty ladders
        self.clip_to_ground_keep_speed();
        self.pm_slide_move(false);
    }

    /// `PM_CheckLadder`.
    fn pm_check_ladder(&mut self) {
        if !self.has_ability(ability11::CANUSELADDERS) {
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
        // extra friction
        let mut forward = length(&self.ps.velocity);
        forward -= 20.0;
        if forward <= 0.0 {
            self.ps.velocity = [0.0; 3];
        } else {
            bg11::vector_normalize(&mut self.ps.velocity);
            self.ps.velocity = scale(&self.ps.velocity, forward);
        }
    }

    /// `PM_NoclipMove`.
    fn pm_noclip_move(&mut self) {
        self.ps.viewheight = k::DEFAULT_VIEWHEIGHT;
        let speed = length(&self.ps.velocity);
        if speed < 1.0 {
            self.ps.velocity = VEC3_ORIGIN;
        } else {
            let mut drop = 0.0f32;
            let friction = (PM_FRICTION as f64 * 1.5) as f32; // extra friction
            let control = if speed < PM_STOPSPEED { PM_STOPSPEED } else { speed };
            drop += control * friction * self.pml.frametime;
            let mut newspeed = speed - drop;
            if newspeed < 0.0 {
                newspeed = 0.0;
            }
            newspeed /= speed;
            self.ps.velocity = scale(&self.ps.velocity, newspeed);
        }
        // accelerate
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
        let mut wishspeed = bg11::vector_normalize(&mut wishdir);
        wishspeed *= scale_;
        self.pm_accelerate(&wishdir, wishspeed, PM_ACCELERATE);
        // move
        self.ps.origin = ma(&self.ps.origin, self.pml.frametime, &self.ps.velocity);
    }

    /// `PM_FootstepForSurface`.
    fn pm_footstep_for_surface(&self) -> i32 {
        if self.state(k::SS_CREEPSLOWED) {
            return Event11::FootstepSquelch as i32;
        }
        if self.pml.ground_trace.surface_flags & surface::NOSTEPS as i32 != 0 {
            return 0;
        }
        if self.pml.ground_trace.surface_flags & surface::METALSTEPS as i32 != 0 {
            return Event11::FootstepMetal as i32;
        }
        Event11::Footstep as i32
    }

    /// `PM_CrashLand`.
    fn pm_crash_land(&mut self) {
        // decide which landing animation to use
        if self.ps.pm_flags & k::PMF_BACKWARDS_JUMP != 0 {
            let a = if !self.nonseg() { A::LegsLandb as i32 } else { Nspa::Landback as i32 };
            self.pm_force_legs_anim(a);
        } else {
            let a = if !self.nonseg() { A::LegsLand as i32 } else { Nspa::Land as i32 };
            self.pm_force_legs_anim(a);
        }
        if !self.nonseg() {
            self.ps.legs_timer = k::TIMER_LAND;
        } else {
            self.ps.torso_timer = k::TIMER_LAND;
        }
        // calculate the exact velocity on landing
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
        // ducking while falling doubles damage
        if self.ps.pm_flags & k::PMF_DUCKED != 0 {
            delta *= 2.0;
        }
        // never take falling damage if completely underwater
        if self.waterlevel == 3 {
            return;
        }
        // reduce falling damage if there is standing water
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
            self.ps.stats[ST_FALLDIST] = delta as i32;
            if delta > k::AVG_FALL_DISTANCE {
                self.ev(Event11::FallFar);
            } else if delta > k::MIN_FALL_DISTANCE {
                // this is a pain grunt, so don't play it if dead
                if self.ps.stats[ST_HEALTH] > 0 {
                    self.ev(Event11::FallMedium);
                }
            } else if delta > 7.0 {
                self.ev(Event11::FallShort);
            } else {
                let e = self.pm_footstep_for_surface();
                self.pm_add_event(e);
            }
        }
        // start footstep cycle over
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
            // we just transitioned into freefall
            let mut point = self.ps.origin;
            point[2] -= 64.0;
            let trace = self.world.trace(&self.ps.origin, None, None, &point, self.ps.client_num, self.tracemask);
            if trace.fraction == 1.0 {
                self.pm_jump_anim();
            }
        }
        if self.has_ability(ability11::TAKESFALLDAMAGE) && self.ps.velocity[2] < k::FALLING_THRESHOLD && self.pml.previous_velocity[2] >= k::FALLING_THRESHOLD {
            self.ev(Event11::Falling);
        }
        self.ps.ground_entity_num = ENTITYNUM_NONE;
        self.pml.ground_plane = false;
        self.pml.walking = false;
    }

    /// The "just transided from ceiling to floor" delta correction of
    /// PM_GroundClimbTrace and PM_GroundTrace.
    fn pm_ceiling_to_floor_correction(&mut self) {
        let (forward, _, _) = angle_vectors(&self.ps.viewangles);
        let rotated = rotate_point_around_vector(&self.ps.grapple_point, &forward, 180.0);
        let angles = vectoangles(&rotated);
        self.ps.delta_angles[YAW] -= angle2short_f(angles[YAW] - self.ps.viewangles[YAW]);
    }

    /// The wall walking surface normal (grapplePoint, or the ceiling).
    fn surf_normal(&self) -> Vec3 {
        if self.state(k::SS_WALLCLIMBINGCEILING) {
            [0.0, 0.0, -1.0]
        } else {
            self.ps.grapple_point
        }
    }

    /// `PM_GroundClimbTrace`.
    fn pm_ground_climb_trace(&mut self) {
        let ref_normal: Vec3 = [0.0, 0.0, 1.0];
        let ceiling_normal: Vec3 = [0.0, 0.0, -1.0];
        let horizontal: Vec3 = [1.0, 0.0, 0.0];
        // If we're on the ceiling then grapplePoint is a rotation normal.. otherwise its a surface normal.
        let surf_normal = self.surf_normal();
        // construct a vector which reflects the direction the player is looking wrt the surface normal
        let mut movedir = project_point_on_plane(&self.pml.forward, &surf_normal);
        bg11::vector_normalize(&mut movedir);
        let lookdir = movedir;
        if self.cmd.forwardmove < 0 {
            movedir = neg(&movedir);
        }
        // allow strafe transitions
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
                    // we are going to step this frame so skip the transition test
                    if self.pm_predict_step_move() {
                        i += 1;
                        continue;
                    }
                    // trace into direction we are moving
                    let point = ma(&origin, 0.25, &movedir);
                    trace = self.trace(&origin, &point, self.tracemask);
                }
                1 => {
                    // trace straight down anto "ground" surface
                    let point = ma(&origin, -0.25, &surf_normal);
                    trace = self.trace(&origin, &point, self.tracemask);
                }
                2 => {
                    if self.pml.ground_plane && self.pm_predict_step_move() {
                        // step down
                        let point = ma(&origin, -STEPSIZE as f32, &surf_normal);
                        trace = self.trace(&origin, &point, self.tracemask);
                    } else {
                        i += 1;
                        continue;
                    }
                }
                3 => {
                    // trace "underneath" BBOX so we can traverse angles > 180deg
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
                    // fall back so we don't have to modify PM_GroundTrace too much
                    let point = [origin[0], origin[1], origin[2] - 0.25];
                    trace = self.trace(&origin, &point, self.tracemask);
                }
            }

            // if we hit something
            if trace.fraction < 1.0 && trace.surface_flags & (surface::SKY | surface::SLICK) as i32 == 0 && !(trace.entity_num != ENTITYNUM_WORLD && i != 4) {
                if i == 2 || i == 3 {
                    if i == 2 {
                        self.pm_step_event(&self.ps.origin.clone(), &trace.endpos, &surf_normal);
                    }
                    self.ps.origin = trace.endpos;
                }
                // calculate a bunch of stuff...
                let mut trace_cross_surf = cross(&trace.plane.normal, &surf_normal);
                bg11::vector_normalize(&mut trace_cross_surf);
                let mut trace_cross_ref = cross(&trace.plane.normal, &ref_normal);
                bg11::vector_normalize(&mut trace_cross_ref);
                let mut surf_cross_ref = cross(&surf_normal, &ref_normal);
                bg11::vector_normalize(&mut surf_cross_ref);
                // calculate angle between surf and trace
                let trace_dot_surf = dot(&trace.plane.normal, &surf_normal);
                let mut trace_ang_surf = rad2deg((trace_dot_surf as f64).acos()) as f32;
                if trace_ang_surf > 180.0 {
                    trace_ang_surf -= 180.0;
                }
                // calculate angle between trace and ref
                let trace_dot_ref = dot(&trace.plane.normal, &ref_normal);
                let mut trace_ang_ref = rad2deg((trace_dot_ref as f64).acos()) as f32;
                if trace_ang_ref > 180.0 {
                    trace_ang_ref -= 180.0;
                }
                // calculate angle between surf and ref
                let surf_dot_ref = dot(&surf_normal, &ref_normal);
                let mut surf_ang_ref = rad2deg((surf_dot_ref as f64).acos()) as f32;
                if surf_ang_ref > 180.0 {
                    surf_ang_ref -= 180.0;
                }
                // if the trace result and old surface normal are different then we must have transided to a new surface
                if !compare(&trace.plane.normal, &surf_normal) {
                    // if the trace result or the old vector is not the floor or ceiling correct the YAW angle
                    if !compare(&trace.plane.normal, &ref_normal) && !compare(&surf_normal, &ref_normal) && !compare(&trace.plane.normal, &ceiling_normal) && !compare(&surf_normal, &ceiling_normal) {
                        // calculate reference rotated through to trace plane
                        let ref_to_trace = rotate_point_around_vector(&trace_cross_ref, &horizontal, -trace_ang_ref);
                        // calculate reference rotated through to surf plane then to trace plane
                        let temp_vec = rotate_point_around_vector(&surf_cross_ref, &horizontal, -surf_ang_ref);
                        let ref_to_surf_to_trace = rotate_point_around_vector(&trace_cross_surf, &temp_vec, -trace_ang_surf);
                        // calculate angle between refTOtrace and refTOsurfTOtrace
                        let rtt_dot = dot(&ref_to_trace, &ref_to_surf_to_trace);
                        let mut rtt_ang = angle2short_d(rad2deg((rtt_dot as f64).acos()));
                        if rtt_ang > 32768 {
                            rtt_ang -= 32768;
                        }
                        let mut tv = cross(&ref_to_trace, &ref_to_surf_to_trace);
                        bg11::vector_normalize(&mut tv);
                        if dot(&trace.plane.normal, &tv) > 0.0 {
                            rtt_ang = -rtt_ang;
                        }
                        // phew! - correct the angle
                        self.ps.delta_angles[YAW] -= rtt_ang;
                    }
                    // construct a plane dividing the surf and trace normals
                    let mut abc = cross(&trace_cross_surf, &surf_normal);
                    bg11::vector_normalize(&mut abc);
                    let d = dot(&abc, &self.ps.origin);
                    // construct a point representing where the player is looking
                    let point = add(&self.ps.origin, &lookdir);
                    // check whether point is on one side of the plane, if so invert the correction angle
                    if (abc[0] * point[0] + abc[1] * point[1] + abc[2] * point[2] - d) > 0.0 {
                        trace_ang_surf = -trace_ang_surf;
                    }
                    // find the . product of the lookdir and traceCROSSsurf
                    let mut ld_dot_tcs = dot(&lookdir, &trace_cross_surf);
                    if ld_dot_tcs < 0.0 {
                        trace_cross_surf = neg(&trace_cross_surf);
                        ld_dot_tcs = dot(&lookdir, &trace_cross_surf);
                    }
                    // set the correction angle
                    trace_ang_surf *= 1.0f32 - ld_dot_tcs;
                    if self.ps.persistant[P_STATE] & k::PS_WALLCLIMBINGFOLLOW == 0 {
                        // correct the angle
                        self.ps.delta_angles[PITCH] -= angle2short_f(trace_ang_surf);
                    }
                    // transition from wall to ceiling: normal for subsequent viewangle rotations
                    if compare(&trace.plane.normal, &ceiling_normal) {
                        let mut gp = cross(&surf_normal, &trace.plane.normal);
                        bg11::vector_normalize(&mut gp);
                        self.ps.grapple_point = gp;
                        self.ps.stats[ST_STATE] |= k::SS_WALLCLIMBINGCEILING;
                    }
                    // transition from ceiling to wall
                    if compare(&surf_normal, &ceiling_normal) {
                        let to_angles = vectoangles(&trace.plane.normal);
                        let surf_angles = vectoangles(&self.ps.grapple_point);
                        self.ps.delta_angles[1] -= angle2short_f(((surf_angles[1] - to_angles[1]) * 2.0) - 180.0);
                    }
                }
                self.pml.ground_trace = trace;
                // so everything knows where we're wallclimbing (ie client side)
                self.ps.e_flags |= k::EF_WALLCLIMB;
                // if we're not stuck to the ceiling then set grapplePoint to be a surface normal
                if !compare(&trace.plane.normal, &ceiling_normal) {
                    self.ps.grapple_point = trace.plane.normal;
                    self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBINGCEILING;
                }
                // IMPORTANT: break out of the for loop if we've hit something
                break;
            } else if trace.allsolid && !self.pm_correct_all_solid(&mut trace) {
                return;
            }
            i += 1;
        }

        if trace.fraction >= 1.0 {
            // if the trace didn't hit anything, we are in free fall
            self.pm_ground_trace_missed();
            self.pml.ground_plane = false;
            self.pml.walking = false;
            self.ps.e_flags &= !k::EF_WALLCLIMB;
            // just transided from ceiling to floor... apply delta correction
            if self.state(k::SS_WALLCLIMBINGCEILING) {
                self.pm_ceiling_to_floor_correction();
            }
            self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBINGCEILING;
            // we get very bizarre effects if we don't do this :0
            self.ps.grapple_point = ref_normal;
            return;
        }
        self.pml.ground_plane = true;
        self.pml.walking = true;
        // hitting solid ground will end a waterjump
        if self.ps.pm_flags & k::PMF_TIME_WATERJUMP != 0 {
            self.ps.pm_flags &= !(k::PMF_TIME_WATERJUMP | k::PMF_TIME_LAND);
            self.ps.pm_time = 0;
        }
        self.ps.ground_entity_num = trace.entity_num;
        self.pm_add_touch_ent(trace.entity_num);
    }

    /// `PM_GroundTrace`.
    fn pm_ground_trace(&mut self) {
        let ref_normal: Vec3 = [0.0, 0.0, 1.0];
        if self.has_ability(ability11::WALLCLIMBER) {
            if self.ps.persistant[P_STATE] & k::PS_WALLCLIMBINGTOGGLE != 0 {
                // toggle wall climbing if holding crouch
                if self.cmd.upmove < 0 && self.ps.pm_flags & k::PMF_CROUCH_HELD == 0 {
                    if !self.state(k::SS_WALLCLIMBING) {
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
            if self.ps.pm_type == PmType11::Dead as i32 {
                self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBING;
            }
            if self.state(k::SS_WALLCLIMBING) {
                self.pm_ground_climb_trace();
                return;
            }
            // just transided from ceiling to floor... apply delta correction
            if self.state(k::SS_WALLCLIMBINGCEILING) {
                self.pm_ceiling_to_floor_correction();
            }
        }
        self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBING;
        self.ps.stats[ST_STATE] &= !k::SS_WALLCLIMBINGCEILING;
        self.ps.e_flags &= !k::EF_WALLCLIMB;

        let point = [self.ps.origin[0], self.ps.origin[1], self.ps.origin[2] - 0.25];
        let mut trace = self.trace(&self.ps.origin.clone(), &point, self.tracemask);
        self.pml.ground_trace = trace;
        // do something corrective if the trace starts in a solid...
        if trace.allsolid && !self.pm_correct_all_solid(&mut trace) {
            return;
        }
        // make sure that the surfNormal is reset to the ground
        self.ps.grapple_point = ref_normal;

        // if the trace didn't hit anything, we are in free fall
        if trace.fraction == 1.0 {
            let mut stepped_down = false;
            // try to step down
            if self.pml.ground_plane && self.pm_predict_step_move() {
                let point = [self.ps.origin[0], self.ps.origin[1], self.ps.origin[2] - STEPSIZE as f32];
                trace = self.trace(&self.ps.origin.clone(), &point, self.tracemask);
                // if we hit something
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
                if self.has_ability(ability11::WALLJUMPER) {
                    let mut movedir = project_point_on_plane(&self.pml.forward, &ref_normal);
                    bg11::vector_normalize(&mut movedir);
                    if self.cmd.forwardmove < 0 {
                        movedir = neg(&movedir);
                    }
                    // allow strafe transitions
                    if self.cmd.rightmove != 0 {
                        movedir = self.pml.right;
                        if self.cmd.rightmove < 0 {
                            movedir = neg(&movedir);
                        }
                    }
                    // trace into direction we are moving
                    let point = ma(&self.ps.origin, 0.25, &movedir);
                    let trace = self.trace(&self.ps.origin.clone(), &point, self.tracemask);
                    if trace.fraction < 1.0 && trace.surface_flags & (surface::SKY | surface::SLICK) as i32 == 0 && trace.entity_num == ENTITYNUM_WORLD && !compare(&trace.plane.normal, &self.ps.grapple_point) {
                        self.ps.grapple_point = trace.plane.normal;
                        self.pm_check_wall_jump();
                    }
                }
                return;
            }
        }

        // check if getting thrown off the ground
        if self.ps.velocity[2] > 0.0 && dot(&self.ps.velocity, &trace.plane.normal) > 10.0 {
            // go into jump animation
            self.pm_jump_anim();
            self.ps.ground_entity_num = ENTITYNUM_NONE;
            self.pml.ground_plane = false;
            self.pml.walking = false;
            return;
        }
        // slopes that are too steep will not be considered onground
        if trace.plane.normal[2] < MIN_WALK_NORMAL {
            self.ps.ground_entity_num = ENTITYNUM_NONE;
            self.pml.ground_plane = true;
            self.pml.walking = false;
            return;
        }
        self.pml.ground_plane = true;
        self.pml.walking = true;
        // hitting solid ground will end a waterjump
        if self.ps.pm_flags & k::PMF_TIME_WATERJUMP != 0 {
            self.ps.pm_flags &= !(k::PMF_TIME_WATERJUMP | k::PMF_TIME_LAND);
            self.ps.pm_time = 0;
        }
        if self.ps.ground_entity_num == ENTITYNUM_NONE {
            // just hit the ground
            if self.has_ability(ability11::TAKESFALLDAMAGE) {
                self.pm_crash_land();
            }
            // don't do landing time if we were just going down a slope
            if self.pml.previous_velocity[2] < -200.0 {
                // don't allow another jump for a little while
                self.ps.pm_flags |= k::PMF_TIME_LAND;
                self.ps.pm_time = 250;
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

    /// `PM_CheckDuck`: sets mins, maxs, and ps->viewheight.
    fn pm_check_duck(&mut self) {
        let (pmins, pmaxs, pcmaxs, _, _) = bg11::class_bbox(self.pclass());
        let (pcvh_stand, mut pccvh) = bg11::class_viewheight(self.pclass());
        // iD bug? you can still crouch when you're a spectator
        if self.ps.persistant[P_TEAM] == TEAM_SPECTATOR {
            pccvh = pcvh_stand;
        }
        self.mins[0] = pmins[0];
        self.mins[1] = pmins[1];
        self.maxs[0] = pmaxs[0];
        self.maxs[1] = pmaxs[1];
        self.mins[2] = pmins[2];
        if self.ps.pm_type == PmType11::Dead as i32 {
            self.maxs[2] = -8.0;
            self.ps.viewheight = k::DEAD_VIEWHEIGHT;
            return;
        }
        // If the standing and crouching viewheights are the same the class can't crouch
        if self.cmd.upmove < 0 && pcvh_stand != pccvh && self.ps.pm_type != PmType11::Jetpack as i32 && !bg11::inventory_contains_upgrade(Upgrade11Id::Bsuit as i32, &self.ps.stats) {
            // duck
            self.ps.pm_flags |= k::PMF_DUCKED;
        } else if self.ps.pm_flags & k::PMF_DUCKED != 0 {
            // try to stand up
            self.maxs[2] = pmaxs[2];
            let o = self.ps.origin;
            let trace = self.trace(&o, &o, self.tracemask);
            if !trace.allsolid {
                self.ps.pm_flags &= !k::PMF_DUCKED;
            }
        }
        if self.ps.pm_flags & k::PMF_DUCKED != 0 {
            self.maxs[2] = pcmaxs[2];
            self.ps.viewheight = pccvh;
        } else {
            self.maxs[2] = pmaxs[2];
            self.ps.viewheight = pcvh_stand;
        }
    }

    /// `PM_Footsteps`.
    fn pm_footsteps(&mut self) {
        let v = self.ps.velocity;
        if self.has_ability(ability11::WALLCLIMBER) && self.pml.ground_plane {
            self.xyspeed = ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt() as f32;
        } else {
            self.xyspeed = ((v[0] * v[0] + v[1] * v[1]) as f64).sqrt() as f32;
        }
        let ns = self.nonseg();
        if self.ps.ground_entity_num == ENTITYNUM_NONE {
            // airborne leaves position in cycle intact, but doesn't advance
            if self.waterlevel > 1 {
                self.pm_continue_legs_anim(if !ns { A::LegsSwim as i32 } else { Nspa::Swim as i32 });
            }
            return;
        }
        let (f, r) = (self.cmd.forwardmove, self.cmd.rightmove);
        // if not trying to move
        if f == 0 && r == 0 {
            if self.xyspeed < 5.0 {
                self.ps.bob_cycle = 0; // start at beginning of cycle again
                if self.ps.pm_flags & k::PMF_DUCKED != 0 {
                    self.pm_continue_legs_anim(if !ns { A::LegsIdlecr as i32 } else { Nspa::Stand as i32 });
                } else {
                    self.pm_continue_legs_anim(if !ns { A::LegsIdle as i32 } else { Nspa::Stand as i32 });
                }
            }
            return;
        }
        let side = |walk: bool, back: bool| -> i32 {
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
            bobmove = 0.5; // ducked characters bob much faster
            let a = if backwards {
                if !ns { A::LegsBackcr as i32 } else { side(true, true) }
            } else if !ns {
                A::LegsWalkcr as i32
            } else {
                side(true, false)
            };
            self.pm_continue_legs_anim(a);
            // ducked characters never play footsteps
        } else if self.cmd.buttons & k::BUTTON_WALKING == 0 {
            bobmove = 0.4; // faster speeds bob faster
            let a = if self.is(Weapon11Id::Level4) && self.ps.pm_flags & k::PMF_CHARGE != 0 {
                Nspa::Charge as i32
            } else if backwards {
                if !ns { A::LegsBack as i32 } else { side(false, true) }
            } else if !ns {
                A::LegsRun as i32
            } else {
                side(false, false)
            };
            self.pm_continue_legs_anim(a);
            footstep = true;
        } else {
            bobmove = 0.3; // walking bobs slow
            let a = if backwards {
                if !ns { A::LegsBackwalk as i32 } else { side(true, true) }
            } else if !ns {
                A::LegsWalk as i32
            } else {
                side(true, false)
            };
            self.pm_continue_legs_anim(a);
        }
        bobmove *= bg11::bob_cycle(self.pclass());
        if self.state(k::SS_SPEEDBOOST) {
            bobmove *= k::HUMAN_SPRINT_MODIFIER;
        }
        // check for footstep / splash sounds
        let old = self.ps.bob_cycle;
        self.ps.bob_cycle = ((old as f32 + bobmove * self.pml.msec as f32) as i32) & 255;
        // if we just crossed a cycle boundary, play an apropriate footstep event
        if ((old + 64) ^ (self.ps.bob_cycle + 64)) & 128 != 0 {
            match self.waterlevel {
                0 => {
                    // on ground will only play sounds if running
                    if footstep && !self.no_footsteps {
                        let e = self.pm_footstep_for_surface();
                        self.pm_add_event(e);
                    }
                }
                1 => self.ev(Event11::Footsplash), // splashing
                2 => self.ev(Event11::Swim),       // wading / swimming at surface
                _ => {}                            // no sound when completely underwater
            }
        }
    }

    /// `PM_WaterEvents`.
    fn pm_water_events(&mut self) {
        let prev = self.pml.previous_waterlevel;
        if prev == 0 && self.waterlevel != 0 {
            self.ev(Event11::WaterTouch);
        }
        if prev != 0 && self.waterlevel == 0 {
            self.ev(Event11::WaterLeave);
        }
        if prev != 3 && self.waterlevel == 3 {
            self.ev(Event11::WaterUnder);
        }
        if prev == 3 && self.waterlevel != 3 {
            self.ev(Event11::WaterClear);
        }
    }

    /// `PM_BeginWeaponChange`.
    fn pm_begin_weapon_change(&mut self, weapon: i32) {
        if weapon < 0 || weapon >= Weapon11Id::COUNT as i32 + 1 {
            return;
        }
        if !bg11::inventory_contains_weapon(weapon, &self.ps.stats) && weapon != 0 {
            return;
        }
        if self.ps.weaponstate == WeaponState::Dropping as i32 {
            return;
        }
        // special case to prevent storing a charged up lcannon
        if self.is(Weapon11Id::Lcannon) {
            self.ps.stats[ST_MISC] = 0;
        }
        self.ev(Event11::ChangeWeapon);
        self.ps.weaponstate = WeaponState::Dropping as i32;
        self.ps.weapon_time += 200;
        self.ps.persistant[P_NEWWEAPON] = weapon;
        // reset build weapon
        self.ps.stats[ST_BUILDABLE] = 0;
        if !self.nonseg() {
            self.pm_start_torso_anim(A::TorsoDrop as i32);
        }
    }

    /// `PM_FinishWeaponChange`.
    fn pm_finish_weapon_change(&mut self) {
        let mut weapon = self.ps.persistant[P_NEWWEAPON];
        if weapon < 0 || weapon >= Weapon11Id::COUNT as i32 + 1 {
            weapon = 0;
        }
        if !bg11::inventory_contains_weapon(weapon, &self.ps.stats) {
            weapon = 0;
        }
        self.ps.weapon = weapon;
        self.ps.weaponstate = WeaponState::Raising as i32;
        self.ps.weapon_time += 250;
        if !self.nonseg() {
            self.pm_start_torso_anim(A::TorsoRaise as i32);
        }
    }

    /// `PM_TorsoAnimation`.
    fn pm_torso_animation(&mut self) {
        if self.nonseg() {
            return;
        }
        if self.ps.weaponstate == WeaponState::Ready as i32 {
            let a = if self.is(Weapon11Id::Blaster) { A::TorsoStand2 } else { A::TorsoStand };
            self.pm_continue_torso_anim(a as i32);
        }
    }

    fn weapon_ready(&mut self) {
        self.ps.weapon_time = 0;
        self.ps.weaponstate = WeaponState::Ready as i32;
    }

    /// `PM_Weapon`: generates weapon events and modifes the weapon counter.
    fn pm_weapon(&mut self) {
        let mut add_time = 200; // default addTime - should never be used
        let (mut attack1, attack2, attack3);
        // don't allow attack until all buttons are up
        if self.ps.pm_flags & k::PMF_RESPAWNED != 0 {
            return;
        }
        // ignore if spectator
        if self.ps.persistant[P_TEAM] == TEAM_SPECTATOR {
            return;
        }
        if self.state(k::SS_INFESTING) {
            return;
        }
        if self.state(k::SS_HOVELING) {
            return;
        }
        // check for dead player
        if self.ps.stats[ST_HEALTH] <= 0 {
            self.ps.weapon = 0;
            return;
        }
        // make weapon function
        if self.ps.weapon_time > 0 {
            self.ps.weapon_time -= self.pml.msec;
        }
        // check for weapon change: can't change if weapon is firing, but can
        // change again if lowering or raising
        if self.ps.weapon_time <= 0 || self.ps.weaponstate != WeaponState::Firing as i32 {
            // must press use to switch weapons
            if self.cmd.buttons & k::BUTTON_USE_HOLDABLE != 0 {
                if self.ps.pm_flags & k::PMF_USE_ITEM_HELD == 0 {
                    let w = self.cmd.weapon as i32;
                    if w <= 32 {
                        // if trying to select a weapon, select it
                        if self.ps.weapon != w {
                            self.pm_begin_weapon_change(w);
                        }
                    } else if w > 32 {
                        // if trying to toggle an upgrade, toggle it
                        if bg11::inventory_contains_upgrade(w - 32, &self.ps.stats) {
                            if bg11::upgrade_is_active(w - 32, &self.ps.stats) {
                                bg11::deactivate_upgrade(w - 32, &mut self.ps.stats);
                            } else {
                                bg11::activate_upgrade(w - 32, &mut self.ps.stats);
                            }
                        }
                    }
                    self.ps.pm_flags |= k::PMF_USE_ITEM_HELD;
                }
            } else {
                self.ps.pm_flags &= !k::PMF_USE_ITEM_HELD;
            }
            // something external thinks a weapon change is necessary
            if self.ps.pm_flags & k::PMF_WEAPON_SWITCH != 0 {
                self.ps.pm_flags &= !k::PMF_WEAPON_SWITCH;
                let nw = self.ps.persistant[P_NEWWEAPON];
                self.pm_begin_weapon_change(nw);
            }
        }
        if self.ps.weapon_time > 0 {
            return;
        }
        // change weapon if time
        if self.ps.weaponstate == WeaponState::Dropping as i32 {
            self.pm_finish_weapon_change();
            return;
        }
        if self.ps.weaponstate == WeaponState::Raising as i32 {
            self.ps.weaponstate = WeaponState::Ready as i32;
            if !self.nonseg() {
                let a = if self.is(Weapon11Id::Blaster) { A::TorsoStand2 } else { A::TorsoStand };
                self.pm_continue_torso_anim(a as i32);
            }
            return;
        }
        // start the animation even if out of ammo
        let (mut ammo, mut clips) = bg11::unpack_ammo_array(self.ps.weapon, &self.ps.ammo, &self.ps.powerups);
        let max_clips = bg11::ammo_for_weapon(self.ps.weapon).map(|x| x.1).unwrap_or(0);
        // check for out of ammo
        if ammo == 0 && clips == 0 && !bg11::infinite_ammo(self.ps.weapon) {
            self.ev(Event11::Noammo);
            self.ps.weapon_time += 200;
            return;
        }
        // done reloading so give em some ammo
        if self.ps.weaponstate == WeaponState::Reloading as i32 {
            if max_clips > 0 {
                clips -= 1;
                if let Some((max_ammo, _)) = bg11::ammo_for_weapon(self.ps.weapon) {
                    ammo = max_ammo;
                }
            }
            if bg11::uses_energy(self.ps.weapon) && bg11::inventory_contains_upgrade(Upgrade11Id::Battpack as i32, &self.ps.stats) {
                ammo = (ammo as f32 * k::BATTPACK_MODIFIER) as i32;
            }
            bg11::pack_ammo_array(self.ps.weapon, &mut self.ps.ammo, &mut self.ps.powerups, ammo, clips);
            // allow some time for the weapon to be raised
            self.ps.weaponstate = WeaponState::Raising as i32;
            self.pm_start_torso_anim(A::TorsoRaise as i32);
            self.ps.weapon_time += 250;
            return;
        }
        // check for end of clip
        if (ammo == 0 || self.ps.pm_flags & k::PMF_WEAPON_RELOAD != 0) && clips != 0 {
            self.ps.pm_flags &= !k::PMF_WEAPON_RELOAD;
            self.ps.weaponstate = WeaponState::Reloading as i32;
            // drop the weapon
            self.pm_start_torso_anim(A::TorsoDrop as i32);
            add_time = bg11::reload_time(self.ps.weapon);
            self.ps.weapon_time += add_time;
            return;
        }
        let w = self.ps.weapon;
        let auto_hit = self.auto_weapon_hit[(w & 31) as usize];
        let buttons = self.cmd.buttons;
        // check if non-auto primary/secondary attacks are permited
        if w == wp(Weapon11Id::Level0) {
            // venom is only autohit
            attack1 = false;
            attack2 = false;
            attack3 = false;
            if !auto_hit {
                self.weapon_ready();
                return;
            }
        } else if w == wp(Weapon11Id::Level3) || w == wp(Weapon11Id::Level3upg) {
            // pouncing has primary secondary AND autohit procedures
            attack1 = buttons & k::BUTTON_ATTACK != 0;
            attack2 = buttons & k::BUTTON_ATTACK2 != 0;
            attack3 = buttons & k::BUTTON_USE_HOLDABLE != 0;
            if !auto_hit && !attack1 && !attack2 && !attack3 {
                self.weapon_ready();
                return;
            }
        } else if w == wp(Weapon11Id::Lcannon) {
            attack1 = buttons & k::BUTTON_ATTACK != 0;
            attack2 = buttons & k::BUTTON_ATTACK2 != 0;
            attack3 = buttons & k::BUTTON_USE_HOLDABLE != 0;
            if (attack1 || self.ps.stats[ST_MISC] == 0) && !attack2 && !attack3 {
                if self.ps.stats[ST_MISC] < k::LCANNON_TOTAL_CHARGE {
                    self.weapon_ready();
                    return;
                } else {
                    attack1 = !attack1;
                }
            }
            // erp this looks confusing
            if self.ps.stats[ST_MISC] > k::LCANNON_MIN_CHARGE {
                attack1 = !attack1;
            } else if self.ps.stats[ST_MISC] > 0 {
                self.ps.stats[ST_MISC] = 0;
                self.weapon_ready();
                return;
            }
        } else {
            // by default primary and secondary attacks are allowed
            attack1 = buttons & k::BUTTON_ATTACK != 0;
            attack2 = buttons & k::BUTTON_ATTACK2 != 0;
            attack3 = buttons & k::BUTTON_USE_HOLDABLE != 0;
            if !attack1 && !attack2 && !attack3 {
                self.weapon_ready();
                return;
            }
        }
        // fire events for non auto weapons
        if attack3 {
            if bg11::has_third_mode(w) {
                // hacky special case for slowblob
                if w == wp(Weapon11Id::Level3upg) && ammo == 0 {
                    self.ev(Event11::Noammo);
                    self.ps.weapon_time += 200;
                    return;
                }
                self.ps.generic1 = WeaponMode::Tertiary as i32;
                self.ev(Event11::FireWeapon3);
                add_time = bg11::repeat_rate3(w);
            } else {
                self.weapon_ready();
                self.ps.generic1 = WeaponMode::Notfiring as i32;
                return;
            }
        } else if attack2 {
            if bg11::has_alt_mode(w) {
                self.ps.generic1 = WeaponMode::Secondary as i32;
                self.ev(Event11::FireWeapon2);
                add_time = bg11::repeat_rate2(w);
            } else {
                self.weapon_ready();
                self.ps.generic1 = WeaponMode::Notfiring as i32;
                return;
            }
        } else if attack1 {
            self.ps.generic1 = WeaponMode::Primary as i32;
            self.ev(Event11::FireWeapon);
            add_time = bg11::repeat_rate1(w);
        }
        // fire events for autohit weapons
        if auto_hit {
            if w == wp(Weapon11Id::Level0) {
                self.ps.generic1 = WeaponMode::Primary as i32;
                self.ev(Event11::FireWeapon);
                add_time = bg11::repeat_rate1(w);
            } else if w == wp(Weapon11Id::Level3) || w == wp(Weapon11Id::Level3upg) {
                self.ps.generic1 = WeaponMode::Secondary as i32;
                self.ev(Event11::FireWeapon2);
                add_time = bg11::repeat_rate2(w);
            }
        }
        if !self.nonseg() {
            if w == wp(Weapon11Id::Flamer) {
                if self.ps.weaponstate == WeaponState::Ready as i32 {
                    self.pm_start_torso_anim(A::TorsoAttack as i32);
                }
            } else if w == wp(Weapon11Id::Blaster) {
                self.pm_start_torso_anim(A::TorsoAttack2 as i32);
            } else {
                self.pm_start_torso_anim(A::TorsoAttack as i32);
            }
        } else {
            if w == wp(Weapon11Id::Level4) {
                // hack to get random attack animations
                let num = self.ps.weapon_time.abs() % 3;
                if num == 0 {
                    self.pm_force_legs_anim(Nspa::Attack1 as i32);
                } else if num == 1 {
                    self.pm_force_legs_anim(Nspa::Attack2 as i32);
                } else if num == 2 {
                    self.pm_force_legs_anim(Nspa::Attack3 as i32);
                }
            } else if attack1 {
                self.pm_force_legs_anim(Nspa::Attack1 as i32);
            } else if attack2 {
                self.pm_force_legs_anim(Nspa::Attack2 as i32);
            } else if attack3 {
                self.pm_force_legs_anim(Nspa::Attack3 as i32);
            }
            self.ps.torso_timer = k::TIMER_ATTACK;
        }
        self.ps.weaponstate = WeaponState::Firing as i32;
        // take an ammo away if not infinite
        if !bg11::infinite_ammo(w) {
            // special case for lCanon
            if w == wp(Weapon11Id::Lcannon) && attack1 {
                ammo -= ((self.ps.stats[ST_MISC] as f32 / k::LCANNON_TOTAL_CHARGE as f32) * 10.0f32).ceil() as i32;
                // stay on the safe side
                if ammo < 0 {
                    ammo = 0;
                }
            } else {
                ammo -= 1;
            }
            bg11::pack_ammo_array(w, &mut self.ps.ammo, &mut self.ps.powerups, ammo, clips);
        } else if w == wp(Weapon11Id::Level3upg) && attack3 {
            // special case for slowblob
            ammo -= 1;
            bg11::pack_ammo_array(w, &mut self.ps.ammo, &mut self.ps.powerups, ammo, clips);
        }
        // FIXME: predicted angles miss a problem??
        if w == wp(Weapon11Id::Chaingun) {
            let at = add_time as f32;
            if self.ps.pm_flags & k::PMF_DUCKED != 0 || bg11::inventory_contains_upgrade(Upgrade11Id::Bsuit as i32, &self.ps.stats) {
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
        if self.cmd.buttons & k::BUTTON_GESTURE != 0 && self.ps.torso_timer == 0 {
            if !self.nonseg() {
                self.pm_start_torso_anim(A::TorsoGesture as i32);
            } else {
                self.pm_force_legs_anim(Nspa::Gesture as i32);
            }
            self.ps.torso_timer = k::TIMER_GESTURE;
            self.ev(Event11::Taunt);
        }
    }

    /// `PM_DropTimers`.
    fn pm_drop_timers(&mut self) {
        let msec = self.pml.msec;
        // drop misc timing counter
        if self.ps.pm_time != 0 {
            if msec >= self.ps.pm_time {
                self.ps.pm_flags &= !k::PMF_ALL_TIMES;
                self.ps.pm_time = 0;
            } else {
                self.ps.pm_time -= msec;
            }
        }
        // drop animation counter
        for t in [&mut self.ps.legs_timer, &mut self.ps.torso_timer] {
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
        let (ammo, clips) = bg11::unpack_ammo_array(self.ps.weapon, &self.ps.ammo, &self.ps.powerups);
        // clear results
        self.numtouch = 0;
        self.watertype = 0;
        self.waterlevel = 0;
        if self.ps.stats[ST_HEALTH] <= 0 {
            self.tracemask &= !(contents::BODY as i32); // corpses can fly through bodies
        }
        // make sure walking button is clear if they are running, to avoid
        // proxy no-footsteps cheats
        if (self.cmd.forwardmove as i32).abs() > 64 || (self.cmd.rightmove as i32).abs() > 64 {
            self.cmd.buttons &= !k::BUTTON_WALKING;
        }
        // set the talk balloon flag
        if self.cmd.buttons & k::BUTTON_TALK != 0 {
            self.ps.e_flags |= k::EF_TALK;
        } else {
            self.ps.e_flags &= !k::EF_TALK;
        }
        // set the firing flags for continuous beam weapons
        let can_fire = self.ps.pm_flags & k::PMF_RESPAWNED == 0 && self.ps.pm_type != PmType11::Intermission as i32 && ((ammo > 0 || clips > 0) || bg11::infinite_ammo(self.ps.weapon));
        for (button, flag) in [(k::BUTTON_ATTACK, k::EF_FIRING), (k::BUTTON_ATTACK2, k::EF_FIRING2), (k::BUTTON_USE_HOLDABLE, k::EF_FIRING3)] {
            if can_fire && self.cmd.buttons & button != 0 {
                self.ps.e_flags |= flag;
            } else {
                self.ps.e_flags &= !flag;
            }
        }
        // clear the respawned flag if attack and use are cleared
        if self.ps.stats[ST_HEALTH] > 0 && self.cmd.buttons & (k::BUTTON_ATTACK | k::BUTTON_USE_HOLDABLE) == 0 {
            self.ps.pm_flags &= !k::PMF_RESPAWNED;
        }
        // if talk button is down, dissallow all other input
        if self.cmd.buttons & k::BUTTON_TALK != 0 {
            self.cmd.buttons = k::BUTTON_TALK;
            self.cmd.forwardmove = 0;
            self.cmd.rightmove = 0;
            self.cmd.upmove = 0;
        }
        // clear all pmove local vars
        self.pml = Pml::default();
        // determine the time
        self.pml.msec = (self.cmd.server_time - self.ps.command_time).clamp(1, 200);
        self.ps.command_time = self.cmd.server_time;
        // save old org in case we get stuck
        self.pml.previous_origin = self.ps.origin;
        // save old velocity for crashlanding
        self.pml.previous_velocity = self.ps.velocity;
        self.pml.frametime = (self.pml.msec as f64 * 0.001) as f32;
        let (f, r, u) = angle_vectors(&self.ps.viewangles);
        self.pml.forward = f;
        self.pml.right = r;
        self.pml.up = u;
        if self.cmd.upmove < 10 {
            // not holding jump
            self.ps.pm_flags &= !k::PMF_JUMP_HELD;
        }
        // decide if backpedaling animations should be used
        if self.cmd.forwardmove < 0 {
            self.ps.pm_flags |= k::PMF_BACKWARDS_RUN;
        } else if self.cmd.forwardmove > 0 || (self.cmd.forwardmove == 0 && self.cmd.rightmove != 0) {
            self.ps.pm_flags &= !k::PMF_BACKWARDS_RUN;
        }
        if self.ps.pm_type >= PmType11::Dead as i32 {
            self.cmd.forwardmove = 0;
            self.cmd.rightmove = 0;
            self.cmd.upmove = 0;
        }
        if self.ps.pm_type == PmType11::Spectator as i32 {
            // update the viewangles
            update_view_angles(self.ps, &self.cmd);
            self.pm_check_duck();
            self.pm_fly_move();
            self.pm_drop_timers();
            return;
        }
        if self.ps.pm_type == PmType11::Noclip as i32 {
            update_view_angles(self.ps, &self.cmd);
            self.pm_noclip_move();
            self.pm_drop_timers();
            return;
        }
        if self.ps.pm_type == PmType11::Freeze as i32 {
            return; // no movement at all
        }
        if self.ps.pm_type == PmType11::Intermission as i32 || self.ps.pm_type == PmType11::Spintermission as i32 {
            return; // no movement at all
        }
        // set watertype, and waterlevel
        self.pm_set_water_level();
        self.pml.previous_waterlevel = self.waterlevel;
        // set mins, maxs, and viewheight
        self.pm_check_duck();
        self.pm_check_ladder();
        // set groundentity
        self.pm_ground_trace();
        // update the viewangles
        update_view_angles(self.ps, &self.cmd);
        if self.ps.pm_type == PmType11::Dead as i32 || self.ps.pm_type == PmType11::Grabbed as i32 {
            self.pm_dead_move();
        }
        self.pm_drop_timers();
        if self.ps.pm_type == PmType11::Jetpack as i32 {
            self.pm_jet_pack_move();
        } else if self.ps.pm_flags & k::PMF_TIME_WATERJUMP != 0 {
            self.pm_water_jump_move();
        } else if self.waterlevel > 1 {
            self.pm_water_move();
        } else if self.pml.ladder {
            self.pm_ladder_move();
        } else if self.pml.walking {
            if self.has_ability(ability11::WALLCLIMBER) && self.state(k::SS_WALLCLIMBING) {
                self.pm_climb_move(); // walking on any surface
            } else {
                self.pm_walk_move(); // walking on ground
            }
        } else {
            self.pm_air_move();
        }
        self.pm_animate();
        // set groundentity, watertype, and waterlevel
        self.pm_ground_trace();
        // must update after every GroundTrace()
        update_view_angles(self.ps, &self.cmd);
        self.pm_set_water_level();
        // weapons
        self.pm_weapon();
        // torso animation
        self.pm_torso_animation();
        // footstep events / legs animations
        self.pm_footsteps();
        // entering / leaving water splashes
        self.pm_water_events();
        // snap some parts of playerstate to save network bandwidth (round-half-even, dec009)
        for i in 0..3 {
            self.ps.velocity[i] = self.ps.velocity[i].round_ties_even();
        }
    }

    /// `Pmove`: chops long moves into <= 66 ms (or pmove_msec) steps.
    pub fn pmove(&mut self) {
        let final_time = self.cmd.server_time;
        if final_time < self.ps.command_time {
            return; // should not happen
        }
        if final_time > self.ps.command_time + 1000 {
            self.ps.command_time = final_time - 1000;
        }
        self.ps.pmove_framecount = (self.ps.pmove_framecount + 1) & ((1 << 6) - 1);
        // chop the move up if it is too long, to prevent framerate dependent behavior
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

    // ============================================================ bg_slidemove.c

    /// `PM_SlideMove`: returns true if the velocity was clipped in some way.
    pub fn pm_slide_move(&mut self, gravity: bool) -> bool {
        const MAX_CLIP_PLANES: usize = 5;
        let numbumps = 4;
        let mut primal_velocity = self.ps.velocity;
        // the C leaves endVelocity uninitialised when !gravity; it is then only
        // ever clipped and discarded, so any value is equivalent
        let mut end_velocity = [0.0f32; 3];
        if gravity {
            end_velocity = self.ps.velocity;
            end_velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
            self.ps.velocity[2] = ((self.ps.velocity[2] + end_velocity[2]) as f64 * 0.5) as f32;
            primal_velocity[2] = end_velocity[2];
            if self.pml.ground_plane {
                // slide along the ground plane
                self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &self.pml.ground_trace.plane.normal, OVERCLIP);
            }
        }
        let mut time_left = self.pml.frametime;
        let mut planes = [[0.0f32; 3]; MAX_CLIP_PLANES];
        // never turn against the ground plane
        let mut numplanes = if self.pml.ground_plane {
            planes[0] = self.pml.ground_trace.plane.normal;
            1
        } else {
            0
        };
        // never turn against original velocity
        bg11::vector_normalize2(&self.ps.velocity, &mut planes[numplanes]);
        numplanes += 1;
        let mut bumpcount = 0;
        while bumpcount < numbumps {
            // calculate position we are trying to move to
            let end = ma(&self.ps.origin, time_left, &self.ps.velocity);
            // see if we can make it there
            let trace = self.world.trace(&self.ps.origin, Some(&self.mins), Some(&self.maxs), &end, self.ps.client_num, self.tracemask);
            if trace.allsolid {
                // entity is completely trapped in another solid
                self.ps.velocity[2] = 0.0; // don't build up falling damage, but allow sideways acceleration
                return true;
            }
            if trace.fraction > 0.0 {
                // actually covered some distance
                self.ps.origin = trace.endpos;
            }
            if trace.fraction == 1.0 {
                break; // moved the entire distance
            }
            // save entity for contact
            self.pm_add_touch_ent(trace.entity_num);
            time_left -= time_left * trace.fraction;
            if numplanes >= MAX_CLIP_PLANES {
                // this shouldn't really happen
                self.ps.velocity = [0.0; 3];
                return true;
            }
            // if this is the same plane we hit before, nudge velocity
            // out along it, which fixes some epsilon issues with
            // non-axial planes
            let mut same = false;
            for p in planes.iter().take(numplanes) {
                if (dot(&trace.plane.normal, p) as f64) > 0.99 {
                    self.ps.velocity = add(&trace.plane.normal, &self.ps.velocity);
                    same = true;
                    break;
                }
            }
            if same {
                bumpcount += 1;
                continue;
            }
            planes[numplanes] = trace.plane.normal;
            numplanes += 1;
            // modify velocity so it parallels all of the clip planes
            // find a plane that it enters
            for i in 0..numplanes {
                let into = dot(&self.ps.velocity, &planes[i]);
                if (into as f64) >= 0.1 {
                    continue; // move doesn't interact with the plane
                }
                // see how hard we are hitting things
                if -into > self.pml.impact_speed {
                    self.pml.impact_speed = -into;
                }
                // slide along the plane
                let mut clip_velocity = Self::clip_velocity(&self.ps.velocity, &planes[i], OVERCLIP);
                // slide along the plane
                let mut end_clip_velocity = Self::clip_velocity(&end_velocity, &planes[i], OVERCLIP);
                // see if there is a second plane that the new move enters
                for j in 0..numplanes {
                    if j == i {
                        continue;
                    }
                    if (dot(&clip_velocity, &planes[j]) as f64) >= 0.1 {
                        continue; // move doesn't interact with the plane
                    }
                    // try clipping the move to the plane
                    clip_velocity = Self::clip_velocity(&clip_velocity, &planes[j], OVERCLIP);
                    end_clip_velocity = Self::clip_velocity(&end_clip_velocity, &planes[j], OVERCLIP);
                    // see if it goes back into the first clip plane
                    if dot(&clip_velocity, &planes[i]) >= 0.0 {
                        continue;
                    }
                    // slide the original velocity along the crease
                    let mut dir = cross(&planes[i], &planes[j]);
                    bg11::vector_normalize(&mut dir);
                    let d = dot(&dir, &self.ps.velocity);
                    clip_velocity = scale(&dir, d);
                    let mut dir = cross(&planes[i], &planes[j]);
                    bg11::vector_normalize(&mut dir);
                    let d = dot(&dir, &end_velocity);
                    end_clip_velocity = scale(&dir, d);
                    // see if there is a third plane the the new move enters
                    for kk in 0..numplanes {
                        if kk == i || kk == j {
                            continue;
                        }
                        if (dot(&clip_velocity, &planes[kk]) as f64) >= 0.1 {
                            continue; // move doesn't interact with the plane
                        }
                        // stop dead at a tripple plane interaction
                        self.ps.velocity = [0.0; 3];
                        return true;
                    }
                }
                // if we have fixed all interactions, try another move
                self.ps.velocity = clip_velocity;
                end_velocity = end_clip_velocity;
                break;
            }
            bumpcount += 1;
        }
        if gravity {
            self.ps.velocity = end_velocity;
        }
        // don't change velocity if in a timer (FIXME: is this correct?)
        if self.ps.pm_time != 0 {
            self.ps.velocity = primal_velocity;
        }
        bumpcount != 0
    }

    /// `PM_StepEvent`.
    pub fn pm_step_event(&mut self, from: &Vec3, to: &Vec3, normal: &Vec3) {
        let delta = sub(from, to);
        let mut d_normal = delta;
        bg11::vector_normalize(&mut d_normal);
        let mut size = dot(normal, &d_normal) * length(&delta);
        let ev = if size > 0.0 {
            if size > 2.0 {
                Some(if size < 7.0 {
                    Event11::Stepdn4
                } else if size < 11.0 {
                    Event11::Stepdn8
                } else if size < 15.0 {
                    Event11::Stepdn12
                } else {
                    Event11::Stepdn16
                })
            } else {
                None
            }
        } else {
            size = (size as f64).abs() as f32;
            if size > 2.0 {
                Some(if size < 7.0 {
                    Event11::Step4
                } else if size < 11.0 {
                    Event11::Step8
                } else if size < 15.0 {
                    Event11::Step12
                } else {
                    Event11::Step16
                })
            } else {
                None
            }
        };
        if let Some(e) = ev {
            self.ev(e);
        }
    }

    /// `PM_StepSlideMove`.
    pub fn pm_step_slide_move(&mut self, gravity: bool, predictive: bool) -> bool {
        let mut stepped = false;
        let normal: Vec3 = if self.state(k::SS_WALLCLIMBING) { self.surf_normal() } else { [0.0, 0.0, 1.0] };
        let start_o = self.ps.origin;
        let start_v = self.ps.velocity;
        if !self.pm_slide_move(gravity) {
            let down = ma(&start_o, -STEPSIZE as f32, &normal);
            let trace = self.world.trace(&start_o, Some(&self.mins), Some(&self.maxs), &down, self.ps.client_num, self.tracemask);
            // we can step down
            if trace.fraction > 0.01 && trace.fraction < 1.0 && !trace.allsolid && self.pml.ground_plane {
                stepped = true;
            }
        } else {
            let down = ma(&start_o, -STEPSIZE as f32, &normal);
            let trace = self.world.trace(&start_o, Some(&self.mins), Some(&self.maxs), &down, self.ps.client_num, self.tracemask);
            // never step up when you still have up velocity
            if dot(&trace.plane.normal, &self.ps.velocity) > 0.0 && (trace.fraction == 1.0 || dot(&trace.plane.normal, &normal) < 0.7) {
                return stepped;
            }
            let up = ma(&start_o, STEPSIZE as f32, &normal);
            // test the player position if they were a stepheight higher
            let trace = self.world.trace(&start_o, Some(&self.mins), Some(&self.maxs), &up, self.ps.client_num, self.tracemask);
            if trace.allsolid {
                return stepped; // can't step up
            }
            let step_v = sub(&trace.endpos, &start_o);
            let mut step_v_normal = step_v;
            bg11::vector_normalize(&mut step_v_normal);
            let step_size = dot(&normal, &step_v_normal) * length(&step_v);
            // try slidemove from this position
            self.ps.origin = trace.endpos;
            self.ps.velocity = start_v;
            if !self.pm_slide_move(gravity) {
                stepped = true;
            }
            // push down the final amount
            let down = ma(&self.ps.origin, -step_size, &normal);
            let trace = self.world.trace(&self.ps.origin, Some(&self.mins), Some(&self.maxs), &down, self.ps.client_num, self.tracemask);
            if !trace.allsolid {
                self.ps.origin = trace.endpos;
            }
            if trace.fraction < 1.0 {
                self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &trace.plane.normal, OVERCLIP);
            }
        }
        if !predictive && stepped {
            self.pm_step_event(&start_o, &self.ps.origin.clone(), &normal);
        }
        stepped
    }

    /// `PM_PredictStepMove`.
    pub fn pm_predict_step_move(&mut self) -> bool {
        let velocity = self.ps.velocity;
        let origin = self.ps.origin;
        let impact_speed = self.pml.impact_speed;
        let stepped = self.pm_step_slide_move(false, true);
        self.ps.velocity = velocity;
        self.ps.origin = origin;
        self.pml.impact_speed = impact_speed;
        stepped
    }
}

/// `BG_AddPredictableEventToPlayerstate` on a 1.1 playerState.
pub fn add_predictable_event(ps: &mut PlayerState11, ev: i32, parm: i32) {
    let i = (ps.event_sequence & (k::MAX_PS_EVENTS - 1)) as usize;
    ps.events[i] = ev;
    ps.event_parms[i] = parm;
    ps.event_sequence += 1;
}

/// `PM_UpdateViewAngles` of 1.1.0, including its angle normalisation that
/// leaves every view angle in [180, 540) (`while( tempang[ i ] < 180 )`).
pub fn update_view_angles(ps: &mut PlayerState11, cmd: &UserCmd) {
    if ps.pm_type == PmType11::Intermission as i32 || ps.pm_type == PmType11::Spintermission as i32 {
        return; // no view changes at all
    }
    if ps.pm_type != PmType11::Spectator as i32 && ps.stats[ST_HEALTH] <= 0 {
        return; // no view changes at all
    }
    let mut temp = [0i16; 3];
    let mut tempang = [0.0f32; 3];
    // circularly clamp the angles with deltas
    for i in 0..3 {
        temp[i] = cmd.angles[i].wrapping_add(ps.delta_angles[i]) as i16;
        if i == PITCH {
            // don't let the player look up or down more than 90 degrees
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
    // convert viewangles -> axis
    let axis = angles_to_axis(&tempang);
    let rotaxis = if ps.stats[ST_STATE] & k::SS_WALLCLIMBING == 0 {
        axis
    } else {
        bg11::rotate_axis(&ps.grapple_point, &axis, false, ps.stats[ST_STATE] & k::SS_WALLCLIMBINGCEILING != 0).unwrap_or(axis)
    };
    // convert the new axis back to angles
    let mut tempang = axis_to_angles(&rotaxis);
    // force angles to -180 <= x <= 180 (sic)
    for a in tempang.iter_mut() {
        while *a > 180.0 {
            *a -= 360.0;
        }
        while *a < 180.0 {
            *a += 360.0;
        }
    }
    ps.viewangles = tempang;
    // pull the view into the lock point
    if ps.pm_type == PmType11::Grabbed as i32 && !bg11::inventory_contains_upgrade(Upgrade11Id::Bsuit as i32, &ps.stats) {
        let dir = byte_to_dir(ps.stats[ST_VIEWLOCK]);
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
