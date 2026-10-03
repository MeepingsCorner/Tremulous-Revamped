//! The cgame view (cg_view.c `CG_CalcViewValues` and everything it calls),
//! the player-state transitions that drive it (cg_playerstate.c), the view
//! parts of `CG_EntityEvent` (cg_event.c: landing, step smoothing, wall-jump
//! smoothing, spawn time, pain sounds) and the first-person weapon position
//! (cg_weapons.c `CG_CalculateWeaponPosition` / `CG_AddViewWeapon`).
//!
//! Pure presentation state: `ViewState` plays the part of the `cg_t` fields
//! the C keeps for these functions. Time is `cg.time` in milliseconds on the
//! simulation clock (`level.time` plus the part of a tick already elapsed).

use std::collections::HashMap;
use trem_sim::gen::constants as k;
use trem_sim::gen::enums::{ability, contents, Event, Pers, PmType, Stat, Team};
use trem_sim::gen::weapons::WeaponId;
use trem_sim::kernel::bg;
use trem_sim::kernel::qmath::{
    angle_normalize180, angle_vectors, angles_to_axis, axis_to_angles, compare, cross, dot, length, ma, normalize, project_point_on_plane, rotate_point_around_vector, short2angle, sub, vectoangles, Vec3,
};
use trem_sim::kernel::types::{PlayerState, UserCmd};
use trem_sim::kernel::world::{GameEvent, World, MAX_CLIENTS};

// cg_local.h
pub const DAMAGE_DEFLECT_TIME: f32 = 100.0;
pub const DAMAGE_RETURN_TIME: f32 = 400.0;
pub const DAMAGE_TIME: i32 = 500;
pub const LAND_DEFLECT_TIME: i32 = 150;
pub const LAND_RETURN_TIME: i32 = 300;
pub const DUCK_TIME: i32 = 100;
pub const MAX_STEP_CHANGE: f32 = 32.0;
pub const ZOOM_TIME: i32 = 150;
pub const MUZZLE_FLASH_TIME: i32 = 20;
pub const MAXSMOOTHS: usize = 32;
// cg_view.c
const PCLOUD_ROLL_AMPLITUDE: f32 = 25.0;
const PCLOUD_ROLL_FREQUENCY: f32 = 0.4;
const PCLOUD_ZOOM_AMPLITUDE: f32 = 15.0;
const PCLOUD_ZOOM_FREQUENCY: f32 = 0.625;
const PCLOUD_DISORIENT_DURATION: i32 = 2500;
const LEVEL4_FEEDBACK: f32 = 10.0;
const LEVEL3_FEEDBACK: f32 = 20.0;
const STRUGGLE_DIST: f32 = 5.0;
const STRUGGLE_TIME: f32 = 250.0;
const WAVE_AMPLITUDE: f32 = 1.0;
const WAVE_FREQUENCY: f32 = 0.4;
const FOVWARPTIME: f32 = 400.0;
pub const BASE_FOV_Y: f32 = 73.739_79;
const MAX_FOV_Y: f32 = 120.0;
const MAX_FOV_WARP_Y: f32 = 127.5;
// cg_weapons.c
const SPIN_SPEED: f32 = 0.9;
const COAST_TIME: i32 = 1000;

const M_PI: f32 = std::f32::consts::PI;

/// The cgame cvars these functions read (cg_main.c defaults).
#[derive(Clone, Debug)]
pub struct ViewCvars {
    pub runpitch: f32,
    pub runroll: f32,
    pub gun_x: f32,
    pub gun_y: f32,
    pub gun_z: f32,
    pub draw_gun: i32,
    pub third_person_range: f32,
    pub third_person_angle: f32,
    pub third_person_pitch_follow: i32,
    pub third_person_shoulder_view_mode: i32,
    pub static_death_cam: i32,
    pub ww_smooth_time: i32,
    pub camera_mode: i32,
}

impl Default for ViewCvars {
    fn default() -> Self {
        ViewCvars {
            runpitch: 0.002,
            runroll: 0.005,
            gun_x: 0.0,
            gun_y: 0.0,
            gun_z: 0.0,
            draw_gun: 1,
            third_person_range: 75.0,
            third_person_angle: 0.0,
            third_person_pitch_follow: 0,
            third_person_shoulder_view_mode: 1,
            static_death_cam: 0,
            ww_smooth_time: 300,
            camera_mode: 0,
        }
    }
}

impl ViewCvars {
    /// The names as cg_main.c registers them (`cvarTable`).
    pub const NAMES: &'static [&'static str] = &["cg_runpitch", "cg_runroll", "cg_gunX", "cg_gunY", "cg_gunZ", "cg_drawGun", "cg_thirdPersonRange", "cg_thirdPersonAngle", "cg_thirdPersonPitchFollow", "cg_thirdPersonShoulderViewMode", "cg_staticDeathCam", "cg_wwSmoothTime", "cg_cameraMode"];

    /// Set a cvar by name; None when the name is not one of these.
    pub fn set(&mut self, name: &str, v: &str) -> Option<()> {
        let f = v.parse::<f32>().unwrap_or(0.0);
        let i = trem_sim::kernel::spawn::c_atoi(v);
        match name.to_ascii_lowercase().as_str() {
            "cg_runpitch" => self.runpitch = f,
            "cg_runroll" => self.runroll = f,
            "cg_gunx" => self.gun_x = f,
            "cg_guny" => self.gun_y = f,
            "cg_gunz" => self.gun_z = f,
            "cg_drawgun" => self.draw_gun = i,
            "cg_thirdpersonrange" => self.third_person_range = f,
            "cg_thirdpersonangle" => self.third_person_angle = f,
            "cg_thirdpersonpitchfollow" => self.third_person_pitch_follow = i,
            "cg_thirdpersonshoulderviewmode" => self.third_person_shoulder_view_mode = i,
            "cg_staticdeathcam" => self.static_death_cam = i,
            "cg_wwsmoothtime" => self.ww_smooth_time = i,
            "cg_cameramode" => self.camera_mode = i,
            _ => return None,
        }
        Some(())
    }

    pub fn get(&self, name: &str) -> Option<String> {
        Some(match name.to_ascii_lowercase().as_str() {
            "cg_runpitch" => self.runpitch.to_string(),
            "cg_runroll" => self.runroll.to_string(),
            "cg_gunx" => self.gun_x.to_string(),
            "cg_guny" => self.gun_y.to_string(),
            "cg_gunz" => self.gun_z.to_string(),
            "cg_drawgun" => self.draw_gun.to_string(),
            "cg_thirdpersonrange" => self.third_person_range.to_string(),
            "cg_thirdpersonangle" => self.third_person_angle.to_string(),
            "cg_thirdpersonpitchfollow" => self.third_person_pitch_follow.to_string(),
            "cg_thirdpersonshoulderviewmode" => self.third_person_shoulder_view_mode.to_string(),
            "cg_staticdeathcam" => self.static_death_cam.to_string(),
            "cg_wwsmoothtime" => self.ww_smooth_time.to_string(),
            "cg_cameramode" => self.camera_mode.to_string(),
            _ => return None,
        })
    }
}

/// `smooth_t`: one wall-walk / wall-jump view rotation in progress.
#[derive(Clone, Copy, Debug, Default)]
pub struct Smooth {
    pub rot_axis: Vec3,
    pub rot_angle: f32,
    pub time: i32,
    pub time_mod: f32,
}

/// `cg.refdef` as built by `CG_CalcViewValues`.
#[derive(Clone, Copy, Debug)]
pub struct Refdef {
    pub vieworg: Vec3,
    /// cg.refdefViewAngles
    pub angles: Vec3,
    pub axis: [Vec3; 3],
    /// horizontal fov (the camera projection applies both, see game.rs `CG_CalcVrect`)
    pub fov_x: f32,
    pub fov_y: f32,
    /// cg.renderingThirdPerson
    pub third_person: bool,
    /// (trap_S_Respatialize's underwater flag: no underwater audio here)
    #[allow(dead_code)]
    pub inwater: bool,
    /// `CG_CalcFov` asked for `trap_SendClientCommand( "follow\n" )` this frame
    pub send_follow: bool,
}

