//! client `cl_input.c`: the `kbutton_t` button commands (`IN_KeyDown`,
//! `IN_KeyUp`, `CL_KeyState`, `IN_MLookDown`, `IN_CenterView`) and the
//! usercmd built from them every frame (`CL_CreateNewCommands`,
//! `CL_CreateCmd`, `CL_AdjustAngles`, `CL_KeyMove`, `CL_MouseEvent`,
//! `CL_MouseMove`, `CL_JoystickEvent`, `CL_JoystickMove`, `CL_CmdButtons`,
//! `CL_FinishMove`). Pure Rust; the cvars are read by the caller from the
//! client cvar table (`CL_Init` defaults in `register_cvars`).

use crate::cmd::{Cvars, CVAR_ARCHIVE};
use trem_sim::kernel::qmath::{angle2short_f, short2angle};
use trem_sim::kernel::types::UserCmd;

/// q_shared.h BUTTON_TALK (displays talk balloon and disables actions)
pub const BUTTON_TALK: i32 = 2;
/// q_shared.h BUTTON_WALKING
pub const BUTTON_WALKING: i32 = 16;
/// q_shared.h BUTTON_ANY (any key whatsoever)
pub const BUTTON_ANY: i32 = 2048;

/// `kbutton_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KButton {
    /// key nums holding it down
    pub down: [i32; 2],
    /// msec timestamp
    pub downtime: u32,
    /// msec down this frame if both a down and up happened
    pub msec: u32,
    /// current state
    pub active: bool,
    /// set when down, not cleared when up
    pub was_pressed: bool,
}

/// `IN_KeyDown`: argv(1) is the key number ("" = typed at the console),
/// argv(2) the event time.
pub fn in_key_down(b: &mut KButton, arg1: &str, arg2: &str) {
    let k = if !arg1.is_empty() { trem_sim::kernel::spawn::c_atoi(arg1) } else { -1 };
    if k == b.down[0] || k == b.down[1] {
        return; // repeating key
    }
    if b.down[0] == 0 {
        b.down[0] = k;
    } else if b.down[1] == 0 {
        b.down[1] = k;
    } else {
        // "Three keys down for a button!"
        return;
    }
    if b.active {
        return; // still down
    }
    // save timestamp for partial frame summing
    b.downtime = trem_sim::kernel::spawn::c_atoi(arg2) as u32;
    b.active = true;
    b.was_pressed = true;
}

/// `IN_KeyUp`: release by the key that pressed it; typed at the console
/// (no key number) it clears the button.
pub fn in_key_up(b: &mut KButton, arg1: &str, arg2: &str, frame_msec: u32) {
    if arg1.is_empty() {
        // typed manually at the console, assume for unsticking, so clear all
        b.down = [0, 0];
        b.active = false;
        return;
    }
    let k = trem_sim::kernel::spawn::c_atoi(arg1);
    if b.down[0] == k {
        b.down[0] = 0;
    } else if b.down[1] == k {
        b.down[1] = 0;
    } else {
        return; // key up without coresponding down (menu pass through)
    }
    if b.down[0] != 0 || b.down[1] != 0 {
        return; // some other key is still holding it down
    }
    b.active = false;
    // save timestamp for partial frame summing
    let uptime = trem_sim::kernel::spawn::c_atoi(arg2) as u32;
    if uptime != 0 {
        b.msec = b.msec.wrapping_add(uptime.wrapping_sub(b.downtime));
    } else {
        b.msec += frame_msec / 2;
    }
    b.active = false;
}

/// `CL_KeyState`: the fraction of the frame the key was down.
pub fn cl_key_state(key: &mut KButton, com_frame_time: u32, frame_msec: u32) -> f32 {
    let mut msec = key.msec as i64;
    key.msec = 0;
    if key.active {
        // still down
        if key.downtime == 0 {
            msec = com_frame_time as i64;
        } else {
            msec += com_frame_time as i64 - key.downtime as i64;
        }
        key.downtime = com_frame_time;
    }
    let val = msec as f32 / frame_msec.max(1) as f32;
    val.clamp(0.0, 1.0)
}

