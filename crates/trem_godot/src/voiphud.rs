//! The port's voice-chat HUD (dec053): the microphone's state (on, off,
//! talking, with the capture level) and the players heard talking, drawn
//! by TremConsole over the game like `SCR_DrawVoipMeter` (640x480 virtual
//! units, the console's bigchars at 10 units, at the left between the chat
//! and the tutorial lines; dec056). `cl_voipHud 0` hides it.

use crate::console::ConDraw;

/// How long a talker stays listed after its last voice packet (ms).
pub const TALKER_HOLD_MS: u64 = 400;
/// How long a microphone note ("MIC OFF", a refusal) stays (ms).
pub const NOTE_MS: u64 = 3000;
/// The local player's "talking" floor with voice activation off (the
/// capture power's 0..100 scale: about -40 dBFS RMS). Everything is sent
/// then, so cl_voipVADThreshold (0.25, -26 dBFS) is not what decides it,
/// and normal speech on a default-gain microphone stays under it.
pub const SPEECH_FLOOR: f32 = 0.01;

/// The power over which the local player counts as talking: what voice
/// activation sends (cl_voipVADThreshold) with it on, else SPEECH_FLOOR.
pub fn talk_threshold(use_vad: bool, vad_threshold: f32) -> f32 {
    if use_vad {
        vad_threshold.max(SPEECH_FLOOR)
    } else {
        SPEECH_FLOOR
    }
}

/// What the HUD shows this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoipHud {
    /// the toggle (or push-to-talk) is capturing
    pub mic_on: bool,
    /// the captured level is over cl_voipVADThreshold
    pub talking: bool,
    /// the capture level, 0..1
    pub level: f32,
    /// a note under the microphone line: (text, is a warning)
    pub note: Option<(String, bool)>,
    /// names of the players heard talking
    pub talkers: Vec<String>,
}

impl VoipHud {
    pub fn is_empty(&self) -> bool {
        !self.mic_on && self.note.is_none() && self.talkers.is_empty()
    }
}

/// `cl_voipCaptureMult`-scaled power (CL_CaptureVoip's clc.voipPower,
/// 0..100 scale) as a 0..1 bar, like SCR_DrawVoipMeter's `power * 10`.
pub fn level_of(power: f32) -> f32 {
    (power * 10.0).clamp(0.0, 1.0)
}

/// A name without its ^N colour codes, at most `max` characters.
pub fn plain_name(name: &str, max: usize) -> String {
    let mut out = String::new();
    let mut it = name.chars().peekable();
    while let Some(c) = it.next() {
        if c == '^' && it.peek().is_some_and(|n| *n != '^') {
            it.next();
            continue;
        }
        if c.is_ascii() && !c.is_ascii_control() {
            out.push(c);
        }
    }
    out.chars().take(max).collect()
}

const X: f32 = 10.0;
/// the top of the panel: below the chat / notify lines, above the
/// tutorial text and the HUD's bottom-left rings
const Y: f32 = 190.0;
/// character size
const CH: f32 = 10.0;
const LINE: f32 = 14.0;
const GREEN: [f32; 4] = [0.3, 1.0, 0.3, 1.0];
const BRIGHT: [f32; 4] = [0.6, 1.0, 0.6, 1.0];
const GREY: [f32; 4] = [0.7, 0.7, 0.7, 1.0];
const YELLOW: [f32; 4] = [1.0, 0.85, 0.2, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// CH-unit text with the console's drop shadow.
fn text(out: &mut Vec<ConDraw>, x: f32, y: f32, s: &str, color: [f32; 4]) {
    for (off, c) in [(1.0, [0.0, 0.0, 0.0, color[3]]), (0.0, color)] {
        for (i, ch) in s.bytes().enumerate() {
            if ch != b' ' {
                out.push(ConDraw::VirtChar(x + i as f32 * CH + off, y + off, CH, ch, c));
            }
        }
    }
}

/// The HUD's draw calls.
pub fn draw(h: &VoipHud) -> Vec<ConDraw> {
    let mut out = Vec::new();
    let mut y = Y;
    if h.mic_on {
        out.push(ConDraw::Fill([X - 4.0, y - 4.0, 182.0, CH + 8.0], [0.0, 0.0, 0.0, 0.55]));
        let (label, color) = if h.talking { ("TALKING", BRIGHT) } else { ("MIC ON", GREEN) };
        text(&mut out, X, y, label, color);
        // the level bar
        let (bx, bw) = (X + 7.0 * CH + 6.0, 98.0);
        out.push(ConDraw::Fill([bx, y + 2.0, bw, CH - 4.0], [0.2, 0.2, 0.2, 0.8]));
        if h.level > 0.0 {
            out.push(ConDraw::Fill([bx, y + 2.0, bw * h.level.clamp(0.0, 1.0), CH - 4.0], color));
        }
        y += LINE + 2.0;
    }
    if let Some((note, warn)) = &h.note {
        // wrapped at 30 characters
        let words: Vec<&str> = note.split(' ').collect();
        let mut line = String::new();
        let mut lines = Vec::new();
        for w in words {
            if !line.is_empty() && line.len() + 1 + w.len() > 30 {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(w);
        }
        if !line.is_empty() {
            lines.push(line);
        }
        for l in lines {
            text(&mut out, X, y, &l, if *warn { YELLOW } else { GREY });
            y += LINE - 2.0;
        }
        y += 2.0;
    }
    for name in &h.talkers {
        text(&mut out, X, y, ")))", GREEN);
        text(&mut out, X + 4.0 * CH, y, name, WHITE);
        y += LINE - 2.0;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(d: &[ConDraw]) -> String {
        // the white / coloured pass only (every second VirtChar run is the shadow)
        d.iter().filter_map(|c| if let ConDraw::VirtChar(_, _, _, ch, col) = c { (col[0] > 0.0).then_some(*ch as char) } else { None }).collect()
    }

    #[test]
    fn hud_states() {
        assert!(VoipHud::default().is_empty());
        assert!(draw(&VoipHud::default()).is_empty());
        let on = VoipHud { mic_on: true, level: 0.5, ..Default::default() };
        assert_eq!(chars(&draw(&on)), "MICON");
        let fills = draw(&on).iter().filter(|d| matches!(d, ConDraw::Fill(..))).count();
        assert_eq!(fills, 3, "panel, bar background, level");
        let talking = VoipHud { mic_on: true, talking: true, level: 0.9, ..Default::default() };
        assert_eq!(chars(&draw(&talking)), "TALKING");
        let off = VoipHud { note: Some(("MIC OFF".into(), false)), ..Default::default() };
        assert_eq!(chars(&draw(&off)), "MICOFF");
        let heard = VoipHud { talkers: vec!["Bob".into()], ..Default::default() };
        assert_eq!(chars(&draw(&heard)), ")))Bob");
    }

    #[test]
    fn names_and_levels() {
        assert_eq!(plain_name("^1Gr^7anger", 32), "Granger");
        assert_eq!(plain_name("abcdef", 3), "abc");
        assert_eq!(level_of(0.05), 0.5);
        assert_eq!(level_of(3.0), 1.0);
    }

    #[test]
    fn local_talk_threshold() {
        // voice activation off: speech at -35 dBFS RMS (power ~0.03) is talking
        assert!(0.03 >= talk_threshold(false, 0.25));
        // on: what voice activation sends
        assert_eq!(talk_threshold(true, 0.25), 0.25);
        assert_eq!(talk_threshold(true, 0.0), SPEECH_FLOOR);
    }
}