/// The `cg_t` view fields.
pub struct ViewState {
    pub cvars: ViewCvars,
    pub time: i32,
    bobcycle: i32,
    bobfracsin: f32,
    xyspeed: f32,
    land_change: f32,
    land_time: i32,
    step_change: f32,
    step_time: i32,
    duck_change: f32,
    duck_time: i32,
    damage_time: i32,
    v_dmg_pitch: f32,
    v_dmg_roll: f32,
    pub damage_x: f32,
    pub damage_y: f32,
    pub damage_value: f32,
    pub v_dmg_time: i32,
    pub attacker_time: i32,
    spawn_time: i32,
    zoomed: bool,
    zoom_time: i32,
    /// cg.zoomSensitivity (sent with the usercmds: scales mouse look)
    pub zoom_sensitivity: f32,
    last_normal: Vec3,
    last_vangles: Vec3,
    s_list: [Smooth; MAXSMOOTHS],
    was_dead_last_frame: bool,
    killer_pos: Vec3,
    tp_pitch: f32,
    shoulder_pitch: f32,
    forward_move_time: i32,
    right_move_time: i32,
    up_move_time: i32,
    /// the previous snapshot player state (CG_TransitionPlayerState `ops`)
    ops: Option<PlayerState>,
    /// centity_t pe.painTime per entity
    pain_time: HashMap<i32, i32>,
    /// cg.predictedPlayerEntity.muzzleFlashTime
    pub muzzle_flash_time: i32,
    /// cg.predictedPlayerEntity.pe barrel spin (CG_MachinegunSpinAngle)
    barrel_time: i32,
    barrel_angle: f32,
    barrel_spinning: bool,
    /// cg.refdef.viewaxis of the previous frame (read by CG_DamageFeedback)
    last_axis: [Vec3; 3],
    /// the usercmd of the previous frame (trap_GetUserCmd( cmdNum - 1 ))
    old_cmd: UserCmd,
    /// rand() for the gun flash roll and the lucifer cannon vibration
    seed: u32,
    /// the last refdef built (debug readout)
    pub last_refdef: Option<Refdef>,
    /// what the presentation drew for the ghost / view weapon (debug readout)
    pub draw_debug: String,
    /// cg.chaseFollow: following in third person (the chase cam)
    pub chase_follow: bool,
    /// noDrift of the view weapon's animation.cfg (`CG_ParseWeaponAnimationFile`)
    pub weapon_no_drift: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            cvars: ViewCvars::default(),
            time: 0,
            bobcycle: 0,
            bobfracsin: 0.0,
            xyspeed: 0.0,
            land_change: 0.0,
            land_time: 0,
            step_change: 0.0,
            step_time: 0,
            duck_change: 0.0,
            duck_time: 0,
            damage_time: 0,
            v_dmg_pitch: 0.0,
            v_dmg_roll: 0.0,
            damage_x: 0.0,
            damage_y: 0.0,
            damage_value: 0.0,
            v_dmg_time: 0,
            attacker_time: 0,
            spawn_time: 0,
            zoomed: false,
            zoom_time: 0,
            zoom_sensitivity: 1.0,
            last_normal: [0.0, 0.0, 1.0],
            last_vangles: [0.0; 3],
            s_list: [Smooth::default(); MAXSMOOTHS],
            was_dead_last_frame: false,
            killer_pos: [0.0; 3],
            tp_pitch: 0.0,
            shoulder_pitch: 0.0,
            forward_move_time: 0,
            right_move_time: 0,
            up_move_time: 0,
            ops: None,
            pain_time: HashMap::new(),
            muzzle_flash_time: -100000,
            barrel_time: 0,
            barrel_angle: 0.0,
            barrel_spinning: false,
            last_axis: [[0.0; 3]; 3],
            old_cmd: UserCmd::default(),
            seed: 0x1234_5678,
            last_refdef: None,
            draw_debug: String::new(),
            chase_follow: false,
            weapon_no_drift: false,
        }
    }
}

fn class_has(class: i32, a: u32) -> bool {
    bg::class_has_ability(class, a)
}

/// `cg.bobcycle`, `cg.bobfracsin`, `cg.xyspeed` from the player state
/// (`CG_CalcViewValues`).
pub fn bob_values(bob_cycle: i32, velocity: &Vec3) -> (i32, f32, f32) {
    let bobcycle = (bob_cycle & 128) >> 7;
    let bobfracsin = (((bob_cycle & 127) as f64 / 127.0 * std::f64::consts::PI).sin()).abs() as f32;
    let mut xyspeed = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
    // the bob velocity should't get too fast to avoid jerking
    if xyspeed > 300.0 {
        xyspeed = 300.0;
    }
    (bobcycle, bobfracsin, xyspeed)
}

/// The remaining step offset of `CG_StepOffset` (subtracted along the
/// surface normal).
pub fn step_offset(step_change: f32, step_time: i32, time: i32, steptime: i32) -> f32 {
    let time_delta = time - step_time;
    if (time_delta as f32) < steptime as f32 {
        step_change * (steptime - time_delta) as f32 / steptime as f32
    } else {
        0.0
    }
}

/// The new `cg.stepChange` after an EV_STEP_* / EV_STEPDN_* event
/// (`CG_EntityEvent`): `step` is +4..16 up or -4..-16 down.
pub fn step_event(step_change: f32, step_time: i32, time: i32, steptime: i32, step: i32) -> f32 {
    let delta = time - step_time;
    let old = if delta < steptime { step_change * (steptime - delta) as f32 / steptime as f32 } else { 0.0 };
    (old + step as f32).clamp(-MAX_STEP_CHANGE, MAX_STEP_CHANGE)
}

/// The land offset `CG_OffsetFirstPersonView` adds to the view height.
pub fn land_offset(land_change: f32, land_time: i32, time: i32) -> f32 {
    let delta = (time - land_time) as f32;
    if delta < LAND_DEFLECT_TIME as f32 {
        land_change * delta / LAND_DEFLECT_TIME as f32
    } else if delta < (LAND_DEFLECT_TIME + LAND_RETURN_TIME) as f32 {
        land_change * (1.0 - (delta - LAND_DEFLECT_TIME as f32) / LAND_RETURN_TIME as f32)
    } else {
        0.0
    }
}

/// The class field of view before zoom (`CG_CalcFov`): fov * 0.75 as an
/// int, clamped, warped from MAX_FOV_WARP_Y after a spawn for FOVWARPS classes.
pub fn class_fov_y(class: i32, spawn_time: i32, time: i32) -> (f32, i32) {
    let attrib_fov = (bg::class(class).fov as f32 * 0.75) as i32;
    let mut fov_y = (attrib_fov as f32).clamp(1.0, MAX_FOV_Y);
    if spawn_time as f32 > (time as f32 - FOVWARPTIME) && class_has(class, ability::FOVWARPS) {
        let fraction = (time - spawn_time) as f32 / FOVWARPTIME;
        fov_y = MAX_FOV_WARP_Y - ((MAX_FOV_WARP_Y - fov_y) * fraction);
    }
    (fov_y, attrib_fov)
}

/// fov_x for a vertical fov and a viewport aspect (width / height).
pub fn fov_x_for(fov_y: f32, aspect: f32) -> f32 {
    let y = 1.0 / (0.5 * fov_y.to_radians()).tan();
    2.0 * aspect.atan2(y).to_degrees()
}

/// `CG_CalculateWeaponPosition` (pure part): the gun origin and angles from
/// the view, bob and land state. `no_drift`: animation.cfg "noDrift".
#[allow(clippy::too_many_arguments)]
pub fn calculate_weapon_position(vieworg: &Vec3, view_angles: &Vec3, bobcycle: i32, bobfracsin: f32, xyspeed: f32, class_bob: f32, no_drift: bool, land_change: f32, land_time: i32, time: i32) -> (Vec3, Vec3) {
    let mut origin = *vieworg;
    let mut angles = *view_angles;
    // on odd legs, invert some angles
    let scale = if bobcycle & 1 != 0 { -xyspeed } else { xyspeed };
    // gun angles from bobbing; bob amount is class dependant
    if class_bob != 0.0 {
        angles[2] += scale * bobfracsin * 0.005;
        angles[1] += scale * bobfracsin * 0.01;
        angles[0] += xyspeed * bobfracsin * 0.005;
    }
    // drop the weapon when landing
    if !no_drift {
        let delta = time - land_time;
        if delta < LAND_DEFLECT_TIME {
            origin[2] += land_change * 0.25 * delta as f32 / LAND_DEFLECT_TIME as f32;
        } else if delta < LAND_DEFLECT_TIME + LAND_RETURN_TIME {
            origin[2] += land_change * 0.25 * (LAND_DEFLECT_TIME + LAND_RETURN_TIME - delta) as f32 / LAND_RETURN_TIME as f32;
        }
        // idle drift
        let scale = xyspeed + 40.0;
        let fracsin = (time as f64 * 0.001).sin() as f32;
        angles[2] += scale * fracsin * 0.01;
        angles[1] += scale * fracsin * 0.01;
        angles[0] += scale * fracsin * 0.01;
    }
    (origin, angles)
}

/// Rotate the three axis rows around `axis` by `deg` (RotatePointAroundVector x3).
fn rotate_axis_rows(inp: &[Vec3; 3], axis: &Vec3, deg: f32) -> [Vec3; 3] {
    [rotate_point_around_vector(axis, &inp[0], deg), rotate_point_around_vector(axis, &inp[1], deg), rotate_point_around_vector(axis, &inp[2], deg)]
}

impl ViewState {
    /// A new map: clear the cg_t state, keep the cvars.
    pub fn reset(&mut self) {
        *self = ViewState { cvars: self.cvars.clone(), ..Default::default() };
    }

    fn rand(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(1103515245).wrapping_add(12345);
        (self.seed >> 16) & 0x7fff
    }
    /// random(): 0..1
    fn random(&mut self) -> f32 {
        (self.rand() & 0x7fff) as f32 / 0x7fff as f32
    }
    /// crandom(): -1..1
    fn crandom(&mut self) -> f32 {
        2.0 * (self.random() - 0.5)
    }