/// `ClampChar`.
fn clamp_char(i: i32) -> i8 {
    i.clamp(-128, 127) as i8
}

/// The cl_input.c cvars (`CL_Init`: cl_main.c defaults).
#[derive(Clone, Copy, Debug)]
pub struct InputCvars {
    pub sensitivity: f32,
    pub m_pitch: f32,
    pub m_yaw: f32,
    pub m_forward: f32,
    pub m_side: f32,
    pub m_filter: i32,
    pub cl_run: i32,
    pub cl_freelook: i32,
    pub cl_yawspeed: f32,
    pub cl_pitchspeed: f32,
    pub cl_anglespeedkey: f32,
    pub cl_mouse_accel: f32,
    pub cl_mouse_accel_style: i32,
    pub cl_mouse_accel_offset: f32,
    /// j_pitch, j_yaw, j_forward, j_side, j_up
    pub j_pitch: f32,
    pub j_yaw: f32,
    pub j_forward: f32,
    pub j_side: f32,
    pub j_up: f32,
    /// j_pitch_axis, j_yaw_axis, j_forward_axis, j_side_axis, j_up_axis
    /// (`Cvar_CheckRange( 0, MAX_JOYSTICK_AXIS-1, qtrue )`)
    pub j_axes: [usize; 5],
}

impl Default for InputCvars {
    fn default() -> Self {
        InputCvars {
            sensitivity: 5.0,
            m_pitch: 0.022,
            m_yaw: 0.022,
            m_forward: 0.25,
            m_side: 0.25,
            m_filter: 0,
            cl_run: 1,
            cl_freelook: 1,
            cl_yawspeed: 140.0,
            cl_pitchspeed: 140.0,
            cl_anglespeedkey: 1.5,
            cl_mouse_accel: 0.0,
            cl_mouse_accel_style: 0,
            cl_mouse_accel_offset: 5.0,
            j_pitch: 0.022,
            j_yaw: -0.022,
            j_forward: -0.25,
            j_side: 0.25,
            j_up: 1.0,
            j_axes: [3, 4, 1, 0, 2],
        }
    }
}

/// cl_main.c `CL_Init`'s `Cvar_Get`s for the input cvars (legacy defaults).
pub fn register_cvars(c: &mut Cvars) {
    for (n, v, f) in [
        ("cl_yawspeed", "140", CVAR_ARCHIVE),
        ("cl_pitchspeed", "140", CVAR_ARCHIVE),
        ("cl_anglespeedkey", "1.5", 0),
        ("cl_run", "1", CVAR_ARCHIVE),
        ("sensitivity", "5", CVAR_ARCHIVE),
        ("cl_mouseAccel", "0", CVAR_ARCHIVE),
        ("cl_freelook", "1", CVAR_ARCHIVE),
        ("cl_mouseAccelStyle", "0", CVAR_ARCHIVE),
        ("cl_mouseAccelOffset", "5", CVAR_ARCHIVE),
        ("m_pitch", "0.022", CVAR_ARCHIVE),
        ("m_yaw", "0.022", CVAR_ARCHIVE),
        ("m_forward", "0.25", CVAR_ARCHIVE),
        ("m_side", "0.25", CVAR_ARCHIVE),
        ("m_filter", "0", CVAR_ARCHIVE),
        ("j_pitch", "0.022", CVAR_ARCHIVE),
        ("j_yaw", "-0.022", CVAR_ARCHIVE),
        ("j_forward", "-0.25", CVAR_ARCHIVE),
        ("j_side", "0.25", CVAR_ARCHIVE),
        ("j_up", "1", CVAR_ARCHIVE),
        ("j_pitch_axis", "3", CVAR_ARCHIVE),
        ("j_yaw_axis", "4", CVAR_ARCHIVE),
        ("j_forward_axis", "1", CVAR_ARCHIVE),
        ("j_side_axis", "0", CVAR_ARCHIVE),
        ("j_up_axis", "2", CVAR_ARCHIVE),
    ] {
        c.get(n, v, f);
    }
}

