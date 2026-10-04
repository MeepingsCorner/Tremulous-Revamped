//! Rendering IBSP maps from the player's own pk3 files (dec003): pk3 access
//! through Godot's ZIPReader, shader -> texture resolution, patch
//! subdivision (`R_SubdividePatchToGrid`) and lightmapped meshes. Presentation only — collision comes
//! from `trem_sim::kernel::bsp`.

use godot::classes::image::Format as ImageFormat;
use godot::classes::mesh::PrimitiveType;
use godot::classes::{Image, ImageTexture, MeshInstance3D, Shader, ShaderMaterial, SurfaceTool};
use crate::assets::{flat_texture, load_image, Pk3Set};
use godot::prelude::*;
use std::collections::HashMap;
use trem_sim::kernel::bsp::{Bsp, DrawVert, MST_PATCH, MST_PLANAR, MST_TRIANGLE_SOUP};

use crate::geom::{q2g, q2g_dir};

const SURF_SKY: i32 = 0x4;
const SURF_NODRAW: i32 = 0x80;

const WORLD_SHADER: &str = r#"
shader_type spatial;
// trem_gfx 0: vanilla, the original renderer's result as emission (no
// lighting, no post effects); 1: enhanced, the same texture and baked light
// as a lit surface: the light map is the irradiance (SSAO / SSIL / the ray
// tracer act on it), a generated normal map bumps it along the light grid's
// dominant direction (CUSTOM0: direction xyz, directed fraction w).
// r_gamma 1.3 (a common Tremulous setting): gamma-space result -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode cull_disabled;
global uniform float trem_gfx;
uniform sampler2D tex : filter_linear_mipmap_anisotropic, repeat_enable;
// the 4K pack's texture (enhanced mode only)
uniform sampler2D tex_hd : filter_linear_mipmap_anisotropic, repeat_enable;
uniform bool has_hd = false;
uniform sampler2D lm : filter_linear;
uniform bool use_lm = false;
uniform vec4 tint = vec4(1.0);
uniform sampler2D nrm : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform bool has_nrm = false;
uniform float rough = 0.7;
uniform float spec = 0.4;
uniform float bump = 1.0;
uniform float irr_scale = 1.0;
// enhanced materials (gfx.rs surface_class): metalness, a wet clearcoat,
// and the close-range detail normal map that stands in for parallax mapping
uniform float metal = 0.0;
uniform float wet = 0.0;
uniform float detail_k = 0.0;
uniform sampler2D detail : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
varying vec4 grid;
void vertex() {
    grid = CUSTOM0;
}
void fragment() {
    vec4 t = ((trem_gfx > 0.5 && has_hd) ? texture(tex_hd, UV) : texture(tex, UV)) * tint;
    if (t.a < 0.5) { discard; }
    vec3 light = use_lm ? texture(lm, UV2).rgb : COLOR.rgb;
    // R_ColorShiftLightingBytes (mapOverBrightBits 2 - overBrightBits 1 = x2,
    // hue-preserving clamp), then the x2 hardware overbright; all in gamma space.
    light *= 2.0;
    float m = max(light.r, max(light.g, light.b));
    if (m > 1.0) { light /= m; }
    if (trem_gfx < 0.5) {
        ALBEDO = vec3(0.0);
        ROUGHNESS = 1.0;
        SPECULAR = 0.0;
        METALLIC = 0.0;
        EMISSION = pow(clamp(t.rgb * light * 2.0, 0.0, 1.0), vec3(R_GAMMA_EXP));
    } else {
        vec3 alb = pow(clamp(t.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP));
        // the overbright is kept (HDR, tone mapped) instead of clamped
        vec3 baked = pow(light * 2.0, vec3(R_GAMMA_EXP));
        float k = 1.0;
        if (has_nrm) {
            vec3 tn = texture(nrm, UV).xyz;
            vec3 nm = tn * 2.0 - 1.0;
            nm.xy *= bump;
            vec3 nb = normalize(TANGENT * nm.x + BINORMAL * nm.y + NORMAL * nm.z);
            vec3 L = normalize((VIEW_MATRIX * vec4(grid.xyz, 0.0)).xyz + vec3(1e-5));
            float d = clamp(grid.w, 0.0, 0.85);
            float lit_b = (1.0 - d) + d * max(dot(nb, L), 0.0);
            float lit_g = (1.0 - d) + d * max(dot(NORMAL, L), 0.0);
            k = clamp(lit_b / max(lit_g, 0.05), 0.45, 1.7);
            NORMAL_MAP = tn;
            NORMAL_MAP_DEPTH = bump;
        }
        // close range: fine relief (whiteout blend over the surface's own
        // normal map), fading out by 12 units so distant walls stay calm
        float near = 1.0 - smoothstep(4.0, 12.0, length(VERTEX));
        if (detail_k > 0.0 && near > 0.0) {
            vec3 base_n = has_nrm ? texture(nrm, UV).xyz * 2.0 - 1.0 : vec3(0.0, 0.0, 1.0);
            vec3 dn = texture(detail, UV * 6.0).xyz * 2.0 - 1.0;
            vec3 bn = normalize(vec3(base_n.xy + dn.xy * detail_k * near, base_n.z * dn.z));
            NORMAL_MAP = bn * 0.5 + 0.5;
            NORMAL_MAP_DEPTH = has_nrm ? bump : 1.0;
        }
        // cavity: dark texels (cracks, seams) take less of the baked light
        float lum = dot(alb, vec3(0.3, 0.59, 0.11));
        AO = mix(1.0, clamp(lum * 4.0, 0.55, 1.0), 0.35);
        AO_LIGHT_AFFECT = 0.0;
        ALBEDO = alb;
        ROUGHNESS = rough;
        SPECULAR = spec;
        METALLIC = metal;
        CLEARCOAT = wet;
        CLEARCOAT_ROUGHNESS = 0.15;
        IRRADIANCE = vec4(baked * k * irr_scale, 1.0);
        RADIANCE = vec4(baked * 0.3 * irr_scale, 1.0);
    }
}
"#;

