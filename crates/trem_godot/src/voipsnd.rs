//! The sound-system half of VoIP (dec038) for the kernel's `CL_CaptureVoip`
//! / `CL_PlayVoip` (trem_sim kernel voip.rs): snd_openal.c's
//! `S_AL_StartCapture`, `S_AL_AvailableCaptureSamples`, `S_AL_Capture`,
//! `S_AL_StopCapture` (48 kHz mono 16-bit, the capture device the C opens
//! with `alcCaptureOpenDevice( 48000, AL_FORMAT_MONO16 )`), `S_AL_MasterGain`
//! (the listener gain: here the Master bus on top of s_volume) and
//! `S_AL_RawSamples` for the VoIP streams (`sender + 1` direct at the
//! player's voip gain, `sender + MAX_CLIENTS + 1` spatialized at the
//! talker's entity like `S_SpatializeOrigin` / an OpenAL source at the
//! entity's origin).
//!
//! The microphone is an AudioStreamMicrophone on a "VoipCapture" bus that
//! holds an AudioEffectCapture and sends nothing on (its fader is at -80 dB:
//! the effect sees the signal before the fader); the bus runs at the output
//! mix rate, resampled here to 48 kHz. A synthetic source (a tone; tests and
//! machines without a microphone) can stand in for it (`set_test_tone`).

use crate::cmd::Cvars;
use crate::geom::q2g;
use godot::classes::{AudioEffectCapture, AudioServer, AudioStreamGenerator, AudioStreamGeneratorPlayback, AudioStreamMicrophone, AudioStreamPlayer, AudioStreamPlayer3D, Node};
use godot::prelude::*;
use std::collections::VecDeque;
use trem_sim::kernel::voip::{VoipHost, VoipPlayback, MAX_CLIENTS};

/// The capture bus.
const CAPTURE_BUS: &str = "VoipCapture";
/// `alcCaptureOpenDevice`'s rate.
const RATE: f64 = 48000.0;
/// The capture device's own buffer: what is not read within it is lost.
const MAX_BUFFERED: usize = 48000;

/// The VoIP sound state of the Godot host.
#[derive(Default)]
pub struct VoipSound {
    mic: Option<Gd<AudioStreamPlayer>>,
    capture_fx: Option<Gd<AudioEffectCapture>>,
    /// 48 kHz mono samples captured and not read yet
    buf: VecDeque<i16>,
    /// resampler position between output-rate frames
    resample_pos: f64,
    last_in: f32,
    capturing: bool,
    /// the capture runs as the microphone test (outside an online game)
    monitoring: bool,
    /// a synthetic source instead of the microphone: (freq, amplitude, phase in samples, start ms)
    test_tone: Option<(f32, f32, u64, u64)>,
    /// S_MasterGain
    master_gain: f32,
    direct: Vec<Option<Gd<AudioStreamPlayer>>>,
    spatial: Vec<Option<Gd<AudioStreamPlayer3D>>>,
    /// samples received per sender (for the logs / tests)
    pub received: Vec<usize>,
    /// peak level of the received voice per sender
    pub peak: Vec<i16>,
    /// when each sender's voice last arrived (ms, Time::get_ticks_msec; the voice HUD)
    pub last_heard: Vec<u64>,
    /// the microphone frames pumped and their peak level (tests, logs)
    pub pumped: u64,
    pub pump_peak: f32,
}

/// Linear gain -> dB (silence below -80 dB).
fn db(g: f32) -> f32 {
    if g <= 0.0001 {
        -80.0
    } else {
        20.0 * g.log10()
    }
}

