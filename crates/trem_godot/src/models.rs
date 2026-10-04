//! Tremulous models in Godot: a `Library` that loads MD3 models, skins,
//! animation configs, class/buildable/weapon configs and sounds from the
//! player's pk3 files, and `ModelInst`, an animated MD3 instance.
//!
//! MD3 is vertex animation. Each surface becomes one ArrayMesh plus two
//! float textures (position and normal per vertex per frame); a vertex
//! shader fetches the two frames of `refEntity_t.oldframe/frame` and blends
//! them by `backlerp`, exactly what the Q3 renderer does on the CPU.

use crate::assets::{load_image, Blend, Pk3Set, ShaderInfo};
use crate::geom::{q2g, q2g_dir};
use crate::md3::{self, AnimSet, Md3};
use godot::classes::image::Format as ImageFormat;
use godot::classes::mesh::{ArrayType, PrimitiveType};
use godot::classes::{ArrayMesh, AudioStream, AudioStreamWav, Image, ImageTexture, MeshInstance3D, Node3D, Shader, ShaderMaterial, Texture2D};
use godot::prelude::*;
use std::collections::HashMap;
use std::rc::Rc;

/// The vertex animation lines of VAT_SHADER (left out of the ragdoll shaders).
const VAT_VERTEX: &str = "    vec3 pa = texelFetch(pos_tex, ivec2(VERTEX_ID, frame_a), 0).xyz;
    vec3 pb = texelFetch(pos_tex, ivec2(VERTEX_ID, frame_b), 0).xyz;
    VERTEX = mix(pb, pa, backlerp);
    vec3 na = texelFetch(nrm_tex, ivec2(VERTEX_ID, frame_a), 0).xyz;
    vec3 nb = texelFetch(nrm_tex, ivec2(VERTEX_ID, frame_b), 0).xyz;
    NORMAL = normalize(mix(nb, na, backlerp));
";