/// Additive / alpha-blended stages (glows, beams, glass): no lightmap, no depth write.
const WORLD_SHADER_ADD: &str = r#"
shader_type spatial;
// r_gamma 1.3 (a common Tremulous setting): gamma-space result -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode unshaded, cull_disabled, blend_add, depth_draw_never;
global uniform float trem_gfx;
uniform sampler2D tex : filter_linear_mipmap, repeat_enable;
uniform sampler2D tex_hd : filter_linear_mipmap, repeat_enable;
uniform bool has_hd = false;
uniform sampler2D lm : filter_linear;
uniform bool use_lm = false;
uniform vec4 tint = vec4(1.0);
void fragment() {
    vec4 t = ((trem_gfx > 0.5 && has_hd) ? texture(tex_hd, UV) : texture(tex, UV)) * tint;
    // enhanced: glows go over 1.0 so the bloom picks them up
    ALBEDO = pow(clamp(t.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP)) * (trem_gfx > 0.5 ? 1.8 : 1.0);
}
"#;
const WORLD_SHADER_MIX: &str = r#"
shader_type spatial;
// r_gamma 1.3 (a common Tremulous setting): gamma-space result -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode unshaded, cull_disabled, blend_mix, depth_draw_never;
global uniform float trem_gfx;
uniform sampler2D tex : filter_linear_mipmap, repeat_enable;
uniform sampler2D tex_hd : filter_linear_mipmap, repeat_enable;
uniform bool has_hd = false;
uniform sampler2D lm : filter_linear;
uniform bool use_lm = false;
uniform vec4 tint = vec4(1.0);
void fragment() {
    vec4 t = ((trem_gfx > 0.5 && has_hd) ? texture(tex_hd, UV) : texture(tex, UV)) * tint;
    vec3 light = use_lm ? texture(lm, UV2).rgb * 2.0 : vec3(1.0);
    vec3 c = trem_gfx > 0.5 ? pow(clamp(t.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP)) * pow(light, vec3(R_GAMMA_EXP)) : pow(clamp(t.rgb * light, 0.0, 1.0), vec3(R_GAMMA_EXP));
    ALBEDO = c;
    ALPHA = t.a;
}
"#;

/// The world's opaque triangles for the ray tracer (rt.rs), in Godot space:
/// positions, indices and per-vertex surface data (the outgoing baked
/// radiance = mean albedo x light map, and the mean albedo, both linear).
#[derive(Default, Clone)]
pub struct RtGeometry {
    pub pos: Vec<[f32; 3]>,
    pub rad: Vec<[f32; 4]>,
    pub alb: Vec<[f32; 4]>,
    pub idx: Vec<u32>,
}

/// Longest edge (Quake units) of a ray tracer triangle: the light map is
/// sampled at the vertices, so big faces are split to keep its detail.
const RT_MAX_EDGE: f32 = 48.0;
const RT_MAX_TRIS: usize = 1_500_000;

fn color_shift_f(c: [f32; 3]) -> [f32; 3] {
    let mut l = [c[0] * 2.0, c[1] * 2.0, c[2] * 2.0];
    let m = l[0].max(l[1]).max(l[2]);
    if m > 1.0 {
        l.iter_mut().for_each(|x| *x /= m);
    }
    l
}

fn lightmap_textures(bsp: &Bsp) -> Vec<Gd<ImageTexture>> {
    let n = bsp.lightmaps.len() / (128 * 128 * 3);
    (0..n)
        .filter_map(|i| {
            let bytes = &bsp.lightmaps[i * 128 * 128 * 3..(i + 1) * 128 * 128 * 3];
            let data = PackedByteArray::from(bytes);
            let img = Image::create_from_data(128, 128, false, ImageFormat::RGB8, &data)?;
            ImageTexture::create_from_image(&img)
        })
        .collect()
}