impl InputCvars {
    pub fn from_cvars(c: &Cvars) -> Self {
        InputCvars {
            sensitivity: c.value("sensitivity"),
            m_pitch: c.value("m_pitch"),
            m_yaw: c.value("m_yaw"),
            m_forward: c.value("m_forward"),
            m_side: c.value("m_side"),
            m_filter: c.integer("m_filter"),
            cl_run: c.integer("cl_run"),
            cl_freelook: c.integer("cl_freelook"),
            cl_yawspeed: c.value("cl_yawspeed"),
            cl_pitchspeed: c.value("cl_pitchspeed"),
            cl_anglespeedkey: c.value("cl_anglespeedkey"),
            cl_mouse_accel: c.value("cl_mouseAccel"),
            cl_mouse_accel_style: c.integer("cl_mouseAccelStyle"),
            cl_mouse_accel_offset: c.value("cl_mouseAccelOffset"),
            j_pitch: c.value("j_pitch"),
            j_yaw: c.value("j_yaw"),
            j_forward: c.value("j_forward"),
            j_side: c.value("j_side"),
            j_up: c.value("j_up"),
            j_axes: ["j_pitch_axis", "j_yaw_axis", "j_forward_axis", "j_side_axis", "j_up_axis"].map(|n| c.integer(n).clamp(0, crate::joystick::MAX_JOYSTICK_AXIS as i32 - 1) as usize),
        }
    }
}

/// The per-frame inputs of `CL_CreateCmd` that live outside cl_input.c.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameIn {
    /// `Key_GetCatcher()`
    pub catcher: i32,
    /// `anykeydown`
    pub anykeydown: bool,
    /// cl.cgameUserCmdValue (cg.weaponSelect)
    pub weapon: u8,
    /// cl.cgameSensitivity (cg.zoomSensitivity)
    pub cgame_sensitivity: f32,
    /// cl.serverTime
    pub server_time: i32,
}

/// The cl_input.c state: kbuttons, mouse accumulators, cl.viewangles.
#[derive(Clone, Debug, Default)]
pub struct ClInput {
    pub left: KButton,
    pub right: KButton,
    pub forward: KButton,
    pub back: KButton,
    pub lookup: KButton,
    pub lookdown: KButton,
    pub moveleft: KButton,
    pub moveright: KButton,
    pub strafe: KButton,
    pub speed: KButton,
    pub up: KButton,
    pub down: KButton,
    pub buttons: [KButton; 16],
    /// in_mlooking
    pub mlooking: bool,
    /// cl.viewangles
    pub viewangles: [f32; 3],
    mouse_dx: [f32; 2],
    mouse_dy: [f32; 2],
    mouse_index: usize,
    /// frame_msec / old_com_frameTime
    pub frame_msec: u32,
    old_com_frame_time: u32,
    /// com_frameTime of this frame
    pub com_frame_time: u32,
    /// the button bits of the last cmd that came from wasPressed only
    pressed_only: i32,
    /// cl.joystickAxis (set by joystick events, kept until changed)
    pub joystick_axis: [i32; crate::joystick::MAX_JOYSTICK_AXIS],
}

impl ClInput {
    fn kbutton(&mut self, name: &str) -> Option<&mut KButton> {
        Some(match name {
            "moveup" => &mut self.up,
            "movedown" => &mut self.down,
            "left" => &mut self.left,
            "right" => &mut self.right,
            "forward" => &mut self.forward,
            "back" => &mut self.back,
            "lookup" => &mut self.lookup,
            "lookdown" => &mut self.lookdown,
            "strafe" => &mut self.strafe,
            "moveleft" => &mut self.moveleft,
            "moveright" => &mut self.moveright,
            "speed" => &mut self.speed,
            "attack" => &mut self.buttons[0],
            n => {
                // +button0 .. +button14 (CL_InitInput registers 15 of them)
                let i: usize = n.strip_prefix("button")?.parse().ok()?;
                if i >= 15 || n != format!("button{i}") {
                    return None;
                }
                &mut self.buttons[i]
            }
        })
    }