const VAT_SHADER: &str = r#"
shader_type spatial;
// r_gamma 1.3 (a common Tremulous setting): gamma-space result -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode cull_disabled, depth_draw_opaque;
#define ENHANCED_LIT
global uniform float trem_gfx;
uniform sampler2D pos_tex : filter_nearest;
uniform sampler2D nrm_tex : filter_nearest;
uniform sampler2D albedo_tex : filter_linear_mipmap, repeat_enable;
// the 4K pack's texture (hdtex.rs), sampled in the enhanced mode
uniform sampler2D albedo_hd : filter_linear_mipmap_anisotropic, repeat_enable;
uniform bool has_hd = false;
uniform vec3 amb = vec3(0.5);
uniform vec3 dcol = vec3(0.5);
uniform vec3 ldir = vec3(0.3, 0.8, 0.5);
uniform int frame_a = 0;
uniform int frame_b = 0;
uniform float backlerp = 0.0;
uniform vec4 tint : source_color = vec4(1.0);
uniform float emission = 0.0;
// enhanced: subsurface scattering (models.rs sss_class) - flesh lets light
// through; sss_skin limits it to skin-coloured texels (shared body textures)
uniform float sss = 0.0;
uniform float sss_depth = 0.1;
uniform bool sss_skin = false;
// the shader script's alphaFunc: without it the renderer ignores the image's
// alpha (telenode_parts.tga / telenode_top.tga carry an all-zero alpha channel)
uniform bool alpha_test = false;
// RF_DEPTHHACK (the view weapon): 1 = off; else view-space positions are
// pulled towards the eye by this factor. The projection of every vertex is
// unchanged (x/z, y/z) but its depth is in front of any wall the player
// touches, like the renderer's glDepthRange( 0, 0.3 ) for these entities.
uniform float depth_hack = 1.0;
// the stages' tcMods (kind 1 scale, 2 scroll, 3 rotate; 0 none), rgbGen
// identity (unlit), and a second, alpha-blended stage over the first
uniform ivec4 tc0_k = ivec4(0);
uniform vec4 tc0_a = vec4(0.0);
uniform vec4 tc0_b = vec4(0.0);
uniform bool lit0 = true;
uniform bool layer2 = false;
uniform sampler2D albedo2_tex : filter_linear_mipmap, repeat_enable;
uniform ivec4 tc1_k = ivec4(0);
uniform vec4 tc1_a = vec4(0.0);
uniform vec4 tc1_b = vec4(0.0);
uniform bool lit1 = true;
// RB_CalcScaleTexCoords / RB_CalcScrollTexCoords / RB_CalcRotateTexCoords
vec2 tcmod(vec2 st, int k, vec2 a, float t) {
    if (k == 1) {
        return st * a;
    } else if (k == 2) {
        return st + fract(a * t);
    } else if (k == 3) {
        float d = radians(-a.x * t);
        float s = sin(d);
        float c = cos(d);
        return vec2(st.x * c - st.y * s + 0.5 - 0.5 * c + 0.5 * s, st.x * s + st.y * c + 0.5 - 0.5 * s - 0.5 * c);
    }
    return st;
}
vec2 tcmods(vec2 st, ivec4 k, vec4 a, vec4 b, float t) {
    st = tcmod(st, k.x, a.xy, t);
    st = tcmod(st, k.y, a.zw, t);
    st = tcmod(st, k.z, b.xy, t);
    return tcmod(st, k.w, b.zw, t);
}
void vertex() {
    vec3 pa = texelFetch(pos_tex, ivec2(VERTEX_ID, frame_a), 0).xyz;
    vec3 pb = texelFetch(pos_tex, ivec2(VERTEX_ID, frame_b), 0).xyz;
    VERTEX = mix(pb, pa, backlerp);
    vec3 na = texelFetch(nrm_tex, ivec2(VERTEX_ID, frame_a), 0).xyz;
    vec3 nb = texelFetch(nrm_tex, ivec2(VERTEX_ID, frame_b), 0).xyz;
    NORMAL = normalize(mix(nb, na, backlerp));
    vec3 view_pos = (MODELVIEW_MATRIX * vec4(VERTEX, 1.0)).xyz;
    POSITION = PROJECTION_MATRIX * vec4(view_pos * depth_hack, 1.0);
}
void fragment() {
#ifdef ENHANCED_LIT
    // the shader writes subsurface outputs (enhanced), so Godot runs its
    // transmittance on every material using it: safe values everywhere else
    // (a zero transmittance depth turns the vanilla models black)
    SSS_STRENGTH = 0.0;
    SSS_TRANSMITTANCE_COLOR = vec4(0.0);
    SSS_TRANSMITTANCE_DEPTH = 1.0;
    SSS_TRANSMITTANCE_BOOST = 0.0;
#endif
    vec2 uv0 = tcmods(UV, tc0_k, tc0_a, tc0_b, TIME);
    vec4 c = ((trem_gfx > 0.5 && has_hd) ? texture(albedo_hd, uv0) : texture(albedo_tex, uv0)) * tint;
    if (alpha_test && c.a < 0.5) { discard; }
    // RB_CalcDiffuseColor: ambient + directed * N.L from the light grid, in
    // gamma space with the x2 hardware overbright, then to linear
    vec3 n = normalize((INV_VIEW_MATRIX * vec4(NORMAL, 0.0)).xyz);
    vec3 l = min(amb + dcol * max(dot(n, ldir), 0.0), vec3(1.0));
    vec3 rgb = lit0 ? c.rgb * l * 2.0 : c.rgb;
    if (layer2) {
        // GL_SRC_ALPHA GL_ONE_MINUS_SRC_ALPHA over the first stage
        vec4 c2 = texture(albedo2_tex, tcmods(UV, tc1_k, tc1_a, tc1_b, TIME)) * tint;
        rgb = mix(rgb, lit1 ? c2.rgb * l * 2.0 : c2.rgb, c2.a);
    }
#ifdef ENHANCED_LIT
    if (trem_gfx < 0.5) {
        // vanilla: tr_shade.c's result, unlit
        ALBEDO = vec3(0.0);
        ROUGHNESS = 1.0;
        SPECULAR = 0.0;
        EMISSION = pow(clamp(rgb + c.rgb * emission, 0.0, 1.0), vec3(R_GAMMA_EXP));
    } else {
        // enhanced: the texture lit by the light grid as irradiance (so
        // SSAO, the ray tracer and dynamic lights act on it), HDR
        vec3 base = c.rgb;
        if (layer2) {
            vec4 c2 = texture(albedo2_tex, tcmods(UV, tc1_k, tc1_a, tc1_b, TIME)) * tint;
            base = mix(base, c2.rgb, c2.a);
        }
        vec3 lin_l = pow(amb * 2.0 + dcol * 2.0 * max(dot(n, ldir), 0.0), vec3(R_GAMMA_EXP));
        if (lit0) {
            ALBEDO = pow(clamp(base, 0.0, 1.0), vec3(R_GAMMA_EXP));
            IRRADIANCE = vec4(lin_l, 1.0);
            // a faint sheen only: a white reflection at the light grid's level
            // lifted the dark alien skins to grey (the "white cast")
            RADIANCE = vec4(lin_l * 0.05, 1.0);
            EMISSION = pow(clamp(c.rgb * emission, 0.0, 1.0), vec3(R_GAMMA_EXP));
            float k = sss;
            if (sss_skin) {
                // skin: red over green over blue, warm, not grey, not dark
                vec3 g = clamp(base, 0.0, 1.0);
                float mx = max(g.r, max(g.g, g.b));
                float sat = (mx - min(g.r, min(g.g, g.b))) / max(mx, 1e-3);
                k *= smoothstep(0.04, 0.12, g.r - g.b) * step(g.b, g.g + 0.02) * smoothstep(0.15, 0.3, g.r) * (1.0 - smoothstep(0.6, 0.8, sat));
            }
            SSS_STRENGTH = k;
            // light through thin parts takes the surface's own colour, warmed
            SSS_TRANSMITTANCE_COLOR = vec4(clamp(ALBEDO * 1.6 + vec3(0.12, 0.02, 0.0), 0.0, 1.0), 1.0);
            SSS_TRANSMITTANCE_DEPTH = sss_depth;
            SSS_TRANSMITTANCE_BOOST = 0.25 * k;
            // r_modelDebug (gfx.rs set_global): 1 no SSS, 2 no reflections, 3 both
            int mdbg = int(round((trem_gfx - 1.0) * 100.0));
            if (mdbg == 1 || mdbg == 3) {
                SSS_STRENGTH = 0.0;
                SSS_TRANSMITTANCE_BOOST = 0.0;
            }
            if (mdbg == 2 || mdbg == 3) {
                RADIANCE = vec4(0.0);
            }
        } else {
            ALBEDO = vec3(0.0);
            EMISSION = pow(clamp(rgb + c.rgb * emission, 0.0, 1.0), vec3(R_GAMMA_EXP)) * 1.4;
        }
        ROUGHNESS = 0.7;
        SPECULAR = lit0 ? 0.2 : 0.0;
        int mdbg2 = int(round((trem_gfx - 1.0) * 100.0));
        if (mdbg2 == 2 || mdbg2 == 3) {
            SPECULAR = 0.0;
        }
    }
#else
    ALBEDO = pow(clamp(rgb + c.rgb * emission, 0.0, 1.0), vec3(R_GAMMA_EXP));
#endif
}
"#;

