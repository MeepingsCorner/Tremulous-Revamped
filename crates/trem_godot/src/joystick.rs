//! sdl_input.c's joystick (`IN_InitJoystick`, `IN_JoyMove`): buttons,
//! hats and axes of the opened stick become key events (K_JOY1..K_JOY32,
//! the arrow keys for the first two axes) or, with in_joystickUseAnalog,
//! `SE_JOYSTICK_AXIS` events for cl_input.c `CL_JoystickMove` (input.rs).
//! Legacy's SDL2 code has no game-controller path (no `IN_GamepadMove`,
//! no K_PAD0_* keys): it reads the raw joystick.
//!
//! Godot reports a pad in the SDL game-controller layout, so the device is
//! presented to this code the way SDL's raw joystick API reports the most
//! common pad (an Xbox-style controller under the Linux xpad driver):
//! buttons A B X Y LB RB BACK START GUIDE LSTICK RSTICK (K_JOY1..11), the
//! d-pad as hat 0 (K_JOY29..32), axes 0..5 = left x, left y, left trigger,
//! right x, right y, right trigger. Godot's triggers rest at 0 where the
//! raw driver's rest at -32768, so a released trigger is not a held key.
//! Pure Rust; the Godot polling is `poll_godot`.

use crate::cmd::{Cvars, CVAR_ARCHIVE, CVAR_ROM};
use crate::keys::{K_DOWNARROW, K_JOY1, K_LEFTARROW, K_RIGHTARROW, K_UPARROW};

/// qcommon.h MAX_JOYSTICK_AXIS
pub const MAX_JOYSTICK_AXIS: usize = 16;

/// K_JOYn as a key number.
const fn joy(n: i32) -> i32 {
    K_JOY1 + n - 1
}

/// `joy_keys[]`: axis movement translated into keypresses (negative,
/// positive per axis).
pub const JOY_KEYS: [i32; 16] = [K_LEFTARROW, K_RIGHTARROW, K_UPARROW, K_DOWNARROW, joy(17), joy(18), joy(19), joy(20), joy(21), joy(22), joy(23), joy(24), joy(25), joy(26), joy(27), joy(28)];

/// `hat_keys[]`: the 4 highest buttons are used for the first hat (up,
/// right, down, left per hat).
pub const HAT_KEYS: [i32; 16] = [joy(29), joy(30), joy(31), joy(32), joy(25), joy(26), joy(27), joy(28), joy(21), joy(22), joy(23), joy(24), joy(17), joy(18), joy(19), joy(20)];

/// SDL_HAT_* bits.
pub const HAT_UP: u8 = 1;
pub const HAT_RIGHT: u8 = 2;
pub const HAT_DOWN: u8 = 4;
pub const HAT_LEFT: u8 = 8;

/// An event `IN_JoyMove` queues (`Com_QueueEvent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoyEvent {
    /// SE_KEY
    Key(i32, bool),
    /// SE_JOYSTICK_AXIS (axis, value)
    Axis(usize, i32),
}

/// One poll of the opened stick (`SDL_JoystickGet*`).
#[derive(Clone, Debug, Default)]
pub struct StickInput {
    pub buttons: Vec<bool>,
    pub hats: Vec<u8>,
    /// -32768..32767
    pub axes: Vec<i16>,
}

/// `IN_Init`'s joystick cvars (sdl_input.c / `IN_InitJoystick`).
pub fn register_cvars(c: &mut Cvars) {
    // CVAR_LATCH: read by IN_InitJoystick (here: the first frame and in_restart)
    c.get("in_joystick", "0", CVAR_ARCHIVE);
    c.get("joy_threshold", "0.15", CVAR_ARCHIVE);
    c.get("in_joystickNo", "0", CVAR_ARCHIVE);
    c.get("in_joystickUseAnalog", "0", CVAR_ARCHIVE);
    c.get("in_availableJoysticks", "", CVAR_ROM);
}

/// `stick_state` and whether a stick is open (`stick != NULL`).
#[derive(Clone, Debug, Default)]
pub struct JoyState {
    pub inited: bool,
    /// in_joystickNo of the opened stick
    pub stick: Option<usize>,
    pub use_analog: bool,
    buttons: [bool; 16],
    oldaxes: u32,
    oldaaxes: [i32; MAX_JOYSTICK_AXIS],
    oldhats: u32,
}

