//! FMOD + Steam Audio sound path (port addition): the Rust side of
//! native/trem_audio.c, loaded at run time from `res://bin/trem_audio.dll`
//! (with fmod.dll, phonon.dll and phonon_fmod.dll beside it; see
//! tools/get_audio_libs.ps1).
//!
//! The game's one-shot and looping sounds play through FMOD with Steam
//! Audio's HRTF spatializer, occlusion / transmission through the map and a
//! simulated reverb of the map's geometry. `s_fmod 0` (or a missing DLL)
//! keeps Godot's audio. `s_occlusion` and `s_reverb` switch the simulation
//! parts.

use std::collections::HashMap;

use crate::cmd::{Cvars, CVAR_ARCHIVE};

/// (name, default) of the sound path's cvars (CVAR_ARCHIVE).
/// s_hdSounds: the enhanced mode plays the high-band pack (tools/sfx_hd.py,
/// user://sfx_hd) where it has a sound; vanilla always plays the originals.
/// s_pathing: sound paths around corners (Steam Audio pathing, enhanced
/// mode). Off by default: Steam Audio 4.8.1's path baker faults on these
/// maps (an access violation inside phonon.dll, caught, but not safe).
pub const FMOD_CVARS: &[(&str, &str)] = &[("s_fmod", "1"), ("s_occlusion", "-1"), ("s_reverb", "1"), ("s_hdSounds", "1"), ("s_pathing", "0")];

/// `s_occlusion`: -1 follows the graphics mode (on in enhanced, off in
/// vanilla, which has no occlusion), 0 always off, 1 always on.
pub fn occlusion_on(s_occlusion: i32, enhanced: bool) -> bool {
    match s_occlusion {
        0 => false,
        v if v < 0 => enhanced,
        _ => true,
    }
}

/// The player's own sounds, placed relative to the view in enhanced mode
/// (vanilla plays them at the player's origin, or centred).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnSound {
    /// firing, dry fire, weapon change: at the gun, ahead and to the right
    Weapon,
    /// footsteps, landing, splashes: at the feet
    Feet,
    /// taunt, pain, death, jump: centred (unspatialized)
    Voice,
    /// anything else of the player's: at the chest
    Body,
}

/// Where an own sound plays: None = centred, else a point in Godot space
/// from the view origin and basis (`fwd`, `right`, `up` unit vectors).
pub fn own_sound_pos(kind: OwnSound, eye: [f32; 3], fwd: [f32; 3], right: [f32; 3], up: [f32; 3]) -> Option<[f32; 3]> {
    // Godot units are 32 Quake units: the gun sits ~16 units ahead, 6 right
    // and 6 down of the eye; the feet 50 below it (26 eye + 24 to the floor)
    let (f, r, u) = match kind {
        OwnSound::Voice => return None,
        OwnSound::Weapon => (0.5, 0.2, -0.2),
        OwnSound::Feet => (0.0, 0.0, -1.56),
        OwnSound::Body => (0.0, 0.0, -0.8),
    };
    Some([0, 1, 2].map(|i| eye[i] + fwd[i] * f + right[i] * r + up[i] * u))
}

pub fn register_cvars(c: &mut Cvars) {
    for (n, v) in FMOD_CVARS {
        c.get(n, v, CVAR_ARCHIVE);
    }
    // One-time migration (s_soundCfgVersion 1): s_occlusion became -1 =
    // auto (follow r_enhanced). Values archived before it were left by the
    // sound tests (s_occlusion 0 / 1), not chosen by the player.
    c.get("s_soundCfgVersion", "0", CVAR_ARCHIVE);
    if c.integer("s_soundCfgVersion") < 1 {
        c.set("s_occlusion", "-1");
        c.set("s_soundCfgVersion", "1");
    }
}

/// The map's triangles for the acoustic simulation (Godot space) and a
/// material per triangle (0 generic, 1 metal, 2 rock / organic).
#[derive(Default, Clone)]
pub struct AcousticMesh {
    pub verts: Vec<[f32; 3]>,
    pub tris: Vec<[i32; 3]>,
    pub mats: Vec<i32>,
}

