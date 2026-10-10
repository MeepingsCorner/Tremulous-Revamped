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
/// r_hdTextures / r_hdPath ("" = user://hd) / r_hdMaxSize: the 4K pack (hdtex.rs)
/// r_upscale: -1 auto (FSR 2 while ray / path tracing), 0 off, 1 always
/// (enhanced only); r_upscaleScale: its render scale
/// r_dlightShadows: how many dynamic lights cast shadows (enhanced only)
pub const GFX_CVARS: &[(&str, &str)] = &[
    ("r_enhanced", "1"),
    ("r_rtMode", "-1"),
    ("r_rtBackend", "0"),
    ("r_hdTextures", "1"),
    ("r_hdPath", ""),
    ("r_hdMaxSize", "4096"),
    ("r_upscale", "-1"),
    ("r_upscaleScale", "0.77"),
    ("r_dlightShadows", "4"),
    // r_ragdolls: corpses fall as ragdolls in the enhanced mode (ragdoll.rs)
    ("r_ragdolls", "1"),
    // r_dynamicShadows: players, structures and corpses throw shadows on the
    // map (the shadow sun, enhanced only)
    ("r_dynamicShadows", "1"),
    ("r_modelDebug", "0"),
];

/// The node group of every dynamic light (muzzle flashes, missiles,
/// effects): enhanced mode gives the most important of them shadows.
pub const DLIGHT_GROUP: &str = "trem_dlight";

/// Enhanced dynamic shadows: directional "shadow lights" fixed in the world,
/// along the map's light directions (lamps.rs; the ones lighting the area
/// around the view). They light nothing: the shaders' light() (bspview.rs,
/// models.rs) only take the directed share of the baked / grid light away
/// where a shadow falls, from the light whose direction the spot's own
/// light grid has. The
/// map and the solid surfaces of every model (players, their weapons,
/// structures, ragdolls, missiles, map models) take its shadows (this render
/// layer, its cull mask); the models cast them (this layer, its shadow
/// caster mask), the map does not (its light and shadows are baked: from
/// inside a room the sun would black it out).
pub const SHADOW_RECEIVER_LAYER: u32 = 2;
pub const SHADOW_CASTER_LAYER: u32 = 4;
/// Our own body in first person (shadow only): the shadow sun takes it, the
/// flashlight held at the eye does not (it would light the inside of it).
pub const SHADOW_SELF_LAYER: u32 = 8;
/// The shader global: how much of the directed light a shadow takes.
pub const SHADOW_GLOBAL: &str = "trem_shadow_k";
pub const SHADOW_STRENGTH: f32 = 0.65;

/// FSR 2 for this frame? -1 auto: while ray / path tracing (the expensive
/// modes); 1: always in the enhanced mode; 0 / vanilla: never.
pub fn upscale_on(r_upscale: i32, enhanced: bool, rt: RtMode) -> bool {
    enhanced && (r_upscale > 0 || (r_upscale < 0 && rt != RtMode::Off))
}

/// The dynamic lights that get shadows: the `max` with the largest
/// range / (1 + distance to the view), so near and big lights win.
pub fn shadow_casters(lights: &[(f32, f32)], max: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..lights.len()).collect();
    let score = |i: usize| lights[i].1 / (1.0 + lights[i].0);
    idx.sort_by(|&a, &b| score(b).partial_cmp(&score(a)).unwrap_or(std::cmp::Ordering::Equal));
    idx.truncate(max);
    idx
}

/// Godot FogMaterial density for a Q3 fog opaque at `dist` Quake units:
/// transmittance e^(-density * d) reaches 2% at d = dist / 32 (Godot units).
pub fn fog_density(dist: f32) -> f32 {
    50f32.ln() / (dist / 32.0).max(0.05)
}

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

