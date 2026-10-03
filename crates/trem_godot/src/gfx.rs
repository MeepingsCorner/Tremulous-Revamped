//! Graphics modes (port addition, not in the original renderer).
//!
//! * **vanilla**: the look of the original renderer (the world and model
//!   shaders write their tr_shade.c result as emission; no post effects);
//! * **enhanced**: the same textures, light maps and light grid as physically
//!   lit Godot materials: the baked light becomes the materials' irradiance
//!   (so SSAO / SSIL / SSR / dynamic lights act on it), generated normal and
//!   roughness maps, a bumped baked light (the light grid's dominant
//!   direction), tone mapping, bloom, and on GPUs with hardware ray tracing
//!   the ray-traced / path-traced effects of `rt.rs`.
//!
//! `r_enhanced` (archived, F10 toggles it like Halo: CE Anniversary's
//! graphics switch) picks the mode live; `r_rtMode` -1 auto, 0 off,
//! 1 ray tracing (AO + reflections), 2 path tracing (AO + multi-bounce GI +
//! reflections). Auto follows the GPU found at startup (`GpuCaps`): NVIDIA
//! ray tracing, AMD path tracing, nothing without ray-query support (the
//! effects are never created there, so such GPUs cannot crash in them).

use godot::classes::environment::{AmbientSource, GlowBlendMode, ReflectionSource, ToneMapper};
use godot::classes::image::Format as ImageFormat;
use godot::classes::rendering_device::Features;
use godot::classes::{Environment, Image, ImageTexture, RenderingServer};
use godot::prelude::*;

use crate::cmd::{Cvars, CVAR_ARCHIVE};

/// The global shader uniform every world / model shader reads (0 vanilla, 1 enhanced).
pub const GLOBAL: &str = "trem_gfx";

/// (name, default) of the port's graphics cvars (CVAR_ARCHIVE).
/// r_rtBackend: 0 auto (from the GPU), 1 software, 2 hardware (where supported)
pub const GFX_CVARS: &[(&str, &str)] = &[("r_enhanced", "1"), ("r_rtMode", "-1"), ("r_rtBackend", "0")];

pub fn register_cvars(c: &mut Cvars) {
    for (n, v) in GFX_CVARS {
        c.get(n, v, CVAR_ARCHIVE);
    }
}

/// r_gamma 1.3 (a common Tremulous setting) as the shaders use it: gamma-space -> linear.
pub const R_GAMMA_EXP: f32 = 2.2 / 1.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RtMode {
    #[default]
    Off,
    Raytrace,
    Pathtrace,
}

impl RtMode {
    pub fn name(self) -> &'static str {
        match self {
            RtMode::Off => "off",
            RtMode::Raytrace => "ray tracing",
            RtMode::Pathtrace => "path tracing",
        }
    }
    pub fn as_int(self) -> i32 {
        self as i32
    }
}

/// How the ray-traced effects are traced on this GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RtBackend {
    /// no RenderingDevice (the Compatibility / OpenGL renderer): none
    #[default]
    None,
    /// compute shaders walking a BVH built on the CPU: any Vulkan / D3D12 /
    /// Metal GPU (older AMD Radeon, GTX, Intel), at half resolution
    Software,
    /// the GPU's ray tracing hardware through Vulkan ray tracing pipelines
    /// (NVIDIA RTX, AMD Radeon RX 6000 and newer, Intel Arc)
    Hardware,
}

impl RtBackend {
    pub fn name(self) -> &'static str {
        match self {
            RtBackend::None => "none",
            RtBackend::Software => "software (compute BVH)",
            RtBackend::Hardware => "hardware (Vulkan ray tracing)",
        }
    }
}

/// What the GPU found at startup can do.
#[derive(Clone, Debug, Default)]
pub struct GpuCaps {
    pub vendor: String,
    pub device: String,
    pub driver: String,
    /// Vulkan with `SUPPORTS_RAYTRACING_PIPELINE`
    pub hw_rt: bool,
    /// a RenderingDevice (compute shaders): Forward+ / Mobile on Vulkan, D3D12, Metal
    pub compute: bool,
}