impl AcousticMesh {
    /// FNV-1a over the geometry: names a map's pathing cache, and changes
    /// whenever the map (or the acoustic mesh builder) does.
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut eat = |b: &[u8]| {
            for &x in b {
                h = (h ^ x as u64).wrapping_mul(0x100000001b3);
            }
        };
        for v in &self.verts {
            for c in v {
                eat(&c.to_le_bytes());
            }
        }
        for t in &self.tris {
            for i in t {
                eat(&i.to_le_bytes());
            }
        }
        h
    }
}

/// The material of a texture name (trem_audio.c's table).
pub fn material_of(shader: &str) -> i32 {
    let n = shader.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|s| n.contains(s));
    if has(&["metal", "floor", "grate", "plate", "panel", "pipe", "steel", "door", "tech", "human"]) {
        1
    } else if has(&["rock", "stone", "dirt", "organic", "alien", "creep", "flesh", "cliff", "ground", "terrain"]) {
        2
    } else {
        0
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::{c_char, c_void, CStr};

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;

    pub struct Api {
        pub init: unsafe extern "C" fn(*const u16, i32) -> i32,
        pub shutdown: unsafe extern "C" fn(),
        pub error: unsafe extern "C" fn() -> *const c_char,
        pub set_scene: unsafe extern "C" fn(*const f32, i32, *const i32, i32, *const i32) -> i32,
        pub sound_load: unsafe extern "C" fn(*const u8, i32) -> i32,
        pub play: unsafe extern "C" fn(i32, *const f32, i32, i32, f32) -> i32,
        pub voice_set_pos: unsafe extern "C" fn(i32, *const f32),
        pub voice_stop: unsafe extern "C" fn(i32),
        pub voice_playing: unsafe extern "C" fn(i32) -> i32,
        pub listener: unsafe extern "C" fn(*const f32, *const f32, *const f32),
        pub options: unsafe extern "C" fn(i32, i32, f32),
        pub enhanced: unsafe extern "C" fn(i32),
        pub pathing_cache: unsafe extern "C" fn(*const c_char),
        pub update: unsafe extern "C" fn(),
        pub stats: unsafe extern "C" fn(*mut c_char, i32),
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn load(dir: &str) -> Result<Api, String> {
        let path = format!("{}\\trem_audio.dll", dir.trim_end_matches(['\\', '/']).replace('/', "\\"));
        let w = wide(&path);
        // SAFETY: plain Win32 calls with NUL-terminated strings
        let m = unsafe { LoadLibraryExW(w.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
        if m.is_null() {
            return Err(format!("{path} could not be loaded (missing, or fmod.dll / phonon.dll missing beside it)"));
        }
        macro_rules! f {
            ($n:literal) => {{
                let p = unsafe { GetProcAddress(m, concat!($n, "\0").as_ptr() as *const c_char) };
                if p.is_null() {
                    return Err(format!("trem_audio.dll lacks {}", $n));
                }
                // SAFETY: the export has the signature of trem_audio.c
                unsafe { std::mem::transmute::<*mut c_void, _>(p) }
            }};
        }
        Ok(Api {
            init: f!("ta_init"),
            shutdown: f!("ta_shutdown"),
            error: f!("ta_error"),
            set_scene: f!("ta_set_scene"),
            sound_load: f!("ta_sound_load"),
            play: f!("ta_play"),
            voice_set_pos: f!("ta_voice_set_pos"),
            voice_stop: f!("ta_voice_stop"),
            voice_playing: f!("ta_voice_playing"),
            listener: f!("ta_listener"),
            options: f!("ta_options"),
            enhanced: f!("ta_enhanced"),
            pathing_cache: f!("ta_pathing_cache"),
            update: f!("ta_update"),
            stats: f!("ta_stats"),
        })
    }

    pub fn error(api: &Api) -> String {
        // SAFETY: ta_error returns a static NUL-terminated buffer
        unsafe { CStr::from_ptr((api.error)()).to_string_lossy().into_owned() }
    }

    pub fn wide_dir(dir: &str) -> Vec<u16> {
        wide(&dir.trim_end_matches(['\\', '/']).replace('/', "\\"))
    }
}

pub struct FmodAudio {
    #[cfg(windows)]
    api: sys::Api,
    sounds: HashMap<String, i32>,
    options: (bool, bool, f32),
    enhanced: Option<bool>,
}

impl FmodAudio {
    /// Load trem_audio.dll from `dir` and start FMOD + Steam Audio.
    #[cfg(windows)]
    pub fn open(dir: &str, sample_rate: i32) -> Result<Self, String> {
        let api = sys::load(dir)?;
        let w = sys::wide_dir(dir);
        // SAFETY: the API of trem_audio.c
        if unsafe { (api.init)(w.as_ptr(), sample_rate) } == 0 {
            return Err(sys::error(&api));
        }
        Ok(FmodAudio { api, sounds: HashMap::new(), options: (true, true, -1.0), enhanced: None })
    }
    #[cfg(not(windows))]
    pub fn open(_dir: &str, _sample_rate: i32) -> Result<Self, String> {
        Err("the FMOD sound path is built for Windows only".into())
    }

    /// The sound of `key` (a pk3 path), loaded from `bytes` the first time.
    pub fn sound(&mut self, key: &str, bytes: impl FnOnce() -> Option<Vec<u8>>) -> Option<i32> {
        if let Some(&id) = self.sounds.get(key) {
            return (id >= 0).then_some(id);
        }
        #[allow(unused_mut)]
        let mut id = -1;
        #[cfg(windows)]
        if let Some(b) = bytes() {
            // SAFETY: FMOD copies the data (FMOD_CREATESAMPLE)
            id = unsafe { (self.api.sound_load)(b.as_ptr(), b.len() as i32) };
        }
        #[cfg(not(windows))]
        let _ = bytes;
        self.sounds.insert(key.to_string(), id);
        (id >= 0).then_some(id)
    }

    /// Play a sound at `pos` (Godot space), or as the listener's own sound.
    /// Returns a voice handle.
    /// `pos` None plays it unspatialized (vanilla's local sounds); `own`
    /// marks the player's own positioned sounds, which are never occluded.
    /// `big`: a loud sound (fire, impact, explosion), which gets its own
    /// reflections in enhanced mode.
    pub fn play(&mut self, id: i32, pos: Option<[f32; 3]>, own: bool, big: bool, looped: bool, volume: f32) -> i32 {
        #[cfg(windows)]
        {
            let p = pos.unwrap_or([0.0; 3]);
            let kind = (if pos.is_none() { 1 } else if own { 2 } else { 0 }) | if big { 4 } else { 0 };
            // SAFETY: the API of trem_audio.c
            unsafe { (self.api.play)(id, p.as_ptr(), kind, looped as i32, volume) }
        }
        #[cfg(not(windows))]
        {
            let _ = (id, pos, own, big, looped, volume);
            -1
        }
    }
    #[allow(dead_code)] // the looping sounds still play through Godot
    pub fn set_pos(&mut self, voice: i32, pos: [f32; 3]) {
        #[cfg(windows)]
        unsafe {
            (self.api.voice_set_pos)(voice, pos.as_ptr())
        }
        #[cfg(not(windows))]
        let _ = (voice, pos);
    }
    #[allow(dead_code)]
    pub fn stop(&mut self, voice: i32) {
        #[cfg(windows)]
        unsafe {
            (self.api.voice_stop)(voice)
        }
        #[cfg(not(windows))]
        let _ = voice;
    }
    #[allow(dead_code)]
    pub fn playing(&self, voice: i32) -> bool {
        #[cfg(windows)]
        unsafe {
            (self.api.voice_playing)(voice) != 0
        }
        #[cfg(not(windows))]
        {
            let _ = voice;
            false
        }
    }
    /// The listener: position, forward, up (Godot space).
    pub fn listener(&mut self, pos: [f32; 3], fwd: [f32; 3], up: [f32; 3]) {
        #[cfg(windows)]
        unsafe {
            (self.api.listener)(pos.as_ptr(), fwd.as_ptr(), up.as_ptr())
        }
        #[cfg(not(windows))]
        let _ = (pos, fwd, up);
    }
    /// Enhanced-mode audio (limiter, doppler, reflections, pathing).
    pub fn set_enhanced(&mut self, on: bool) {
        if self.enhanced == Some(on) {
            return;
        }
        self.enhanced = Some(on);
        #[cfg(windows)]
        unsafe {
            (self.api.enhanced)(on as i32)
        }
    }
    pub fn set_options(&mut self, occlusion: bool, reverb: bool, master: f32) {
        if (occlusion, reverb, master) == self.options {
            return;
        }
        self.options = (occlusion, reverb, master);
        #[cfg(windows)]
        unsafe {
            (self.api.options)(occlusion as i32, reverb as i32, master)
        }
    }
    /// `cache_dir`: where the map's baked pathing probes are kept
    /// (`<mesh hash>.probes`), so each map is baked once.
    /// `cache_dir` None: no pathing for this scene.
    pub fn set_scene(&mut self, m: &AcousticMesh, cache_dir: Option<&str>) -> bool {
        #[cfg(windows)]
        {
            let file = match cache_dir {
                Some(d) => {
                    let _ = std::fs::create_dir_all(d);
                    format!("{}/{:016x}.probes\0", d.trim_end_matches(['/', '\\']), m.hash())
                }
                None => "\0".to_string(),
            };
            // SAFETY: a NUL-terminated UTF-8 path; trem_audio.c copies it
            unsafe { (self.api.pathing_cache)(file.as_ptr() as *const std::ffi::c_char) };
            let v: Vec<f32> = m.verts.iter().flat_map(|p| *p).collect();
            let t: Vec<i32> = m.tris.iter().flat_map(|p| *p).collect();
            // SAFETY: Steam Audio copies the mesh
            unsafe { (self.api.set_scene)(v.as_ptr(), m.verts.len() as i32, t.as_ptr(), m.tris.len() as i32, m.mats.as_ptr()) != 0 }
        }
        #[cfg(not(windows))]
        {
            let _ = (m, cache_dir);
            false
        }
    }
    pub fn update(&mut self) {
        #[cfg(windows)]
        unsafe {
            (self.api.update)()
        }
    }
    /// Counters, output device and last error (`s_fmodinfo`).
    pub fn stats(&self) -> String {
        #[cfg(windows)]
        {
            let mut buf = vec![0 as std::ffi::c_char; 2048];
            // SAFETY: ta_stats writes a NUL-terminated string into the buffer
            unsafe {
                (self.api.stats)(buf.as_mut_ptr(), buf.len() as i32);
                std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
            }
        }
        #[cfg(not(windows))]
        String::new()
    }

    pub fn error(&self) -> String {
        #[cfg(windows)]
        {
            sys::error(&self.api)
        }
        #[cfg(not(windows))]
        String::new()
    }
}

impl Drop for FmodAudio {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            (self.api.shutdown)()
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn archived_test_occlusion_is_migrated_once() {
        let mut c = crate::cmd::Cvars::default();
        c.set("s_occlusion", "1"); // as an old autogen.cfg leaves it
        super::register_cvars(&mut c);
        assert_eq!(c.integer("s_occlusion"), -1);
        c.set("s_occlusion", "0"); // the player's own choice afterwards
        super::register_cvars(&mut c);
        assert_eq!(c.integer("s_occlusion"), 0);
    }

    #[test]
    fn own_sounds_sit_at_gun_feet_or_centre() {
        use super::{own_sound_pos, OwnSound};
        let (e, f, r, u) = ([0.0, 2.0, 0.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(own_sound_pos(OwnSound::Voice, e, f, r, u), None);
        let g = own_sound_pos(OwnSound::Weapon, e, f, r, u).unwrap();
        assert!(g[0] > 0.0 && g[1] < 2.0 && g[2] < 0.0, "gun ahead, right, below: {g:?}");
        let ft = own_sound_pos(OwnSound::Feet, e, f, r, u).unwrap();
        assert!((ft[1] - 0.44).abs() < 1e-4);
    }

    #[test]
    fn mesh_hash_names_the_geometry() {
        let a = super::AcousticMesh { verts: vec![[0.0, 1.0, 2.0]], tris: vec![[0, 0, 0]], mats: vec![0] };
        let mut b = super::AcousticMesh { verts: vec![[0.0, 1.0, 2.0]], tris: vec![[0, 0, 0]], mats: vec![0] };
        assert_eq!(a.hash(), b.hash());
        b.verts[0][1] = 1.5;
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn occlusion_follows_enhanced_on_auto() {
        assert!(super::occlusion_on(-1, true));
        assert!(!super::occlusion_on(-1, false));
        assert!(!super::occlusion_on(0, true));
        assert!(super::occlusion_on(1, false));
    }

    use super::*;
    #[test]
    fn materials_by_name() {
        assert_eq!(material_of("textures/atcs/floor_metal"), 1);
        assert_eq!(material_of("textures/arachnid2/rock_wall"), 2);
        assert_eq!(material_of("textures/common/caulk"), 0);
    }
}
