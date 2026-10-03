//! Kernel `marks` — the renderer half of wall marks (renderergl1/tr_marks.c):
//! `R_MarkFragments` clips a mark polygon, projected along a direction,
//! against the world's draw surfaces found by `R_BoxSurfaces_r` — planar
//! faces (`SF_FACE`), patch grids (`SF_GRID`) and triangle soups
//! (`SF_TRIANGLES`, only with `r_marksOnTriangleMeshes`, default 0). The
//! fragments are the polygons cgame turns into mark polys (cg_marks.c
//! `CG_ImpactMark`), so marks are ordinary meshes in any renderer.
//!
//! The draw surfaces come from the BSP the way tr_bsp.c loads them
//! (`ParseFace` takes the face plane from `lightmapVecs[2]`, `ParseMesh`
//! skips nodraw patches). Patch grids are tr_curve.c's
//! `R_SubdividePatchToGrid` (r_subdivisions 4: columns and rows are
//! inserted until every chord is within 4 units of the curve, colinear
//! ones are culled, `PutPointsOnCurve`, `MakeMeshNormals`), which
//! trem_godot's bspview draws too, so a mark lies on the drawn curve (dec030).

use super::bsp::{Bsp, MST_PATCH, MST_PLANAR, MST_TRIANGLE_SOUP};
use super::qmath::*;
use crate::gen::enums::{contents, surface};

/// `MAX_VERTS_ON_POLY` (tr_marks.c).
pub const MAX_VERTS_ON_POLY: usize = 64;
/// `MARKER_OFFSET` (tr_marks.c: 0).
const MARKER_OFFSET: f32 = 0.0;
/// tr_local.h MAX_GRID_SIZE: max dimensions of a grid mesh in memory.
pub const MAX_GRID_SIZE: usize = 65;
/// tr_init.c `r_subdivisions` default.
pub const R_SUBDIVISIONS: f32 = 4.0;
/// `R_BoxSurfaces_r` list size in `R_MarkFragments`.
const MAX_BOX_SURFACES: usize = 64;

const SIDE_FRONT: usize = 0;
const SIDE_BACK: usize = 1;
const SIDE_ON: usize = 2;

/// `markFragment_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkFragment {
    pub first_point: usize,
    pub num_points: usize,
}

/// `cplane_t` as the renderer keeps it (type + signbits for BoxOnPlaneSide).
#[derive(Clone, Copy, Debug, Default)]
pub struct RPlane {
    pub normal: Vec3,
    pub dist: f32,
    pub ty: u8,
    pub signbits: u8,
}

impl RPlane {
    /// `PlaneTypeForNormal` + `SetPlaneSignbits`.
    pub fn new(normal: Vec3, dist: f32) -> Self {
        let ty = if normal[0] == 1.0 {
            0
        } else if normal[1] == 1.0 {
            1
        } else if normal[2] == 1.0 {
            2
        } else {
            3
        };
        let mut signbits = 0u8;
        for j in 0..3 {
            if normal[j] < 0.0 {
                signbits |= 1 << j;
            }
        }
        RPlane { normal, dist, ty, signbits }
    }
}

/// `BoxOnPlaneSide` (q_math.c): 1 front, 2 back, 3 both.
pub fn box_on_plane_side(emins: &Vec3, emaxs: &Vec3, p: &RPlane) -> i32 {
    // fast axial cases
    if p.ty < 3 {
        if p.dist <= emins[p.ty as usize] {
            return 1;
        }
        if p.dist >= emaxs[p.ty as usize] {
            return 2;
        }
        return 3;
    }
    // general case
    let mut dist = [0.0f32; 2];
    if p.signbits < 8 {
        for i in 0..3 {
            let b = ((p.signbits >> i) & 1) as usize;
            dist[b] += p.normal[i] * emaxs[i];
            dist[1 - b] += p.normal[i] * emins[i];
        }
    }
    let mut sides = 0;
    if dist[0] >= p.dist {
        sides = 1;
    }
    if dist[1] < p.dist {
        sides |= 2;
    }
    sides
}