/// `model_debug` (r_modelDebug, enhanced only) rides in the fraction: the
/// model shader reads 1 = no subsurface scattering, 2 = no reflections, 3 both.
pub fn set_global(enhanced: bool, model_debug: i32) {
    let v = if enhanced { 1.0 + 0.01 * model_debug.clamp(0, 9) as f32 } else { 0.0 };
    RenderingServer::singleton().global_shader_parameter_set(GLOBAL, &v.to_variant());
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
    // volumetric fog: the fog brushes' volumes, and a faint haze the
    // dynamic lights scatter in (light shafts from muzzle flashes, missiles)
    env.set_volumetric_fog_enabled(true);
    env.set_volumetric_fog_density(0.004);
    env.set_volumetric_fog_albedo(Color::from_rgb(0.85, 0.85, 0.9));
    env.set_volumetric_fog_anisotropy(0.35);
    env.set_volumetric_fog_length(64.0);
    env.set_volumetric_fog_sky_affect(0.0);
    env.set_volumetric_fog_temporal_reprojection_enabled(true);
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
/// Enhanced-mode material of a surface class (by texture name): metalness,
/// a wet clearcoat sheen, and how strongly the close-range detail normal
/// map applies; `base` is surface_props' (roughness, specular, bump).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceClass {
    pub base: (f32, f32, f32),
    pub metal: f32,
    pub wet: f32,
    pub detail: f32,
}

pub fn surface_class(name: &str) -> SurfaceClass {
    let n = name.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|s| n.contains(s));
    let base = surface_props(name);
    let (metal, wet, detail) = if has(&["glass", "window", "screen", "monitor"]) {
        (0.0, 0.0, 0.0)
    } else if has(&["water", "slime", "liquid", "goo"]) {
        (0.0, 0.8, 0.1)
    } else if has(&["chrome", "steel", "shiny", "polish"]) {
        (0.6, 0.0, 0.15)
    } else if has(&["metal", "floor", "grate", "plate", "panel", "pipe", "trim", "base_", "human", "tech", "door", "rail"]) {
        (0.35, 0.0, 0.25)
    } else if has(&["organic", "alien", "creep", "flesh", "egg", "hive"]) {
        (0.0, 0.55, 0.45)
    } else if has(&["rock", "stone", "dirt", "ground", "concrete", "cement", "brick", "sand", "terrain", "cliff"]) {
        (0.0, 0.0, 0.6)
    } else if has(&["light", "lamp", "glow", "flare"]) {
        (0.0, 0.0, 0.0)
    } else {
        (0.0, 0.0, 0.35)
    };
    SurfaceClass { base, metal, wet, detail }
}

/// A tileable detail normal map (256², several octaves of periodic value
/// noise): the close-range relief that stands in for parallax mapping.
pub fn detail_normal_rgba(size: usize) -> Vec<u8> {
    let hash = |x: i64, y: i64, p: i64| -> f32 {
        let (x, y) = (x.rem_euclid(p), y.rem_euclid(p));
        let mut h = (x as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ (y as u64).wrapping_mul(0xC2B2AE3D27D4EB4F);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58476D1CE4E5B9);
        h ^= h >> 32;
        (h & 0xFFFF) as f32 / 65535.0
    };
    let noise = |u: f32, v: f32, p: i64| -> f32 {
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = hash(x0, y0, p) + (hash(x0 + 1, y0, p) - hash(x0, y0, p)) * sx;
        let b = hash(x0, y0 + 1, p) + (hash(x0 + 1, y0 + 1, p) - hash(x0, y0 + 1, p)) * sx;
        a + (b - a) * sy
    };
    let height = |x: f32, y: f32| -> f32 {
        let mut h = 0.0;
        let mut amp = 0.5;
        for o in 0..4 {
            let p = 8i64 << o; // lattice periods divide the tile: seamless
            h += amp * noise(x / size as f32 * p as f32, y / size as f32 * p as f32, p);
            amp *= 0.5;
        }
        h
    };
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32, y as f32);
            let dx = height(fx + 1.0, fy) - height(fx - 1.0, fy);
            let dy = height(fx, fy + 1.0) - height(fx, fy - 1.0);
            let n = [-dx * 6.0, -dy * 6.0, 1.0];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let i = (y * size + x) * 4;
            for k in 0..3 {
                out[i + k] = ((n[k] / l * 0.5 + 0.5) * 255.0).round() as u8;
            }
            out[i + 3] = 255;
        }
    }
    out
}