    /// `CG_addSmoothOp`.
    pub fn add_smooth_op(&mut self, rot_axis: Vec3, rot_angle: f32, time_mod: f32) {
        let t = self.time;
        let st = self.cvars.ww_smooth_time;
        // iterate through smooth array
        if let Some(s) = self.s_list.iter_mut().find(|s| s.time + st < t) {
            // found an unused index in the smooth array
            *s = Smooth { rot_axis, rot_angle, time: t, time_mod };
        }
        // no free indices in the smooth array
    }

    /// `CG_smoothWWTransitions`.
    pub fn smooth_ww_transitions(&mut self, ps: &PlayerState, following: bool, inp: &Vec3) -> Vec3 {
        if following {
            return *inp;
        }
        let ref_normal: Vec3 = [0.0, 0.0, 1.0];
        let ceiling_normal: Vec3 = [0.0, 0.0, -1.0];
        // set surfNormal
        let surf_normal = bg::client_normal(ps);
        let mut in_axis = angles_to_axis(inp);
        // if we are moving from one surface to another smooth the transition
        if !compare(&surf_normal, &self.last_normal) {
            let (rot_axis, rot_angle, time_mod);
            // if we moving from the ceiling to the floor special case
            // ( x product of colinear vectors is undefined)
            if compare(&ceiling_normal, &self.last_normal) && compare(&ref_normal, &surf_normal) {
                let (temp, _, _) = angle_vectors(inp);
                let mut a = project_point_on_plane(&temp, &ref_normal);
                normalize(&mut a);
                rot_axis = a;
                rot_angle = 180.0;
                time_mod = 1.5;
            } else {
                let last_axis = angles_to_axis(&self.last_vangles);
                let tr = dot(&in_axis[0], &last_axis[0]) + dot(&in_axis[1], &last_axis[1]) + dot(&in_axis[2], &last_axis[2]);
                // (clamped: the C feeds acos rounding errors just past +-1, a NaN)
                rot_angle = (((tr - 1.0) / 2.0).clamp(-1.0, 1.0) as f64).acos().to_degrees() as f32;
                let mut a = cross(&last_axis[0], &in_axis[0]);
                let b = cross(&last_axis[1], &in_axis[1]);
                let c = cross(&last_axis[2], &in_axis[2]);
                for i in 0..3 {
                    a[i] += b[i] + c[i];
                }
                normalize(&mut a);
                rot_axis = a;
                time_mod = 1.0;
            }
            // add the op
            self.add_smooth_op(rot_axis, rot_angle, time_mod);
        }
        // iterate through ops
        let mut performed = false;
        let mut out_axis = in_axis;
        for i in (0..MAXSMOOTHS).rev() {
            let s = self.s_list[i];
            let smooth_time = (self.cvars.ww_smooth_time as f32 * s.time_mod) as i32 as f32;
            // if this op has time remaining, perform it
            if (self.time as f32) < s.time as f32 + smooth_time {
                let st_local = 1.0 - (((s.time as f32 + smooth_time) - self.time as f32) / smooth_time);
                let s_fraction = -((st_local * M_PI).cos() + 1.0) / 2.0;
                out_axis = rotate_axis_rows(&in_axis, &s.rot_axis, s_fraction * s.rot_angle);
                in_axis = out_axis;
                performed = true;
            }
        }
        // if we performed any ops then return the smoothed angles
        // otherwise simply return the in angles
        let out = if performed { axis_to_angles(&out_axis) } else { *inp };
        // copy the current normal to the lastNormal
        self.last_vangles = *inp;
        self.last_normal = surf_normal;
        out
    }

    /// `CG_smoothWJTransitions`. As in the C, stLocal is an integer
    /// division (remaining time / cg_wwSmoothTime), so it is 0 while an op
    /// runs and the rotation fraction is 0: only the axis round trip remains.
    pub fn smooth_wj_transitions(&mut self, following: bool, inp: &Vec3) -> Vec3 {
        if following {
            return *inp;
        }
        let mut in_axis = angles_to_axis(inp);
        let mut out_axis = in_axis;
        let mut performed = false;
        let st = self.cvars.ww_smooth_time.max(1);
        for i in (0..MAXSMOOTHS).rev() {
            let s = self.s_list[i];
            if self.time < s.time + st {
                let st_local = (((s.time + st) - self.time) / st) as f32;
                let s_fraction = 1.0 - (((st_local * M_PI * 2.0).cos() + 1.0) / 2.0);
                out_axis = rotate_axis_rows(&in_axis, &s.rot_axis, s_fraction * s.rot_angle);
                in_axis = out_axis;
                performed = true;
            }
        }
        if performed {
            axis_to_angles(&out_axis)
        } else {
            *inp
        }
    }

    /// `CG_DamageFeedback`.
    pub fn damage_feedback(&mut self, ps: &PlayerState, yaw_byte: i32, pitch_byte: i32, damage: i32) {
        // show the attacking player's head and name in corner
        self.attacker_time = self.time;
        // the lower on health you are, the greater the view kick will be
        let health = ps.stat(Stat::Health);
        let scale = if health < 40 { 1.0 } else { 40.0 / health as f32 };
        let kick = (damage as f32 * scale).clamp(5.0, 10.0);
        // if yaw and pitch are both 255, make the damage always centered (falling, etc)
        if yaw_byte == 255 && pitch_byte == 255 {
            self.damage_x = 0.0;
            self.damage_y = 0.0;
            self.v_dmg_roll = 0.0;
            self.v_dmg_pitch = -kick;
        } else {
            // positional
            let pitch = pitch_byte as f32 / 255.0 * 360.0;
            let yaw = yaw_byte as f32 / 255.0 * 360.0;
            let (d, _, _) = angle_vectors(&[pitch, yaw, 0.0]);
            let dir = [-d[0], -d[1], -d[2]];
            let ax = self.last_axis;
            let mut front = dot(&dir, &ax[0]);
            let left = dot(&dir, &ax[1]);
            let up = dot(&dir, &ax[2]);
            let dist = length(&[front, left, 0.0]).max(0.1);
            self.v_dmg_roll = kick * left;
            self.v_dmg_pitch = -kick * front;
            if front <= 0.1 {
                front = 0.1;
            }
            self.damage_x = -left / front;
            self.damage_y = up / dist;
        }
        // clamp the position
        self.damage_x = self.damage_x.clamp(-1.0, 1.0);
        self.damage_y = self.damage_y.clamp(-1.0, 1.0);
        // don't let the screen flashes vary as much
        self.damage_value = kick.min(10.0);
        self.v_dmg_time = self.time + DAMAGE_TIME;
        self.damage_time = self.time;
    }

    /// `CG_PainEvent`: the pain sound for `ent` ("*painNN_1.wav" custom
    /// sound name), at most two a second.
    pub fn pain_event(&mut self, ent: i32, health: i32) -> Option<String> {
        let last = self.pain_time.get(&ent).copied().unwrap_or(-100000);
        // don't do more than two pain sounds a second
        if self.time - last < 500 {
            return None;
        }
        let snd = if health < 25 {
            "pain25_1.wav"
        } else if health < 50 {
            "pain50_1.wav"
        } else if health < 75 {
            "pain75_1.wav"
        } else {
            "pain100_1.wav"
        };
        // save pain time for programitic twitch animation
        self.pain_time.insert(ent, self.time);
        Some(snd.to_string())
    }

    /// `CG_TransitionPlayerState` (with `CG_CheckLocalSounds` and the
    /// `CG_Respawn` / `CG_ResetPainBlend` parts kept here): returns the
    /// custom pain sounds to start on `ps.clientNum`.
    pub fn transition_player_state(&mut self, ps: &PlayerState) -> Vec<String> {
        let mut sounds = Vec::new();
        let mut ops = self.ops.unwrap_or(*ps);
        // check for changing follow mode
        if ps.client_num != ops.client_num {
            // make sure we don't get any unwanted transition effects
            ops = *ps;
        }
        // damage events (player is getting wounded)
        if ps.damage_event != ops.damage_event && ps.damage_count != 0 {
            self.damage_feedback(ps, ps.damage_yaw, ps.damage_pitch, ps.damage_count);
        }
        if ps.pm_type != PmType::Intermission as i32 && ps.persistant[Pers::Specstate as usize] == 0 {
            // CG_CheckLocalSounds: don't play the sounds if the player just spawned
            if ps.persistant[Pers::Specstate as usize] == ops.persistant[Pers::Specstate as usize] {
                // health changes of more than -1 should make pain sounds
                if ps.stat(Stat::Health) < ops.stat(Stat::Health) - 1 && ps.stat(Stat::Health) > 0 {
                    if let Some(s) = self.pain_event(ps.client_num, ps.stat(Stat::Health)) {
                        sounds.push(s);
                    }
                }
            }
        }
        // smooth the ducking viewheight change
        if ps.viewheight != ops.viewheight {
            self.duck_change = (ps.viewheight - ops.viewheight) as f32;
            self.duck_time = self.time;
        }
        self.ops = Some(*ps);
        sounds
    }