/// `Q_rsqrt` (q_math.c): the fast inverse square root with one Newton step.
pub fn q_rsqrt(number: f32) -> f32 {
    let x2 = number * 0.5;
    let i = 0x5f37_59df_u32.wrapping_sub(number.to_bits() >> 1);
    let y = f32::from_bits(i);
    y * (1.5 - x2 * y * y)
}

/// `VectorNormalizeFast`.
pub fn normalize_fast(v: &mut Vec3) {
    let il = q_rsqrt(dot(v, v));
    v[0] *= il;
    v[1] *= il;
    v[2] *= il;
}

/// A world draw surface as the mark code sees it (`msurface_t` data).
#[derive(Clone, Debug)]
pub enum DrawSurf {
    /// `srfSurfaceFace_t`: points, triangle indexes, plane.
    Face { points: Vec<Vec3>, indexes: Vec<i32>, plane: RPlane },
    /// `srfGridMesh_t`s of one patch: (width, height, row-major verts).
    Grid { grids: Vec<(usize, usize, Vec<Vec3>)> },
    /// `srfTriangles_t`.
    Triangles { verts: Vec<Vec3>, indexes: Vec<i32> },
    /// `SF_SKIP` and every other kind (flares, nodraw patches).
    Skip,
}

/// `mnode_t` (contents -1) / leaf with its mark surfaces.
#[derive(Clone, Debug)]
enum RNode {
    Node { plane: RPlane, children: [usize; 2] },
    Leaf { first: usize, num: usize },
}

/// The renderer's world (`tr.world`) reduced to what `R_MarkFragments` reads.
#[derive(Clone, Debug)]
pub struct MarkWorld {
    nodes: Vec<RNode>,
    /// `marksurfaces`: indexes into `surfs`.
    mark_surfaces: Vec<usize>,
    pub surfs: Vec<DrawSurf>,
    /// shader surfaceFlags / contentFlags of each surface.
    flags: Vec<(i32, i32)>,
    /// `r_marksOnTriangleMeshes` (default 0).
    pub marks_on_triangle_meshes: bool,
}

/// A `drawVert_t` for `subdivide_patch_to_grid`: its position, the
/// midpoint of two (`LerpDrawVert`) and the normal `MakeMeshNormals` sets.
pub trait GridVert: Copy + Default {
    fn xyz(&self) -> Vec3;
    fn lerp_half(a: &Self, b: &Self) -> Self;
    fn set_normal(&mut self, n: Vec3);
}

/// The mark code's vertex: position and normal.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridPoint {
    pub xyz: Vec3,
    pub normal: Vec3,
}

impl GridVert for GridPoint {
    fn xyz(&self) -> Vec3 {
        self.xyz
    }
    fn lerp_half(a: &Self, b: &Self) -> Self {
        let h = |x: f32, y: f32| 0.5 * (x + y);
        GridPoint { xyz: [h(a.xyz[0], b.xyz[0]), h(a.xyz[1], b.xyz[1]), h(a.xyz[2], b.xyz[2])], normal: a.normal }
    }
    fn set_normal(&mut self, n: Vec3) {
        self.normal = n;
    }
}

type Ctrl<T> = Vec<[T; MAX_GRID_SIZE]>;

/// tr_curve.c `Transpose`.
fn transpose<T: GridVert>(width: usize, height: usize, ctrl: &mut Ctrl<T>) {
    if width > height {
        for i in 0..height {
            for j in i + 1..width {
                if j < height {
                    // swap the value
                    let temp = ctrl[j][i];
                    ctrl[j][i] = ctrl[i][j];
                    ctrl[i][j] = temp;
                } else {
                    // just copy
                    ctrl[j][i] = ctrl[i][j];
                }
            }
        }
    } else {
        for i in 0..width {
            for j in i + 1..height {
                if j < width {
                    let temp = ctrl[i][j];
                    ctrl[i][j] = ctrl[j][i];
                    ctrl[j][i] = temp;
                } else {
                    ctrl[i][j] = ctrl[j][i];
                }
            }
        }
    }
}