    /// The `CL_InitInput` commands: `+name`/`-name` kbuttons, `+mlook` /
    /// `-mlook`, `centerview`. False when argv[0] is not one of them.
    /// `delta_pitch` is cl.snap.ps.delta_angles[PITCH] (`IN_CenterView`).
    pub fn command(&mut self, argv: &[String], cl_freelook: i32, delta_pitch: i32) -> bool {
        let Some(c) = argv.first() else { return false };
        let c = c.to_ascii_lowercase();
        let a1 = argv.get(1).map(|s| s.as_str()).unwrap_or("");
        let a2 = argv.get(2).map(|s| s.as_str()).unwrap_or("");
        match c.as_str() {
            "centerview" => self.center_view(delta_pitch),
            // IN_MLookDown / IN_MLookUp
            "+mlook" => self.mlooking = true,
            "-mlook" => {
                self.mlooking = false;
                if cl_freelook == 0 {
                    self.center_view(delta_pitch);
                }
            }
            _ => {
                let (down, name) = match c.split_at(1) {
                    ("+", n) => (true, n),
                    ("-", n) => (false, n),
                    _ => return false,
                };
                let frame_msec = self.frame_msec;
                let Some(b) = self.kbutton(name) else { return false };
                if down {
                    in_key_down(b, a1, a2);
                } else {
                    in_key_up(b, a1, a2, frame_msec);
                }
            }
        }
        true
    }

    /// `IN_CenterView`.
    pub fn center_view(&mut self, delta_pitch: i32) {
        self.viewangles[0] = -(short2angle(delta_pitch) as f32);
    }

    /// `CL_MouseEvent` with no key catcher: accumulate for `CL_MouseMove`.
    pub fn mouse_event(&mut self, dx: f32, dy: f32) {
        self.mouse_dx[self.mouse_index] += dx;
        self.mouse_dy[self.mouse_index] += dy;
    }

    /// `CL_CreateNewCommands`'s frame_msec (1..200 ms).
    pub fn new_frame(&mut self, com_frame_time: u32) {
        let fm = com_frame_time.wrapping_sub(self.old_com_frame_time);
        // if running over 1000fps, act as if each frame is 1ms; if running
        // less than 5fps, truncate the extra time to prevent unexpected moves
        self.frame_msec = fm.clamp(1, 200);
        self.old_com_frame_time = com_frame_time;
        self.com_frame_time = com_frame_time;
    }

    fn key_state(&mut self, which: fn(&mut ClInput) -> &mut KButton) -> f32 {
        let (t, f) = (self.com_frame_time, self.frame_msec);
        cl_key_state(which(self), t, f)
    }

    /// `CL_AdjustAngles`: keyboard turning (+left/+right/+lookup/+lookdown).
    pub fn adjust_angles(&mut self, cv: &InputCvars, frametime: u32) {
        let speed = if self.speed.active { 0.001 * frametime as f32 * cv.cl_anglespeedkey } else { 0.001 * frametime as f32 };
        if !self.strafe.active {
            self.viewangles[1] -= speed * cv.cl_yawspeed * self.key_state(|c| &mut c.right);
            self.viewangles[1] += speed * cv.cl_yawspeed * self.key_state(|c| &mut c.left);
        }
        self.viewangles[0] -= speed * cv.cl_pitchspeed * self.key_state(|c| &mut c.lookup);
        self.viewangles[0] += speed * cv.cl_pitchspeed * self.key_state(|c| &mut c.lookdown);
    }