/// The hat's key events for `bits` (`IN_JoyMove`'s switch: diagonals are two keys).
fn hat_events(out: &mut Vec<JoyEvent>, hat: usize, bits: u8, down: bool) {
    let keys = &HAT_KEYS[4 * hat..4 * hat + 4];
    // only the eight SDL hat values are handled (centred / other: nothing)
    let parts: &[usize] = match bits {
        HAT_UP => &[0],
        HAT_RIGHT => &[1],
        HAT_DOWN => &[2],
        HAT_LEFT => &[3],
        b if b == HAT_RIGHT | HAT_UP => &[0, 1],
        b if b == HAT_RIGHT | HAT_DOWN => &[2, 1],
        b if b == HAT_LEFT | HAT_UP => &[0, 3],
        b if b == HAT_LEFT | HAT_DOWN => &[2, 3],
        _ => &[],
    };
    for &p in parts {
        out.push(JoyEvent::Key(keys[p], down));
    }
}

impl JoyState {
    /// `IN_InitJoystick`: `names` are the connected devices
    /// (in_availableJoysticks); opens in_joystickNo when in_joystick is set.
    pub fn init(&mut self, c: &mut Cvars, names: &[String]) {
        *self = JoyState { inited: true, ..Default::default() };
        let list: String = names.iter().map(|n| format!("{n}\n")).collect();
        c.get("in_availableJoysticks", &list, CVAR_ROM);
        c.set("in_availableJoysticks", &list);
        if c.integer("in_joystick") == 0 {
            return;
        }
        let total = names.len() as i32;
        if c.integer("in_joystickNo") < 0 || c.integer("in_joystickNo") >= total {
            c.set("in_joystickNo", "0");
        }
        self.use_analog = c.integer("in_joystickUseAnalog") != 0;
        let no = c.integer("in_joystickNo");
        if (0..total).contains(&no) {
            self.stick = Some(no as usize);
        }
    }

    /// `IN_JoyMove`: the events since the last poll.
    pub fn joy_move(&mut self, threshold: f32, s: &StickInput) -> Vec<JoyEvent> {
        let mut out = Vec::new();
        if self.stick.is_none() {
            return out;
        }
        // now query the stick buttons...
        for (i, &pressed) in s.buttons.iter().take(self.buttons.len()).enumerate() {
            if pressed != self.buttons[i] {
                out.push(JoyEvent::Key(K_JOY1 + i as i32, pressed));
                self.buttons[i] = pressed;
            }
        }
        // look at the hats...
        let mut hats = 0u32;
        for (i, &h) in s.hats.iter().take(4).enumerate() {
            hats |= (h as u32) << (8 * i);
        }
        if hats != self.oldhats {
            for i in 0..4 {
                let (new, old) = ((hats >> (8 * i)) as u8, (self.oldhats >> (8 * i)) as u8);
                if new != old {
                    // release event, then press event
                    hat_events(&mut out, i, old, false);
                    hat_events(&mut out, i, new, true);
                }
            }
        }
        self.oldhats = hats;
        // finally, look at the axes...
        let mut axes = 0u32;
        if self.use_analog {
            for (i, &a) in s.axes.iter().take(MAX_JOYSTICK_AXIS).enumerate() {
                let mut axis = a as i32;
                let f = axis.unsigned_abs() as f32 / 32767.0;
                if f < threshold {
                    axis = 0;
                }
                if axis != self.oldaaxes[i] {
                    out.push(JoyEvent::Axis(i, axis));
                    self.oldaaxes[i] = axis;
                }
            }
        } else {
            for (i, &a) in s.axes.iter().take(16).enumerate() {
                let f = a as f32 / 32767.0;
                if f < -threshold {
                    axes |= 1 << (i * 2);
                } else if f > threshold {
                    axes |= 1 << (i * 2 + 1);
                }
            }
        }
        // Time to update axes state based on old vs. new.
        if axes != self.oldaxes {
            for (i, &key) in JOY_KEYS.iter().enumerate() {
                if axes & (1 << i) != 0 && self.oldaxes & (1 << i) == 0 {
                    out.push(JoyEvent::Key(key, true));
                }
                if axes & (1 << i) == 0 && self.oldaxes & (1 << i) != 0 {
                    out.push(JoyEvent::Key(key, false));
                }
            }
        }
        self.oldaxes = axes;
        out
    }
}

/// Godot's game-controller button order for the raw xpad order above
/// (A B X Y LB RB BACK START GUIDE LSTICK RSTICK, then the extra buttons).
const GODOT_BUTTONS: [i32; 16] = [0, 1, 2, 3, 9, 10, 4, 6, 5, 7, 8, 15, 16, 17, 18, 19];