#[derive(Clone, Copy, Default)]
struct V {
    xyz: [f32; 3],
    st: [f32; 2],
    lm: [f32; 2],
    n: [f32; 3],
    c: [f32; 4],
}
fn v_of(d: &DrawVert) -> V {
    V { xyz: d.xyz, st: d.st, lm: d.lightmap, n: d.normal, c: d.color.map(|x| x as f32 / 255.0) }
}
fn lerp(a: &V, b: &V, t: f32) -> V {
    let l = |x: f32, y: f32| x + (y - x) * t;
    V {
        xyz: [l(a.xyz[0], b.xyz[0]), l(a.xyz[1], b.xyz[1]), l(a.xyz[2], b.xyz[2])],
        st: [l(a.st[0], b.st[0]), l(a.st[1], b.st[1])],
        lm: [l(a.lm[0], b.lm[0]), l(a.lm[1], b.lm[1])],
        n: [l(a.n[0], b.n[0]), l(a.n[1], b.n[1]), l(a.n[2], b.n[2])],
        c: [l(a.c[0], b.c[0]), l(a.c[1], b.c[1]), l(a.c[2], b.c[2]), l(a.c[3], b.c[3])],
    }
}
impl trem_sim::kernel::marks::GridVert for V {
    fn xyz(&self) -> [f32; 3] {
        self.xyz
    }
    /// tr_curve.c `LerpDrawVert` (the byte colours as floats)
    fn lerp_half(a: &V, b: &V) -> V {
        lerp(a, b, 0.5)
    }
    fn set_normal(&mut self, n: [f32; 3]) {
        self.n = n;
    }
}

/// A patch as the renderer draws it: tr_curve.c `R_SubdividePatchToGrid`
/// (r_subdivisions 4, the grid the mark code clips against too) and
/// tr_surface.c `RB_SurfaceGrid`'s triangles at full detail.
fn tessellate(ctrl: &[V], w: usize, h: usize, out_v: &mut Vec<V>, out_i: &mut Vec<u32>) {
    let (gw, gh, verts) = trem_sim::kernel::marks::subdivide_patch_to_grid(w, h, ctrl, trem_sim::kernel::marks::R_SUBDIVISIONS);
    if gw < 2 || gh < 2 {
        return;
    }
    let base = out_v.len() as u32;
    out_v.extend_from_slice(&verts);
    let row = gw as u32;
    for i in 0..gh as u32 - 1 {
        for j in 0..gw as u32 - 1 {
            // v1 = i*width + j + 1, v2 = v1 - 1, v3 = v2 + width, v4 = v3 + 1: (v2 v3 v1) (v1 v3 v4)
            let v2 = base + i * row + j;
            let (v1, v3) = (v2 + 1, v2 + row);
            out_i.extend_from_slice(&[v2, v3, v1, v1, v3, v3 + 1]);
        }
    }
}

/// Mesh builder for one BSP: lightmaps, shader-script images and the three
/// world shaders are made once and shared by every model drawn from it.
/// A fog brush (tr_bsp.c R_LoadFogs): its box in Quake units and the
/// shader's `fogparms` colour and distance to opaque.
#[derive(Clone, Debug, PartialEq)]
pub struct FogBox {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub color: [f32; 3],
    pub dist: f32,
}

/// The fogs lump (dfog_t: shader[64], brushNum, visibleSide) as boxes: the
/// first six sides of a fog brush are its axial planes, -x +x -y +y -z +z.
pub fn fog_boxes(bsp: &Bsp, scripts: &HashMap<String, crate::assets::ShaderInfo>) -> Vec<FogBox> {
    let Some(raw) = bsp.raw.get(&trem_sim::kernel::bsp::LUMP_FOGS) else { return Vec::new() };
    let mut out = Vec::new();
    for rec in raw.chunks_exact(72) {
        let name: String = rec[..64].iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
        let brush = i32::from_le_bytes([rec[64], rec[65], rec[66], rec[67]]);
        let Some(b) = usize::try_from(brush).ok().and_then(|i| bsp.brushes.get(i)) else { continue };
        if b.num_sides < 6 {
            continue;
        }
        let dist = |k: i32| -> Option<f32> {
            let side = bsp.brush_sides.get((b.first_side + k) as usize)?;
            bsp.planes.get(side.plane as usize).map(|p| p[3])
        };
        let (Some(x0), Some(x1), Some(y0), Some(y1), Some(z0), Some(z1)) = (dist(0), dist(1), dist(2), dist(3), dist(4), dist(5)) else { continue };
        let Some((color, d)) = scripts.get(&name.to_lowercase()).and_then(|i| i.fog) else { continue };
        if d <= 0.0 {
            continue;
        }
        out.push(FogBox { mins: [-x0, -y0, -z0], maxs: [x1, y1, z1], color, dist: d });
    }
    out
}

