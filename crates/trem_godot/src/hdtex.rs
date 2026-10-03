//! 4K texture packs for the enhanced graphics mode (port addition).
//!
//! `TremHdBuilder` makes the pack offline from the player's own pk3 files:
//! every world / model / sky image is padded with its own wrapped edges (so
//! tiling textures stay seamless), upscaled with Real-ESRGAN
//! (realesrgan-ncnn-vulkan, on the GPU) in one or two x4 passes, cropped,
//! resized to `target` (4096) on its longest side with Lanczos, given its
//! original alpha back, mipmapped, block-compressed (S3TC: DXT1, or DXT5
//! with alpha) and written as `<key>.hdtex`, with a generated normal map
//! (`<key>.hdnrm`, 1024) beside it.
//!
//! At run time `with_store` serves them by image key (the pk3 path without
//! its extension). The pack lives outside the game's search path (default
//! `user://hd`), so pure servers' pk3 checks never see it: it works on
//! vanilla 1.1.0 and 1.3 servers alike. The enhanced mode samples the HD
//! texture, vanilla the original (F10 switches both).
//! `r_hdTextures 0` turns the pack off; `r_hdMaxSize` (4096 / 2048 / 1024)
//! skips the larger mip levels to save VRAM.

use godot::classes::image::{CompressMode, Format as ImageFormat, Interpolation};
use godot::classes::{Image, ImageTexture};
use godot::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"TREMHD01";
/// magic, w, h, format, mip count, mean rgb, flags
const HEADER: usize = 8 + 4 * 5 + 4 * 3;

/// The image key of a pk3 path: lower case, '/' separators, no extension.
pub fn key_of(path: &str) -> String {
    let p = path.replace('\\', "/").to_lowercase();
    match p.rsplit_once('.') {
        Some((stem, ext)) if matches!(ext, "tga" | "jpg" | "jpeg" | "png") => stem.to_string(),
        _ => p,
    }
}

/// Images worth upscaling: world textures, models and skies (not the HUD,
/// menus, fonts, icons or level shots).
pub fn wanted(path: &str) -> bool {
    let p = path.to_lowercase();
    let img = p.ends_with(".tga") || p.ends_with(".jpg") || p.ends_with(".png");
    img && (p.starts_with("textures/") || p.starts_with("models/") || p.starts_with("env/"))
}

fn block_bytes(fmt: ImageFormat) -> Option<usize> {
    match fmt {
        ImageFormat::DXT1 | ImageFormat::RGTC_R => Some(8),
        ImageFormat::DXT3 | ImageFormat::DXT5 | ImageFormat::BPTC_RGBA | ImageFormat::RGTC_RG => Some(16),
        _ => None,
    }
}

fn level_bytes(fmt: ImageFormat, w: usize, h: usize) -> usize {
    match block_bytes(fmt) {
        Some(b) => w.div_ceil(4).max(1) * h.div_ceil(4).max(1) * b,
        None => w * h * 4,
    }
}

/// Write an Image (with its mip chain) as a .hdtex file.
pub fn write(path: &Path, img: &Gd<Image>, mean: [f32; 3], alpha: bool) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let data = img.get_data();
    let mut f = std::fs::File::create(path)?;
    f.write_all(MAGIC)?;
    for v in [img.get_width() as u32, img.get_height() as u32, img.get_format().ord() as u32, img.get_mipmap_count() as u32 + 1, alpha as u32] {
        f.write_all(&v.to_le_bytes())?;
    }
    for m in mean {
        f.write_all(&m.to_le_bytes())?;
    }
    f.write_all(data.as_slice())
}

