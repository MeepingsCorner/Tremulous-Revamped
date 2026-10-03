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
pub const FMOD_CVARS: &[(&str, &str)] = &[("s_fmod", "1"), ("s_occlusion", "1"), ("s_reverb", "1")];

pub fn register_cvars(c: &mut Cvars) {
    for (n, v) in FMOD_CVARS {
        c.get(n, v, CVAR_ARCHIVE);
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
        pub update: unsafe extern "C" fn(),
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
            update: f!("ta_update"),
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
        Ok(FmodAudio { api, sounds: HashMap::new(), options: (true, true, -1.0) })
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
    pub fn play(&mut self, id: i32, pos: Option<[f32; 3]>, looped: bool, volume: f32) -> i32 {
        #[cfg(windows)]
        {
            let p = pos.unwrap_or([0.0; 3]);
            // SAFETY: the API of trem_audio.c
            unsafe { (self.api.play)(id, p.as_ptr(), pos.is_none() as i32, looped as i32, volume) }
        }
        #[cfg(not(windows))]
        {
            let _ = (id, pos, looped, volume);
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
    pub fn set_scene(&mut self, m: &AcousticMesh) -> bool {
        #[cfg(windows)]
        {
            let v: Vec<f32> = m.verts.iter().flat_map(|p| *p).collect();
            let t: Vec<i32> = m.tris.iter().flat_map(|p| *p).collect();
            // SAFETY: Steam Audio copies the mesh
            unsafe { (self.api.set_scene)(v.as_ptr(), m.verts.len() as i32, t.as_ptr(), m.tris.len() as i32, m.mats.as_ptr()) != 0 }
        }
        #[cfg(not(windows))]
        {
            let _ = m;
            false
        }
    }
    pub fn update(&mut self) {
        #[cfg(windows)]
        unsafe {
            (self.api.update)()
        }
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
    use super::*;
    #[test]
    fn materials_by_name() {
        assert_eq!(material_of("textures/atcs/floor_metal"), 1);
        assert_eq!(material_of("textures/arachnid2/rock_wall"), 2);
        assert_eq!(material_of("textures/common/caulk"), 0);
    }
}