impl VoipSound {
    /// `S_AL_Init`'s capture device (s_alCapture): the capture bus and
    /// the microphone player, added under `parent`.
    fn ensure_device(&mut self, parent: &mut Gd<Node>) {
        if self.mic.is_some() {
            return;
        }
        let mut a = AudioServer::singleton();
        let mut bus = a.get_bus_index(CAPTURE_BUS);
        if bus < 0 {
            bus = a.get_bus_count();
            a.add_bus();
            a.set_bus_name(bus, CAPTURE_BUS);
            a.set_bus_send(bus, "Master");
            a.set_bus_volume_db(bus, -80.0);
            let mut fx = AudioEffectCapture::new_gd();
            fx.set_buffer_length(1.0);
            a.add_bus_effect(bus, &fx);
        }
        self.capture_fx = a.get_bus_effect(bus, 0).and_then(|e| e.try_cast::<AudioEffectCapture>().ok());
        let mut p = AudioStreamPlayer::new_alloc();
        p.set_name("VoipMicrophone");
        p.set_stream(&AudioStreamMicrophone::new_gd());
        p.set_bus(CAPTURE_BUS);
        parent.add_child(&p);
        self.mic = Some(p);
    }

    /// `s_voipInputDevice`: the capture device by name ("Default": the
    /// system's), as AudioServer lists them (`input_devices`).
    pub fn set_input_device(&mut self, name: &str) {
        let mut a = AudioServer::singleton();
        let known = a.get_input_device_list().as_slice().iter().any(|d| d.to_string() == name);
        let dev = if known { name } else { "Default" };
        a.set_input_device(dev);
        // a capture in progress continues from the new device
        if self.capturing {
            if let Some(m) = self.mic.as_mut() {
                m.stop();
                m.play();
            }
        }
    }

    /// The microphone test outside an online game: the capture runs for
    /// the voice HUD only and nothing is sent. The power (CL_CaptureVoip's
    /// 0..100 scale) of what arrived since the last call, None if nothing.
    pub fn monitor(&mut self, parent: &mut Gd<Node>) -> Option<f32> {
        if !self.capturing {
            self.ensure_device(parent);
            self.capturing = true;
            self.monitoring = true;
            self.buf.clear();
            self.resample_pos = 0.0;
            if let Some(fx) = self.capture_fx.as_mut() {
                fx.clear_buffer();
            }
            if let Some(t) = self.test_tone.as_mut() {
                t.2 = 0;
                t.3 = godot::classes::Time::singleton().get_ticks_msec();
            } else if let Some(m) = self.mic.as_mut() {
                m.play();
            }
        }
        self.pump();
        let n = self.buf.len();
        if n == 0 {
            return None;
        }
        let sum: f32 = self.buf.drain(..).map(|s| s as f32 * s as f32).sum();
        Some(sum / (32768.0 * 32768.0 * n as f32) * 100.0)
    }

    /// The microphone test ends.
    pub fn stop_monitor(&mut self) {
        if std::mem::take(&mut self.monitoring) {
            self.stop();
        }
    }

    pub fn monitoring(&self) -> bool {
        self.monitoring
    }

    /// The microphone's capture effect was found (tests, logs).
    pub fn has_capture_fx(&self) -> bool {
        self.capture_fx.is_some()
    }

    /// 48 kHz samples waiting for the kernel.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// The capture restarted (the voice page's Refresh): the microphone
    /// player stops and plays again and what was buffered is dropped.
    pub fn restart_capture(&mut self) {
        if !self.capturing {
            return;
        }
        self.buf.clear();
        self.resample_pos = 0.0;
        self.last_in = 0.0;
        if let Some(fx) = self.capture_fx.as_mut() {
            fx.clear_buffer();
        }
        if self.test_tone.is_none() {
            if let Some(m) = self.mic.as_mut() {
                m.stop();
                m.play();
            }
        }
    }

    /// The capture devices AudioServer knows ("Default" first).
    pub fn input_devices() -> Vec<String> {
        let mut v: Vec<String> = AudioServer::singleton().get_input_device_list().as_slice().iter().map(|d| d.to_string()).collect();
        v.retain(|d| d != "Default");
        v.insert(0, "Default".into());
        v
    }

    /// A tone in place of the microphone (None: the microphone again).
    pub fn set_test_tone(&mut self, tone: Option<(f32, f32)>) {
        self.test_tone = tone.map(|(f, a)| (f, a, 0, godot::classes::Time::singleton().get_ticks_msec()));
    }

