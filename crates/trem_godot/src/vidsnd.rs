//! The engine cvars the option menus edit, with their effect in Godot: the
//! renderer's (tr_init.c `R_Register`, sdl_glimp.c `GLimp_SetMode`,
//! `GLimp_EndFrame`, `R_SetColorMappings`, `GL_TextureMode`), the sound
//! system's (snd_main.c `S_Init`, `S_UpdateBackgroundTrack`) and
//! common.c's `com_maxfps`, plus the archived ui / client / server cvars of
//! the menus. They live in the client's cvar store (`Cvar_Get` with the
//! original defaults and flags), so `autogen.cfg` keeps them.
//!
//! As in the engine some apply at once (r_gamma, r_fullscreen, r_texturemode,
//! s_volume, s_musicvolume, com_maxfps) and the latched ones on `vid_restart`
//! (r_width / r_height / r_noborder / r_picmip / r_ext_* / r_swapInterval);
//! `snd_restart` re-applies the sound cvars and does a `vid_restart`.
//!
//! Godot mapping: r_fullscreen / r_noborder / r_width / r_height ->
//! DisplayServer window mode, borderless flag and size (0 x 0 = the desktop);
//! r_gamma -> a last full-screen pass `pow( c, 1 / r_gamma )` (the gamma
//! table of R_SetColorMappings); s_volume -> the Master bus, s_musicvolume ->
//! a "Music" bus for the background track; com_maxfps -> Engine.max_fps;
//! r_picmip -> the root viewport's texture mipmap bias (one level per picmip
//! step); r_texturemode -> the canvas texture filter (NEAREST* / LINEAR /
//! *_MIPMAP_*); r_ext_texture_filter_anisotropic + r_ext_max_anisotropy ->
//! viewport anisotropic filtering; r_ext_multisample -> 3D MSAA;
//! r_swapInterval -> vsync. The rest (r_colorbits, r_texturebits,
//! r_subdivisions, r_lodbias, r_vertexLight, r_fastsky, r_finish, s_khz,
//! s_useOpenAL, s_doppler, ...) are stored for the menus only: the port's
//! renderer and mixer have no counterpart for them.
use godot::classes::display_server::{VSyncMode, WindowFlags, WindowMode};
use godot::classes::window::{ContentScaleAspect, ContentScaleMode};
use godot::classes::viewport::{AnisotropicFiltering, DefaultCanvasItemTextureFilter, Msaa};
use godot::classes::{AudioServer, CanvasLayer, ColorRect, Control, DisplayServer, Engine, Node, Shader, ShaderMaterial, Viewport};
use godot::prelude::*;

use crate::cmd::{Cvars, CVAR_ARCHIVE, CVAR_ROM, CVAR_USERINFO};

