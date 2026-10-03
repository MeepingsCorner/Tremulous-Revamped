//! The cgame's model and effect debugging tools: cg_view.c `CG_TestModel_f`,
//! `CG_TestGun_f`, `CG_TestModelNextFrame_f` / `PrevFrame_f` /
//! `NextSkin_f` / `PrevSkin_f`, `CG_AddTestModel`; cg_particles.c
//! `CG_TestPS_f` / `CG_DestroyTestPS_f` and cg_trails.c `CG_TestTS_f` /
//! `CG_DestroyTestTS_f` (effects.rs spawns them); cg_ents.c
//! `CG_DrawBoundingBox` / `CG_DrawBoxFace` (cg_drawBBOX) and cg_view.c
//! `CG_DrawSurfNormal` (cg_drawSurfNormal) as polys with the `outline`
//! shader; the cg_debugEvents print (trem_sim `bg::event_name`). `TestModel` is the pure state (unit tested); the
//! Godot instances live in `DevState`.

use crate::geom::q2g;
use crate::models::{model_basis, Library, ModelInst};
use godot::classes::{ImmediateMesh, MeshInstance3D, StandardMaterial3D};
use godot::prelude::*;
use trem_sim::kernel::qmath::{angle_vectors, ma};

pub type Vec3 = [f32; 3];

/// cg_ents.c / cg_view.c debug polys: (four corners, colour).
pub type DebugPoly = ([Vec3; 4], [f32; 4]);

/// `cg.testModelEntity` / `cg.testModelBarrelEntity` / `cg.testGun`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestModel {
    /// cg.testModelName
    pub name: String,
    /// cg.testModelBarrelName: the name with "_barrel.md3" for its last four characters
    pub barrel_name: String,
    pub frame: i32,
    pub oldframe: i32,
    pub backlerp: f32,
    pub skin_num: i32,
    pub origin: Vec3,
    /// angles of testModelEntity.axis (`AnglesToAxis`)
    pub angles: Vec3,
    pub gun: bool,
}

impl TestModel {
    /// `CG_TestModel_f` (argv after the command): None without a model
    /// name. `vieworg` / `viewangles` are cg.refdef's.
    pub fn new(argv: &[String], vieworg: Vec3, viewangles: Vec3) -> Option<TestModel> {
        let name = argv.get(1)?.clone();
        let mut barrel: String = name.chars().take(name.chars().count().saturating_sub(4)).collect();
        barrel.push_str("_barrel.md3");
        let mut m = TestModel { name, barrel_name: barrel, ..Default::default() };
        if argv.len() == 3 {
            m.backlerp = trem_sim::kernel::spawn::c_atof(&argv[2]) as f32;
            m.frame = 1;
            m.oldframe = 0;
        }
        let (f, _, _) = angle_vectors(&viewangles);
        m.origin = ma(&vieworg, 100.0, &f);
        m.angles = [0.0, 180.0 + viewangles[1], 0.0];
        Some(m)
    }

    /// `CG_TestModelNextFrame_f` / `CG_TestModelPrevFrame_f`: the print.
    pub fn step_frame(&mut self, next: bool) -> String {
        self.frame = if next { self.frame + 1 } else { (self.frame - 1).max(0) };
        format!("frame {}\n", self.frame)
    }

    /// `CG_TestModelNextSkin_f` / `CG_TestModelPrevSkin_f`.
    pub fn step_skin(&mut self, next: bool) -> String {
        self.skin_num = if next { self.skin_num + 1 } else { (self.skin_num - 1).max(0) };
        format!("skin {}\n", self.skin_num)
    }

    /// `CG_AddTestModel`'s gun placement: the view origin moved by
    /// cg_gun_x/y/z along the view axis, the view's angles.
    pub fn place_gun(&mut self, vieworg: Vec3, viewangles: Vec3, gun: Vec3) {
        let (f, r, u) = angle_vectors(&viewangles);
        // viewaxis[1] is left (-right)
        let left = [-r[0], -r[1], -r[2]];
        let mut o = ma(&vieworg, gun[0], &f);
        o = ma(&o, gun[1], &left);
        o = ma(&o, gun[2], &u);
        self.origin = o;
        self.angles = viewangles;
    }
}