/// tr_curve.c `MakeMeshNormals`: handles all the complicated wrapping and
/// degenerate cases.
fn make_mesh_normals<T: GridVert>(width: usize, height: usize, ctrl: &mut Ctrl<T>) {
    const NEIGHBORS: [[i32; 2]; 8] = [[0, 1], [1, 1], [1, 0], [1, -1], [0, -1], [-1, -1], [-1, 0], [-1, 1]];
    let (w, h) = (width as i32, height as i32);
    let wrap_width = (0..height).all(|i| length_squared(&sub(&ctrl[i][0].xyz(), &ctrl[i][width - 1].xyz())) <= 1.0);
    let wrap_height = (0..width).all(|i| length_squared(&sub(&ctrl[0][i].xyz(), &ctrl[height - 1][i].xyz())) <= 1.0);
    for i in 0..w {
        for j in 0..h {
            let base = ctrl[j as usize][i as usize].xyz();
            let mut around = [[0.0f32; 3]; 8];
            let mut good = [false; 8];
            for k in 0..8 {
                for dist in 1..=3 {
                    let mut x = i + NEIGHBORS[k][0] * dist;
                    let mut y = j + NEIGHBORS[k][1] * dist;
                    if wrap_width {
                        if x < 0 {
                            x = w - 1 + x;
                        } else if x >= w {
                            x = 1 + x - w;
                        }
                    }
                    if wrap_height {
                        if y < 0 {
                            y = h - 1 + y;
                        } else if y >= h {
                            y = 1 + y - h;
                        }
                    }
                    if x < 0 || x >= w || y < 0 || y >= h {
                        break; // edge of patch
                    }
                    let mut temp = [0.0; 3];
                    if normalize2(&sub(&ctrl[y as usize][x as usize].xyz(), &base), &mut temp) == 0.0 {
                        continue; // degenerate edge, get more dist
                    }
                    good[k] = true;
                    around[k] = temp;
                    break; // good edge
                }
            }
            let mut sum = [0.0f32; 3];
            for k in 0..8 {
                if !good[k] || !good[(k + 1) & 7] {
                    continue; // didn't get two points
                }
                let mut normal = [0.0; 3];
                if normalize2(&cross(&around[(k + 1) & 7], &around[k]), &mut normal) == 0.0 {
                    continue;
                }
                sum = add(&normal, &sum);
            }
            let mut n = [0.0; 3];
            normalize2(&sum, &mut n);
            ctrl[j as usize][i as usize].set_normal(n);
        }
    }
}

/// tr_curve.c `InvertCtrl`.
fn invert_ctrl<T: GridVert>(width: usize, height: usize, ctrl: &mut Ctrl<T>) {
    for row in ctrl.iter_mut().take(height) {
        for j in 0..width / 2 {
            row.swap(j, width - 1 - j);
        }
    }
}

/// tr_curve.c `PutPointsOnCurve`.
fn put_points_on_curve<T: GridVert>(ctrl: &mut Ctrl<T>, width: usize, height: usize) {
    for i in 0..width {
        let mut j = 1;
        while j < height {
            let prev = T::lerp_half(&ctrl[j][i], &ctrl[j + 1][i]);
            let next = T::lerp_half(&ctrl[j][i], &ctrl[j - 1][i]);
            ctrl[j][i] = T::lerp_half(&prev, &next);
            j += 2;
        }
    }
    for row in ctrl.iter_mut().take(height) {
        let mut i = 1;
        while i < width {
            let prev = T::lerp_half(&row[i], &row[i + 1]);
            let next = T::lerp_half(&row[i], &row[i - 1]);
            row[i] = T::lerp_half(&prev, &next);
            i += 2;
        }
    }
}

