//! Wall marks as the original draws them (cg_marks.c, renderer tr_marks.c):
//! `CG_ImpactMark` builds the mark square, clips it against the world's draw
//! surfaces with `R_MarkFragments` (trem_sim kernel `marks`, the C's
//! trap_CM_MarkFragments) and keeps the fragments as mark polys
//! (`CG_AllocMark`, `MAX_MARK_POLYS`), which `CG_AddMarks` fades and draws
//! every frame; temporary marks — `CG_PlayerShadow` blob shadows,
//! `CG_PlayerSplash` wakes, `CG_Creep` under alien structures and creep
//! under trapper-locked players — are clipped and drawn for one frame
//! (trap_R_AddPolyToScene). The polys are ordinary triangle meshes with the
//! mark shader's blend (`GL_ZERO GL_ONE_MINUS_SRC_COLOR` darkening, alpha
//! blend, additive; dec030) and a polygonOffset-like pull towards the camera, so they
//! draw in Forward+ and Compatibility alike. A mark shader with several
//! stages, `tcMod rotate / stretch / scroll / scale` or `rgbGen wave` (the
//! wake) draws every stage with the renderer's per-frame texture matrix
//! and wave colour (tr_shade_calc.c `RB_CalcRotateTexCoords`,
//! `RB_CalcStretchTexCoords`, `RB_CalcScrollTexCoords`,
//! `RB_CalcScaleTexCoords`, `RB_CalcTransformTexCoords`,
//! `RB_CalcWaveColor`, `EvalWaveForm` over tr_init.c's function tables).

use crate::geom::q2g;
use crate::models::Library;
use godot::classes::geometry_instance_3d::ShadowCastingSetting;
use godot::classes::mesh::PrimitiveType;
use godot::classes::{ImmediateMesh, MeshInstance3D, Node3D, Shader, ShaderMaterial};
use godot::prelude::*;
use std::collections::{HashMap, VecDeque};
use trem_sim::gen::constants as k;
use trem_sim::kernel::marks::{impact_mark_polygon, MarkWorld};
use trem_sim::kernel::qmath::{self as qm, Vec3};
use trem_sim::kernel::world::{EType, World};

/// `MARK_TOTAL_TIME` / `MARK_FADE_TIME`.
const MARK_TOTAL_TIME: i32 = 10000;
const MARK_FADE_TIME: i32 = 1000;
/// `MAX_MARK_POLYS` (cg_local.h).
const MAX_MARK_POLYS: usize = 256;
/// `MAX_VERTS_ON_POLY` (cg_local.h: the most a stored mark poly keeps).
const MAX_VERTS_ON_POLY: usize = 10;
/// `MAX_MARK_FRAGMENTS` / `MAX_MARK_POINTS` (cg_marks.c).
const MAX_MARK_FRAGMENTS: usize = 128;
const MAX_MARK_POINTS: usize = 384;
/// cg_buildable.c
const CREEP_SIZE: f32 = 64.0;
const CREEP_DISTANCE: f32 = 64.0;
const CREEP_SCALEDOWN_TIME: i32 = 3000;
/// cg_addMarks (default 1) and cg_shadows (default 1: blob shadow marks).
const CG_ADD_MARKS: bool = true;
const CG_SHADOWS: i32 = 1;
/// cg_players.c
const SHADOW_DISTANCE: f32 = 128.0;

/// `polyVert_t`.
#[derive(Clone, Copy, Debug, Default)]
struct PolyVert {
    xyz: Vec3,
    st: [f32; 2],
    modulate: [u8; 4],
}

/// `markPoly_t`.
#[derive(Clone, Debug)]
struct MarkPoly {
    time: i32,
    shader: String,
    alpha_fade: bool,
    color: [f32; 4],
    verts: Vec<PolyVert>,
}

/// How a mark shader's stage composites (its `blendFunc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkBlend {
    /// `GL_SRC_ALPHA GL_ONE_MINUS_SRC_ALPHA` / `blend`.
    Mix,
    /// `GL_ONE GL_ONE` / `add`.
    Add,
    /// `GL_ZERO GL_ONE_MINUS_SRC_COLOR`: dst * (1 - src).
    Subtract,
    /// `GL_DST_COLOR GL_ZERO` / `filter`: dst * src.
    Filter,
}

/// The first stage's blendFunc of shader `name` in `scripts/*.shader`.
fn blend_of_script(text: &str, name: &str) -> Option<MarkBlend> {
    let mut depth = 0;
    let mut cur: Option<String> = None;
    let mut stage = 0;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        let toks: Vec<String> = line.split_whitespace().map(|t| t.to_lowercase()).collect();
        if toks.is_empty() {
            continue;
        }
        if depth == 0 && toks[0] != "{" && toks[0] != "}" {
            cur = Some(toks[0].clone());
        }
        for t in &toks {
            if t == "{" {
                depth += 1;
                if depth == 2 {
                    stage += 1;
                }
            } else if t == "}" {
                depth -= 1;
                if depth == 0 {
                    cur = None;
                    stage = 0;
                }
            }
        }
        if depth == 2 && stage == 1 && toks[0] == "blendfunc" && cur.as_deref() == Some(name) {
            let a: Vec<&str> = toks[1..].iter().map(|s| s.as_str()).collect();
            return Some(match a.as_slice() {
                ["add"] | ["gl_one", "gl_one"] | ["gl_src_alpha", "gl_one"] => MarkBlend::Add,
                ["filter"] | ["gl_dst_color", "gl_zero"] | ["gl_zero", "gl_src_color"] => MarkBlend::Filter,
                ["gl_zero", "gl_one_minus_src_color"] => MarkBlend::Subtract,
                _ => MarkBlend::Mix,
            });
        }
    }
    None
}