pub struct BspRender {
    script_images: HashMap<String, crate::assets::ShaderInfo>,
    lightmaps: Vec<Gd<ImageTexture>>,
    shader: Gd<Shader>,
    shader_add: Gd<Shader>,
    shader_mix: Gd<Shader>,
    white: Gd<ImageTexture>,
    tex_cache: HashMap<i32, Gd<ImageTexture>>,
    /// per shader: the generated normal map and the mean linear albedo (gfx.rs)
    derived: HashMap<i32, (Option<Gd<ImageTexture>>, [f32; 3])>,
    /// per shader: the 4K pack's albedo (hdtex.rs), sampled in the enhanced mode
    hd: HashMap<i32, Option<Gd<ImageTexture>>>,
    /// the light grid, for the enhanced mode's bumped baked light (CUSTOM0)
    grid: Option<LightGrid>,
    /// the world's opaque triangles for the ray tracer (model 0)
    pub rt: RtGeometry,
    /// the same, unsplit, for the acoustic simulation (fmodaudio.rs)
    pub acoustic: crate::fmodaudio::AcousticMesh,
    /// the map's fog brushes (enhanced mode: volumetric fog volumes)
    pub fogs: Vec<FogBox>,
}

impl BspRender {
    pub fn new(bsp: &Bsp, pk3: &mut Pk3Set) -> Self {
        let mk = |code: &str| {
            let mut sh = Shader::new_gd();
            sh.set_code(code);
            sh
        };
        BspRender {
            script_images: pk3.shader_images(),
            lightmaps: lightmap_textures(bsp),
            shader: mk(WORLD_SHADER),
            shader_add: mk(WORLD_SHADER_ADD),
            shader_mix: mk(WORLD_SHADER_MIX),
            white: flat_texture(Color::from_rgb(1.0, 1.0, 1.0)),
            tex_cache: HashMap::new(),
            derived: HashMap::new(),
            hd: HashMap::new(),
            grid: LightGrid::from_bsp(bsp),
            rt: RtGeometry::default(),
            acoustic: Default::default(),
            fogs: Vec::new(),
        }
        .with_fogs(bsp)
    }

    fn with_fogs(mut self, bsp: &Bsp) -> Self {
        self.fogs = fog_boxes(bsp, &self.script_images);
        self
    }