/// tr_curve.c `R_SubdividePatchToGrid` (r_subdivisions `subdivisions`):
/// the `srfGridMesh_t` of a patch as (width, height, row-major verts).
pub fn subdivide_patch_to_grid<T: GridVert>(width: usize, height: usize, points: &[T], subdivisions: f32) -> (usize, usize, Vec<T>) {
    if width < 3 || height < 3 || points.len() < width * height || width > MAX_GRID_SIZE || height > MAX_GRID_SIZE {
        return (0, 0, Vec::new());
    }
    let (mut width, mut height) = (width, height);
    let mut ctrl: Ctrl<T> = vec![[T::default(); MAX_GRID_SIZE]; MAX_GRID_SIZE];
    let mut error_table = [[0.0f32; MAX_GRID_SIZE]; 2];
    for i in 0..width {
        for j in 0..height {
            ctrl[j][i] = points[j * width + i];
        }
    }
    for dir in 0..2 {
        error_table[dir] = [0.0; MAX_GRID_SIZE];
        // horizontal subdivisions
        let mut j: i64 = 0;
        while (j + 2) < width as i64 {
            let ju = j as usize;
            // check subdivided midpoints against control points
            let mut max_len = 0.0f32;
            for row in ctrl.iter().take(height) {
                // calculate the point on the curve
                let mut midxyz = [0.0f32; 3];
                for l in 0..3 {
                    midxyz[l] = (row[ju].xyz()[l] + row[ju + 1].xyz()[l] * 2.0 + row[ju + 2].xyz()[l]) * 0.25;
                }
                // see how far off the line it is
                let midxyz = sub(&midxyz, &row[ju].xyz());
                let mut d = sub(&row[ju + 2].xyz(), &row[ju].xyz());
                normalize(&mut d);
                let dd = dot(&midxyz, &d);
                let projected = scale(&d, dd);
                let len = length_squared(&sub(&midxyz, &projected)); // we will do the sqrt later
                if len > max_len {
                    max_len = len;
                }
            }
            let max_len = (max_len as f64).sqrt() as f32;
            // if all the points are on the lines, remove the entire columns
            if max_len < 0.1 {
                error_table[dir][ju + 1] = 999.0;
                j += 2;
                continue;
            }
            // see if we want to insert subdivided columns
            if width + 2 > MAX_GRID_SIZE {
                error_table[dir][ju + 1] = 1.0 / max_len;
                j += 2;
                continue; // can't subdivide any more
            }
            if max_len <= subdivisions {
                error_table[dir][ju + 1] = 1.0 / max_len;
                j += 2;
                continue; // didn't need subdivision
            }
            error_table[dir][ju + 2] = 1.0 / max_len;
            // insert two columns and replace the peak
            width += 2;
            for row in ctrl.iter_mut().take(height) {
                let prev = T::lerp_half(&row[ju], &row[ju + 1]);
                let next = T::lerp_half(&row[ju + 1], &row[ju + 2]);
                let mid = T::lerp_half(&prev, &next);
                let mut k = width - 1;
                while k > ju + 3 {
                    row[k] = row[k - 2];
                    k -= 1;
                }
                row[ju + 1] = prev;
                row[ju + 2] = mid;
                row[ju + 3] = next;
            }
            // back up and recheck this set again, it may need more subdivision
            // (j -= 2, then the loop's j += 2)
        }
        transpose(width, height, &mut ctrl);
        std::mem::swap(&mut width, &mut height);
    }
    // put all the aproximating points on the curve
    put_points_on_curve(&mut ctrl, width, height);
    // cull out any rows or columns that are colinear
    let mut i = 1;
    while i + 1 < width {
        if error_table[0][i] != 999.0 {
            i += 1;
            continue;
        }
        for j in i + 1..width {
            for row in ctrl.iter_mut().take(height) {
                row[j - 1] = row[j];
            }
            error_table[0][j - 1] = error_table[0][j];
        }
        width -= 1;
        i += 1;
    }
    let mut i = 1;
    while i + 1 < height {
        if error_table[1][i] != 999.0 {
            i += 1;
            continue;
        }
        for j in i + 1..height {
            for k in 0..width {
                ctrl[j - 1][k] = ctrl[j][k];
            }
            error_table[1][j - 1] = error_table[1][j];
        }
        height -= 1;
        i += 1;
    }
    // flip for longest tristrips as an optimization (visually identical)
    if height > width {
        transpose(width, height, &mut ctrl);
        std::mem::swap(&mut width, &mut height);
        invert_ctrl(width, height, &mut ctrl);
    }
    // calculate normals
    make_mesh_normals(width, height, &mut ctrl);
    // R_CreateSurfaceGridMesh: copy the results out to a grid
    let mut verts = Vec::with_capacity(width * height);
    for row in ctrl.iter().take(height) {
        verts.extend_from_slice(&row[..width]);
    }
    (width, height, verts)
}