/// tr_local.h FUNCTABLE_SIZE
const FUNCTABLE_SIZE: i32 = 1024;
const FUNCTABLE_MASK: i32 = FUNCTABLE_SIZE - 1;

/// `genFunc_t` (the table functions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GenFunc {
    Sin,
    Triangle,
    Square,
    Sawtooth,
    InverseSawtooth,
}

/// `waveForm_t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wave {
    pub func: GenFunc,
    pub base: f32,
    pub amplitude: f32,
    pub phase: f32,
    pub frequency: f32,
}

/// tr_init.c's function tables (`tr.sinTable` ...), entry `i`.
pub fn func_table(func: GenFunc, i: i32) -> f32 {
    let i = i & FUNCTABLE_MASK;
    let n = FUNCTABLE_SIZE;
    match func {
        GenFunc::Sin => ((i as f32 * 360.0 / (n - 1) as f32) as f64 * std::f64::consts::PI / 180.0).sin() as f32,
        GenFunc::Square => if i < n / 2 { 1.0 } else { -1.0 },
        GenFunc::Sawtooth => i as f32 / n as f32,
        GenFunc::InverseSawtooth => 1.0 - i as f32 / n as f32,
        GenFunc::Triangle => {
            let tri = |i: i32| if i < n / 4 { i as f32 / (n / 4) as f32 } else { 1.0 - (i - n / 4) as f32 / (n / 4) as f32 };
            if i < n / 2 { tri(i) } else { -tri(i - n / 2) }
        }
    }
}

/// `EvalWaveForm` (`WAVEVALUE` at tess.shaderTime `time` seconds).
pub fn eval_wave(w: &Wave, time: f32) -> f32 {
    let index = ((w.phase + time * w.frequency) * FUNCTABLE_SIZE as f32) as i32;
    w.base + func_table(w.func, index) * w.amplitude
}

/// A stage's `tcMod`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TcMod {
    Rotate(f32),
    Stretch(Wave),
    Scroll([f32; 2]),
    Scale([f32; 2]),
}

/// A stage's `rgbGen`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RgbGen {
    /// vertex / exactVertex (the mark's modulate)
    Vertex,
    /// identity
    Identity,
    Wave(Wave),
}

/// One stage of a mark shader.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkStage {
    pub image: String,
    pub clamp: bool,
    pub blend: MarkBlend,
    pub rgb_gen: RgbGen,
    pub tc_mods: Vec<TcMod>,
}

/// An affine texture-coordinate transform: s' = m[0]·(s, t, 1), t' = m[1]·(s, t, 1).
pub type TexMatrix = [[f32; 3]; 2];

const TEX_IDENTITY: TexMatrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

/// `RB_CalcTransformTexCoords` applied after `m`.
fn then(m: &TexMatrix, matrix: [[f32; 2]; 2], translate: [f32; 2]) -> TexMatrix {
    let mut out = [[0.0; 3]; 2];
    for k in 0..3 {
        let (s, t) = (m[0][k], m[1][k]);
        out[0][k] = s * matrix[0][0] + t * matrix[1][0] + if k == 2 { translate[0] } else { 0.0 };
        out[1][k] = s * matrix[0][1] + t * matrix[1][1] + if k == 2 { translate[1] } else { 0.0 };
    }
    out
}

/// The stage's tcMods in order at shader time `time` (seconds).
pub fn stage_tex_matrix(mods: &[TcMod], time: f32) -> TexMatrix {
    let mut m = TEX_IDENTITY;
    for tm in mods {
        m = match *tm {
            TcMod::Rotate(degs_per_second) => {
                // RB_CalcRotateTexCoords
                let degs = -degs_per_second * time;
                let index = (degs * (FUNCTABLE_SIZE as f32 / 360.0)) as i32;
                let sin = func_table(GenFunc::Sin, index);
                let cos = func_table(GenFunc::Sin, index + FUNCTABLE_SIZE / 4);
                then(&m, [[cos, sin], [-sin, cos]], [0.5 - 0.5 * cos + 0.5 * sin, 0.5 - 0.5 * sin - 0.5 * cos])
            }
            TcMod::Stretch(w) => {
                // RB_CalcStretchTexCoords
                let p = 1.0 / eval_wave(&w, time);
                then(&m, [[p, 0.0], [0.0, p]], [0.5 - 0.5 * p, 0.5 - 0.5 * p])
            }
            TcMod::Scroll(speed) => {
                // RB_CalcScrollTexCoords: adjust - floor( adjust )
                let a = [speed[0] * time, speed[1] * time].map(|x| x - x.floor());
                then(&m, [[1.0, 0.0], [0.0, 1.0]], a)
            }
            TcMod::Scale(sc) => then(&m, [[sc[0], 0.0], [0.0, sc[1]]], [0.0, 0.0]),
        };
    }
    m
}