/// `SDL_JoystickUpdate` + the SDL_JoystickGet* reads, from Godot's Input
/// for the `n`th connected pad (`in_joystickNo`).
pub fn poll_godot(n: usize) -> Option<StickInput> {
    use godot::classes::Input;
    use godot::global::{JoyAxis, JoyButton};
    use godot::obj::{EngineEnum, Singleton};
    let input = Input::singleton();
    let pads = input.get_connected_joypads();
    let dev = pads.get(n)? as i32;
    let pressed = |b: i32| input.is_joy_button_pressed(dev, JoyButton::from_ord(b));
    let buttons = GODOT_BUTTONS.iter().map(|&b| pressed(b)).collect();
    let mut hat = 0u8;
    for (b, bit) in [(JoyButton::DPAD_UP, HAT_UP), (JoyButton::DPAD_RIGHT, HAT_RIGHT), (JoyButton::DPAD_DOWN, HAT_DOWN), (JoyButton::DPAD_LEFT, HAT_LEFT)] {
        if input.is_joy_button_pressed(dev, b) {
            hat |= bit;
        }
    }
    let ax = |a: JoyAxis| (input.get_joy_axis(dev, a).clamp(-1.0, 1.0) * 32767.0) as i16;
    let axes = vec![ax(JoyAxis::LEFT_X), ax(JoyAxis::LEFT_Y), ax(JoyAxis::TRIGGER_LEFT), ax(JoyAxis::RIGHT_X), ax(JoyAxis::RIGHT_Y), ax(JoyAxis::TRIGGER_RIGHT)];
    Some(StickInput { buttons, hats: vec![hat], axes })
}

/// The connected pads' names (`SDL_JoystickNameForIndex`).
pub fn godot_joystick_names() -> Vec<String> {
    use godot::obj::Singleton;
    let input = godot::classes::Input::singleton();
    input.get_connected_joypads().iter_shared().map(|d| input.get_joy_name(d as i32).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(analog: bool) -> JoyState {
        let mut c = Cvars::default();
        register_cvars(&mut c);
        c.set("in_joystick", "1");
        c.set("in_joystickUseAnalog", if analog { "1" } else { "0" });
        let mut j = JoyState::default();
        j.init(&mut c, &["pad".into()]);
        assert_eq!(c.string("in_availableJoysticks"), "pad\n");
        j
    }

    #[test]
    fn closed_unless_in_joystick() {
        let mut c = Cvars::default();
        register_cvars(&mut c);
        let mut j = JoyState::default();
        j.init(&mut c, &["pad".into()]);
        assert_eq!(j.stick, None);
        let s = StickInput { buttons: vec![true], ..Default::default() };
        assert!(j.joy_move(0.15, &s).is_empty());
    }

    #[test]
    fn buttons_hats_and_digital_axes_are_keys() {
        let mut j = opened(false);
        let mut s = StickInput { buttons: vec![false; 11], hats: vec![0], axes: vec![0; 6] };
        assert!(j.joy_move(0.15, &s).is_empty());
        s.buttons[0] = true;
        s.hats[0] = HAT_RIGHT | HAT_UP;
        s.axes[1] = -20000; // left stick forward: UPARROW
        s.axes[3] = 32767; // axis 3 positive: K_JOY20
        let ev = j.joy_move(0.15, &s);
        assert_eq!(ev, vec![JoyEvent::Key(K_JOY1, true), JoyEvent::Key(joy(29), true), JoyEvent::Key(joy(30), true), JoyEvent::Key(K_UPARROW, true), JoyEvent::Key(joy(20), true)]);
        // below the threshold: released
        s.axes[1] = -3000;
        s.hats[0] = HAT_DOWN;
        let ev = j.joy_move(0.15, &s);
        assert_eq!(ev, vec![JoyEvent::Key(joy(29), false), JoyEvent::Key(joy(30), false), JoyEvent::Key(joy(31), true), JoyEvent::Key(K_UPARROW, false)]);
        assert!(j.joy_move(0.15, &s).is_empty());
    }

    #[test]
    fn analog_axes_are_axis_events_with_a_dead_zone() {
        let mut j = opened(true);
        let mut s = StickInput { axes: vec![0; 6], ..Default::default() };
        s.axes[0] = 16000;
        s.axes[4] = 1000; // inside joy_threshold: 0
        assert_eq!(j.joy_move(0.15, &s), vec![JoyEvent::Axis(0, 16000)]);
        s.axes[0] = 0;
        assert_eq!(j.joy_move(0.15, &s), vec![JoyEvent::Axis(0, 0)]);
    }
}