impl GpuCaps {
    pub fn detect() -> Self {
        let rs = RenderingServer::singleton();
        let driver = rs.get_current_rendering_driver_name().to_string();
        let mut c = GpuCaps { vendor: rs.get_video_adapter_vendor().to_string(), device: rs.get_video_adapter_name().to_string(), driver, hw_rt: false, compute: false };
        if let Some(rd) = rs.get_rendering_device() {
            c.compute = true;
            c.hw_rt = c.driver.eq_ignore_ascii_case("vulkan") && rd.has_feature(Features::RAYTRACING_PIPELINE);
        }
        c
    }
    #[allow(dead_code)]
    pub fn is_nvidia(&self) -> bool {
        let v = format!("{} {}", self.vendor, self.device).to_ascii_lowercase();
        v.contains("nvidia") || v.contains("geforce")
    }
    pub fn is_amd(&self) -> bool {
        let v = format!("{} {}", self.vendor, self.device).to_ascii_lowercase();
        v.contains("amd") || v.contains("advanced micro") || v.contains("radeon") || v.contains("ati ")
    }
    pub fn backend(&self) -> RtBackend {
        if self.hw_rt {
            RtBackend::Hardware
        } else if self.compute {
            RtBackend::Software
        } else {
            RtBackend::None
        }
    }
    /// `r_rtMode -1`: AMD path tracing (on its ray accelerators, or the
    /// software tracer on Radeons without them), NVIDIA and the rest ray
    /// tracing; nothing on the Compatibility renderer.
    pub fn auto_rt(&self) -> RtMode {
        match self.backend() {
            RtBackend::None => RtMode::Off,
            _ if self.is_amd() => RtMode::Pathtrace,
            _ => RtMode::Raytrace,
        }
    }
    /// The mode `r_rtMode` asks for, limited to what the GPU can do.
    pub fn rt_mode(&self, cvar: i32) -> RtMode {
        if self.backend() == RtBackend::None {
            return RtMode::Off;
        }
        match cvar {
            0 => RtMode::Off,
            1 => RtMode::Raytrace,
            2 => RtMode::Pathtrace,
            _ => self.auto_rt(),
        }
    }
    pub fn describe(&self) -> String {
        format!("GPU: {} ({}), driver {}, ray tracing backend: {}, default mode: {}", self.device, self.vendor, self.driver, self.backend().name(), self.auto_rt().name())
    }
}

pub fn set_global(enhanced: bool) {
    RenderingServer::singleton().global_shader_parameter_set(GLOBAL, &(if enhanced { 1.0f32 } else { 0.0 }).to_variant());
}

/// The vanilla environment's settings as build_level made them.
#[derive(Clone, Debug)]
pub struct VanillaEnv {
    pub ambient_source: AmbientSource,
    pub ambient_color: Color,
    pub ambient_energy: f32,
}

impl VanillaEnv {
    pub fn capture(env: &Gd<Environment>) -> Self {
        VanillaEnv { ambient_source: env.get_ambient_source(), ambient_color: env.get_ambient_light_color(), ambient_energy: env.get_ambient_light_energy() }
    }
}

/// Configure the Environment for a mode. `rt`: the ray-traced effects run
/// (they replace SSAO / SSR, which would double them).
pub fn configure_environment(env: &mut Gd<Environment>, enhanced: bool, rt: RtMode, v: &VanillaEnv) {
    if !enhanced {
        env.set_tonemapper(ToneMapper::LINEAR);
        env.set_tonemap_exposure(1.0);
        env.set_glow_enabled(true);
        env.set_glow_intensity(0.8);
        env.set_glow_strength(1.0);
        env.set_glow_bloom(0.0);
        env.set_glow_blend_mode(GlowBlendMode::SOFTLIGHT);
        env.set_glow_hdr_bleed_threshold(1.0);
        env.set_ssao_enabled(false);
        env.set_ssil_enabled(false);
        env.set_ssr_enabled(false);
        env.set_adjustment_enabled(false);
        env.set_volumetric_fog_enabled(false);
        env.set_ambient_source(v.ambient_source);
        env.set_ambient_light_color(v.ambient_color);
        env.set_ambient_light_energy(v.ambient_energy);
        env.set_reflection_source(ReflectionSource::BG);
        return;
    }
    // filmic tone mapping of the HDR baked light (overbright > 1 now kept)
    env.set_tonemapper(ToneMapper::AGX);
    env.set_tonemap_exposure(1.35);
    env.set_tonemap_agx_contrast(1.15);
    // bloom from the bright parts only (lights, glows, sky)
    env.set_glow_enabled(true);
    env.set_glow_intensity(0.55);
    env.set_glow_strength(1.0);
    env.set_glow_bloom(0.04);
    env.set_glow_blend_mode(GlowBlendMode::SCREEN);
    env.set_glow_hdr_bleed_threshold(1.1);
    env.set_glow_hdr_bleed_scale(1.5);
    // screen-space ambient occlusion and indirect light (on the baked light,
    // which the materials hand over as irradiance); the ray-traced modes
    // trace these instead
    let ss = rt == RtMode::Off;
    env.set_ssao_enabled(ss);
    env.set_ssao_radius(1.2);
    env.set_ssao_intensity(2.2);
    env.set_ssao_power(1.6);
    env.set_ssao_detail(0.6);
    env.set_ssao_direct_light_affect(0.15);
    env.set_ssil_enabled(true);
    env.set_ssil_radius(4.0);
    env.set_ssil_intensity(0.8);
    env.set_ssr_enabled(ss);
    env.set_ssr_max_steps(96);
    env.set_ssr_fade_in(0.15);
    env.set_ssr_fade_out(2.0);
    env.set_ssr_depth_tolerance(0.3);
    env.set_adjustment_enabled(true);
    env.set_adjustment_brightness(1.0);
    env.set_adjustment_contrast(1.04);
    env.set_adjustment_saturation(1.08);
    // the materials without their own irradiance (effects, placeholders)
    env.set_ambient_source(AmbientSource::COLOR);
    env.set_ambient_light_color(v.ambient_color);
    env.set_ambient_light_energy(v.ambient_energy.min(0.6));
    env.set_reflection_source(ReflectionSource::BG);
}

