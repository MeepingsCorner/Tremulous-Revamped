//! Kernel `bsp` (Format) — IBSP v46 reader/writer (qcommon/qfiles.h) and the
//! clip-map half of qcommon/cm_load.c. The collision model built here is the
//! same `ClipMap` the golden traces verify; only the loader is new.
//!
//! Not ported: curved patch collision (cm_patch.c), submodel (mover) clip
//! trees, area portals and PVS. Brush models other than model 0 are kept for
//! rendering only.

use super::cm::{Brush, BrushSide, CModel, CPatch, ClipMap, Leaf, Node, Plane};
use super::qmath::Vec3;
use std::collections::BTreeMap;

pub const BSP_IDENT: &[u8; 4] = b"IBSP";
pub const BSP_VERSION: i32 = 46;
pub const HEADER_LUMPS: usize = 17;
pub const LUMP_ENTITIES: usize = 0;
pub const LUMP_SHADERS: usize = 1;
pub const LUMP_PLANES: usize = 2;
pub const LUMP_NODES: usize = 3;
pub const LUMP_LEAFS: usize = 4;
pub const LUMP_LEAFSURFACES: usize = 5;
pub const LUMP_LEAFBRUSHES: usize = 6;
pub const LUMP_MODELS: usize = 7;
pub const LUMP_BRUSHES: usize = 8;
pub const LUMP_BRUSHSIDES: usize = 9;
pub const LUMP_DRAWVERTS: usize = 10;
pub const LUMP_DRAWINDEXES: usize = 11;
pub const LUMP_FOGS: usize = 12;
pub const LUMP_SURFACES: usize = 13;
pub const LUMP_LIGHTMAPS: usize = 14;