/// The grids of a patch surface for the mark code (one `srfGridMesh_t`).
pub fn patch_grids(ctrl: &[Vec3], w: usize, h: usize) -> Vec<(usize, usize, Vec<Vec3>)> {
    let pts: Vec<GridPoint> = ctrl.iter().map(|&xyz| GridPoint { xyz, normal: [0.0; 3] }).collect();
    let (gw, gh, verts) = subdivide_patch_to_grid(w, h, &pts, R_SUBDIVISIONS);
    if gw == 0 {
        return Vec::new();
    }
    vec![(gw, gh, verts.iter().map(|v| v.xyz).collect())]
}

impl MarkWorld {
    /// `R_LoadSurfaces` / `R_LoadNodesAndLeafs` / `R_LoadMarksurfaces` for
    /// the parts the mark code uses.
    pub fn from_bsp(bsp: &Bsp) -> Self {
        let mut surfs = Vec::with_capacity(bsp.surfaces.len());
        let mut flags = Vec::with_capacity(bsp.surfaces.len());
        for s in &bsp.surfaces {
            let (sf, cf) = bsp.shaders.get(s.shader.max(0) as usize).map(|x| (x.surface_flags, x.content_flags)).unwrap_or((0, 0));
            flags.push((sf, cf));
            let fv = s.first_vert.max(0) as usize;
            let nv = s.num_verts.max(0) as usize;
            let verts: Vec<Vec3> = bsp.verts.get(fv..fv + nv).map(|v| v.iter().map(|d| d.xyz).collect()).unwrap_or_default();
            let fi = s.first_index.max(0) as usize;
            let ni = s.num_indexes.max(0) as usize;
            let indexes: Vec<i32> = bsp.indexes.get(fi..fi + ni).map(|x| x.to_vec()).unwrap_or_default();
            surfs.push(match s.surface_type {
                // ParseFace: the plane information from the lightmap vector
                MST_PLANAR if !verts.is_empty() => {
                    // (a sheet-authored BSP has no lightmap vectors: the vertex normal)
                    let normal = if s.lightmap_vecs[2] == [0.0; 3] { bsp.verts[fv].normal } else { s.lightmap_vecs[2] };
                    let dist = dot(&verts[0], &normal);
                    DrawSurf::Face { points: verts, indexes, plane: RPlane::new(normal, dist) }
                }
                // ParseMesh: nodraw patches are only there for clipping
                MST_PATCH if sf & surface::NODRAW as i32 != 0 => DrawSurf::Skip,
                MST_PATCH => DrawSurf::Grid { grids: patch_grids(&verts, s.patch_width.max(0) as usize, s.patch_height.max(0) as usize) },
                MST_TRIANGLE_SOUP => DrawSurf::Triangles { verts, indexes },
                _ => DrawSurf::Skip,
            });
        }
        let num_nodes = bsp.nodes.len();
        let mut nodes = Vec::with_capacity(num_nodes + bsp.leafs.len());
        let child = |c: i32| if c >= 0 { c as usize } else { num_nodes + (-1 - c) as usize };
        for n in &bsp.nodes {
            let p = bsp.planes.get(n.plane.max(0) as usize).copied().unwrap_or([0.0, 0.0, 1.0, 0.0]);
            nodes.push(RNode::Node { plane: RPlane::new([p[0], p[1], p[2]], p[3]), children: [child(n.children[0]), child(n.children[1])] });
        }
        for l in &bsp.leafs {
            nodes.push(RNode::Leaf { first: l.first_leaf_surface.max(0) as usize, num: l.num_leaf_surfaces.max(0) as usize });
        }
        let mut mark_surfaces: Vec<usize> = bsp.leaf_surfaces.iter().map(|&s| s.max(0) as usize).collect();
        if mark_surfaces.is_empty() {
            // a sheet-authored BSP (bsp::from_sheet) lists no leaf surfaces:
            // one leaf holding the world model's surfaces
            let n = bsp.models.first().map(|m| m.num_surfaces.max(0) as usize).unwrap_or(surfs.len()).min(surfs.len());
            nodes = vec![RNode::Leaf { first: 0, num: n }];
            mark_surfaces = (0..n).collect();
        }
        MarkWorld { nodes, mark_surfaces, surfs, flags, marks_on_triangle_meshes: false }
    }