/// Surface response guessed from a texture's name (the data has no
/// material maps): (roughness, specular, normal-map strength).
pub fn surface_props(name: &str) -> (f32, f32, f32) {
    let n = name.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|s| n.contains(s));
    if has(&["glass", "window", "screen", "monitor"]) {
        (0.08, 0.6, 0.2)
    } else if has(&["water", "slime", "liquid", "goo"]) {
        (0.05, 0.5, 0.3)
    } else if has(&["chrome", "steel", "shiny", "polish"]) {
        (0.25, 0.6, 0.7)
    } else if has(&["metal", "floor", "grate", "plate", "panel", "pipe", "trim", "base_", "human", "tech", "door", "rail"]) {
        (0.45, 0.5, 1.0)
    } else if has(&["organic", "alien", "creep", "flesh", "slime", "egg", "hive"]) {
        (0.35, 0.45, 1.2)
    } else if has(&["rock", "stone", "dirt", "ground", "concrete", "cement", "brick", "sand", "terrain", "cliff"]) {
        (0.85, 0.3, 1.3)
    } else if has(&["light", "lamp", "glow", "flare"]) {
        (0.4, 0.5, 0.3)
    } else {
        (0.7, 0.4, 0.9)
    }
}

/// A texture's generated maps: the tangent-space normal map (OpenGL / Godot
/// convention, from the albedo's luminance as a height field) and the mean
/// albedo in linear space (the ray tracer's surface colour).
pub struct DerivedMaps {
    pub normal: Option<Gd<ImageTexture>>,
    pub mean_linear: [f32; 3],
}