    /// `CL_KeyMove`: forward/side/up from the movement kbuttons; 127 when
    /// running (+speed xor cl_run), else 64 with BUTTON_WALKING.
    pub fn key_move(&mut self, cmd: &mut UserCmd, cv: &InputCvars) {
        let movespeed = if self.speed.active ^ (cv.cl_run != 0) {
            cmd.buttons &= !BUTTON_WALKING;
            127.0
        } else {
            cmd.buttons |= BUTTON_WALKING;
            64.0
        };
        let mut side = 0.0;
        if self.strafe.active {
            side += movespeed * self.key_state(|c| &mut c.right);
            side -= movespeed * self.key_state(|c| &mut c.left);
        }
        side += movespeed * self.key_state(|c| &mut c.moveright);
        side -= movespeed * self.key_state(|c| &mut c.moveleft);
        let mut up = 0.0;
        up += movespeed * self.key_state(|c| &mut c.up);
        up -= movespeed * self.key_state(|c| &mut c.down);
        let mut forward = 0.0;
        forward += movespeed * self.key_state(|c| &mut c.forward);
        forward -= movespeed * self.key_state(|c| &mut c.back);
        // (int) truncation of the float sums
        cmd.forwardmove = clamp_char(forward as i32);
        cmd.rightmove = clamp_char(side as i32);
        cmd.upmove = clamp_char(up as i32);
    }

    /// `CL_MouseMove`: sensitivity (and cl_mouseAccel), cl.cgameSensitivity,
    /// m_yaw / m_pitch (+strafe and no-freelook move instead).
    pub fn mouse_move(&mut self, cmd: &mut UserCmd, cv: &InputCvars, cgame_sensitivity: f32) {
        // allow mouse smoothing
        let (mut mx, mut my) = if cv.m_filter != 0 {
            ((self.mouse_dx[0] + self.mouse_dx[1]) * 0.5, (self.mouse_dy[0] + self.mouse_dy[1]) * 0.5)
        } else {
            (self.mouse_dx[self.mouse_index], self.mouse_dy[self.mouse_index])
        };
        self.mouse_index ^= 1;
        self.mouse_dx[self.mouse_index] = 0.0;
        self.mouse_dy[self.mouse_index] = 0.0;
        if mx == 0.0 && my == 0.0 {
            return;
        }
        let fm = self.frame_msec.max(1) as f32;
        if cv.cl_mouse_accel != 0.0 {
            if cv.cl_mouse_accel_style == 0 {
                let rate = (mx * mx + my * my).sqrt() / fm;
                let accel = cv.sensitivity + rate * cv.cl_mouse_accel;
                mx *= accel;
                my *= accel;
            } else {
                let rate = [mx.abs() / fm, my.abs() / fm];
                let power = [(rate[0] / cv.cl_mouse_accel_offset).powf(cv.cl_mouse_accel), (rate[1] / cv.cl_mouse_accel_offset).powf(cv.cl_mouse_accel)];
                mx = cv.sensitivity * (mx + if mx < 0.0 { -power[0] } else { power[0] } * cv.cl_mouse_accel_offset);
                my = cv.sensitivity * (my + if my < 0.0 { -power[1] } else { power[1] } * cv.cl_mouse_accel_offset);
            }
        } else {
            mx *= cv.sensitivity;
            my *= cv.sensitivity;
        }
        // ingame FOV
        mx *= cgame_sensitivity;
        my *= cgame_sensitivity;
        // add mouse X/Y movement to cmd
        if self.strafe.active {
            cmd.rightmove = clamp_char(cmd.rightmove as i32 + (cv.m_side * mx) as i32);
        } else {
            self.viewangles[1] -= cv.m_yaw * mx;
        }
        if (self.mlooking || cv.cl_freelook != 0) && !self.strafe.active {
            self.viewangles[0] += cv.m_pitch * my;
        } else {
            cmd.forwardmove = clamp_char(cmd.forwardmove as i32 - (cv.m_forward * my) as i32);
        }
    }