    /// Pull what the device captured into the 48 kHz buffer.
    fn pump(&mut self) {
        if !self.capturing {
            return;
        }
        if let Some((freq, amp, phase, start)) = self.test_tone.as_mut() {
            // the synthetic device delivers 48 samples a millisecond
            let now = godot::classes::Time::singleton().get_ticks_msec();
            let due = (now.saturating_sub(*start)) * 48;
            while *phase < due {
                let t = *phase as f32 / RATE as f32;
                self.buf.push_back(((t * *freq * std::f32::consts::TAU).sin() * *amp) as i16);
                *phase += 1;
            }
        } else if let Some(fx) = self.capture_fx.as_mut() {
            let n = fx.get_frames_available();
            if n > 0 {
                let frames = fx.get_buffer(n);
                let step = AudioServer::singleton().get_mix_rate() as f64 / RATE;
                // linear resampling of the mono mix to 48 kHz
                self.pumped += frames.len() as u64;
                for f in frames.as_slice() {
                    let x = (f.x + f.y) * 0.5;
                    self.pump_peak = self.pump_peak.max(x.abs());
                    while self.resample_pos < 1.0 {
                        let s = self.last_in + (x - self.last_in) * self.resample_pos as f32;
                        self.buf.push_back((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                        self.resample_pos += step;
                    }
                    self.resample_pos -= 1.0;
                    self.last_in = x;
                }
            }
        }
        while self.buf.len() > MAX_BUFFERED {
            self.buf.pop_front();
        }
    }

    /// `S_StopCapture` when the session went away without the kernel's
    /// clean-up pass (no network host any more).
    pub fn idle(&mut self, s_volume: f32) {
        if self.capturing && !self.monitoring {
            self.stop();
            self.master_gain = 1.0;
            AudioServer::singleton().set_bus_volume_db(0, crate::vidsnd::volume_db(s_volume));
        }
    }

    fn stop(&mut self) {
        self.capturing = false;
        if let Some(m) = self.mic.as_mut() {
            m.stop();
        }
        self.buf.clear();
    }

    /// `S_AL_RawSamples` for the VoIP streams; `origin` is the talker's
    /// entity origin (Quake units) for the spatial stream.
    pub fn play(&mut self, parent: &mut Gd<Node>, p: &VoipPlayback, origin: Option<[f32; 3]>) {
        if self.direct.is_empty() {
            self.direct.resize(MAX_CLIENTS, None);
            self.spatial.resize(MAX_CLIENTS, None);
            self.received.resize(MAX_CLIENTS, 0);
            self.peak.resize(MAX_CLIENTS, 0);
            self.last_heard.resize(MAX_CLIENTS, 0);
        }
        let s = p.sender.min(MAX_CLIENTS - 1);
        self.last_heard[s] = godot::classes::Time::singleton().get_ticks_msec();
        self.received[s] += p.samples.len();
        self.peak[s] = self.peak[s].max(p.samples.iter().map(|x| x.saturating_abs()).max().unwrap_or(0));
        let gen = || {
            let mut g = AudioStreamGenerator::new_gd();
            g.set_mix_rate(RATE as f32);
            g.set_buffer_length(1.0);
            g
        };
        let frames: PackedVector2Array = p.samples.iter().map(|&x| {
            let v = x as f32 / 32768.0;
            Vector2::new(v, v)
        }).collect();
        let playback = if p.entity < 0 {
            if self.direct[s].is_none() {
                let mut n = AudioStreamPlayer::new_alloc();
                n.set_name(&format!("VoipDirect{s}"));
                n.set_stream(&gen());
                parent.add_child(&n);
                self.direct[s] = Some(n);
            }
            let n = self.direct[s].as_mut().unwrap();
            n.set_volume_db(db(p.volume));
            if !n.is_playing() {
                n.play();
            }
            n.get_stream_playback()
        } else {
            if self.spatial[s].is_none() {
                let mut n = crate::audio::new_3d();
                n.set_name(&format!("VoipSpatial{s}"));
                n.set_stream(&gen());
                parent.add_child(&n);
                self.spatial[s] = Some(n);
            }
            let n = self.spatial[s].as_mut().unwrap();
            if let Some(o) = origin {
                n.set_global_position(q2g(&o));
            }
            n.set_volume_db(db(p.volume));
            if !n.is_playing() {
                n.play();
            }
            n.get_stream_playback()
        };
        if let Some(mut pb) = playback.and_then(|pb| pb.try_cast::<AudioStreamGeneratorPlayback>().ok()) {
            let room = pb.get_frames_available().max(0) as usize;
            let f = if frames.len() > room { frames.subarray(0..room) } else { frames };
            pb.push_buffer(&f);
        }
    }

    /// Spatial streams follow their talkers.
    pub fn follow(&mut self, origin: impl Fn(usize) -> Option<[f32; 3]>) {
        for (i, n) in self.spatial.iter_mut().enumerate() {
            if let (Some(n), Some(o)) = (n.as_mut(), origin(i)) {
                n.set_global_position(q2g(&o));
            }
        }
    }
}

/// `CL_CaptureVoip`'s engine: the client cvars, this sound state, the
/// cgame's last attacker and crosshair player.
pub struct Host<'a> {
    pub cvars: &'a mut Cvars,
    pub snd: &'a mut VoipSound,
    pub parent: Gd<Node>,
    pub attacker: i32,
    pub crosshair: i32,
    /// dec056: the microphone toggle is on. Voice activation listens only
    /// then (CL_CaptureVoip keeps cl_voipSend at 1 while cl_voipUseVAD is
    /// set, so the toggle could not turn the microphone off)
    pub mic_open: bool,
}

impl VoipHost for Host<'_> {
    fn cvar_string(&self, name: &str) -> String {
        if !self.mic_open && name.eq_ignore_ascii_case("cl_voipUseVAD") {
            return "0".into();
        }
        // voice activation on the level after Mic Volume (voiphud.rs)
        if name.eq_ignore_ascii_case("cl_voipVADThreshold") {
            let t = crate::voiphud::effective_vad_threshold(self.cvars.value("cl_voipVADThreshold"), self.cvars.value("cl_voipCaptureMult"));
            return t.to_string();
        }
        self.cvars.string(name)
    }
    fn cvar_set(&mut self, name: &str, value: &str) {
        self.cvars.set(name, value);
    }
    fn cvar_take_modified(&mut self, name: &str) -> bool {
        self.cvars.take_modified(name)
    }
    fn start_capture(&mut self) {
        let mut parent = self.parent.clone();
        self.snd.ensure_device(&mut parent);
        self.snd.capturing = true;
        self.snd.monitoring = false;
        self.snd.buf.clear();
        self.snd.resample_pos = 0.0;
        if let Some(fx) = self.snd.capture_fx.as_mut() {
            fx.clear_buffer();
        }
        if let Some(t) = self.snd.test_tone.as_mut() {
            t.2 = 0;
            t.3 = godot::classes::Time::singleton().get_ticks_msec();
        } else if let Some(m) = self.snd.mic.as_mut() {
            m.play();
        }
    }
    fn available_capture_samples(&mut self) -> usize {
        self.snd.pump();
        self.snd.buf.len()
    }
    fn capture(&mut self, samples: usize) -> Vec<i16> {
        self.snd.pump();
        let n = samples.min(self.snd.buf.len());
        self.snd.buf.drain(..n).collect()
    }
    fn stop_capture(&mut self) {
        self.snd.stop();
    }
    fn master_gain(&mut self, gain: f32) {
        // the listener gain over s_volume (vidsnd's Master bus level)
        self.snd.master_gain = gain;
        let v = self.cvars.value("s_volume");
        AudioServer::singleton().set_bus_volume_db(0, crate::vidsnd::volume_db(v * gain));
    }
    fn last_attacker(&self) -> i32 {
        self.attacker
    }
    fn crosshair_player(&self) -> i32 {
        self.crosshair
    }
}