    /// A world of one leaf holding `surfs` (no flags): tests and tools.
    pub fn from_surfaces(surfs: Vec<DrawSurf>) -> Self {
        let n = surfs.len();
        MarkWorld { nodes: vec![RNode::Leaf { first: 0, num: n }], mark_surfaces: (0..n).collect(), flags: vec![(0, 0); n], surfs, marks_on_triangle_meshes: false }
    }

    /// `R_BoxSurfaces_r`: the draw surfaces in the leaves the box touches
    /// that a mark projected along `dir` can land on.
    fn box_surfaces_r(&self, mut node: usize, mins: &Vec3, maxs: &Vec3, list: &mut Vec<usize>, listsize: usize, seen: &mut [bool], dir: &Vec3) {
        // do the tail recursion in a loop
        let (first, num) = loop {
            match &self.nodes[node] {
                RNode::Node { plane, children } => {
                    let s = box_on_plane_side(mins, maxs, plane);
                    if s == 1 {
                        node = children[0];
                    } else if s == 2 {
                        node = children[1];
                    } else {
                        self.box_surfaces_r(children[0], mins, maxs, list, listsize, seen, dir);
                        node = children[1];
                    }
                }
                RNode::Leaf { first, num } => break (*first, *num),
            }
        };
        // add the individual surfaces
        for m in first..first + num {
            if list.len() >= listsize {
                break;
            }
            let Some(&s) = self.mark_surfaces.get(m) else { break };
            if s >= self.surfs.len() {
                continue;
            }
            let (sf, cf) = self.flags[s];
            // check if the surface has NOIMPACT or NOMARKS set
            if sf & (surface::NOIMPACT | surface::NOMARKS) as i32 != 0 || cf & contents::FOG as i32 != 0 {
                seen[s] = true;
            } else {
                match &self.surfs[s] {
                    // extra check for surfaces to avoid list overflows
                    DrawSurf::Face { plane, .. } => {
                        // the face plane should go through the box
                        let side = box_on_plane_side(mins, maxs, plane);
                        if side == 1 || side == 2 {
                            seen[s] = true;
                        } else if dot(&plane.normal, dir) > -0.5 {
                            // don't add faces that make sharp angles with the projection direction
                            seen[s] = true;
                        }
                    }
                    DrawSurf::Grid { .. } | DrawSurf::Triangles { .. } => {}
                    DrawSurf::Skip => seen[s] = true,
                }
            }
            // the surface may have already been added if it spans multiple leafs
            if !seen[s] {
                seen[s] = true;
                list.push(s);
            }
        }
    }