    /// `CL_CmdButtons`: kbutton n is button bit 1 << n (sent even if pressed
    /// and released within the frame); BUTTON_TALK with a key catcher,
    /// BUTTON_ANY while any key is down.
    pub fn cmd_buttons(&mut self, cmd: &mut UserCmd, catcher: i32, anykeydown: bool) {
        self.pressed_only = 0;
        for i in 0..15 {
            if self.buttons[i].active || self.buttons[i].was_pressed {
                cmd.buttons |= 1 << i;
                if !self.buttons[i].active {
                    self.pressed_only |= 1 << i;
                }
            }
            self.buttons[i].was_pressed = false;
        }
        if catcher != 0 {
            cmd.buttons |= BUTTON_TALK;
        }
        // allow the game to know if any key at all is currently pressed,
        // even if it isn't bound to anything
        if anykeydown && catcher == 0 {
            cmd.buttons |= BUTTON_ANY;
        }
    }

    /// A created cmd's buttons without the ones that were only pressed
    /// and released within its frame (for the later ticks of that frame).
    pub fn held_buttons(&self, buttons: i32) -> i32 {
        buttons & !self.pressed_only
    }

    /// `CL_CreateCmd` (`CL_AdjustAngles`, `CL_CmdButtons`, `CL_KeyMove`,
    /// `CL_MouseMove`, `CL_JoystickMove`, the 90 degree
    /// pitch wrap guard, `CL_FinishMove`). `frametime` is cls.frametime.
    pub fn create_cmd(&mut self, cv: &InputCvars, fr: &FrameIn, frametime: u32) -> UserCmd {
        let old = self.viewangles;
        // keyboard angle adjustment
        self.adjust_angles(cv, frametime);
        let mut cmd = UserCmd::default();
        self.cmd_buttons(&mut cmd, fr.catcher, fr.anykeydown);
        // get basic movement from keyboard
        self.key_move(&mut cmd, cv);
        // get basic movement from mouse
        self.mouse_move(&mut cmd, cv, fr.cgame_sensitivity);
        // get basic movement from joystick
        self.joystick_move(&mut cmd, cv, frametime);
        // check to make sure the angles haven't wrapped
        if self.viewangles[0] - old[0] > 90.0 {
            self.viewangles[0] = old[0] + 90.0;
        } else if old[0] - self.viewangles[0] > 90.0 {
            self.viewangles[0] = old[0] - 90.0;
        }
        // CL_FinishMove: the state that the cgame is currently sending
        cmd.weapon = fr.weapon;
        cmd.server_time = fr.server_time;
        for i in 0..3 {
            cmd.angles[i] = angle2short_f(self.viewangles[i]);
        }
        cmd
    }

    /// `CL_JoystickEvent`: joystick values stay set until changed.
    pub fn joystick_event(&mut self, axis: usize, value: i32) {
        if let Some(a) = self.joystick_axis.get_mut(axis) {
            *a = value;
        }
    }

    /// `CL_JoystickMove`.
    pub fn joystick_move(&mut self, cmd: &mut UserCmd, cv: &InputCvars, frametime: u32) {
        if !(self.speed.active ^ (cv.cl_run != 0)) {
            cmd.buttons |= BUTTON_WALKING;
        }
        let anglespeed = if self.speed.active { 0.001 * frametime as f32 * cv.cl_anglespeedkey } else { 0.001 * frametime as f32 };
        let ax = |n: usize| self.joystick_axis[cv.j_axes[n]] as f32;
        let [pitch, yaw, forward, side, up] = [0, 1, 2, 3, 4].map(ax);
        if !self.strafe.active {
            self.viewangles[1] += anglespeed * cv.j_yaw * yaw;
            cmd.rightmove = clamp_char(cmd.rightmove as i32 + (cv.j_side * side) as i32);
        } else {
            self.viewangles[1] += anglespeed * cv.j_side * side;
            cmd.rightmove = clamp_char(cmd.rightmove as i32 + (cv.j_yaw * yaw) as i32);
        }
        if self.mlooking {
            self.viewangles[0] += anglespeed * cv.j_forward * forward;
            cmd.forwardmove = clamp_char(cmd.forwardmove as i32 + (cv.j_pitch * pitch) as i32);
        } else {
            self.viewangles[0] += anglespeed * cv.j_pitch * pitch;
            cmd.forwardmove = clamp_char(cmd.forwardmove as i32 + (cv.j_forward * forward) as i32);
        }
        cmd.upmove = clamp_char(cmd.upmove as i32 + (cv.j_up * up) as i32);
    }