    /// The view parts of `CG_EntityEvent` for an event of entity `e.entity`;
    /// `ps` is our (snapshot) player state. Returns a custom sound name to
    /// start on the entity instead of the default one, if any (EV_PAIN).
    pub fn entity_event(&mut self, e: &GameEvent, ps: &PlayerState, following: bool) {
        let me = ps.client_num;
        let steptime = if ps.persistant[Pers::Specstate as usize] != 0 { 200 } else { bg::class(ps.stat(Stat::Class)).steptime };
        let ours = e.entity == me;
        match e.event {
            Event::FallShort if ours => {
                // smooth landing z changes
                self.land_change = -8.0;
                self.land_time = self.time;
            }
            Event::FallMedium if ours => {
                self.land_change = -16.0;
                self.land_time = self.time;
            }
            Event::FallFar => {
                // don't play a pain sound right after this
                self.pain_time.insert(e.entity, self.time);
                if ours {
                    self.land_change = -24.0;
                    self.land_time = self.time;
                }
            }
            Event::Step4 | Event::Step8 | Event::Step12 | Event::Step16 | Event::Stepdn4 | Event::Stepdn8 | Event::Stepdn12 | Event::Stepdn16 => {
                // if we are interpolating, we don't need to smooth steps
                if !ours || following {
                    return;
                }
                let ev = e.event as i32;
                let step = if ev >= Event::Stepdn4 as i32 { -4 * (ev - Event::Stepdn4 as i32 + 1) } else { 4 * (ev - Event::Step4 as i32 + 1) };
                self.step_change = step_event(self.step_change, self.step_time, self.time, steptime, step);
                self.step_time = self.time;
            }
            Event::Jump => {
                if class_has(ps.stat(Stat::Class), ability::WALLJUMPER) && ours {
                    let ref_normal: Vec3 = [0.0, 0.0, 1.0];
                    let surf_normal = ps.grapple_point;
                    // if we are moving from one surface to another smooth the transition
                    if !compare(&surf_normal, &self.last_normal) && surf_normal[2] != 1.0 {
                        let mut rot_axis = cross(&ref_normal, &surf_normal);
                        normalize(&mut rot_axis);
                        self.add_smooth_op(rot_axis, 15.0, 1.0);
                    }
                    self.last_normal = surf_normal;
                }
            }
            Event::FireWeapon | Event::FireWeapon2 | Event::FireWeapon3 if ours => {
                // CG_FireWeapon: mark the entity as muzzle flashing
                self.muzzle_flash_time = self.time;
            }
            Event::AlienEvolve | Event::PlayerRespawn if ours => self.spawn_time = self.time,
            _ => {}
        }
    }

    /// `CG_StepOffset`.
    fn apply_step_offset(&self, ps: &PlayerState, vieworg: &mut Vec3) {
        let normal = bg::client_normal(ps);
        let steptime = bg::class(ps.stat(Stat::Class)).steptime;
        let s = step_offset(self.step_change, self.step_time, self.time, steptime);
        if s != 0.0 {
            *vieworg = ma(vieworg, -s, &normal);
        }
    }

    /// `CG_OffsetFirstPersonView`.
    #[allow(clippy::too_many_arguments)]
    fn offset_first_person_view(&mut self, w: &World, ps: &PlayerState, cmd: &UserCmd, following: bool, origin: &mut Vec3, angles: &mut Vec3) {
        let normal = bg::client_normal(ps);
        if ps.pm_type == PmType::Intermission as i32 {
            return;
        }
        // if dead, fix the angle and don't add any kick
        if ps.stat(Stat::Health) <= 0 {
            angles[2] = 40.0;
            angles[0] = -15.0;
            angles[1] = ps.stat(Stat::Viewlock) as f32;
            origin[2] += ps.viewheight as f32;
            return;
        }
        // add angles based on damage kick
        if self.damage_time != 0 {
            let mut ratio = (self.time - self.damage_time) as f32;
            if ratio < DAMAGE_DEFLECT_TIME {
                ratio /= DAMAGE_DEFLECT_TIME;
                angles[0] += ratio * self.v_dmg_pitch;
                angles[2] += ratio * self.v_dmg_roll;
            } else {
                ratio = 1.0 - (ratio - DAMAGE_DEFLECT_TIME) / DAMAGE_RETURN_TIME;
                if ratio > 0.0 {
                    angles[0] += ratio * self.v_dmg_pitch;
                    angles[2] += ratio * self.v_dmg_roll;
                }
            }
        }
        // add angles based on velocity: against cg.refdef.viewaxis, which
        // CG_CalcViewValues memsets and only fills after this function, so
        // both terms are 0 in the original
        let viewaxis = [[0.0f32; 3]; 3];
        let delta = dot(&ps.velocity, &viewaxis[0]);
        angles[0] += delta * self.cvars.runpitch;
        let delta = dot(&ps.velocity, &viewaxis[1]);
        angles[2] -= delta * self.cvars.runroll;
        // add angles based on bob; bob amount is class dependant
        let mut bob2 = if ps.persistant[Pers::Specstate as usize] != 0 { 0.0 } else { bg::class(ps.stat(Stat::Class)).bob };
        // give a charging player some feedback
        if ps.weapon == WeaponId::Level4 as i32 && ps.stat(Stat::Misc) > 0 {
            let fraction = (ps.stat(Stat::Misc) as f32 / k::LEVEL4_TRAMPLE_CHARGE_MAX as f32).min(1.0);
            bob2 *= 1.0 + fraction * LEVEL4_FEEDBACK;
        }
        let ducked = ps.pm_flags & k::PMF_DUCKED != 0;
        if bob2 != 0.0 {
            // make sure the bob is visible even at low speeds
            let speed = if self.xyspeed > 200.0 { self.xyspeed } else { 200.0 };
            let mut delta = self.bobfracsin * bob2 * speed;
            if ducked {
                delta *= 3.0; // crouching
            }
            angles[0] += delta;
            let mut delta = self.bobfracsin * bob2 * speed;
            if ducked {
                delta *= 3.0; // crouching accentuates roll
            }
            if self.bobcycle & 1 != 0 {
                delta = -delta;
            }
            angles[2] += delta;
        }
        // provide some feedback for pouncing
        if (ps.weapon == WeaponId::Level3 as i32 || ps.weapon == WeaponId::Level3upg as i32) && ps.stat(Stat::Misc) > 0 {
            let (mut forward, _, _) = angle_vectors(angles);
            normalize(&mut forward);
            let fraction1 = (ps.stat(Stat::Misc) as f32 / k::LEVEL3_POUNCE_TIME_UPG as f32).min(1.0);
            let fraction2 = -(fraction1 * M_PI / 2.0).sin();
            *origin = ma(origin, LEVEL3_FEEDBACK * fraction2, &forward);
        }
        // allow the player to struggle a little whilst grabbed
        if ps.pm_type == PmType::Grabbed as i32 {
            let (forward, right, up) = angle_vectors(angles);
            let f_fraction = ((self.time - self.forward_move_time) as f32 / STRUGGLE_TIME).min(1.0);
            let r_fraction = ((self.time - self.right_move_time) as f32 / STRUGGLE_TIME).min(1.0);
            let u_fraction = ((self.time - self.up_move_time) as f32 / STRUGGLE_TIME).min(1.0);
            match cmd.forwardmove.signum() {
                1 => *origin = ma(origin, STRUGGLE_DIST * f_fraction, &forward),
                -1 => *origin = ma(origin, -STRUGGLE_DIST * f_fraction, &forward),
                _ => self.forward_move_time = self.time,
            }
            match cmd.rightmove.signum() {
                1 => *origin = ma(origin, STRUGGLE_DIST * r_fraction, &right),
                -1 => *origin = ma(origin, -STRUGGLE_DIST * r_fraction, &right),
                _ => self.right_move_time = self.time,
            }
            match cmd.upmove.signum() {
                1 => *origin = ma(origin, STRUGGLE_DIST * u_fraction, &up),
                -1 => *origin = ma(origin, -STRUGGLE_DIST * u_fraction, &up),
                _ => self.up_move_time = self.time,
            }
        }
        let poisoned = self.poisoned_time(w, ps);
        if ps.e_flags & k::EF_POISONCLOUDED != 0 && self.time - poisoned < PCLOUD_DISORIENT_DURATION && !following {
            let scale = (1.0 - (self.time - poisoned) as f32 / trem_sim::kernel::client::poison_cloud_time(ps) as f32).max(0.0);
            let fraction = ((self.time - poisoned) as f32 / 500.0 * M_PI * PCLOUD_ROLL_FREQUENCY).sin() * scale;
            let pitch_fraction = ((self.time - poisoned) as f32 / 200.0 * M_PI * PCLOUD_ROLL_FREQUENCY).sin() * scale;
            angles[2] += fraction * PCLOUD_ROLL_AMPLITUDE;
            angles[1] += fraction * PCLOUD_ROLL_AMPLITUDE;
            angles[0] += pitch_fraction * PCLOUD_ROLL_AMPLITUDE / 2.0;
        }
        // this *feels* more realisitic for humans
        if ps.stat(Stat::Team) == Team::Humans as i32 && (ps.pm_type == PmType::Normal as i32 || ps.pm_type == PmType::Jetpack as i32) {
            angles[0] += self.bobfracsin * bob2 * 0.5;
            // heavy breathing effects //FIXME: sound
            if ps.stat(Stat::Stamina) < k::STAMINA_BREATHING_LEVEL {
                let delta_breath = (ps.stat(Stat::Stamina) - k::STAMINA_BREATHING_LEVEL) as f32 / -250.0;
                let mut delta_angle = ((self.time as f32 / 150.0) as f64).cos() as f32 * delta_breath;
                delta_angle += delta_angle.abs() * 0.5;
                angles[0] -= delta_angle;
            }
        }
        // add view height
        *origin = ma(origin, ps.viewheight as f32, &normal);
        // smooth out duck height changes
        let time_delta = self.time - self.duck_time;
        if time_delta < DUCK_TIME {
            origin[2] -= self.duck_change * (DUCK_TIME - time_delta) as f32 / DUCK_TIME as f32;
        }
        // add bob height
        let bob = (self.bobfracsin * self.xyspeed * bob2).min(6.0);
        *origin = ma(origin, bob, &normal);
        // add fall height
        origin[2] += land_offset(self.land_change, self.land_time, self.time);
        // add step offset
        self.apply_step_offset(ps, origin);
    }