/// View-space scale of RF_DEPTHHACK models: 0.3 like the C's depth range
/// (0, 0.3); the hand models start ~4 units (r_znear) from the eye, which
/// stays beyond the camera's near plane (0.03 m = ~1 unit).
const DEPTH_HACK_SCALE: f32 = 0.3;

pub struct SurfRes {
    /// the surface's index in the MD3 (empty surfaces have no SurfRes)
    pub md3_index: usize,
    pub name: String,
    pub shader: String,
    pub mesh: Gd<ArrayMesh>,
    pub pos: Gd<ImageTexture>,
    pub nrm: Gd<ImageTexture>,
}

pub struct ModelRes {
    pub md3: Md3,
    pub surfs: Vec<SurfRes>,
}

#[derive(Clone, Debug, Default)]
pub struct BuildableCfg {
    pub models: [Option<String>; 4],
    pub scale: f32,
    pub z_offset: f32,
}

#[derive(Clone, Debug, Default)]
pub struct WeaponCfg {
    pub model: Option<String>,
    /// per mode (primary, secondary, tertiary): flash, impact, impact-flesh sounds.
    pub flash: [Vec<String>; 3],
    pub impact: [Vec<String>; 3],
    pub impact_flesh: [Vec<String>; 3],
    pub missile_model: [Option<String>; 3],
    /// (radius in Quake units, shader/image)
    pub missile_sprite: [Option<(f32, String)>; 3],
    pub flash_dlight: [Option<[f32; 3]>; 3],
    pub missile_dlight: [Option<[f32; 3]>; 3],
    /// particle / trail systems per mode (cg_weapons.c CG_ParseWeaponModeSection)
    pub missile_ps: [Option<String>; 3],
    pub missile_ts: [Option<String>; 3],
    pub muzzle_ps: [Option<String>; 3],
    pub impact_ps: [Option<String>; 3],
    pub always_impact: [bool; 3],
    /// continuousFlash: the flash model stays up while firing
    pub continuous_flash: [bool; 3],
    /// disableIn3rdPerson
    pub disable_in_3rd_person: bool,
    /// impactMark <size> <shader>
    pub impact_mark: [Option<(f32, String)>; 3],
    /// models/weapons/<name>/animation.cfg (`CG_ParseWeaponAnimationFile`):
    /// None when absent (the C keeps zeroed animations: frame 0)
    pub anims: Option<Rc<AnimSet>>,
    /// its noDrift flag (`CG_CalculateWeaponPosition`)
    pub no_drift: bool,
}

pub struct Library {
    pub pk3: Pk3Set,
    shader_images: HashMap<String, ShaderInfo>,
    textures: HashMap<String, Option<Gd<Texture2D>>>,
    models: HashMap<String, Option<Rc<ModelRes>>>,
    skins: HashMap<String, Rc<HashMap<String, String>>>,
    anims: HashMap<String, Option<Rc<AnimSet>>>,
    sounds: HashMap<String, Option<Gd<AudioStream>>>,
    weapons: HashMap<String, Rc<WeaponCfg>>,
    buildables: HashMap<String, Rc<BuildableCfg>>,
    vat: Gd<Shader>,
    vat_add: Gd<Shader>,
    vat_mix: Gd<Shader>,
    /// the same three for ragdolls (ragdoll.rs): the skinned mesh's own
    /// vertices instead of the animation textures
    rag: Gd<Shader>,
    rag_add: Gd<Shader>,
    rag_mix: Gd<Shader>,
    white: Gd<Texture2D>,
    default_tex: Option<Gd<Texture2D>>,
}

impl Library {
    /// None when the search path has no Tremulous data pk3.
    pub fn open(paths: &[String]) -> Option<Self> {
        let mut pk3 = Pk3Set::open(paths);
        if pk3.read("models/players/human_base/lower.md3").is_none() {
            return None;
        }
        let shader_images = pk3.shader_images();
        let mk = |code: String| {
            let mut sh = Shader::new_gd();
            sh.set_code(&code);
            sh
        };
        let vat = mk(VAT_SHADER.to_string());
        let vat_add = mk(VAT_SHADER.replace("render_mode cull_disabled, depth_draw_opaque;\n#define ENHANCED_LIT", "render_mode cull_disabled, unshaded, blend_add, depth_draw_never;").replace("if (alpha_test && c.a < 0.5) { discard; }", "").replace("min(amb + dcol * max(dot(n, ldir), 0.0), vec3(1.0))", "vec3(0.5)"));
        let vat_mix = mk(VAT_SHADER.replace("render_mode cull_disabled, depth_draw_opaque;\n#define ENHANCED_LIT", "render_mode cull_disabled, unshaded, blend_mix, depth_draw_never;").replace("if (alpha_test && c.a < 0.5) { discard; }", "ALPHA = c.a;"));
        let white = crate::assets::flat_texture(Color::from_rgb(1.0, 1.0, 1.0)).upcast::<Texture2D>();
        let rag_of = |sh: &Gd<Shader>| mk(sh.get_code().to_string().replace(VAT_VERTEX, ""));
        let (rag, rag_add, rag_mix) = (rag_of(&vat), rag_of(&vat_add), rag_of(&vat_mix));
        Some(Library { pk3, shader_images, vat_add, vat_mix, rag, rag_add, rag_mix, textures: HashMap::new(), models: HashMap::new(), skins: HashMap::new(), anims: HashMap::new(), sounds: HashMap::new(), weapons: HashMap::new(), buildables: HashMap::new(), vat, white, default_tex: None })
    }

