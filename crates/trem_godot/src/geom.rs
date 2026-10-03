//! Presentation geometry: Quake <-> Godot coordinate mapping and brush
//! polygonisation (the classic "big quad clipped by every other plane").

use godot::prelude::*;
use trem_sim::gen::brushes::BrushDef;

/// Quake units per metre is ~32 (a 56-unit human is ~1.75 m).
pub const SCALE: f32 = 1.0 / 32.0;

/// Quake (x fwd, y left, z up) -> Godot (x right, y up, z back), scaled.
#[inline]
pub fn q2g(v: &[f32; 3]) -> Vector3 {
    Vector3::new(v[0] * SCALE, v[2] * SCALE, -v[1] * SCALE)
}
/// Direction mapping (no scale).
#[inline]
pub fn q2g_dir(v: &[f32; 3]) -> Vector3 {
    Vector3::new(v[0], v[2], -v[1])
}

/// Camera/body basis from Quake view angles: columns right, up, back.
pub fn basis_from_angles(angles: &[f32; 3]) -> Basis {
    let (f, r, u) = trem_sim::kernel::qmath::angle_vectors(angles);
    Basis::from_cols(q2g_dir(&r), q2g_dir(&u), -q2g_dir(&f))
}

/// Convex polygons (Quake space) of a sheet brush, via the BSP kernel's windings.
pub fn brush_polygons(b: &BrushDef) -> Vec<(Vec<[f64; 3]>, [f64; 3])> {
    let mut p: Vec<[f64; 4]> = Vec::new();
    for k in 0..3 {
        let mut n = [0.0; 4];
        n[k] = 1.0;
        n[3] = b.maxs[k] as f64;
        p.push(n);
        let mut n = [0.0; 4];
        n[k] = -1.0;
        n[3] = -b.mins[k] as f64;
        p.push(n);
    }
    if let Some(c) = b.cut {
        p.push(c.map(|x| x as f64));
    }
    trem_sim::kernel::bsp::brush_windings(&p)
}

/// "#rrggbb" -> Color.
pub fn hex_color(s: &str) -> Color {
    let h = s.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128) as f32 / 255.0;
    Color::from_rgb(p(0), p(2), p(4))
}