    /// cg.poisonedTime: set by the "poisoncloud" server command, sent when
    /// the cloud hits (g_weapon.c poisonCloud), i.e. lastPoisonCloudedTime.
    fn poisoned_time(&self, w: &World, ps: &PlayerState) -> i32 {
        let i = ps.client_num;
        if i >= 0 && (i as usize) < MAX_CLIENTS {
            w.clients[i as usize].last_poison_clouded_time
        } else {
            -100000
        }
    }

    /// `CG_OffsetThirdPersonView`.
    fn offset_third_person_view(&mut self, w: &World, ps: &PlayerState, cmd: &UserCmd, following: bool, vieworg: &mut Vec3, angles: &mut Vec3) {
        // If cg_thirdpersonShoulderViewMode == 2, do shoulder view instead
        // If cg_thirdpersonShoulderViewMode == 1, do shoulder view when chasing
        //   a wallwalker because it's really erratic to watch
        let mode = self.cvars.third_person_shoulder_view_mode;
        if mode == 2 || (mode == 1 && ps.stat(Stat::State) & k::SS_WALLCLIMBING != 0 && ps.stat(Stat::Health) > 0) {
            self.offset_shoulder_view(w, ps, cmd, following, vieworg, angles);
            return;
        }
        let surf_normal = bg::client_normal(ps);
        // Set the view origin to the class's view height
        *vieworg = ma(vieworg, ps.viewheight as f32, &surf_normal);
        // Set the focus point where the camera will look (at the player's vieworg)
        let focus_point = *vieworg;
        // If player is dead, we want the player to be between us and the killer
        // so pretend that the player was looking at the killer, then place cam behind them.
        let alive = ps.stat(Stat::Health) > 0;
        if !alive {
            let killer = ps.stat(Stat::Viewlock);
            // already looking at ourself
            if killer != ps.client_num {
                if !self.was_dead_last_frame || self.cvars.static_death_cam == 0 {
                    self.killer_pos = lerp_origin(w, killer);
                    self.was_dead_last_frame = true;
                }
                *angles = vectoangles(&sub(&self.killer_pos, vieworg));
            }
        }
        // get and rangecheck cg_thirdPersonRange
        let range = self.cvars.third_person_range.clamp(30.0, 150.0);
        let mut rotation_angles: Vec3;
        // Unless in demo, PLAYING in third person, or in dead-third-person cam,
        // allow the player to control camera position offsets using the mouse position.
        if following && alive {
            // Prevent pitch from wrapping and clamp it within a [-75, 90] range.
            let delta_pitch = short2angle(cmd.angles[0] - self.old_cmd.angles[0]) as f32;
            if delta_pitch.abs() < 200.0 {
                self.tp_pitch += delta_pitch;
            }
            let mut mouse = [self.tp_pitch, -1.0 * short2angle(cmd.angles[1]) as f32, 0.0];
            for m in mouse.iter_mut() {
                *m = angle_normalize180(*m);
            }
            // Ignore the original pitch though; it's too jerky otherwise
            if self.cvars.third_person_pitch_follow == 0 {
                angles[0] = 0.0;
            }
            rotation_angles = [0.0; 3];
            for i in 0..3 {
                rotation_angles[i] = angle_normalize180(angles[i]) + mouse[i];
            }
            // Don't let pitch go too high/too low or the camera flips around and
            // that's really annoying. However, when we're not on the floor or
            // ceiling (wallwalk) pitch may not be pitch, so just let it go.
            if surf_normal[2] > 0.5 || surf_normal[2] < -0.5 {
                rotation_angles[0] = rotation_angles[0].clamp(-85.0, 85.0);
            }
            // Perform the rotations specified by rotationAngles.
            let axis = angles_to_axis(&rotation_angles);
            let rot = if ps.stat(Stat::State) & k::SS_WALLCLIMBING != 0 { bg::rotate_axis(&ps.grapple_point, &axis, false, ps.e_flags & k::EF_WALLCLIMBCEILING != 0) } else { None };
            // Convert the new axis back to angles.
            rotation_angles = axis_to_angles(&rot.unwrap_or(axis));
        } else if alive {
            // If we're playing the game in third person, the viewangles already
            // take care of our mouselook, so just use them.
            rotation_angles = *angles;
        } else {
            // dead
            rotation_angles = [20.0, angles[1], 0.0];
        }
        rotation_angles[1] -= self.cvars.third_person_angle;
        // Move the camera range distance back.
        let (forward, _, _) = angle_vectors(&rotation_angles);
        let mut view = ma(vieworg, -range, &forward);
        // Ensure that the current camera position isn't out of bounds and that there
        // is nothing between the camera and the player.
        if self.cvars.camera_mode == 0 {
            // Trace a ray from the origin to the viewpoint to make sure the view isn't
            // in a solid block.  Use an 8 by 8 block to prevent the view from near clipping anything
            let (mins, maxs) = ([-8.0f32; 3], [8.0f32; 3]);
            let tr = w.trace(vieworg, Some(&mins), Some(&maxs), &view, ps.client_num, k::MASK_SOLID);
            if tr.fraction != 1.0 {
                view = tr.endpos;
                view[2] += (1.0 - tr.fraction) * 32.0;
                // Try another trace to this position, because a tunnel may have the ceiling
                // close enogh that this is poking out.
                let tr = w.trace(vieworg, Some(&mins), Some(&maxs), &view, ps.client_num, k::MASK_SOLID);
                view = tr.endpos;
            }
        }
        // Set the camera position to what we calculated.
        *vieworg = view;
        // The above checks may have moved the camera such that the existing viewangles
        // may not still face the player. Recalculate them to do so.
        // but if we're dead, don't bother because we'd rather see what killed us
        if alive {
            *angles = vectoangles(&sub(&focus_point, vieworg));
        }
    }

    /// `CG_OffsetShoulderView`.
    fn offset_shoulder_view(&mut self, w: &World, ps: &PlayerState, cmd: &UserCmd, following: bool, vieworg: &mut Vec3, angles: &mut Vec3) {
        // Ignore following pitch; it's too jerky otherwise.
        if self.cvars.third_person_pitch_follow == 0 {
            angles[0] = 0.0;
        }
        let (forward, right, up) = angle_vectors(angles);
        let off = trem_sim::gen::classes::ClassId::from_num(ps.stat(Stat::Class).clamp(0, 255) as u8).map(|c| c.cold().shoulder_offsets).unwrap_or([0.0; 3]);
        *vieworg = ma(vieworg, off[0], &forward);
        *vieworg = ma(vieworg, off[1], &right);
        *vieworg = ma(vieworg, off[2], &up);
        // If someone is playing like this, the rest is already taken care of
        // so just get the firstperson effects and leave.
        if !following {
            self.offset_first_person_view(w, ps, cmd, following, vieworg, angles);
            return;
        }
        // Get mouse input for camera rotation.
        let delta = short2angle(cmd.angles[0] - self.old_cmd.angles[0]) as f32;
        if delta.abs() < 200.0 {
            self.shoulder_pitch += delta;
        }
        let mut rot = [0.0f32; 3];
        rot[0] = angle_normalize180(self.shoulder_pitch + angle_normalize180(angles[0])).clamp(-90.0, 90.0);
        // Yaw and Roll are much easier.
        rot[1] = short2angle(cmd.angles[1]) as f32 + angles[1];
        rot[2] = 0.0;
        // Perform the rotations.
        let axis = angles_to_axis(&rot);
        let r = if ps.stat(Stat::State) & k::SS_WALLCLIMBING != 0 { bg::rotate_axis(&ps.grapple_point, &axis, false, ps.e_flags & k::EF_WALLCLIMBCEILING != 0) } else { None };
        // Actually set the viewangles.
        *angles = axis_to_angles(&r.unwrap_or(axis));
        // Now run the first person stuff so we get various effects added.
        self.offset_first_person_view(w, ps, cmd, following, vieworg, angles);
    }

