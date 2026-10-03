//! Kernel `patch` — port of qcommon/cm_patch.c (curved-surface collision) and
//! the cm_polylib.c winding helpers it uses.
//!
//! Float semantics follow the C exactly: arithmetic on `float`s stays f32,
//! anything mixed with a double literal (0.5*, SURFACE_CLIP_EPSILON, 0.1,
//! fabs) is evaluated in f64 and rounded where C stores it.

use super::cm::SURFACE_CLIP_EPSILON;
use super::qmath::*;

pub const MAX_FACETS: usize = 1024;
pub const MAX_PATCH_PLANES: usize = 2048;
pub const MAX_GRID_SIZE: usize = 129;
const SUBDIVIDE_DISTANCE: f32 = 16.0;
const PLANE_TRI_EPSILON: f64 = 0.1;
const WRAP_POINT_EPSILON: f64 = 0.1;
const POINT_EPSILON: f64 = 0.1;
const NORMAL_EPSILON: f64 = 0.0001;
const DIST_EPSILON: f64 = 0.02;
const MAX_MAP_BOUNDS: f32 = 65535.0;
const MAX_POINTS_ON_WINDING: usize = 64;
const MAX_BORDERS: usize = 4 + 6 + 16;

const SIDE_FRONT: i32 = 0;
const SIDE_BACK: i32 = 1;
const SIDE_ON: i32 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PatchPlane {
    pub plane: [f32; 4],
    pub signbits: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facet {
    pub surface_plane: i32,
    pub num_borders: usize,
    pub border_planes: [i32; MAX_BORDERS],
    pub border_inward: [bool; MAX_BORDERS],
    pub border_no_adjust: [bool; MAX_BORDERS],
}

impl Default for Facet {
    fn default() -> Self {
        Facet { surface_plane: 0, num_borders: 0, border_planes: [0; MAX_BORDERS], border_inward: [false; MAX_BORDERS], border_no_adjust: [false; MAX_BORDERS] }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatchCollide {
    pub bounds: [Vec3; 2],
    pub planes: Vec<PatchPlane>,
    pub facets: Vec<Facet>,
}

// ------------------------------------------------------------------ windings

pub type Winding = Vec<Vec3>;

/// `BaseWindingForPlane`.
pub fn base_winding_for_plane(normal: &Vec3, dist: f32) -> Winding {
    let mut max = -MAX_MAP_BOUNDS;
    let mut x = usize::MAX;
    for i in 0..3 {
        let v = normal[i].abs();
        if v > max {
            x = i;
            max = v;
        }
    }
    let mut vup = [0.0f32; 3];
    match x {
        0 | 1 => vup[2] = 1.0,
        _ => vup[0] = 1.0,
    }
    let v = dot(&vup, normal);
    vup = ma(&vup, -v, normal);
    let mut n = [0.0; 3];
    normalize2(&vup, &mut n);
    vup = n;
    let org = scale(normal, dist);
    let vright = cross(&vup, normal);
    let vup = scale(&vup, MAX_MAP_BOUNDS);
    let vright = scale(&vright, MAX_MAP_BOUNDS);
    vec![add(&sub(&org, &vright), &vup), add(&add(&org, &vright), &vup), sub(&add(&org, &vright), &vup), sub(&sub(&org, &vright), &vup)]
}

/// `WindingBounds`.
pub fn winding_bounds(w: &Winding) -> (Vec3, Vec3) {
    let mut mins = [MAX_MAP_BOUNDS; 3];
    let mut maxs = [-MAX_MAP_BOUNDS; 3];
    for p in w {
        for j in 0..3 {
            let v = p[j];
            if v < mins[j] {
                mins[j] = v;
            }
            if v > maxs[j] {
                maxs[j] = v;
            }
        }
    }
    (mins, maxs)
}

/// `ChopWindingInPlace`: None when chopped away entirely.
pub fn chop_winding_in_place(w: Option<Winding>, normal: &Vec3, dist: f32, epsilon: f32) -> Result<Option<Winding>, String> {
    let Some(inw) = w else { return Ok(None) };
    let n = inw.len();
    let mut dists = vec![0.0f32; n + 1];
    let mut sides = vec![0i32; n + 1];
    let mut counts = [0usize; 3];
    for i in 0..n {
        let mut d = dot(&inw[i], normal);
        d -= dist;
        dists[i] = d;
        sides[i] = if d > epsilon {
            SIDE_FRONT
        } else if d < -epsilon {
            SIDE_BACK
        } else {
            SIDE_ON
        };
        counts[sides[i] as usize] += 1;
    }
    sides[n] = sides[0];
    dists[n] = dists[0];
    if counts[0] == 0 {
        return Ok(None);
    }
    if counts[1] == 0 {
        return Ok(Some(inw));
    }
    let maxpts = n + 4;
    let mut f: Winding = Vec::with_capacity(maxpts);
    for i in 0..n {
        let p1 = inw[i];
        if sides[i] == SIDE_ON {
            f.push(p1);
            continue;
        }
        if sides[i] == SIDE_FRONT {
            f.push(p1);
        }
        if sides[i + 1] == SIDE_ON || sides[i + 1] == sides[i] {
            continue;
        }
        let p2 = inw[(i + 1) % n];
        let d = dists[i] / (dists[i] - dists[i + 1]);
        let mut mid = [0.0f32; 3];
        for j in 0..3 {
            mid[j] = if normal[j] == 1.0 {
                dist
            } else if normal[j] == -1.0 {
                -dist
            } else {
                p1[j] + d * (p2[j] - p1[j])
            };
        }
        f.push(mid);
    }
    if f.len() > maxpts {
        return Err("ClipWinding: points exceeded estimate".into());
    }
    if f.len() > MAX_POINTS_ON_WINDING {
        return Err("ClipWinding: MAX_POINTS_ON_WINDING".into());
    }
    Ok(Some(f))
}

// --------------------------------------------------------------------- grid

struct Grid {
    width: usize,
    height: usize,
    wrap_width: bool,
    wrap_height: bool,
    /// [width][height]
    points: Vec<Vec3>,
}

impl Grid {
    fn p(&self, i: usize, j: usize) -> Vec3 {
        self.points[i * MAX_GRID_SIZE + j]
    }
    fn set(&mut self, i: usize, j: usize, v: Vec3) {
        self.points[i * MAX_GRID_SIZE + j] = v;
    }
}

fn signbits_for_normal(n: &[f32]) -> u8 {
    let mut bits = 0;
    for j in 0..3 {
        if n[j] < 0.0 {
            bits |= 1 << j;
        }
    }
    bits
}

/// `CM_PlaneFromPoints`.
fn plane_from_points(a: &Vec3, b: &Vec3, c: &Vec3) -> Option<[f32; 4]> {
    let d1 = sub(b, a);
    let d2 = sub(c, a);
    let mut n = cross(&d2, &d1);
    if normalize(&mut n) == 0.0 {
        return None;
    }
    Some([n[0], n[1], n[2], dot(a, &n)])
}

/// `CM_NeedsSubdivision`.
fn needs_subdivision(a: &Vec3, b: &Vec3, c: &Vec3) -> bool {
    let mut lmid = [0.0f32; 3];
    let mut cmid = [0.0f32; 3];
    for i in 0..3 {
        lmid[i] = (0.5 * (a[i] + c[i]) as f64) as f32;
    }
    for i in 0..3 {
        cmid[i] = (0.5 * (0.5 * (a[i] + b[i]) as f64 + 0.5 * (b[i] + c[i]) as f64)) as f32;
    }
    let delta = sub(&cmid, &lmid);
    length(&delta) >= SUBDIVIDE_DISTANCE
}

/// `CM_Subdivide`.
fn subdivide(a: &Vec3, b: &Vec3, c: &Vec3) -> (Vec3, Vec3, Vec3) {
    let mut o1 = [0.0f32; 3];
    let mut o2 = [0.0f32; 3];
    let mut o3 = [0.0f32; 3];
    for i in 0..3 {
        o1[i] = (0.5 * (a[i] + b[i]) as f64) as f32;
        o3[i] = (0.5 * (b[i] + c[i]) as f64) as f32;
        o2[i] = (0.5 * (o1[i] + o3[i]) as f64) as f32;
    }
    (o1, o2, o3)
}

fn transpose_grid(g: &mut Grid) {
    if g.width > g.height {
        for i in 0..g.height {
            for j in i + 1..g.width {
                if j < g.height {
                    let t = g.p(i, j);
                    let v = g.p(j, i);
                    g.set(i, j, v);
                    g.set(j, i, t);
                } else {
                    let v = g.p(j, i);
                    g.set(i, j, v);
                }
            }
        }
    } else {
        for i in 0..g.width {
            for j in i + 1..g.height {
                if j < g.width {
                    let t = g.p(j, i);
                    let v = g.p(i, j);
                    g.set(j, i, v);
                    g.set(i, j, t);
                } else {
                    let v = g.p(i, j);
                    g.set(j, i, v);
                }
            }
        }
    }
    std::mem::swap(&mut g.width, &mut g.height);
    std::mem::swap(&mut g.wrap_width, &mut g.wrap_height);
}

fn set_grid_wrap_width(g: &mut Grid) {
    let mut i = 0;
    while i < g.height {
        let mut j = 0;
        while j < 3 {
            let d = g.p(0, i)[j] - g.p(g.width - 1, i)[j];
            if (d as f64) < -WRAP_POINT_EPSILON || (d as f64) > WRAP_POINT_EPSILON {
                break;
            }
            j += 1;
        }
        if j != 3 {
            break;
        }
        i += 1;
    }
    g.wrap_width = i == g.height;
}

fn subdivide_grid_columns(g: &mut Grid) -> Result<(), String> {
    let mut i = 0usize;
    while i + 2 < g.width {
        let mut j = 0;
        while j < g.height {
            if needs_subdivision(&g.p(i, j), &g.p(i + 1, j), &g.p(i + 2, j)) {
                break;
            }
            j += 1;
        }
        if j == g.height {
            for j in 0..g.height {
                for k in i + 2..g.width {
                    let v = g.p(k, j);
                    g.set(k - 1, j, v);
                }
            }
            g.width -= 1;
            i += 1;
            continue;
        }
        if g.width + 2 > MAX_GRID_SIZE {
            return Err("CM_SubdivideGridColumns: grid too large".into());
        }
        for j in 0..g.height {
            let prev = g.p(i, j);
            let mid = g.p(i + 1, j);
            let next = g.p(i + 2, j);
            let mut k = g.width - 1;
            while k > i + 1 {
                let v = g.p(k, j);
                g.set(k + 2, j, v);
                k -= 1;
            }
            let (a, b, c) = subdivide(&prev, &mid, &next);
            g.set(i + 1, j, a);
            g.set(i + 2, j, b);
            g.set(i + 3, j, c);
        }
        g.width += 2;
    }
    Ok(())
}

fn compare_points(a: &Vec3, b: &Vec3) -> bool {
    for k in 0..3 {
        let d = (a[k] - b[k]) as f64;
        if d < -POINT_EPSILON || d > POINT_EPSILON {
            return false;
        }
    }
    true
}

fn remove_degenerate_columns(g: &mut Grid) {
    let mut i: isize = 0;
    while (i as i64) < g.width as i64 - 1 {
        let iu = i as usize;
        let mut j = 0;
        while j < g.height {
            if !compare_points(&g.p(iu, j), &g.p(iu + 1, j)) {
                break;
            }
            j += 1;
        }
        if j != g.height {
            i += 1;
            continue;
        }
        for j in 0..g.height {
            for k in iu + 2..g.width {
                let v = g.p(k, j);
                g.set(k - 1, j, v);
            }
        }
        g.width -= 1;
        // check against the next column (i-- then i++)
    }
}

// ------------------------------------------------------- collide generation

struct Gen {
    planes: Vec<PatchPlane>,
    facets: Vec<Facet>,
}

fn plane_equal(p: &PatchPlane, plane: &[f32; 4]) -> Option<bool> {
    let close = |a: &[f32; 4], b: &[f32; 4]| {
        ((a[0] - b[0]) as f64).abs() < NORMAL_EPSILON && ((a[1] - b[1]) as f64).abs() < NORMAL_EPSILON && ((a[2] - b[2]) as f64).abs() < NORMAL_EPSILON && ((a[3] - b[3]) as f64).abs() < DIST_EPSILON
    };
    if close(&p.plane, plane) {
        return Some(false);
    }
    let inv = [-plane[0], -plane[1], -plane[2], -plane[3]];
    if close(&p.plane, &inv) {
        return Some(true);
    }
    None
}

fn snap_vector(n: &mut Vec3) {
    for i in 0..3 {
        if ((n[i] - 1.0) as f64).abs() < NORMAL_EPSILON {
            *n = [0.0; 3];
            n[i] = 1.0;
            break;
        }
        if ((n[i] - -1.0) as f64).abs() < NORMAL_EPSILON {
            *n = [0.0; 3];
            n[i] = -1.0;
            break;
        }
    }
}

impl Gen {
    /// `CM_FindPlane2` -> (index, flipped)
    fn find_plane2(&mut self, plane: &[f32; 4]) -> Result<(i32, bool), String> {
        for (i, p) in self.planes.iter().enumerate() {
            if let Some(f) = plane_equal(p, plane) {
                return Ok((i as i32, f));
            }
        }
        if self.planes.len() == MAX_PATCH_PLANES {
            return Err("MAX_PATCH_PLANES".into());
        }
        self.planes.push(PatchPlane { plane: *plane, signbits: signbits_for_normal(plane) });
        Ok((self.planes.len() as i32 - 1, false))
    }

    /// `CM_FindPlane`
    fn find_plane(&mut self, p1: &Vec3, p2: &Vec3, p3: &Vec3) -> Result<i32, String> {
        let Some(plane) = plane_from_points(p1, p2, p3) else { return Ok(-1) };
        let pn = [plane[0], plane[1], plane[2]];
        'next: for (i, pp) in self.planes.iter().enumerate() {
            let n = [pp.plane[0], pp.plane[1], pp.plane[2]];
            if dot(&pn, &n) < 0.0 {
                continue;
            }
            for p in [p1, p2, p3] {
                let d = (dot(p, &n) - pp.plane[3]) as f64;
                if d < -PLANE_TRI_EPSILON || d > PLANE_TRI_EPSILON {
                    continue 'next;
                }
            }
            return Ok(i as i32);
        }
        if self.planes.len() == MAX_PATCH_PLANES {
            return Err("MAX_PATCH_PLANES".into());
        }
        self.planes.push(PatchPlane { plane, signbits: signbits_for_normal(&plane) });
        Ok(self.planes.len() as i32 - 1)
    }

    fn normal(&self, p: i32) -> Vec3 {
        let q = &self.planes[p as usize].plane;
        [q[0], q[1], q[2]]
    }

    fn point_on_plane_side(&self, p: &Vec3, plane_num: i32) -> i32 {
        if plane_num == -1 {
            return SIDE_ON;
        }
        let pl = &self.planes[plane_num as usize].plane;
        let d = (dot(p, &[pl[0], pl[1], pl[2]]) - pl[3]) as f64;
        if d > PLANE_TRI_EPSILON {
            return SIDE_FRONT;
        }
        if d < -PLANE_TRI_EPSILON {
            return SIDE_BACK;
        }
        SIDE_ON
    }

    fn edge_plane_num(&mut self, g: &Grid, gp: &[[i32; 2]], i: usize, j: usize, k: i32) -> Result<i32, String> {
        let gplane = |tri: usize| -> i32 {
            let c = gp[i * MAX_GRID_SIZE + j];
            if c[tri] != -1 {
                return c[tri];
            }
            if c[1 - tri] != -1 {
                return c[1 - tri];
            }
            -1
        };
        let (p1, p2, tri, swap) = match k {
            0 => (g.p(i, j), g.p(i + 1, j), 0, false),
            2 => (g.p(i, j + 1), g.p(i + 1, j + 1), 1, true),
            3 => (g.p(i, j), g.p(i, j + 1), 1, true),
            1 => (g.p(i + 1, j), g.p(i + 1, j + 1), 0, false),
            4 => (g.p(i + 1, j + 1), g.p(i, j), 0, false),
            5 => (g.p(i, j), g.p(i + 1, j + 1), 1, false),
            _ => return Err("CM_EdgePlaneNum: bad k".into()),
        };
        let p = gplane(tri);
        if p == -1 {
            return Ok(-1);
        }
        let up = ma(&p1, 4.0, &self.normal(p));
        if swap {
            self.find_plane(&p2, &p1, &up)
        } else {
            self.find_plane(&p1, &p2, &up)
        }
    }

    fn set_border_inward(&self, f: &mut Facet, g: &Grid, i: usize, j: usize, which: i32) {
        let pts: Vec<Vec3> = match which {
            -1 => vec![g.p(i, j), g.p(i + 1, j), g.p(i + 1, j + 1), g.p(i, j + 1)],
            0 => vec![g.p(i, j), g.p(i + 1, j), g.p(i + 1, j + 1)],
            _ => vec![g.p(i + 1, j + 1), g.p(i, j + 1), g.p(i, j)],
        };
        for k in 0..f.num_borders {
            let (mut front, mut back) = (0, 0);
            for p in &pts {
                let side = self.point_on_plane_side(p, f.border_planes[k]);
                if side == SIDE_FRONT {
                    front += 1;
                }
                if side == SIDE_BACK {
                    back += 1;
                }
            }
            if front > 0 && back == 0 {
                f.border_inward[k] = true;
            } else if back > 0 && front == 0 {
                f.border_inward[k] = false;
            } else if front == 0 && back == 0 {
                f.border_planes[k] = -1;
            } else {
                f.border_inward[k] = false;
            }
        }
    }

    fn border_plane(&self, f: &Facet, j: usize) -> [f32; 4] {
        let mut plane = self.planes[f.border_planes[j] as usize].plane;
        if !f.border_inward[j] {
            plane = [0.0 - plane[0], 0.0 - plane[1], 0.0 - plane[2], -plane[3]];
        }
        plane
    }

    fn validate_facet(&self, f: &Facet) -> Result<bool, String> {
        if f.surface_plane == -1 {
            return Ok(false);
        }
        let plane = self.planes[f.surface_plane as usize].plane;
        let mut w = Some(base_winding_for_plane(&[plane[0], plane[1], plane[2]], plane[3]));
        let mut j = 0;
        while j < f.num_borders && w.is_some() {
            if f.border_planes[j] == -1 {
                return Ok(false);
            }
            let p = self.border_plane(f, j);
            w = chop_winding_in_place(w, &[p[0], p[1], p[2]], p[3], 0.1)?;
            j += 1;
        }
        let Some(w) = w else { return Ok(false) };
        let (mins, maxs) = winding_bounds(&w);
        for j in 0..3 {
            if maxs[j] - mins[j] > MAX_MAP_BOUNDS {
                return Ok(false);
            }
            if mins[j] >= MAX_MAP_BOUNDS {
                return Ok(false);
            }
            if maxs[j] <= -MAX_MAP_BOUNDS {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn add_border(&mut self, f: &mut Facet, plane: &[f32; 4]) -> Result<bool, String> {
        if f.num_borders >= MAX_BORDERS {
            return Ok(false);
        }
        let (idx, flipped) = self.find_plane2(plane)?;
        f.border_planes[f.num_borders] = idx;
        f.border_no_adjust[f.num_borders] = false;
        f.border_inward[f.num_borders] = flipped;
        Ok(true)
    }

    fn present(&self, f: &Facet, plane: &[f32; 4]) -> bool {
        if plane_equal(&self.planes[f.surface_plane as usize], plane).is_some() {
            return true;
        }
        (0..f.num_borders).any(|i| plane_equal(&self.planes[f.border_planes[i] as usize], plane).is_some())
    }

    /// `CM_AddFacetBevels`.
    fn add_facet_bevels(&mut self, f: &mut Facet) -> Result<(), String> {
        let plane = self.planes[f.surface_plane as usize].plane;
        let mut w = Some(base_winding_for_plane(&[plane[0], plane[1], plane[2]], plane[3]));
        let mut j = 0;
        while j < f.num_borders && w.is_some() {
            if f.border_planes[j] != f.surface_plane {
                let p = self.border_plane(f, j);
                w = chop_winding_in_place(w, &[p[0], p[1], p[2]], p[3], 0.1)?;
            }
            j += 1;
        }
        let Some(w) = w else { return Ok(()) };
        let (mins, maxs) = winding_bounds(&w);

        // axial planes
        for axis in 0..3 {
            for dir in [-1.0f32, 1.0] {
                let mut plane = [0.0f32; 4];
                plane[axis] = dir;
                plane[3] = if dir == 1.0 { maxs[axis] } else { -mins[axis] };
                if self.present(f, &plane) {
                    continue;
                }
                if self.add_border(f, &plane)? {
                    f.num_borders += 1;
                }
            }
        }

        // edge bevels
        let n = w.len();
        for j in 0..n {
            let k = (j + 1) % n;
            let mut vec = sub(&w[j], &w[k]);
            if normalize(&mut vec) < 0.5 {
                continue;
            }
            snap_vector(&mut vec);
            if (0..3).any(|k| vec[k] == -1.0 || vec[k] == 1.0) {
                continue;
            }
            for axis in 0..3 {
                for dir in [-1.0f32, 1.0] {
                    let mut vec2 = [0.0f32; 3];
                    vec2[axis] = dir;
                    let mut pn = cross(&vec, &vec2);
                    if normalize(&mut pn) < 0.5 {
                        continue;
                    }
                    let plane = [pn[0], pn[1], pn[2], dot(&w[j], &pn)];
                    if w.iter().any(|p| ((dot(p, &pn) - plane[3]) as f64) > 0.1) {
                        continue;
                    }
                    if self.present(f, &plane) {
                        continue;
                    }
                    if !self.add_border(f, &plane)? {
                        continue;
                    }
                    let nb = f.num_borders;
                    let mut np = self.planes[f.border_planes[nb] as usize].plane;
                    if !f.border_inward[nb] {
                        np = [-np[0], -np[1], -np[2], -np[3]];
                    }
                    let w2 = chop_winding_in_place(Some(w.clone()), &[np[0], np[1], np[2]], np[3], 0.1)?;
                    if w2.is_none() {
                        continue;
                    }
                    f.num_borders += 1;
                }
            }
        }

        // add opposite plane
        if f.num_borders >= MAX_BORDERS {
            return Ok(());
        }
        f.border_planes[f.num_borders] = f.surface_plane;
        f.border_no_adjust[f.num_borders] = false;
        f.border_inward[f.num_borders] = true;
        f.num_borders += 1;
        Ok(())
    }

    fn push_facet(&mut self, mut f: Facet) -> Result<(), String> {
        if self.validate_facet(&f)? {
            self.add_facet_bevels(&mut f)?;
            self.facets.push(f);
        }
        Ok(())
    }

    /// `CM_PatchCollideFromGrid`.
    fn from_grid(&mut self, g: &Grid) -> Result<(), String> {
        let mut gp = vec![[-1i32; 2]; MAX_GRID_SIZE * MAX_GRID_SIZE];
        let at = |i: usize, j: usize| i * MAX_GRID_SIZE + j;
        for i in 0..g.width - 1 {
            for j in 0..g.height - 1 {
                gp[at(i, j)][0] = self.find_plane(&g.p(i, j), &g.p(i + 1, j), &g.p(i + 1, j + 1))?;
                gp[at(i, j)][1] = self.find_plane(&g.p(i + 1, j + 1), &g.p(i, j + 1), &g.p(i, j))?;
            }
        }
        const TOP: usize = 0;
        const RIGHT: usize = 1;
        const BOTTOM: usize = 2;
        const LEFT: usize = 3;
        for i in 0..g.width - 1 {
            for j in 0..g.height - 1 {
                let mut borders = [-1i32; 4];
                let mut no_adjust = [false; 4];
                let cur = gp[at(i, j)];

                borders[TOP] = -1;
                if j > 0 {
                    borders[TOP] = gp[at(i, j - 1)][1];
                } else if g.wrap_height {
                    borders[TOP] = gp[at(i, g.height - 2)][1];
                }
                no_adjust[TOP] = borders[TOP] == cur[0];
                if borders[TOP] == -1 || no_adjust[TOP] {
                    borders[TOP] = self.edge_plane_num(g, &gp, i, j, 0)?;
                }

                borders[BOTTOM] = -1;
                if j < g.height - 2 {
                    borders[BOTTOM] = gp[at(i, j + 1)][0];
                } else if g.wrap_height {
                    borders[BOTTOM] = gp[at(i, 0)][0];
                }
                no_adjust[BOTTOM] = borders[BOTTOM] == cur[1];
                if borders[BOTTOM] == -1 || no_adjust[BOTTOM] {
                    borders[BOTTOM] = self.edge_plane_num(g, &gp, i, j, 2)?;
                }

                borders[LEFT] = -1;
                if i > 0 {
                    borders[LEFT] = gp[at(i - 1, j)][0];
                } else if g.wrap_width {
                    borders[LEFT] = gp[at(g.width - 2, j)][0];
                }
                no_adjust[LEFT] = borders[LEFT] == cur[1];
                if borders[LEFT] == -1 || no_adjust[LEFT] {
                    borders[LEFT] = self.edge_plane_num(g, &gp, i, j, 3)?;
                }

                borders[RIGHT] = -1;
                if i < g.width - 2 {
                    borders[RIGHT] = gp[at(i + 1, j)][1];
                } else if g.wrap_width {
                    borders[RIGHT] = gp[at(0, j)][1];
                }
                no_adjust[RIGHT] = borders[RIGHT] == cur[0];
                if borders[RIGHT] == -1 || no_adjust[RIGHT] {
                    borders[RIGHT] = self.edge_plane_num(g, &gp, i, j, 1)?;
                }

                if self.facets.len() == MAX_FACETS {
                    return Err("MAX_FACETS".into());
                }
                if cur[0] == cur[1] {
                    if cur[0] == -1 {
                        continue;
                    }
                    let mut f = Facet { surface_plane: cur[0], num_borders: 4, ..Default::default() };
                    for (k, e) in [TOP, RIGHT, BOTTOM, LEFT].into_iter().enumerate() {
                        f.border_planes[k] = borders[e];
                        f.border_no_adjust[k] = no_adjust[e];
                    }
                    self.set_border_inward(&mut f, g, i, j, -1);
                    self.push_facet(f)?;
                } else {
                    let mut f = Facet { surface_plane: cur[0], num_borders: 3, ..Default::default() };
                    f.border_planes[0] = borders[TOP];
                    f.border_no_adjust[0] = no_adjust[TOP];
                    f.border_planes[1] = borders[RIGHT];
                    f.border_no_adjust[1] = no_adjust[RIGHT];
                    f.border_planes[2] = cur[1];
                    if f.border_planes[2] == -1 {
                        f.border_planes[2] = borders[BOTTOM];
                        if f.border_planes[2] == -1 {
                            f.border_planes[2] = self.edge_plane_num(g, &gp, i, j, 4)?;
                        }
                    }
                    self.set_border_inward(&mut f, g, i, j, 0);
                    self.push_facet(f)?;

                    if self.facets.len() == MAX_FACETS {
                        return Err("MAX_FACETS".into());
                    }
                    let mut f = Facet { surface_plane: cur[1], num_borders: 3, ..Default::default() };
                    f.border_planes[0] = borders[BOTTOM];
                    f.border_no_adjust[0] = no_adjust[BOTTOM];
                    f.border_planes[1] = borders[LEFT];
                    f.border_no_adjust[1] = no_adjust[LEFT];
                    f.border_planes[2] = cur[0];
                    if f.border_planes[2] == -1 {
                        f.border_planes[2] = borders[TOP];
                        if f.border_planes[2] == -1 {
                            f.border_planes[2] = self.edge_plane_num(g, &gp, i, j, 5)?;
                        }
                    }
                    self.set_border_inward(&mut f, g, i, j, 1);
                    self.push_facet(f)?;
                }
            }
        }
        Ok(())
    }
}

/// `CM_GeneratePatchCollide`: `points` are concatenated rows (width per row).
pub fn generate(width: usize, height: usize, points: &[Vec3]) -> Result<PatchCollide, String> {
    if width <= 2 || height <= 2 || points.len() < width * height {
        return Err(format!("CM_GeneratePatchFacets: bad parameters: ({width}, {height})"));
    }
    if width & 1 == 0 || height & 1 == 0 {
        return Err("CM_GeneratePatchFacets: even sizes are invalid for quadratic meshes".into());
    }
    if width > MAX_GRID_SIZE || height > MAX_GRID_SIZE {
        return Err("CM_GeneratePatchFacets: source is > MAX_GRID_SIZE".into());
    }
    let mut g = Grid { width, height, wrap_width: false, wrap_height: false, points: vec![[0.0; 3]; MAX_GRID_SIZE * MAX_GRID_SIZE] };
    for i in 0..width {
        for j in 0..height {
            g.set(i, j, points[j * width + i]);
        }
    }
    set_grid_wrap_width(&mut g);
    subdivide_grid_columns(&mut g)?;
    remove_degenerate_columns(&mut g);
    transpose_grid(&mut g);
    set_grid_wrap_width(&mut g);
    subdivide_grid_columns(&mut g)?;
    remove_degenerate_columns(&mut g);

    let (mut mins, mut maxs) = clear_bounds();
    for i in 0..g.width {
        for j in 0..g.height {
            add_point_to_bounds(&g.p(i, j), &mut mins, &mut maxs);
        }
    }
    let mut gen = Gen { planes: Vec::new(), facets: Vec::new() };
    gen.from_grid(&g)?;
    for k in 0..3 {
        mins[k] -= 1.0;
        maxs[k] += 1.0;
    }
    Ok(PatchCollide { bounds: [mins, maxs], planes: gen.planes, facets: gen.facets })
}

// ------------------------------------------------------------------ tracing

/// The traceWork_t members the patch code reads.
pub struct PatchWork<'a> {
    pub start: &'a Vec3,
    pub end: &'a Vec3,
    pub offsets: &'a [Vec3; 8],
    pub bounds: &'a [Vec3; 2],
    pub is_point: bool,
}

fn pdot(p: &[f32; 4], v: &Vec3) -> f32 {
    p[0] * v[0] + p[1] * v[1] + p[2] * v[2]
}

/// `CM_TracePointThroughPatchCollide` (cm_playerCurveClip 1). Returns the new
/// (fraction, plane) when a facet is hit closer than `fraction`.
fn trace_point(tw: &PatchWork, pc: &PatchCollide, fraction: &mut f32, hit: &mut Option<[f32; 4]>) {
    let n = pc.planes.len();
    let mut front = vec![false; n];
    let mut inter = vec![0.0f32; n];
    for (i, p) in pc.planes.iter().enumerate() {
        let offset = pdot(&p.plane, &tw.offsets[p.signbits as usize]);
        let d1 = pdot(&p.plane, tw.start) - p.plane[3] + offset;
        let d2 = pdot(&p.plane, tw.end) - p.plane[3] + offset;
        front[i] = d1 > 0.0;
        if d1 == d2 {
            inter[i] = 99999.0;
        } else {
            inter[i] = d1 / (d1 - d2);
            if inter[i] <= 0.0 {
                inter[i] = 99999.0;
            }
        }
    }
    for f in &pc.facets {
        let sp = f.surface_plane as usize;
        if !front[sp] {
            continue;
        }
        let intersect = inter[sp];
        if intersect < 0.0 {
            continue;
        }
        if intersect > *fraction {
            continue;
        }
        let mut j = 0;
        while j < f.num_borders {
            let k = f.border_planes[j] as usize;
            if front[k] ^ f.border_inward[j] {
                if inter[k] > intersect {
                    break;
                }
            } else if inter[k] < intersect {
                break;
            }
            j += 1;
        }
        if j == f.num_borders {
            let p = &pc.planes[sp];
            let offset = pdot(&p.plane, &tw.offsets[p.signbits as usize]);
            let d1 = pdot(&p.plane, tw.start) - p.plane[3] + offset;
            let d2 = pdot(&p.plane, tw.end) - p.plane[3] + offset;
            *fraction = ((d1 as f64 - SURFACE_CLIP_EPSILON) / (d1 - d2) as f64) as f32;
            if *fraction < 0.0 {
                *fraction = 0.0;
            }
            *hit = Some(p.plane);
        }
    }
}

/// `CM_CheckFacetPlane` -> (keep, hit)
fn check_facet_plane(plane: &[f32; 4], start: &Vec3, end: &Vec3, enter: &mut f32, leave: &mut f32) -> (bool, bool) {
    let d1 = pdot(plane, start) - plane[3];
    let d2 = pdot(plane, end) - plane[3];
    if d1 > 0.0 && ((d2 as f64) >= SURFACE_CLIP_EPSILON || d2 >= d1) {
        return (false, false);
    }
    if d1 <= 0.0 && d2 <= 0.0 {
        return (true, false);
    }
    let mut hit = false;
    if d1 > d2 {
        let mut f = ((d1 as f64 - SURFACE_CLIP_EPSILON) / (d1 - d2) as f64) as f32;
        if f < 0.0 {
            f = 0.0;
        }
        if f > *enter {
            *enter = f;
            hit = true;
        }
    } else {
        let mut f = ((d1 as f64 + SURFACE_CLIP_EPSILON) / (d1 - d2) as f64) as f32;
        if f > 1.0 {
            f = 1.0;
        }
        if f < *leave {
            *leave = f;
        }
    }
    (true, hit)
}

fn bounds_intersect_eps(mins: &Vec3, maxs: &Vec3, mins2: &Vec3, maxs2: &Vec3) -> bool {
    let e = SURFACE_CLIP_EPSILON;
    !((maxs[0] as f64) < mins2[0] as f64 - e
        || (maxs[1] as f64) < mins2[1] as f64 - e
        || (maxs[2] as f64) < mins2[2] as f64 - e
        || (mins[0] as f64) > maxs2[0] as f64 + e
        || (mins[1] as f64) > maxs2[1] as f64 + e
        || (mins[2] as f64) > maxs2[2] as f64 + e)
}

fn border<'a>(pc: &'a PatchCollide, f: &Facet, j: usize) -> (&'a PatchPlane, [f32; 4]) {
    let p = &pc.planes[f.border_planes[j] as usize];
    let plane = if f.border_inward[j] { [-p.plane[0], -p.plane[1], -p.plane[2], -p.plane[3]] } else { p.plane };
    (p, plane)
}

/// `CM_TraceThroughPatchCollide` (AABB). Updates `fraction` and returns the
/// hit plane when something closer was hit.
pub fn trace_through(tw: &PatchWork, pc: &PatchCollide, fraction: &mut f32) -> Option<[f32; 4]> {
    if !bounds_intersect_eps(&tw.bounds[0], &tw.bounds[1], &pc.bounds[0], &pc.bounds[1]) {
        return None;
    }
    let mut result = None;
    if tw.is_point {
        trace_point(tw, pc, fraction, &mut result);
        return result;
    }
    for f in &pc.facets {
        let mut enter = -1.0f32;
        let mut leave = 1.0f32;
        let mut hitnum: i64 = -1;
        let mut best = [0.0f32; 4];
        let sp = &pc.planes[f.surface_plane as usize];
        let mut plane = sp.plane;
        let offset = pdot(&plane, &tw.offsets[sp.signbits as usize]);
        plane[3] -= offset;
        let (keep, hit) = check_facet_plane(&plane, tw.start, tw.end, &mut enter, &mut leave);
        if !keep {
            continue;
        }
        if hit {
            best = plane;
        }
        let mut j = 0;
        while j < f.num_borders {
            let (p, mut plane) = border(pc, f, j);
            let offset = pdot(&plane, &tw.offsets[p.signbits as usize]);
            plane[3] = (plane[3] as f64 + (offset as f64).abs()) as f32;
            let (keep, hit) = check_facet_plane(&plane, tw.start, tw.end, &mut enter, &mut leave);
            if !keep {
                break;
            }
            if hit {
                hitnum = j as i64;
                best = plane;
            }
            j += 1;
        }
        if j < f.num_borders {
            continue;
        }
        if hitnum == f.num_borders as i64 - 1 {
            continue;
        }
        if enter < leave && enter >= 0.0 && enter < *fraction {
            *fraction = enter;
            result = Some(best);
        }
    }
    result
}

/// `CM_PositionTestInPatchCollide` (AABB).
pub fn position_test(tw: &PatchWork, pc: &PatchCollide) -> bool {
    if tw.is_point {
        return false;
    }
    for f in &pc.facets {
        let sp = &pc.planes[f.surface_plane as usize];
        let mut plane = sp.plane;
        let offset = pdot(&plane, &tw.offsets[sp.signbits as usize]);
        plane[3] -= offset;
        if pdot(&plane, tw.start) - plane[3] > 0.0 {
            continue;
        }
        let mut j = 0;
        while j < f.num_borders {
            let (p, mut plane) = border(pc, f, j);
            let offset = pdot(&plane, &tw.offsets[p.signbits as usize]);
            plane[3] = (plane[3] as f64 + (offset as f64).abs()) as f32;
            if pdot(&plane, tw.start) - plane[3] > 0.0 {
                break;
            }
            j += 1;
        }
        if j < f.num_borders {
            continue;
        }
        return true;
    }
    false
}
