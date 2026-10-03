//! Player-owned game data (dec003): the pk3 search path (FS_* semantics via
//! Godot's ZIPReader), shader-script image resolution, and texture/sound
//! decoding shared by the BSP, model and audio front ends.

use godot::classes::image::Format as ImageFormat;
use godot::classes::{Image, ImageTexture, ZipReader};
use godot::prelude::*;
use std::collections::HashMap;

/// A search path of pk3 archives; later archives override earlier ones (FS_*).
pub struct Pk3Set {
    readers: Vec<Gd<ZipReader>>,
    index: HashMap<String, (usize, String)>,
}

impl Pk3Set {
    pub fn open(paths: &[String]) -> Self {
        let mut sorted: Vec<String> = paths.to_vec();
        sorted.sort_by_key(|p| p.to_lowercase());
        let mut s = Pk3Set { readers: Vec::new(), index: HashMap::new() };
        for p in sorted {
            let mut z = ZipReader::new_gd();
            if z.open(&p) != godot::global::Error::OK {
                godot_warn!("pk3: cannot open {p}");
                continue;
            }
            let ri = s.readers.len();
            for f in z.get_files().as_slice() {
                let f = f.to_string();
                s.index.insert(f.to_lowercase(), (ri, f));
            }
            s.readers.push(z);
        }
        s
    }

    pub fn read(&mut self, name: &str) -> Option<PackedByteArray> {
        let (ri, real) = self.index.get(&name.to_lowercase())?.clone();
        let data = self.readers[ri].read_file(&real);
        (!data.is_empty()).then_some(data)
    }

    pub fn has_file(&self, name: &str) -> bool {
        self.index.contains_key(&name.to_lowercase())
    }

    pub fn files(&self) -> impl Iterator<Item = &String> {
        self.index.values().map(|(_, f)| f)
    }

    pub fn maps(&self) -> Vec<String> {
        let mut m: Vec<String> = self.files().filter(|f| f.to_lowercase().starts_with("maps/") && f.to_lowercase().ends_with(".bsp")).map(|f| f[5..f.len() - 4].to_string()).collect();
        m.sort();
        m.dedup();
        m
    }

    /// shader name -> first image stage of its script (scripts/*.shader).
    pub fn shader_images(&mut self) -> HashMap<String, ShaderInfo> {
        let scripts: Vec<String> = self.files().filter(|f| f.to_lowercase().starts_with("scripts/") && f.to_lowercase().ends_with(".shader")).cloned().collect();
        let mut out = HashMap::new();
        for sc in scripts {
            let Some(data) = self.read(&sc) else { continue };
            let text = String::from_utf8_lossy(data.as_slice()).into_owned();
            parse_shader_script(&text, &mut out);
        }
        out
    }
}

/// Minimal shader-script reader: for each top-level shader, the first
/// `map`/`clampmap`/`animmap` image that is not $lightmap/$whiteimage, else qer_editorimage.
/// How a shader's visible stage composites (Q3 blendFunc of that stage).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    Opaque,
    /// GL_ONE GL_ONE ("add"): glows, beams, flames.
    Add,
    /// GL_SRC_ALPHA GL_ONE_MINUS_SRC_ALPHA ("blend").
    Mix,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderInfo {
    pub image: Option<String>,
    pub blend: Blend,
    pub alpha_test: bool,
    pub two_sided: bool,
    pub sky: Option<String>,
    pub nodraw: bool,
    /// every image stage in order (model shaders draw the first two: an
    /// opaque base and a blended layer, e.g. the advanced marauder's skin)
    pub stages: Vec<StageInfo>,
}

