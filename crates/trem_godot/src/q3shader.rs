//! The full shader-script language of the renderer (tr_shader.c ParseShader /
//! ParseStage / ParseDeform / ParseTexMod / ParseWaveForm / ParseSkyParms),
//! for drawing map surfaces stage by stage as the original does: every
//! stage's image or animMap, blendFunc, alphaFunc, rgbGen, alphaGen, tcGen,
//! tcMods, depthFunc, plus the shader's deformVertexes, polygonOffset, sort,
//! cull, skyparms and fogparms. (assets.rs keeps the short summary the
//! model, mark and particle code read.)

use std::collections::HashMap;

/// tr_local.h genFunc_t
pub const GF_SIN: i32 = 0;
pub const GF_SQUARE: i32 = 1;
pub const GF_TRIANGLE: i32 = 2;
pub const GF_SAWTOOTH: i32 = 3;
pub const GF_INVERSE_SAWTOOTH: i32 = 4;
pub const GF_NOISE: i32 = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wave {
    pub func: i32,
    pub base: f32,
    pub amp: f32,
    pub phase: f32,
    pub freq: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TcMod {
    Scale(f32, f32),
    Scroll(f32, f32),
    /// degrees per second
    Rotate(f32),
    /// base amplitude phase frequency (`tcMod turb`)
    Turb(Wave),
    Stretch(Wave),
    /// m00 m01 m10 m11 t0 t1
    Transform([f32; 6]),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TcGen {
    #[default]
    Base,
    Lightmap,
    Environment,
    /// `tcGen vector ( sx sy sz ) ( tx ty tz )`
    Vector([f32; 3], [f32; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RgbGen {
    IdentityLighting,
    Identity,
    Const([f32; 3]),
    Wave(Wave),
    Vertex,
    ExactVertex,
    OneMinusVertex,
    Entity,
    OneMinusEntity,
    LightingDiffuse,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AlphaGen {
    /// the stage's colour source alpha (ALPHAGEN_IDENTITY / SKIP)
    #[default]
    Identity,
    Const(f32),
    Wave(Wave),
    Vertex,
    OneMinusVertex,
    LightingSpecular,
    Entity,
    /// `alphaGen portal <range>`: opaque here (no portals are drawn)
    Portal(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// the frames (one unless animMap) and the animMap frequency
    Image(Vec<String>, f32),
    Lightmap,
    White,
}

/// GL blend factors as the shader names them, lower case (`gl_one`, ...).
pub type BlendFunc = (String, String);

#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub source: Source,
    pub clamp: bool,
    /// None: no blending (also GL_ONE GL_ZERO)
    pub blend: Option<BlendFunc>,
    /// 0 none, 1 GT0, 2 LT128, 3 GE128
    pub alpha_func: u8,
    pub depth_write: bool,
    pub depth_equal: bool,
    pub rgb_gen: RgbGen,
    pub alpha_gen: AlphaGen,
    pub tc_gen: TcGen,
    pub tc_mods: Vec<TcMod>,
    pub detail: bool,
    /// an explicit `depthWrite` (a blended stage writes depth only then)
    pub explicit_depth_write: bool,
}

impl Stage {
    pub fn is_lightmap(&self) -> bool {
        self.source == Source::Lightmap
    }
    /// A blend that multiplies what is under it by the stage (the light map
    /// / texture modulate of a two-stage surface).
    pub fn is_filter(&self) -> bool {
        matches!(self.blend.as_ref().map(|(s, d)| (s.as_str(), d.as_str())), Some(("gl_dst_color", "gl_zero")) | Some(("gl_zero", "gl_src_color")))
    }
    pub fn animated(&self) -> bool {
        matches!(&self.source, Source::Image(f, _) if f.len() > 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deform {
    /// spread, wave: along the normal
    Wave(f32, Wave),
    /// bulge width height speed
    Bulge(f32, f32, f32),
    /// move x y z wave
    Move([f32; 3], Wave),
    AutoSprite,
    AutoSprite2,
    /// `deformVertexes normal`: lighting only, nothing to draw differently
    Normal,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkyParms {
    /// `env/foo` for env/foo_rt.tga ...; None for `-`
    pub outer: Option<String>,
    pub cloud_height: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Q3Shader {
    pub name: String,
    pub stages: Vec<Stage>,
    pub deforms: Vec<Deform>,
    /// `cull disable` / `none` / `twosided`
    pub two_sided: bool,
    pub polygon_offset: bool,
    /// explicit `sort` (tr_shader.c ParseSort), 0 when absent
    pub sort: f32,
    pub sky: Option<SkyParms>,
    pub fog: Option<([f32; 3], f32)>,
    pub nodraw: bool,
    pub editor_image: Option<String>,
    /// `q3map_sun` / `q3map_sunExt`: (intensity, direction towards the sun
    /// in Quake space)
    pub sun: Option<(f32, [f32; 3])>,
}

impl Q3Shader {
    /// FinishShader's default sort when the script has none: a decal, else
    /// blended only when the first stage blends (see-through if it writes
    /// depth), else opaque.
    pub fn effective_sort(&self) -> f32 {
        if self.sort > 0.0 {
            return self.sort;
        }
        if self.polygon_offset {
            return 4.0; // SS_DECAL
        }
        match self.stages.first() {
            Some(st) if st.blend.is_some() => {
                if st.depth_write {
                    5.0 // SS_SEE_THROUGH
                } else {
                    9.0 // SS_BLEND0
                }
            }
            _ => 3.0, // SS_OPAQUE
        }
    }
}


fn wave_func(s: &str) -> i32 {
    match s.to_ascii_lowercase().as_str() {
        "square" => GF_SQUARE,
        "triangle" => GF_TRIANGLE,
        "sawtooth" => GF_SAWTOOTH,
        "inversesawtooth" => GF_INVERSE_SAWTOOTH,
        "noise" => GF_NOISE,
        _ => GF_SIN,
    }
}

/// A line's tokens; `( 1 2 3 )` brackets may touch the numbers.
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for t in line.split_whitespace() {
        let mut cur = String::new();
        for ch in t.chars() {
            if ch == '(' || ch == ')' {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(ch.to_string());
            } else {
                cur.push(ch);
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

struct Args<'a> {
    t: &'a [String],
    i: usize,
}
impl<'a> Args<'a> {
    fn word(&mut self) -> String {
        while self.i < self.t.len() && (self.t[self.i] == "(" || self.t[self.i] == ")") {
            self.i += 1;
        }
        let w = self.t.get(self.i).cloned().unwrap_or_default();
        self.i += 1;
        w
    }
    fn f(&mut self) -> f32 {
        self.word().parse().unwrap_or(0.0)
    }
    fn wave(&mut self) -> Wave {
        let func = wave_func(&self.word());
        Wave { func, base: self.f(), amp: self.f(), phase: self.f(), freq: self.f() }
    }
    fn vec3(&mut self) -> [f32; 3] {
        [self.f(), self.f(), self.f()]
    }
}

fn normalize_blend(a: &[String]) -> Option<BlendFunc> {
    let l: Vec<String> = a.iter().map(|x| x.to_ascii_lowercase()).collect();
    let (s, d) = match l.as_slice() {
        [x, ..] if x == "add" => ("gl_one".to_string(), "gl_one".to_string()),
        [x, ..] if x == "filter" => ("gl_dst_color".to_string(), "gl_zero".to_string()),
        [x, ..] if x == "blend" => ("gl_src_alpha".to_string(), "gl_one_minus_src_alpha".to_string()),
        [s, d, ..] => (s.clone(), d.clone()),
        _ => return None,
    };
    // GL_ONE GL_ZERO is no blending at all (ParseStage clears the bits)
    if s == "gl_one" && d == "gl_zero" {
        return None;
    }
    Some((s, d))
}

/// Parses every shader of a script file into `out` (the first definition
/// of a name wins, as FindShaderInShaderText finds it first).
pub fn parse_script(text: &str, out: &mut HashMap<String, Q3Shader>) {
    // strip comments, keep one line per directive
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut in_block_comment = false;
    for raw in text.lines() {
        let mut l = String::new();
        let mut rest = raw;
        loop {
            if in_block_comment {
                match rest.find("*/") {
                    Some(p) => {
                        rest = &rest[p + 2..];
                        in_block_comment = false;
                    }
                    None => break,
                }
            } else {
                let lc = rest.find("//");
                let bc = rest.find("/*");
                match (lc, bc) {
                    (Some(a), Some(b)) if a < b => {
                        l.push_str(&rest[..a]);
                        break;
                    }
                    (_, Some(b)) => {
                        l.push_str(&rest[..b]);
                        rest = &rest[b + 2..];
                        in_block_comment = true;
                    }
                    (Some(a), None) => {
                        l.push_str(&rest[..a]);
                        break;
                    }
                    (None, None) => {
                        l.push_str(rest);
                        break;
                    }
                }
            }
        }
        // braces become their own lines
        let mut cur = String::new();
        for ch in l.chars() {
            if ch == '{' || ch == '}' {
                let t = tokens(&cur);
                if !t.is_empty() {
                    lines.push(t);
                }
                cur.clear();
                lines.push(vec![ch.to_string()]);
            } else {
                cur.push(ch);
            }
        }
        let t = tokens(&cur);
        if !t.is_empty() {
            lines.push(t);
        }
    }
    let mut i = 0;
    while i < lines.len() {
        let name = lines[i][0].clone();
        i += 1;
        if name == "{" || name == "}" || i >= lines.len() || lines[i][0] != "{" {
            continue;
        }
        i += 1;
        let mut sh = Q3Shader { name: name.to_lowercase().replace('\\', "/"), ..Default::default() };
        let mut depth = 1;
        let mut stage: Option<Stage> = None;
        let mut rgb_set = false;
        while i < lines.len() && depth > 0 {
            let toks = &lines[i];
            i += 1;
            match toks[0].as_str() {
                "{" => {
                    depth += 1;
                    if depth == 2 {
                        stage = Some(Stage {
                            source: Source::White,
                            clamp: false,
                            blend: None,
                            alpha_func: 0,
                            depth_write: true,
                            depth_equal: false,
                            rgb_gen: RgbGen::IdentityLighting,
                            alpha_gen: AlphaGen::Identity,
                            tc_gen: TcGen::Base,
                            tc_mods: Vec::new(),
                            detail: false,
                            explicit_depth_write: false,
                        });
                        rgb_set = false;
                    }
                    continue;
                }
                "}" => {
                    depth -= 1;
                    if depth == 1 {
                        if let Some(mut st) = stage.take() {
                            // ParseStage's defaults
                            if !rgb_set {
                                let src = st.blend.as_ref().map(|b| b.0.as_str()).unwrap_or("");
                                st.rgb_gen = if src.is_empty() || src == "gl_one" || src == "gl_src_alpha" { RgbGen::IdentityLighting } else { RgbGen::Identity };
                            }
                            if st.blend.is_some() && !st.explicit_depth_write {
                                st.depth_write = false;
                            }
                            if st.is_lightmap() {
                                st.tc_gen = TcGen::Lightmap;
                            }
                            if sh.stages.len() < 8 {
                                sh.stages.push(st);
                            }
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let kw = toks[0].to_ascii_lowercase();
            let mut a = Args { t: &toks[1..], i: 0 };
            if depth == 1 {
                match kw.as_str() {
                    "qer_editorimage" => sh.editor_image = Some(a.word()),
                    "cull" => {
                        let m = a.word().to_ascii_lowercase();
                        sh.two_sided = m == "disable" || m == "none" || m == "twosided";
                    }
                    "polygonoffset" => sh.polygon_offset = true,
                    "sort" => {
                        let w = a.word().to_ascii_lowercase();
                        sh.sort = match w.as_str() {
                            "portal" => 1.0,
                            "sky" => 2.0,
                            "opaque" => 3.0,
                            "decal" => 4.0,
                            "seethrough" => 5.0,
                            "banner" => 6.0,
                            "additive" => 10.0,
                            "nearest" => 16.0,
                            "underwater" => 8.0,
                            _ => w.parse().unwrap_or(0.0),
                        };
                    }
                    "skyparms" => {
                        let outer = a.word();
                        let ch: f32 = a.word().parse().unwrap_or(0.0);
                        sh.sky = Some(SkyParms { outer: (outer != "-" && !outer.is_empty()).then_some(outer), cloud_height: if ch > 0.0 { ch } else { 512.0 } });
                    }
                    "fogparms" => {
                        let c = a.vec3();
                        let d = a.f();
                        sh.fog = Some((c, d));
                    }
                    "q3map_sun" | "q3map_sunext" => {
                        let _rgb = a.vec3();
                        let i = a.f();
                        let (yaw, elev) = (a.f().to_radians(), a.f().to_radians());
                        sh.sun = Some((i, [yaw.cos() * elev.cos(), yaw.sin() * elev.cos(), elev.sin()]));
                    }
                    "surfaceparm" => {
                        if a.word().eq_ignore_ascii_case("nodraw") {
                            sh.nodraw = true;
                        }
                    }
                    "deformvertexes" => {
                        let kind = a.word().to_ascii_lowercase();
                        let d = match kind.as_str() {
                            "wave" => {
                                let div = a.f();
                                let w = a.wave();
                                Some(Deform::Wave(if div != 0.0 { 1.0 / div } else { 100.0 }, w))
                            }
                            "bulge" => Some(Deform::Bulge(a.f(), a.f(), a.f())),
                            "move" => {
                                let v = a.vec3();
                                Some(Deform::Move(v, a.wave()))
                            }
                            "autosprite" => Some(Deform::AutoSprite),
                            "autosprite2" => Some(Deform::AutoSprite2),
                            "normal" => Some(Deform::Normal),
                            _ => None,
                        };
                        if let Some(d) = d {
                            if sh.deforms.len() < 3 {
                                sh.deforms.push(d);
                            }
                        }
                    }
                    _ => {}
                }
                continue;
            }
            let Some(st) = stage.as_mut() else { continue };
            match kw.as_str() {
                "map" | "clampmap" => {
                    let w = a.word();
                    st.clamp = kw == "clampmap";
                    st.source = if w.eq_ignore_ascii_case("$lightmap") {
                        Source::Lightmap
                    } else if w.eq_ignore_ascii_case("$whiteimage") || w == "*white" {
                        Source::White
                    } else {
                        Source::Image(vec![w], 0.0)
                    };
                }
                "animmap" => {
                    let freq = a.f();
                    let mut frames = Vec::new();
                    loop {
                        let w = a.word();
                        if w.is_empty() || frames.len() >= 8 {
                            break;
                        }
                        frames.push(w);
                    }
                    if !frames.is_empty() {
                        st.source = Source::Image(frames, freq);
                    }
                }
                "videomap" => st.source = Source::White,
                "alphafunc" => {
                    st.alpha_func = match a.word().to_ascii_lowercase().as_str() {
                        "gt0" => 1,
                        "lt128" => 2,
                        _ => 3,
                    };
                }
                "depthfunc" => st.depth_equal = a.word().eq_ignore_ascii_case("equal"),
                "depthwrite" => {
                    st.depth_write = true;
                    st.explicit_depth_write = true;
                }
                "detail" => st.detail = true,
                "blendfunc" => {
                    let rest: Vec<String> = toks[1..].to_vec();
                    st.blend = normalize_blend(&rest);
                }
                "rgbgen" => {
                    rgb_set = true;
                    st.rgb_gen = match a.word().to_ascii_lowercase().as_str() {
                        "identity" => RgbGen::Identity,
                        "identitylighting" => RgbGen::IdentityLighting,
                        "const" => RgbGen::Const(a.vec3()),
                        "wave" => RgbGen::Wave(a.wave()),
                        "vertex" => RgbGen::Vertex,
                        "exactvertex" => RgbGen::ExactVertex,
                        "oneminusvertex" => RgbGen::OneMinusVertex,
                        "entity" => RgbGen::Entity,
                        "oneminusentity" => RgbGen::OneMinusEntity,
                        "lightingdiffuse" => RgbGen::LightingDiffuse,
                        _ => RgbGen::IdentityLighting,
                    };
                }
                "alphagen" => {
                    st.alpha_gen = match a.word().to_ascii_lowercase().as_str() {
                        "const" => AlphaGen::Const(a.f()),
                        "wave" => AlphaGen::Wave(a.wave()),
                        "vertex" => AlphaGen::Vertex,
                        "oneminusvertex" => AlphaGen::OneMinusVertex,
                        "lightingspecular" => AlphaGen::LightingSpecular,
                        "entity" => AlphaGen::Entity,
                        "portal" => AlphaGen::Portal(a.f()),
                        _ => AlphaGen::Identity,
                    };
                }
                "tcgen" | "texgen" => {
                    st.tc_gen = match a.word().to_ascii_lowercase().as_str() {
                        "environment" => TcGen::Environment,
                        "lightmap" => TcGen::Lightmap,
                        "vector" => {
                            let s = a.vec3();
                            TcGen::Vector(s, a.vec3())
                        }
                        _ => TcGen::Base,
                    };
                }
                "tcmod" => {
                    let m = match a.word().to_ascii_lowercase().as_str() {
                        "scale" => Some(TcMod::Scale(a.f(), a.f())),
                        "scroll" => Some(TcMod::Scroll(a.f(), a.f())),
                        "rotate" => Some(TcMod::Rotate(a.f())),
                        "turb" => {
                            // tcMod turb <base> <amp> <phase> <freq> (no function name)
                            Some(TcMod::Turb(Wave { func: GF_SIN, base: a.f(), amp: a.f(), phase: a.f(), freq: a.f() }))
                        }
                        "stretch" => Some(TcMod::Stretch(a.wave())),
                        "transform" => Some(TcMod::Transform([a.f(), a.f(), a.f(), a.f(), a.f(), a.f()])),
                        _ => None,
                    };
                    if let Some(m) = m {
                        if st.tc_mods.len() < 4 {
                            st.tc_mods.push(m);
                        }
                    }
                }
                _ => {}
            }
        }
        out.entry(sh.name.clone()).or_insert(sh);
    }
}

/// Every shader of the search path's `scripts/*.shader` (sorted by file name).
pub fn load_all(pk3: &mut crate::assets::Pk3Set) -> HashMap<String, Q3Shader> {
    let mut scripts: Vec<String> = pk3.files().filter(|f| f.to_lowercase().starts_with("scripts/") && f.to_lowercase().ends_with(".shader")).cloned().collect();
    scripts.sort_by_key(|s| s.to_lowercase());
    let mut out = HashMap::new();
    for sc in scripts {
        let Some(data) = pk3.read(&sc) else { continue };
        let text = String::from_utf8_lossy(data.as_slice()).into_owned();
        parse_script(&text, &mut out);
    }
    out
}

// ---------------------------------------------------------------------------
// Drawing: GLSL shared by the world's base shader (bspview.rs) and the
// per-stage shader, and the uniforms a stage / a shader's deforms set.

/// Functions: tr_shade_calc.c's wave tables, EvalWaveForm, the tcMods, and
/// the Quake <-> Godot axes (geom.rs q2g: x, z, -y; 32 units a metre).
pub const GLSL_COMMON: &str = r#"
const float Q3_UNITS = 32.0;
float q3_hash(float n) { return fract(sin(n * 12.9898) * 43758.5453) * 2.0 - 1.0; }
float q3_noise(float t) { float i = floor(t); float f = fract(t); f = f * f * (3.0 - 2.0 * f); return mix(q3_hash(i), q3_hash(i + 1.0), f); }
float q3_table(int f, float x) {
    x = fract(x);
    if (f == 1) { return x < 0.5 ? 1.0 : -1.0; }
    if (f == 2) { float y = x < 0.5 ? x : x - 0.5; float v = y < 0.25 ? y * 4.0 : 1.0 - (y - 0.25) * 4.0; return x < 0.5 ? v : -v; }
    if (f == 3) { return x; }
    if (f == 4) { return 1.0 - x; }
    return sin(x * 6.28318530718);
}
// EvalWaveForm: w = (base, amplitude, phase, frequency)
float q3_wave(int f, vec4 w, float t) {
    if (f == 5) { return w.x + q3_noise((t + w.z) * w.w) * w.y; }
    return q3_table(f, w.z + t * w.w) * w.y + w.x;
}
vec3 q3_pos(vec3 g) { return vec3(g.x, -g.z, g.y) * Q3_UNITS; }
vec3 q3_dir(vec3 g) { return vec3(g.x, -g.z, g.y); }
vec3 q3_gdir(vec3 q) { return vec3(q.x, q.z, -q.y); }
// R_ColorShiftLightingBytes (x2, hue-preserving clamp)
vec3 q3_shift(vec3 c) { c *= 2.0; float m = max(c.r, max(c.g, c.b)); return m > 1.0 ? c / m : c; }
vec2 q3_tcmod(int k, vec4 a, vec4 b, vec2 st, vec3 qp, float t) {
    if (k == 1) { return st * a.xy; }
    if (k == 2) { vec2 o = a.xy * t; return st + (o - floor(o)); }
    if (k == 3) {
        float r = radians(-a.x * t);
        float s = sin(r);
        float c = cos(r);
        return vec2(st.x * c - st.y * s + 0.5 - 0.5 * c + 0.5 * s, st.x * s + st.y * c + 0.5 - 0.5 * s - 0.5 * c);
    }
    if (k == 4) {
        float now = a.z + t * a.w;
        return st + vec2(sin(((qp.x + qp.z) / 1024.0 + now) * 6.28318530718), sin((qp.y / 1024.0 + now) * 6.28318530718)) * a.y;
    }
    if (k == 5) {
        float p = 1.0 / q3_wave(int(b.x), a, t);
        return st * p + vec2(0.5 - 0.5 * p);
    }
    if (k == 6) { return vec2(st.x * a.x + st.y * a.z + b.x, st.x * a.y + st.y * a.w + b.y); }
    return st;
}
// a shader's deformVertexes (RB_DeformTessGeometry), in the model's space
uniform ivec4 q3_dk;
uniform mat4 q3_da;
uniform mat4 q3_db;
// view-space pull towards the camera: polygonOffset decals and the stages
// drawn over a surface's first pass (depthFunc equal)
uniform float q3_pull = 1.0;
"#;

/// vertex() statements: the deforms (VERTEX, NORMAL, UV), and q3_cam (the
/// camera in the model's Godot space) for the stage shader's tcGens.
pub const GLSL_VERTEX_DEFORM: &str = r#"
    mat4 q3_im = inverse(MODEL_MATRIX);
    vec3 q3_cam = (q3_im * vec4(INV_VIEW_MATRIX[3].xyz, 1.0)).xyz;
    if (q3_dk != ivec4(0)) {
        vec3 q3_right = normalize((q3_im * vec4(INV_VIEW_MATRIX[0].xyz, 0.0)).xyz);
        vec3 q3_up = normalize((q3_im * vec4(INV_VIEW_MATRIX[1].xyz, 0.0)).xyz);
        vec3 q3_fwd = -normalize((q3_im * vec4(INV_VIEW_MATRIX[2].xyz, 0.0)).xyz);
        for (int k = 0; k < 4; k++) {
            int kind = q3_dk[k];
            vec4 a = q3_da[k];
            vec4 b = q3_db[k];
            if (kind == 1) {
                vec3 qp = q3_pos(VERTEX);
                float s = q3_wave(int(b.y), vec4(a.x, a.y, a.z + (qp.x + qp.y + qp.z) * b.x, a.w), TIME);
                VERTEX += NORMAL * s / Q3_UNITS;
            } else if (kind == 2) {
                VERTEX += NORMAL * sin(UV.x * a.x + TIME * a.z) * a.y / Q3_UNITS;
            } else if (kind == 3) {
                VERTEX += q3_gdir(b.xyz) * q3_wave(int(b.w), a, TIME) / Q3_UNITS;
            } else if (kind == 4) {
                int c = int(CUSTOM2.w + 0.5);
                float sl = (c == 0 || c == 3) ? 1.0 : -1.0;
                float su = (c == 0 || c == 1) ? 1.0 : -1.0;
                VERTEX = CUSTOM1.xyz - q3_right * CUSTOM1.w * sl + q3_up * CUSTOM1.w * su;
                UV = vec2((c == 1 || c == 2) ? 1.0 : 0.0, c >= 2 ? 1.0 : 0.0);
                NORMAL = -q3_fwd;
            } else if (kind == 5) {
                vec3 minor = normalize(cross(CUSTOM2.xyz - CUSTOM1.xyz, q3_fwd));
                VERTEX = (CUSTOM2.w > 0.5 ? CUSTOM2.xyz : CUSTOM1.xyz) + minor * CUSTOM1.w;
            }
        }
    }
"#;

pub const GLSL_VERTEX_PULL: &str = r#"
    // always written: a shader that writes POSITION anywhere gets no default
    // projection, and every pass of a surface must land on the same depth
    vec4 q3_vv = MODELVIEW_MATRIX * vec4(VERTEX, 1.0);
    q3_vv.xyz *= q3_pull;
    POSITION = PROJECTION_MATRIX * q3_vv;
"#;

/// How a stage composites: Godot's blend mode, and what the shader outputs
/// for it so that the framebuffer gets the GL blend's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    Opaque,
    Add,
    Mix,
    Premul,
    Mul,
}

/// (variant, output kind) of a GL blend; output kinds: 0 rgb,a; 1 rgb*a;
/// 2 rgb,1-a; 3 rgb+a; 4 rgb+1; 5 1-rgb; 6 a; 7 1-a; 8 2*rgb.
pub fn blend_variant(b: Option<&BlendFunc>) -> (Variant, i32) {
    let Some((s, d)) = b else { return (Variant::Opaque, 0) };
    match (s.as_str(), d.as_str()) {
        ("gl_one", "gl_one") => (Variant::Add, 0),
        ("gl_src_alpha", "gl_one") => (Variant::Add, 1),
        // screen-like blends (flames, sparks): light added like GL_ONE GL_ONE
        ("gl_one", "gl_one_minus_src_color") | ("gl_one_minus_dst_color", "gl_one") | ("gl_one_minus_dst_color", "gl_zero") => (Variant::Add, 0),
        ("gl_src_alpha", "gl_one_minus_src_alpha") => (Variant::Mix, 0),
        ("gl_one_minus_src_alpha", "gl_src_alpha") => (Variant::Mix, 2),
        ("gl_one", "gl_one_minus_src_alpha") => (Variant::Premul, 0),
        ("gl_dst_color", "gl_zero") | ("gl_zero", "gl_src_color") => (Variant::Mul, 0),
        ("gl_dst_color", "gl_src_alpha") => (Variant::Mul, 3),
        ("gl_dst_color", "gl_one") => (Variant::Mul, 4),
        ("gl_zero", "gl_one_minus_src_color") => (Variant::Mul, 5),
        // the framebuffer's alpha is 1 under a surface: dst * src
        ("gl_dst_color", "gl_one_minus_dst_alpha") => (Variant::Mul, 0),
        ("gl_zero", "gl_src_alpha") => (Variant::Mul, 6),
        ("gl_zero", "gl_one_minus_src_alpha") => (Variant::Mul, 7),
        ("gl_dst_color", "gl_src_color") => (Variant::Mul, 8),
        ("gl_one", "gl_zero") => (Variant::Opaque, 0),
        _ => (Variant::Mix, 0),
    }
}

/// The shader of one stage pass (tr_shade.c RB_IterateStagesGeneric for one
/// stage: ComputeColors, ComputeTexCoords, the blend), unshaded.
/// `offset`: polygonOffset, the depth moved towards the camera by the
/// fragment's depth slope too (glPolygonOffset(-1, -2) of the C).
pub fn stage_shader_code(v: Variant, depth_write: bool, offset: bool) -> String {
    let mode = match v {
        Variant::Opaque => "depth_draw_opaque".to_string(),
        Variant::Add => format!("blend_add, {}", if depth_write { "depth_draw_always" } else { "depth_draw_never" }),
        Variant::Mix => format!("blend_mix, {}", if depth_write { "depth_draw_always" } else { "depth_draw_never" }),
        Variant::Premul => format!("blend_premul_alpha, {}", if depth_write { "depth_draw_always" } else { "depth_draw_never" }),
        Variant::Mul => format!("blend_mul, {}", if depth_write { "depth_draw_always" } else { "depth_draw_never" }),
    };
    let out = match v {
        // the x2 overbright of the screen applies to what a stage adds, not to a multiply
        // trem_mul_k: the Compatibility renderer stores its HDR buffer at 1/4
        // (glow on) and scales a multiply pass's colour the same way (gfx.rs)
        Variant::Mul => "ALBEDO = pow(clamp(o.rgb, 0.0, 4.0), vec3(R_GAMMA_EXP)) * trem_mul_k;",
        Variant::Opaque => "ALBEDO = pow(clamp(o.rgb * 2.0, 0.0, 1.0), vec3(R_GAMMA_EXP));",
        // enhanced: only bright texels (bulbs, flares) are pushed over 1.0 for the
        // bloom; dim see-through layers (atcs's force fields) stay as in vanilla
        Variant::Add => "ALBEDO = pow(clamp(o.rgb * 2.0, 0.0, 1.0), vec3(R_GAMMA_EXP)); if (trem_gfx > 0.5) { ALBEDO *= 1.0 + 0.8 * smoothstep(0.55, 1.0, max(ALBEDO.r, max(ALBEDO.g, ALBEDO.b))); }",
        Variant::Mix => "ALBEDO = pow(clamp(o.rgb * 2.0, 0.0, 1.0), vec3(R_GAMMA_EXP)); ALPHA = clamp(o.a, 0.0, 1.0);",
        // GL_ONE GL_ONE_MINUS_SRC_ALPHA: the colour as is over (1 - alpha) of what is under
        Variant::Premul => "ALBEDO = pow(clamp(o.rgb * 2.0, 0.0, 1.0), vec3(R_GAMMA_EXP)); ALPHA = clamp(o.a, 0.0, 1.0);",
    };
    // the base image's mask scales what the pass adds to what is under it
    let mask = match v {
        Variant::Opaque => "",
        Variant::Mix => "o.a *= q3_m;",
        Variant::Add => "o.rgb *= q3_m;",
        Variant::Premul => "o *= q3_m;",
        Variant::Mul => "o.rgb = mix(vec3(1.0), o.rgb, q3_m);",
    };
    let mut s = String::new();
    s.push_str("shader_type spatial;\nrender_mode unshaded, cull_disabled, fog_disabled, ");
    s.push_str(&mode);
    s.push_str(";\nconst float R_GAMMA_EXP = 2.2 / 1.3;\nglobal uniform float trem_gfx;\nglobal uniform float trem_mul_k;\n");
    s.push_str(GLSL_COMMON);
    s.push_str(STAGE_UNIFORMS);
    s.push_str("void vertex() {\n");
    s.push_str(GLSL_VERTEX_DEFORM);
    s.push_str(STAGE_VERTEX);
    s.push_str(GLSL_VERTEX_PULL);
    s.push_str("}\nvoid fragment() {\n");
    s.push_str(STAGE_FRAGMENT);
    s.push_str(mask);
    s.push_str(out);
    if offset {
        s.push_str(GLSL_DEPTH_OFFSET);
    }
    s.push_str("\n}\n");
    s
}

const GLSL_DEPTH_OFFSET: &str = r#"
#if CURRENT_RENDERER == RENDERER_COMPATIBILITY
    DEPTH = FRAGCOORD.z - fwidth(FRAGCOORD.z) * 1.5 - 4.0e-6;
#else
    DEPTH = FRAGCOORD.z + fwidth(FRAGCOORD.z) * 1.5 + 4.0e-6;
#endif
"#;

const STAGE_UNIFORMS: &str = r#"
// 0 image, 1 clamped image, 2 animMap, 3 the light map, 4 white
uniform int src_kind = 0;
uniform sampler2D tex : filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D tex_c : filter_linear_mipmap_anisotropic, repeat_disable;
uniform sampler2DArray frames : filter_linear_mipmap, repeat_enable;
uniform int nframes = 1;
uniform float anim_freq = 0.0;
uniform sampler2D lm : filter_linear, repeat_disable;
// enhanced: the 4K pack's image of the stage (hdtex.rs)
uniform sampler2D tex_hd : filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D tex_hd_c : filter_linear_mipmap_anisotropic, repeat_disable;
uniform bool has_hd = false;
// the base image's alpha cuts this pass (an overlay that repeated the base
// image over it, folded in: bspview.rs plan_passes)
uniform sampler2D mask_tex : filter_linear_mipmap_anisotropic, repeat_enable;
uniform bool has_mask = false;
// tcGen 0 base, 1 lightmap, 2 environment, 3 vector
uniform int tc_gen = 0;
uniform vec3 tcv_s;
uniform vec3 tcv_t;
uniform ivec4 tck;
uniform mat4 tca;
uniform mat4 tcb;
// rgbGen 0 identityLighting, 1 identity, 2 const, 3 wave, 4 vertex,
// 5 exactVertex, 6 oneMinusVertex, 7 entity, 8 oneMinusEntity
uniform int rgb_kind = 0;
uniform vec3 rgb_const = vec3(1.0);
uniform vec4 rgb_wave;
uniform int rgb_func = 0;
// alphaGen 0 identity, 1 const, 2 wave, 3 vertex, 4 oneMinusVertex, 5 lightingSpecular
uniform int a_kind = 0;
uniform float a_const = 1.0;
uniform vec4 a_wave;
uniform int a_func = 0;
uniform int alpha_func = 0;
uniform int out_kind = 0;
// a light map stage filtered onto the whole surface, folded into this pass
uniform bool lm_mod = false;
uniform vec3 lm_mod_k = vec3(1.0);
varying vec2 q3_st;
varying vec2 q3_lmst;
varying float q3_spec;
"#;

const STAGE_VERTEX: &str = r#"
    vec3 qp = q3_pos(VERTEX);
    vec3 qn = normalize(q3_dir(NORMAL));
    vec3 qc = q3_pos(q3_cam);
    vec2 st = tc_gen == 1 ? UV2 : UV;
    if (tc_gen == 2) {
        vec3 viewer = normalize(qc - qp);
        vec3 r = qn * 2.0 * dot(qn, viewer) - viewer;
        st = vec2(0.5 + r.y * 0.5, 0.5 - r.z * 0.5);
    } else if (tc_gen == 3) {
        st = vec2(dot(qp, tcv_s), dot(qp, tcv_t));
    }
    for (int k = 0; k < 4; k++) {
        st = q3_tcmod(tck[k], tca[k], tcb[k], st, qp, TIME);
    }
    q3_st = st;
    q3_lmst = UV2;
    // RB_CalcSpecularAlpha: the fixed light at (-960, 1980, 96)
    vec3 ld = normalize(vec3(-960.0, 1980.0, 96.0) - qp);
    vec3 refl = qn * 2.0 * dot(qn, ld) - ld;
    float l = max(dot(refl, normalize(qc - qp)), 0.0);
    l *= l;
    l *= l;
    q3_spec = min(l, 1.0);
"#;

const STAGE_FRAGMENT: &str = r#"
    vec4 tx = vec4(1.0);
    bool hd = has_hd && trem_gfx > 0.5;
    if (src_kind == 0) {
        tx = hd ? texture(tex_hd, q3_st) : texture(tex, q3_st);
    } else if (src_kind == 1) {
        tx = hd ? texture(tex_hd_c, q3_st) : texture(tex_c, q3_st);
    } else if (src_kind == 2) {
        float f = mod(floor(TIME * anim_freq), float(max(nframes, 1)));
        tx = texture(frames, vec3(q3_st, f));
    } else if (src_kind == 3) {
        tx = vec4(q3_shift(texture(lm, q3_st).rgb), 1.0);
    }
    vec3 vc = q3_shift(COLOR.rgb);
    vec3 rgb = vec3(0.5);
    if (rgb_kind == 1 || rgb_kind == 7) {
        rgb = vec3(1.0);
    } else if (rgb_kind == 2) {
        rgb = rgb_const;
    } else if (rgb_kind == 3) {
        float g = rgb_func == 5 ? q3_wave(5, rgb_wave, TIME) : q3_wave(rgb_func, rgb_wave, TIME) * 0.5;
        rgb = vec3(clamp(g, 0.0, 1.0));
    } else if (rgb_kind == 4) {
        rgb = vc * 0.5;
    } else if (rgb_kind == 5) {
        rgb = vc;
    } else if (rgb_kind == 6) {
        rgb = (vec3(1.0) - vc) * 0.5;
    } else if (rgb_kind == 8) {
        rgb = vec3(0.0);
    }
    float a = 1.0;
    if (a_kind == 1) {
        a = a_const;
    } else if (a_kind == 2) {
        a = clamp(q3_wave(a_func, a_wave, TIME), 0.0, 1.0);
    } else if (a_kind == 3) {
        a = COLOR.a;
    } else if (a_kind == 4) {
        a = 1.0 - COLOR.a;
    } else if (a_kind == 5) {
        a = q3_spec;
    }
    vec4 c = tx * vec4(rgb, a);
    if (lm_mod) { c.rgb *= q3_shift(texture(lm, q3_lmst).rgb) * lm_mod_k; }
    if ((alpha_func == 1 && c.a <= 0.0) || (alpha_func == 2 && c.a >= 0.5) || (alpha_func == 3 && c.a < 0.5)) { discard; }
    vec4 o = c;
    if (out_kind == 1) { o = vec4(c.rgb * c.a, 1.0); }
    else if (out_kind == 2) { o = vec4(c.rgb, 1.0 - c.a); }
    else if (out_kind == 3) { o = vec4(c.rgb + vec3(c.a), 1.0); }
    else if (out_kind == 4) { o = vec4(c.rgb + vec3(1.0), 1.0); }
    else if (out_kind == 5) { o = vec4(vec3(1.0) - c.rgb, 1.0); }
    else if (out_kind == 6) { o = vec4(vec3(c.a), 1.0); }
    else if (out_kind == 7) { o = vec4(vec3(1.0 - c.a), 1.0); }
    else if (out_kind == 8) { o = vec4(c.rgb * 2.0, 1.0); }
    float q3_m = has_mask ? 1.0 - texture(mask_tex, UV).a : 1.0;
"#;

fn wave_v4(w: &Wave) -> godot::prelude::Vector4 {
    godot::prelude::Vector4::new(w.base, w.amp, w.phase, w.freq)
}

fn mat_of(cols: [[f32; 4]; 4]) -> godot::builtin::Projection {
    use godot::prelude::Vector4;
    let c = |i: usize| Vector4::new(cols[i][0], cols[i][1], cols[i][2], cols[i][3]);
    godot::builtin::Projection::from_cols(c(0), c(1), c(2), c(3))
}

/// The deform uniforms of a shader (q3_dk / q3_da / q3_db) and whether a
/// mesh needs the autosprite data (CUSTOM1 / CUSTOM2).
pub fn set_deforms(mat: &mut godot::prelude::Gd<godot::classes::ShaderMaterial>, deforms: &[Deform]) {
    use godot::prelude::*;
    let mut k = [0i32; 4];
    let mut a = [[0f32; 4]; 4];
    let mut b = [[0f32; 4]; 4];
    for (i, d) in deforms.iter().take(4).enumerate() {
        match *d {
            Deform::Wave(spread, w) => {
                k[i] = 1;
                a[i] = [w.base, w.amp, w.phase, w.freq];
                b[i] = [spread, w.func as f32, 0.0, 0.0];
            }
            Deform::Bulge(width, height, speed) => {
                k[i] = 2;
                a[i] = [width, height, speed, 0.0];
            }
            Deform::Move(v, w) => {
                k[i] = 3;
                a[i] = [w.base, w.amp, w.phase, w.freq];
                b[i] = [v[0], v[1], v[2], w.func as f32];
            }
            Deform::AutoSprite => k[i] = 4,
            Deform::AutoSprite2 => k[i] = 5,
            Deform::Normal => {}
        }
    }
    mat.set_shader_parameter("q3_dk", &Vector4i::new(k[0], k[1], k[2], k[3]).to_variant());
    mat.set_shader_parameter("q3_da", &mat_of(a).to_variant());
    mat.set_shader_parameter("q3_db", &mat_of(b).to_variant());
}

/// The uniforms of one stage (everything but its textures).
pub fn set_stage(mat: &mut godot::prelude::Gd<godot::classes::ShaderMaterial>, st: &Stage, out_kind: i32) {
    use godot::prelude::*;
    let (tg, vs, vt) = match st.tc_gen {
        TcGen::Base => (0, [0.0; 3], [0.0; 3]),
        TcGen::Lightmap => (1, [0.0; 3], [0.0; 3]),
        TcGen::Environment => (2, [0.0; 3], [0.0; 3]),
        TcGen::Vector(s, t) => (3, s, t),
    };
    mat.set_shader_parameter("tc_gen", &tg.to_variant());
    mat.set_shader_parameter("tcv_s", &Vector3::new(vs[0], vs[1], vs[2]).to_variant());
    mat.set_shader_parameter("tcv_t", &Vector3::new(vt[0], vt[1], vt[2]).to_variant());
    let mut k = [0i32; 4];
    let mut a = [[0f32; 4]; 4];
    let mut b = [[0f32; 4]; 4];
    for (i, m) in st.tc_mods.iter().take(4).enumerate() {
        match *m {
            TcMod::Scale(s, t) => {
                k[i] = 1;
                a[i] = [s, t, 0.0, 0.0];
            }
            TcMod::Scroll(s, t) => {
                k[i] = 2;
                a[i] = [s, t, 0.0, 0.0];
            }
            TcMod::Rotate(d) => {
                k[i] = 3;
                a[i] = [d, 0.0, 0.0, 0.0];
            }
            TcMod::Turb(w) => {
                k[i] = 4;
                a[i] = [w.base, w.amp, w.phase, w.freq];
            }
            TcMod::Stretch(w) => {
                k[i] = 5;
                a[i] = [w.base, w.amp, w.phase, w.freq];
                b[i] = [w.func as f32, 0.0, 0.0, 0.0];
            }
            TcMod::Transform(m) => {
                k[i] = 6;
                a[i] = [m[0], m[1], m[2], m[3]];
                b[i] = [m[4], m[5], 0.0, 0.0];
            }
        }
    }
    mat.set_shader_parameter("tck", &Vector4i::new(k[0], k[1], k[2], k[3]).to_variant());
    mat.set_shader_parameter("tca", &mat_of(a).to_variant());
    mat.set_shader_parameter("tcb", &mat_of(b).to_variant());
    let (rk, rc, rw) = match st.rgb_gen {
        RgbGen::IdentityLighting | RgbGen::LightingDiffuse => (0, [1.0; 3], Wave::default()),
        RgbGen::Identity => (1, [1.0; 3], Wave::default()),
        RgbGen::Const(c) => (2, c, Wave::default()),
        RgbGen::Wave(w) => (3, [1.0; 3], w),
        RgbGen::Vertex => (4, [1.0; 3], Wave::default()),
        RgbGen::ExactVertex => (5, [1.0; 3], Wave::default()),
        RgbGen::OneMinusVertex => (6, [1.0; 3], Wave::default()),
        RgbGen::Entity => (7, [1.0; 3], Wave::default()),
        RgbGen::OneMinusEntity => (8, [1.0; 3], Wave::default()),
    };
    let rf = rw.func;
    mat.set_shader_parameter("rgb_kind", &rk.to_variant());
    mat.set_shader_parameter("rgb_const", &Vector3::new(rc[0], rc[1], rc[2]).to_variant());
    mat.set_shader_parameter("rgb_wave", &wave_v4(&rw).to_variant());
    mat.set_shader_parameter("rgb_func", &rf.to_variant());
    let (ak, ac, aw) = match st.alpha_gen {
        AlphaGen::Identity | AlphaGen::Entity | AlphaGen::Portal(_) => (0, 1.0, Wave::default()),
        AlphaGen::Const(c) => (1, c, Wave::default()),
        AlphaGen::Wave(w) => (2, 1.0, w),
        AlphaGen::Vertex => (3, 1.0, Wave::default()),
        AlphaGen::OneMinusVertex => (4, 1.0, Wave::default()),
        AlphaGen::LightingSpecular => (5, 1.0, Wave::default()),
    };
    mat.set_shader_parameter("a_kind", &ak.to_variant());
    mat.set_shader_parameter("a_const", &ac.to_variant());
    mat.set_shader_parameter("a_wave", &wave_v4(&aw).to_variant());
    mat.set_shader_parameter("a_func", &aw.func.to_variant());
    mat.set_shader_parameter("alpha_func", &(st.alpha_func as i32).to_variant());
    mat.set_shader_parameter("out_kind", &out_kind.to_variant());
}

/// rgbGen of a stage folded into the base pass as a constant factor (fb
/// units: identityLighting is tr.identityLight 0.5), None if it varies.
pub fn const_rgb(g: &RgbGen) -> Option<[f32; 3]> {
    match *g {
        RgbGen::Identity | RgbGen::Entity => Some([1.0; 3]),
        RgbGen::IdentityLighting | RgbGen::LightingDiffuse => Some([0.5; 3]),
        RgbGen::Const(c) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_light_fixture() {
        let src = "textures/atcs/cubelight_32_blue_s_10k\n{\n\tsurfaceparm nomarks\n\tqer_editorimage textures/atcs/cubelight_32_blue.tga\n\t{\n\t\tmap $lightmap\n\t\trgbGen identity\n\t}\n\t{\n\t\tmap textures/atcs/cubelight_32_blue.tga\n\t\tblendFunc GL_DST_COLOR GL_ZERO\n\t\trgbGen identity\n\t}\n\t{\n\t\tmap textures/atcs/cubelight_32_blue.blend.tga\n\t\tblendfunc GL_ONE GL_ONE\n\t}\n}\n";
        let mut m = HashMap::new();
        parse_script(src, &mut m);
        let s = &m["textures/atcs/cubelight_32_blue_s_10k"];
        assert_eq!(s.stages.len(), 3);
        assert!(s.stages[0].is_lightmap());
        assert_eq!(s.stages[0].tc_gen, TcGen::Lightmap);
        assert!(s.stages[1].is_filter());
        assert_eq!(s.stages[2].blend, Some(("gl_one".into(), "gl_one".into())));
        // an add stage without rgbGen: identityLighting
        assert_eq!(s.stages[2].rgb_gen, RgbGen::IdentityLighting);
        assert!(!s.stages[2].depth_write);
    }

    #[test]
    fn parses_waves_deforms_and_sky() {
        let src = "textures/n/slime\n{\n deformVertexes wave 100 sin 0 1 .5 .5\n {\n  map textures/n/s.tga\n  rgbGen const ( 0.5 0.5 0.5 )\n  blendfunc add\n  tcMod turb .3 .2 1 .05\n  tcMod scroll .01 .01\n }\n}\ntextures/u/sky /* c */\n{\n skyparms - 512 -\n {\n  map textures/u/a.jpg\n  tcMod scroll 0.05 .1\n }\n {\n  animMap 5 a.tga b.tga c.tga\n  rgbGen wave sawtooth 0 1 0 0.5\n  alphaGen lightingSpecular\n  tcGen environment\n }\n}\n";
        let mut m = HashMap::new();
        parse_script(src, &mut m);
        let s = &m["textures/n/slime"];
        assert_eq!(s.deforms, vec![Deform::Wave(0.01, Wave { func: GF_SIN, base: 0.0, amp: 1.0, phase: 0.5, freq: 0.5 })]);
        assert_eq!(s.stages[0].rgb_gen, RgbGen::Const([0.5, 0.5, 0.5]));
        assert_eq!(s.stages[0].tc_mods.len(), 2);
        assert!(matches!(s.stages[0].tc_mods[0], TcMod::Turb(w) if (w.amp - 0.2).abs() < 1e-6 && (w.freq - 0.05).abs() < 1e-6));
        let k = &m["textures/u/sky"];
        assert_eq!(k.sky, Some(SkyParms { outer: None, cloud_height: 512.0 }));
        assert_eq!(k.stages[1].source, Source::Image(vec!["a.tga".into(), "b.tga".into(), "c.tga".into()], 5.0));
        assert_eq!(k.stages[1].rgb_gen, RgbGen::Wave(Wave { func: GF_SAWTOOTH, base: 0.0, amp: 1.0, phase: 0.0, freq: 0.5 }));
        assert_eq!(k.stages[1].alpha_gen, AlphaGen::LightingSpecular);
        assert_eq!(k.stages[1].tc_gen, TcGen::Environment);
    }

    #[test]
    fn blend_variants() {
        let b = |s: &str, d: &str| blend_variant(Some(&(s.to_string(), d.to_string())));
        assert_eq!(b("gl_dst_color", "gl_zero"), (Variant::Mul, 0));
        assert_eq!(b("gl_dst_color", "gl_src_alpha"), (Variant::Mul, 3));
        assert_eq!(b("gl_one_minus_src_alpha", "gl_src_alpha"), (Variant::Mix, 2));
        assert_eq!(blend_variant(None), (Variant::Opaque, 0));
        assert_eq!(normalize_blend(&["GL_ONE".into(), "GL_ZERO".into()]), None);
    }
}