thread_local! {
    static DETAIL: std::cell::RefCell<Option<Gd<ImageTexture>>> = const { std::cell::RefCell::new(None) };
    /// particle materials: (material, alpha-blended smoke?) - switched with the mode
    static FX_MATS: std::cell::RefCell<Vec<(Gd<godot::classes::StandardMaterial3D>, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
    static FX_HAZE: std::cell::RefCell<Vec<Gd<godot::classes::MeshInstance3D>>> = const { std::cell::RefCell::new(Vec::new()) };
    static FX_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Drop the thread-local engine objects (TremGame::exit_tree).
pub fn release_statics() {
    DETAIL.with(|d| d.borrow_mut().take());
    FX_MATS.with(|v| v.borrow_mut().clear());
    FX_HAZE.with(|v| v.borrow_mut().clear());
}

/// The shared detail normal map (built once).
pub fn detail_normal() -> Gd<ImageTexture> {
    DETAIL.with(|d| {
        d.borrow_mut()
            .get_or_insert_with(|| {
                let n = 256;
                let img = Image::create_from_data(n as i32, n as i32, false, ImageFormat::RGBA8, &PackedByteArray::from(detail_normal_rgba(n))).expect("detail normal");
                let mut img = img;
                img.generate_mipmaps();
                ImageTexture::create_from_image(&img).expect("detail texture")
            })
            .clone()
    })
}

fn fx_apply(m: &mut Gd<godot::classes::StandardMaterial3D>, smoke: bool, on: bool) {
    use godot::classes::base_material_3d::{Feature, ShadingMode, TextureParam};
    // soft particles: fade where they meet the world instead of a hard cut
    m.set_proximity_fade_enabled(on);
    m.set_proximity_fade_distance(0.4);
    if smoke {
        // lit smoke: shaded by the scene's lights, with a floor of its own colour
        m.set_shading_mode(if on { ShadingMode::PER_VERTEX } else { ShadingMode::UNSHADED });
        m.set_feature(Feature::EMISSION, on);
        if on {
            if let Some(t) = m.get_texture(TextureParam::ALBEDO) {
                m.set_texture(TextureParam::EMISSION, &t);
            }
            m.set_emission(Color::from_rgb(1.0, 1.0, 1.0));
            m.set_emission_energy_multiplier(0.45);
        }
    }
}

/// A particle material (particles.rs) that follows the graphics mode.
pub fn register_fx_material(mut m: Gd<godot::classes::StandardMaterial3D>, smoke: bool) {
    fx_apply(&mut m, smoke, FX_ON.get());
    FX_MATS.with(|v| v.borrow_mut().push((m, smoke)));
}

/// A heat-haze layer (flame particles), shown in the enhanced mode only.
pub fn register_fx_haze(mut mi: Gd<godot::classes::MeshInstance3D>) {
    mi.set_visible(FX_ON.get());
    FX_HAZE.with(|v| v.borrow_mut().push(mi));
}

fn fx_set_enhanced(on: bool) {
    if FX_ON.get() == on {
        return;
    }
    FX_ON.set(on);
    FX_MATS.with(|v| {
        for (m, smoke) in v.borrow_mut().iter_mut() {
            fx_apply(m, *smoke, on);
        }
    });
    FX_HAZE.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|m| m.is_instance_valid());
        for m in v.iter_mut() {
            m.set_visible(on);
        }
    });
}

/// The heat haze over flame particles: the screen behind, wobbled.
pub const HAZE_SHADER: &str = r#"
shader_type spatial;
render_mode unshaded, blend_mix, depth_draw_never, cull_disabled;
uniform sampler2D screen : hint_screen_texture, filter_linear_mipmap;
uniform sampler2D tex : filter_linear_mipmap;
void fragment() {
    float a = texture(tex, UV).a * COLOR.a;
    vec2 off = vec2(sin(TIME * 21.0 + UV.y * 31.0), cos(TIME * 17.0 + UV.x * 29.0)) * 0.008 * a;
    ALBEDO = textureLod(screen, SCREEN_UV + off, 0.0).rgb;
    ALPHA = clamp(a * 1.4, 0.0, 0.9);
}
"#;

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
    let (normal, mean_linear) = derive_maps_image(img, want_normal);
    DerivedMaps { normal: normal.and_then(|i| ImageTexture::create_from_image(&i)), mean_linear }
}

/// `derive_maps` as Images (the HD texture builder compresses them).
pub fn derive_maps_image(img: &Gd<Image>, want_normal: bool) -> (Option<Gd<Image>>, [f32; 3]) {
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
        return (None, [0.5; 3]);
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
        return (None, mean_linear);
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
        return (None, mean_linear);
    }
    let normal = Image::create_from_data(w as i32, h as i32, false, ImageFormat::RGB8, &PackedByteArray::from(out.as_slice())).map(|mut i| {
        i.generate_mipmaps();
        i
    });
    (normal, mean_linear)
}

#[cfg(test)]
mod tests {
    use godot::prelude::Vector3;

