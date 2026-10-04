//! Ragdoll corpses (port addition, enhanced mode only).
//!
//! The player models are vertex-animated MD3s: they have no skeleton. The
//! skeletons come from `res://data/rigs.txt` (tools/rig_md3.py), recovered
//! from each model's own animation: every vertex belongs to one rigid group,
//! the groups form a tree, and each group has a joint and a swing limit.
//!
//! At the moment of death the corpse's current frame is taken as the
//! ragdoll's rest pose: the mesh is rebuilt in world space exactly as it is
//! drawn, every bone is placed by the rigid transform that carries its
//! vertices from the rig's rest frame to the drawn frame (Horn's quaternion
//! method), and Godot physics (Jolt) takes over: one capsule per bone, cone
//! joints at the rig's joints, an impulse from the killing hit.
//!
//! Nothing here touches the game state: the server's corpse is drawn as a
//! ragdoll on this screen only, and the vanilla mode (and F10 back to it)
//! draws the original corpse, which stays in place, hidden, all along.

use std::collections::HashMap;

/// One bone of a rig: its parent, the MD3 part its vertices are in, its
/// joint (MD3 space of the rest frames) and its swing limit (degrees).
#[derive(Clone, Debug, PartialEq)]
pub struct BoneDef {
    pub name: String,
    pub parent: Option<usize>,
    pub part: String,
    pub joint: [f32; 3],
    pub limit: f32,
}

/// A model's skeleton: rest frame per part, bones (parents before their
/// children) and the bone of each vertex of each surface, per part.
#[derive(Clone, Debug, Default)]
pub struct Rig {
    pub rest: HashMap<String, usize>,
    pub bones: Vec<BoneDef>,
    pub labels: HashMap<String, Vec<Vec<u16>>>,
}

/// `rigs.txt` -> model directory -> rig. Bones are reordered so that every
/// parent comes before its children (labels follow).
pub fn parse_rigs(text: &str) -> HashMap<String, Rig> {
    let mut out = HashMap::new();
    let mut cur: Option<(String, Rig)> = None;
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first().copied() {
            Some("model") if w.len() >= 2 => cur = Some((w[1].to_string(), Rig::default())),
            Some("rest") if w.len() >= 3 => {
                if let Some((_, r)) = cur.as_mut() {
                    r.rest.insert(w[1].to_string(), w[2].parse().unwrap_or(0));
                }
            }
            Some("bone") if w.len() >= 8 => {
                if let Some((_, r)) = cur.as_mut() {
                    let f = |s: &str| s.parse::<f32>().unwrap_or(0.0);
                    r.bones.push(BoneDef { name: w[1].to_string(), parent: w[2].parse().ok(), part: w[3].to_string(), joint: [f(w[4]), f(w[5]), f(w[6])], limit: f(w[7]) });
                }
            }
            Some("labels") if w.len() >= 3 => {
                if let Some((_, r)) = cur.as_mut() {
                    let si: usize = w[2].parse().unwrap_or(0);
                    let v = r.labels.entry(w[1].to_string()).or_default();
                    if v.len() <= si {
                        v.resize(si + 1, Vec::new());
                    }
                    v[si] = w[3..].iter().map(|x| x.parse().unwrap_or(0)).collect();
                }
            }
            Some("end") => {
                if let Some((name, r)) = cur.take() {
                    out.insert(name, topo(r));
                }
            }
            _ => {}
        }
    }
    out
}

/// Parents before children; labels renumbered.
fn topo(mut r: Rig) -> Rig {
    let n = r.bones.len();
    let mut order = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    while order.len() < n {
        let before = order.len();
        for i in 0..n {
            if !placed[i] && r.bones[i].parent.map(|p| p < n && placed[p]).unwrap_or(true) {
                placed[i] = true;
                order.push(i);
            }
        }
        if order.len() == before {
            // a cycle (a malformed file): the rest become roots
            for i in 0..n {
                if !placed[i] {
                    placed[i] = true;
                    r.bones[i].parent = None;
                    order.push(i);
                }
            }
        }
    }
    let mut new_of = vec![0usize; n];
    for (new, &old) in order.iter().enumerate() {
        new_of[old] = new;
    }
    let bones: Vec<BoneDef> = order.iter().map(|&o| {
        let mut b = r.bones[o].clone();
        b.parent = b.parent.filter(|&p| p < n).map(|p| new_of[p]);
        b
    }).collect();
    for surfs in r.labels.values_mut() {
        for s in surfs.iter_mut() {
            for l in s.iter_mut() {
                *l = new_of.get(*l as usize).copied().unwrap_or(0) as u16;
            }
        }
    }
    r.bones = bones;
    r
}