    /// `R_MarkFragments`: clip the polygon `points` projected along
    /// `projection` against the world. Returns the fragment points and the
    /// fragments (at most `max_points` / `max_fragments`).
    pub fn mark_fragments(&self, points: &[Vec3], projection: &Vec3, max_points: usize, max_fragments: usize) -> (Vec<Vec3>, Vec<MarkFragment>) {
        let mut point_buffer = Vec::new();
        let mut fragments = Vec::new();
        if points.is_empty() || max_fragments == 0 {
            return (point_buffer, fragments);
        }
        let mut projection_dir = [0.0; 3];
        normalize2(projection, &mut projection_dir);
        // find all the brushes that are to be considered
        let mut mins = [99999.0f32; 3];
        let mut maxs = [-99999.0f32; 3];
        let mut add_bounds = |p: &Vec3| {
            for j in 0..3 {
                mins[j] = mins[j].min(p[j]);
                maxs[j] = maxs[j].max(p[j]);
            }
        };
        for p in points {
            add_bounds(p);
            add_bounds(&add(p, projection));
            // make sure we get all the leafs (also the one(s) in front of the hit surface)
            add_bounds(&ma(p, -20.0, &projection_dir));
        }
        let points = &points[..points.len().min(MAX_VERTS_ON_POLY)];
        let n = points.len();
        // create the bounding planes for the to be projected polygon
        let mut normals: Vec<Vec3> = Vec::with_capacity(n + 2);
        let mut dists: Vec<f32> = Vec::with_capacity(n + 2);
        for i in 0..n {
            let v1 = sub(&points[(i + 1) % n], &points[i]);
            let v2 = add(&points[i], projection);
            let v2 = sub(&points[i], &v2);
            let mut nrm = cross(&v1, &v2);
            normalize_fast(&mut nrm);
            dists.push(dot(&nrm, &points[i]));
            normals.push(nrm);
        }
        // add near and far clipping planes for projection
        normals.push(projection_dir);
        dists.push(dot(&projection_dir, &points[0]) - 32.0);
        let inv = neg(&projection_dir);
        normals.push(inv);
        dists.push(dot(&inv, &points[0]) - 20.0);

        let mut surfaces = Vec::new();
        let mut seen = vec![false; self.surfs.len()];
        self.box_surfaces_r(0, &mins, &maxs, &mut surfaces, MAX_BOX_SURFACES, &mut seen, &projection_dir);

        let add_tri = |tri: [Vec3; 3], pb: &mut Vec<Vec3>, fr: &mut Vec<MarkFragment>| -> bool {
            add_mark_fragments(&tri, &normals, &dists, max_points, pb, fr);
            fr.len() == max_fragments
        };
        for &s in &surfaces {
            match &self.surfs[s] {
                DrawSurf::Grid { grids } => {
                    for (width, height, verts) in grids {
                        let (width, height) = (*width, *height);
                        for m in 0..height - 1 {
                            for nn in 0..width - 1 {
                                // We triangulate the grid and chop all triangles within
                                // the bounding planes of the to be projected polygon.
                                let dv = m * width + nn;
                                let tri = [verts[dv], verts[dv + width], verts[dv + 1]];
                                // check the normal of this triangle
                                let v1 = sub(&tri[0], &tri[1]);
                                let v2 = sub(&tri[2], &tri[1]);
                                let mut normal = cross(&v1, &v2);
                                normalize_fast(&mut normal);
                                if dot(&normal, &projection_dir) < -0.1 && add_tri(tri, &mut point_buffer, &mut fragments) {
                                    return (point_buffer, fragments); // not enough space for more fragments
                                }
                                let tri = [verts[dv + 1], verts[dv + width], verts[dv + width + 1]];
                                let v1 = sub(&tri[0], &tri[1]);
                                let v2 = sub(&tri[2], &tri[1]);
                                let mut normal = cross(&v1, &v2);
                                normalize_fast(&mut normal);
                                if dot(&normal, &projection_dir) < -0.05 && add_tri(tri, &mut point_buffer, &mut fragments) {
                                    return (point_buffer, fragments);
                                }
                            }
                        }
                    }
                }
                DrawSurf::Face { points: fp, indexes, plane } => {
                    // check the normal of this face
                    if dot(&plane.normal, &projection_dir) > -0.5 {
                        continue;
                    }
                    for k_ in (0..indexes.len().saturating_sub(2)).step_by(3) {
                        let mut tri = [[0.0; 3]; 3];
                        for j in 0..3 {
                            let v = fp.get(indexes[k_ + j].max(0) as usize).copied().unwrap_or_default();
                            tri[j] = ma(&v, MARKER_OFFSET, &plane.normal);
                        }
                        if add_tri(tri, &mut point_buffer, &mut fragments) {
                            return (point_buffer, fragments);
                        }
                    }
                }
                DrawSurf::Triangles { verts, indexes } if self.marks_on_triangle_meshes => {
                    for k_ in (0..indexes.len().saturating_sub(2)).step_by(3) {
                        let mut tri = [[0.0; 3]; 3];
                        for j in 0..3 {
                            tri[j] = verts.get(indexes[k_ + j].max(0) as usize).copied().unwrap_or_default();
                        }
                        if add_tri(tri, &mut point_buffer, &mut fragments) {
                            return (point_buffer, fragments);
                        }
                    }
                }
                _ => {}
            }
        }
        (point_buffer, fragments)
    }
}