/// `CG_DrawBoundingBox`: the six faces of origin + [mins, maxs] (red, 50%).
pub fn bounding_box(origin: Vec3, mins: Vec3, maxs: Vec3) -> Vec<DebugPoly> {
    let p = |x: f32, y: f32, z: f32| [origin[0] + x, origin[1] + y, origin[2] + z];
    let ppp = p(maxs[0], maxs[1], maxs[2]);
    let mpp = p(mins[0], maxs[1], maxs[2]);
    let mmp = p(mins[0], mins[1], maxs[2]);
    let pmp = p(maxs[0], mins[1], maxs[2]);
    let ppm = p(maxs[0], maxs[1], mins[2]);
    let mpm = p(mins[0], maxs[1], mins[2]);
    let mmm = p(mins[0], mins[1], mins[2]);
    let pmm = p(maxs[0], mins[1], mins[2]);
    // CG_DrawBoxFace: color { 255, 0, 0, 128 } as modulate bytes, verts d c b a
    let c = [1.0, 0.0, 0.0, 128.0 / 255.0];
    let face = |a: Vec3, b: Vec3, cc: Vec3, d: Vec3| ([d, cc, b, a], c);
    vec![face(ppp, mpp, mmp, pmp), face(ppp, pmp, pmm, ppm), face(mpp, ppp, ppm, mpm), face(mmp, mpp, mpm, mmm), face(pmp, mmp, mmm, pmm), face(mmm, mpm, ppm, pmm)]
}

/// cg_ents.c `CG_AddPacketEntities`' cg_drawBBOX box of an entity from its
/// packed `solid` (x = solid & 255, zd = (solid >> 8) & 255,
/// zu = ((solid >> 16) & 255) - 32).
pub fn solid_box(solid: i32) -> (Vec3, Vec3) {
    let x = (solid & 255) as f32;
    let zd = ((solid >> 8) & 255) as f32;
    let zu = ((solid >> 16) & 255) as f32 - 32.0;
    ([-x, -x, -zd], [x, x, zu])
}

/// sv_world.c `SV_LinkEntity`'s `s.solid` ("encode the size into the
/// entityState_t for client prediction"); SOLID_BMODEL for brush models.
pub fn packed_solid(bmodel: bool, contents: i32, mins: Vec3, maxs: Vec3) -> i32 {
    use trem_sim::gen::enums::contents as c;
    if bmodel {
        return 0xffffff;
    }
    if contents as u32 & (c::SOLID | c::BODY) == 0 {
        return 0;
    }
    // assume that x/y are equal and symetric; z is not symetric, and z maxs can be negative
    let i = (maxs[0] as i32).clamp(1, 255);
    let j = (-mins[2] as i32).clamp(1, 255);
    let k = ((maxs[2] + 32.0) as i32).clamp(1, 255);
    (k << 16) | (j << 8) | i
}

/// cg_view.c NORMAL_HEIGHT / NORMAL_WIDTH
const NORMAL_HEIGHT: f32 = 64.0;
const NORMAL_WIDTH: f32 = 6.0;

/// `CG_DrawSurfNormal`: a green quad standing on the traced surface
/// (`end` and `normal` of the trace, `left` = cg.refdef.viewaxis[1]).
pub fn surf_normal(end: Vec3, normal: Vec3, left: Vec3) -> DebugPoly {
    let v1 = ma(&end, NORMAL_WIDTH, &left);
    let top = ma(&end, NORMAL_HEIGHT, &normal);
    let v2 = ma(&top, NORMAL_WIDTH, &left);
    // color { 0, 255, 0, 128 } as modulate bytes
    ([end, v1, v2, top], [0.0, 1.0, 0.0, 128.0 / 255.0])
}

/// The cg_debugEvents line of `CG_EntityEvent`.
pub fn debug_event_line(entity: i32, event: i32) -> String {
    format!("ent:{entity:3}  event:{event:3} {}\n", trem_sim::kernel::bg::event_name(event))
}

/// The test model's instances and the debug poly mesh.
#[derive(Default)]
pub struct DevState {
    pub test: Option<TestModel>,
    model: Option<ModelInst>,
    barrel: Option<ModelInst>,
    /// (name, skin) the instances were made for
    made: Option<(String, i32)>,
    polys: Option<Gd<MeshInstance3D>>,
}

impl DevState {
    /// `trap_R_RegisterModel` + the entity: (re)instance the model when the
    /// name or skin changed (`skinNum` picks `surface->shaders[ skinNum %
    /// numShaders ]`). False: "Can't register model".
    fn instance(&mut self, lib: &mut Library, parent: &mut Gd<Node3D>) -> bool {
        let Some(t) = self.test.clone() else { return false };
        if self.made.as_ref() == Some(&(t.name.clone(), t.skin_num)) {
            return self.model.is_some();
        }
        self.clear_instances();
        self.made = Some((t.name.clone(), t.skin_num));
        let Some(res) = lib.model(&t.name) else { return false };
        let skin_map = |res: &crate::models::ModelRes, n: i32| -> std::collections::HashMap<String, String> {
            res.md3.surfaces.iter().filter(|s| !s.shaders.is_empty()).map(|s| (s.name.to_lowercase(), s.shaders[(n.max(0) as usize) % s.shaders.len()].clone())).collect()
        };
        let skin = skin_map(&res, t.skin_num);
        let m = lib.instance(&res, Some(&skin));
        parent.add_child(&m.root);
        self.model = Some(m);
        if let Some(bres) = lib.model(&t.barrel_name) {
            let b = lib.instance(&bres, None);
            parent.add_child(&b.root);
            self.barrel = Some(b);
        }
        true
    }