pub const MST_PLANAR: i32 = 1;
pub const MST_PATCH: i32 = 2;
pub const MST_TRIANGLE_SOUP: i32 = 3;
pub const MST_FLARE: i32 = 4;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shader {
    pub name: String,
    pub surface_flags: i32,
    pub content_flags: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DNode {
    pub plane: i32,
    pub children: [i32; 2],
    pub mins: [i32; 3],
    pub maxs: [i32; 3],
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DLeaf {
    pub cluster: i32,
    pub area: i32,
    pub mins: [i32; 3],
    pub maxs: [i32; 3],
    pub first_leaf_surface: i32,
    pub num_leaf_surfaces: i32,
    pub first_leaf_brush: i32,
    pub num_leaf_brushes: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DModel {
    pub mins: Vec3,
    pub maxs: Vec3,
    pub first_surface: i32,
    pub num_surfaces: i32,
    pub first_brush: i32,
    pub num_brushes: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DBrush {
    pub first_side: i32,
    pub num_sides: i32,
    pub shader: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DBrushSide {
    pub plane: i32,
    pub shader: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawVert {
    pub xyz: Vec3,
    pub st: [f32; 2],
    pub lightmap: [f32; 2],
    pub normal: Vec3,
    pub color: [u8; 4],
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DSurface {
    pub shader: i32,
    pub fog: i32,
    pub surface_type: i32,
    pub first_vert: i32,
    pub num_verts: i32,
    pub first_index: i32,
    pub num_indexes: i32,
    pub lightmap_num: i32,
    pub lightmap_x: i32,
    pub lightmap_y: i32,
    pub lightmap_width: i32,
    pub lightmap_height: i32,
    pub lightmap_origin: Vec3,
    pub lightmap_vecs: [Vec3; 3],
    pub patch_width: i32,
    pub patch_height: i32,
}

/// A whole IBSP file. Lumps not listed (fogs, lightgrid, visibility) are kept raw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bsp {
    pub entities: String,
    pub shaders: Vec<Shader>,
    pub planes: Vec<[f32; 4]>,
    pub nodes: Vec<DNode>,
    pub leafs: Vec<DLeaf>,
    pub leaf_surfaces: Vec<i32>,
    pub leaf_brushes: Vec<i32>,
    pub models: Vec<DModel>,
    pub brushes: Vec<DBrush>,
    pub brush_sides: Vec<DBrushSide>,
    pub verts: Vec<DrawVert>,
    pub indexes: Vec<i32>,
    pub surfaces: Vec<DSurface>,
    /// 128x128 RGB lightmaps, raw.
    pub lightmaps: Vec<u8>,
    pub raw: BTreeMap<usize, Vec<u8>>,
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}
impl Rd<'_> {
    fn i(&mut self) -> i32 {
        let v = i32::from_le_bytes(self.b[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    fn f(&mut self) -> f32 {
        let v = f32::from_le_bytes(self.b[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    fn v3(&mut self) -> Vec3 {
        [self.f(), self.f(), self.f()]
    }
    fn i3(&mut self) -> [i32; 3] {
        [self.i(), self.i(), self.i()]
    }
    fn name(&mut self) -> String {
        let s = &self.b[self.p..self.p + 64];
        self.p += 64;
        let end = s.iter().position(|&c| c == 0).unwrap_or(64);
        String::from_utf8_lossy(&s[..end]).into_owned()
    }
}

fn lump<'a>(bytes: &'a [u8], lumps: &[(usize, usize)], n: usize, size: usize) -> Result<(Rd<'a>, usize), String> {
    let (ofs, len) = lumps[n];
    if ofs + len > bytes.len() {
        return Err(format!("lump {n} out of range"));
    }
    if size > 0 && len % size != 0 {
        return Err(format!("lump {n}: funny lump size {len} (record {size})"));
    }
    Ok((Rd { b: &bytes[ofs..ofs + len], p: 0 }, if size > 0 { len / size } else { len }))
}

/// Parse an IBSP v46 file.
pub fn parse(bytes: &[u8]) -> Result<Bsp, String> {
    if bytes.len() < 8 + HEADER_LUMPS * 8 || &bytes[0..4] != BSP_IDENT {
        return Err("not an IBSP file".into());
    }
    let version = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != BSP_VERSION {
        return Err(format!("BSP version {version}, expected {BSP_VERSION}"));
    }
    let mut lumps = Vec::with_capacity(HEADER_LUMPS);
    for i in 0..HEADER_LUMPS {
        let o = 8 + i * 8;
        let ofs = i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
        let len = i32::from_le_bytes(bytes[o + 4..o + 8].try_into().unwrap()) as usize;
        lumps.push((ofs, len));
    }
    let mut b = Bsp::default();
    let (r, n) = lump(bytes, &lumps, LUMP_ENTITIES, 0)?;
    let ents = &r.b[..n];
    let end = ents.iter().position(|&c| c == 0).unwrap_or(n);
    b.entities = String::from_utf8_lossy(&ents[..end]).into_owned();
    let (mut r, n) = lump(bytes, &lumps, LUMP_SHADERS, 72)?;
    for _ in 0..n {
        b.shaders.push(Shader { name: r.name(), surface_flags: r.i(), content_flags: r.i() });
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_PLANES, 16)?;
    for _ in 0..n {
        b.planes.push([r.f(), r.f(), r.f(), r.f()]);
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_NODES, 36)?;
    for _ in 0..n {
        b.nodes.push(DNode { plane: r.i(), children: [r.i(), r.i()], mins: r.i3(), maxs: r.i3() });
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_LEAFS, 48)?;
    for _ in 0..n {
        b.leafs.push(DLeaf { cluster: r.i(), area: r.i(), mins: r.i3(), maxs: r.i3(), first_leaf_surface: r.i(), num_leaf_surfaces: r.i(), first_leaf_brush: r.i(), num_leaf_brushes: r.i() });
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_LEAFSURFACES, 4)?;
    b.leaf_surfaces = (0..n).map(|_| r.i()).collect();
    let (mut r, n) = lump(bytes, &lumps, LUMP_LEAFBRUSHES, 4)?;
    b.leaf_brushes = (0..n).map(|_| r.i()).collect();
    let (mut r, n) = lump(bytes, &lumps, LUMP_MODELS, 40)?;
    for _ in 0..n {
        b.models.push(DModel { mins: r.v3(), maxs: r.v3(), first_surface: r.i(), num_surfaces: r.i(), first_brush: r.i(), num_brushes: r.i() });
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_BRUSHES, 12)?;
    for _ in 0..n {
        b.brushes.push(DBrush { first_side: r.i(), num_sides: r.i(), shader: r.i() });
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_BRUSHSIDES, 8)?;
    for _ in 0..n {
        b.brush_sides.push(DBrushSide { plane: r.i(), shader: r.i() });
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_DRAWVERTS, 44)?;
    for _ in 0..n {
        let xyz = r.v3();
        let st = [r.f(), r.f()];
        let lm = [r.f(), r.f()];
        let normal = r.v3();
        let c = [r.b[r.p], r.b[r.p + 1], r.b[r.p + 2], r.b[r.p + 3]];
        r.p += 4;
        b.verts.push(DrawVert { xyz, st, lightmap: lm, normal, color: c });
    }
    let (mut r, n) = lump(bytes, &lumps, LUMP_DRAWINDEXES, 4)?;
    b.indexes = (0..n).map(|_| r.i()).collect();
    let (mut r, n) = lump(bytes, &lumps, LUMP_SURFACES, 104)?;
    for _ in 0..n {
        b.surfaces.push(DSurface {
            shader: r.i(),
            fog: r.i(),
            surface_type: r.i(),
            first_vert: r.i(),
            num_verts: r.i(),
            first_index: r.i(),
            num_indexes: r.i(),
            lightmap_num: r.i(),
            lightmap_x: r.i(),
            lightmap_y: r.i(),
            lightmap_width: r.i(),
            lightmap_height: r.i(),
            lightmap_origin: r.v3(),
            lightmap_vecs: [r.v3(), r.v3(), r.v3()],
            patch_width: r.i(),
            patch_height: r.i(),
        });
    }
    let (r, n) = lump(bytes, &lumps, LUMP_LIGHTMAPS, 0)?;
    b.lightmaps = r.b[..n].to_vec();
    for l in [LUMP_FOGS, 15, 16] {
        let (r, n) = lump(bytes, &lumps, l, 0)?;
        b.raw.insert(l, r.b[..n].to_vec());
    }
    b.validate()?;
    Ok(b)
}

impl Bsp {
    /// The index checks cm_load.c makes with Com_Error.
    pub fn validate(&self) -> Result<(), String> {
        if self.planes.is_empty() || self.nodes.is_empty() || self.leafs.is_empty() || self.models.is_empty() {
            return Err("map has no planes/nodes/leafs/models".into());
        }
        let np = self.planes.len() as i32;
        for (i, n) in self.nodes.iter().enumerate() {
            if n.plane < 0 || n.plane >= np {
                return Err(format!("node {i}: bad plane"));
            }
            for c in n.children {
                if c >= self.nodes.len() as i32 || -1 - c >= self.leafs.len() as i32 {
                    return Err(format!("node {i}: bad child {c}"));
                }
            }
        }
        for (i, l) in self.leafs.iter().enumerate() {
            if l.first_leaf_brush < 0 || (l.first_leaf_brush + l.num_leaf_brushes) as usize > self.leaf_brushes.len() {
                return Err(format!("leaf {i}: bad leafbrush range"));
            }
        }
        for (i, b) in self.brushes.iter().enumerate() {
            if b.shader < 0 || b.shader as usize >= self.shaders.len() {
                return Err(format!("CMod_LoadBrushes: bad shaderNum: {}", b.shader));
            }
            if b.num_sides < 6 || b.first_side < 0 || (b.first_side + b.num_sides) as usize > self.brush_sides.len() {
                return Err(format!("brush {i}: bad sides"));
            }
        }
        for s in &self.brush_sides {
            if s.shader < 0 || s.shader as usize >= self.shaders.len() {
                return Err(format!("CMod_LoadBrushSides: bad shaderNum: {}", s.shader));
            }
            if s.plane < 0 || s.plane >= np {
                return Err("brush side: bad plane".into());
            }
        }
        for &lb in &self.leaf_brushes {
            if lb < 0 || lb as usize >= self.brushes.len() {
                return Err("bad leafbrush".into());
            }
        }
        Ok(())
    }

    /// `CM_LoadMap` (collision half): planes, nodes, leafs, leafbrushes,
    /// brushes (contents from shader, bounds from the six axial sides),
    /// brush sides (surface flags from shader).
    pub fn clip_map(&self) -> ClipMap {
        let mut cm = ClipMap::default();
        for p in &self.planes {
            let n = [p[0], p[1], p[2]];
            let ty = if n[0] == 1.0 { 0 } else if n[1] == 1.0 { 1 } else if n[2] == 1.0 { 2 } else { 3 };
            cm.planes.push(Plane::new(n, p[3], ty));
        }
        cm.nodes = self.nodes.iter().map(|n| Node { plane: n.plane as u32, children: n.children }).collect();
        cm.leafs = self
            .leafs
            .iter()
            .map(|l| Leaf { cluster: l.cluster, area: l.area, first_leaf_brush: l.first_leaf_brush as u32, num_leaf_brushes: l.num_leaf_brushes as u32, first_leaf_surface: l.first_leaf_surface as u32, num_leaf_surfaces: l.num_leaf_surfaces as u32 })
            .collect();
        cm.leaf_brushes = self.leaf_brushes.iter().map(|&x| x as u32).collect();
        cm.leaf_surfaces = self.leaf_surfaces.iter().map(|&x| x as u32).collect();
        // CMod_LoadSubmodels: a leaf per inline model, its indexes appended
        for (i, m) in self.models.iter().enumerate() {
            let mut out = CModel { mins: [m.mins[0] - 1.0, m.mins[1] - 1.0, m.mins[2] - 1.0], maxs: [m.maxs[0] + 1.0, m.maxs[1] + 1.0, m.maxs[2] + 1.0], leaf: Leaf::default() };
            if i > 0 {
                out.leaf.first_leaf_brush = cm.leaf_brushes.len() as u32;
                out.leaf.num_leaf_brushes = m.num_brushes.max(0) as u32;
                cm.leaf_brushes.extend((0..m.num_brushes.max(0)).map(|j| (m.first_brush + j) as u32));
                out.leaf.first_leaf_surface = cm.leaf_surfaces.len() as u32;
                out.leaf.num_leaf_surfaces = m.num_surfaces.max(0) as u32;
                cm.leaf_surfaces.extend((0..m.num_surfaces.max(0)).map(|j| (m.first_surface + j) as u32));
            }
            cm.models.push(out);
        }
        // CMod_LoadPatches
        cm.patches = self
            .surfaces
            .iter()
            .map(|sf| {
                if sf.surface_type != MST_PATCH {
                    return None;
                }
                let (w, h) = (sf.patch_width.max(0) as usize, sf.patch_height.max(0) as usize);
                let first = sf.first_vert.max(0) as usize;
                if w * h > 1024 || first + w * h > self.verts.len() || sf.shader < 0 || sf.shader as usize >= self.shaders.len() {
                    return None;
                }
                let pts: Vec<Vec3> = self.verts[first..first + w * h].iter().map(|v| v.xyz).collect();
                let sh = &self.shaders[sf.shader as usize];
                super::patch::generate(w, h, &pts).ok().map(|pc| CPatch { contents: sh.content_flags, surface_flags: sh.surface_flags, pc })
            })
            .collect();
        cm.sides = self.brush_sides.iter().map(|s| BrushSide { plane: s.plane as u32, surface_flags: self.shaders[s.shader as usize].surface_flags }).collect();
        for b in &self.brushes {
            let d = |k: i32| self.planes[self.brush_sides[(b.first_side + k) as usize].plane as usize][3];
            let bounds = [[-d(0), -d(2), -d(4)], [d(1), d(3), d(5)]];
            cm.brushes.push(Brush { contents: self.shaders[b.shader as usize].content_flags, bounds, first_side: b.first_side as u32, num_sides: b.num_sides as u32, tag: b.shader as u32 });
        }
        cm.finalize();
        cm.load_visibility(self.raw.get(&16).map(|v| v.as_slice()).unwrap_or(&[]));
        cm
    }

    /// Entity lump as a list of key/value maps (in file order).
    pub fn entity_list(&self) -> Vec<BTreeMap<String, String>> {
        parse_entities(&self.entities)
    }

    /// Serialize (used by tests and by the sheet-map exporter).
    pub fn write(&self) -> Vec<u8> {
        let mut lumps: Vec<Vec<u8>> = vec![Vec::new(); HEADER_LUMPS];
        let w = |v: &mut Vec<u8>, x: i32| v.extend_from_slice(&x.to_le_bytes());
        let wf = |v: &mut Vec<u8>, x: f32| v.extend_from_slice(&x.to_le_bytes());
        let mut e = self.entities.as_bytes().to_vec();
        e.push(0);
        lumps[LUMP_ENTITIES] = e;
        for s in &self.shaders {
            let l = &mut lumps[LUMP_SHADERS];
            let mut name = [0u8; 64];
            let nb = s.name.as_bytes();
            name[..nb.len().min(63)].copy_from_slice(&nb[..nb.len().min(63)]);
            l.extend_from_slice(&name);
            w(l, s.surface_flags);
            w(l, s.content_flags);
        }
        for p in &self.planes {
            for x in p {
                wf(&mut lumps[LUMP_PLANES], *x);
            }
        }
        for n in &self.nodes {
            let l = &mut lumps[LUMP_NODES];
            for x in [n.plane, n.children[0], n.children[1], n.mins[0], n.mins[1], n.mins[2], n.maxs[0], n.maxs[1], n.maxs[2]] {
                w(l, x);
            }
        }
        for f in &self.leafs {
            let l = &mut lumps[LUMP_LEAFS];
            for x in [f.cluster, f.area, f.mins[0], f.mins[1], f.mins[2], f.maxs[0], f.maxs[1], f.maxs[2], f.first_leaf_surface, f.num_leaf_surfaces, f.first_leaf_brush, f.num_leaf_brushes] {
                w(l, x);
            }
        }
        for &x in &self.leaf_surfaces {
            w(&mut lumps[LUMP_LEAFSURFACES], x);
        }
        for &x in &self.leaf_brushes {
            w(&mut lumps[LUMP_LEAFBRUSHES], x);
        }
        for m in &self.models {
            let l = &mut lumps[LUMP_MODELS];
            for x in m.mins.iter().chain(m.maxs.iter()) {
                wf(l, *x);
            }
            for x in [m.first_surface, m.num_surfaces, m.first_brush, m.num_brushes] {
                w(l, x);
            }
        }
        for b in &self.brushes {
            for x in [b.first_side, b.num_sides, b.shader] {
                w(&mut lumps[LUMP_BRUSHES], x);
            }
        }
        for s in &self.brush_sides {
            for x in [s.plane, s.shader] {
                w(&mut lumps[LUMP_BRUSHSIDES], x);
            }
        }
        for v in &self.verts {
            let l = &mut lumps[LUMP_DRAWVERTS];
            for x in v.xyz.iter().chain(v.st.iter()).chain(v.lightmap.iter()).chain(v.normal.iter()) {
                wf(l, *x);
            }
            l.extend_from_slice(&v.color);
        }
        for &x in &self.indexes {
            w(&mut lumps[LUMP_DRAWINDEXES], x);
        }
        for s in &self.surfaces {
            let l = &mut lumps[LUMP_SURFACES];
            for x in [s.shader, s.fog, s.surface_type, s.first_vert, s.num_verts, s.first_index, s.num_indexes, s.lightmap_num, s.lightmap_x, s.lightmap_y, s.lightmap_width, s.lightmap_height] {
                w(l, x);
            }
            for x in s.lightmap_origin.iter().chain(s.lightmap_vecs.iter().flatten()) {
                wf(l, *x);
            }
            w(l, s.patch_width);
            w(l, s.patch_height);
        }
        lumps[LUMP_LIGHTMAPS] = self.lightmaps.clone();
        for (k, v) in &self.raw {
            lumps[*k] = v.clone();
        }
        let mut out = Vec::new();
        out.extend_from_slice(BSP_IDENT);
        out.extend_from_slice(&BSP_VERSION.to_le_bytes());
        let mut ofs = 8 + HEADER_LUMPS * 8;
        for l in &lumps {
            out.extend_from_slice(&(ofs as i32).to_le_bytes());
            out.extend_from_slice(&(l.len() as i32).to_le_bytes());
            ofs += (l.len() + 3) & !3;
        }
        for l in &lumps {
            out.extend_from_slice(l);
            while out.len() % 4 != 0 {
                out.push(0);
            }
        }
        out
    }
}

/// Q3 entity-string parser ({ "key" "value" ... } blocks).
pub fn parse_entities(s: &str) -> Vec<BTreeMap<String, String>> {
    let mut out = Vec::new();
    let mut cur: Option<BTreeMap<String, String>> = None;
    let mut pending: Option<String> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => cur = Some(BTreeMap::new()),
            '}' => {
                if let Some(m) = cur.take() {
                    out.push(m);
                }
                pending = None;
            }
            '"' => {
                let mut tok = String::new();
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                    tok.push(d);
                }
                if let Some(m) = cur.as_mut() {
                    match pending.take() {
                        None => pending = Some(tok),
                        Some(k) => {
                            m.insert(k, tok);
                        }
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for d in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// "x y z" -> Vec3.
pub fn parse_vec(s: &str) -> Option<Vec3> {
    let v: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// Export a sheet-authored map as an IBSP (single-leaf tree, brushes in q3
/// side order -x,+x,-y,+y,-z,+z[,cut], planar faces with colour-only shaders).
/// Used to test the loader and to give map authors a starting point.
pub fn from_sheet(map: crate::gen::maps::MapId) -> Bsp {
    let mut b = Bsp::default();
    let mut shader_ix: BTreeMap<(String, i32, i32), i32> = BTreeMap::new();
    let mut shader = |b: &mut Bsp, name: &str, surf: i32, cont: i32| -> i32 {
        *shader_ix.entry((name.to_string(), surf, cont)).or_insert_with(|| {
            b.shaders.push(Shader { name: name.to_string(), surface_flags: surf, content_flags: cont });
            b.shaders.len() as i32 - 1
        })
    };
    let mut leaf_brushes = Vec::new();
    for br in crate::gen::brushes::BRUSHES.iter().filter(|x| x.map == map) {
        let sh = shader(&mut b, &format!("textures/sheet/{}", br.material), br.surface as i32, br.contents as i32);
        let first_side = b.brush_sides.len() as i32;
        for axis in 0..3 {
            for (sign, d) in [(-1.0f32, -br.mins[axis]), (1.0, br.maxs[axis])] {
                let mut n = [0.0f32; 3];
                n[axis] = sign;
                b.planes.push([n[0], n[1], n[2], d]);
                b.brush_sides.push(DBrushSide { plane: b.planes.len() as i32 - 1, shader: sh });
            }
        }
        if let Some(c) = br.cut {
            b.planes.push(c);
            b.brush_sides.push(DBrushSide { plane: b.planes.len() as i32 - 1, shader: sh });
        }
        b.brushes.push(DBrush { first_side, num_sides: b.brush_sides.len() as i32 - first_side, shader: sh });
        leaf_brushes.push(b.brushes.len() as i32 - 1);
        // drawable faces
        let planes: Vec<[f64; 4]> = b.brush_sides[first_side as usize..].iter().map(|s| b.planes[s.plane as usize].map(|x| x as f64)).collect();
        for (poly, n) in brush_windings(&planes) {
            let first_vert = b.verts.len() as i32;
            let first_index = b.indexes.len() as i32;
            // planar texture projection along the face's major axis (q3map TextureAxisFromPlane)
            let major = (0..3).max_by(|&x, &y| n[x].abs().partial_cmp(&n[y].abs()).unwrap()).unwrap();
            let (ua, va) = match major { 0 => (1, 2), 1 => (0, 2), _ => (0, 1) };
            for p in &poly {
                let xyz = [p[0] as f32, p[1] as f32, p[2] as f32];
                b.verts.push(DrawVert { xyz, st: [p[ua] as f32 / 64.0, -p[va] as f32 / 64.0], lightmap: [0.0; 2], normal: n.map(|x| x as f32), color: [128, 128, 128, 255] });
            }
            for t in 1..poly.len() as i32 - 1 {
                b.indexes.extend_from_slice(&[0, t, t + 1]);
            }
            b.surfaces.push(DSurface { shader: sh, fog: -1, surface_type: MST_PLANAR, first_vert, num_verts: poly.len() as i32, first_index, num_indexes: b.indexes.len() as i32 - first_index, lightmap_num: -1, ..Default::default() });
        }
    }
    b.planes.push([1.0, 0.0, 0.0, -65536.0]);
    b.nodes.push(DNode { plane: b.planes.len() as i32 - 1, children: [-1, -1], mins: [-65536; 3], maxs: [65536; 3] });
    b.leafs.push(DLeaf { cluster: 0, area: 0, mins: [-65536; 3], maxs: [65536; 3], first_leaf_surface: 0, num_leaf_surfaces: 0, first_leaf_brush: 0, num_leaf_brushes: leaf_brushes.len() as i32 });
    b.leaf_brushes = leaf_brushes;
    let nbrush = b.brushes.len() as i32;
    b.models.push(DModel { mins: [-65536.0; 3], maxs: [65536.0; 3], first_surface: 0, num_surfaces: b.surfaces.len() as i32, first_brush: 0, num_brushes: nbrush });
    // entities
    let md = map.def();
    let mut ents = format!("{{\n\"classname\" \"worldspawn\"\n\"gravity\" \"{}\"\n\"alienBuildPoints\" \"{}\"\n\"humanBuildPoints\" \"{}\"\n}}\n", md.gravity, md.alien_bp, md.human_bp);
    for me in crate::gen::map_entities::MAP_ENTITIES.iter().filter(|e| e.map == map) {
        use crate::gen::enums::{EntityKind, Team};
        let class = match (me.kind, me.team, me.buildable) {
            (EntityKind::Buildable, _, Some(bd)) => bd.cold().entity_name.to_string(),
            (EntityKind::PlayerStart, _, _) => "info_player_intermission".into(),
            (EntityKind::Intermission, Team::Aliens, _) => "info_alien_intermission".into(),
            (EntityKind::Intermission, Team::Humans, _) => "info_human_intermission".into(),
            _ => continue,
        };
        ents.push_str(&format!("{{\n\"classname\" \"{class}\"\n\"origin\" \"{} {} {}\"\n\"angle\" \"{}\"\n}}\n", me.origin[0], me.origin[1], me.origin[2], me.yaw));
    }
    b.entities = ents;
    b
}

fn dot64(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Convex face windings of a brush given its planes (n.x <= d inside):
/// a huge quad on each plane clipped by every other plane (q3map BaseWindingForPlane + ChopWinding).
/// Windings are clockwise seen from outside, like q3map output.
pub fn brush_windings(planes: &[[f64; 4]]) -> Vec<(Vec<[f64; 3]>, [f64; 3])> {
    let mut out = Vec::new();
    for (i, pl) in planes.iter().enumerate() {
        let n = [pl[0], pl[1], pl[2]];
        let a = (0..3).max_by(|&x, &y| n[x].abs().partial_cmp(&n[y].abs()).unwrap()).unwrap();
        let mut up = [0.0; 3];
        if a == 2 { up[0] = 1.0 } else { up[2] = 1.0 }
        let v = dot64(&up, &n);
        for k in 0..3 {
            up[k] -= v * n[k];
        }
        let l = dot64(&up, &up).sqrt();
        up.iter_mut().for_each(|x| *x /= l);
        let right = [up[1] * n[2] - up[2] * n[1], up[2] * n[0] - up[0] * n[2], up[0] * n[1] - up[1] * n[0]];
        let org = [n[0] * pl[3], n[1] * pl[3], n[2] * pl[3]];
        let big = 131072.0;
        let mk = |s: f64, t: f64| [org[0] + right[0] * s + up[0] * t, org[1] + right[1] * s + up[1] * t, org[2] + right[2] * s + up[2] * t];
        let mut poly = vec![mk(-big, big), mk(big, big), mk(big, -big), mk(-big, -big)];
        for (j, o) in planes.iter().enumerate() {
            if i == j || poly.len() < 3 {
                continue;
            }
            let on = [o[0], o[1], o[2]];
            let mut next = Vec::new();
            let eps = 0.01;
            for k in 0..poly.len() {
                let p = poly[k];
                let q = poly[(k + 1) % poly.len()];
                let dp = dot64(&p, &on) - o[3];
                let dq = dot64(&q, &on) - o[3];
                if dp <= eps {
                    next.push(p);
                }
                if (dp < -eps && dq > eps) || (dp > eps && dq < -eps) {
                    let t = dp / (dp - dq);
                    next.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t]);
                }
            }
            poly = next;
        }
        if poly.len() >= 3 {
            out.push((poly, n));
        }
    }
    out
}