/// tr_init.c `R_Register` / sdl_glimp.c (name, default, flags). The
/// default of r_fullscreen is the window's state at startup (the engine's
/// "1" would force a first start full screen over the project's window).
pub const VIDEO_CVARS: &[(&str, &str)] = &[
    ("r_mode", "-2"),
    ("r_fullscreen", "1"),
    ("r_noborder", "0"),
    ("r_width", "0"),
    ("r_height", "0"),
    ("r_gamma", "1"),
    ("r_picmip", "1"),
    ("r_texturemode", "GL_LINEAR_MIPMAP_NEAREST"),
    ("r_ext_texture_filter_anisotropic", "0"),
    ("r_ext_max_anisotropy", "2"),
    ("r_ext_multisample", "0"),
    ("r_ext_compressed_textures", "0"),
    ("r_allowExtensions", "1"),
    ("r_swapInterval", "0"),
    ("r_texturebits", "0"),
    ("r_colorbits", "0"),
    ("r_depthbits", "0"),
    ("r_stencilbits", "8"),
    ("r_subdivisions", "4"),
    ("r_lodbias", "0"),
    ("r_vertexLight", "0"),
    ("r_dynamiclight", "1"),
    ("r_fastsky", "0"),
    ("r_finish", "0"),
    ("r_inGameVideo", "1"),
];
/// snd_main.c `S_Init`
pub const SOUND_CVARS: &[(&str, &str)] = &[("s_volume", "0.8"), ("s_musicvolume", "0.25"), ("s_doppler", "1"), ("s_khz", "22"), ("s_useOpenAL", "0")];
/// common.c / cl_main.c (CVAR_ARCHIVE)
/// (`cl_netProtocol`: the network mode of dec040, 0 Auto, 71, 69)
pub const CLIENT_CVARS: &[(&str, &str)] = &[("com_maxfps", "85"), ("cl_allowDownload", "0"), ("cg_shadows", "1"), ("cg_bounceParticles", "0"), ("cl_netProtocol", "0")];
/// ui_main.c `cvarTable` (the CVAR_ARCHIVE ones)
pub const UI_CVARS: &[(&str, &str)] = &[
    ("ui_browserShowFull", "1"),
    // the join server menu's protocol filter (dec040): 0 all, 71, 69
    ("ui_browserProtocol", "0"),
    ("ui_browserShowEmpty", "1"),
    ("ui_dedicated", "0"),
    ("ui_netSource", "0"),
    ("ui_selectedMap", "0"),
    ("ui_lastServerRefresh_0", ""),
    ("ui_lastServerRefresh_1", ""),
    ("ui_lastServerRefresh_2", ""),
    ("ui_lastServerRefresh_3", ""),
    ("ui_lastServerRefresh_0_time", ""),
    ("ui_lastServerRefresh_1_time", ""),
    ("ui_lastServerRefresh_2_time", ""),
    ("ui_lastServerRefresh_3_time", ""),
    ("ui_smallFont", "0.2"),
    ("ui_bigFont", "0.5"),
    ("ui_findPlayer", ""),
    ("ui_serverStatusTimeOut", "7000"),
    ("ui_textWrapCache", "1"),
    ("cg_emoticons", "1"),
    ("ui_chatCommands", "1"),
];
/// the server cvars of createserver.menu (sv_init.c / g_main.c; the
/// archived ones persist)
pub const SERVER_CVARS: &[(&str, &str, i32)] = &[
    ("sv_hostname", "noname", CVAR_ARCHIVE),
    ("timelimit", "0", CVAR_ARCHIVE),
    ("sv_maxclients", "8", 0),
    ("g_needpassword", "0", 0),
    ("g_password", "", 0),
    ("sv_pure", "1", 0),
    ("sv_allowdownload", "0", 0),
    ("g_allowVote", "1", CVAR_ARCHIVE),
    ("sv_minping", "0", CVAR_ARCHIVE),
    ("sv_maxping", "0", CVAR_ARCHIVE),
    ("g_synchronousClients", "0", 0),
    ("sv_maxRate", "0", CVAR_ARCHIVE),
    ("sv_zombietime", "2", 0),
    ("sv_reconnectlimit", "3", 0),
    ("sv_master1", "master.tremulous.net", 0),
];

/// `Cvar_Get` of every cvar above (`windowed`: the startup window state);
/// the ui's share is `UI_RegisterCvars`.
pub fn register(c: &mut Cvars, windowed: bool) {
    for (n, v) in VIDEO_CVARS.iter().chain(SOUND_CVARS).chain(CLIENT_CVARS).chain(UI_CVARS) {
        let v = if *n == "r_fullscreen" && windowed { "0" } else { v };
        c.get(n, v, CVAR_ARCHIVE);
    }
    for (n, v, f) in SERVER_CVARS {
        c.get(n, v, *f);
    }
    c.get("name", "UnnamedPlayer", CVAR_USERINFO | CVAR_ARCHIVE);
    c.get("rate", "90000", CVAR_USERINFO | CVAR_ARCHIVE);
    c.get("r_availableModes", "", CVAR_ROM);
    c.get("cl_motdString", "", CVAR_ROM);
}

/// `GLimp_SetMode`: r_width x r_height when both are set, else the
/// desktop, else 640x480.
#[cfg_attr(not(test), allow(dead_code))]
pub fn resolution(r_width: i32, r_height: i32, desktop: (i32, i32)) -> (i32, i32) {
    if r_width > 0 && r_height > 0 {
        (r_width, r_height)
    } else if desktop.1 > 0 {
        desktop
    } else {
        (640, 480)
    }
}

/// The modes the Video Mode list offers, smallest first: the monitor sizes
/// at 1% or more in the Steam Hardware Survey (September 2026, primary
/// display resolution) plus the lower render sizes 800x600, 1024x768,
/// 1280x720 and 1600x900.
pub const COMMON_MODES: &[(i32, i32)] = &[
    (800, 600), (1024, 768), (1280, 720), (1366, 768), (1600, 900), (1920, 1080), (1920, 1200),
    (2560, 1440), (2560, 1600), (3440, 1440), (3840, 2160),
];