    pub fn text(&mut self, path: &str) -> Option<String> {
        self.pk3.read(path).map(|d| String::from_utf8_lossy(d.as_slice()).into_owned())
    }

    /// An image by shader or file name (shader scripts first, then .tga/.jpg/.png).
    /// Shader-script info for a shader/image name (extension-less lookup).
    pub fn shader_info(&self, name: &str) -> ShaderInfo {
        let key = name.to_lowercase().replace('\\', "/");
        let stem = key.trim_end_matches(".tga").trim_end_matches(".jpg");
        self.shader_images.get(stem).cloned().unwrap_or_default()
    }

    /// Whether a shader script defines `name` (else the renderer looks for
    /// an image file of that name and, failing that, uses its default shader).
    pub fn shader_defined(&self, name: &str) -> bool {
        let key = name.to_lowercase().replace('\\', "/");
        let stem = key.trim_end_matches(".tga").trim_end_matches(".jpg");
        self.shader_images.contains_key(stem)
    }

    /// The renderer's default image (`R_CreateDefaultImage`).
    pub fn default_texture(&mut self) -> Gd<Texture2D> {
        self.default_tex.get_or_insert_with(|| crate::assets::default_texture().upcast::<Texture2D>()).clone()
    }

    pub fn texture(&mut self, name: &str) -> Option<Gd<Texture2D>> {
        let name = &name.replace('\\', "/");
        let key = name.to_lowercase();
        if let Some(t) = self.textures.get(&key) {
            return t.clone();
        }
        let t = if key.starts_with("gfx/null") || key.is_empty() {
            None
        } else {
            let stem = key.trim_end_matches(".tga").trim_end_matches(".jpg").to_string();
            let img = self.shader_images.get(&stem).and_then(|i| i.image.clone()).and_then(|p| load_image(&mut self.pk3, &p)).or_else(|| load_image(&mut self.pk3, name));
            img.and_then(|i| ImageTexture::create_from_image(&i)).map(|t| t.upcast::<Texture2D>())
        };
        self.textures.insert(key, t.clone());
        t
    }

    /// The 4K pack's texture of an image / shader name (hdtex.rs).
    pub fn texture_hd(&mut self, name: &str) -> Option<Gd<ImageTexture>> {
        let key = name.to_lowercase().replace('\\', "/");
        let stem = key.trim_end_matches(".tga").trim_end_matches(".jpg").to_string();
        let img = self.shader_images.get(&stem).and_then(|i| i.image.clone());
        img.and_then(|p| crate::hdtex::get(&crate::hdtex::key_of(&p))).or_else(|| crate::hdtex::get(&crate::hdtex::key_of(&stem))).map(|h| h.albedo)
    }

    pub fn skin(&mut self, path: &str) -> Rc<HashMap<String, String>> {
        if let Some(s) = self.skins.get(path) {
            return s.clone();
        }
        let s = Rc::new(self.text(path).map(|t| md3::parse_skin(&t)).unwrap_or_default());
        self.skins.insert(path.to_string(), s.clone());
        s
    }

    pub fn model(&mut self, path: &str) -> Option<Rc<ModelRes>> {
        if let Some(m) = self.models.get(path) {
            return m.clone();
        }
        let m = self.pk3.read(path).and_then(|d| match md3::parse(d.as_slice()) {
            Ok(m) => Some(Rc::new(build_res(m))),
            Err(e) => {
                godot_warn!("{path}: {e}");
                None
            }
        });
        self.models.insert(path.to_string(), m.clone());
        m
    }

    pub fn player_anims(&mut self, model_dir: &str) -> Option<Rc<AnimSet>> {
        let key = format!("p:{model_dir}");
        if let Some(a) = self.anims.get(&key) {
            return a.clone();
        }
        let a = self.text(&format!("models/players/{model_dir}/animation.cfg")).and_then(|t| md3::parse_player_anims(&t).ok()).map(Rc::new);
        self.anims.insert(key, a.clone());
        a
    }

    pub fn buildable_anims(&mut self, name: &str) -> Option<Rc<AnimSet>> {
        let key = format!("b:{name}");
        if let Some(a) = self.anims.get(&key) {
            return a.clone();
        }
        let a = self.text(&format!("models/buildables/{name}/animation.cfg")).and_then(|t| md3::parse_buildable_anims(&t).ok()).map(Rc::new);
        self.anims.insert(key, a.clone());
        a
    }