/// Height from luminance -> 3x3 box blur -> Sobel -> normal. Small (<8 px)
/// or flat images get no normal map.
pub fn derive_maps(img: &Gd<Image>, want_normal: bool) -> DerivedMaps {
    let mut im = img.duplicate_resource();
    if im.is_compressed() {
        im.decompress();
    }
    im.clear_mipmaps();
    if im.get_format() != ImageFormat::RGBA8 {
        im.convert(ImageFormat::RGBA8);
    }
    let (w, h) = (im.get_width() as usize, im.get_height() as usize);
    let data = im.get_data();
    let px = data.as_slice();
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return DerivedMaps { normal: None, mean_linear: [0.5; 3] };
    }
    let mut sum = [0f64; 3];
    let mut lum = vec![0f32; w * h];
    for i in 0..w * h {
        let (r, g, b) = (px[i * 4] as f32 / 255.0, px[i * 4 + 1] as f32 / 255.0, px[i * 4 + 2] as f32 / 255.0);
        sum[0] += r as f64;
        sum[1] += g as f64;
        sum[2] += b as f64;
        lum[i] = 0.299 * r + 0.587 * g + 0.114 * b;
    }
    let n = (w * h) as f64;
    let mean_linear = [0, 1, 2].map(|k| ((sum[k] / n) as f32).powf(R_GAMMA_EXP));
    if !want_normal || w < 8 || h < 8 {
        return DerivedMaps { normal: None, mean_linear };
    }
    // wrap-around sampling (the textures tile)
    let at = |v: &Vec<f32>, x: isize, y: isize| v[((y.rem_euclid(h as isize)) as usize) * w + (x.rem_euclid(w as isize)) as usize];
    let mut hgt = vec![0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let mut s = 0.0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    s += at(&lum, x + dx, y + dy);
                }
            }
            hgt[y as usize * w + x as usize] = s / 9.0;
        }
    }
    // strength in texels: a full black->white step over one texel tilts ~70°
    let strength = 2.5f32 * (w.min(h) as f32 / 256.0).max(0.5);
    let mut out = vec![0u8; w * h * 3];
    let mut flat = true;
    for y in 0..h as isize {
        for x in 0..w as isize {
            let gx = (at(&hgt, x + 1, y - 1) + 2.0 * at(&hgt, x + 1, y) + at(&hgt, x + 1, y + 1)) - (at(&hgt, x - 1, y - 1) + 2.0 * at(&hgt, x - 1, y) + at(&hgt, x - 1, y + 1));
            let gy = (at(&hgt, x - 1, y + 1) + 2.0 * at(&hgt, x, y + 1) + at(&hgt, x + 1, y + 1)) - (at(&hgt, x - 1, y - 1) + 2.0 * at(&hgt, x, y - 1) + at(&hgt, x + 1, y - 1));
            // image y runs down, the normal map's green runs up
            let (nx, ny, nz) = (-gx * strength, gy * strength, 1.0f32);
            let l = (nx * nx + ny * ny + nz * nz).sqrt();
            let (nx, ny, nz) = (nx / l, ny / l, nz / l);
            if nz < 0.999 {
                flat = false;
            }
            let o = (y as usize * w + x as usize) * 3;
            out[o] = ((nx * 0.5 + 0.5) * 255.0).round() as u8;
            out[o + 1] = ((ny * 0.5 + 0.5) * 255.0).round() as u8;
            out[o + 2] = ((nz * 0.5 + 0.5) * 255.0).round() as u8;
        }
    }
    if flat {
        return DerivedMaps { normal: None, mean_linear };
    }
    let normal = Image::create_from_data(w as i32, h as i32, false, ImageFormat::RGB8, &PackedByteArray::from(out.as_slice())).and_then(|mut i| {
        i.generate_mipmaps();
        ImageTexture::create_from_image(&i)
    });
    DerivedMaps { normal, mean_linear }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rt_mode_follows_the_gpu() {
        let nv = GpuCaps { vendor: "NVIDIA".into(), device: "NVIDIA GeForce RTX 4060".into(), driver: "vulkan".into(), hw_rt: true, compute: true };
        let amd = GpuCaps { vendor: "AMD".into(), device: "AMD Radeon RX 7800 XT".into(), driver: "vulkan".into(), hw_rt: true, compute: true };
        let old_amd = GpuCaps { vendor: "AMD".into(), device: "Radeon RX 580".into(), driver: "vulkan".into(), hw_rt: false, compute: true };
        let gtx = GpuCaps { vendor: "NVIDIA".into(), device: "GeForce GTX 1060".into(), driver: "d3d12".into(), hw_rt: false, compute: true };
        let gl = GpuCaps { vendor: "Intel".into(), device: "HD 4000".into(), driver: "opengl3".into(), hw_rt: false, compute: false };
        assert_eq!((nv.backend(), nv.rt_mode(-1)), (RtBackend::Hardware, RtMode::Raytrace));
        assert_eq!((amd.backend(), amd.rt_mode(-1)), (RtBackend::Hardware, RtMode::Pathtrace));
        // Radeons without ray accelerators path trace in software
        assert_eq!((old_amd.backend(), old_amd.rt_mode(-1)), (RtBackend::Software, RtMode::Pathtrace));
        assert_eq!((gtx.backend(), gtx.rt_mode(-1)), (RtBackend::Software, RtMode::Raytrace));
        // nothing to trace with on the Compatibility renderer, whatever is asked
        assert_eq!(gl.rt_mode(2), RtMode::Off);
        assert_eq!(nv.rt_mode(2), RtMode::Pathtrace);
        assert_eq!(amd.rt_mode(0), RtMode::Off);
    }

    #[test]
    fn surface_props_by_name() {
        assert!(surface_props("textures/atcs/glass_1").0 < 0.2);
        assert!(surface_props("textures/arachnid2/rock_wall").0 > 0.8);
        assert!(surface_props("textures/atcs/floor_grate").0 < 0.6);
    }
}