/// Read a .hdtex, skipping the mip levels larger than `max_size`.
pub fn read(path: &Path, max_size: usize) -> Option<(Gd<Image>, [f32; 3])> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; HEADER];
    f.read_exact(&mut h).ok()?;
    if &h[..8] != MAGIC {
        return None;
    }
    let u = |i: usize| u32::from_le_bytes(h[8 + 4 * i..12 + 4 * i].try_into().unwrap()) as usize;
    let (mut w, mut hh, fmt, mips) = (u(0), u(1), u(2), u(3));
    let mean = [0, 1, 2].map(|k| f32::from_le_bytes(h[28 + 4 * k..32 + 4 * k].try_into().unwrap()));
    let fmt = ImageFormat::try_from_ord(fmt as i32)?;
    let mut skip = 0usize;
    let mut levels = mips.max(1);
    while (w > max_size || hh > max_size) && levels > 1 {
        skip += level_bytes(fmt, w, hh);
        w = (w / 2).max(1);
        hh = (hh / 2).max(1);
        levels -= 1;
    }
    let mut total = 0usize;
    let (mut lw, mut lh) = (w, hh);
    for _ in 0..levels {
        total += level_bytes(fmt, lw, lh);
        lw = (lw / 2).max(1);
        lh = (lh / 2).max(1);
    }
    f.seek(SeekFrom::Current(skip as i64)).ok()?;
    let mut data = vec![0u8; total];
    f.read_exact(&mut data).ok()?;
    let img = Image::create_from_data(w as i32, hh as i32, levels > 1, fmt, &PackedByteArray::from(data.as_slice()))?;
    Some((img, mean))
}

/// An HD texture: albedo, its normal map, mean linear albedo.
#[derive(Clone)]
pub struct HdTex {
    pub albedo: Gd<ImageTexture>,
    pub normal: Option<Gd<ImageTexture>>,
    pub mean: [f32; 3],
}

pub struct HdStore {
    root: PathBuf,
    max_size: usize,
    cache: HashMap<String, Option<HdTex>>,
}

impl HdStore {
    pub fn get(&mut self, key: &str) -> Option<HdTex> {
        if let Some(t) = self.cache.get(key) {
            return t.clone();
        }
        let t = read(&self.root.join(format!("{key}.hdtex")), self.max_size).and_then(|(img, mean)| {
            let albedo = ImageTexture::create_from_image(&img)?;
            let normal = read(&self.root.join(format!("{key}.hdnrm")), self.max_size).and_then(|(n, _)| ImageTexture::create_from_image(&n));
            Some(HdTex { albedo, normal, mean })
        });
        self.cache.insert(key.to_string(), t.clone());
        t
    }
}

thread_local! {
    static STORE: RefCell<Option<HdStore>> = const { RefCell::new(None) };
}

/// (Re)open the pack: `root` "" = user://hd; `enabled` false drops it.
pub fn configure(enabled: bool, root: &str, max_size: usize) {
    let root = if root.is_empty() { godot::classes::ProjectSettings::singleton().globalize_path("user://hd").to_string() } else { root.to_string() };
    let root = PathBuf::from(root);
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let same = s.as_ref().is_some_and(|h| h.root == root && h.max_size == max_size);
        if !enabled || !root.is_dir() {
            *s = None;
        } else if !same {
            *s = Some(HdStore { root, max_size: max_size.clamp(64, 8192), cache: HashMap::new() });
        }
    });
}

/// The HD texture of an image key, when the pack has it.
pub fn get(key: &str) -> Option<HdTex> {
    STORE.with(|s| s.borrow_mut().as_mut().and_then(|h| h.get(key)))
}

pub fn active() -> bool {
    STORE.with(|s| s.borrow().is_some())
}

// --- the offline builder ---------------------------------------------------

#[derive(Clone)]
struct Item {
    key: String,
    src: String,
    w: usize,
    h: usize,
    pad: usize,
    /// Real-ESRGAN x4 passes (1 or 2)
    passes: u32,
}

/// Builds the pack (tests/hd_build.gd drives it: prepare, the Real-ESRGAN
/// runs, then finish(i) for every item).
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct TremHdBuilder {
    base: Base<RefCounted>,
    pk3: Option<crate::assets::Pk3Set>,
    items: Vec<Item>,
    out: PathBuf,
    work: PathBuf,
    target: usize,
}