    /// The light map at (s, t) of light map `lm` (bilinear), or the vertex
    /// colour without one; gamma space, before the overbright shift.
    fn sample_lightmap(bsp: &Bsp, lm: i32, v: &V) -> [f32; 3] {
        let n = bsp.lightmaps.len() / (128 * 128 * 3);
        if lm < 0 || lm as usize >= n {
            return [v.c[0], v.c[1], v.c[2]];
        }
        let base = lm as usize * 128 * 128 * 3;
        let fx = (v.lm[0] * 128.0 - 0.5).clamp(0.0, 127.0);
        let fy = (v.lm[1] * 128.0 - 0.5).clamp(0.0, 127.0);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(127), (y0 + 1).min(127));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let px = |x: usize, y: usize, k: usize| bsp.lightmaps[base + (y * 128 + x) * 3 + k] as f32 / 255.0;
        [0, 1, 2].map(|k| {
            let a = px(x0, y0, k) + (px(x1, y0, k) - px(x0, y0, k)) * tx;
            let b = px(x0, y1, k) + (px(x1, y1, k) - px(x0, y1, k)) * tx;
            a + (b - a) * ty
        })
    }

    /// Add one triangle to the ray tracer's geometry, split until its
    /// longest edge is at most RT_MAX_EDGE.
    fn rt_triangle(&mut self, bsp: &Bsp, lm: i32, alb: [f32; 3], a: V, b: V, c: V, depth: u32) {
        if self.rt.idx.len() / 3 >= RT_MAX_TRIS {
            return;
        }
        let d = |p: &V, q: &V| ((p.xyz[0] - q.xyz[0]).powi(2) + (p.xyz[1] - q.xyz[1]).powi(2) + (p.xyz[2] - q.xyz[2]).powi(2)).sqrt();
        let (ab, bc, ca) = (d(&a, &b), d(&b, &c), d(&c, &a));
        let m = ab.max(bc).max(ca);
        if m > RT_MAX_EDGE && depth < 8 {
            if m == ab {
                let mid = lerp(&a, &b, 0.5);
                self.rt_triangle(bsp, lm, alb, a, mid, c, depth + 1);
                self.rt_triangle(bsp, lm, alb, mid, b, c, depth + 1);
            } else if m == bc {
                let mid = lerp(&b, &c, 0.5);
                self.rt_triangle(bsp, lm, alb, a, b, mid, depth + 1);
                self.rt_triangle(bsp, lm, alb, a, mid, c, depth + 1);
            } else {
                let mid = lerp(&c, &a, 0.5);
                self.rt_triangle(bsp, lm, alb, a, b, mid, depth + 1);
                self.rt_triangle(bsp, lm, alb, mid, b, c, depth + 1);
            }
            return;
        }
        for v in [a, b, c] {
            let l = color_shift_f(Self::sample_lightmap(bsp, lm, &v));
            let baked = l.map(|x| (x * 2.0).powf(crate::gfx::R_GAMMA_EXP));
            let g = q2g(&v.xyz);
            self.rt.idx.push(self.rt.pos.len() as u32);
            self.rt.pos.push([g.x, g.y, g.z]);
            self.rt.rad.push([alb[0] * baked[0], alb[1] * baked[1], alb[2] * baked[2], 1.0]);
            self.rt.alb.push([alb[0], alb[1], alb[2], 1.0]);
        }
    }

    /// One MeshInstance3D per (shader, lightmap) batch of inline model `m`
    /// (0 = the world), in the model's own coordinates.
    pub fn model(&mut self, bsp: &Bsp, pk3: &mut Pk3Set, m: usize) -> Vec<Gd<MeshInstance3D>> {
        let Some(model) = bsp.models.get(m).copied() else { return Vec::new() };
        let mut batches: HashMap<(i32, i32), (Vec<V>, Vec<u32>)> = HashMap::new();
        for si in model.first_surface..model.first_surface + model.num_surfaces {
            let Some(s) = bsp.surfaces.get(si as usize) else { continue };
            let Some(sh) = bsp.shaders.get(s.shader as usize) else { continue };
            if sh.surface_flags & (SURF_NODRAW | SURF_SKY) != 0 {
                continue;
            }
            let verts = &bsp.verts[s.first_vert as usize..(s.first_vert + s.num_verts) as usize];
            let (bv, bi) = batches.entry((s.shader, s.lightmap_num)).or_default();
            match s.surface_type {
                MST_PLANAR | MST_TRIANGLE_SOUP => {
                    let base = bv.len() as u32;
                    bv.extend(verts.iter().map(v_of));
                    for k in 0..s.num_indexes {
                        bi.push(base + bsp.indexes[(s.first_index + k) as usize] as u32);
                    }
                }
                MST_PATCH => {
                    let ctrl: Vec<V> = verts.iter().map(v_of).collect();
                    tessellate(&ctrl, s.patch_width as usize, s.patch_height as usize, bv, bi);
                }
                _ => {}
            }
        }
        let mut keys: Vec<_> = batches.keys().copied().collect();
        keys.sort();
        let mut out = Vec::new();
        for key in keys {
            let (vs, is) = &batches[&key];
            if is.is_empty() {
                continue;
            }
            let (shader_num, lm) = key;
            let info = self.script_images.get(&bsp.shaders[shader_num as usize].name.to_lowercase()).cloned().unwrap_or_default();
            if info.nodraw {
                continue;
            }
            let opaque = info.blend == crate::assets::Blend::Opaque;
            let script_images = &self.script_images;
            let derived = &mut self.derived;
            let hd_cache = &mut self.hd;
            let tex = self
                .tex_cache
                .entry(shader_num)
                .or_insert_with(|| {
                    let name = bsp.shaders[shader_num as usize].name.clone();
                    let script_img = script_images.get(&name.to_lowercase()).and_then(|p| p.image.clone());
                    let img = script_img.as_ref().and_then(|p| load_image(pk3, p)).or_else(|| load_image(pk3, &name));
                    // the 4K pack's texture (hdtex.rs): its normal map and
                    // mean colour come with it; else the generated maps (gfx.rs)
                    let hd = script_img.as_ref().and_then(|p| crate::hdtex::get(&crate::hdtex::key_of(p))).or_else(|| crate::hdtex::get(&crate::hdtex::key_of(&name)));
                    let d = match (hd.as_ref(), img.as_ref()) {
                        (Some(h), _) => crate::gfx::DerivedMaps { normal: if opaque { h.normal.clone() } else { None }, mean_linear: h.mean },
                        (None, Some(i)) => crate::gfx::derive_maps(i, opaque),
                        (None, None) => crate::gfx::DerivedMaps { normal: None, mean_linear: [0.3, 0.3, 0.32] },
                    };
                    derived.insert(shader_num, (d.normal, d.mean_linear));
                    hd_cache.insert(shader_num, hd.map(|h| h.albedo));
                    match img.and_then(|i| ImageTexture::create_from_image(&i)) {
                        Some(t) => t,
                        None => flat_texture(Color::from_rgb(0.55, 0.55, 0.6)),
                    }
                })
                .clone();
            let (nrm, mean) = self.derived.get(&shader_num).cloned().unwrap_or((None, [0.3; 3]));
            // the ray tracer sees the world's opaque surfaces
            if m == 0 && opaque {
                let sh = &bsp.shaders[shader_num as usize];
                let mat = crate::fmodaudio::material_of(&sh.name, sh.surface_flags);
                let base = self.acoustic.verts.len() as i32;
                for v in vs {
                    let g = q2g(&v.xyz);
                    self.acoustic.verts.push([g.x, g.y, g.z]);
                }
                for t in is.chunks_exact(3) {
                    self.acoustic.tris.push([base + t[0] as i32, base + t[1] as i32, base + t[2] as i32]);
                    self.acoustic.mats.push(mat);
                }
                for t in is.chunks_exact(3) {
                    let (a, b, c) = (vs[t[0] as usize], vs[t[1] as usize], vs[t[2] as usize]);
                    self.rt_triangle(bsp, lm, mean, a, b, c, 0);
                }
            }
            let mut st = SurfaceTool::new_gd();
            st.begin(PrimitiveType::TRIANGLES);
            st.set_custom_format(0, godot::classes::surface_tool::CustomFormat::RGBA_FLOAT);
            for v in vs {
                st.set_normal(q2g_dir(&v.n));
                st.set_uv(Vector2::new(v.st[0], v.st[1]));
                st.set_uv2(Vector2::new(v.lm[0], v.lm[1]));
                st.set_color(Color::from_rgba(v.c[0], v.c[1], v.c[2], v.c[3]));
                // the light grid's dominant direction (Godot space) and the
                // directed share of its light, for the bumped baked light
                let g = match self.grid.as_ref() {
                    Some(g) => {
                        let (amb, dir, d) = g.sample(&v.xyz);
                        let la = amb[0] * 0.3 + amb[1] * 0.59 + amb[2] * 0.11;
                        let ld = dir[0] * 0.3 + dir[1] * 0.59 + dir[2] * 0.11;
                        let gd = q2g_dir(&d);
                        Color::from_rgba(gd.x, gd.y, gd.z, if la + ld > 0.0 { ld / (la + ld) } else { 0.0 })
                    }
                    None => Color::from_rgba(0.0, 1.0, 0.0, 0.0),
                };
                st.set_custom(0, g);
                st.add_vertex(q2g(&v.xyz));
            }
            // Q3 winding is clockwise seen from the front, Godot's too: keep order.
            for &i in is {
                st.add_index(i as i32);
            }
            if nrm.is_some() {
                st.generate_tangents();
            }
            let Some(mesh) = st.commit() else { continue };
            let mut mat = ShaderMaterial::new_gd();
            mat.set_shader(match info.blend {
                crate::assets::Blend::Add => &self.shader_add,
                crate::assets::Blend::Mix => &self.shader_mix,
                crate::assets::Blend::Opaque => &self.shader,
            });
            mat.set_shader_parameter("tex", &tex.to_variant());
            if let Some(Some(hd)) = self.hd.get(&shader_num) {
                mat.set_shader_parameter("tex_hd", &hd.to_variant());
                mat.set_shader_parameter("has_hd", &true.to_variant());
            }
            let use_lm = lm >= 0 && (lm as usize) < self.lightmaps.len();
            mat.set_shader_parameter("use_lm", &use_lm.to_variant());
            mat.set_shader_parameter("lm", &(if use_lm { self.lightmaps[lm as usize].clone() } else { self.white.clone() }).to_variant());
            if opaque {
                let class = crate::gfx::surface_class(&bsp.shaders[shader_num as usize].name);
                let (rough, spec, bump) = class.base;
                mat.set_shader_parameter("rough", &rough.to_variant());
                mat.set_shader_parameter("spec", &spec.to_variant());
                mat.set_shader_parameter("bump", &bump.to_variant());
                mat.set_shader_parameter("metal", &class.metal.to_variant());
                mat.set_shader_parameter("wet", &class.wet.to_variant());
                mat.set_shader_parameter("detail_k", &class.detail.to_variant());
                mat.set_shader_parameter("detail", &crate::gfx::detail_normal().to_variant());
                if let Some(n) = nrm.as_ref() {
                    mat.set_shader_parameter("nrm", &n.to_variant());
                    mat.set_shader_parameter("has_nrm", &true.to_variant());
                }
            }
            let mut mi = MeshInstance3D::new_alloc();
            mi.set_mesh(&mesh);
            mi.set_material_override(&mat);
            mi.set_name(&format!("bsp{m}_{shader_num}_{lm}"));
            out.push(mi);
        }
        out
    }
}