/// The graphics mode's live state on TremGame.
#[derive(Default)]
pub struct GfxRuntime {
    pub caps: Option<GpuCaps>,
    pub env: Option<Gd<Environment>>,
    pub vanilla: Option<VanillaEnv>,
    /// build_level's sun and the lights over the reactor / overmind: the
    /// sheet arena's lighting; a BSP map's light is its light maps, so the
    /// enhanced (lit) mode hides them there
    pub sun: Option<Gd<godot::classes::DirectionalLight3D>>,
    pub fake_lights: Vec<Gd<godot::classes::OmniLight3D>>,
    pub rt: Option<Gd<crate::rt::TremRtEffect>>,
    /// the 3D view's SubViewport and its MSAA setting: the ray-traced
    /// effects change the resolved colour, which a multisampled transparent
    /// pass would resolve over, so they run without MSAA (SMAA instead)
    pub view_vp: Option<Gd<godot::classes::SubViewport>>,
    pub view_msaa: Option<(godot::classes::viewport::Msaa, godot::classes::viewport::ScreenSpaceAa)>,
    pub is_bsp: bool,
    pub enhanced: bool,
    pub mode: RtMode,
    pub sky: [f32; 3],
    /// the next map's ray tracer geometry (from the loader)
    pub pending_geom: Option<crate::bspview::RtGeometry>,
}

impl GfxRuntime {
    /// Apply the cvars: `enhanced` (r_enhanced), `rt_cvar` (r_rtMode),
    /// `debug` (r_rtDebug).
    pub fn apply(&mut self, enhanced: bool, rt_cvar: i32, debug: i32, backend_cvar: i32) {
        let caps = self.caps.clone().unwrap_or_default();
        let backend = match (caps.backend(), backend_cvar) {
            (RtBackend::None, _) => RtBackend::None,
            (_, 1) => RtBackend::Software,
            (b, _) => b,
        };
        if let Some(rt) = self.rt.as_mut() {
            rt.bind_mut().set_backend(backend);
        }
        let failed = self.rt.as_ref().is_some_and(|r| r.bind().failed());
        let mode = if enhanced && !failed && self.rt.is_some() { caps.rt_mode(rt_cvar) } else { RtMode::Off };
        set_global(enhanced);
        if let (Some(env), Some(v)) = (self.env.as_mut(), self.vanilla.as_ref()) {
            configure_environment(env, enhanced, mode, v);
        }
        let hide = enhanced && self.is_bsp;
        if let Some(s) = self.sun.as_mut() {
            if s.is_instance_valid() {
                s.set_visible(!hide);
            }
        }
        for l in self.fake_lights.iter_mut() {
            if l.is_instance_valid() {
                l.set_visible(!hide);
            }
        }
        if let Some(vp) = self.view_vp.as_mut().filter(|v| v.is_instance_valid()) {
            use godot::classes::viewport::{Msaa, ScreenSpaceAa};
            let orig = *self.view_msaa.get_or_insert((vp.get_msaa_3d(), vp.get_screen_space_aa()));
            if mode != RtMode::Off {
                vp.set_msaa_3d(Msaa::DISABLED);
                vp.set_screen_space_aa(ScreenSpaceAa::SMAA);
            } else {
                vp.set_msaa_3d(orig.0);
                vp.set_screen_space_aa(orig.1);
            }
        }
        if let Some(rt) = self.rt.as_mut() {
            let mut r = rt.bind_mut();
            r.set_sky(self.sky);
            r.set_debug(debug);
            if let Some(g) = self.pending_geom.take() {
                r.set_geometry(g);
            }
            r.set_mode(mode);
        }
        self.enhanced = enhanced;
        self.mode = mode;
    }

    pub fn describe(&self) -> String {
        let caps = self.caps.clone().unwrap_or_default();
        let status = self.rt.as_ref().map(|r| r.bind().describe()).unwrap_or_else(|| "not available".into());
        format!(
            "{}\ngraphics: {}, ray-traced effects: {} ({})\n",
            caps.describe(),
            if self.enhanced { "enhanced" } else { "vanilla" },
            self.mode.name(),
            if status.is_empty() { "idle".to_string() } else { status }
        )
    }
}