    /// `CG_CalcFov`: (fov_x, fov_y, inwater) for a viewport of `aspect`
    /// (width / height).
    fn calc_fov(&mut self, w: &World, ps: &PlayerState, cmd: &UserCmd, third_person: bool, following: bool, vieworg: &Vec3, aspect: f32) -> (f32, f32, bool, bool) {
        // switch follow modes if necessary: cycle between free -> follow -> third-person follow
        let send_follow = self.follow_toggle(ps, cmd);
        let mut fov_y;
        if ps.pm_type == PmType::Intermission as i32 || ps.persistant[Pers::Specstate as usize] != 0 || third_person {
            // if in intermission or third person, use a fixed value
            fov_y = BASE_FOV_Y;
        } else {
            // don't lock the fov globally - we need to be able to change it
            let (f, attrib_fov) = class_fov_y(ps.stat(Stat::Class), self.spawn_time, self.time);
            fov_y = f;
            // account for zooms
            let wd = bg::weapon(ps.weapon);
            let mut zoom_fov = wd.zoom_fov * 0.75;
            if zoom_fov < 1.0 {
                zoom_fov = 1.0;
            } else if zoom_fov > attrib_fov as f32 {
                zoom_fov = attrib_fov as f32;
            }
            // only do all the zoom stuff if the client CAN zoom
            // FIXME: zoom control is currently hard coded to BUTTON_ATTACK2
            if wd.can_zoom {
                let f = (self.time - self.zoom_time) as f32 / ZOOM_TIME as f32;
                if self.zoomed {
                    fov_y = if f > 1.0 { zoom_fov } else { fov_y + f * (zoom_fov - fov_y) };
                    // BUTTON_ATTACK2 isn't held so unzoom next time
                    if cmd.buttons & k::BUTTON_ATTACK2 == 0 {
                        self.zoomed = false;
                        self.zoom_time = self.time.min(self.time + self.time - self.zoom_time - ZOOM_TIME);
                    }
                } else {
                    if f <= 1.0 {
                        fov_y = zoom_fov + f * (fov_y - zoom_fov);
                    }
                    // BUTTON_ATTACK2 is held so zoom next time
                    if cmd.buttons & k::BUTTON_ATTACK2 != 0 {
                        self.zoomed = true;
                        self.zoom_time = self.time.min(self.time + self.time - self.zoom_time - ZOOM_TIME);
                    }
                }
            }
        }
        let mut fov_x = fov_x_for(fov_y, aspect);
        // warp if underwater
        let c = w.point_contents(vieworg, -1);
        let inwater = c & (contents::WATER | contents::SLIME | contents::LAVA) as i32 != 0;
        if inwater {
            let phase = self.time as f32 / 1000.0 * WAVE_FREQUENCY * M_PI * 2.0;
            let v = WAVE_AMPLITUDE * phase.sin();
            fov_x += v;
            fov_y -= v;
        }
        let poisoned = self.poisoned_time(w, ps);
        if ps.e_flags & k::EF_POISONCLOUDED != 0 && self.time - poisoned < PCLOUD_DISORIENT_DURATION && ps.stat(Stat::Health) > 0 && !following {
            let scale = 1.0 - (self.time - poisoned) as f32 / trem_sim::kernel::client::poison_cloud_time(ps) as f32;
            let phase = (self.time - poisoned) as f32 / 1000.0 * PCLOUD_ZOOM_FREQUENCY * M_PI * 2.0;
            let v = PCLOUD_ZOOM_AMPLITUDE * phase.sin() * scale;
            fov_x += v;
            fov_y += v;
        }
        self.zoom_sensitivity = if !self.zoomed { 1.0 } else { fov_y / 75.0 };
        (fov_x, fov_y, inwater, send_follow)
    }

    /// The follow-mode switch at the top of `CG_CalcFov`: a fresh
    /// BUTTON_USE_HOLDABLE press while following toggles cg.chaseFollow
    /// (leaving the chase cam also sends "follow", which stops following);
    /// a spectator not following sends "follow". True = send "follow".
    pub fn follow_toggle(&mut self, ps: &PlayerState, cmd: &UserCmd) -> bool {
        if cmd.buttons & k::BUTTON_USE_HOLDABLE == 0 || self.old_cmd.buttons & k::BUTTON_USE_HOLDABLE != 0 {
            return false;
        }
        if ps.pm_flags & k::PMF_FOLLOW != 0 {
            if !self.chase_follow {
                self.chase_follow = true;
                false
            } else {
                self.chase_follow = false;
                true
            }
        } else {
            ps.persistant[Pers::Specstate as usize] != 0
        }
    }

    /// `CG_CalcViewValues` for our snapshot player state `ps` (the followed
    /// player's when following). `third_person_cvar`: cg_thirdPerson.
    #[allow(clippy::too_many_arguments)]
    pub fn calc_view_values(&mut self, w: &World, ps: &PlayerState, cmd: &UserCmd, third_person_cvar: bool, aspect: f32, time: i32) -> Refdef {
        self.time = time;
        let following = ps.pm_flags & k::PMF_FOLLOW != 0;
        // CG_DrawActiveFrame: decide on third person view
        let third_person = third_person_cvar || ps.stat(Stat::Health) <= 0 || (self.chase_follow && following);
        let mut vieworg;
        let mut angles;
        // intermission view
        if ps.pm_type == PmType::Intermission as i32 || ps.pm_type == PmType::Freeze as i32 || ps.pm_type == PmType::Spectator as i32 {
            vieworg = ps.origin;
            angles = ps.viewangles;
        } else {
            (self.bobcycle, self.bobfracsin, self.xyspeed) = bob_values(ps.bob_cycle, &ps.velocity);
            vieworg = ps.origin;
            let class = ps.stat(Stat::Class);
            angles = if class_has(class, ability::WALLCLIMBER) {
                self.smooth_ww_transitions(ps, following, &ps.viewangles)
            } else if class_has(class, ability::WALLJUMPER) {
                self.smooth_wj_transitions(following, &ps.viewangles)
            } else {
                ps.viewangles
            };
            // clumsy logic, but it needs to be this way round because the CS propogation
            // delay screws things up otherwise
            if !class_has(class, ability::WALLJUMPER) && ps.stat(Stat::State) & k::SS_WALLCLIMBING == 0 {
                self.last_normal = [0.0, 0.0, 1.0];
            }
            // (no client-side prediction here: no error decay)
            if ps.stat(Stat::Health) > 0 {
                self.was_dead_last_frame = false;
            }
            if third_person {
                // back away from character
                self.offset_third_person_view(w, ps, cmd, following, &mut vieworg, &mut angles);
            } else {
                // offset for local bobbing and kicks
                self.offset_first_person_view(w, ps, cmd, following, &mut vieworg, &mut angles);
            }
        }
        if angles.iter().any(|a| !a.is_finite()) {
            angles = ps.viewangles;
        }
        // position eye reletive to origin
        let axis = angles_to_axis(&angles);
        let (fov_x, fov_y, inwater, send_follow) = self.calc_fov(w, ps, cmd, third_person, following, &vieworg, aspect);
        self.last_axis = axis;
        self.old_cmd = *cmd;
        let rd = Refdef { vieworg, angles, axis, fov_x, fov_y, third_person, inwater, send_follow };
        self.last_refdef = Some(rd);
        rd
    }

    /// "cg_viewdebug": the last refdef and smoothing state.
    pub fn debug_string(&self) -> String {
        match &self.last_refdef {
            Some(r) => format!(
                "vieworg {:.1} {:.1} {:.1} angles {:.1} {:.1} {:.1} fov {:.1}x{:.1} third {} zoomed {} lastNormal {:.2} {:.2} {:.2}",
                r.vieworg[0], r.vieworg[1], r.vieworg[2], r.angles[0], r.angles[1], r.angles[2], r.fov_x, r.fov_y, r.third_person, self.zoomed, self.last_normal[0], self.last_normal[1], self.last_normal[2]
            ) + " " + &self.draw_debug,
            None => "no view".into(),
        }
    }

    /// Whether `CG_AddViewWeapon` draws the gun at all this frame.
    pub fn draws_view_weapon(&self, ps: &PlayerState, rd: &Refdef) -> bool {
        ps.weapon != 0 && ps.persistant[Pers::Specstate as usize] == 0 && ps.pm_type != PmType::Intermission as i32 && !rd.third_person && self.cvars.draw_gun != 0
    }

