//! Collision queries in the text format of re/harness/cm_ref.c, so the port's
//! answers can be compared byte-for-byte with the original C on real BSPs.
//! (Verification tooling, not game code.)

use crate::kernel::cm::ClipMap;
use crate::kernel::qmath::Vec3;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn unit(&mut self) -> f32 {
        (self.next() & 0xffffff) as f32 / 16777216.0
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
    /// Coordinates are often on 1/8 grid (like snapped origins), sometimes not.
    fn coord(&mut self, a: f32, b: f32) -> f32 {
        let v = self.range(a, b);
        if self.below(3) == 0 {
            v
        } else {
            (v * 8.0).round() / 8.0
        }
    }
}

const HULLS: [([f32; 3], [f32; 3]); 7] = [
    ([0.0; 3], [0.0; 3]),
    ([-15.0, -15.0, -24.0], [15.0, 15.0, 32.0]),
    ([-15.0, -15.0, -24.0], [15.0, 15.0, 16.0]),
    ([-15.0, -15.0, -15.0], [15.0, 15.0, 15.0]),
    ([-32.0, -32.0, -21.0], [32.0, 32.0, 48.0]),
    ([-3.0, -3.0, -3.0], [3.0, 3.0, 3.0]),
    ([-8.0, -4.0, -10.0], [12.0, 4.0, 2.0]),
];
const MASKS: [i32; 3] = [0x1 | 0x10000 | 0x2000000, 0x1, 0x1 | 0x2000000 | 0x4000000];

fn hx(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

/// `n` deterministic queries biased towards patches and inline models.
pub fn queries(cm: &ClipMap, n: usize, seed: u64) -> Vec<String> {
    let mut r = Lcg(seed);
    let world = cm.models.first().copied().unwrap_or_default();
    let patches: Vec<([f32; 3], [f32; 3])> = cm.patches.iter().flatten().map(|p| (p.pc.bounds[0], p.pc.bounds[1])).collect();
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let kind = r.below(10);
        let (mn, mx) = HULLS[r.below(HULLS.len())];
        let mask = MASKS[r.below(MASKS.len())];
        let near = |r: &mut Lcg, b: &([f32; 3], [f32; 3]), pad: f32| -> Vec3 { [r.coord(b.0[0] - pad, b.1[0] + pad), r.coord(b.0[1] - pad, b.1[1] + pad), r.coord(b.0[2] - pad, b.1[2] + pad)] };
        let wb = (world.mins, world.maxs);
        if kind == 9 && cm.models.len() > 1 {
            // inline model, moved and maybe rotated
            let m = 1 + r.below(cm.models.len() - 1);
            let mb = (cm.models[m].mins, cm.models[m].maxs);
            let origin = if r.below(2) == 0 { [0.0; 3] } else { [r.coord(-64.0, 64.0), r.coord(-64.0, 64.0), r.coord(-64.0, 64.0)] };
            let angles = match r.below(3) {
                0 => [0.0; 3],
                1 => [0.0, r.coord(0.0, 360.0), 0.0],
                _ => [r.coord(-90.0, 90.0), r.coord(0.0, 360.0), r.coord(-45.0, 45.0)],
            };
            let s = near(&mut r, &mb, 48.0);
            let s = [s[0] + origin[0], s[1] + origin[1], s[2] + origin[2]];
            if r.below(5) == 0 {
                out.push(format!("C {m} {} {} {} {} {} {} {} {} {}", hx(s[0]), hx(s[1]), hx(s[2]), hx(origin[0]), hx(origin[1]), hx(origin[2]), hx(angles[0]), hx(angles[1]), hx(angles[2])));
                continue;
            }
            let e = if r.below(6) == 0 { s } else { near(&mut r, &mb, 64.0) };
            let e = if e == s { s } else { [e[0] + origin[0], e[1] + origin[1], e[2] + origin[2]] };
            out.push(format!(
                "T {m} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {:x}",
                hx(s[0]), hx(s[1]), hx(s[2]), hx(e[0]), hx(e[1]), hx(e[2]), hx(mn[0]), hx(mn[1]), hx(mn[2]), hx(mx[0]), hx(mx[1]), hx(mx[2]), hx(origin[0]), hx(origin[1]), hx(origin[2]), hx(angles[0]), hx(angles[1]), hx(angles[2]), mask
            ));
            continue;
        }
        let around = if !patches.is_empty() && kind < 6 { patches[r.below(patches.len())] } else { wb };
        let s = near(&mut r, &around, if kind < 6 { 40.0 } else { 0.0 });
        if kind == 8 {
            out.push(format!("C 0 {} {} {} {} {} {} {} {} {}", hx(s[0]), hx(s[1]), hx(s[2]), hx(0.0), hx(0.0), hx(0.0), hx(0.0), hx(0.0), hx(0.0)));
            continue;
        }
        let e = match r.below(5) {
            0 => s,
            1 => [s[0], s[1], s[2] - r.coord(1.0, 256.0)],
            2 => near(&mut r, &around, 64.0),
            _ => {
                let len = r.range(1.0, 512.0);
                let d = [r.range(-1.0, 1.0), r.range(-1.0, 1.0), r.range(-1.0, 1.0)];
                [s[0] + d[0] * len, s[1] + d[1] * len, s[2] + d[2] * len]
            }
        };
        out.push(format!(
            "T 0 {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {:x}",
            hx(s[0]), hx(s[1]), hx(s[2]), hx(e[0]), hx(e[1]), hx(e[2]), hx(mn[0]), hx(mn[1]), hx(mn[2]), hx(mx[0]), hx(mx[1]), hx(mx[2]), hx(0.0), hx(0.0), hx(0.0), hx(0.0), hx(0.0), hx(0.0), mask
        ));
    }
    out
}