/// `GLimp_DetectAvailableModes` (`r_availableModes`): Godot lists no
/// display modes, so the common modes that fit on the desktop plus the
/// desktop's own size, smallest first (by area, then width). Any other size
/// still works through `r_width` / `r_height` (the menu shows it as its
/// "Custom" entry).
pub fn available_modes(desktop: (i32, i32)) -> String {
    let known = desktop.0 > 0 && desktop.1 > 0;
    let mut m: Vec<(i32, i32)> = COMMON_MODES.iter().copied().filter(|&(w, h)| !known || (w <= desktop.0 && h <= desktop.1)).collect();
    if known && !m.contains(&desktop) {
        m.push(desktop);
    }
    m.sort_by_key(|&(w, h)| (w * h, w));
    m.iter().map(|(w, h)| format!("{w}x{h}")).collect::<Vec<_>>().join(" ")
}

/// The desktop size (`DisplayServer.screen_get_size`).
pub fn desktop_size() -> (i32, i32) {
    let d = DisplayServer::singleton().screen_get_size();
    (d.x, d.y)
}

/// The window size for a mode in windowed mode: the mode, shrunk (keeping
/// its aspect) to fit the screen when it is larger.
pub fn windowed_size(mode: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    if screen.0 <= 0 || screen.1 <= 0 || (mode.0 <= screen.0 && mode.1 <= screen.1) {
        return mode;
    }
    let s = (screen.0 as f32 / mode.0 as f32).min(screen.1 as f32 / mode.1 as f32);
    (((mode.0 as f32 * s) as i32).max(320), ((mode.1 as f32 * s) as i32).max(240))
}

/// `GL_TextureMode`: (nearest, mipmapped) of a GL filter name; None for an
/// unknown one ("bad filter name").
pub fn texture_mode(s: &str) -> Option<(bool, bool)> {
    Some(match s.to_ascii_uppercase().as_str() {
        "GL_NEAREST" => (true, false),
        "GL_LINEAR" => (false, false),
        "GL_NEAREST_MIPMAP_NEAREST" | "GL_NEAREST_MIPMAP_LINEAR" => (true, true),
        "GL_LINEAR_MIPMAP_NEAREST" | "GL_LINEAR_MIPMAP_LINEAR" => (false, true),
        _ => return None,
    })
}

/// r_ext_texture_filter_anisotropic / r_ext_max_anisotropy -> 0, 2, 4, 8, 16.
pub fn anisotropy(enabled: bool, max: f32) -> i32 {
    if !enabled {
        return 0;
    }
    [16, 8, 4, 2].into_iter().find(|&l| max >= l as f32).unwrap_or(2)
}

/// r_ext_multisample (0..4, Cvar_CheckRange) -> MSAA samples.
pub fn multisample(n: i32) -> i32 {
    match n.clamp(0, 4) {
        0 | 1 => 0,
        2 | 3 => 2,
        _ => 4,
    }
}

/// Linear volume -> bus decibels (silence below -80 dB).
pub fn volume_db(v: f32) -> f32 {
    if v <= 0.0001 {
        -80.0
    } else {
        (20.0 * v.log10()).max(-80.0)
    }
}

/// The Music bus sends into Master (scaled by s_volume); the background
/// track's own level is s_musicvolume (S_UpdateBackgroundTrack), so the
/// bus carries their ratio.
pub fn music_bus_db(s_volume: f32, s_musicvolume: f32) -> f32 {
    if s_volume <= 0.0001 {
        return -80.0;
    }
    volume_db(s_musicvolume / s_volume).min(24.0)
}

/// `Com_Frame`'s com_maxfps: 0 = no limit.
pub fn max_fps(v: i32) -> i32 {
    v.max(0)
}

/// R_SetColorMappings' exponent: 1 / r_gamma (r_gamma clamped 0.5 .. 3).
pub fn gamma_exponent(r_gamma: f32) -> f32 {
    1.0 / r_gamma.clamp(0.5, 3.0)
}

const GAMMA_SHADER: &str = r#"
shader_type canvas_item;
uniform sampler2D screen_tex : hint_screen_texture, filter_nearest;
uniform float inv_gamma = 1.0;
void fragment() {
    vec4 c = textureLod(screen_tex, SCREEN_UV, 0.0);
    COLOR = vec4(pow(clamp(c.rgb, 0.0, 1.0), vec3(inv_gamma)), 1.0);
}
"#;