    fn clear_instances(&mut self) {
        if let Some(m) = self.model.take() {
            m.free();
        }
        if let Some(b) = self.barrel.take() {
            b.free();
        }
        self.made = None;
    }

    /// `CG_TestModel_f` / `CG_TestGun_f`: the new state (a print on failure).
    pub fn test_model_f(&mut self, argv: &[String], vieworg: Vec3, viewangles: Vec3, gun: bool, lib: Option<&mut Library>, parent: &mut Gd<Node3D>) -> String {
        self.test = None;
        self.clear_instances();
        let Some(mut t) = TestModel::new(argv, vieworg, viewangles) else { return String::new() };
        t.gun = gun;
        self.test = Some(t);
        if !lib.is_some_and(|l| self.instance(l, parent)) {
            // the entity keeps hModel 0: nothing is drawn
            self.test = None;
            return "Can't register model\n".into();
        }
        String::new()
    }

    /// nextframe / prevframe / nextskin / prevskin (they work on the
    /// cleared entity too: frame 0, skin 0).
    pub fn step(&mut self, cmd: &str) -> String {
        let t = self.test.get_or_insert_with(Default::default);
        match cmd {
            "nextframe" => t.step_frame(true),
            "prevframe" => t.step_frame(false),
            "nextskin" => t.step_skin(true),
            _ => t.step_skin(false),
        }
    }

    /// `CG_AddTestModel` for this frame (`light`: the light grid at the model).
    pub fn add_test_model(&mut self, lib: Option<&mut Library>, parent: &mut Gd<Node3D>, vieworg: Vec3, viewangles: Vec3, gun_offset: Vec3, light: impl Fn(&Vec3) -> ([f32; 3], [f32; 3], Vector3)) -> Option<String> {
        if self.test.as_ref().is_none_or(|t| t.name.is_empty()) {
            return None;
        }
        // re-register the model, because the level may have changed
        let ok = match lib {
            Some(l) => self.instance(l, parent),
            None => false,
        };
        if !ok {
            return Some("Can't register model\n".into());
        }
        let t = self.test.as_mut().unwrap();
        if t.gun {
            t.place_gun(vieworg, viewangles, gun_offset);
        }
        let (origin, angles, frames, gun) = (t.origin, t.angles, (t.oldframe, t.frame, t.backlerp), t.gun);
        let (amb, dcol, ld) = light(&origin);
        let xf = Transform3D::new(model_basis(&angles), q2g(&origin));
        let m = self.model.as_mut().unwrap();
        m.set_frames(frames.0, frames.1, frames.2);
        m.set_transform(xf);
        m.set_depth_hack(gun);
        // RF_MINLIGHT for the gun
        m.set_light(if gun { amb.map(|c| c.max(0.125)) } else { amb }, dcol, ld);
        let tag = m.tag("tag_barrel");
        if let Some(b) = self.barrel.as_mut() {
            b.set_frames(0, 0, 0.0);
            b.set_transform(xf * tag.unwrap_or(Transform3D::IDENTITY));
            b.set_light(amb, dcol, ld);
            b.set_depth_hack(gun);
        }
        None
    }