    /// overrides/buildables/<name>.cfg (model 0..3, modelScale, zOffset).
    pub fn buildable_cfg(&mut self, name: &str) -> Rc<BuildableCfg> {
        if let Some(c) = self.buildables.get(name) {
            return c.clone();
        }
        let mut cfg = BuildableCfg { scale: 1.0, ..Default::default() };
        if let Some(t) = self.text(&format!("overrides/buildables/{name}.cfg")).or_else(|| self.text(&format!("configs/buildables/{name}.cfg"))) {
            for l in md3::parse_kv(&t) {
                match l[0].to_lowercase().as_str() {
                    "model" if l.len() >= 3 => {
                        if let Ok(i) = l[1].parse::<usize>() {
                            if i < 4 {
                                cfg.models[i] = Some(l[2].clone());
                            }
                        }
                    }
                    "modelscale" => cfg.scale = l.get(1).and_then(|x| x.parse().ok()).unwrap_or(1.0),
                    "zoffset" => cfg.z_offset = l.get(1).and_then(|x| x.parse().ok()).unwrap_or(0.0),
                    _ => {}
                }
            }
        }
        let cfg = Rc::new(cfg);
        self.buildables.insert(name.to_string(), cfg.clone());
        cfg
    }

    /// models/weapons/<name>/weapon.cfg (cg_weapons.c CG_ParseWeaponFile, the parts used here).
    pub fn weapon_cfg(&mut self, name: &str) -> Rc<WeaponCfg> {
        if let Some(c) = self.weapons.get(name) {
            return c.clone();
        }
        let mut cfg = WeaponCfg::default();
        if let Some(t) = self.text(&format!("models/weapons/{name}/weapon.cfg")) {
            let mut mode: Option<usize> = None;
            for l in md3::parse_kv(&t) {
                let key = l[0].to_lowercase();
                match key.as_str() {
                    "weaponmodel" => cfg.model = l.get(1).cloned(),
                    "primary" => mode = Some(0),
                    "secondary" => mode = Some(1),
                    "tertiary" => mode = Some(2),
                    "}" => mode = None,
                    "missilemodel" => {
                        if let (Some(md), Some(p)) = (mode, l.get(1)) {
                            cfg.missile_model[md] = Some(p.clone());
                        }
                    }
                    "missileparticlesystem" | "missiletrailsystem" | "muzzleparticlesystem" | "impactparticlesystem" => {
                        if let (Some(md), Some(p)) = (mode, l.get(1)) {
                            let slot = match key.as_str() {
                                "missileparticlesystem" => &mut cfg.missile_ps,
                                "missiletrailsystem" => &mut cfg.missile_ts,
                                "muzzleparticlesystem" => &mut cfg.muzzle_ps,
                                _ => &mut cfg.impact_ps,
                            };
                            slot[md] = Some(p.clone());
                        }
                    }
                    "impactmark" => {
                        if let (Some(md), Some(sz), Some(p)) = (mode, l.get(1).map(|x| trem_sim::kernel::spawn::c_atoi(x).max(0) as f32), l.get(2)) {
                            cfg.impact_mark[md] = Some((sz, p.clone()));
                        }
                    }
                    "alwaysimpact" => {
                        if let Some(md) = mode {
                            cfg.always_impact[md] = true;
                        }
                    }
                    "continuousflash" => {
                        if let Some(md) = mode {
                            cfg.continuous_flash[md] = true;
                        }
                    }
                    "disablein3rdperson" => cfg.disable_in_3rd_person = true,
                    "missilesprite" => {
                        if let (Some(md), Some(sz), Some(p)) = (mode, l.get(1).and_then(|x| x.parse::<f32>().ok()), l.get(2)) {
                            cfg.missile_sprite[md] = Some((sz, p.clone()));
                        }
                    }
                    "flashdlightcolor" | "missiledlightcolor" => {
                        if let Some(md) = mode {
                            let c: Vec<f32> = l[1..].iter().filter_map(|x| x.parse().ok()).collect();
                            if c.len() == 3 {
                                if key == "flashdlightcolor" {
                                    cfg.flash_dlight[md] = Some([c[0], c[1], c[2]]);
                                } else {
                                    cfg.missile_dlight[md] = Some([c[0], c[1], c[2]]);
                                }
                            }
                        }
                    }
                    "flashsound" | "impactsound" | "impactfleshsound" => {
                        if let (Some(m), Some(p)) = (mode, l.last()) {
                            let list = match key.as_str() {
                                "flashsound" => &mut cfg.flash[m],
                                "impactsound" => &mut cfg.impact[m],
                                _ => &mut cfg.impact_flesh[m],
                            };
                            list.push(p.clone());
                        }
                    }
                    _ => {}
                }
                if l.iter().any(|x| x == "{") && mode.is_none() {
                    // "primary {" on one line handled above; stray braces ignored
                }
            }
        }
        // CG_RegisterWeapon: the weapon's animation.cfg
        match self.text(&format!("models/weapons/{name}/animation.cfg")).map(|t| md3::parse_weapon_anims(&t)) {
            Some(Ok((a, nd))) => {
                cfg.anims = Some(Rc::new(a));
                cfg.no_drift = nd;
            }
            Some(Err(e)) => godot_warn!("models/weapons/{name}/animation.cfg: {e}"),
            None => {}
        }
        let cfg = Rc::new(cfg);
        self.weapons.insert(name.to_string(), cfg.clone());
        cfg
    }