/// The engine side of these cvars in Godot.
#[derive(Default)]
pub struct VidSnd {
    gamma: Option<(Gd<CanvasLayer>, Gd<ShaderMaterial>)>,
    /// the window cvars are applied (false when the game runs inside a
    /// test scene: the tests keep their window)
    pub window_cvars: bool,
    started: bool,
}

/// The cvars applied at once when they change.
const IMMEDIATE: &[&str] = &["r_gamma", "r_fullscreen", "r_texturemode", "s_volume", "s_musicvolume", "com_maxfps", "s_doppler"];

impl VidSnd {
    /// The renderer / sound start-up has run.
    pub fn started(&self) -> bool {
        self.started
    }

    /// Once a frame: the first call is the renderer / sound start-up (every
    /// cvar), later ones apply the immediate cvars that changed.
    pub fn frame(&mut self, c: &mut Cvars, parent: &mut Gd<Node>) {
        if !self.started {
            self.started = true;
            for n in IMMEDIATE {
                c.take_modified(n);
            }
            self.vid_restart(c, parent);
            self.snd_restart(c);
            return;
        }
        let mut changed: Vec<&str> = Vec::new();
        for n in IMMEDIATE {
            if c.take_modified(n) {
                changed.push(n);
            }
        }
        for n in changed {
            match n {
                "r_gamma" => self.apply_gamma(c, parent),
                "r_fullscreen" => self.apply_window(c, false, parent),
                "r_texturemode" => Self::apply_texture_mode(c, parent),
                "com_maxfps" => Engine::singleton().set_max_fps(max_fps(c.integer("com_maxfps"))),
                _ => self.snd_restart(c),
            }
        }
    }

    /// `vid_restart`: the latched renderer cvars, the window and gamma.
    pub fn vid_restart(&mut self, c: &mut Cvars, parent: &mut Gd<Node>) {
        c.set("r_availableModes", &available_modes(desktop_size()));
        self.apply_window(c, true, parent);
        self.apply_gamma(c, parent);
        Self::apply_texture_mode(c, parent);
        if let Some(mut vp) = parent.get_viewport() {
            // r_picmip: drop that many mip levels
            vp.set_texture_mipmap_bias(c.integer("r_picmip").clamp(0, 16) as f32);
            let a = match anisotropy(c.integer("r_ext_texture_filter_anisotropic") != 0, c.value("r_ext_max_anisotropy")) {
                16 => AnisotropicFiltering::ANISOTROPY_16X,
                8 => AnisotropicFiltering::ANISOTROPY_8X,
                4 => AnisotropicFiltering::ANISOTROPY_4X,
                2 => AnisotropicFiltering::ANISOTROPY_2X,
                _ => AnisotropicFiltering::DISABLED,
            };
            vp.set_anisotropic_filtering_level(a);
            vp.set_msaa_3d(match multisample(c.integer("r_ext_multisample")) {
                4 => Msaa::MSAA_4X,
                2 => Msaa::MSAA_2X,
                _ => Msaa::DISABLED,
            });
        }
        if self.window_cvars {
            DisplayServer::singleton().window_set_vsync_mode(if c.integer("r_swapInterval") != 0 { VSyncMode::ENABLED } else { VSyncMode::DISABLED });
        }
        Engine::singleton().set_max_fps(max_fps(c.integer("com_maxfps")));
    }

    /// `snd_restart` / the s_volume / s_musicvolume changes.
    pub fn snd_restart(&mut self, c: &mut Cvars) {
        let mut a = AudioServer::singleton();
        let (sv, mv) = (c.value("s_volume"), c.value("s_musicvolume"));
        a.set_bus_volume_db(0, volume_db(sv));
        let mut bus = a.get_bus_index("Music");
        if bus < 0 {
            bus = a.get_bus_count();
            a.add_bus();
            a.set_bus_name(bus, "Music");
            a.set_bus_send(bus, "Master");
        }
        a.set_bus_volume_db(bus, music_bus_db(sv, mv));
    }