    #[test]
    fn upscale_auto_only_with_ray_tracing_in_enhanced() {
        use super::{upscale_on, RtMode};
        assert!(upscale_on(-1, true, RtMode::Pathtrace));
        assert!(!upscale_on(-1, true, RtMode::Off));
        assert!(upscale_on(1, true, RtMode::Off));
        assert!(!upscale_on(1, false, RtMode::Raytrace));
        assert!(!upscale_on(0, true, RtMode::Raytrace));
    }

    #[test]
    fn near_big_lights_get_the_shadows() {
        // (distance, range)
        let l = [(50.0, 6.0), (2.0, 9.0), (10.0, 6.0), (1.0, 2.0)];
        assert_eq!(super::shadow_casters(&l, 2), vec![1, 3]);
        assert!(super::shadow_casters(&l, 0).is_empty());
    }

    #[test]
    fn fog_is_opaque_at_its_distance() {
        let d = super::fog_density(512.0);
        assert!(((-d * 16.0f32).exp() - 0.02).abs() < 1e-3);
    }

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
    fn surface_classes() {
        assert!(super::surface_class("textures/atcs/floor_grate").metal > 0.2);
        assert!(super::surface_class("textures/arachnid2/creep_wall").wet > 0.3);
        assert_eq!(super::surface_class("textures/atcs/glass_1").detail, 0.0);
    }