    pub fn sound(&mut self, path: &str) -> Option<Gd<AudioStream>> {
        let key = path.to_lowercase();
        if let Some(s) = self.sounds.get(&key) {
            return s.clone();
        }
        let s = self.pk3.read(path).and_then(|d| AudioStreamWav::load_from_buffer(&d)).map(|s| s.upcast::<AudioStream>());
        self.sounds.insert(key, s.clone());
        s
    }

    pub fn has(&self, path: &str) -> bool {
        self.pk3.contains(path)
    }

    /// A model material's ragdoll copy: the same uniforms, the ragdoll
    /// variant of its shader.
    pub fn ragdoll_material(&self, m: &Gd<ShaderMaterial>) -> Gd<ShaderMaterial> {
        let mut c = m.duplicate_resource();
        let sh = m.get_shader();
        let rag = if sh.as_ref() == Some(&self.vat_add) {
            &self.rag_add
        } else if sh.as_ref() == Some(&self.vat_mix) {
            &self.rag_mix
        } else {
            &self.rag
        };
        c.set_shader(rag);
        c
    }

    /// Instance a model with an optional skin (surface name -> image).
    pub fn instance(&mut self, res: &Rc<ModelRes>, skin: Option<&HashMap<String, String>>) -> ModelInst {
        let mut root = Node3D::new_alloc();
        let mut mis = Vec::new();
        let mut mis_surf = Vec::new();
        for s in &res.surfs {
            let img = skin.and_then(|sk| sk.get(&s.name.to_lowercase()).cloned()).unwrap_or_else(|| s.shader.clone());
            if img.to_lowercase().starts_with("gfx/null") {
                continue;
            }
            let info = self.shader_info(&img);
            if info.nodraw {
                continue;
            }
            // a skin/surface shader in no script and no image file: the
            // renderer's default shader (its "*default" image)
            let tex = match self.texture(&img) {
                Some(t) => t,
                None if !self.shader_defined(&img) => self.default_texture(),
                None => self.white.clone(),
            };
            let mut mat = ShaderMaterial::new_gd();
            mat.set_shader(match info.blend {
                Blend::Add => &self.vat_add,
                Blend::Mix => &self.vat_mix,
                Blend::Opaque => &self.vat,
            });
            mat.set_shader_parameter("pos_tex", &s.pos.to_variant());
            mat.set_shader_parameter("alpha_test", &info.alpha_test.to_variant());
            mat.set_shader_parameter("nrm_tex", &s.nrm.to_variant());
            mat.set_shader_parameter("albedo_tex", &tex.to_variant());
            if let Some(hd) = self.texture_hd(&img) {
                mat.set_shader_parameter("albedo_hd", &hd.to_variant());
                mat.set_shader_parameter("has_hd", &true.to_variant());
            }
            if let Some(c) = sss_class(&img) {
                mat.set_shader_parameter("sss", &c.strength.to_variant());
                mat.set_shader_parameter("sss_depth", &c.depth.to_variant());
                mat.set_shader_parameter("sss_skin", &c.skin_only.to_variant());
            }
            // the first image stage's tcMods / rgbGen, and a blended second
            // stage over an opaque first (level2adv: the scrolling electric
            // pattern showing through adv.tga's transparent veins)
            if let Some(st0) = info.stages.first() {
                set_tcmods(&mut mat, "tc0", &st0.tcmods);
                if st0.unlit && info.blend != Blend::Add {
                    mat.set_shader_parameter("lit0", &false.to_variant());
                }
                if let Some(st1) = info.stages.get(1).filter(|t| st0.blend == Blend::Opaque && t.blend == Blend::Mix) {
                    if let Some(t2) = self.texture(&st1.image) {
                        mat.set_shader_parameter("layer2", &true.to_variant());
                        mat.set_shader_parameter("albedo2_tex", &t2.to_variant());
                        set_tcmods(&mut mat, "tc1", &st1.tcmods);
                        mat.set_shader_parameter("lit1", &(!st1.unlit).to_variant());
                    }
                }
            }
            let mut mi = MeshInstance3D::new_alloc();
            mi.set_mesh(&s.mesh);
            mi.set_material_override(&mat);
            root.add_child(&mi);
            mis.push((mi, mat));
            mis_surf.push(s.md3_index);
        }
        ModelInst { root, mis, mis_surf, res: res.clone(), last: (-1, -1, -1.0) }
    }
}

impl Pk3Set {
    pub fn contains(&self, path: &str) -> bool {
        self.has_file(path)
    }
}

