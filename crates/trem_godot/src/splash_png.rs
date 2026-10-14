//! The boot splash with the version baked in (dec080). Godot draws
//! `application/boot_splash/image` before the extension is loaded, so the
//! version cannot be drawn at run time: build.rs includes this file and writes
//! godot/splash/boot_splash.png from splash/base.png (the TREMULOUS title from
//! the game's menu render, "REVAMPED", "Loading...") plus the version line,
//! drawn with splash/glyphs.png (ASCII 32..126, DejaVu Sans 20 px, white
//! coverage, one strip; widths in GLYPH_W). Only 8-bit grey / RGBA,
//! non-interlaced PNGs are read; zlib is miniz_oxide (already in Cargo.lock
//! through zip).

/// Advance of each glyph in glyphs.png, ASCII 32 (space) to 126 (`~`).
pub const GLYPH_W: [u8; 95] = [
    6, 9, 10, 18, 14, 20, 17, 7, 9, 9, 11, 18, 7, 8, 7, 8, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    8, 8, 18, 18, 18, 12, 21, 15, 15, 15, 16, 14, 13, 17, 16, 7, 11, 15, 13, 18, 16, 17, 13, 17,
    15, 14, 16, 16, 15, 21, 15, 16, 15, 9, 8, 9, 18, 14, 11, 13, 14, 12, 14, 13, 9, 14, 14, 7, 9,
    13, 7, 20, 14, 13, 14, 14, 10, 11, 9, 14, 13, 17, 13, 13, 12, 14, 8, 14, 18,
];
/// Top row of the version line in base.png (a 1280x720 canvas, the title centred): between "REVAMPED" and "Loading...".
pub const VERSION_TOP: usize = 446;
/// The menu's version-line grey (forecolor .63 .63 .63).
pub const VERSION_RGB: [u8; 3] = [161, 161, 161];