/// `R_ChopPolyBehindPlane`: keep the part of `input` in front of the plane.
pub fn chop_poly_behind_plane(input: &[Vec3], normal: &Vec3, dist: f32, epsilon: f32) -> Vec<Vec3> {
    let n = input.len();
    let mut out = Vec::with_capacity(n + 2);
    // don't clip if it might overflow
    if n >= MAX_VERTS_ON_POLY - 2 {
        return out;
    }
    let mut dists = vec![0.0f32; n + 1];
    let mut sides = vec![0usize; n + 1];
    let mut counts = [0usize; 3];
    // determine sides for each point
    for i in 0..n {
        let d = dot(&input[i], normal) - dist;
        dists[i] = d;
        sides[i] = if d > epsilon {
            SIDE_FRONT
        } else if d < -epsilon {
            SIDE_BACK
        } else {
            SIDE_ON
        };
        counts[sides[i]] += 1;
    }
    sides[n] = sides[0];
    dists[n] = dists[0];
    if counts[SIDE_FRONT] == 0 {
        return out;
    }
    if counts[SIDE_BACK] == 0 {
        out.extend_from_slice(input);
        return out;
    }
    for i in 0..n {
        let p1 = input[i];
        if sides[i] == SIDE_ON {
            out.push(p1);
            continue;
        }
        if sides[i] == SIDE_FRONT {
            out.push(p1);
        }
        if sides[i + 1] == SIDE_ON || sides[i + 1] == sides[i] {
            continue;
        }
        // generate a split point
        let p2 = input[(i + 1) % n];
        let d = dists[i] - dists[i + 1];
        let t = if d == 0.0 { 0.0 } else { dists[i] / d };
        out.push([p1[0] + t * (p2[0] - p1[0]), p1[1] + t * (p2[1] - p1[1]), p1[2] + t * (p2[2] - p1[2])]);
    }
    out
}

/// `R_AddMarkFragments`: chop one surface triangle by every bounding plane
/// of the projected polygon and keep what is left as a fragment.
pub fn add_mark_fragments(tri: &[Vec3], normals: &[Vec3], dists: &[f32], max_points: usize, point_buffer: &mut Vec<Vec3>, fragments: &mut Vec<MarkFragment>) {
    let mut clip = tri.to_vec();
    for (nrm, d) in normals.iter().zip(dists) {
        clip = chop_poly_behind_plane(&clip, nrm, *d, 0.5);
        if clip.is_empty() {
            break;
        }
    }
    // completely clipped away?
    if clip.is_empty() {
        return;
    }
    // add this fragment to the returned list
    if clip.len() + point_buffer.len() > max_points {
        return; // not enough space for this polygon
    }
    fragments.push(MarkFragment { first_point: point_buffer.len(), num_points: clip.len() });
    point_buffer.extend_from_slice(&clip);
}

/// `CG_ImpactMark`'s polygon and texture axis: the four corners of a
/// `radius` square on the plane `dir` through `origin`, rotated
/// `orientation` degrees, plus axis[1] / axis[2] for the texture coordinates.
pub fn impact_mark_polygon(origin: &Vec3, dir: &Vec3, orientation: f32, radius: f32) -> ([Vec3; 4], [Vec3; 3]) {
    // create the texture axis
    let mut a0 = [0.0; 3];
    normalize2(dir, &mut a0);
    let a1 = perpendicular_vector(&a0);
    let a2 = rotate_point_around_vector(&a0, &a1, orientation);
    let a1 = cross(&a0, &a2);
    let axis = [a0, a1, a2];
    let mut pts = [[0.0f32; 3]; 4];
    for i in 0..3 {
        pts[0][i] = origin[i] - radius * axis[1][i] - radius * axis[2][i];
        pts[1][i] = origin[i] + radius * axis[1][i] - radius * axis[2][i];
        pts[2][i] = origin[i] + radius * axis[1][i] + radius * axis[2][i];
        pts[3][i] = origin[i] - radius * axis[1][i] + radius * axis[2][i];
    }
    (pts, axis)
}