    /// The debug polys of this frame (`trap_R_AddPolyToScene( outlineShader )`).
    pub fn draw_polys(&mut self, parent: &mut Gd<Node3D>, polys: &[DebugPoly]) {
        if polys.is_empty() && self.polys.is_none() {
            return;
        }
        let mi = self.polys.get_or_insert_with(|| {
            let mut mi = MeshInstance3D::new_alloc();
            mi.set_mesh(&ImmediateMesh::new_gd());
            let mut mat = StandardMaterial3D::new_gd();
            mat.set_shading_mode(godot::classes::base_material_3d::ShadingMode::UNSHADED);
            mat.set_transparency(godot::classes::base_material_3d::Transparency::ALPHA);
            mat.set_cull_mode(godot::classes::base_material_3d::CullMode::DISABLED);
            mat.set_flag(godot::classes::base_material_3d::Flags::ALBEDO_FROM_VERTEX_COLOR, true);
            mi.set_material_override(&mat);
            mi.set_cast_shadows_setting(godot::classes::geometry_instance_3d::ShadowCastingSetting::OFF);
            parent.add_child(&mi);
            mi
        });
        let Some(mesh) = mi.get_mesh() else { return };
        let Ok(mut im) = mesh.try_cast::<ImmediateMesh>() else { return };
        im.clear_surfaces();
        if polys.is_empty() {
            return;
        }
        im.surface_begin(godot::classes::mesh::PrimitiveType::TRIANGLES);
        for (v, c) in polys {
            let col = Color::from_rgba(c[0], c[1], c[2], c[3]);
            for i in [0, 1, 2, 0, 2, 3] {
                im.surface_set_color(col);
                im.surface_add_vertex(q2g(&v[i]));
            }
        }
        im.surface_end();
    }

    /// A new map: the test model goes (cg is cleared by `CG_Init`).
    pub fn reset(&mut self) {
        self.test = None;
        self.clear_instances();
        if let Some(p) = self.polys.take() {
            let mut p = p;
            p.queue_free();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn test_model_placement_barrel_name_and_frames() {
        assert!(TestModel::new(&argv("testmodel"), [0.0; 3], [0.0; 3]).is_none());
        let m = TestModel::new(&argv("testmodel models/weapons/rifle/rifle.md3"), [10.0, 0.0, 50.0], [0.0, 90.0, 0.0]).unwrap();
        assert_eq!(m.barrel_name, "models/weapons/rifle/rifle_barrel.md3");
        // 100 units along the view, facing back at the viewer
        assert!((m.origin[0] - 10.0).abs() < 1e-3 && (m.origin[1] - 100.0).abs() < 1e-3 && (m.origin[2] - 50.0).abs() < 1e-3);
        assert_eq!(m.angles, [0.0, 270.0, 0.0]);
        assert_eq!((m.frame, m.oldframe, m.backlerp), (0, 0, 0.0));
        let mut m = TestModel::new(&argv("testmodel a.md3 0.25"), [0.0; 3], [0.0; 3]).unwrap();
        assert_eq!((m.frame, m.oldframe, m.backlerp), (1, 0, 0.25));
        assert_eq!(m.step_frame(true), "frame 2\n");
        m.step_frame(false);
        m.step_frame(false);
        assert_eq!(m.step_frame(false), "frame 0\n");
        assert_eq!(m.step_skin(false), "skin 0\n");
        assert_eq!(m.step_skin(true), "skin 1\n");
        // the gun follows the view with cg_gun_x/y/z
        m.place_gun([0.0; 3], [0.0, 0.0, 0.0], [5.0, 2.0, -1.0]);
        assert!((m.origin[0] - 5.0).abs() < 1e-4 && (m.origin[1] - 2.0).abs() < 1e-4 && (m.origin[2] + 1.0).abs() < 1e-4);
    }

    #[test]
    fn bbox_faces_and_surface_normal() {
        let faces = bounding_box([0.0, 0.0, 0.0], [-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]);
        assert_eq!(faces.len(), 6);
        // the top face: d c b a = pmp mmp mpp ppp
        assert_eq!(faces[0].0, [[1.0, -2.0, 3.0], [-1.0, -2.0, 3.0], [-1.0, 2.0, 3.0], [1.0, 2.0, 3.0]]);
        assert_eq!(solid_box(15 | (24 << 8) | (64 << 16)), ([-15.0, -15.0, -24.0], [15.0, 15.0, 32.0]));
        assert_eq!(packed_solid(false, 1, [-15.0, -15.0, -24.0], [15.0, 15.0, 32.0]), 15 | (24 << 8) | (64 << 16));
        assert_eq!(packed_solid(false, 0, [-15.0; 3], [15.0; 3]), 0);
        let (v, c) = surf_normal([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        assert_eq!(v, [[0.0; 3], [0.0, 6.0, 0.0], [0.0, 6.0, 64.0], [0.0, 0.0, 64.0]]);
        assert_eq!(c[1], 1.0);
    }

    #[test]
    fn event_names_match_the_event_enum() {
        use trem_sim::kernel::bg::{event_name, EVENT_NAMES};
        assert_eq!(EVENT_NAMES.len(), 74);
        assert_eq!(event_name(trem_sim::gen::enums::Event::Obituary as i32), "EV_OBITUARY");
        assert_eq!(event_name(99), "UNKNOWN");
        assert_eq!(debug_event_line(5, 1), "ent:  5  event:  1 EV_FOOTSTEP\n");
    }
}