pub struct Image {
    pub w: usize,
    pub h: usize,
    /// 1 (grey) or 4 (RGBA) bytes per pixel
    pub ch: usize,
    pub px: Vec<u8>,
}

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut table = [0u32; 256];
    for (n, t) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    let mut c = !0u32;
    for b in parts.iter().flat_map(|p| p.iter()) {
        c = table[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = (
        (p - a as i16).abs(),
        (p - b as i16).abs(),
        (p - c as i16).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn decode(png: &[u8]) -> Result<Image, String> {
    if !png.starts_with(SIG) {
        return Err("not a PNG".into());
    }
    let (mut pos, mut idat, mut hdr) = (SIG.len(), Vec::new(), None);
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &png[pos + 4..pos + 8];
        let data = png.get(pos + 8..pos + 8 + len).ok_or("truncated PNG")?;
        match kind {
            b"IHDR" if len == 13 => hdr = Some(data.to_vec()),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let h = hdr.ok_or("no IHDR")?;
    let (w, ht) = (
        u32::from_be_bytes(h[0..4].try_into().unwrap()) as usize,
        u32::from_be_bytes(h[4..8].try_into().unwrap()) as usize,
    );
    let ch = match (h[8], h[9], h[12]) {
        (8, 0, 0) => 1,
        (8, 6, 0) => 4,
        _ => return Err("only 8-bit grey or RGBA, non-interlaced".into()),
    };
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).map_err(|e| format!("{e:?}"))?;
    let stride = w * ch;
    if raw.len() != ht * (stride + 1) {
        return Err("bad image data size".into());
    }
    let mut px = vec![0u8; ht * stride];
    for y in 0..ht {
        let f = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= ch { px[y * stride + x - ch] } else { 0 };
            let b = if y > 0 { px[(y - 1) * stride + x] } else { 0 };
            let c = if x >= ch && y > 0 {
                px[(y - 1) * stride + x - ch]
            } else {
                0
            };
            let pred = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err("bad filter".into()),
            };
            px[y * stride + x] = src[x].wrapping_add(pred);
        }
    }
    Ok(Image { w, h: ht, ch, px })
}

/// An RGBA image as PNG; per row the filter with the smallest sum, zlib level 9.
pub fn encode_rgba(img: &Image) -> Vec<u8> {
    assert_eq!(img.ch, 4);
    let stride = img.w * 4;
    let mut raw = Vec::with_capacity(img.h * (stride + 1));
    let mut row = vec![0u8; stride];
    for y in 0..img.h {
        let mut best: Option<(u64, u8, Vec<u8>)> = None;
        for f in 0..5u8 {
            for x in 0..stride {
                let cur = img.px[y * stride + x];
                let a = if x >= 4 {
                    img.px[y * stride + x - 4]
                } else {
                    0
                };
                let b = if y > 0 {
                    img.px[(y - 1) * stride + x]
                } else {
                    0
                };
                let c = if x >= 4 && y > 0 {
                    img.px[(y - 1) * stride + x - 4]
                } else {
                    0
                };
                let pred = match f {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    _ => paeth(a, b, c),
                };
                row[x] = cur.wrapping_sub(pred);
            }
            let cost = row
                .iter()
                .map(|&v| (v as i8).unsigned_abs() as u64)
                .sum::<u64>();
            if best.as_ref().is_none_or(|b| cost < b.0) {
                best = Some((cost, f, row.clone()));
            }
        }
        let (_, f, r) = best.unwrap();
        raw.push(f);
        raw.extend_from_slice(&r);
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(img.w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(img.h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = SIG.to_vec();
    for (kind, data) in [
        (&b"IHDR"[..], ihdr),
        (b"IDAT", miniz_oxide::deflate::compress_to_vec_zlib(&raw, 9)),
        (b"IEND", Vec::new()),
    ] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(&data);
        out.extend_from_slice(&crc32(&[kind, &data]).to_be_bytes());
    }
    out
}

/// Draws `text` centred at row `top` over an RGBA image; characters outside
/// ASCII 32..126 are left out. Returns how many glyphs were drawn.
pub fn draw_text(img: &mut Image, glyphs: &Image, text: &str, top: usize, rgb: [u8; 3]) -> usize {
    let mut x0 = vec![0usize; GLYPH_W.len() + 1];
    for (i, w) in GLYPH_W.iter().enumerate() {
        x0[i + 1] = x0[i] + *w as usize;
    }
    let idx: Vec<usize> = text
        .chars()
        .filter(|c| (' '..='~').contains(c))
        .map(|c| c as usize - 32)
        .collect();
    let width: usize = idx.iter().map(|&i| GLYPH_W[i] as usize).sum();
    let mut pen = img.w.saturating_sub(width) / 2;
    for &i in &idx {
        for gy in 0..glyphs.h.min(img.h.saturating_sub(top)) {
            for gx in 0..GLYPH_W[i] as usize {
                let (x, y) = (pen + gx, top + gy);
                if x >= img.w {
                    break;
                }
                let t = glyphs.px[(gy * glyphs.w + x0[i] + gx) * glyphs.ch] as f32 / 255.0;
                if t == 0.0 {
                    continue;
                }
                let p = &mut img.px[(y * img.w + x) * 4..(y * img.w + x) * 4 + 4];
                let ba = p[3] as f32 / 255.0;
                let oa = t + ba * (1.0 - t);
                for k in 0..3 {
                    let v = (rgb[k] as f32 * t + p[k] as f32 * ba * (1.0 - t)) / oa;
                    p[k] = v.round().clamp(0.0, 255.0) as u8;
                }
                p[3] = (oa * 255.0).round() as u8;
            }
        }
        pen += GLYPH_W[i] as usize;
    }
    idx.len()
}

/// The splash PNG: base.png with `line` at VERSION_TOP, or base.png's own
/// bytes when the line is empty (no version: B without a version line).
pub fn bake(base_png: &[u8], glyphs_png: &[u8], line: &str) -> Result<Vec<u8>, String> {
    if line.trim().is_empty() {
        return Ok(base_png.to_vec());
    }
    let mut img = decode(base_png)?;
    let glyphs = decode(glyphs_png)?;
    if img.ch != 4 {
        return Err("base.png must be RGBA".into());
    }
    if draw_text(&mut img, &glyphs, line, VERSION_TOP, VERSION_RGB) == 0 {
        return Ok(base_png.to_vec());
    }
    Ok(encode_rgba(&img))
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &[u8] = include_bytes!("../splash/base.png");
    const GLYPHS: &[u8] = include_bytes!("../splash/glyphs.png");

    #[test]
    fn splash_glyph_table_matches_atlas() {
        let g = decode(GLYPHS).unwrap();
        assert_eq!(g.ch, 1);
        assert_eq!(GLYPH_W.iter().map(|&w| w as usize).sum::<usize>(), g.w);
        assert!(GLYPH_W.iter().all(|&w| w > 0));
    }

    #[test]
    fn splash_png_round_trip() {
        let b = decode(BASE).unwrap();
        let again = decode(&encode_rgba(&b)).unwrap();
        assert_eq!((again.w, again.h, again.ch), (b.w, b.h, 4));
        assert!(again.px == b.px);
    }

    /// No version (or nothing drawable): the splash is base.png unchanged.
    #[test]
    fn splash_without_version_is_the_base() {
        assert_eq!(bake(BASE, GLYPHS, "").unwrap(), BASE);
        assert_eq!(bake(BASE, GLYPHS, "  ").unwrap(), BASE);
        assert_eq!(bake(BASE, GLYPHS, "\u{e9}\u{e9}").unwrap(), BASE);
    }

    /// The version line changes only its own band, centred, in the menu's grey.
    #[test]
    fn splash_version_line_drawn_in_its_band() {
        let line = crate::version_line::version_line("v1.0-12-gabc1234", "2026-10-14");
        let b = decode(BASE).unwrap();
        let s = decode(&bake(BASE, GLYPHS, &line).unwrap()).unwrap();
        assert_eq!((s.w, s.h), (b.w, b.h));
        let (mut minx, mut maxx, mut n) = (usize::MAX, 0, 0);
        for y in 0..b.h {
            for x in 0..b.w {
                let i = (y * b.w + x) * 4;
                if s.px[i..i + 4] != b.px[i..i + 4] {
                    assert!(
                        (VERSION_TOP..VERSION_TOP + 25).contains(&y),
                        "row {y} changed"
                    );
                    (minx, maxx, n) = (minx.min(x), maxx.max(x), n + 1);
                }
            }
        }
        assert!(n > 200, "{n} pixels changed");
        let centre = (minx + maxx) as i64 / 2 - b.w as i64 / 2;
        assert!(centre.abs() <= 6, "off centre by {centre}");
        // some pixel carries the full grey
        assert!(s.px.chunks(4).any(|p| p[..3] == VERSION_RGB && p[3] == 255));
    }
}