/// `tcMod` of a stage: 1 scale s t, 2 scroll s t (per second), 3 rotate
/// degrees per second (others are not drawn).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TcMod {
    pub kind: i32,
    pub a: [f32; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageInfo {
    pub image: String,
    pub blend: Blend,
    /// `rgbGen identity` / `identityLighting`: drawn unlit
    pub unlit: bool,
    pub tcmods: Vec<TcMod>,
}

fn blend_of(args: &[String]) -> Option<Blend> {
    let a: Vec<String> = args.iter().map(|x| x.to_lowercase()).collect();
    match a.as_slice() {
        [x] if x == "add" => Some(Blend::Add),
        [x] if x == "blend" => Some(Blend::Mix),
        [x] if x == "filter" => None,
        [s, d] if s == "gl_one" && d == "gl_one" => Some(Blend::Add),
        [s, d] if s == "gl_src_alpha" && d == "gl_one" => Some(Blend::Add),
        // "screen" style (flame.shader's flames, sparks): adds light like GL_ONE GL_ONE
        [s, d] if s == "gl_one" && d == "gl_one_minus_src_color" => Some(Blend::Add),
        [s, d] if s == "gl_one_minus_dst_color" && d == "gl_one" => Some(Blend::Add),
        // premultiplied alpha
        [s, d] if s == "gl_one" && d == "gl_one_minus_src_alpha" => Some(Blend::Mix),
        [s, d] if s == "gl_src_alpha" && d == "gl_one_minus_src_alpha" => Some(Blend::Mix),
        _ => None,
    }
}

/// Minimal shader-script reader: per shader, the first image stage (not
/// $lightmap/$whiteimage) with its blendFunc/alphaFunc, else qer_editorimage;
/// plus cull/skyparms/surfaceparm nodraw.
pub fn parse_shader_script(text: &str, out: &mut HashMap<String, ShaderInfo>) {
    let mut depth = 0;
    let mut name: Option<String> = None;
    let mut info = ShaderInfo::default();
    let mut editor: Option<String> = None;
    let mut stage_img: Option<String> = None;
    let mut stage_blend: Option<Blend> = None;
    let mut stage_alpha = false;
    let mut stage_unlit = false;
    let mut stage_tc: Vec<TcMod> = Vec::new();
    let mut stage_index = 0;
    let mut lightmap_first = false;
    let mut stage_is_lightmap = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let toks: Vec<String> = line.split_whitespace().map(|x| x.to_string()).collect();
        let mut i = 0;
        while i < toks.len() {
            let t = toks[i].as_str();
            let lt = t.to_lowercase();
            match t {
                "{" => {
                    depth += 1;
                    if depth == 2 {
                        stage_img = None;
                        stage_blend = None;
                        stage_alpha = false;
                        stage_is_lightmap = false;
                        stage_unlit = false;
                        stage_tc.clear();
                    }
                }
                "}" => {
                    if depth == 2 {
                        if stage_is_lightmap && stage_index == 0 {
                            lightmap_first = true;
                        }
                        if let Some(img) = stage_img.as_ref() {
                            info.stages.push(StageInfo { image: img.clone(), blend: stage_blend.unwrap_or(Blend::Opaque), unlit: stage_unlit, tcmods: std::mem::take(&mut stage_tc) });
                        }
                        if info.image.is_none() && stage_img.is_some() {
                            info.image = stage_img.take();
                            // a texture stage filtered onto a lightmap is an ordinary opaque surface
                            info.blend = if lightmap_first { Blend::Opaque } else { stage_blend.unwrap_or(Blend::Opaque) };
                            info.alpha_test = stage_alpha;
                        }
                        stage_index += 1;
                    }
                    depth -= 1;
                    if depth == 0 {
                        if let Some(n) = name.take() {
                            if info.image.is_none() {
                                info.image = editor.take();
                            }
                            out.entry(n.to_lowercase().replace('\\', "/")).or_insert(std::mem::take(&mut info));
                        }
                        info = ShaderInfo::default();
                        editor = None;
                        stage_index = 0;
                        lightmap_first = false;
                    }
                }
                _ if depth == 0 => name = Some(t.to_string()),
                _ if depth == 1 => match lt.as_str() {
                    "qer_editorimage" => {
                        editor = toks.get(i + 1).cloned();
                        i += 1;
                    }
                    "cull" => {
                        let m = toks.get(i + 1).map(|x| x.to_lowercase()).unwrap_or_default();
                        info.two_sided = m == "disable" || m == "none" || m == "twosided";
                        i += 1;
                    }
                    "skyparms" => {
                        info.sky = toks.get(i + 1).cloned().filter(|x| x != "-");
                        i += 3;
                    }
                    "surfaceparm" => {
                        if toks.get(i + 1).map(|x| x.eq_ignore_ascii_case("nodraw")).unwrap_or(false) {
                            info.nodraw = true;
                        }
                        i += 1;
                    }
                    _ => {}
                },
                _ if depth == 2 => match lt.as_str() {
                    "map" | "clampmap" | "animmap" => {
                        let mut k = i + 1;
                        if lt == "animmap" {
                            k += 1; // frequency
                        }
                        if let Some(a) = toks.get(k) {
                            if a.eq_ignore_ascii_case("$lightmap") {
                                stage_is_lightmap = true;
                            } else if !a.starts_with('$') && stage_img.is_none() {
                                stage_img = Some(a.to_string());
                            }
                        }
                        i = toks.len();
                        continue;
                    }
                    "blendfunc" => {
                        stage_blend = blend_of(&toks[i + 1..]);
                        i = toks.len();
                        continue;
                    }
                    "alphafunc" => stage_alpha = true,
                    "rgbgen" => {
                        let m = toks.get(i + 1).map(|x| x.to_lowercase()).unwrap_or_default();
                        stage_unlit = m == "identity" || m == "identitylighting";
                        i = toks.len();
                        continue;
                    }
                    "tcmod" => {
                        let m = toks.get(i + 1).map(|x| x.to_lowercase()).unwrap_or_default();
                        let f = |j: usize| toks.get(i + j).and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
                        let tc = match m.as_str() {
                            "scale" => Some(TcMod { kind: 1, a: [f(2), f(3)] }),
                            "scroll" => Some(TcMod { kind: 2, a: [f(2), f(3)] }),
                            "rotate" => Some(TcMod { kind: 3, a: [f(2), 0.0] }),
                            _ => None,
                        };
                        if let Some(tc) = tc {
                            stage_tc.push(tc);
                        }
                        i = toks.len();
                        continue;
                    }
                    _ => {}
                },
                _ => {}
            }
            i += 1;
        }
    }
}