/// The 4K pack's six skybox faces (hdtex.rs), when it has all of them.
pub fn sky_textures_hd(bsp: &Bsp, pk3: &mut Pk3Set) -> Option<Vec<Gd<godot::classes::Texture2D>>> {
    let infos = pk3.shader_images();
    let env = bsp.shaders.iter().filter(|s| s.surface_flags & SURF_SKY != 0).find_map(|s| infos.get(&s.name.to_lowercase()).and_then(|i| i.sky.clone()))?;
    ["rt", "lf", "bk", "ft", "up", "dn"].iter().map(|suf| crate::hdtex::get(&crate::hdtex::key_of(&format!("{env}_{suf}"))).map(|h| h.albedo.upcast::<godot::classes::Texture2D>())).collect()
}

/// Six skybox images (rt, lf, bk, ft, up, dn) of the map's sky shader
/// (`skyparms <env> ...` -> <env>_rt.tga ...), in Q3's axis order.
pub fn sky_textures(bsp: &Bsp, pk3: &mut Pk3Set) -> Option<Vec<Gd<ImageTexture>>> {
    let infos = pk3.shader_images();
    let env = bsp.shaders.iter().filter(|s| s.surface_flags & SURF_SKY != 0).find_map(|s| infos.get(&s.name.to_lowercase()).and_then(|i| i.sky.clone()))?;
    let mut out = Vec::new();
    for suf in ["rt", "lf", "bk", "ft", "up", "dn"] {
        let img = load_image(pk3, &format!("{env}_{suf}"))?;
        out.push(ImageTexture::create_from_image(&img)?);
    }
    Some(out)
}