fn wrap_pad(rgba: &[u8], w: usize, h: usize, p: usize) -> Vec<u8> {
    let (pw, ph) = (w + 2 * p, h + 2 * p);
    let mut out = vec![0u8; pw * ph * 3];
    for y in 0..ph {
        let sy = (y as isize - p as isize).rem_euclid(h as isize) as usize;
        for x in 0..pw {
            let sx = (x as isize - p as isize).rem_euclid(w as isize) as usize;
            let s = (sy * w + sx) * 4;
            let d = (y * pw + x) * 3;
            out[d..d + 3].copy_from_slice(&rgba[s..s + 3]);
        }
    }
    out
}

#[godot_api]
impl TremHdBuilder {
    /// Decode every wanted image of the pk3s not yet in `out_dir`, pad it and
    /// write the Real-ESRGAN inputs to `work_dir`/{a_in, b_in} (a: two
    /// passes, b: one). Returns the number of items.
    #[func]
    fn prepare(&mut self, pk3_paths: PackedStringArray, out_dir: GString, work_dir: GString, target: i32) -> i32 {
        let paths: Vec<String> = pk3_paths.as_slice().iter().map(|s| s.to_string()).collect();
        let mut pk3 = crate::assets::Pk3Set::open(&paths);
        self.out = PathBuf::from(out_dir.to_string());
        self.work = PathBuf::from(work_dir.to_string());
        self.target = target.max(256) as usize;
        for d in ["a_in", "a_mid", "a_out", "b_in", "b_out"] {
            let _ = std::fs::create_dir_all(self.work.join(d));
        }
        let mut files: Vec<String> = pk3.files().filter(|f| wanted(f)).cloned().collect();
        files.sort();
        let mut seen = std::collections::HashSet::new();
        self.items.clear();
        for f in files {
            let key = key_of(&f);
            if !seen.insert(key.clone()) || self.out.join(format!("{key}.hdtex")).exists() {
                continue;
            }
            let Some(mut img) = crate::assets::load_image(&mut pk3, &f) else { continue };
            img.clear_mipmaps();
            img.convert(ImageFormat::RGBA8);
            let (w, h) = (img.get_width() as usize, img.get_height() as usize);
            if w.max(h) < 48 {
                continue;
            }
            let m = w.max(h);
            let passes = if m * 8 <= self.target { 2 } else { 1 };
            let pad = (m / 8).max(4);
            let data = img.get_data();
            let padded = wrap_pad(data.as_slice(), w, h, pad);
            let Some(pimg) = Image::create_from_data((w + 2 * pad) as i32, (h + 2 * pad) as i32, false, ImageFormat::RGB8, &PackedByteArray::from(padded.as_slice())) else { continue };
            let idx = self.items.len();
            let dir = if passes == 2 { "a_in" } else { "b_in" };
            let _ = pimg.save_png(&self.work.join(dir).join(format!("{idx:05}.png")).to_string_lossy().to_string());
            self.items.push(Item { key, src: f, w, h, pad, passes });
        }
        self.pk3 = Some(pk3);
        godot_print!("hd: {} images to upscale ({} with two passes)", self.items.len(), self.items.iter().filter(|i| i.passes == 2).count());
        self.items.len() as i32
    }