fn build_res(m: Md3) -> ModelRes {
    let (mn, mx) = m.all_bounds();
    let a = q2g(&mn);
    let b = q2g(&mx);
    let lo = Vector3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
    let hi = Vector3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
    let aabb = Aabb::new(lo - Vector3::splat(0.1), hi - lo + Vector3::splat(0.2));
    let mut surfs = Vec::new();
    for (md3_index, s) in m.surfaces.iter().enumerate() {
        let nv = s.st.len();
        if nv == 0 || s.triangles.is_empty() {
            continue;
        }
        let frames = m.num_frames;
        let mut pos_bytes = Vec::with_capacity(nv * frames * 12);
        let mut nrm_bytes = Vec::with_capacity(nv * frames * 12);
        for f in 0..frames {
            for k in 0..nv {
                let p = q2g(&s.xyz[f][k]);
                let n = q2g_dir(&s.normals[f][k]);
                for x in [p.x, p.y, p.z] {
                    pos_bytes.extend_from_slice(&x.to_le_bytes());
                }
                for x in [n.x, n.y, n.z] {
                    nrm_bytes.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        let pos_img = Image::create_from_data(nv as i32, frames as i32, false, ImageFormat::RGBF, &PackedByteArray::from(pos_bytes.as_slice()));
        let nrm_img = Image::create_from_data(nv as i32, frames as i32, false, ImageFormat::RGBF, &PackedByteArray::from(nrm_bytes.as_slice()));
        let (Some(pi), Some(ni)) = (pos_img, nrm_img) else { continue };
        let (Some(pos), Some(nrm)) = (ImageTexture::create_from_image(&pi), ImageTexture::create_from_image(&ni)) else { continue };
        let verts: PackedVector3Array = s.xyz[0].iter().map(q2g).collect();
        let normals: PackedVector3Array = s.normals[0].iter().map(q2g_dir).collect();
        let uvs: PackedVector2Array = s.st.iter().map(|t| Vector2::new(t[0], t[1])).collect();
        // Q3 triangles are clockwise from the front, like Godot's.
        let idx: PackedInt32Array = s.triangles.iter().flat_map(|t| [t[0] as i32, t[1] as i32, t[2] as i32]).collect();
        let mut arrays = VarArray::new();
        arrays.resize(ArrayType::MAX.ord() as usize, &Variant::nil());
        arrays.set(ArrayType::VERTEX.ord() as usize, &verts.to_variant());
        arrays.set(ArrayType::NORMAL.ord() as usize, &normals.to_variant());
        arrays.set(ArrayType::TEX_UV.ord() as usize, &uvs.to_variant());
        arrays.set(ArrayType::INDEX.ord() as usize, &idx.to_variant());
        let mut mesh = ArrayMesh::new_gd();
        mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);
        mesh.set_custom_aabb(aabb);
        surfs.push(SurfRes { md3_index, name: s.name.clone(), shader: s.shaders.first().cloned().unwrap_or_default(), mesh, pos, nrm });
    }
    ModelRes { md3: m, surfs }
}

/// One animated MD3 in the scene.
pub struct ModelInst {
    pub root: Gd<Node3D>,
    mis: Vec<(Gd<MeshInstance3D>, Gd<ShaderMaterial>)>,
    /// the MD3 surface of each entry of `mis`
    mis_surf: Vec<usize>,
    pub res: Rc<ModelRes>,
    last: (i32, i32, f32),
}

impl ModelInst {
    pub fn mesh_count(&self) -> usize {
        self.mis.len()
    }
    /// The frames drawn now: (old, new, backlerp), as `set_frames` clamped them.
    pub fn frames(&self) -> (i32, i32, f32) {
        self.last
    }
    /// The surfaces drawn (MD3 surface index, material).
    pub fn drawn(&self) -> impl Iterator<Item = (usize, &Gd<ShaderMaterial>)> {
        self.mis_surf.iter().copied().zip(self.mis.iter().map(|(_, m)| m))
    }
}

impl ModelInst {
    pub fn set_frames(&mut self, old: i32, new: i32, backlerp: f32) {
        let n = self.res.md3.num_frames as i32;
        let old = old.clamp(0, n - 1);
        let new = new.clamp(0, n - 1);
        if self.last == (old, new, backlerp) {
            return;
        }
        self.last = (old, new, backlerp);
        for (_, m) in self.mis.iter_mut() {
            m.set_shader_parameter("frame_a", &old.to_variant());
            m.set_shader_parameter("frame_b", &new.to_variant());
            m.set_shader_parameter("backlerp", &backlerp.to_variant());
        }
    }
    /// Light-grid lighting at the entity (ambient, directed, direction in Godot space).
    pub fn set_light(&mut self, amb: [f32; 3], dcol: [f32; 3], ldir: Vector3) {
        let a = Vector3::new(amb[0], amb[1], amb[2]);
        let d = Vector3::new(dcol[0], dcol[1], dcol[2]);
        for (_, m) in self.mis.iter_mut() {
            m.set_shader_parameter("amb", &a.to_variant());
            m.set_shader_parameter("dcol", &d.to_variant());
            m.set_shader_parameter("ldir", &ldir.to_variant());
        }
    }
    /// `RF_DEPTHHACK` (see the shader's depth_hack).
    pub fn set_depth_hack(&mut self, on: bool) {
        for (mi, m) in self.mis.iter_mut() {
            // RF_FIRST_PERSON models cast no shadows
            mi.set_cast_shadows_setting(if on { godot::classes::geometry_instance_3d::ShadowCastingSetting::OFF } else { godot::classes::geometry_instance_3d::ShadowCastingSetting::ON });
            m.set_shader_parameter("depth_hack", &(if on { DEPTH_HACK_SCALE } else { 1.0f32 }).to_variant());
        }
    }
    pub fn set_tint(&mut self, c: Color, emission: f32) {
        for (_, m) in self.mis.iter_mut() {
            m.set_shader_parameter("tint", &c.to_variant());
            m.set_shader_parameter("emission", &emission.to_variant());
        }
    }
    /// Model-local transform of a tag at the current frames (`CG_PositionEntityOnTag`).
    pub fn tag(&self, name: &str) -> Option<Transform3D> {
        let (old, new, back) = self.last;
        let t = self.res.md3.lerp_tag(name, old.max(0) as usize, new.max(0) as usize, back.max(0.0))?;
        Some(Transform3D::new(tag_basis(&t.axis), q2g(&t.origin)))
    }
    pub fn set_transform(&mut self, t: Transform3D) {
        self.root.set_transform(t);
    }
    pub fn set_visible(&mut self, v: bool) {
        self.root.set_visible(v);
    }
    pub fn free(mut self) {
        self.root.queue_free();
    }
}

/// Quake axis[3] (forward, left, up rows) -> Godot basis (see geom.rs mapping).
/// A stage's first four tcMods as the VAT shader's `<p>_k/_a/_b` uniforms.
fn set_tcmods(mat: &mut Gd<ShaderMaterial>, p: &str, tc: &[crate::assets::TcMod]) {
    let mut k = [0i32; 4];
    let mut a = [0f32; 8];
    for (i, t) in tc.iter().take(4).enumerate() {
        k[i] = t.kind;
        a[i * 2] = t.a[0];
        a[i * 2 + 1] = t.a[1];
    }
    mat.set_shader_parameter(&format!("{p}_k"), &Vector4i::new(k[0], k[1], k[2], k[3]).to_variant());
    mat.set_shader_parameter(&format!("{p}_a"), &Vector4::new(a[0], a[1], a[2], a[3]).to_variant());
    mat.set_shader_parameter(&format!("{p}_b"), &Vector4::new(a[4], a[5], a[6], a[7]).to_variant());
}

pub fn tag_basis(axis: &[[f32; 3]; 3]) -> Basis {
    Basis::from_cols(q2g_dir(&axis[0]), q2g_dir(&axis[2]), -q2g_dir(&axis[1]))
}

/// `AnglesToAxis` as a model basis (model +x forward).
pub fn model_basis(angles: &[f32; 3]) -> Basis {
    let (f, r, u) = trem_sim::kernel::qmath::angle_vectors(angles);
    let left = [-r[0], -r[1], -r[2]];
    tag_basis(&[f, left, u])
}

/// Enhanced-mode subsurface scattering of a model surface (by its texture):
/// alien bodies and structures are flesh; on humans only the skin (the head,
/// and the bare arms of the shared body texture).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SssClass {
    pub strength: f32,
    /// transmittance depth (Godot units): thicker flesh lets less through
    pub depth: f32,
    pub skin_only: bool,
}

pub fn sss_class(texture: &str) -> Option<SssClass> {
    let t = texture.to_ascii_lowercase().replace('\\', "/");
    const ALIEN_BUILD: &[&str] = &["overmind", "eggpod", "acid_tube", "trapper", "booster", "hive", "hovel", "barricade", "creep"];
    if t.starts_with("models/players/human") {
        // the light-armour body; the battle suit (human_bsuit) is all armour
        return match t.rsplit('/').next().unwrap_or("") {
            n if n.starts_with("h_base") => Some(SssClass { strength: 0.55, depth: 0.08, skin_only: true }),
            n if n.starts_with("base") && t.contains("human_base") => Some(SssClass { strength: 0.5, depth: 0.06, skin_only: true }),
            _ => None,
        };
    }
    if t.starts_with("models/players/") {
        // dretch .. tyrant and the granger: flesh all over
        let big = t.contains("level4") || t.contains("level3");
        return Some(SssClass { strength: 0.7, depth: if big { 0.25 } else { 0.12 }, skin_only: false });
    }
    if t.starts_with("models/buildables/") && ALIEN_BUILD.iter().any(|b| t.contains(b)) {
        return Some(SssClass { strength: 0.6, depth: 0.3, skin_only: false });
    }
    None
}

#[cfg(test)]
mod sss_tests {
    use super::sss_class;

    #[test]
    fn flesh_gets_subsurface_scattering() {
        assert!(sss_class("models/players/level2/level2").is_some_and(|c| !c.skin_only));
        assert!(sss_class("models/players/builder/builder.tga").is_some());
        assert!(sss_class("models/buildables/overmind/overmind.tga").is_some());
        assert!(sss_class("models/buildables/eggpod/pod_strands.tga").is_some());
        assert!(sss_class("models/players/human_base/h_base").is_some_and(|c| c.skin_only));
        assert!(sss_class("models/players/human_base/base.tga").is_some_and(|c| c.skin_only));
    }

    #[test]
    fn armour_and_human_structures_do_not() {
        assert_eq!(sss_class("models/players/human_bsuit/human_bsuit.tga"), None);
        assert_eq!(sss_class("models/players/human_base/armour.tga"), None);
        assert_eq!(sss_class("models/buildables/reactor/reactor.tga"), None);
        assert_eq!(sss_class("models/buildables/mgturret/turret.tga"), None);
        assert_eq!(sss_class("models/weapons/rifle/rifle.tga"), None);
    }
}

#[cfg(test)]
mod ragdoll_shader_tests {
    #[test]
    fn the_ragdoll_shaders_drop_the_vertex_animation() {
        // Library::open builds the ragdoll shaders by removing VAT_VERTEX
        assert!(super::VAT_SHADER.contains(super::VAT_VERTEX));
        let rag = super::VAT_SHADER.replace(super::VAT_VERTEX, "");
        assert!(!rag.contains("texelFetch(pos_tex") && rag.contains("void vertex()") && rag.contains("POSITION ="));
    }
}