/// `RB_CalcWaveColor` (identityLight and the hardware overbright cancel:
/// the port draws gamma-space colour 1:1).
pub fn wave_color(w: &Wave, time: f32) -> f32 {
    eval_wave(w, time).clamp(0.0, 1.0)
}

fn parse_wave(t: &[String]) -> Option<Wave> {
    let func = match t.first()?.as_str() {
        "sin" => GenFunc::Sin,
        "triangle" => GenFunc::Triangle,
        "square" => GenFunc::Square,
        "sawtooth" => GenFunc::Sawtooth,
        "inversesawtooth" => GenFunc::InverseSawtooth,
        _ => return None,
    };
    let n: Vec<f32> = t[1..].iter().filter_map(|x| x.parse().ok()).collect();
    (n.len() >= 4).then(|| Wave { func, base: n[0], amplitude: n[1], phase: n[2], frequency: n[3] })
}

/// tr_shader.c `ParseStage` for what mark shaders use: the stages of
/// `name` (map / clampMap, blendFunc, rgbGen, tcMod), None when the
/// script does not define it.
pub fn parse_mark_stages(text: &str, name: &str) -> Option<Vec<MarkStage>> {
    let mut depth = 0;
    let mut cur: Option<String> = None;
    let mut stages: Vec<MarkStage> = Vec::new();
    let mut found = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        for piece in line.split_inclusive(['{', '}']) {
            let body = piece.trim_end_matches(['{', '}']).trim();
            let toks: Vec<String> = body.split_whitespace().map(|t| t.to_lowercase()).collect();
            if !toks.is_empty() {
                if depth == 0 {
                    cur = Some(toks[0].clone());
                } else if depth == 2 && cur.as_deref() == Some(name) {
                    if let Some(st) = stages.last_mut() {
                        match toks[0].as_str() {
                            "map" | "clampmap" => {
                                st.image = toks.get(1).cloned().unwrap_or_default();
                                st.clamp = toks[0] == "clampmap";
                            }
                            "blendfunc" => {
                                let a: Vec<&str> = toks[1..].iter().map(|s| s.as_str()).collect();
                                st.blend = match a.as_slice() {
                                    ["add"] | ["gl_one", "gl_one"] | ["gl_src_alpha", "gl_one"] => MarkBlend::Add,
                                    ["filter"] | ["gl_dst_color", "gl_zero"] | ["gl_zero", "gl_src_color"] => MarkBlend::Filter,
                                    ["gl_zero", "gl_one_minus_src_color"] => MarkBlend::Subtract,
                                    _ => MarkBlend::Mix,
                                };
                            }
                            // the last rgbGen of a stage wins
                            "rgbgen" => {
                                st.rgb_gen = match toks.get(1).map(|s| s.as_str()) {
                                    Some("wave") => parse_wave(&toks[2..]).map(RgbGen::Wave).unwrap_or(RgbGen::Identity),
                                    Some("vertex") | Some("exactvertex") => RgbGen::Vertex,
                                    _ => RgbGen::Identity,
                                }
                            }
                            "tcmod" => {
                                let f = |i: usize| toks.get(i).and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
                                match toks.get(1).map(|s| s.as_str()) {
                                    Some("rotate") => st.tc_mods.push(TcMod::Rotate(f(2))),
                                    Some("stretch") => {
                                        if let Some(w) = parse_wave(&toks[2..]) {
                                            st.tc_mods.push(TcMod::Stretch(w));
                                        }
                                    }
                                    Some("scroll") => st.tc_mods.push(TcMod::Scroll([f(2), f(3)])),
                                    Some("scale") => st.tc_mods.push(TcMod::Scale([f(2), f(3)])),
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if piece.ends_with('{') {
                depth += 1;
                if depth == 2 && cur.as_deref() == Some(name) {
                    // a stage without rgbGen is identity (or vertex for marks drawn with the modulate)
                    stages.push(MarkStage { image: String::new(), clamp: false, blend: MarkBlend::Mix, rgb_gen: RgbGen::Identity, tc_mods: Vec::new() });
                }
            } else if piece.ends_with('}') {
                depth -= 1;
                if depth == 0 {
                    if cur.as_deref() == Some(name) {
                        found = true;
                        break;
                    }
                    cur = None;
                }
            }
        }
        if found {
            break;
        }
    }
    found.then_some(stages)
}

/// Whether a mark shader needs the per-frame stage path.
fn animated(stages: &[MarkStage]) -> bool {
    stages.len() > 1 || stages.iter().any(|s| !s.tc_mods.is_empty() || matches!(s.rgb_gen, RgbGen::Wave(_)))
}

const MARK_STAGE_SHADER: &str = r#"
shader_type spatial;
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode unshaded, cull_disabled, depth_draw_never, skip_vertex_transform, BLEND;
uniform sampler2D tex : filter_linear, REPEAT;
// RB_CalcTransformTexCoords: s' = dot(tc_s, (s, t, 1)), t' = dot(tc_t, (s, t, 1))
uniform vec3 tc_s = vec3(1.0, 0.0, 0.0);
uniform vec3 tc_t = vec3(0.0, 1.0, 0.0);
// rgbGen: the vertex colour (vertex) or this (identity, wave)
uniform bool use_vertex = true;
uniform vec4 stage_color = vec4(1.0);
void vertex() {
    VERTEX = (MODELVIEW_MATRIX * vec4(VERTEX, 1.0)).xyz * 0.997;
    UV = vec2(dot(tc_s, vec3(UV, 1.0)), dot(tc_t, vec3(UV, 1.0)));
}
void fragment() {
    vec4 col = use_vertex ? COLOR : vec4(stage_color.rgb, COLOR.a);
    vec4 c = texture(tex, UV) * col;
    BODY
}
"#;

/// The ShaderMaterials of an animated mark shader: one per stage, chained
/// with `next_pass` (multi-pass like the renderer's stages).
fn stage_materials(lib: &mut Library, stages: &[MarkStage]) -> Option<Vec<Gd<ShaderMaterial>>> {
    let mut out: Vec<Gd<ShaderMaterial>> = Vec::new();
    for st in stages {
        let tex = lib.texture(&st.image)?;
        let body = match st.blend {
            MarkBlend::Mix => ("blend_mix", "ALBEDO = pow(clamp(c.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP)); ALPHA = c.a;"),
            MarkBlend::Add => ("blend_add", "ALBEDO = pow(clamp(c.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP));"),
            MarkBlend::Subtract => ("blend_mix", "ALBEDO = vec3(0.0); ALPHA = clamp(max(c.r, max(c.g, c.b)), 0.0, 1.0);"),
            MarkBlend::Filter => ("blend_mix", "ALBEDO = vec3(0.0); ALPHA = clamp(1.0 - max(c.r, max(c.g, c.b)), 0.0, 1.0);"),
        };
        let mut sh = Shader::new_gd();
        let code = MARK_STAGE_SHADER.replace("BLEND", body.0).replace("BODY", body.1).replace("REPEAT", if st.clamp { "repeat_disable" } else { "repeat_enable" });
        sh.set_code(&code);
        let mut m = ShaderMaterial::new_gd();
        m.set_shader(&sh);
        m.set_shader_parameter("tex", &tex.to_variant());
        m.set_shader_parameter("use_vertex", &matches!(st.rgb_gen, RgbGen::Vertex).to_variant());
        m.set_render_priority(-1);
        out.push(m);
    }
    for i in (1..out.len()).rev() {
        let next = out[i].clone();
        out[i - 1].set_next_pass(&next);
    }
    Some(out)
}

/// This frame's uniforms of an animated mark shader's stages.
fn update_stage_materials(mats: &mut [Gd<ShaderMaterial>], stages: &[MarkStage], time: f32) {
    for (m, st) in mats.iter_mut().zip(stages) {
        let tm = stage_tex_matrix(&st.tc_mods, time);
        m.set_shader_parameter("tc_s", &Vector3::new(tm[0][0], tm[0][1], tm[0][2]).to_variant());
        m.set_shader_parameter("tc_t", &Vector3::new(tm[1][0], tm[1][1], tm[1][2]).to_variant());
        let c = match st.rgb_gen {
            RgbGen::Wave(w) => wave_color(&w, time),
            _ => 1.0,
        };
        m.set_shader_parameter("stage_color", &Color::from_rgba(c, c, c, 1.0).to_variant());
    }
}

const MARK_SHADER: &str = r#"
shader_type spatial;
// r_gamma 1.3 as in the world shaders: gamma-space colour -> linear
const float R_GAMMA_EXP = 2.2 / 1.3;
render_mode unshaded, cull_disabled, depth_draw_never, skip_vertex_transform, BLEND;
uniform sampler2D tex : filter_linear, repeat_disable;
void vertex() {
    // polygonOffset: pull the poly a little towards the camera (view space)
    VERTEX = (MODELVIEW_MATRIX * vec4(VERTEX, 1.0)).xyz * 0.997;
}
void fragment() {
    vec4 c = texture(tex, UV) * COLOR;
    BODY
}
"#;

fn mark_material(lib: &mut Library, shader: &str, blend: MarkBlend) -> Option<Gd<ShaderMaterial>> {
    let tex = lib.texture(shader)?;
    let (mode, body) = match blend {
        MarkBlend::Mix => ("blend_mix", "ALBEDO = pow(clamp(c.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP)); ALPHA = c.a;"),
        MarkBlend::Add => ("blend_add", "ALBEDO = pow(clamp(c.rgb, 0.0, 1.0), vec3(R_GAMMA_EXP));"),
        // dst * (1 - src) for the grey mark images, as an alpha blend of black
        // (Godot's multiply blend does not keep dst in the Compatibility renderer)
        MarkBlend::Subtract => ("blend_mix", "ALBEDO = vec3(0.0); ALPHA = clamp(max(c.r, max(c.g, c.b)), 0.0, 1.0);"),
        // dst * src
        MarkBlend::Filter => ("blend_mix", "ALBEDO = vec3(0.0); ALPHA = clamp(1.0 - max(c.r, max(c.g, c.b)), 0.0, 1.0);"),
    };
    let mut sh = Shader::new_gd();
    let code = MARK_SHADER.replace("BLEND", mode).replace("BODY", body);
    sh.set_code(&code);
    let mut m = ShaderMaterial::new_gd();
    m.set_shader(&sh);
    m.set_shader_parameter("tex", &tex.to_variant());
    // under the particles and trails drawn on top of the walls
    m.set_render_priority(-1);
    Some(m)
}

pub struct Marks {
    root: Gd<Node3D>,
    mesh: Gd<ImmediateMesh>,
    _mi: Gd<MeshInstance3D>,
    /// The world's draw surfaces for `R_MarkFragments` (None: no marks).
    world: Option<MarkWorld>,
    /// `cg_activeMarkPolys`, newest first.
    active: VecDeque<MarkPoly>,
    /// This frame's temporary polys (trap_R_AddPolyToScene of temporary marks).
    temp: Vec<MarkPoly>,
    materials: HashMap<String, Option<Gd<ShaderMaterial>>>,
    blends: HashMap<String, MarkBlend>,
    /// animated (multi-stage / tcMod / rgbGen wave) shaders: their stages
    /// and stage materials (the first is the surface's material)
    stages: HashMap<String, Option<(Vec<MarkStage>, Vec<Gd<ShaderMaterial>>)>>,
    /// cg.time of the last update (ms)
    now: i32,
}

impl Marks {
    /// `CG_InitMarkPolys`.
    pub fn new(parent: &Gd<Node3D>) -> Self {
        let mut root = Node3D::new_alloc();
        root.set_name("Marks");
        parent.clone().add_child(&root);
        let mesh = ImmediateMesh::new_gd();
        let mut mi = MeshInstance3D::new_alloc();
        mi.set_mesh(&mesh);
        mi.set_cast_shadows_setting(ShadowCastingSetting::OFF);
        root.add_child(&mi);
        Marks { root, mesh, _mi: mi, world: None, active: VecDeque::new(), temp: Vec::new(), materials: HashMap::new(), blends: HashMap::new(), stages: HashMap::new(), now: 0 }
    }

    /// The map's draw surfaces (from its BSP) the marks are clipped to.
    pub fn set_world(&mut self, world: MarkWorld) {
        self.world = Some(world);
        self.active.clear();
    }

    pub fn count(&self) -> (usize, usize) {
        (self.active.len(), self.temp.len())
    }

    /// `CG_FreeMarkPoly` + `CG_AllocMark`: always succeeds, freeing the
    /// oldest marks (every poly of the oldest time) when all are in use.
    fn alloc_mark(&mut self, mark: MarkPoly) {
        if self.active.len() >= MAX_MARK_POLYS {
            if let Some(time) = self.active.back().map(|m| m.time) {
                while self.active.back().is_some_and(|m| m.time == time) {
                    self.active.pop_back();
                }
            }
        }
        self.active.push_front(mark);
    }

    fn blend(&mut self, lib: &mut Library, shader: &str) -> MarkBlend {
        let key = shader.to_lowercase();
        if let Some(b) = self.blends.get(&key) {
            return *b;
        }
        let scripts: Vec<String> = lib.pk3.files().filter(|f| f.to_lowercase().starts_with("scripts/") && f.to_lowercase().ends_with(".shader")).cloned().collect();
        let mut b = MarkBlend::Mix;
        for s in scripts {
            if let Some(found) = lib.text(&s).and_then(|t| blend_of_script(&t, &key)) {
                b = found;
            }
        }
        self.blends.insert(key, b);
        b
    }

    /// The stages of an animated mark shader (None: drawn the simple way).
    fn animated_stages(&mut self, lib: &mut Library, shader: &str) -> Option<&mut (Vec<MarkStage>, Vec<Gd<ShaderMaterial>>)> {
        let key = shader.to_lowercase();
        if !self.stages.contains_key(&key) {
            let scripts: Vec<String> = lib.pk3.files().filter(|f| f.to_lowercase().starts_with("scripts/") && f.to_lowercase().ends_with(".shader")).cloned().collect();
            let mut found = None;
            for s in scripts {
                if let Some(st) = lib.text(&s).and_then(|t| parse_mark_stages(&t, &key)) {
                    found = Some(st);
                }
            }
            let entry = found.filter(|st| animated(st)).and_then(|st| stage_materials(lib, &st).map(|m| (st, m)));
            self.stages.insert(key.clone(), entry);
        }
        self.stages.get_mut(&key).and_then(|e| e.as_mut())
    }

    /// `CG_ImpactMark`: origin within a unit of the plane, dir its normal.
    /// Temporary marks are drawn this frame only, never stored.
    pub fn impact_mark(&mut self, shader: &str, origin: Vec3, dir: Vec3, orientation: f32, rgba: [f32; 4], alpha_fade: bool, radius: f32, temporary: bool, now: i32) {
        if !CG_ADD_MARKS {
            return;
        }
        // (the C errors out on radius <= 0)
        let Some(world) = self.world.as_ref().filter(|_| radius > 0.0) else { return };
        // create the texture axis and the full polygon
        let (points, axis) = impact_mark_polygon(&origin, &dir, orientation, radius);
        let tex_coord_scale = 0.5 * 1.0 / radius;
        // get the fragments
        let projection = qm::scale(&dir, -20.0);
        let (mark_points, fragments) = world.mark_fragments(&points, &projection, MAX_MARK_POINTS, MAX_MARK_FRAGMENTS);
        let colors = rgba.map(|c| (c * 255.0) as u8);
        for mf in fragments {
            // we have an upper limit on the complexity of polygons that we store persistantly
            let n = mf.num_points.min(MAX_VERTS_ON_POLY);
            let verts: Vec<PolyVert> = mark_points[mf.first_point..mf.first_point + n]
                .iter()
                .map(|p| {
                    let delta = qm::sub(p, &origin);
                    PolyVert { xyz: *p, st: [0.5 + qm::dot(&delta, &axis[1]) * tex_coord_scale, 0.5 + qm::dot(&delta, &axis[2]) * tex_coord_scale], modulate: colors }
                })
                .collect();
            let mark = MarkPoly { time: now, shader: shader.to_string(), alpha_fade, color: rgba, verts };
            if temporary {
                // if it is a temporary (shadow) mark, add it immediately and forget about it
                self.temp.push(mark);
            } else {
                // otherwise save it persistantly
                self.alloc_mark(mark);
            }
        }
    }

    /// `CG_ImpactMark` as `CG_MissileHitWall` calls it: white, no alpha
    /// fade, stored.
    pub fn impact(&mut self, _lib: &mut Library, shader: &str, origin: Vec3, dir: Vec3, orientation: f32, radius: f32, now: i32) {
        self.impact_mark(shader, origin, dir, orientation, [1.0; 4], false, radius, false, now);
    }

    /// `CG_PlayerShadow` and `CG_PlayerSplash` for every player and corpse
    /// drawn (`CG_Player`, `CG_Corpse`), plus the creep under a
    /// trapper-locked player (`EF_BLOBLOCKED` in `CG_Player`).
    fn player_marks(&mut self, w: &World, now: i32) {
        use trem_sim::gen::enums::{contents, SpectatorState, Stat};
        for i in 0..w.num_entities {
            let e = &w.ents[i];
            if !e.inuse || e.free_after_event {
                continue;
            }
            let (origin, class, yaw) = match e.etype {
                EType::Player if e.linked && i < trem_sim::kernel::world::MAX_CLIENTS => {
                    let c = &w.clients[i];
                    if c.spectator != SpectatorState::Not as i32 {
                        continue;
                    }
                    (c.ps.origin, c.ps.stats[Stat::Class as usize], c.ps.viewangles[1])
                }
                EType::Corpse => (e.origin, e.generic1, e.angles[1]),
                _ => continue,
            };
            // EF_WALLCLIMB / EF_WALLCLIMBCEILING with angles2 = the surface normal
            let surf_normal = if e.etype == EType::Player { trem_sim::kernel::bg::client_normal(&w.clients[i].ps) } else { [0.0, 0.0, 1.0] };
            let shadow_scale = trem_sim::gen::classes::ClassId::from_num(class.clamp(0, 255) as u8).map(|c| c.cold().shadow_scale).unwrap_or(1.0);
            // CG_PlayerShadow
            let (mut mins, mut maxs, ..) = trem_sim::kernel::bg::class_bbox(class);
            mins[2] = 0.0;
            maxs[2] = 2.0;
            if CG_SHADOWS != 0 {
                // send a trace down from the player to the ground
                let end = qm::ma(&origin, -SHADOW_DISTANCE, &surf_normal);
                let tr = w.cm.box_trace(&origin, &end, Some(&mins), Some(&maxs), k::MASK_PLAYERSOLID);
                // no shadow if too high; no mark for stencil or projection shadows
                if !(tr.fraction == 1.0 || tr.startsolid || tr.allsolid) && CG_SHADOWS == 1 {
                    // fade the shadow out with height
                    let alpha = 1.0 - tr.fraction;
                    self.impact_mark("gfx/marks/shadow", tr.endpos, tr.plane.normal, yaw, [0.0, 0.0, 0.0, alpha], false, 24.0 * shadow_scale, true, now);
                }
            }
            // CG_PlayerSplash: a wake where the feet are in liquid and the head is not
            if CG_SHADOWS != 0 {
                let liquid = (contents::WATER | contents::SLIME | contents::LAVA) as i32;
                let (bmins, ..) = trem_sim::kernel::bg::class_bbox(class);
                let mut end = origin;
                end[2] += bmins[2];
                let mut start = origin;
                start[2] += 32.0;
                if w.cm.point_contents(&end) & liquid != 0 && w.cm.point_contents(&start) & (liquid | contents::SOLID as i32) == 0 {
                    let tr = w.cm.box_trace(&start, &end, None, None, liquid);
                    if tr.fraction != 1.0 {
                        self.impact_mark("gfx/marks/wake", tr.endpos, tr.plane.normal, yaw, [1.0; 4], false, 32.0 * shadow_scale, true, now);
                    }
                }
            }
            // es->eFlags & EF_BLOBLOCKED (SS_BLOBLOCKED): creep under the caught player
            if e.etype == EType::Player && w.clients[i].ps.stats[Stat::State as usize] & k::SS_BLOBLOCKED != 0 {
                let base = w.clients[i].ps.origin;
                let mut temp = base;
                temp[2] -= 4096.0;
                let tr = w.trace(&base, None, None, &temp, i as i32, k::MASK_SOLID);
                self.impact_mark("creep", tr.endpos, [0.0, 0.0, 1.0], 0.0, [1.0; 4], false, 32.0, true, now);
            }
        }
    }

    /// `CG_Creep` for every alien structure.
    fn creep(&mut self, w: &World, now: i32) {
        for i in 0..w.num_entities {
            let e = &w.ents[i];
            if !e.inuse || e.etype != EType::Buildable || e.team != trem_sim::gen::enums::Team::Aliens {
                continue;
            }
            let Some(b) = e.buildable else { continue };
            let scale_up = b.def().build_time.max(1);
            // es->time >= 0 grows over the build time, < 0 recedes
            let time = e.s_time;
            let now_s = w.level.time;
            let frac = if time >= 0 && e.spawned {
                1.0
            } else if time >= 0 {
                let msec = now_s - time;
                if (0..scale_up).contains(&msec) { msec as f32 / scale_up as f32 } else { 1.0 }
            } else {
                let msec = now_s + time;
                if (0..CREEP_SCALEDOWN_TIME).contains(&msec) { 1.0 - msec as f32 / CREEP_SCALEDOWN_TIME as f32 } else { 0.0 }
            };
            let end = qm::ma(&e.origin, -CREEP_DISTANCE, &e.origin2);
            let tr = w.trace(&e.origin, None, None, &end, i as i32, k::MASK_PLAYERSOLID);
            let size = CREEP_SIZE * frac;
            if size > 0.0 && tr.fraction < 1.0 {
                self.impact_mark("creep", tr.endpos, e.origin2, 0.0, [1.0; 4], false, size, true, now);
            }
        }
    }

    /// `CG_AddMarks` (fade and expiry of the stored polys), this frame's
    /// temporary marks, then every poly to the mesh.
    pub fn update(&mut self, w: &World, lib: &mut Library, now: i32) {
        self.now = now;
        self.temp.clear();
        self.player_marks(w, now);
        self.creep(w, now);
        // see if it is time to completely remove it
        self.active.retain(|m| now <= m.time + MARK_TOTAL_TIME);
        // fade all marks out with time
        for mp in self.active.iter_mut() {
            let t = mp.time + MARK_TOTAL_TIME - now;
            if t < MARK_FADE_TIME {
                let fade = (255 * t / MARK_FADE_TIME) as u8;
                for v in mp.verts.iter_mut() {
                    if mp.alpha_fade {
                        v.modulate[3] = fade;
                    } else {
                        for c in 0..3 {
                            v.modulate[c] = (mp.color[c] * fade as f32) as u8;
                        }
                    }
                }
            }
        }
        self.draw(lib);
    }

    /// trap_R_AddPolyToScene for every poly: one mesh surface per shader.
    fn draw(&mut self, lib: &mut Library) {
        self.mesh.clear_surfaces();
        let mut by_shader: Vec<(String, Vec<usize>)> = Vec::new();
        let n_active = self.active.len();
        for idx in 0..n_active + self.temp.len() {
            let s = if idx < n_active { &self.active[idx].shader } else { &self.temp[idx - n_active].shader };
            match by_shader.iter_mut().find(|(x, _)| x == s) {
                Some((_, v)) => v.push(idx),
                None => by_shader.push((s.clone(), vec![idx])),
            }
        }
        // tess.shaderTime: the refdef time in seconds
        let time = self.now as f32 * 0.001;
        for (shader, polys) in by_shader {
            if let Some((stages, mats)) = self.animated_stages(lib, &shader) {
                update_stage_materials(mats, stages, time);
                let first = mats[0].clone();
                self.materials.insert(shader.clone(), Some(first));
            }
            if !self.materials.contains_key(&shader) {
                let blend = self.blend(lib, &shader);
                let m = mark_material(lib, &shader, blend);
                self.materials.insert(shader.clone(), m);
            }
            let Some(mat) = self.materials.get(&shader).cloned().flatten() else { continue };
            self.mesh.surface_begin_ex(PrimitiveType::TRIANGLES).material(&mat).done();
            let mut any = false;
            for idx in polys {
                let p = if idx < n_active { &self.active[idx] } else { &self.temp[idx - n_active] };
                for t in 1..p.verts.len().saturating_sub(1) {
                    for v in [&p.verts[0], &p.verts[t], &p.verts[t + 1]] {
                        self.mesh.surface_set_color(Color::from_rgba8(v.modulate[0], v.modulate[1], v.modulate[2], v.modulate[3]));
                        self.mesh.surface_set_uv(Vector2::new(v.st[0], v.st[1]));
                        self.mesh.surface_add_vertex(q2g(&v.xyz));
                        any = true;
                    }
                }
            }
            if !any {
                // an empty surface is an error: give it a degenerate triangle
                for _ in 0..3 {
                    self.mesh.surface_add_vertex(Vector3::ZERO);
                }
            }
            self.mesh.surface_end();
        }
        let _ = &self.root;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_shader_blends() {
        let text = "gfx/marks/shadow\n{\n  polygonOffset\n  {\n    map gfx/marks/shadow.tga\n    blendFunc GL_SRC_ALPHA GL_ONE_MINUS_SRC_ALPHA\n  }\n}\n\
gfx/marks/bullet_mrk\n{\n  polygonOffset\n  {\n    map gfx/marks/bullet_mrk.tga\n    blendFunc GL_ZERO GL_ONE_MINUS_SRC_COLOR\n    rgbGen exactVertex\n  }\n}\n\
gfx/marks/wake\n{\n  {\n    clampmap gfx/marks/splash.tga\n    blendFunc GL_ONE GL_ONE\n  }\n  {\n    blendFunc GL_ZERO GL_ONE_MINUS_SRC_COLOR\n  }\n}\ncreep\n{\n  {\n    clampmap gfx/misc/creep.tga\n    blendfunc blend\n  }\n}\n";
        assert_eq!(blend_of_script(text, "gfx/marks/shadow"), Some(MarkBlend::Mix));
        assert_eq!(blend_of_script(text, "gfx/marks/bullet_mrk"), Some(MarkBlend::Subtract));
        assert_eq!(blend_of_script(text, "gfx/marks/wake"), Some(MarkBlend::Add));
        assert_eq!(blend_of_script(text, "creep"), Some(MarkBlend::Mix));
        assert_eq!(blend_of_script(text, "gfx/marks/burn_mrk"), None);
    }

    const WAKE: &str = "gfx/marks/wake\n{\n  polygonOffset\n  {\n    clampmap gfx/marks/splash.tga\n    blendFunc GL_ONE GL_ONE\n    rgbGen vertex\n    tcmod rotate 250\n    tcMod stretch sin .9 0.1 0 0.7\n    rgbGen wave sin .7 .3 .25 .5\n  }\n  {\n    clampmap gfx/marks/splash.tga\n    blendFunc GL_ONE GL_ONE\n    rgbGen vertex\n    tcmod rotate -230\n    tcMod stretch sin .9 0.05 0 0.9\n    rgbGen wave sin .7 .3 .25 .4\n  }\n}\n";

    #[test]
    fn wake_stages_parse() {
        let st = parse_mark_stages(WAKE, "gfx/marks/wake").unwrap();
        assert_eq!(st.len(), 2);
        let sin = |b, a, p, f| Wave { func: GenFunc::Sin, base: b, amplitude: a, phase: p, frequency: f };
        assert_eq!(st[0].image, "gfx/marks/splash.tga");
        assert!(st[0].clamp);
        assert_eq!(st[0].blend, MarkBlend::Add);
        // the later rgbGen wins over "rgbGen vertex"
        assert_eq!(st[0].rgb_gen, RgbGen::Wave(sin(0.7, 0.3, 0.25, 0.5)));
        assert_eq!(st[0].tc_mods, vec![TcMod::Rotate(250.0), TcMod::Stretch(sin(0.9, 0.1, 0.0, 0.7))]);
        assert_eq!(st[1].tc_mods, vec![TcMod::Rotate(-230.0), TcMod::Stretch(sin(0.9, 0.05, 0.0, 0.9))]);
        assert!(animated(&st));
        assert!(parse_mark_stages(WAKE, "gfx/marks/shadow").is_none());
    }

    #[test]
    fn tcmod_and_wave_follow_the_renderer() {
        // tr.sinTable[ i ] = sin( DEG2RAD( i * 360 / 1023 ) )
        assert_eq!(func_table(GenFunc::Sin, 0), 0.0);
        assert!((func_table(GenFunc::Sin, 256) - (256.0f64 * 360.0 / 1023.0).to_radians().sin() as f32).abs() < 1e-6);
        assert_eq!(func_table(GenFunc::Triangle, 256), 1.0);
        assert_eq!(func_table(GenFunc::Triangle, 768), -1.0);
        let w = Wave { func: GenFunc::Sin, base: 0.7, amplitude: 0.3, phase: 0.25, frequency: 0.5 };
        // phase .25 at t = 0: index 256, near the peak
        assert!((wave_color(&w, 0.0) - (0.7 + 0.3 * func_table(GenFunc::Sin, 256))).abs() < 1e-6);
        // at t = 1 s: index (0.25 + 0.5) * 1024 = 768, the trough
        assert!((eval_wave(&w, 1.0) - (0.7 + 0.3 * func_table(GenFunc::Sin, 768))).abs() < 1e-6);
        // no tcMods: identity; rotate keeps the centre (0.5, 0.5) fixed
        assert_eq!(stage_tex_matrix(&[], 3.0), TEX_IDENTITY);
        for t in [0.0f32, 0.37, 1.5] {
            let m = stage_tex_matrix(&[TcMod::Rotate(250.0), TcMod::Stretch(Wave { func: GenFunc::Sin, base: 0.9, amplitude: 0.1, phase: 0.0, frequency: 0.7 })], t);
            let c = [m[0][0] * 0.5 + m[0][1] * 0.5 + m[0][2], m[1][0] * 0.5 + m[1][1] * 0.5 + m[1][2]];
            assert!((c[0] - 0.5).abs() < 1e-5 && (c[1] - 0.5).abs() < 1e-5, "{c:?}");
        }
        // rotate 90 degrees a second, after one second: index -256, (s, t) -> (t, 1 - s) turned
        let m = stage_tex_matrix(&[TcMod::Rotate(90.0)], 1.0);
        let (sin, cos) = (func_table(GenFunc::Sin, -256), func_table(GenFunc::Sin, -256 + 256));
        assert_eq!(m, [[cos, -sin, 0.5 - 0.5 * cos + 0.5 * sin], [sin, cos, 0.5 - 0.5 * sin - 0.5 * cos]]);
        // stretch by 1 / wave about the centre; scroll wraps to [0, 1)
        let m = stage_tex_matrix(&[TcMod::Scroll([0.75, -0.5])], 3.0);
        assert_eq!((m[0][2], m[1][2]), (0.25, 0.5));
    }
}