    /// `GLimp_SetMode` / `GLimp_EndFrame`'s r_fullscreen toggle.
    fn apply_window(&mut self, c: &mut Cvars, resize: bool, parent: &Gd<Node>) {
        if !self.window_cvars {
            return;
        }
        let mut ds = DisplayServer::singleton();
        if Engine::singleton().is_embedded_in_editor() {
            godot_warn!("trem: the game runs embedded in the editor's Game tab, which cannot go full screen or change size; turn off \"Embed Game\" in the Game tab (or run the exported game) for r_fullscreen / r_width / r_height");
        }
        let full = c.integer("r_fullscreen") != 0;
        let screen = ds.screen_get_size();
        let (rw, rh) = (c.integer("r_width"), c.integer("r_height"));
        let custom = rw > 0 && rh > 0;
        let mode = ds.window_get_mode();
        let is_full = mode == WindowMode::FULLSCREEN || mode == WindowMode::EXCLUSIVE_FULLSCREEN;
        if full != is_full {
            // EXCLUSIVE_FULLSCREEN: the real full-screen mode (lower latency,
            // no compositor), at the desktop resolution; the chosen mode is
            // the render size below
            ds.window_set_mode(if full { WindowMode::EXCLUSIVE_FULLSCREEN } else { WindowMode::WINDOWED });
        }
        if !full && (resize || full != is_full) {
            ds.window_set_flag(WindowFlags::BORDERLESS, c.integer("r_noborder") != 0);
            if custom {
                let (w, h) = windowed_size((rw, rh), (screen.x, screen.y));
                ds.window_set_size(Vector2i::new(w, h));
                // r_centerWindow
                let p = ds.screen_get_position();
                ds.window_set_position(Vector2i::new(p.x + (screen.x - w).max(0) / 2, p.y + (screen.y - h).max(0) / 2));
            }
        }
        // the mode is the render resolution: the root viewport renders at
        // r_width x r_height and is scaled (aspect kept) to the window or
        // the full screen, so every mode works whatever the monitor; 0 x 0
        // renders at the window's own size
        {
            let mut root = parent.get_tree().get_root();
            if custom {
                root.set_content_scale_mode(ContentScaleMode::VIEWPORT);
                root.set_content_scale_aspect(ContentScaleAspect::KEEP);
                root.set_content_scale_size(Vector2i::new(rw, rh));
            } else {
                root.set_content_scale_mode(ContentScaleMode::DISABLED);
            }
        }
        let got = ds.window_get_mode();
        let now_full = got == WindowMode::FULLSCREEN || got == WindowMode::EXCLUSIVE_FULLSCREEN;
        godot_print!("trem: video mode {} {}{}", if custom { format!("{rw}x{rh}") } else { "desktop".into() }, if now_full { "full screen" } else { "windowed" }, if now_full != full { " (the window refused the mode change)" } else { "" });
    }

    /// `R_SetColorMappings`: the gamma pass above everything.
    fn apply_gamma(&mut self, c: &mut Cvars, parent: &mut Gd<Node>) {
        let g = c.value("r_gamma");
        let on = (g - 1.0).abs() > 0.001 && g > 0.0;
        if on && self.gamma.is_none() {
            let mut layer = CanvasLayer::new_alloc();
            layer.set_layer(127);
            layer.set_name("GammaPass");
            let mut rect = ColorRect::new_alloc();
            rect.set_anchors_preset(godot::classes::control::LayoutPreset::FULL_RECT);
            rect.set_mouse_filter(godot::classes::control::MouseFilter::IGNORE);
            let mut sh = Shader::new_gd();
            sh.set_code(GAMMA_SHADER);
            let mut m = ShaderMaterial::new_gd();
            m.set_shader(&sh);
            rect.set_material(&m);
            layer.add_child(&rect.upcast::<Control>());
            parent.add_child(&layer);
            self.gamma = Some((layer, m));
        }
        if let Some((layer, m)) = self.gamma.as_mut() {
            layer.set_visible(on);
            m.set_shader_parameter("inv_gamma", &gamma_exponent(g).to_variant());
        }
    }