    /// Finish item `i`: crop, resize, alpha, mipmaps, compression, write.
    /// Returns a progress line ("" when skipped).
    #[func]
    fn finish(&mut self, i: i32) -> GString {
        let Some(it) = self.items.get(i as usize).cloned() else { return GString::new() };
        let dir = if it.passes == 2 { "a_out" } else { "b_out" };
        let png = self.work.join(dir).join(format!("{i:05}.png"));
        let Some(mut up) = Image::load_from_file(&png.to_string_lossy().to_string()) else { return GString::from(format!("hd: {} missing upscaled image", it.key).as_str()) };
        let scale = 4usize.pow(it.passes);
        up.convert(ImageFormat::RGBA8);
        let crop = up.get_region(Rect2i::new(Vector2i::new((it.pad * scale) as i32, (it.pad * scale) as i32), Vector2i::new((it.w * scale) as i32, (it.h * scale) as i32)));
        let Some(mut img) = crop else { return GString::new() };
        let m = it.w.max(it.h);
        let (fw, fh) = ((it.w * self.target / m).max(4), (it.h * self.target / m).max(4));
        img.resize_ex(fw as i32, fh as i32).interpolation(Interpolation::LANCZOS).done();
        // the original alpha, smoothly scaled
        let mut alpha = false;
        if let Some(pk3) = self.pk3.as_mut() {
            if let Some(mut src) = crate::assets::load_image(pk3, &it.src) {
                src.clear_mipmaps();
                src.convert(ImageFormat::RGBA8);
                let sd = src.get_data();
                if sd.as_slice().chunks_exact(4).any(|p| p[3] < 250) {
                    alpha = true;
                    src.resize_ex(fw as i32, fh as i32).interpolation(Interpolation::CUBIC).done();
                    let a = src.get_data();
                    let mut d = img.get_data().as_slice().to_vec();
                    for (px, sa) in d.chunks_exact_mut(4).zip(a.as_slice().chunks_exact(4)) {
                        px[3] = sa[3];
                    }
                    if let Some(n) = Image::create_from_data(fw as i32, fh as i32, false, ImageFormat::RGBA8, &PackedByteArray::from(d.as_slice())) {
                        img = n;
                    }
                }
            }
        }
        // the normal map and mean colour from a 1024 copy
        let mut small = img.duplicate_resource();
        let sm = 1024usize.min(fw.max(fh));
        small.resize_ex((fw * sm / fw.max(fh)).max(4) as i32, (fh * sm / fw.max(fh)).max(4) as i32).interpolation(Interpolation::LANCZOS).done();
        let (normal, mean) = crate::gfx::derive_maps_image(&small, !alpha);
        img.generate_mipmaps();
        let _ = img.compress(CompressMode::S3TC);
        let out = self.out.join(format!("{}.hdtex", it.key));
        if let Err(e) = write(&out, &img, mean, alpha) {
            return GString::from(format!("hd: {}: {e}", out.display()).as_str());
        }
        if let Some(mut n) = normal {
            let _ = n.compress(CompressMode::S3TC);
            let _ = write(&self.out.join(format!("{}.hdnrm", it.key)), &n, mean, false);
        }
        for d in ["a_in", "a_mid", "a_out", "b_in", "b_out"] {
            let _ = std::fs::remove_file(self.work.join(d).join(format!("{i:05}.png")));
        }
        GString::from(format!("hd: {} {}x{} -> {}x{}", it.key, it.w, it.h, fw, fh).as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_scope() {
        assert_eq!(key_of("textures/ATCS/Floor.TGA"), "textures/atcs/floor");
        assert_eq!(key_of("models\\buildables\\reactor\\reactor.jpg"), "models/buildables/reactor/reactor");
        assert!(wanted("textures/atcs/floor.tga"));
        assert!(wanted("env/atcs/sky_up.jpg"));
        assert!(!wanted("gfx/2d/crosshair.tga"));
        assert!(!wanted("ui/assets/title.tga"));
        assert!(!wanted("textures/atcs/floor.shader"));
    }

    #[test]
    fn mip_chain_sizes() {
        // 4096 DXT1 top level 8 MiB, 4x4 minimum block
        assert_eq!(level_bytes(ImageFormat::DXT1, 4096, 4096), 8 * 1024 * 1024);
        assert_eq!(level_bytes(ImageFormat::DXT5, 2, 1), 16);
    }

    #[test]
    fn padding_wraps() {
        // 2x2: red green / blue white, padded by 1
        let px = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
        let p = wrap_pad(&px, 2, 2, 1);
        // the top-left padded texel is the bottom-right source texel
        assert_eq!(&p[0..3], &[255, 255, 255]);
        // the centre starts with the source's top-left
        assert_eq!(&p[(4 + 1) * 3..(4 + 1) * 3 + 3], &[255, 0, 0]);
    }
}