    #[test]
    fn detail_normal_tiles_seamlessly() {
        let n = 128;
        let d = super::detail_normal_rgba(n);
        // across the seam (x = n-1 next to x = 0) neighbours differ no more
        // than neighbours inside the tile do: no visible edge when repeated
        let px = |x: usize, y: usize| [d[(y * n + x) * 4] as i32, d[(y * n + x) * 4 + 1] as i32];
        let diff = |a: [i32; 2], b: [i32; 2]| (a[0] - b[0]).abs().max((a[1] - b[1]).abs());
        let (mut seam, mut inner) = (0, 0);
        for y in 0..n {
            seam = seam.max(diff(px(n - 1, y), px(0, y)));
            for x in 0..n - 1 {
                inner = inner.max(diff(px(x, y), px(x + 1, y)));
            }
        }
        assert!(seam <= inner, "seam {seam} vs inside {inner}");
        // it has relief: not every normal points straight up
        assert!(d.chunks(4).any(|p| (p[0] as i32 - 128).abs() > 10));
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
    /// the next map's fog brushes, and the volumes made from them
    pub pending_fogs: Vec<crate::bspview::FogBox>,
    pub fog_nodes: Vec<Gd<godot::classes::FogVolume>>,
    /// the vanilla mode's fog (tr_shade.c RB_FogPass): one screen pass per fog brush
    pub fog_quads: Vec<Gd<godot::classes::MeshInstance3D>>,
    pub upscale: (i32, f32),
    pub model_debug: i32,
    pub shadow_max: usize,
    /// the enhanced shadow lights (SHADOW_RECEIVER_LAYER): (direction
    /// index in lamps.rs ShadowDirs, the light, its fade 0..1)
    pub shadow_lights: Vec<(usize, Gd<godot::classes::DirectionalLight3D>, f32)>,
    /// the directions picked for the view's area, and when to pick again
    pub shadow_want: Vec<usize>,
    pub shadow_pick_t: f32,
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
        set_global(enhanced, self.model_debug);
        fx_set_enhanced(enhanced);
        crate::hdtex::set_ui_enabled(enhanced);
        if let (Some(env), Some(v)) = (self.env.as_mut(), self.vanilla.as_ref()) {
            configure_environment(env, enhanced, mode, v);
        }
        // the Compatibility renderer keeps its HDR buffer (glow) at 1/4
        // brightness and applies that to a multiply blend's colour too:
        // the map's multiply passes (q3shader.rs) make up for it
        let compat = RenderingServer::singleton().get_current_rendering_method().to_string() == "gl_compatibility";
        let glow = self.env.as_ref().is_some_and(|e| e.is_glow_enabled());
        RenderingServer::singleton().global_shader_parameter_set("trem_mul_k", &(if compat && glow { 4.0f32 } else { 1.0 }).to_variant());
        // r_modelDebug 4: the enhanced mode without AgX (linear tone mapping)
        if let (Some(env), true) = (self.env.as_mut(), enhanced && self.model_debug == 4) {
            env.set_tonemapper(ToneMapper::LINEAR);
            env.set_tonemap_exposure(1.0);
        }
        let hide = enhanced && self.is_bsp;
        if let Some(s) = self.sun.as_mut() {
            if s.is_instance_valid() {
                s.set_visible(!hide);
            }
        }
        // the reactor's blue and the overmind's green lights are the sheet
        // arena's lighting: cgame has none, so a BSP map hides them in the
        // classic (vanilla) mode too
        for l in self.fake_lights.iter_mut() {
            if l.is_instance_valid() {
                l.set_visible(!self.is_bsp);
            }
        }
        for f in self.fog_nodes.iter_mut() {
            if f.is_instance_valid() {
                f.set_visible(enhanced);
            }
        }
        let fsr = upscale_on(self.upscale.0, enhanced, mode);
        if let Some(vp) = self.view_vp.as_mut().filter(|v| v.is_instance_valid()) {
            use godot::classes::viewport::{Msaa, Scaling3DMode, ScreenSpaceAa};
            let orig = *self.view_msaa.get_or_insert((vp.get_msaa_3d(), vp.get_screen_space_aa()));
            if fsr {
                // FSR 2 is temporal: it antialiases itself
                vp.set_msaa_3d(Msaa::DISABLED);
                vp.set_screen_space_aa(ScreenSpaceAa::DISABLED);
                vp.set_scaling_3d_mode(Scaling3DMode::FSR2);
                vp.set_scaling_3d_scale(self.upscale.1.clamp(0.5, 1.0));
            } else {
                vp.set_scaling_3d_mode(Scaling3DMode::BILINEAR);
                vp.set_scaling_3d_scale(1.0);
                if mode != RtMode::Off {
                    vp.set_msaa_3d(Msaa::DISABLED);
                    vp.set_screen_space_aa(ScreenSpaceAa::SMAA);
                } else {
                    vp.set_msaa_3d(orig.0);
                    vp.set_screen_space_aa(orig.1);
                }
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

    /// The map's fog brushes as volumetric fog volumes (shown in the
    /// enhanced mode only), children of `parent`.
    pub fn build_fogs(&mut self, parent: &mut Gd<Node>) {
        use godot::classes::{FogMaterial, FogVolume};
        self.fog_nodes.retain(|f| f.is_instance_valid());
        for mut f in self.fog_nodes.drain(..) {
            f.queue_free();
        }
        self.fog_quads.retain(|f| f.is_instance_valid());
        for mut f in self.fog_quads.drain(..) {
            f.queue_free();
        }
        for b in std::mem::take(&mut self.pending_fogs) {
            let c = [0, 1, 2].map(|k| (b.mins[k] + b.maxs[k]) * 0.5);
            let size = crate::geom::q2g_dir(&[b.maxs[0] - b.mins[0], b.maxs[1] - b.mins[1], b.maxs[2] - b.mins[2]]).abs() * crate::geom::SCALE;
            let mut m = FogMaterial::new_gd();
            m.set_density(fog_density(b.dist));
            m.set_albedo(Color::from_rgb(b.color[0], b.color[1], b.color[2]));
            // Q3 fog is not lit: a little of its own colour keeps it visible in the dark
            m.set_emission(Color::from_rgb(b.color[0] * 0.15, b.color[1] * 0.15, b.color[2] * 0.15));
            m.set_edge_fade(0.2);
            let mut v = FogVolume::new_alloc();
            v.set_size(size);
            v.set_material(&m);
            v.set_position(crate::geom::q2g(&c));
            v.set_visible(self.enhanced);
            parent.add_child(&v);
            self.fog_nodes.push(v);
            let q = vanilla_fog_quad(&b);
            parent.add_child(&q);
            self.fog_quads.push(q);
        }
    }

    /// Enhanced mode: shadows for the `shadow_max` dynamic lights that
    /// matter most; vanilla: none (r_dynamiclight lights cast no shadows).
    pub fn shadow_budget(&self, tree: &mut Gd<godot::classes::SceneTree>, view: Vector3) {
        let mut lights: Vec<Gd<godot::classes::OmniLight3D>> = tree.get_nodes_in_group(DLIGHT_GROUP).iter_shared().filter_map(|n| n.try_cast::<godot::classes::OmniLight3D>().ok()).filter(|l| l.is_visible_in_tree()).collect();
        let max = if self.enhanced { self.shadow_max } else { 0 };
        let info: Vec<(f32, f32)> = lights.iter().map(|l| (l.get_global_position().distance_to(view), l.get_param(godot::classes::light_3d::Param::RANGE))).collect();
        let on = shadow_casters(&info, max);
        for (i, l) in lights.iter_mut().enumerate() {
            let want = on.contains(&i);
            if l.has_shadow() != want {
                l.set_shadow(want);
            }
        }
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

/// The vanilla fog of one fog brush: a screen pass that fogs what lies
/// behind each pixel by the length of the view ray inside the brush, as
/// the fog image does (alpha = sqrt(distance in fog / distanceToOpaque),
/// colour fogparms x identityLight under the x2 overbright). Hidden in the
/// enhanced mode (its volumetric fog).
fn vanilla_fog_quad(b: &crate::bspview::FogBox) -> Gd<godot::classes::MeshInstance3D> {
    use godot::classes::{MeshInstance3D, QuadMesh, Shader, ShaderMaterial};
    let c0 = crate::geom::q2g(&b.mins);
    let c1 = crate::geom::q2g(&b.maxs);
    let (lo, hi) = (c0.coord_min(c1), c0.coord_max(c1));
    let mut sh = Shader::new_gd();
    sh.set_code(VANILLA_FOG_SHADER);
    let mut mat = ShaderMaterial::new_gd();
    mat.set_shader(&sh);
    mat.set_shader_parameter("bmin", &lo.to_variant());
    mat.set_shader_parameter("bmax", &hi.to_variant());
    let lin = |x: f32| x.clamp(0.0, 1.0).powf(R_GAMMA_EXP);
    mat.set_shader_parameter("fog_color", &Vector3::new(lin(b.color[0]), lin(b.color[1]), lin(b.color[2])).to_variant());
    mat.set_shader_parameter("opaque_dist", &b.dist.to_variant());
    mat.set_render_priority(127);
    let mut quad = QuadMesh::new_gd();
    quad.set_size(Vector2::new(2.0, 2.0));
    let mut mi = MeshInstance3D::new_alloc();
    mi.set_mesh(&quad);
    mi.set_material_override(&mat);
    mi.set_cast_shadows_setting(godot::classes::geometry_instance_3d::ShadowCastingSetting::OFF);
    // never culled: it covers the screen wherever the camera is
    mi.set_custom_aabb(godot::builtin::Aabb::new(Vector3::splat(-1.0e5), Vector3::splat(2.0e5)));
    mi.set_name("vanilla_fog");
    mi
}

const VANILLA_FOG_SHADER: &str = r#"
shader_type spatial;
render_mode unshaded, blend_mix, depth_draw_never, depth_test_disabled, cull_disabled, fog_disabled, shadows_disabled;
global uniform float trem_gfx;
uniform sampler2D depth_tex : hint_depth_texture, filter_nearest;
uniform vec3 bmin;
uniform vec3 bmax;
uniform vec3 fog_color;
uniform float opaque_dist = 512.0;
void vertex() {
    POSITION = vec4(VERTEX.xy, 0.5, 1.0);
}
void fragment() {
    if (trem_gfx > 0.5) { discard; }
    float depth = texture(depth_tex, SCREEN_UV).r;
#if CURRENT_RENDERER == RENDERER_COMPATIBILITY
    vec3 ndc = vec3(SCREEN_UV, depth) * 2.0 - 1.0;
#else
    vec3 ndc = vec3(SCREEN_UV * 2.0 - 1.0, depth);
#endif
    vec4 view = INV_PROJECTION_MATRIX * vec4(ndc, 1.0);
    vec3 cam = INV_VIEW_MATRIX[3].xyz;
    // the sky (no depth) is fogged like a wall far away
    float far_d = 2048.0;
    vec3 vp = abs(view.w) > 1e-6 ? view.xyz / view.w : vec3(0.0, 0.0, -far_d);
    vec3 world = (INV_VIEW_MATRIX * vec4(vp, 1.0)).xyz;
    vec3 rd = world - cam;
    float len = min(length(rd), far_d);
    rd = normalize(rd);
    vec3 inv = 1.0 / (rd + vec3(1e-7));
    vec3 t0 = (bmin - cam) * inv;
    vec3 t1 = (bmax - cam) * inv;
    vec3 tmin = min(t0, t1);
    vec3 tmax = max(t0, t1);
    float tn = max(max(tmin.x, tmin.y), max(tmin.z, 0.0));
    float tf = min(min(tmax.x, tmax.y), min(tmax.z, len));
    float d = max(tf - tn, 0.0) * 32.0;
    ALBEDO = fog_color;
    ALPHA = sqrt(clamp(d / max(opaque_dist, 1.0), 0.0, 1.0));
}
"#;