    /// `GL_TextureMode` (the 2D canvas filter; the 3D materials keep theirs).
    fn apply_texture_mode(c: &mut Cvars, parent: &mut Gd<Node>) {
        let Some((nearest, mip)) = texture_mode(&c.string("r_texturemode")) else { return };
        let Some(mut vp): Option<Gd<Viewport>> = parent.get_viewport() else { return };
        vp.set_default_canvas_item_texture_filter(match (nearest, mip) {
            (true, false) => DefaultCanvasItemTextureFilter::NEAREST,
            (true, true) => DefaultCanvasItemTextureFilter::NEAREST_WITH_MIPMAPS,
            (false, false) => DefaultCanvasItemTextureFilter::LINEAR,
            (false, true) => DefaultCanvasItemTextureFilter::LINEAR_WITH_MIPMAPS,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vidsnd_available_modes_fit_the_desktop() {
        // 1440p: nothing larger than the monitor, the desktop is a common mode (listed once)
        assert_eq!(available_modes((2560, 1440)), "800x600 1024x768 1280x720 1366x768 1600x900 1920x1080 1920x1200 2560x1440");
        // 4K: the whole list
        let m = available_modes((3840, 2160));
        assert_eq!(m.split(' ').count(), COMMON_MODES.len(), "{m}");
        assert!(m.starts_with("800x600 ") && m.ends_with(" 3440x1440 3840x2160"), "{m}");
        // an odd desktop is added once, in size order
        assert_eq!(available_modes((1680, 1050)), "800x600 1024x768 1280x720 1366x768 1600x900 1680x1050");
        let m = available_modes((5120, 1440));
        assert!(m.ends_with(" 2560x1440 3440x1440 5120x1440") && !m.contains("2560x1600") && !m.contains("3840x2160"), "{m}");
        // 1366x768 desktop: 1600x900 and up hidden
        assert_eq!(available_modes((1366, 768)), "800x600 1024x768 1280x720 1366x768");
        // no desktop size known: the common modes
        assert_eq!(available_modes((0, 0)).split(' ').count(), COMMON_MODES.len());
        // every listed mode appears once
        for d in [(1920, 1080), (2560, 1440), (3840, 2160), (1680, 1050), (1280, 800)] {
            let m = available_modes(d);
            let v: Vec<&str> = m.split(' ').collect();
            let mut u = v.clone();
            u.dedup();
            assert_eq!(v, u, "{d:?}");
            assert_eq!(v.iter().filter(|x| **x == format!("{}x{}", d.0, d.1)).count(), 1, "{d:?}");
        }
    }

    #[test]
    fn vidsnd_option_mapping() {
        assert_eq!(resolution(1024, 768, (1920, 1080)), (1024, 768));
        assert_eq!(resolution(0, 0, (1920, 1080)), (1920, 1080));
        assert_eq!(resolution(0, 0, (0, 0)), (640, 480));
        assert_eq!(available_modes((1920, 1080)), "800x600 1024x768 1280x720 1366x768 1600x900 1920x1080");
        assert_eq!(windowed_size((1280, 720), (1920, 1080)), (1280, 720));
        assert_eq!(windowed_size((3840, 2160), (1920, 1080)), (1920, 1080));
        assert_eq!(texture_mode("GL_LINEAR_MIPMAP_NEAREST"), Some((false, true)));
        assert_eq!(texture_mode("gl_nearest"), Some((true, false)));
        assert_eq!(texture_mode("bogus"), None);
        assert_eq!(anisotropy(false, 16.0), 0);
        assert_eq!(anisotropy(true, 2.0), 2);
        assert_eq!(anisotropy(true, 12.0), 8);
        assert_eq!(multisample(4), 4);
        assert_eq!(multisample(9), 4);
        assert_eq!(volume_db(1.0), 0.0);
        assert!((volume_db(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(volume_db(0.0), -80.0);
        // music level = s_musicvolume whatever s_volume is
        assert!((music_bus_db(0.8, 0.25) + volume_db(0.8) - volume_db(0.25)).abs() < 1e-4);
        assert_eq!(music_bus_db(0.0, 0.5), -80.0);
        assert_eq!(max_fps(-5), 0);
        assert_eq!(gamma_exponent(2.0), 0.5);
        assert_eq!(gamma_exponent(10.0), 1.0 / 3.0);
        let mut c = Cvars::default();
        register(&mut c, true);
        assert_eq!(c.string("r_fullscreen"), "0");
        assert_eq!(c.string("s_volume"), "0.8");
        assert_eq!(c.string("com_maxfps"), "85");
        assert_eq!(c.find("r_gamma").map(|v| v.flags & CVAR_ARCHIVE), Some(CVAR_ARCHIVE));
        assert_eq!(c.find("sv_maxclients").map(|v| v.flags & CVAR_ARCHIVE), Some(0));
        // autogen.cfg keeps the archived ones
        c.set("r_gamma", "1.4");
        assert!(c.write_variables().contains("seta r_gamma \"1.4\""));
    }
}