pub type V3 = [f32; 3];
/// Row-major 3x3.
pub type M3 = [[f32; 3]; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
pub fn mul(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
pub fn centroid(p: &[V3]) -> V3 {
    let mut c = [0.0; 3];
    for q in p {
        c = add(c, *q);
    }
    scale(c, 1.0 / p.len().max(1) as f32)
}

/// Eigen-decomposition of a symmetric NxN matrix (cyclic Jacobi):
/// (eigenvalues, eigenvectors as columns).
fn jacobi<const N: usize>(mut a: [[f64; N]; N]) -> ([f64; N], [[f64; N]; N]) {
    let mut v = [[0.0; N]; N];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..64 {
        let mut off = 0.0;
        for i in 0..N {
            for j in i + 1..N {
                off += a[i][j] * a[i][j];
            }
        }
        if off < 1e-22 {
            break;
        }
        for p in 0..N {
            for q in p + 1..N {
                if a[p][q].abs() < 1e-30 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..N {
                    let akp = a[k][p];
                    let akq = a[k][q];
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..N {
                    let apk = a[p][k];
                    let aqk = a[q][k];
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let vkp = row[p];
                    let vkq = row[q];
                    row[p] = c * vkp - s * vkq;
                    row[q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut e = [0.0; N];
    for (i, x) in e.iter_mut().enumerate() {
        *x = a[i][i];
    }
    (e, v)
}

/// The rotation and translation carrying `p` onto `q` with least squares
/// error (Horn 1987: the largest eigenvector of a 4x4 matrix built from the
/// cross-covariance is the rotation's quaternion). q ~ R p + t.
pub fn kabsch(p: &[V3], q: &[V3]) -> (M3, V3) {
    let cp = centroid(p);
    let cq = centroid(q);
    let mut s = [[0.0f64; 3]; 3];
    for (a, b) in p.iter().zip(q) {
        let a = sub(*a, cp);
        let b = sub(*b, cq);
        for i in 0..3 {
            for j in 0..3 {
                s[i][j] += a[i] as f64 * b[j] as f64;
            }
        }
    }
    let (sxx, sxy, sxz, syx, syy, syz, szx, szy, szz) = (s[0][0], s[0][1], s[0][2], s[1][0], s[1][1], s[1][2], s[2][0], s[2][1], s[2][2]);
    let n = [
        [sxx + syy + szz, syz - szy, szx - sxz, sxy - syx],
        [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
        [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
        [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
    ];
    let (e, v) = jacobi::<4>(n);
    let mut best = 0;
    for i in 1..4 {
        if e[i] > e[best] {
            best = i;
        }
    }
    let (w, x, y, z) = (v[0][best], v[1][best], v[2][best], v[3][best]);
    let r64 = [
        [w * w + x * x - y * y - z * z, 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), w * w - x * x + y * y - z * z, 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), w * w - x * x - y * y + z * z],
    ];
    let r = r64.map(|row| row.map(|x| x as f32));
    let t = sub(cq, mul(&r, cp));
    (r, t)
}

/// A capsule around points given in a bone's frame: (centre, axis (unit),
/// radius, total height). The axis is the points' principal direction.
pub fn fit_capsule(p: &[V3]) -> (V3, V3, f32, f32) {
    if p.is_empty() {
        return ([0.0; 3], [0.0, 1.0, 0.0], 0.05, 0.1);
    }
    let c = centroid(p);
    let mut cov = [[0.0f64; 3]; 3];
    for q in p {
        let d = sub(*q, c);
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] as f64 * d[j] as f64;
            }
        }
    }
    let (e, v) = jacobi::<3>(cov);
    let mut best = 0;
    for i in 1..3 {
        if e[i] > e[best] {
            best = i;
        }
    }
    let axis = [v[0][best] as f32, v[1][best] as f32, v[2][best] as f32];
    let (mut lo, mut hi, mut rad) = (f32::MAX, f32::MIN, 0.0f32);
    let mut radii: Vec<f32> = Vec::with_capacity(p.len());
    for q in p {
        let d = sub(*q, c);
        let along = dot(d, axis);
        lo = lo.min(along);
        hi = hi.max(along);
        radii.push(len(sub(d, scale(axis, along))));
    }
    radii.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // the body, not its spikes: the 70th percentile distance from the axis
    if !radii.is_empty() {
        rad = radii[(radii.len() * 7 / 10).min(radii.len() - 1)];
    }
    let rad = rad.max(0.03);
    let centre = add(c, scale(axis, (lo + hi) * 0.5));
    let height = (hi - lo).max(2.0 * rad);
    (centre, axis, rad, height)
}

/// Skin weights: each vertex follows its own bone, and near a joint it is
/// shared with the bone on the other side (half at the joint, none at
/// `reach` from it) so the mesh bends there instead of tearing.
/// `joints[b]` is bone b's joint (world), `reach[b]` its blend radius.
pub fn skin_weights(pos: V3, bone: usize, parent: &[Option<usize>], joints: &[V3], reach: &[f32]) -> [(usize, f32); 4] {
    let mut w: Vec<(usize, f32)> = vec![(bone, 1.0)];
    // towards the parent, across this bone's own joint
    if let Some(p) = parent[bone] {
        let d = len(sub(pos, joints[bone]));
        if d < reach[bone] {
            w.push((p, 0.5 * (1.0 - d / reach[bone])));
        }
    }
    // towards each child, across the child's joint
    for (c, par) in parent.iter().enumerate() {
        if *par == Some(bone) {
            let d = len(sub(pos, joints[c]));
            if d < reach[c] {
                w.push((c, 0.5 * (1.0 - d / reach[c])));
            }
        }
    }
    let others: f32 = w[1..].iter().map(|x| x.1).sum();
    if others > 0.5 {
        let k = 0.5 / others;
        for x in w[1..].iter_mut() {
            x.1 *= k;
        }
    }
    let own: f32 = 1.0 - w[1..].iter().map(|x| x.1).sum::<f32>();
    w[0].1 = own;
    w.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let mut out = [(0usize, 0.0f32); 4];
    let total: f32 = w.iter().take(4).map(|x| x.1).sum();
    for (i, x) in w.iter().take(4).enumerate() {
        out[i] = (x.0, x.1 / total.max(1e-6));
    }
    out
}

/// The push of a death (metres per second the corpse is thrown at, along
/// the hit), by means of death: light guns nudge, heavy weapons and
/// explosions throw. None: the body just collapses (falls, drowning...).
pub fn death_push(m: trem_sim::gen::enums::MeansOfDeath) -> Option<f32> {
    use trem_sim::gen::enums::MeansOfDeath as M;
    Some(match m {
        M::Blaster | M::Machinegun | M::Lasgun | M::Prifle | M::Chaingun | M::Mgturret => 3.5,
        M::Shotgun | M::Mdriver | M::Painsaw | M::Teslagen => 6.0,
        M::Lcannon | M::LcannonSplash => 12.0,
        M::Grenade | M::Reactor => 15.0,
        M::Flamer | M::FlamerSplash => 2.5,
        M::Level0Bite | M::Level1Claw | M::AbuilderClaw | M::Level2Claw | M::Level2Zap | M::Swarm | M::Slowblob | M::Atube | M::Hspawn | M::Aspawn | M::Overmind => 4.5,
        M::Level3Claw | M::Level3Bounceball => 7.0,
        M::Level3Pounce | M::Level4Claw => 10.0,
        M::Level4Trample | M::Level4Crush => 15.0,
        _ => return None,
    })
}

/// The fastest a body is thrown (m/s, Godot space).
pub const MAX_DEATH_SPEED: f32 = 30.0;

/// The velocity a ragdoll starts with (m/s, Godot space): the body's own
/// momentum (what it had while alive, or on the frame it died if the
/// server's knockback made that faster) plus the push of the killing hit.
pub fn death_velocity(alive: [f32; 3], dying: [f32; 3], push: [f32; 3]) -> [f32; 3] {
    let m = if len(dying) > len(alive) { dying } else { alive };
    let v = add(m, push);
    let l = len(v);
    if l > MAX_DEATH_SPEED { scale(v, MAX_DEATH_SPEED / l) } else { v }
}

/// A class's mass for the ragdoll (kg): what the bodies feel like when hit.
pub fn class_mass(model_dir: &str) -> f32 {
    match model_dir {
        "level0" => 15.0,
        "level1" | "level1upg" => 40.0,
        "builder" | "builderupg" => 45.0,
        "level2" | "level2upg" => 60.0,
        "level3" | "level3upg" => 150.0,
        "level4" => 400.0,
        "human_bsuit" => 140.0,
        _ => 80.0,
    }
}

// =====================================================================
// the Godot side
// =====================================================================

use crate::geom::{q2g, q2g_dir};
use crate::models::{Library, ModelInst};
use godot::classes::mesh::{ArrayType, PrimitiveType};
use godot::classes::physical_bone_3d::JointType;
use godot::classes::{ArrayMesh, CapsuleShape3D, CollisionShape3D, ConcavePolygonShape3D, MeshInstance3D, Node3D, PhysicalBone3D, PhysicalBoneSimulator3D, ShaderMaterial, Skeleton3D, StaticBody3D};
use godot::prelude::*;

/// Ragdoll bodies collide with the world only (not with each other).
pub const LAYER_WORLD: u32 = 1;
pub const LAYER_RAGDOLL: u32 = 2;
/// The game's gravity (800 units/s^2 = 25 m/s^2) over Godot's default 9.8.
const GRAVITY_SCALE: f32 = 25.0 / 9.8;

fn v3(v: Vector3) -> V3 {
    [v.x, v.y, v.z]
}
fn gv(v: V3) -> Vector3 {
    Vector3::new(v[0], v[1], v[2])
}

/// A basis whose Y axis is `y` (unit).
fn basis_y(y: V3) -> Basis {
    let y = gv(y).normalized();
    let helper = if y.x.abs() < 0.9 { Vector3::RIGHT } else { Vector3::FORWARD };
    let x = helper.cross(y).normalized();
    let z = x.cross(y).normalized();
    Basis::from_cols(x, y, z)
}
/// A basis whose X axis is `x` (unit): the cone joint's twist axis.
fn basis_x(x: V3) -> Basis {
    let x = gv(x).normalized();
    let helper = if x.y.abs() < 0.9 { Vector3::UP } else { Vector3::RIGHT };
    let z = x.cross(helper).normalized();
    let y = z.cross(x).normalized();
    Basis::from_cols(x, y, z)
}

/// The world collider ragdolls land on: the map's drawn opaque triangles
/// (the acoustic mesh, Godot space).
pub fn world_collider(verts: &[V3], tris: &[[i32; 3]]) -> Gd<StaticBody3D> {
    let mut faces = PackedVector3Array::new();
    for t in tris {
        for &i in t {
            faces.push(gv(verts[i as usize]));
        }
    }
    let mut shape = ConcavePolygonShape3D::new_gd();
    shape.set_faces(&faces);
    shape.set_backface_collision_enabled(true);
    let mut cs = CollisionShape3D::new_alloc();
    cs.set_shape(&shape);
    let mut body = StaticBody3D::new_alloc();
    body.set_name("RagdollWorld");
    body.set_collision_layer(LAYER_WORLD);
    body.set_collision_mask(0);
    body.add_child(&cs);
    body
}

/// A ragdoll in the scene.
pub struct Ragdoll {
    /// the entity it draws (the dying player, then its corpse)
    pub ent: usize,
    /// drawing the server's corpse entity (else the dying player's)
    pub corpse: bool,
    pub model_dir: String,
    pub root: Gd<Node3D>,
    skel: Gd<Skeleton3D>,
    sim: Gd<PhysicalBoneSimulator3D>,
    bones: Vec<Gd<PhysicalBone3D>>,
    mats: Vec<Gd<ShaderMaterial>>,
    started: bool,
    /// the push of the killing hit (Godot space, m/s), applied at start
    push: Option<Vector3>,
    pub born: f64,
    /// where it was built (its root bone, Godot space): its position until it is in the scene
    start_pos: Vector3,
    /// the corpse entity's first height (Quake units): it sinks below it
    pub corpse_z: Option<f32>,
    pub sunk: f32,
    /// where the server has the body (Godot space): the ragdoll travels
    /// away from it, the corpse entity that takes over spawns at it
    pub server_pos: Vector3,
}

impl Ragdoll {
    /// Build a ragdoll of `parts` (the actor's body parts as drawn now).
    /// Not yet simulating: add `root` to the scene, then `start` it.
    pub fn build(lib: &Library, rig: &Rig, model_dir: &str, model_scale: f32, parts: &[(&str, &ModelInst)], ent: usize, push: Option<Vector3>, now: f64) -> Option<Ragdoll> {
        let nb = rig.bones.len();
        if nb == 0 || parts.is_empty() {
            return None;
        }
        // the rig's rest space: each part's rest frame, the human upper and
        // head placed by the lower / upper parts' tags of those frames
        let part_of = |name: &str| parts.iter().find(|(n, _)| *n == name).map(|(_, m)| *m);
        let rest_frame = |name: &str| rig.rest.get(name).copied().unwrap_or(0);
        let tag_at = |m: &ModelInst, tag: &str, f: usize| -> Option<(V3, M3)> {
            let md = &m.res.md3;
            let ti = md.tags.first()?.iter().position(|t| t.name == tag)?;
            let t = &md.tags.get(f.min(md.num_frames.saturating_sub(1)))?[ti];
            Some((t.origin, t.axis))
        };
        // Q3 tag: p' = origin + p.x axis[0] + p.y axis[1] + p.z axis[2]
        let apply = |p: V3, (o, ax): (V3, M3)| -> V3 { add(o, add(scale(ax[0], p[0]), add(scale(ax[1], p[1]), scale(ax[2], p[2])))) };
        let torso = part_of("lower").and_then(|l| tag_at(l, "tag_torso", rest_frame("lower")));
        let head = part_of("upper").and_then(|u| tag_at(u, "tag_head", rest_frame("upper")));
        let to_rig = |part: &str, p: V3| -> V3 {
            match (part, torso, head) {
                ("upper", Some(t), _) => apply(p, t),
                ("head", Some(t), Some(h)) => apply(apply(p, h), t),
                _ => p,
            }
        };
        // every drawn vertex: its bone, where it is now, its normal now
        struct Vx {
            bone: usize,
            world: V3,
            normal: V3,
        }
        let mut surfaces: Vec<(Vec<Vx>, Vec<[f32; 2]>, Vec<[u32; 3]>, Gd<ShaderMaterial>)> = Vec::new();
        let mut per_bone: Vec<(Vec<V3>, Vec<V3>)> = vec![(Vec::new(), Vec::new()); nb];
        for (part, m) in parts {
            let Some(labels) = rig.labels.get(*part) else { continue };
            let md = &m.res.md3;
            let rf = rest_frame(part).min(md.num_frames.saturating_sub(1));
            let (old, new, back) = m.frames();
            let (old, new) = (old.max(0) as usize, new.max(0) as usize);
            let gt = if m.root.is_inside_tree() { m.root.get_global_transform() } else { m.root.get_transform() };
            let drawn: HashMap<usize, Gd<ShaderMaterial>> = m.drawn().map(|(i, mat)| (i, mat.clone())).collect();
            for (si, s) in md.surfaces.iter().enumerate() {
                let Some(lab) = labels.get(si) else { continue };
                if s.xyz.is_empty() || lab.len() != s.xyz[0].len() {
                    continue;
                }
                let (fo, fnw) = (old.min(s.xyz.len() - 1), new.min(s.xyz.len() - 1));
                let mut vx = Vec::with_capacity(lab.len());
                for (k, &b) in lab.iter().enumerate() {
                    let b = (b as usize).min(nb - 1);
                    let (a, c) = (s.xyz[fo][k], s.xyz[fnw][k]);
                    let cur = [c[0] + (a[0] - c[0]) * back, c[1] + (a[1] - c[1]) * back, c[2] + (a[2] - c[2]) * back];
                    let (na, nc) = (s.normals[fo][k], s.normals[fnw][k]);
                    let ncur = [nc[0] + (na[0] - nc[0]) * back, nc[1] + (na[1] - nc[1]) * back, nc[2] + (na[2] - nc[2]) * back];
                    let world = v3(gt * q2g(&cur));
                    let normal = v3((gt.basis * q2g_dir(&ncur)).normalized());
                    let rest = scale(v3(q2g(&to_rig(part, s.xyz[rf][k]))), model_scale);
                    per_bone[b].0.push(rest);
                    per_bone[b].1.push(world);
                    vx.push(Vx { bone: b, world, normal });
                }
                if let Some(mat) = drawn.get(&si) {
                    surfaces.push((vx, s.st.clone(), s.triangles.clone(), lib.ragdoll_material(mat)));
                }
            }
        }
        if surfaces.is_empty() {
            return None;
        }
        // each bone where the drawn frame has it (a bone with too few
        // vertices to fit follows its parent)
        let mut xf: Vec<Option<(M3, V3)>> = per_bone.iter().map(|(r, w)| (r.len() >= 3).then(|| kabsch(r, w))).collect();
        for b in 0..nb {
            if xf[b].is_none() {
                xf[b] = rig.bones[b].parent.and_then(|p| xf[p]).or(Some(([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], [0.0; 3])));
            }
        }
        let xf: Vec<(M3, V3)> = xf.into_iter().map(|x| x.unwrap()).collect();
        let joints: Vec<V3> = (0..nb).map(|b| {
            if rig.bones[b].parent.is_none() && !per_bone[b].1.is_empty() {
                centroid(&per_bone[b].1)
            } else {
                let j = scale(v3(q2g(&rig.bones[b].joint)), model_scale);
                add(mul(&xf[b].0, j), xf[b].1)
            }
        }).collect();
        let size: Vec<f32> = (0..nb).map(|b| per_bone[b].1.iter().map(|w| len(sub(*w, joints[b]))).fold(0.0, f32::max)).collect();
        let reach: Vec<f32> = size.iter().map(|s| (s * 0.3).clamp(0.03, 0.4)).collect();
        let parents: Vec<Option<usize>> = rig.bones.iter().map(|b| b.parent).collect();
        // the skeleton: rest = the drawn pose
        let mut skel = Skeleton3D::new_alloc();
        skel.set_name("Skeleton");
        let global: Vec<Transform3D> = (0..nb).map(|b| {
            let r = xf[b].0;
            Transform3D::new(Basis::from_rows(gv(r[0]), gv(r[1]), gv(r[2])), gv(joints[b]))
        }).collect();
        for (b, bd) in rig.bones.iter().enumerate() {
            skel.add_bone(&format!("{}_{}", bd.name, b));
        }
        for b in 0..nb {
            let local = match parents[b] {
                Some(p) => {
                    skel.set_bone_parent(b as i32, p as i32);
                    global[p].affine_inverse() * global[b]
                }
                None => global[b],
            };
            skel.set_bone_rest(b as i32, local);
        }
        skel.reset_bone_poses();
        // the mesh, as drawn, skinned to the bones
        let mut mesh = ArrayMesh::new_gd();
        let mut mats = Vec::new();
        for (vx, st, tris, mat) in &surfaces {
            let verts: PackedVector3Array = vx.iter().map(|v| gv(v.world)).collect();
            let normals: PackedVector3Array = vx.iter().map(|v| gv(v.normal)).collect();
            let uvs: PackedVector2Array = st.iter().map(|t| Vector2::new(t[0], t[1])).collect();
            let idx: PackedInt32Array = tris.iter().flat_map(|t| [t[0] as i32, t[1] as i32, t[2] as i32]).collect();
            let mut bones = PackedInt32Array::new();
            let mut weights = PackedFloat32Array::new();
            for v in vx {
                for (b, w) in skin_weights(v.world, v.bone, &parents, &joints, &reach) {
                    bones.push(b as i32);
                    weights.push(w);
                }
            }
            let mut arrays = VarArray::new();
            arrays.resize(ArrayType::MAX.ord() as usize, &Variant::nil());
            arrays.set(ArrayType::VERTEX.ord() as usize, &verts.to_variant());
            arrays.set(ArrayType::NORMAL.ord() as usize, &normals.to_variant());
            arrays.set(ArrayType::TEX_UV.ord() as usize, &uvs.to_variant());
            arrays.set(ArrayType::BONES.ord() as usize, &bones.to_variant());
            arrays.set(ArrayType::WEIGHTS.ord() as usize, &weights.to_variant());
            arrays.set(ArrayType::INDEX.ord() as usize, &idx.to_variant());
            mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);
            mats.push(mat.clone());
        }
        let mut mi = MeshInstance3D::new_alloc();
        mi.set_mesh(&mesh);
        for (i, m) in mats.iter().enumerate() {
            mi.set_surface_override_material(i as i32, m);
        }
        // it moves away from where it was built
        mi.set_extra_cull_margin(4.0);
        skel.add_child(&mi);
        if let Some(skin) = skel.create_skin_from_rest_transforms() {
            mi.set_skin(&skin);
        }
        mi.set_skeleton_path(&NodePath::from(".."));
        // one body per bone: a capsule around its vertices, a cone joint
        // to its parent at the rig's joint, a share of the class's mass
        let mut sim = PhysicalBoneSimulator3D::new_alloc();
        sim.set_name("Physics");
        skel.add_child(&sim);
        let total: usize = per_bone.iter().map(|(r, _)| r.len()).sum::<usize>().max(1);
        let mass = class_mass(model_dir);
        let mut pbs = Vec::new();
        for b in 0..nb {
            let inv = global[b].affine_inverse();
            let local: Vec<V3> = per_bone[b].1.iter().map(|w| v3(inv * gv(*w))).collect();
            let (centre, axis, rad, height) = fit_capsule(&local);
            let mut cap = CapsuleShape3D::new_gd();
            cap.set_radius(rad);
            cap.set_height(height.max(2.0 * rad + 0.01));
            let mut cs = CollisionShape3D::new_alloc();
            cs.set_shape(&cap);
            let body_offset = Transform3D::new(basis_y(axis), gv(centre));
            let mut pb = PhysicalBone3D::new_alloc();
            pb.set("bone_name", &skel.get_bone_name(b as i32).to_variant());
            pb.set_body_offset(body_offset);
            // the joint at the bone's origin (its rig joint), twisting about
            // the direction into the bone
            let into = if len(centre) > 1e-4 { scale(centre, 1.0 / len(centre)) } else { [1.0, 0.0, 0.0] };
            pb.set_joint_offset(body_offset.affine_inverse() * Transform3D::new(basis_x(into), Vector3::ZERO));
            if parents[b].is_some() {
                pb.set_joint_type(JointType::CONE);
                pb.set("joint_constraints/swing_span", &rig.bones[b].limit.clamp(15.0, 110.0).to_variant());
                pb.set("joint_constraints/twist_span", &25.0f32.to_variant());
            } else {
                pb.set_joint_type(JointType::NONE);
            }
            pb.set_mass((mass * per_bone[b].0.len() as f32 / total as f32).max(0.5));
            pb.set_friction(0.45);
            pb.set_bounce(0.05);
            pb.set_linear_damp(0.05);
            pb.set_angular_damp(1.5);
            pb.set_gravity_scale(GRAVITY_SCALE);
            pb.set_collision_layer(LAYER_RAGDOLL);
            pb.set_collision_mask(LAYER_WORLD);
            pb.add_child(&cs);
            sim.add_child(&pb);
            pbs.push(pb);
        }
        let mut root = Node3D::new_alloc();
        root.set_name(&format!("Ragdoll{ent}"));
        root.add_child(&skel);
        let start_pos = gv(joints[0]);
        Some(Ragdoll { ent, corpse: false, model_dir: model_dir.to_string(), root, skel, sim, bones: pbs, mats, started: false, push, born: now, start_pos, corpse_z: None, sunk: 0.0, server_pos: start_pos })
    }

    /// Start the simulation (once `root` is in the scene) and give it the
    /// killing hit's push.
    pub fn start(&mut self) {
        if self.started || !self.root.is_inside_tree() {
            return;
        }
        self.started = true;
        self.sim.physical_bones_start_simulation();
        if let Some(v) = self.push.take() {
            for pb in self.bones.iter_mut() {
                let m = pb.get_mass();
                pb.apply_central_impulse(v * m);
            }
        }
    }

    pub fn started(&self) -> bool {
        self.started
    }
    pub fn bone_count(&self) -> usize {
        self.bones.len()
    }

    /// Where the body is now (its root bone, Godot space).
    pub fn position(&self) -> Vector3 {
        match self.bones.first() {
            Some(b) if b.is_inside_tree() => b.get_global_position(),
            _ => self.start_pos,
        }
    }

    pub fn set_light(&mut self, amb: [f32; 3], dcol: [f32; 3], ldir: Vector3) {
        let a = Vector3::new(amb[0], amb[1], amb[2]).to_variant();
        let d = Vector3::new(dcol[0], dcol[1], dcol[2]).to_variant();
        let l = ldir.to_variant();
        for m in self.mats.iter_mut() {
            m.set_shader_parameter("amb", &a);
            m.set_shader_parameter("dcol", &d);
            m.set_shader_parameter("ldir", &l);
        }
    }

    pub fn set_visible(&mut self, v: bool) {
        self.root.set_visible(v);
    }

    /// The corpse sinks into the floor (the server lowers it before freeing
    /// it): the ragdoll stops moving and goes down by as much.
    pub fn sink_to(&mut self, below: f32) {
        if below <= self.sunk {
            return;
        }
        if self.sunk == 0.0 {
            // freeze it where it lies: the bones keep their last pose
            let poses: Vec<Transform3D> = (0..self.skel.get_bone_count()).map(|b| self.skel.get_bone_global_pose(b)).collect();
            self.sim.physical_bones_stop_simulation();
            for (b, g) in poses.iter().enumerate() {
                let local = match self.skel.get_bone_parent(b as i32) {
                    p if p >= 0 => poses[p as usize].affine_inverse() * *g,
                    _ => *g,
                };
                self.skel.set_bone_pose(b as i32, local);
            }
        }
        self.sunk = below;
        let mut p = self.root.get_position();
        p.y = -below;
        self.root.set_position(p);
    }

    /// A push that arrived after the body was built (the obituary came
    /// late): applied if the body has only just fallen.
    pub fn push_late(&mut self, v: Vector3, now: f64) {
        if now - self.born > 0.5 {
            return;
        }
        if self.started {
            for pb in self.bones.iter_mut() {
                let m = pb.get_mass();
                pb.apply_central_impulse(v * m);
            }
        } else {
            self.push = Some(v);
        }
    }

    pub fn free(mut self) {
        self.root.queue_free();
    }
    /// `free` for a ragdoll still in a list (`retain_mut`).
    pub fn free_in_place(&mut self) {
        self.root.queue_free();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# c\nmodel m\nrest nonseg 3\nbone a 2 nonseg 1 0 0 40\nbone b - nonseg 0 0 0 0\nbone c 1 nonseg 0 1 0 30\nlabels nonseg 0 0 1 2 2\nend\n";

    #[test]
    fn rigs_parse_and_order_parents_first() {
        let r = &parse_rigs(SAMPLE)["m"];
        assert_eq!(r.rest["nonseg"], 3);
        let names: Vec<&str> = r.bones.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["b", "c", "a"]);
        for (i, b) in r.bones.iter().enumerate() {
            assert!(b.parent.map(|p| p < i).unwrap_or(true));
        }
        // a(0)->2, b(1)->0, c(2)->1
        assert_eq!(r.labels["nonseg"][0], vec![2, 0, 1, 1]);
        assert_eq!(r.bones[2].parent, Some(1));
    }

    #[test]
    fn the_shipped_rigs_load() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../godot/data/rigs.txt");
        let Ok(t) = std::fs::read_to_string(&p) else { return };
        let rigs = parse_rigs(&t);
        for m in ["human_base", "human_bsuit", "builder", "level0", "level1", "level2", "level3", "level4"] {
            let r = rigs.get(m).unwrap_or_else(|| panic!("{m} missing"));
            assert!(r.bones.len() >= 8, "{m}");
            assert_eq!(r.bones.iter().filter(|b| b.parent.is_none()).count(), 1, "{m}: one root");
            for surfs in r.labels.values() {
                for s in surfs {
                    assert!(s.iter().all(|&l| (l as usize) < r.bones.len()));
                }
            }
        }
    }

    fn rot(axis: V3, ang: f32) -> M3 {
        let a = scale(axis, 1.0 / len(axis));
        let (s, c) = ang.sin_cos();
        let t = 1.0 - c;
        [
            [t * a[0] * a[0] + c, t * a[0] * a[1] - s * a[2], t * a[0] * a[2] + s * a[1]],
            [t * a[0] * a[1] + s * a[2], t * a[1] * a[1] + c, t * a[1] * a[2] - s * a[0]],
            [t * a[0] * a[2] - s * a[1], t * a[1] * a[2] + s * a[0], t * a[2] * a[2] + c],
        ]
    }

    #[test]
    fn kabsch_recovers_a_rigid_motion() {
        let p: Vec<V3> = vec![[0.0, 0.0, 0.0], [1.0, 0.2, -0.3], [-0.5, 1.4, 0.7], [0.3, -0.8, 1.1], [2.0, 0.5, 0.1]];
        for (axis, ang) in [([0.0, 0.0, 1.0], 0.3), ([1.0, 2.0, -1.0], 2.5), ([0.2, -1.0, 0.4], -1.2), ([1.0, 0.0, 0.0], 3.1)] {
            let r = rot(axis, ang);
            let t = [3.0, -1.0, 0.5];
            let q: Vec<V3> = p.iter().map(|x| add(mul(&r, *x), t)).collect();
            let (r2, t2) = kabsch(&p, &q);
            for (a, b) in p.iter().zip(&q) {
                let e = len(sub(add(mul(&r2, *a), t2), *b));
                assert!(e < 1e-3, "axis {axis:?} angle {ang}: error {e}");
            }
        }
    }

    #[test]
    fn capsule_follows_the_long_axis() {
        let p: Vec<V3> = (0..50).map(|i| [0.1 * ((i * 7 % 5) as f32 - 2.0) * 0.1, i as f32 * 0.04, 0.1 * ((i * 3 % 5) as f32 - 2.0) * 0.1]).collect();
        let (c, axis, r, h) = fit_capsule(&p);
        assert!(axis[1].abs() > 0.99, "{axis:?}");
        assert!((c[1] - 0.98).abs() < 0.05, "{c:?}");
        assert!(r < 0.1 && h > 1.8, "{r} {h}");
    }

    #[test]
    fn weights_blend_only_near_joints() {
        let parent = [None, Some(0)];
        let joints = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let reach = [0.3, 0.3];
        // far from the joint: all its own bone
        let w = skin_weights([0.0, 2.0, 0.0], 1, &parent, &joints, &reach);
        assert_eq!(w[0], (1, 1.0));
        // at the joint: half and half
        let w = skin_weights([0.0, 1.0, 0.0], 1, &parent, &joints, &reach);
        assert!((w[0].1 - 0.5).abs() < 1e-5 && (w[1].1 - 0.5).abs() < 1e-5);
        // the parent's side of the joint shares with the child
        let w = skin_weights([0.0, 0.9, 0.0], 0, &parent, &joints, &reach);
        assert_eq!(w[0].0, 0);
        assert!(w[1].0 == 1 && w[1].1 > 0.0 && w[1].1 < 0.5);
        let total: f32 = w.iter().map(|x| x.1).sum();
        assert!((total - 1.0).abs() < 1e-5);
    }

    #[test]
    fn heavy_weapons_throw_further() {
        use trem_sim::gen::enums::MeansOfDeath as M;
        assert!(death_push(M::Lcannon).unwrap() > death_push(M::Blaster).unwrap());
        assert!(death_push(M::Falling).is_none());
    }

    #[test]
    fn a_body_keeps_its_momentum_and_takes_the_hit() {
        // running at 10 m/s, hit sideways: it carries on and is pushed
        let v = death_velocity([10.0, 0.0, 0.0], [9.0, 0.0, 0.0], [0.0, 1.0, 3.0]);
        assert_eq!(v, [10.0, 1.0, 3.0]);
        // the server's knockback on the dying frame counts when it is faster
        let v = death_velocity([1.0, 0.0, 0.0], [0.0, 0.0, 8.0], [0.0, 0.0, 0.0]);
        assert_eq!(v, [0.0, 0.0, 8.0]);
        // never thrown absurdly far
        let v = death_velocity([100.0, 0.0, 0.0], [0.0; 3], [50.0, 0.0, 0.0]);
        assert!((len(v) - MAX_DEATH_SPEED).abs() < 1e-3);
    }
}