    /// `CG_AddViewWeapon`: the hand origin and axis (`CG_CalculateWeaponPosition`,
    /// cg_gun_x/y/z, the drop at high fov, the lucifer cannon vibration).
    pub fn view_weapon_position(&mut self, ps: &PlayerState, rd: &Refdef) -> (Vec3, [Vec3; 3]) {
        // drop gun lower at higher fov
        let fov_offset = if rd.fov_y > 90.0 { -0.4 * (rd.fov_y - 90.0) } else { 0.0 };
        let bob = bg::class(ps.stat(Stat::Class)).bob;
        // noDrift from the weapon's animation.cfg (none in the 1.1 data)
        let (mut origin, angles) = calculate_weapon_position(&rd.vieworg, &rd.angles, self.bobcycle, self.bobfracsin, self.xyspeed, bob, self.weapon_no_drift, self.land_change, self.land_time, self.time);
        origin = ma(&origin, self.cvars.gun_x, &rd.axis[0]);
        origin = ma(&origin, self.cvars.gun_y, &rd.axis[1]);
        origin = ma(&origin, self.cvars.gun_z + fov_offset, &rd.axis[2]);
        // Lucifer Cannon vibration effect
        if ps.weapon == WeaponId::Lcannon as i32 && ps.stat(Stat::Misc) > 0 {
            let fraction = ps.stat(Stat::Misc) as f32 / k::LCANNON_CHARGE_TIME_MAX as f32;
            let r1 = self.random();
            origin = ma(&origin, r1 * fraction, &rd.axis[0]);
            let r2 = self.random();
            origin = ma(&origin, r2 * fraction, &rd.axis[1]);
        }
        (origin, angles_to_axis(&angles))
    }

    /// `CG_MachinegunSpinAngle` for the view weapon.
    pub fn machinegun_spin_angle(&mut self, firing: bool) -> f32 {
        let mut delta = self.time - self.barrel_time;
        let angle = if self.barrel_spinning {
            self.barrel_angle + delta as f32 * SPIN_SPEED
        } else {
            if delta > COAST_TIME {
                delta = COAST_TIME;
            }
            let speed = 0.5 * (SPIN_SPEED + (COAST_TIME - delta) as f32 / COAST_TIME as f32);
            self.barrel_angle + delta as f32 * speed
        };
        if self.barrel_spinning == !firing {
            self.barrel_time = self.time;
            self.barrel_angle = trem_sim::kernel::qmath::angle_normalize360(angle);
            self.barrel_spinning = firing;
        }
        angle
    }

    /// The flash roll of `CG_AddPlayerWeapon` for the view weapon, when the
    /// flash is drawn this frame (impulse flash within MUZZLE_FLASH_TIME of
    /// the last shot, or held while firing a continuousFlash mode).
    pub fn view_flash_roll(&mut self, firing: bool, continuous: bool) -> Option<f32> {
        if !(continuous && firing) && self.time - self.muzzle_flash_time > MUZZLE_FLASH_TIME {
            return None;
        }
        Some(self.crandom() * 10.0)
    }
}

/// `CG_CalcVrect`: the rendered window (x, y, width, height) in pixels for
/// a `vid_w` x `vid_h` screen and cg_viewsize `size` (the intermission is
/// always full screen: pass 100).
pub fn calc_vrect(vid_w: i32, vid_h: i32, size: i32) -> (i32, i32, i32, i32) {
    let width = (vid_w * size / 100) & !1;
    let height = (vid_h * size / 100) & !1;
    ((vid_w - width) / 2, (vid_h - height) / 2, width, height)
}

/// The refdef projection (`fov_x` across the window width, `fov_y` across
/// its height, as `R_SetupProjection` builds it) from a camera that only
/// takes a vertical fov at the window's aspect: render with the returned
/// vertical fov (wide/tall enough for both) and show the centred part
/// (u, v fractions of the rendered width and height) stretched over the
/// window. Without a warp both fractions are 1.
pub fn projection_crop(fov_x: f32, fov_y: f32, width: f32, height: f32) -> (f32, f32, f32) {
    let tx = (fov_x.to_radians() * 0.5).tan();
    let ty = (fov_y.to_radians() * 0.5).tan();
    let aspect = if height > 0.0 { width / height } else { 1.0 };
    let cam_ty = ty.max(tx / aspect);
    let cam_tx = cam_ty * aspect;
    ((2.0 * cam_ty.atan()).to_degrees(), (tx / cam_tx).min(1.0), (ty / cam_ty).min(1.0))
}

/// `cg_entities[ n ].lerpOrigin`.
fn lerp_origin(w: &World, n: i32) -> Vec3 {
    if n < 0 {
        return [0.0; 3];
    }
    let n = n as usize;
    if n < MAX_CLIENTS && w.ents.get(n).is_some_and(|e| e.client) {
        return w.clients[n].ps.origin;
    }
    w.ents.get(n).map(|e| e.origin).unwrap_or([0.0; 3])
}

/// The weapon mode `ps.generic1` and whether it is firing (`CG_AddPlayerWeapon`).
pub fn view_weapon_firing(ps: &PlayerState) -> (usize, bool) {
    let mut mode = ps.generic1;
    if mode <= 0 || mode >= 4 {
        mode = 1;
    }
    let firing = (ps.e_flags & k::EF_FIRING != 0 && mode == 1) || (ps.e_flags & k::EF_FIRING2 != 0 && mode == 2) || (ps.e_flags & k::EF_FIRING3 != 0 && mode == 3);
    (mode as usize - 1, firing)
}