/// Sky shader reproducing tr_sky.c's box mapping (MakeSkyVec / st_to_vec).
pub const SKY_SHADER: &str = r#"
shader_type sky;
global uniform float trem_gfx;
uniform sampler2D rt : source_color, filter_linear;
uniform sampler2D lf : source_color, filter_linear;
uniform sampler2D bk : source_color, filter_linear;
uniform sampler2D ft : source_color, filter_linear;
uniform sampler2D up : source_color, filter_linear;
uniform sampler2D dn : source_color, filter_linear;
// the 4K pack's faces (enhanced mode)
uniform bool has_hd = false;
uniform sampler2D rt_hd : source_color, filter_linear_mipmap;
uniform sampler2D lf_hd : source_color, filter_linear_mipmap;
uniform sampler2D bk_hd : source_color, filter_linear_mipmap;
uniform sampler2D ft_hd : source_color, filter_linear_mipmap;
uniform sampler2D up_hd : source_color, filter_linear_mipmap;
uniform sampler2D dn_hd : source_color, filter_linear_mipmap;
vec4 face(sampler2D t, float s, float tt) {
    return texture(t, vec2((s + 1.0) * 0.5, 1.0 - (tt + 1.0) * 0.5));
}
// Godot needs one function per sampler setting, so HD faces get their own.
vec4 face_hd(sampler2D t, float s, float tt) {
    return texture(t, vec2((s + 1.0) * 0.5, 1.0 - (tt + 1.0) * 0.5));
}
void sky() {
    vec3 q = vec3(EYEDIR.x, -EYEDIR.z, EYEDIR.y);
    vec3 a = abs(q);
    bool hd = has_hd && trem_gfx > 0.5;
    vec4 c;
    if (a.x >= a.y && a.x >= a.z) {
        c = q.x > 0.0 ? (hd ? face_hd(rt_hd, -q.y / a.x, q.z / a.x) : face(rt, -q.y / a.x, q.z / a.x)) : (hd ? face_hd(lf_hd, q.y / a.x, q.z / a.x) : face(lf, q.y / a.x, q.z / a.x));
    } else if (a.y >= a.z) {
        c = q.y > 0.0 ? (hd ? face_hd(bk_hd, q.x / a.y, q.z / a.y) : face(bk, q.x / a.y, q.z / a.y)) : (hd ? face_hd(ft_hd, -q.x / a.y, q.z / a.y) : face(ft, -q.x / a.y, q.z / a.y));
    } else {
        c = q.z > 0.0 ? (hd ? face_hd(up_hd, -q.y / a.z, -q.x / a.z) : face(up, -q.y / a.z, -q.x / a.z)) : (hd ? face_hd(dn_hd, -q.y / a.z, q.x / a.z) : face(dn, -q.y / a.z, q.x / a.z));
    }
    COLOR = c.rgb;
}
"#;

/// The map's light grid (tr_bsp.c R_LoadLightGrid + tr_light.c
/// R_SetupEntityLightingGrid): ambient + directed light for models.
pub struct LightGrid {
    origin: [f32; 3],
    size: [f32; 3],
    bounds: [i32; 3],
    data: Vec<u8>,
}

/// `R_ColorShiftLightingBytes` with mapOverBrightBits 2, overBrightBits 1 (x2, hue-preserving clamp).
fn color_shift(c: [u8; 3]) -> [f32; 3] {
    let mut r = [c[0] as f32 * 2.0, c[1] as f32 * 2.0, c[2] as f32 * 2.0];
    let m = r[0].max(r[1]).max(r[2]);
    if m > 255.0 {
        r.iter_mut().for_each(|x| *x *= 255.0 / m);
    }
    r
}

impl LightGrid {
    pub fn from_bsp(bsp: &Bsp) -> Option<LightGrid> {
        let data = bsp.raw.get(&15)?.clone();
        let m = bsp.models.first()?;
        let mut size = [64.0f32, 64.0, 128.0];
        if let Some(ws) = bsp.entity_list().into_iter().find(|e| e.get("classname").map(|c| c == "worldspawn").unwrap_or(false)) {
            if let Some(v) = ws.get("gridsize").and_then(|g| trem_sim::kernel::bsp::parse_vec(g)) {
                size = v;
            }
        }
        let mut origin = [0.0; 3];
        let mut bounds = [0; 3];
        for i in 0..3 {
            origin[i] = size[i] * (m.mins[i] / size[i]).ceil();
            let maxs = size[i] * (m.maxs[i] / size[i]).floor();
            bounds[i] = ((maxs - origin[i]) / size[i]) as i32 + 1;
        }
        let n = (bounds[0] * bounds[1] * bounds[2]) as usize;
        if n == 0 || data.len() < n * 8 {
            return None;
        }
        Some(LightGrid { origin, size, bounds, data })
    }