    /// The kbuttons' state forgotten (a new map / `in_restart`).
    pub fn clear(&mut self) {
        let angles = self.viewangles;
        *self = ClInput { viewangles: angles, old_com_frame_time: self.old_com_frame_time, ..Default::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(i: &mut ClInput, line: &str) {
        let a = crate::cmd::tokenize(line);
        assert!(i.command(&a, 1, 0), "{line}");
    }

    #[test]
    fn two_keys_hold_one_button() {
        let mut i = ClInput::default();
        run(&mut i, "+forward 119 100");
        run(&mut i, "+forward 132 110");
        assert!(i.forward.active);
        assert_eq!(i.forward.downtime, 100);
        run(&mut i, "-forward 119 120");
        assert!(i.forward.active, "still held by UPARROW");
        // a release from a key that never pressed it is a menu pass through
        run(&mut i, "-forward 50 125");
        assert!(i.forward.active);
        run(&mut i, "-forward 132 130");
        assert!(!i.forward.active);
        assert_eq!(i.forward.msec, 30);
        // typed at the console: down without a key, up clears
        run(&mut i, "+attack");
        assert!(i.buttons[0].active);
        run(&mut i, "-attack");
        assert!(!i.buttons[0].active);
        // a third key is refused
        run(&mut i, "+back 1 0");
        run(&mut i, "+back 2 0");
        run(&mut i, "+back 3 0");
        assert_eq!(i.back.down, [1, 2]);
        assert!(!i.command(&crate::cmd::tokenize("+button15 1 0"), 1, 0));
        assert!(!i.command(&crate::cmd::tokenize("+nothing 1 0"), 1, 0));
    }

    #[test]
    fn key_move_values() {
        let cv = InputCvars::default();
        let mut i = ClInput::default();
        i.new_frame(1000);
        i.new_frame(1016);
        run(&mut i, "+forward 119 0");
        run(&mut i, "+moveright 100 0");
        run(&mut i, "+moveup 32 0");
        let mut cmd = UserCmd::default();
        i.key_move(&mut cmd, &cv);
        // cl_run 1: full speed, not walking
        assert_eq!((cmd.forwardmove, cmd.rightmove, cmd.upmove), (127, 127, 127));
        assert_eq!(cmd.buttons & BUTTON_WALKING, 0);
        // +speed with cl_run 1 walks at 64
        run(&mut i, "+speed 138 0");
        i.new_frame(1032);
        let mut cmd = UserCmd::default();
        i.key_move(&mut cmd, &cv);
        assert_eq!((cmd.forwardmove, cmd.rightmove), (64, 64));
        assert_eq!(cmd.buttons & BUTTON_WALKING, BUTTON_WALKING);
        // +movedown and +moveup cancel
        run(&mut i, "+movedown 99 0");
        i.new_frame(1048);
        let mut cmd = UserCmd::default();
        i.key_move(&mut cmd, &cv);
        assert_eq!(cmd.upmove, 0);
        // a key pressed for half of the frame moves half as far
        let mut i = ClInput::default();
        i.new_frame(2000);
        i.new_frame(2020);
        run(&mut i, "+back 115 2010");
        let mut cmd = UserCmd::default();
        i.key_move(&mut cmd, &cv);
        assert_eq!(cmd.forwardmove, -63);
        // +strafe turns +left/+right into sidesteps
        run(&mut i, "-back 115 2020");
        run(&mut i, "+strafe 136 0");
        run(&mut i, "+left 134 0");
        i.new_frame(2040);
        let mut cmd = UserCmd::default();
        i.key_move(&mut cmd, &cv);
        assert_eq!(cmd.rightmove, -127);
    }

    #[test]
    fn joystick_move_axes_with_the_j_cvars() {
        let cv = InputCvars::default();
        let mut i = ClInput::default();
        // left stick full forward (axis 1 = j_forward_axis), right stick x
        // (axis 3 = j_pitch_axis), axis 4 (j_yaw_axis) half right
        i.joystick_event(1, -32767);
        i.joystick_event(3, 1000);
        i.joystick_event(4, 16000);
        i.joystick_event(0, 400);
        let mut cmd = UserCmd::default();
        i.joystick_move(&mut cmd, &cv, 100);
        // ClampChar( (int)( -0.25 * -32767 ) )
        assert_eq!(cmd.forwardmove, 127);
        assert_eq!(cmd.rightmove, 100);
        assert!((i.viewangles[0] - 0.1 * 0.022 * 1000.0).abs() < 1e-4);
        assert!((i.viewangles[1] - 0.1 * -0.022 * 16000.0).abs() < 1e-3);
        // cl_run 1 without +speed: not walking
        assert_eq!(cmd.buttons & BUTTON_WALKING, 0);
        // +mlook swaps forward and pitch: the stick's forward pitches
        i.mlooking = true;
        i.viewangles = [0.0; 3];
        let mut cmd = UserCmd::default();
        i.joystick_move(&mut cmd, &cv, 100);
        assert_eq!(cmd.forwardmove, 22);
        assert!((i.viewangles[0] - 0.1 * -0.25 * -32767.0).abs() < 1e-2);
    }

    #[test]
    fn create_cmd_buttons_mouse_and_angles() {
        let cv = InputCvars::default();
        let mut i = ClInput::default();
        i.new_frame(16);
        // a click within one frame is still sent (wasPressed)
        run(&mut i, "+attack 178 5");
        run(&mut i, "-attack 178 9");
        run(&mut i, "+button5 179 10");
        i.mouse_event(10.0, -20.0);
        let fr = FrameIn { cgame_sensitivity: 1.0, weapon: 7, ..Default::default() };
        let cmd = i.create_cmd(&cv, &fr, 16);
        assert_eq!(cmd.buttons & 1, 1);
        assert_eq!(cmd.buttons & 32, 32);
        assert_eq!(i.held_buttons(cmd.buttons) & (1 | 32), 32);
        assert_eq!(cmd.weapon, 7);
        // sensitivity 5 * m_yaw 0.022
        assert!((i.viewangles[1] + 10.0 * 5.0 * 0.022).abs() < 1e-5);
        assert!((i.viewangles[0] + 20.0 * 5.0 * 0.022).abs() < 1e-5);
        assert_eq!(cmd.angles[1], angle2short_f(i.viewangles[1]));
        i.new_frame(32);
        let fr = FrameIn { catcher: 2, ..fr };
        let cmd = i.create_cmd(&cv, &fr, 16);
        assert_eq!(cmd.buttons & 1, 0, "the click was sent once");
        assert_eq!(cmd.buttons & (32 | BUTTON_TALK), 32 | BUTTON_TALK);
        // zoomed: cg.zoomSensitivity scales the turn
        let mut j = ClInput::default();
        j.mouse_event(10.0, 0.0);
        j.create_cmd(&cv, &FrameIn { cgame_sensitivity: 0.5, ..Default::default() }, 16);
        assert!((j.viewangles[1] + 10.0 * 5.0 * 0.022 * 0.5).abs() < 1e-5);
        // cl_freelook 0 without +mlook: mouse y moves instead of looking
        let nofree = InputCvars { cl_freelook: 0, ..cv };
        let mut j = ClInput::default();
        j.mouse_event(0.0, 4.0);
        let cmd = j.create_cmd(&nofree, &FrameIn { cgame_sensitivity: 1.0, ..Default::default() }, 16);
        assert_eq!(j.viewangles[0], 0.0);
        assert_eq!(cmd.forwardmove, -5);
        // centerview: pitch from the delta angles
        run(&mut j, "centerview");
        assert_eq!(j.viewangles[0], 0.0);
    }
}