fn fv(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap_or(0))
}

/// The patch summary lines cm_ref prints after loading.
pub fn patch_lines(cm: &ClipMap) -> Vec<String> {
    let mut out = Vec::new();
    for (i, p) in cm.patches.iter().enumerate() {
        let Some(p) = p else { continue };
        let pc = &p.pc;
        let mut h: u32 = 2166136261;
        let mut mix = |v: u32| {
            h ^= v;
            h = h.wrapping_mul(16777619);
        };
        for pl in &pc.planes {
            for k in 0..4 {
                mix(pl.plane[k].to_bits());
            }
        }
        for f in &pc.facets {
            mix(f.surface_plane as u32);
            mix(f.num_borders as u32);
            for k in 0..f.num_borders {
                mix(f.border_planes[k] as u32);
                mix(f.border_inward[k] as u32);
            }
        }
        let b = pc.bounds;
        out.push(format!("P {i} {} {} {} {} {} {} {} {} {:08x}", pc.planes.len(), pc.facets.len(), hx(b[0][0]), hx(b[0][1]), hx(b[0][2]), hx(b[1][0]), hx(b[1][1]), hx(b[1][2]), h));
    }
    out
}

/// Answer one query line exactly as cm_ref would print it.
pub fn answer(cm: &ClipMap, q: &str) -> String {
    let t: Vec<&str> = q.split_whitespace().collect();
    let v3 = |i: usize| [fv(t[i]), fv(t[i + 1]), fv(t[i + 2])];
    let model: usize = t[1].parse().unwrap_or(0);
    if t[0] == "C" {
        let (p, o, a) = (v3(2), v3(5), v3(8));
        let c = if model == 0 { cm.point_contents(&p) } else { cm.transformed_point_contents(&p, model, &o, &a) };
        return format!("{c:x}");
    }
    let (s, e, mn, mx, o, a) = (v3(2), v3(5), v3(8), v3(11), v3(14), v3(17));
    let mask = i32::from_str_radix(t[20], 16).unwrap_or(1);
    let tr = if model == 0 { cm.box_trace(&s, &e, Some(&mn), Some(&mx), mask) } else { cm.transformed_box_trace(&s, &e, &mn, &mx, model, mask, &o, &a) };
    format!(
        "{} {} {} {} {} {} {} {} {} {} {:x} {:x}",
        hx(tr.fraction),
        hx(tr.endpos[0]),
        hx(tr.endpos[1]),
        hx(tr.endpos[2]),
        hx(tr.plane.normal[0]),
        hx(tr.plane.normal[1]),
        hx(tr.plane.normal[2]),
        hx(tr.plane.dist),
        tr.allsolid as i32,
        tr.startsolid as i32,
        tr.contents,
        tr.surface_flags
    )
}
