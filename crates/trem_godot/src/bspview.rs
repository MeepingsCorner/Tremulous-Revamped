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
// r_gamma 1.3 (a common Tremulous setting): gamma-space result -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode unshaded, cull_disabled;
uniform sampler2D tex : filter_linear_mipmap, repeat_enable;
uniform sampler2D lm : filter_linear;
uniform bool use_lm = false;
uniform vec4 tint = vec4(1.0);
void fragment() {
    vec4 t = texture(tex, UV) * tint;
    if (t.a < 0.5) { discard; }
    vec3 light = use_lm ? texture(lm, UV2).rgb : COLOR.rgb;
    // R_ColorShiftLightingBytes (mapOverBrightBits 2 - overBrightBits 1 = x2,
    // hue-preserving clamp), then the x2 hardware overbright; all in gamma space.
    light *= 2.0;
    float m = max(light.r, max(light.g, light.b));
    if (m > 1.0) { light /= m; }
    ALBEDO = pow(clamp(t.rgb * light * 2.0, 0.0, 1.0), vec3(R_GAMMA_EXP));
}
"#;

/// Additive / alpha-blended stages (glows, beams, glass): no lightmap, no depth write.
const WORLD_SHADER_ADD: &str = r#"
shader_type spatial;
// r_gamma 1.3 (a common Tremulous setting): gamma-space result -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode unshaded, cull_disabled, blend_add, depth_draw_never;
uniform sampler2D tex : filter_linear_mipmap, repeat_enable;
uniform sampler2D lm : filter_linear;
uniform bool use_lm = false;
uniform vec4 tint = vec4(1.0);
void fragment() {
    vec4 t = texture(tex, UV) * tint;
    ALBEDO = pow(clamp(t.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP));
}
"#;
const WORLD_SHADER_MIX: &str = r#"
shader_type spatial;
// r_gamma 1.3 (a common Tremulous setting): gamma-space result -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode unshaded, cull_disabled, blend_mix, depth_draw_never;
uniform sampler2D tex : filter_linear_mipmap, repeat_enable;
uniform sampler2D lm : filter_linear;
uniform bool use_lm = false;
uniform vec4 tint = vec4(1.0);
void fragment() {
    vec4 t = texture(tex, UV) * tint;
    vec3 light = use_lm ? texture(lm, UV2).rgb * 2.0 : vec3(1.0);
    ALBEDO = pow(clamp(t.rgb * light, 0.0, 1.0), vec3(R_GAMMA_EXP));
    ALPHA = t.a;
}
"#;

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
pub struct BspRender {
    script_images: HashMap<String, crate::assets::ShaderInfo>,
    lightmaps: Vec<Gd<ImageTexture>>,
    shader: Gd<Shader>,
    shader_add: Gd<Shader>,
    shader_mix: Gd<Shader>,
    white: Gd<ImageTexture>,
    tex_cache: HashMap<i32, Gd<ImageTexture>>,
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
            let script_images = &self.script_images;
            let tex = self
                .tex_cache
                .entry(shader_num)
                .or_insert_with(|| {
                    let name = bsp.shaders[shader_num as usize].name.clone();
                    let img = script_images.get(&name.to_lowercase()).and_then(|p| p.image.clone()).and_then(|p| load_image(pk3, &p)).or_else(|| load_image(pk3, &name));
                    match img.and_then(|i| ImageTexture::create_from_image(&i)) {
                        Some(t) => t,
                        None => flat_texture(Color::from_rgb(0.55, 0.55, 0.6)),
                    }
                })
                .clone();
            let mut st = SurfaceTool::new_gd();
            st.begin(PrimitiveType::TRIANGLES);
            for v in vs {
                st.set_normal(q2g_dir(&v.n));
                st.set_uv(Vector2::new(v.st[0], v.st[1]));
                st.set_uv2(Vector2::new(v.lm[0], v.lm[1]));
                st.set_color(Color::from_rgba(v.c[0], v.c[1], v.c[2], v.c[3]));
                st.add_vertex(q2g(&v.xyz));
            }
            // Q3 winding is clockwise seen from the front, Godot's too: keep order.
            for &i in is {
                st.add_index(i as i32);
            }
            let Some(mesh) = st.commit() else { continue };
            let info = self.script_images.get(&bsp.shaders[shader_num as usize].name.to_lowercase()).cloned().unwrap_or_default();
            if info.nodraw {
                continue;
            }
            let mut mat = ShaderMaterial::new_gd();
            mat.set_shader(match info.blend {
                crate::assets::Blend::Add => &self.shader_add,
                crate::assets::Blend::Mix => &self.shader_mix,
                crate::assets::Blend::Opaque => &self.shader,
            });
            mat.set_shader_parameter("tex", &tex.to_variant());
            let use_lm = lm >= 0 && (lm as usize) < self.lightmaps.len();
            mat.set_shader_parameter("use_lm", &use_lm.to_variant());
            mat.set_shader_parameter("lm", &(if use_lm { self.lightmaps[lm as usize].clone() } else { self.white.clone() }).to_variant());
            let mut mi = MeshInstance3D::new_alloc();
            mi.set_mesh(&mesh);
            mi.set_material_override(&mat);
            mi.set_name(&format!("bsp{m}_{shader_num}_{lm}"));
            out.push(mi);
        }
        out
    }
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
uniform sampler2D rt : source_color, filter_linear;
uniform sampler2D lf : source_color, filter_linear;
uniform sampler2D bk : source_color, filter_linear;
uniform sampler2D ft : source_color, filter_linear;
uniform sampler2D up : source_color, filter_linear;
uniform sampler2D dn : source_color, filter_linear;
vec4 face(sampler2D t, float s, float tt) {
    return texture(t, vec2((s + 1.0) * 0.5, 1.0 - (tt + 1.0) * 0.5));
}
void sky() {
    vec3 q = vec3(EYEDIR.x, -EYEDIR.z, EYEDIR.y);
    vec3 a = abs(q);
    vec4 c;
    if (a.x >= a.y && a.x >= a.z) {
        c = q.x > 0.0 ? face(rt, -q.y / a.x, q.z / a.x) : face(lf, q.y / a.x, q.z / a.x);
    } else if (a.y >= a.z) {
        c = q.y > 0.0 ? face(bk, q.x / a.y, q.z / a.y) : face(ft, -q.x / a.y, q.z / a.y);
    } else {
        c = q.z > 0.0 ? face(up, -q.y / a.z, -q.x / a.z) : face(dn, -q.y / a.z, q.x / a.z);
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