    /// (ambient, directed) in 0..1 gamma units and the light direction (Quake space, towards the light).
    pub fn sample(&self, p: &[f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let mut pos = [0i32; 3];
        let mut frac = [0f32; 3];
        for i in 0..3 {
            let v = (p[i] - self.origin[i]) / self.size[i];
            pos[i] = v.floor() as i32;
            frac[i] = v - pos[i] as f32;
            pos[i] = pos[i].clamp(0, self.bounds[i] - 1);
        }
        let step = [8, 8 * self.bounds[0], 8 * self.bounds[0] * self.bounds[1]];
        let base = pos[0] * step[0] + pos[1] * step[1] + pos[2] * step[2];
        let (mut amb, mut dir, mut d) = ([0f32; 3], [0f32; 3], [0f32; 3]);
        let mut total = 0.0;
        'corner: for i in 0..8 {
            let mut factor = 1.0;
            let mut off = base;
            for j in 0..3 {
                if i & (1 << j) != 0 {
                    if pos[j] + 1 > self.bounds[j] - 1 {
                        continue 'corner;
                    }
                    factor *= frac[j];
                    off += step[j];
                } else {
                    factor *= 1.0 - frac[j];
                }
            }
            let o = off as usize;
            let c = &self.data[o..o + 8];
            if c[0] as u32 + c[1] as u32 + c[2] as u32 == 0 {
                continue;
            }
            total += factor;
            let a = color_shift([c[0], c[1], c[2]]);
            let b = color_shift([c[3], c[4], c[5]]);
            for k in 0..3 {
                amb[k] += factor * a[k];
                dir[k] += factor * b[k];
            }
            let lat = c[7] as f32 * (2.0 * std::f32::consts::PI / 255.0);
            let lng = c[6] as f32 * (2.0 * std::f32::consts::PI / 255.0);
            let n = [lat.cos() * lng.sin(), lat.sin() * lng.sin(), lng.cos()];
            for k in 0..3 {
                d[k] += factor * n[k];
            }
        }
        if total > 0.0 && total < 0.99 {
            for k in 0..3 {
                amb[k] /= total;
                dir[k] /= total;
            }
        }
        // minimum light add (identityLight * 32) and the ambient clamp (identityLightByte)
        for k in 0..3 {
            amb[k] = (amb[k] + 16.0).min(127.0) / 255.0;
            dir[k] /= 255.0;
        }
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let d = if l > 0.0 { [d[0] / l, d[1] / l, d[2] / l] } else { [0.0, 0.0, 1.0] };
        (amb, dir, d)
    }
}

#[cfg(test)]
mod fog_tests {
    use super::*;
    use trem_sim::kernel::bsp::{DBrush, DBrushSide};

    #[test]
    fn fog_brush_becomes_a_box() {
        let mut bsp = Bsp::default();
        // planes -x (dist 10 -> min x -10), +x 20, -y 5, +y 6, -z 1, +z 2
        bsp.planes = vec![[-1.0, 0.0, 0.0, 10.0], [1.0, 0.0, 0.0, 20.0], [0.0, -1.0, 0.0, 5.0], [0.0, 1.0, 0.0, 6.0], [0.0, 0.0, -1.0, 1.0], [0.0, 0.0, 1.0, 2.0]];
        bsp.brush_sides = (0..6).map(|i| DBrushSide { plane: i, shader: 0 }).collect();
        bsp.brushes = vec![DBrush { first_side: 0, num_sides: 6, shader: 0 }];
        let mut rec = vec![0u8; 72];
        rec[..9].copy_from_slice(b"fog/green");
        bsp.raw.insert(trem_sim::kernel::bsp::LUMP_FOGS, rec);
        let mut scripts = HashMap::new();
        scripts.insert("fog/green".to_string(), crate::assets::ShaderInfo { fog: Some(([0.1, 0.8, 0.2], 300.0)), ..Default::default() });
        let f = fog_boxes(&bsp, &scripts);
        assert_eq!(f, vec![FogBox { mins: [-10.0, -5.0, -1.0], maxs: [20.0, 6.0, 2.0], color: [0.1, 0.8, 0.2], dist: 300.0 }]);
    }
}

#[cfg(test)]
mod lightgrid_tests {
    use super::*;
    /// TREM_BSP=<file.bsp>: light grid gives plausible light at every map placement.
    #[test]
    fn lightgrid_samples_real_map() {
        let Ok(p) = std::env::var("TREM_BSP") else { return };
        let bsp = trem_sim::kernel::bsp::parse(&std::fs::read(p).unwrap()).unwrap();
        let g = LightGrid::from_bsp(&bsp).expect("grid");
        println!("grid origin {:?} size {:?} bounds {:?}", g.origin, g.size, g.bounds);
        for e in bsp.entity_list() {
            if let Some(o) = e.get("origin").and_then(|o| trem_sim::kernel::bsp::parse_vec(o)) {
                let (a, d, dir) = g.sample(&[o[0], o[1], o[2] + 16.0]);
                println!("{:<28} amb {:.2?} dir {:.2?} {:.2?}", e.get("classname").unwrap(), a, d, dir);
            }
        }
    }
}