pub fn load_image(pk3: &mut Pk3Set, base: &str) -> Option<Gd<Image>> {
    let stem = base.trim_end_matches(".tga").trim_end_matches(".jpg").trim_end_matches(".png");
    for (ext, kind) in [(".tga", 0), (".jpg", 1), (".png", 2)] {
        if let Some(data) = pk3.read(&format!("{stem}{ext}")) {
            let mut img = Image::new_gd();
            let err = match kind {
                0 => img.load_tga_from_buffer(&data),
                1 => img.load_jpg_from_buffer(&data),
                _ => img.load_png_from_buffer(&data),
            };
            if err == godot::global::Error::OK {
                img.generate_mipmaps();
                return Some(img);
            }
        }
    }
    None
}

/// tr_image.c DEFAULT_SIZE.
pub const DEFAULT_SIZE: usize = 16;

/// `R_CreateDefaultImage`: the "*default" image of the renderer's default
/// shader, drawn for any shader / image that is not in the data: a 16x16
/// box, white one-texel border, interior 32 grey. RGBA bytes; the default
/// shader draws it opaque (GLS_DEFAULT, no blending), so alpha is 255 here.
pub fn default_image_rgba() -> Vec<u8> {
    let mut data = vec![32u8; DEFAULT_SIZE * DEFAULT_SIZE * 4];
    let mut set = |y: usize, x: usize| data[(y * DEFAULT_SIZE + x) * 4..][..4].copy_from_slice(&[255; 4]);
    for x in 0..DEFAULT_SIZE {
        set(0, x);
        set(x, 0);
        set(DEFAULT_SIZE - 1, x);
        set(x, DEFAULT_SIZE - 1);
    }
    for px in data.chunks_mut(4) {
        px[3] = 255;
    }
    data
}

/// The default image as a mipmapped texture (IMGFLAG_MIPMAP).
pub fn default_texture() -> Gd<ImageTexture> {
    let mut img = Image::create_from_data(DEFAULT_SIZE as i32, DEFAULT_SIZE as i32, false, ImageFormat::RGBA8, &PackedByteArray::from(default_image_rgba().as_slice())).unwrap();
    img.generate_mipmaps();
    ImageTexture::create_from_image(&img).unwrap()
}

pub fn flat_texture(c: Color) -> Gd<ImageTexture> {
    let mut img = Image::create(4, 4, false, ImageFormat::RGBA8).unwrap();
    img.fill(c);
    ImageTexture::create_from_image(&img).unwrap()
}

/// 128x128 RGB lightmaps, colour-shifted like R_ColorShiftLightingBytes is
/// done here by the shader's 2x overbright instead.
#[cfg(test)]
mod tests {
    use super::*;

    /// flame.shader's GL_ONE GL_ONE_MINUS_SRC_COLOR flames were drawn opaque
    /// (black squares where bullets hit human structures)
    #[test]
    fn screen_blend_is_additive() {
        let v = |a: &str, b: &str| blend_of(&[a.to_string(), b.to_string()]);
        assert_eq!(v("GL_ONE", "GL_ONE_MINUS_SRC_COLOR"), Some(Blend::Add));
        assert_eq!(v("GL_ONE_MINUS_DST_COLOR", "GL_ONE"), Some(Blend::Add));
        assert_eq!(v("GL_ONE", "GL_ONE_MINUS_SRC_ALPHA"), Some(Blend::Mix));
    }
    #[test]
    fn default_image_is_a_box() {
        let d = default_image_rgba();
        let px = |x: usize, y: usize| &d[(y * DEFAULT_SIZE + x) * 4..][..4];
        assert_eq!(px(0, 7), &[255, 255, 255, 255]);
        assert_eq!(px(15, 15), &[255, 255, 255, 255]);
        assert_eq!(px(7, 15), &[255, 255, 255, 255]);
        assert_eq!(px(1, 1), &[32, 32, 32, 255]);
        assert_eq!(px(8, 9), &[32, 32, 32, 255]);
    }

    #[test]
    fn shader_script_first_image_stage() {
        let src = r#"
textures/base/wall
{
    qer_editorimage textures/base/wall_ed.tga
    {
        map $lightmap
    }
    {
        map textures/base/wall_d.tga // comment
        blendFunc filter
    }
}
textures/sky/space
{
    qer_editorimage textures/sky/ed.jpg
    surfaceparm sky
}
"#;
        let mut m = HashMap::new();
        parse_shader_script(src, &mut m);
        assert_eq!(m["textures/base/wall"].image.as_deref(), Some("textures/base/wall_d.tga"));
        assert_eq!(m["textures/base/wall"].blend, Blend::Opaque, "filtered onto the lightmap");
        assert_eq!(m["textures/sky/space"].image.as_deref(), Some("textures/sky/ed.jpg"));
        let mut m2 = HashMap::new();
        parse_shader_script("models/x/beam\n{\n cull disable\n {\n  map models/x/beam.jpg\n  blendfunc add\n }\n}\n", &mut m2);
        assert_eq!(m2["models/x/beam"].blend, Blend::Add);
        assert!(m2["models/x/beam"].two_sided);
    }
}
