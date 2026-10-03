//! Kernel `cm` — port of qcommon/cm_trace.c + cm_test.c (brush/AABB path).
//!
//! The clip map has the same shape as the original `clipMap_t` (planes, brush
//! sides, brushes, nodes, leafs, leaf-brush indices), so it can be filled either
//! from the sheet-authored brush tables or from an IBSP v46 file (kernel `bsp`).
//! Curved patches come from kernel `patch` (cm_patch.c); inline brush models
//! (doors, platforms) are `models[1..]` with their own leaf, traced in their
//! frame by `transformed_box_trace`. Capsule/bisphere traces are not ported.

use super::qmath::{dot, Vec3, VEC3_ORIGIN};
use std::cell::Cell;

pub const SURFACE_CLIP_EPSILON: f64 = 0.125;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
    /// 0,1,2 axial; >= 3 non-axial (the box hull uses 3+axis for negative axes).
    pub ty: u8,
    pub signbits: u8,
}

impl Plane {
    pub fn new(normal: Vec3, dist: f32, ty: u8) -> Self {
        let mut p = Plane { normal, dist, ty, signbits: 0 };
        p.set_signbits();
        p
    }
    /// `SetPlaneSignbits`.
    pub fn set_signbits(&mut self) {
        let mut bits = 0u8;
        for j in 0..3 {
            if self.normal[j] < 0.0 {
                bits |= 1 << j;
            }
        }
        self.signbits = bits;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BrushSide {
    pub plane: u32,
    pub surface_flags: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub contents: i32,
    pub bounds: [Vec3; 2],
    pub first_side: u32,
    pub num_sides: u32,
    /// Index of the authoring row / BSP shader (for rendering).
    pub tag: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub plane: u32,
    pub children: [i32; 2],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Leaf {
    pub cluster: i32,
    pub area: i32,
    pub first_leaf_brush: u32,
    pub num_leaf_brushes: u32,
    pub first_leaf_surface: u32,
    pub num_leaf_surfaces: u32,
}

/// `cPatch_t`.
#[derive(Clone, Debug)]
pub struct CPatch {
    pub contents: i32,
    pub surface_flags: i32,
    pub pc: super::patch::PatchCollide,
}

/// `cmodel_t`: bounds spread by one unit; models 1.. carry their own leaf.
#[derive(Clone, Copy, Debug, Default)]
pub struct CModel {
    pub mins: Vec3,
    pub maxs: Vec3,
    pub leaf: Leaf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trace {
    pub allsolid: bool,
    pub startsolid: bool,
    pub fraction: f32,
    pub endpos: Vec3,
    pub plane: Plane,
    pub surface_flags: i32,
    pub contents: i32,
    pub entity_num: i32,
    pub lateral_fraction: f32,
}

#[derive(Default)]
pub struct ClipMap {
    pub planes: Vec<Plane>,
    pub sides: Vec<BrushSide>,
    pub brushes: Vec<Brush>,
    pub nodes: Vec<Node>,
    pub leafs: Vec<Leaf>,
    pub leaf_brushes: Vec<u32>,
    pub leaf_surfaces: Vec<u32>,
    /// Indexed by BSP surface; None for non-patch surfaces.
    pub patches: Vec<Option<CPatch>>,
    pub models: Vec<CModel>,
    /// PVS (CMod_LoadVisibility): `vised` false means everything is visible.
    /// `CM_NumClusters` is `num_clusters`; `CM_LeafCluster` / `CM_LeafArea`
    /// are `leafs[n].cluster` / `leafs[n].area`.
    pub num_clusters: i32,
    pub cluster_bytes: i32,
    pub visibility: Vec<u8>,
    pub vised: bool,
    /// areas and their portal reference counts (CM_AdjustAreaPortalState).
    pub num_areas: i32,
    pub area_flood: Vec<(i32, i32)>,
    pub area_portals: Vec<i32>,
    pub flood_valid: i32,
    checkcount: Cell<u32>,
    brush_check: Vec<Cell<u32>>,
    patch_check: Vec<Cell<u32>>,
    /// `cbrush_t.collided`: set when the current trace crossed a brush plane.
    brush_collided: Vec<Cell<bool>>,
    /// `cbrush_t.edges` (CMod_CreateBrushSideWindings), built lazily on the
    /// first trace that tests lateral collision.
    brush_edges: std::cell::OnceCell<Vec<Vec<[Vec3; 2]>>>,
}

/// `traceWork_t` (AABB members only).
struct TraceWork {
    start: Vec3,
    end: Vec3,
    size: [Vec3; 2],
    offsets: [Vec3; 8],
    extents: Vec3,
    bounds: [Vec3; 2],
    contents: i32,
    is_point: bool,
    /// TT_BISPHERE start/end radii (None: TT_AABB).
    bisphere: Option<(f32, f32)>,
    test_lateral: bool,
    trace: Trace,
}

impl ClipMap {
    /// Finish construction: allocates per-brush check counters.
    pub fn finalize(&mut self) {
        self.brush_check = (0..self.brushes.len()).map(|_| Cell::new(0)).collect();
        self.patch_check = (0..self.patches.len()).map(|_| Cell::new(0)).collect();
        self.brush_collided = (0..self.brushes.len()).map(|_| Cell::new(false)).collect();
        self.brush_edges = std::cell::OnceCell::new();
    }

    /// `CMod_CreateBrushSideWindings`: every brush side's winding, chopped by
    /// the other sides, composed into a de-duplicated edge list.
    fn create_brush_edges(&self) -> Vec<Vec<[Vec3; 2]>> {
        use super::patch::{base_winding_for_plane, chop_winding_in_place};
        const CM_EDGE_VERTEX_EPSILON: f32 = 0.1;
        let same = |a: &Vec3, b: &Vec3| (0..3).all(|i| (a[i] - b[i]).abs() <= CM_EDGE_VERTEX_EPSILON);
        self.brushes
            .iter()
            .map(|b| {
                let mut windings = Vec::new();
                for j in 0..b.num_sides {
                    let p = *self.side_plane(b, j);
                    let mut w = Some(base_winding_for_plane(&p.normal, p.dist));
                    for kk in 0..b.num_sides {
                        if kk == j || w.is_none() {
                            continue;
                        }
                        let c = *self.side_plane(b, kk);
                        // back side clipaway (planeNum ^ 1)
                        if c.normal == [-p.normal[0], -p.normal[1], -p.normal[2]] && c.dist == -p.dist {
                            continue;
                        }
                        let n = [-c.normal[0], -c.normal[1], -c.normal[2]];
                        w = chop_winding_in_place(w, &n, -c.dist, 0.0).unwrap_or(None);
                    }
                    windings.push(w);
                }
                let mut edges: Vec<[Vec3; 2]> = Vec::new();
                for w in windings.iter().flatten() {
                    for k in 0..w.len().saturating_sub(1) {
                        let (p0, p1) = (w[k], w[k + 1]);
                        // `CMod_AddEdgeToBrush` / `CMod_BrushEdgesAreTheSame`
                        if edges.iter().any(|e| (same(&p0, &e[0]) && same(&p1, &e[1])) || (same(&p1, &e[0]) && same(&p0, &e[1]))) {
                            continue;
                        }
                        edges.push([p0, p1]);
                    }
                }
                edges
            })
            .collect()
    }

    /// `CM_ProximityToBrush`.
    fn proximity_to_brush(&self, tw: &mut TraceWork, bn: usize) {
        let b = &self.brushes[bn];
        // cheapish purely linear trace to test for intersection
        let mut tw2 = Self::work(&tw.start, &tw.end, &VEC3_ORIGIN, &VEC3_ORIGIN, tw.contents);
        tw2.is_point = true;
        self.trace_through_brush(&mut tw2, b, bn);
        if tw2.trace.fraction == 1.0 && !tw2.trace.allsolid && !tw2.trace.startsolid {
            let edges = &self.brush_edges.get_or_init(|| self.create_brush_edges())[bn];
            let mut min_dist = 1e10f32;
            let mut s_at_min = 0.0f32;
            for e in edges {
                let (d, s, _) = distance_between_line_segments_squared(&tw.start, &tw.end, &e[0], &e[1]);
                if d < min_dist {
                    min_dist = d;
                    s_at_min = s;
                }
            }
            let radius = match tw.bisphere {
                Some((sr, er)) => sr + s_at_min * (er - sr),
                None => 0.0, // TT_AABB: FIXME in the C
            };
            let fraction = min_dist / (radius * radius);
            if fraction < tw.trace.lateral_fraction {
                tw.trace.lateral_fraction = fraction;
            }
        } else {
            tw.trace.lateral_fraction = 0.0;
        }
    }

    /// `CM_BiSphereTrace` against the world (model 0) or an inline model.
    pub fn bisphere_trace(&self, start: &Vec3, end: &Vec3, start_rad: f32, end_rad: f32, model: usize, mask: i32) -> Trace {
        let largest = start_rad.max(end_rad);
        self.checkcount.set(self.checkcount.get().wrapping_add(1));
        let mut tw = Self::work(start, end, &VEC3_ORIGIN, &VEC3_ORIGIN, mask);
        tw.bisphere = Some((start_rad, end_rad));
        tw.test_lateral = true;
        tw.trace.lateral_fraction = 1.0;
        if self.nodes.is_empty() {
            return tw.trace;
        }
        for i in 0..3 {
            if tw.start[i] < tw.end[i] {
                tw.bounds[0][i] = tw.start[i] - start_rad;
                tw.bounds[1][i] = tw.end[i] + end_rad;
            } else {
                tw.bounds[0][i] = tw.end[i] + end_rad;
                tw.bounds[1][i] = tw.start[i] - start_rad;
            }
        }
        tw.is_point = false;
        tw.extents = [largest; 3];
        if model > 0 {
            if let Some(m) = self.models.get(model) {
                let l = m.leaf;
                self.trace_through_leaf(&mut tw, &l);
            }
        } else {
            let (s, e) = (tw.start, tw.end);
            self.trace_through_tree(&mut tw, 0, 0.0, 1.0, s, e);
        }
        Self::finish(tw, start, end)
    }

    fn patch_work<'a>(tw: &'a TraceWork) -> super::patch::PatchWork<'a> {
        super::patch::PatchWork { start: &tw.start, end: &tw.end, offsets: &tw.offsets, bounds: &tw.bounds, is_point: tw.is_point }
    }

    /// The patches of a leaf not yet checked by this trace, filtered by contents.
    fn leaf_patches<'a>(&'a self, leaf: &Leaf, contents: i32) -> impl Iterator<Item = &'a CPatch> + 'a {
        let cc = self.checkcount.get();
        let (first, n) = (leaf.first_leaf_surface as usize, leaf.num_leaf_surfaces as usize);
        (first..first + n).filter_map(move |k| {
            let sn = self.leaf_surfaces[k] as usize;
            let p = self.patches.get(sn)?.as_ref()?;
            if self.patch_check[sn].get() == cc {
                return None;
            }
            self.patch_check[sn].set(cc);
            if p.contents & contents == 0 {
                return None;
            }
            Some(p)
        })
    }

    /// Append an axis-aligned brush (optionally clipped by one extra plane) with
    /// sides in CM_InitBoxHull order: +x, -x, +y, -y, +z, -z, [cut].
    pub fn add_box_brush(&mut self, mins: Vec3, maxs: Vec3, cut: Option<[f32; 4]>, contents: i32, surface_flags: i32, tag: u32) -> u32 {
        let first_side = self.sides.len() as u32;
        for axis in 0..3 {
            let mut n = VEC3_ORIGIN;
            n[axis] = 1.0;
            self.planes.push(Plane::new(n, maxs[axis], axis as u8));
            self.sides.push(BrushSide { plane: self.planes.len() as u32 - 1, surface_flags });
            let mut n = VEC3_ORIGIN;
            n[axis] = -1.0;
            self.planes.push(Plane::new(n, -mins[axis], 3 + axis as u8));
            self.sides.push(BrushSide { plane: self.planes.len() as u32 - 1, surface_flags });
        }
        if let Some(c) = cut {
            self.planes.push(Plane::new([c[0], c[1], c[2]], c[3], 3));
            self.sides.push(BrushSide { plane: self.planes.len() as u32 - 1, surface_flags });
        }
        self.brushes.push(Brush { contents, bounds: [mins, maxs], first_side, num_sides: self.sides.len() as u32 - first_side, tag });
        self.brushes.len() as u32 - 1
    }

    /// One node whose both children are leaf 0, which lists every brush. This is
    /// a valid (if unbalanced) clip tree: traversal semantics are unchanged.
    pub fn build_single_leaf_tree(&mut self) {
        self.planes.push(Plane::new([1.0, 0.0, 0.0], -65536.0, 0));
        self.nodes = vec![Node { plane: self.planes.len() as u32 - 1, children: [-1, -1] }];
        self.leaf_brushes = (0..self.brushes.len() as u32).collect();
        self.leafs = vec![Leaf { cluster: 0, area: 0, first_leaf_brush: 0, num_leaf_brushes: self.brushes.len() as u32, ..Default::default() }];
        self.finalize();
    }

    fn side_plane(&self, b: &Brush, i: u32) -> &Plane {
        &self.planes[self.sides[(b.first_side + i) as usize].plane as usize]
    }

    // ------------------------------------------------------------ position test
    fn test_box_in_brush(&self, tw: &mut TraceWork, b: &Brush) {
        if b.num_sides == 0 {
            return;
        }
        if tw.bounds[0][0] > b.bounds[1][0]
            || tw.bounds[0][1] > b.bounds[1][1]
            || tw.bounds[0][2] > b.bounds[1][2]
            || tw.bounds[1][0] < b.bounds[0][0]
            || tw.bounds[1][1] < b.bounds[0][1]
            || tw.bounds[1][2] < b.bounds[0][2]
        {
            return;
        }
        // the first six planes are the axial planes
        for i in 6..b.num_sides {
            let p = self.side_plane(b, i);
            let dist = p.dist - dot(&tw.offsets[p.signbits as usize], &p.normal);
            let d1 = dot(&tw.start, &p.normal) - dist;
            if d1 > 0.0 {
                return;
            }
        }
        tw.trace.startsolid = true;
        tw.trace.allsolid = true;
        tw.trace.fraction = 0.0;
        tw.trace.contents = b.contents;
    }

    fn test_in_leaf(&self, tw: &mut TraceWork, leaf: &Leaf) {
        let cc = self.checkcount.get();
        for k in 0..leaf.num_leaf_brushes {
            let bn = self.leaf_brushes[(leaf.first_leaf_brush + k) as usize] as usize;
            if self.brush_check[bn].get() == cc {
                continue;
            }
            self.brush_check[bn].set(cc);
            let b = &self.brushes[bn];
            if b.contents & tw.contents == 0 {
                continue;
            }
            self.test_box_in_brush(tw, b);
            if tw.trace.allsolid {
                return;
            }
        }
        for p in self.leaf_patches(leaf, tw.contents) {
            if super::patch::position_test(&Self::patch_work(tw), &p.pc) {
                tw.trace.startsolid = true;
                tw.trace.allsolid = true;
                tw.trace.fraction = 0.0;
                tw.trace.contents = p.contents;
                return;
            }
        }
    }

    fn box_on_plane_side(emins: &Vec3, emaxs: &Vec3, p: &Plane) -> i32 {
        if p.ty < 3 {
            if p.dist <= emins[p.ty as usize] {
                return 1;
            }
            if p.dist >= emaxs[p.ty as usize] {
                return 2;
            }
            return 3;
        }
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

    fn box_leafnums_r(&self, bounds: &[Vec3; 2], mut nodenum: i32, out: &mut Vec<i32>) {
        loop {
            if nodenum < 0 {
                if out.len() < 1024 {
                    out.push(-1 - nodenum);
                }
                return;
            }
            let node = &self.nodes[nodenum as usize];
            let s = Self::box_on_plane_side(&bounds[0], &bounds[1], &self.planes[node.plane as usize]);
            if s == 1 {
                nodenum = node.children[0];
            } else if s == 2 {
                nodenum = node.children[1];
            } else {
                self.box_leafnums_r(bounds, node.children[0], out);
                nodenum = node.children[1];
            }
        }
    }

    // --------------------------------------------------------- PVS / areas
    /// `CMod_LoadVisibility` + the area allocation of CMod_LoadLeafs, then
    /// CM_FloodAreaConnections.
    pub fn load_visibility(&mut self, vis: &[u8]) {
        self.num_clusters = 0;
        self.num_areas = 0;
        for l in &self.leafs {
            self.num_clusters = self.num_clusters.max(l.cluster + 1);
            self.num_areas = self.num_areas.max(l.area + 1);
        }
        if self.leafs.is_empty() {
            self.num_clusters = 1;
            self.num_areas = 1;
        }
        let na = self.num_areas.max(0) as usize;
        self.area_flood = vec![(0, 0); na];
        self.area_portals = vec![0; na * na];
        if vis.len() < 8 {
            self.cluster_bytes = (self.num_clusters + 31) & !31;
            self.visibility = vec![255; self.cluster_bytes as usize];
            self.vised = false;
        } else {
            self.vised = true;
            self.num_clusters = i32::from_le_bytes(vis[0..4].try_into().unwrap());
            self.cluster_bytes = i32::from_le_bytes(vis[4..8].try_into().unwrap());
            self.visibility = vis[8..].to_vec();
        }
        self.flood_area_connections();
    }

    /// `CM_PointLeafnum`.
    pub fn point_leafnum(&self, p: &Vec3) -> i32 {
        if self.nodes.is_empty() {
            return 0;
        }
        let mut num = 0i32;
        while num >= 0 {
            let node = &self.nodes[num as usize];
            let plane = &self.planes[node.plane as usize];
            let d = if plane.ty < 3 { p[plane.ty as usize] - plane.dist } else { dot(&plane.normal, p) - plane.dist };
            num = if d < 0.0 { node.children[1] } else { node.children[0] };
        }
        -1 - num
    }

    /// `CM_BoxLeafnums` (no list limit beyond 1024, like MAX_TOTAL_ENT_LEAFS).
    pub fn box_leafnums(&self, mins: &Vec3, maxs: &Vec3) -> Vec<i32> {
        let mut out = Vec::new();
        if !self.nodes.is_empty() {
            self.box_leafnums_r(&[*mins, *maxs], 0, &mut out);
        } else if !self.leafs.is_empty() {
            out.push(0);
        }
        out
    }

    /// `CM_ClusterPVS`: the row of visible-cluster bits for `cluster`.
    pub fn cluster_pvs(&self, cluster: i32) -> &[u8] {
        if cluster < 0 || cluster >= self.num_clusters || !self.vised {
            return &self.visibility;
        }
        let o = (cluster * self.cluster_bytes) as usize;
        self.visibility.get(o..).unwrap_or(&[])
    }

    fn flood_area_r(&mut self, area: usize, floodnum: i32) {
        if self.area_flood[area].1 == self.flood_valid {
            if self.area_flood[area].0 == floodnum {
                return;
            }
            // Com_Error "FloodArea_r: reflooded" cannot happen with symmetric counts
            return;
        }
        self.area_flood[area] = (floodnum, self.flood_valid);
        let na = self.num_areas as usize;
        for i in 0..na {
            if self.area_portals[area * na + i] > 0 {
                self.flood_area_r(i, floodnum);
            }
        }
    }

    /// `CM_FloodAreaConnections`.
    pub fn flood_area_connections(&mut self) {
        self.flood_valid += 1;
        let mut floodnum = 0;
        for i in 0..self.num_areas.max(0) as usize {
            if self.area_flood[i].1 == self.flood_valid {
                continue;
            }
            floodnum += 1;
            self.flood_area_r(i, floodnum);
        }
    }

    /// `CM_AdjustAreaPortalState`.
    pub fn adjust_area_portal_state(&mut self, a1: i32, a2: i32, open: bool) {
        if a1 < 0 || a2 < 0 || a1 >= self.num_areas || a2 >= self.num_areas {
            return;
        }
        let na = self.num_areas as usize;
        let (a1, a2) = (a1 as usize, a2 as usize);
        let d = if open { 1 } else { -1 };
        self.area_portals[a1 * na + a2] += d;
        self.area_portals[a2 * na + a1] += d;
        if self.area_portals[a2 * na + a1] < 0 {
            // Com_Error "negative reference count": clamp instead of dropping the game
            self.area_portals[a1 * na + a2] = 0;
            self.area_portals[a2 * na + a1] = 0;
        }
        self.flood_area_connections();
    }

    /// `CM_AreasConnected`.
    pub fn areas_connected(&self, a1: i32, a2: i32) -> bool {
        if a1 < 0 || a2 < 0 || a1 >= self.num_areas || a2 >= self.num_areas {
            return false;
        }
        self.area_flood[a1 as usize].0 == self.area_flood[a2 as usize].0
    }

    /// `SV_inPVS`: p2 is in p1's potentially visible set and no shut door
    /// (area portal) separates them.
    pub fn in_pvs(&self, p1: &Vec3, p2: &Vec3) -> bool {
        if self.leafs.is_empty() {
            return true;
        }
        let l1 = self.leafs[self.point_leafnum(p1) as usize];
        let l2 = self.leafs[self.point_leafnum(p2) as usize];
        let mask = self.cluster_pvs(l1.cluster);
        if l2.cluster >= 0 {
            let byte = mask.get((l2.cluster >> 3) as usize).copied().unwrap_or(0);
            if byte & (1 << (l2.cluster & 7)) == 0 {
                return false;
            }
        }
        self.areas_connected(l1.area, l2.area)
    }

    fn position_test(&self, tw: &mut TraceWork) {
        let mut b = [super::qmath::add(&tw.start, &tw.size[0]), super::qmath::add(&tw.start, &tw.size[1])];
        for i in 0..3 {
            b[0][i] -= 1.0;
            b[1][i] += 1.0;
        }
        let mut leafs = Vec::new();
        self.checkcount.set(self.checkcount.get().wrapping_add(1));
        self.box_leafnums_r(&b, 0, &mut leafs);
        self.checkcount.set(self.checkcount.get().wrapping_add(1));
        for l in leafs {
            self.test_in_leaf(tw, &self.leafs[l as usize]);
            if tw.trace.allsolid {
                break;
            }
        }
    }

    // ------------------------------------------------------------------ tracing
    fn trace_through_brush(&self, tw: &mut TraceWork, b: &Brush, bn: usize) {
        let mut enter_frac: f32 = -1.0;
        let mut leave_frac: f32 = 1.0;
        let mut clipplane: Option<&Plane> = None;
        let mut leadside: Option<&BrushSide> = None;
        if b.num_sides == 0 {
            return;
        }
        let mut getout = false;
        let mut startout = false;
        for i in 0..b.num_sides {
            let side = &self.sides[(b.first_side + i) as usize];
            let p = &self.planes[side.plane as usize];
            let (d1, d2) = if let Some((sr, er)) = tw.bisphere {
                // adjust the plane distance apropriately for radius
                (dot(&tw.start, &p.normal) - (p.dist + sr), dot(&tw.end, &p.normal) - (p.dist + er))
            } else {
                let dist = p.dist - dot(&tw.offsets[p.signbits as usize], &p.normal);
                (dot(&tw.start, &p.normal) - dist, dot(&tw.end, &p.normal) - dist)
            };
            if d2 > 0.0 {
                getout = true;
            }
            if d1 > 0.0 {
                startout = true;
            }
            if d1 > 0.0 && ((d2 as f64) >= SURFACE_CLIP_EPSILON || d2 >= d1) {
                return;
            }
            if d1 <= 0.0 && d2 <= 0.0 {
                continue;
            }
            if let Some(c) = self.brush_collided.get(bn) {
                c.set(true);
            }
            if d1 > d2 {
                let mut f = ((d1 as f64 - SURFACE_CLIP_EPSILON) / ((d1 - d2) as f64)) as f32;
                if f < 0.0 {
                    f = 0.0;
                }
                if f > enter_frac {
                    enter_frac = f;
                    clipplane = Some(p);
                    leadside = Some(side);
                }
            } else {
                let mut f = ((d1 as f64 + SURFACE_CLIP_EPSILON) / ((d1 - d2) as f64)) as f32;
                if f > 1.0 {
                    f = 1.0;
                }
                if f < leave_frac {
                    leave_frac = f;
                }
            }
        }
        if !startout {
            tw.trace.startsolid = true;
            if !getout {
                tw.trace.allsolid = true;
                tw.trace.fraction = 0.0;
                tw.trace.contents = b.contents;
            }
            return;
        }
        if enter_frac < leave_frac && enter_frac > -1.0 && enter_frac < tw.trace.fraction {
            if enter_frac < 0.0 {
                enter_frac = 0.0;
            }
            tw.trace.fraction = enter_frac;
            if let Some(p) = clipplane {
                tw.trace.plane = *p;
            }
            if let Some(s) = leadside {
                tw.trace.surface_flags = s.surface_flags;
            }
            tw.trace.contents = b.contents;
        }
    }

    fn bounds_intersect(mins: &Vec3, maxs: &Vec3, mins2: &Vec3, maxs2: &Vec3) -> bool {
        let e = SURFACE_CLIP_EPSILON;
        !((maxs[0] as f64) < mins2[0] as f64 - e
            || (maxs[1] as f64) < mins2[1] as f64 - e
            || (maxs[2] as f64) < mins2[2] as f64 - e
            || (mins[0] as f64) > maxs2[0] as f64 + e
            || (mins[1] as f64) > maxs2[1] as f64 + e
            || (mins[2] as f64) > maxs2[2] as f64 + e)
    }

    fn trace_through_leaf(&self, tw: &mut TraceWork, leaf: &Leaf) {
        let cc = self.checkcount.get();
        for k in 0..leaf.num_leaf_brushes {
            let bn = self.leaf_brushes[(leaf.first_leaf_brush + k) as usize] as usize;
            if self.brush_check[bn].get() == cc {
                continue;
            }
            self.brush_check[bn].set(cc);
            let b = &self.brushes[bn];
            if b.contents & tw.contents == 0 {
                continue;
            }
            self.brush_collided[bn].set(false);
            if !Self::bounds_intersect(&tw.bounds[0], &tw.bounds[1], &b.bounds[0], &b.bounds[1]) {
                continue;
            }
            self.trace_through_brush(tw, b, bn);
            if tw.trace.fraction == 0.0 {
                tw.trace.lateral_fraction = 0.0;
                return;
            }
        }
        for p in self.leaf_patches(leaf, tw.contents) {
            // CM_TraceThroughPatch: only normal/dist of the plane are written
            let old = tw.trace.fraction;
            let mut frac = tw.trace.fraction;
            let hit = super::patch::trace_through(&Self::patch_work(tw), &p.pc, &mut frac);
            tw.trace.fraction = frac;
            if let Some(pl) = hit {
                tw.trace.plane.normal = [pl[0], pl[1], pl[2]];
                tw.trace.plane.dist = pl[3];
            }
            if tw.trace.fraction < old {
                tw.trace.surface_flags = p.surface_flags;
                tw.trace.contents = p.contents;
            }
            if tw.trace.fraction == 0.0 {
                tw.trace.lateral_fraction = 0.0;
                return;
            }
        }
        if tw.test_lateral && tw.trace.fraction < 1.0 {
            for k in 0..leaf.num_leaf_brushes {
                let bn = self.leaf_brushes[(leaf.first_leaf_brush + k) as usize] as usize;
                // this brush never collided, so don't bother
                if !self.brush_collided[bn].get() || self.brushes[bn].contents & tw.contents == 0 {
                    continue;
                }
                self.proximity_to_brush(tw, bn);
                if tw.trace.lateral_fraction == 0.0 {
                    return;
                }
            }
            // `CM_ProximityToPatch`: a straight line through a patch zeroes the
            // lateral fraction; the proximity part is unimplemented in the C
            for k in 0..leaf.num_leaf_surfaces {
                let sn = self.leaf_surfaces[(leaf.first_leaf_surface + k) as usize] as usize;
                let Some(Some(p)) = self.patches.get(sn) else { continue };
                if p.contents & tw.contents == 0 {
                    continue;
                }
                let mut tw2 = Self::work(&tw.start, &tw.end, &VEC3_ORIGIN, &VEC3_ORIGIN, tw.contents);
                tw2.is_point = true;
                let mut frac = 1.0f32;
                super::patch::trace_through(&Self::patch_work(&tw2), &p.pc, &mut frac);
                if frac < 1.0 {
                    tw.trace.lateral_fraction = 0.0;
                    return;
                }
            }
        }
    }

    fn trace_through_tree(&self, tw: &mut TraceWork, num: i32, p1f: f32, p2f: f32, p1: Vec3, p2: Vec3) {
        if tw.trace.fraction <= p1f {
            return;
        }
        if num < 0 {
            let leaf = self.leafs[(-1 - num) as usize];
            self.trace_through_leaf(tw, &leaf);
            return;
        }
        let node = self.nodes[num as usize];
        let plane = &self.planes[node.plane as usize];
        let (t1, t2, offset): (f32, f32, f32);
        if plane.ty < 3 {
            t1 = p1[plane.ty as usize] - plane.dist;
            t2 = p2[plane.ty as usize] - plane.dist;
            offset = tw.extents[plane.ty as usize];
        } else {
            t1 = dot(&plane.normal, &p1) - plane.dist;
            t2 = dot(&plane.normal, &p2) - plane.dist;
            offset = if tw.is_point { 0.0 } else { 2048.0 };
        }
        if t1 >= offset + 1.0 && t2 >= offset + 1.0 {
            self.trace_through_tree(tw, node.children[0], p1f, p2f, p1, p2);
            return;
        }
        if t1 < -offset - 1.0 && t2 < -offset - 1.0 {
            self.trace_through_tree(tw, node.children[1], p1f, p2f, p1, p2);
            return;
        }
        let side: usize;
        let (mut frac, mut frac2): (f32, f32);
        if t1 < t2 {
            let idist = (1.0f64 / (t1 - t2) as f64) as f32;
            side = 1;
            frac2 = (((t1 + offset) as f64 + SURFACE_CLIP_EPSILON) * idist as f64) as f32;
            frac = (((t1 - offset) as f64 + SURFACE_CLIP_EPSILON) * idist as f64) as f32;
        } else if t1 > t2 {
            let idist = (1.0f64 / (t1 - t2) as f64) as f32;
            side = 0;
            frac2 = (((t1 - offset) as f64 - SURFACE_CLIP_EPSILON) * idist as f64) as f32;
            frac = (((t1 + offset) as f64 + SURFACE_CLIP_EPSILON) * idist as f64) as f32;
        } else {
            side = 0;
            frac = 1.0;
            frac2 = 0.0;
        }
        frac = frac.clamp(0.0, 1.0);
        let midf = p1f + (p2f - p1f) * frac;
        let mid = [p1[0] + frac * (p2[0] - p1[0]), p1[1] + frac * (p2[1] - p1[1]), p1[2] + frac * (p2[2] - p1[2])];
        self.trace_through_tree(tw, node.children[side], p1f, midf, p1, mid);
        frac2 = frac2.clamp(0.0, 1.0);
        let midf = p1f + (p2f - p1f) * frac2;
        let mid = [p1[0] + frac2 * (p2[0] - p1[0]), p1[1] + frac2 * (p2[1] - p1[1]), p1[2] + frac2 * (p2[2] - p1[2])];
        self.trace_through_tree(tw, node.children[side ^ 1], midf, p2f, mid, p2);
    }

    fn work(start: &Vec3, end: &Vec3, mins: &Vec3, maxs: &Vec3, brushmask: i32) -> TraceWork {
        let mut tw = TraceWork {
            start: [0.0; 3],
            end: [0.0; 3],
            size: [[0.0; 3]; 2],
            offsets: [[0.0; 3]; 8],
            extents: [0.0; 3],
            bounds: [[0.0; 3]; 2],
            contents: brushmask,
            is_point: false,
            bisphere: None,
            test_lateral: false,
            trace: Trace { fraction: 1.0, ..Default::default() },
        };
        for i in 0..3 {
            let offset = ((mins[i] + maxs[i]) as f64 * 0.5) as f32;
            tw.size[0][i] = mins[i] - offset;
            tw.size[1][i] = maxs[i] - offset;
            tw.start[i] = start[i] + offset;
            tw.end[i] = end[i] + offset;
        }
        let (s0, s1) = (tw.size[0], tw.size[1]);
        tw.offsets = [
            [s0[0], s0[1], s0[2]],
            [s1[0], s0[1], s0[2]],
            [s0[0], s1[1], s0[2]],
            [s1[0], s1[1], s0[2]],
            [s0[0], s0[1], s1[2]],
            [s1[0], s0[1], s1[2]],
            [s0[0], s1[1], s1[2]],
            [s1[0], s1[1], s1[2]],
        ];
        for i in 0..3 {
            if tw.start[i] < tw.end[i] {
                tw.bounds[0][i] = tw.start[i] + tw.size[0][i];
                tw.bounds[1][i] = tw.end[i] + tw.size[1][i];
            } else {
                tw.bounds[0][i] = tw.end[i] + tw.size[0][i];
                tw.bounds[1][i] = tw.start[i] + tw.size[1][i];
            }
        }
        tw
    }

    fn finish(tw: TraceWork, start: &Vec3, end: &Vec3) -> Trace {
        let mut t = tw.trace;
        if t.fraction == 1.0 {
            t.endpos = *end;
        } else {
            for i in 0..3 {
                t.endpos[i] = start[i] + t.fraction * (end[i] - start[i]);
            }
        }
        t
    }

    /// `CM_BoxTrace(results, start, end, mins, maxs, 0, brushmask, TT_AABB)`.
    pub fn box_trace(&self, start: &Vec3, end: &Vec3, mins: Option<&Vec3>, maxs: Option<&Vec3>, brushmask: i32) -> Trace {
        self.trace_model(start, end, mins.unwrap_or(&VEC3_ORIGIN), maxs.unwrap_or(&VEC3_ORIGIN), 0, brushmask)
    }

    /// `CM_Trace` (TT_AABB) against model 0 (the world tree) or an inline model's leaf.
    pub fn trace_model(&self, start: &Vec3, end: &Vec3, mins: &Vec3, maxs: &Vec3, model: usize, brushmask: i32) -> Trace {
        self.checkcount.set(self.checkcount.get().wrapping_add(1));
        let mut tw = Self::work(start, end, mins, maxs, brushmask);
        if self.nodes.is_empty() {
            return tw.trace;
        }
        let leaf = if model > 0 { self.models.get(model).map(|m| m.leaf) } else { None };
        if start[0] == end[0] && start[1] == end[1] && start[2] == end[2] {
            match leaf {
                Some(l) => self.test_in_leaf(&mut tw, &l),
                None if model > 0 => {}
                None => self.position_test(&mut tw),
            }
        } else {
            if tw.size[0][0] == 0.0 && tw.size[0][1] == 0.0 && tw.size[0][2] == 0.0 {
                tw.is_point = true;
                tw.extents = [0.0; 3];
            } else {
                tw.is_point = false;
                tw.extents = tw.size[1];
            }
            match leaf {
                Some(l) => self.trace_through_leaf(&mut tw, &l),
                None if model > 0 => {}
                None => {
                    let (s, e) = (tw.start, tw.end);
                    self.trace_through_tree(&mut tw, 0, 0.0, 1.0, s, e);
                }
            }
        }
        Self::finish(tw, start, end)
    }

    /// `CM_TransformedBoxTrace` (TT_AABB) against an inline model placed at
    /// `origin` rotated by `angles`.
    pub fn transformed_box_trace(&self, start: &Vec3, end: &Vec3, mins: &Vec3, maxs: &Vec3, model: usize, brushmask: i32, origin: &Vec3, angles: &Vec3) -> Trace {
        let mut sym = [[0.0f32; 3]; 2];
        let mut sl = [0.0f32; 3];
        let mut el = [0.0f32; 3];
        for i in 0..3 {
            let offset = ((mins[i] + maxs[i]) as f64 * 0.5) as f32;
            sym[0][i] = mins[i] - offset;
            sym[1][i] = maxs[i] - offset;
            sl[i] = start[i] + offset;
            el[i] = end[i] + offset;
        }
        sl = super::qmath::sub(&sl, origin);
        el = super::qmath::sub(&el, origin);
        let rotated = angles[0] != 0.0 || angles[1] != 0.0 || angles[2] != 0.0;
        let mut matrix = [[0.0f32; 3]; 3];
        if rotated {
            let (f, r, u) = super::qmath::angle_vectors(angles);
            matrix = [f, super::qmath::neg(&r), u];
            sl = [dot(&matrix[0], &sl), dot(&matrix[1], &sl), dot(&matrix[2], &sl)];
            el = [dot(&matrix[0], &el), dot(&matrix[1], &el), dot(&matrix[2], &el)];
        }
        let mut t = self.trace_model(&sl, &el, &sym[0], &sym[1], model, brushmask);
        if rotated && t.fraction != 1.0 {
            let tr = [[matrix[0][0], matrix[1][0], matrix[2][0]], [matrix[0][1], matrix[1][1], matrix[2][1]], [matrix[0][2], matrix[1][2], matrix[2][2]]];
            let n = t.plane.normal;
            t.plane.normal = [dot(&tr[0], &n), dot(&tr[1], &n), dot(&tr[2], &n)];
        }
        for i in 0..3 {
            t.endpos[i] = start[i] + t.fraction * (end[i] - start[i]);
        }
        t
    }

    /// `CM_TransformedPointContents` for an inline model.
    pub fn transformed_point_contents(&self, p: &Vec3, model: usize, origin: &Vec3, angles: &Vec3) -> i32 {
        let mut pl = super::qmath::sub(p, origin);
        if angles[0] != 0.0 || angles[1] != 0.0 || angles[2] != 0.0 {
            let (f, r, u) = super::qmath::angle_vectors(angles);
            let t = pl;
            pl = [dot(&t, &f), -dot(&t, &r), dot(&t, &u)];
        }
        match self.models.get(model) {
            Some(m) if model > 0 => self.leaf_contents(&pl, &m.leaf),
            _ => 0,
        }
    }

    /// `CM_PointContents(p, 0)`.
    pub fn point_contents(&self, p: &Vec3) -> i32 {
        if self.nodes.is_empty() {
            return 0;
        }
        let mut num = 0i32;
        while num >= 0 {
            let node = &self.nodes[num as usize];
            let plane = &self.planes[node.plane as usize];
            let d = if plane.ty < 3 { p[plane.ty as usize] - plane.dist } else { dot(&plane.normal, p) - plane.dist };
            num = if d < 0.0 { node.children[1] } else { node.children[0] };
        }
        let leaf = self.leafs[(-1 - num) as usize];
        self.leaf_contents(p, &leaf)
    }

    fn leaf_contents(&self, p: &Vec3, leaf: &Leaf) -> i32 {
        if leaf.area == -1 {
            return 1; // CONTENTS_SOLID
        }
        let mut contents = 0;
        for k in 0..leaf.num_leaf_brushes {
            let b = &self.brushes[self.leaf_brushes[(leaf.first_leaf_brush + k) as usize] as usize];
            let e = SURFACE_CLIP_EPSILON;
            if (b.bounds[1][0] as f64) < p[0] as f64 - e
                || (b.bounds[1][1] as f64) < p[1] as f64 - e
                || (b.bounds[1][2] as f64) < p[2] as f64 - e
                || (b.bounds[0][0] as f64) > p[0] as f64 + e
                || (b.bounds[0][1] as f64) > p[1] as f64 + e
                || (b.bounds[0][2] as f64) > p[2] as f64 + e
            {
                continue;
            }
            let mut inside = true;
            for i in 0..b.num_sides {
                let pl = self.side_plane(b, i);
                if dot(p, &pl.normal) > pl.dist {
                    inside = false;
                    break;
                }
            }
            if inside {
                contents |= b.contents;
            }
        }
        contents
    }
}

/// Trace an AABB against a single box entity, exactly as SV_ClipMoveToEntities
/// does via CM_TempBoxModel + CM_TransformedBoxTrace (no rotation).
pub fn trace_against_box(start: &Vec3, end: &Vec3, mins: &Vec3, maxs: &Vec3, emins: &Vec3, emaxs: &Vec3, eorigin: &Vec3, contents: i32, brushmask: i32) -> Trace {
    let mut cm = ClipMap::default();
    cm.add_box_brush(*emins, *emaxs, None, contents, 0, 0);
    cm.leaf_brushes = vec![0];
    cm.leafs = vec![Leaf { cluster: 0, area: 0, first_leaf_brush: 0, num_leaf_brushes: 1, ..Default::default() }];
    cm.finalize();
    cm.load_visibility(&[]);
    cm.checkcount.set(1);
    // CM_TransformedBoxTrace: symmetric box, start_l = (start + offset) - origin
    let mut sym = [[0.0f32; 3]; 2];
    let mut s = [0.0f32; 3];
    let mut e = [0.0f32; 3];
    for i in 0..3 {
        let offset = ((mins[i] + maxs[i]) as f64 * 0.5) as f32;
        sym[0][i] = mins[i] - offset;
        sym[1][i] = maxs[i] - offset;
        s[i] = (start[i] + offset) - eorigin[i];
        e[i] = (end[i] + offset) - eorigin[i];
    }
    let mut tw = ClipMap::work(&s, &e, &sym[0], &sym[1], brushmask);
    let leaf = cm.leafs[0];
    if s == e {
        cm.test_in_leaf(&mut tw, &leaf);
    } else {
        tw.is_point = tw.size[0] == [0.0; 3];
        tw.extents = if tw.is_point { [0.0; 3] } else { tw.size[1] };
        cm.trace_through_leaf(&mut tw, &leaf);
    }
    let mut t = tw.trace;
    // TransformedBoxTrace always recomputes endpos from the original segment
    for i in 0..3 {
        t.endpos[i] = start[i] + t.fraction * (end[i] - start[i]);
    }
    t
}

/// `CM_TransformedBiSphereTrace` against a temp box model at `eorigin`.
pub fn bisphere_against_box(start: &Vec3, end: &Vec3, start_rad: f32, end_rad: f32, emins: &Vec3, emaxs: &Vec3, eorigin: &Vec3, contents: i32, mask: i32) -> Trace {
    let mut cm = ClipMap::default();
    cm.add_box_brush(*emins, *emaxs, None, contents, 0, 0);
    cm.leaf_brushes = vec![0];
    cm.leafs = vec![Leaf { cluster: 0, area: 0, first_leaf_brush: 0, num_leaf_brushes: 1, ..Default::default() }];
    cm.models = vec![CModel::default(), CModel { mins: *emins, maxs: *emaxs, leaf: cm.leafs[0] }];
    // a placeholder node so the "map not loaded" check passes
    cm.nodes = vec![Node { plane: 0, children: [-1, -1] }];
    cm.finalize();
    let s = super::qmath::sub(start, eorigin);
    let e = super::qmath::sub(end, eorigin);
    let mut t = cm.bisphere_trace(&s, &e, start_rad, end_rad, 1, mask);
    for i in 0..3 {
        t.endpos[i] = start[i] + t.fraction * (end[i] - start[i]);
    }
    t
}

/// Build the clip map for a sheet-authored map (brushes.tsv rows for `map`, in
/// sheet order) exactly as re/harness/cm_world.c builds the C oracle's.
pub fn from_sheet(map: crate::gen::maps::MapId) -> ClipMap {
    let mut cm = ClipMap::default();
    for (i, b) in crate::gen::brushes::BRUSHES.iter().enumerate() {
        if b.map != map {
            continue;
        }
        cm.add_box_brush(b.mins, b.maxs, b.cut, b.contents as i32, b.surface as i32, i as u32);
    }
    cm.build_single_leaf_tree();
    cm
}

/// `DistanceBetweenLineSegmentsSquared` (q_math.c): (dist², s, t).
pub fn distance_between_line_segments_squared(sp0: &Vec3, sp1: &Vec3, tp0: &Vec3, tp1: &Vec3) -> (f32, f32, f32) {
    const LINE_DISTANCE_EPSILON: f32 = 1e-05;
    let smag = super::qmath::sub(sp1, sp0);
    let tmag = super::qmath::sub(tp1, tp0);
    let diff = super::qmath::sub(sp0, tp0);
    let a = dot(&smag, &smag);
    let b = dot(&smag, &tmag);
    let c = dot(&tmag, &tmag);
    let d = dot(&smag, &diff);
    let e = dot(&tmag, &diff);
    let dd = a * c - b * b;
    let (mut sd, mut td) = (dd, dd);
    let (mut sn, mut tn);
    if dd < LINE_DISTANCE_EPSILON {
        sn = 0.0;
        sd = 1.0;
        tn = e;
        td = c;
    } else {
        sn = b * e - c * d;
        tn = a * e - b * d;
        if sn < 0.0 {
            sn = 0.0;
            tn = e;
            td = c;
        } else if sn > sd {
            sn = sd;
            tn = e + b;
            td = c;
        }
    }
    if tn < 0.0 {
        tn = 0.0;
        if -d < 0.0 {
            sn = 0.0;
        } else if -d > a {
            sn = sd;
        } else {
            sn = -d;
            sd = a;
        }
    } else if tn > td {
        tn = td;
        if (-d + b) < 0.0 {
            sn = 0.0;
        } else if (-d + b) > a {
            sn = sd;
        } else {
            sn = -d + b;
            sd = a;
        }
    }
    let s = if sn.abs() < LINE_DISTANCE_EPSILON { 0.0 } else { sn / sd };
    let t = if tn.abs() < LINE_DISTANCE_EPSILON { 0.0 } else { tn / td };
    let mut sep = [0.0f32; 3];
    for i in 0..3 {
        sep[i] = diff[i] + smag[i] * s - tmag[i] * t;
    }
    (dot(&sep, &sep), s, t)
}