/// `CG_PositionAndOrientateBuildable` for `CG_GhostBuildable`: the axis
/// from the view yaw and the surface normal, and the origin dropped onto
/// the surface (CG_CapTrace has no capsule trace here: both traces are box
/// traces, so the box fraction is used).
pub fn position_and_orientate_buildable(w: &World, angles: &Vec3, in_origin: &Vec3, normal: &Vec3, skip: i32, mins: &Vec3, maxs: &Vec3) -> ([Vec3; 3], Vec3) {
    const TRACE_DEPTH: f32 = 64.0;
    let (forward, _, _) = angle_vectors(angles);
    let mut axis = [[0.0f32; 3]; 3];
    axis[2] = *normal;
    axis[0] = project_point_on_plane(&forward, &axis[2]);
    if normalize(&mut axis[0]) == 0.0 {
        let (_, _, up) = angle_vectors(angles);
        axis[0] = project_point_on_plane(&up, &axis[2]);
        normalize(&mut axis[0]);
    }
    let c = cross(&axis[0], &axis[2]);
    axis[1] = [-c[0], -c[1], -c[2]];
    let end = ma(in_origin, -TRACE_DEPTH, normal);
    let start = ma(in_origin, 1.0, normal);
    let mask = (contents::SOLID | contents::PLAYERCLIP) as i32;
    let tr = w.trace(&start, Some(mins), Some(maxs), &end, skip, mask);
    (axis, ma(in_origin, tr.fraction * -TRACE_DEPTH, normal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn bob_values_match_c() {
        // bobCycle 64: half way through the first leg
        let (c, s, v) = bob_values(64, &[300.0, 400.0, 10.0]);
        assert_eq!(c, 0);
        assert!(close(s, ((64.0f64 / 127.0 * std::f64::consts::PI).sin()) as f32));
        assert!(close(v, 300.0)); // clamped from 500
        let (c, s, _) = bob_values(128 + 127, &[0.0; 3]);
        assert_eq!(c, 1);
        assert!(s.abs() < 1e-5);
    }

    #[test]
    fn step_smoothing() {
        // a 16 unit step up with a 100 ms steptime decays linearly
        let ch = step_event(0.0, -1000, 0, 100, 16);
        assert!(close(ch, 16.0));
        assert!(close(step_offset(ch, 0, 0, 100), 16.0));
        assert!(close(step_offset(ch, 0, 50, 100), 8.0));
        assert!(close(step_offset(ch, 0, 100, 100), 0.0));
        // a second step half way through keeps the remainder, clamped to 32
        let ch2 = step_event(ch, 0, 50, 100, 16);
        assert!(close(ch2, 24.0));
        let ch3 = step_event(ch2, 50, 50, 100, 16);
        assert!(close(ch3, 32.0));
        // step down
        assert!(close(step_event(0.0, -1000, 0, 100, -8), -8.0));
    }

    #[test]
    fn land_offsets() {
        assert!(close(land_offset(-16.0, 0, 0), 0.0));
        assert!(close(land_offset(-16.0, 0, 75), -8.0));
        assert!(close(land_offset(-16.0, 0, 150), -16.0));
        assert!(close(land_offset(-16.0, 0, 300), -8.0));
        assert!(close(land_offset(-16.0, 0, 450), 0.0));
    }

    #[test]
    fn class_fovs() {
        use trem_sim::gen::classes::ClassId;
        // human 90 -> 67 (int of 67.5), dretch 140 -> 105, basilisk 120 -> 90
        assert_eq!(class_fov_y(ClassId::HumanBase as i32, -10000, 0).0, 67.0);
        assert_eq!(class_fov_y(ClassId::Level0 as i32, -10000, 0).0, 105.0);
        assert_eq!(class_fov_y(ClassId::Level1 as i32, -10000, 0).0, 90.0);
        // FOVWARPS: starts at 127.5 right after the spawn
        assert!(close(class_fov_y(ClassId::Level0 as i32, 1000, 1000).0, MAX_FOV_WARP_Y));
        assert!(close(class_fov_y(ClassId::Level0 as i32, 1000, 1200).0, 127.5 - 22.5 * 0.5));
        // humans don't warp
        assert_eq!(class_fov_y(ClassId::HumanBase as i32, 1000, 1000).0, 67.0);
        // 4:3 BASE_FOV_Y gives a 90 degree horizontal fov
        assert!((fov_x_for(BASE_FOV_Y, 4.0 / 3.0) - 90.0).abs() < 0.01);
    }

    #[test]
    fn zoom_lerps_to_weapon_fov() {
        let mut v = ViewState::default();
        let w = World::new(trem_sim::gen::maps::MapId::Arena);
        let mut ps = PlayerState::default();
        ps.stats[Stat::Class as usize] = trem_sim::gen::classes::ClassId::HumanBase as i32;
        ps.stats[Stat::Health as usize] = 100;
        ps.weapon = WeaponId::Mdriver as i32;
        let zoom = (bg::weapon(ps.weapon).zoom_fov * 0.75).min(67.0);
        assert!(bg::weapon(ps.weapon).can_zoom);
        let mut cmd = UserCmd::default();
        cmd.buttons = k::BUTTON_ATTACK2;
        v.time = 10000;
        let (_, f0, _, _) = v.calc_fov(&w, &ps, &cmd, false, false, &[0.0; 3], 4.0 / 3.0);
        assert!(close(f0, 67.0));
        v.time += ZOOM_TIME / 2;
        let (_, f1, _, _) = v.calc_fov(&w, &ps, &cmd, false, false, &[0.0; 3], 4.0 / 3.0);
        assert!(close(f1, 67.0 + 0.5 * (zoom - 67.0)));
        v.time += ZOOM_TIME;
        let (_, f2, _, _) = v.calc_fov(&w, &ps, &cmd, false, false, &[0.0; 3], 4.0 / 3.0);
        assert!(close(f2, zoom));
        assert!(close(v.zoom_sensitivity, zoom / 75.0));
        // third person: fixed BASE_FOV_Y
        let (_, f3, _, _) = v.calc_fov(&w, &ps, &cmd, true, false, &[0.0; 3], 4.0 / 3.0);
        assert!(close(f3, BASE_FOV_Y));
    }

    #[test]
    fn wallwalk_transition_rotates_smoothly() {
        let mut v = ViewState::default();
        let mut ps = PlayerState::default();
        ps.stats[Stat::Class as usize] = trem_sim::gen::classes::ClassId::Level1 as i32;
        ps.viewangles = [0.0, 0.0, 0.0];
        v.time = 5000;
        // on the floor
        let a = v.smooth_ww_transitions(&ps, false, &ps.viewangles);
        assert_eq!(a, [0.0; 3]);
        // climb onto a wall facing +x: the sim rotates the view 90 degrees
        // (looking up the wall); the camera starts from the old view
        ps.stats[Stat::State as usize] |= k::SS_WALLCLIMBING;
        ps.grapple_point = [-1.0, 0.0, 0.0];
        let new = [-90.0, 0.0, 0.0];
        v.time += 1;
        let a0 = v.smooth_ww_transitions(&ps, false, &new);
        let f0 = angle_vectors(&a0).0;
        // almost still looking along +x
        assert!(f0[0] > 0.99, "{f0:?}");
        // half way (cos easing): 45 degrees
        v.time += 150;
        let a1 = v.smooth_ww_transitions(&ps, false, &new);
        let f1 = angle_vectors(&a1).0;
        assert!((f1[2] - (45.0f32).to_radians().sin()).abs() < 0.03, "{f1:?}");
        // done: the sim's view
        v.time += 200;
        let a2 = v.smooth_ww_transitions(&ps, false, &new);
        assert_eq!(a2, new);
    }

    #[test]
    fn walljump_smoothing_is_a_noop_like_the_c() {
        let mut v = ViewState::default();
        v.time = 5000;
        v.add_smooth_op([0.0, 1.0, 0.0], 15.0, 1.0);
        v.time += 100;
        let a = v.smooth_wj_transitions(false, &[10.0, 20.0, 0.0]);
        let f = angle_vectors(&a).0;
        let g = angle_vectors(&[10.0, 20.0, 0.0]).0;
        assert!(dot(&f, &g) > 0.9999);
    }

    #[test]
    fn damage_feedback_kick() {
        let mut v = ViewState::default();
        let mut ps = PlayerState::default();
        ps.stats[Stat::Health as usize] = 80;
        v.time = 1000;
        // world damage: centred, kick = clamp(20 * 40/80) = 10
        v.damage_feedback(&ps, 255, 255, 20);
        assert!(close(v.v_dmg_pitch, -10.0));
        assert!(close(v.v_dmg_roll, 0.0));
        // damage_from straight ahead of a view along +x (dir = -damage_from):
        // the kick pitches the view up (front = -1 -> v_dmg_pitch = +kick)
        v.last_axis = angles_to_axis(&[0.0, 0.0, 0.0]);
        v.damage_feedback(&ps, 0, 0, 4);
        assert!(close(v.v_dmg_pitch, 5.0));
        assert!(v.v_dmg_roll.abs() < 1e-4);
        // from the left: roll
        v.damage_feedback(&ps, 64, 0, 4);
        assert!(v.v_dmg_roll < -4.9);
    }

    #[test]
    fn weapon_position_bob_and_drift() {
        let (o, a) = calculate_weapon_position(&[0.0; 3], &[0.0; 3], 1, 1.0, 100.0, 0.002, false, -16.0, 0, 75);
        // landing drop: -16 * 0.25 * 75/150
        assert!(close(o[2], -2.0));
        let drift = 140.0 * (0.075f64).sin() as f32 * 0.01;
        assert!(close(a[2], -100.0 * 0.005 + drift));
        assert!(close(a[1], -100.0 * 0.01 + drift));
        assert!(close(a[0], 100.0 * 0.005 + drift));
    }

    #[test]
    fn vrect_like_cg_calcvrect() {
        assert_eq!(calc_vrect(1280, 720, 100), (0, 0, 1280, 720));
        // 70%: 896 x 504 centred
        assert_eq!(calc_vrect(1280, 720, 70), (192, 108, 896, 504));
        // odd sizes are rounded down to even
        assert_eq!(calc_vrect(641, 481, 50), (160, 120, 320, 240));
    }

    #[test]
    fn projection_crop_applies_both_fovs() {
        // no warp: fov_x from fov_y at the window aspect -> nothing cropped
        let fx = fov_x_for(BASE_FOV_Y, 16.0 / 9.0);
        let (cy, u, v) = projection_crop(fx, BASE_FOV_Y, 1600.0, 900.0);
        assert!(close(cy, BASE_FOV_Y) && (u - 1.0).abs() < 1e-4 && (v - 1.0).abs() < 1e-4);
        // underwater: fov_x + 1, fov_y - 1
        let (cy, u, v) = projection_crop(fx + 1.0, BASE_FOV_Y - 1.0, 1600.0, 900.0);
        let tx = ((fx + 1.0).to_radians() / 2.0).tan();
        let ty = ((BASE_FOV_Y - 1.0).to_radians() / 2.0).tan();
        let cty = (cy.to_radians() / 2.0).tan();
        // the shown part spans exactly tan(fov_x/2) and tan(fov_y/2)
        assert!((u * cty * 16.0 / 9.0 - tx).abs() < 1e-5, "u {u}");
        assert!((v * cty - ty).abs() < 1e-5, "v {v}");
        assert!(u >= 0.99 && v < 1.0);
    }

    #[test]
    fn follow_toggle_cycles_like_cg_calcfov() {
        let mut v = ViewState::default();
        let mut ps = PlayerState::default();
        ps.persistant[Pers::Specstate as usize] = 1;
        let press = UserCmd { buttons: k::BUTTON_USE_HOLDABLE, ..Default::default() };
        // free spectator: a press sends "follow"
        assert!(v.follow_toggle(&ps, &press));
        // following: first press -> chase cam, nothing sent
        ps.pm_flags |= k::PMF_FOLLOW;
        v.old_cmd = UserCmd::default();
        assert!(!v.follow_toggle(&ps, &press) && v.chase_follow);
        // held: no edge
        v.old_cmd = press;
        assert!(!v.follow_toggle(&ps, &press) && v.chase_follow);
        // second press: back to free ("follow" toggles following off)
        v.old_cmd = UserCmd::default();
        assert!(v.follow_toggle(&ps, &press) && !v.chase_follow);
    }

    #[test]
    fn pain_event_rate_limit() {
        let mut v = ViewState::default();
        v.time = 1000;
        assert_eq!(v.pain_event(3, 20).as_deref(), Some("pain25_1.wav"));
        v.time = 1400;
        assert_eq!(v.pain_event(3, 60), None);
        v.time = 1600;
        assert_eq!(v.pain_event(3, 60).as_deref(), Some("pain75_1.wav"));
    }
}
