//! Animated players, corpses and structures built from the original models
//! (port of the presentation half of cg_players.c and cg_buildable.c).

use crate::geom::q2g;
use crate::md3::{AnimSet, LerpFrame};
use crate::models::{model_basis, tag_basis, Library, ModelInst};
use godot::classes::base_material_3d::BillboardMode;
use godot::classes::{AudioStreamPlayer3D, Label3D, Node3D};
use godot::prelude::*;
use std::rc::Rc;
use trem_sim::gen::buildables::BuildableId;
use trem_sim::gen::classes::ClassId;
use trem_sim::gen::constants as k;
use trem_sim::gen::enums::{PlayerAnim, Stat, Team};
use trem_sim::gen::upgrades::UpgradeId;
use trem_sim::gen::weapons::WeaponId;
use trem_sim::kernel::bg;
use trem_sim::kernel::qmath::Vec3;
use trem_sim::kernel::world::{EType, World};

const MOVEMENT_OFFSETS: [f32; 8] = [0.0, 22.0, 45.0, -22.0, 0.0, 22.0, -45.0, -22.0];
pub const ANIM_FORCEBIT: i32 = 0x40;

pub struct Actor {
    pub key: String,
    parts: Vec<ModelInst>,
    legs: LerpFrame,
    torso: LerpFrame,
    anims: Rc<AnimSet>,
    nonseg: bool,
    scale: f32,
    z_offset: f32,
    label: Option<Gd<Label3D>>,
    // buildable animation state (centity_t buildableAnim & co)
    banim: i32,
    old_banim: i32,
    idle: bool,
    bsounds: Vec<(bool, bool)>,
    bname: String,
    team: Team,
    looping: Option<Gd<AudioStreamPlayer3D>>,
    loop_anim: i32,
    pub model_dir: String,
    /// CG_PlayerUpgrades: jetpack / battpack on the torso's tag_head, and the
    /// jetpack flash on the jetpack's tag_flash while it is active.
    jetpack: Option<ModelInst>,
    jetpack_flash: Option<ModelInst>,
    battpack: Option<ModelInst>,
    jetpack_active: bool,
}

/// Sound request produced while animating (buildable animation sounds).
pub struct SoundReq {
    pub path: String,
    pub origin: Vec3,
    pub ent: usize,
}

fn yaw_basis(yaw: f32) -> Basis {
    model_basis(&[0.0, yaw, 0.0])
}

fn scale_basis(b: Basis, s: f32) -> Basis {
    b * Basis::from_scale(Vector3::splat(s))
}

fn angle180(mut a: f32) -> f32 {
    while a > 180.0 {
        a -= 360.0;
    }
    while a < -180.0 {
        a += 360.0;
    }
    a
}

/// The model directory, skin and scale of a class (classes sheet cold columns,
/// seeded from overrides/classes/*.cfg).
pub fn class_model(class: ClassId, vanilla: bool) -> Option<(String, String, f32, f32)> {
    // a Tremulous 1.1.0 game: 1.1's model, skin, scale and z offset
    // (BG_Find*ForClass with data-1.1.0.pk3's overrides, dec041)
    if let Some(c11) = trem_sim::kernel::v11::class11_of(class).filter(|_| vanilla) {
        let cold = c11.cold();
        let model = cold.model?;
        return Some((model.to_string(), cold.skin.unwrap_or("default").to_string(), cold.model_scale, c11.def().z_offset));
    }
    let cold = class.cold();
    let model = cold.model?;
    Some((model.to_string(), cold.skin.unwrap_or("default").to_string(), cold.model_scale, class.def().z_offset))
}

impl Actor {
    pub fn root_nodes(&self) -> Vec<Gd<Node3D>> {
        let mut v: Vec<Gd<Node3D>> = self.parts.iter().chain(self.jetpack.iter()).chain(self.jetpack_flash.iter()).chain(self.battpack.iter()).map(|p| p.root.clone()).collect();
        if let Some(l) = &self.label {
            v.push(l.clone().upcast());
        }
        if let Some(l) = &self.looping {
            v.push(l.clone().upcast());
        }
        v
    }

    pub fn free(self) {
        for p in self.parts.into_iter().chain(self.jetpack).chain(self.jetpack_flash).chain(self.battpack) {
            p.free();
        }
        if let Some(mut l) = self.label {
            l.queue_free();
        }
        if let Some(mut l) = self.looping {
            l.queue_free();
        }
    }

    /// World transform (Godot space) of a model tag on this actor: the jetpack's
    /// tag_flash for "jetpack:tag_flash", else the first part (weapon, head,
    /// torso, legs order) that has it.
    pub fn world_tag(&self, name: &str) -> Option<Transform3D> {
        if let Some(t) = name.strip_prefix("jetpack:") {
            let j = self.jetpack.as_ref()?;
            return j.tag(t).map(|tt| j.root.get_transform() * tt);
        }
        self.parts.iter().rev().find_map(|p| p.tag(name).map(|t| p.root.get_transform() * t))
    }

    /// One line per actor for the `actor_info` test hook.
    pub fn debug_line(&self) -> String {
        let parts: Vec<String> = self.parts.iter().map(|p| {
            let r = &p.root;
            let o = if r.is_inside_tree() { r.get_global_position() } else { r.get_position() };
            format!("[in_tree={} vis={} pos=({:.2},{:.2},{:.2}) surfs={}/{}]", r.is_inside_tree(), r.is_visible_in_tree(), o.x, o.y, o.z, p.mesh_count(), p.res.surfs.len())
        }).collect();
        format!("{} scale={} z_offset={} parts={}", self.key, self.scale, self.z_offset, parts.join(" "))
    }
    pub fn set_visible(&mut self, v: bool) {
        for p in self.parts.iter_mut().chain(self.jetpack.iter_mut()).chain(self.battpack.iter_mut()) {
            p.set_visible(v);
        }
        let active = v && self.jetpack_active;
        if let Some(f) = self.jetpack_flash.as_mut() {
            f.set_visible(active);
        }
        if let Some(l) = self.label.as_mut() {
            l.set_visible(v);
        }
    }

    /// A player or corpse model for `class` (segmented humans get torso, head
    /// and their held weapon; aliens are one non-segmented model).
    pub fn player(lib: &mut Library, class: ClassId, team: Team, stats: &[i32], weapon: i32, name: Option<&str>, key: String, now: i32, vanilla: bool) -> Option<Actor> {
        let (dir, skin, scale, z_offset) = class_model(class, vanilla)?;
        let anims = lib.player_anims(&dir)?;
        let mut parts = Vec::new();
        if anims.nonseg_or(false) {
            let res = lib.model(&format!("models/players/{dir}/nonseg.md3"))?;
            let sk = lib.skin(&format!("models/players/{dir}/nonseg_{skin}.skin"));
            parts.push(lib.instance(&res, Some(&sk)));
        } else {
            let larmour = bg::inventory_contains_upgrade(UpgradeId::Larmour.num() as i32, stats);
            let helmet = bg::inventory_contains_upgrade(UpgradeId::Helmet.num() as i32, stats);
            let body_skin = if larmour { "light" } else { skin.as_str() };
            let head_skin = if helmet { "light" } else { skin.as_str() };
            for (part, sk) in [("lower", body_skin), ("upper", body_skin), ("head", head_skin)] {
                let res = lib.model(&format!("models/players/{dir}/{part}.md3"))?;
                let mut skf = format!("models/players/{dir}/{part}_{sk}.skin");
                if !lib.has(&skf) {
                    skf = format!("models/players/{dir}/{part}_default.skin");
                }
                let s = lib.skin(&skf);
                parts.push(lib.instance(&res, Some(&s)));
            }
            if let Some(wid) = WeaponId::from_num(weapon as u8).filter(|_| weapon > 0) {
                let cfg = lib.weapon_cfg(wid.id());
                if let Some(m) = cfg.model.clone() {
                    if let Some(res) = lib.model(&m) {
                        parts.push(lib.instance(&res, None));
                    }
                }
            }
        }
        let label = name.map(|n| {
            let mut l = Label3D::new_alloc();
            l.set_text(n);
            l.set_billboard_mode(BillboardMode::ENABLED);
            l.set_pixel_size(0.004);
            l.set_font_size(40);
            l.set_modulate(if team == Team::Aliens { Color::from_rgb(0.7, 1.0, 0.5) } else { Color::from_rgb(0.6, 0.8, 1.0) });
            l
        });
        let segmented = !anims.nonseg_or(false);
        let mut upgrade = |held: bool, path: &str| -> Option<ModelInst> {
            if !(held && segmented) {
                return None;
            }
            lib.model(path).map(|res| lib.instance(&res, None))
        };
        let jetpack = upgrade(bg::inventory_contains_upgrade(UpgradeId::Jetpack.num() as i32, stats), "models/players/human_base/jetpack.md3");
        let jetpack_flash = upgrade(jetpack.is_some(), "models/players/human_base/jetpack_flash.md3");
        let battpack = upgrade(bg::inventory_contains_upgrade(UpgradeId::Battpack.num() as i32, stats), "models/players/human_base/battpack.md3");
        let mut a = Actor { key, parts, legs: LerpFrame::default(), torso: LerpFrame::default(), nonseg: anims.nonseg_or(false), anims, scale, z_offset, label, banim: 0, old_banim: 0, idle: true, bsounds: Vec::new(), bname: String::new(), team, looping: None, loop_anim: -1, model_dir: dir, jetpack, jetpack_flash, battpack, jetpack_active: false };
        a.legs.clear(&a.anims.clone(), 0, now);
        a.torso.clear(&a.anims.clone(), 0, now);
        Some(a)
    }

    /// A structure: models 0..2 of overrides/buildables/<name>.cfg.
    pub fn buildable(lib: &mut Library, b: BuildableId, key: String) -> Option<Actor> {
        let name = b.id().to_string();
        let cfg = lib.buildable_cfg(&name);
        let Some(anims) = lib.buildable_anims(&name) else {
            godot_warn!("buildable {name}: models/buildables/{name}/animation.cfg missing or unparsable");
            return None;
        };
        let mut parts = Vec::new();
        for m in cfg.models.iter().take(3) {
            match m {
                Some(path) => match lib.model(path) {
                    Some(res) => parts.push(lib.instance(&res, None)),
                    None => {
                        godot_warn!("buildable {name}: model {path} failed to load");
                        // keep the parts that did load (a turret without its barrel still shows)
                        break;
                    }
                },
                None => break,
            }
        }
        if parts.is_empty() {
            godot_warn!("buildable {name}: no model in overrides/buildables/{name}.cfg (models {:?})", cfg.models);
            return None;
        }
        let bsounds = lib.text(&format!("sound/buildables/{name}/sound.cfg")).map(|t| crate::md3::parse_buildable_sounds(&t)).unwrap_or_default();
        let looping = Some(crate::audio::new_3d());
        Some(Actor { key, parts, legs: LerpFrame::default(), torso: LerpFrame::default(), anims, nonseg: false, scale: cfg.scale, z_offset: cfg.z_offset, label: None, banim: 0, old_banim: 0, idle: true, bsounds, bname: name, team: b.def().team, looping, loop_anim: -1, model_dir: String::new(), jetpack: None, jetpack_flash: None, battpack: None, jetpack_active: false })
    }

    /// Animate and place a player (`CG_Player`) or a corpse.
    pub fn update_player(&mut self, w: &World, i: usize, now: i32) {
        let e = &w.ents[i];
        // CG_Player: a wall climber's model stands on its surface. The local
        // player has SS_WALLCLIMBING; a remote one only its entity's
        // EF_WALLCLIMB / EF_WALLCLIMBCEILING and angles2 (= grapplePoint,
        // which on a ceiling is the rotation axis, not the normal)
        let (origin, view, legs_anim, torso_anim, dir, wallclimb, normal, ceiling) = if e.etype == EType::Player {
            let ps = &w.clients[i].ps;
            let dead = ps.stats[Stat::Health as usize] <= 0 || ps.e_flags & k::EF_DEAD != 0;
            let climbing = !dead && (ps.stats[Stat::State as usize] & k::SS_WALLCLIMBING != 0 || ps.e_flags & k::EF_WALLCLIMB != 0);
            (ps.origin, ps.viewangles, ps.legs_anim, ps.torso_anim, if dead { 0 } else { ps.movement_dir.clamp(0, 7) as usize }, climbing, ps.grapple_point, ps.e_flags & k::EF_WALLCLIMBCEILING != 0)
        } else {
            (e.origin, e.angles, e.legs_anim, e.torso_anim, 0, false, [0.0, 0.0, 1.0], false)
        };
        let wallclimb = wallclimb && (ceiling || normal != [0.0, 0.0, 0.0]);
        // a ceiling climber's grapplePoint is a horizontal rotation axis, so
        // without EF_WALLCLIMBCEILING (a 1.1 server keeps the ceiling in the
        // playerState's STAT_STATE only) it reads as a wall normal: a
        // horizontal "normal" with no wall behind it but a ceiling above is
        // the ceiling (CG_Player traces the surface it draws the model on too)
        let ceiling = ceiling || (wallclimb && normal[2].abs() < 0.3 && {
            let back = [origin[0] - normal[0] * 32.0, origin[1] - normal[1] * 32.0, origin[2] - normal[2] * 32.0];
            let up_end = [origin[0], origin[1], origin[2] + 48.0];
            let wall = w.trace(&origin, None, None, &back, i as i32, k::MASK_PLAYERSOLID);
            let roof = w.trace(&origin, None, None, &up_end, i as i32, k::MASK_PLAYERSOLID);
            wall.fraction >= 1.0 && roof.fraction < 1.0 && roof.plane.normal[2] < -0.7
        });
        let anims = self.anims.clone();
        let up = if !wallclimb { [0.0, 0.0, 1.0] } else if ceiling { [0.0, 0.0, -1.0] } else { normal };
        let mut base_o = origin;
        for c in 0..3 {
            base_o[c] += up[c] * self.z_offset;
        }
        let pos = q2g(&base_o);
        if self.nonseg {
            self.legs.run_player(&anims, legs_anim, now);
            let mut basis = yaw_basis(view[1]);
            if wallclimb {
                // CG_PlayerAngles: the view (world angles) back into the
                // surface's frame for the heading, then CG_Player rotates
                // the upright model onto the surface
                use trem_sim::kernel::qmath::{angles_to_axis, axis_to_angles};
                let rel = bg::rotate_axis(&normal, &angles_to_axis(&view), true, ceiling).map(|a| axis_to_angles(&a)).unwrap_or(view);
                let axis = angles_to_axis(&[0.0, rel[1], 0.0]);
                if let Some(r) = bg::rotate_axis(&normal, &axis, false, ceiling) {
                    basis = tag_basis(&r);
                }
            }
            let (o, f, b) = (self.legs.old_frame, self.legs.frame, self.legs.backlerp);
            let p = &mut self.parts[0];
            p.set_frames(o, f, b);
            p.set_transform(Transform3D::new(scale_basis(basis, self.scale), pos));
        } else {
            self.legs.run_player(&anims, legs_anim, now);
            self.torso.run_player(&anims, torso_anim, now);
            let pitch = angle180(view[0]);
            let off = MOVEMENT_OFFSETS[dir];
            let legs_yaw = view[1] + off;
            let torso_yaw = view[1] + 0.25 * off;
            let torso_pitch = 0.75 * pitch;
            let base = Transform3D::new(scale_basis(yaw_basis(legs_yaw), self.scale), pos);
            self.parts[0].set_frames(self.legs.old_frame, self.legs.frame, self.legs.backlerp);
            self.parts[0].set_transform(base);
            let tag = self.parts[0].tag("tag_torso").unwrap_or(Transform3D::IDENTITY);
            let torso = base * tag * Transform3D::new(model_basis(&[torso_pitch, torso_yaw - legs_yaw, 0.0]), Vector3::ZERO);
            let (o, f, b) = (self.torso.old_frame, self.torso.frame, self.torso.backlerp);
            self.parts[1].set_frames(o, f, b);
            self.parts[2].set_frames(o, f, b);
            self.parts[1].set_transform(torso);
            let head_tag = self.parts[1].tag("tag_head").unwrap_or(Transform3D::IDENTITY);
            let head = torso * head_tag * Transform3D::new(model_basis(&[pitch - torso_pitch, view[1] - torso_yaw, 0.0]), Vector3::ZERO);
            self.parts[2].set_transform(head);
            if self.parts.len() > 3 {
                let wt = self.parts[1].tag("tag_weapon").unwrap_or(Transform3D::IDENTITY);
                self.parts[3].set_frames(0, 0, 0.0);
                self.parts[3].set_transform(torso * wt);
            }
            // CG_PlayerUpgrades (FIXME in the C: tag_back does not exist, tag_head is used)
            let on_head = torso * head_tag;
            if let Some(b) = self.battpack.as_mut() {
                b.set_frames(0, 0, 0.0);
                b.set_transform(on_head);
            }
            let held_active = e.etype == EType::Player && bg::upgrade_is_active(UpgradeId::Jetpack.num() as i32, &w.clients[i].ps.stats);
            self.jetpack_active = held_active;
            if let Some(j) = self.jetpack.as_mut() {
                j.set_frames(0, 0, 0.0);
                j.set_transform(on_head);
                let ft = j.tag("tag_flash").unwrap_or(Transform3D::IDENTITY);
                if let Some(f) = self.jetpack_flash.as_mut() {
                    f.set_frames(0, 0, 0.0);
                    f.set_transform(on_head * ft);
                    f.set_visible(held_active);
                }
            }
        }
        if let Some(l) = self.label.as_mut() {
            let mut top = origin;
            top[2] += 48.0 * self.scale;
            l.set_position(q2g(&top));
        }
    }

    pub fn set_light(&mut self, amb: [f32; 3], dcol: [f32; 3], ldir: Vector3) {
        for p in self.parts.iter_mut() {
            p.set_light(amb, dcol, ldir);
        }
    }

    /// The parts a ragdoll is built from: (rig part name, model), in the
    /// rig's order (the held weapon and upgrades are not part of the body).
    pub fn body_parts(&self) -> Vec<(&'static str, &ModelInst)> {
        if self.nonseg {
            self.parts.first().map(|p| vec![("nonseg", p)]).unwrap_or_default()
        } else if self.parts.len() >= 3 {
            vec![("lower", &self.parts[0]), ("upper", &self.parts[1]), ("head", &self.parts[2])]
        } else {
            Vec::new()
        }
    }
    pub fn model_scale(&self) -> f32 {
        self.scale
    }

    pub fn footsteps(&self) -> &str {
        &self.anims.footsteps
    }

    /// The torso lerp frame (first-person hand animation follows it).
    pub fn torso_frames(&self) -> (i32, i32, f32) {
        (self.torso.old_frame, self.torso.frame, self.torso.backlerp)
    }
    pub fn torso_anims(&self) -> Rc<AnimSet> {
        self.anims.clone()
    }

    /// Animate and place a structure (`CG_Buildable` + `CG_BuildableAnimation`).
    pub fn update_buildable(&mut self, w: &World, i: usize, now: i32, out: &mut Vec<SoundReq>) {
        let e = &w.ents[i];
        let anims = self.anims.clone();
        let spawned = e.e_flags & k::EF_B_SPAWNED != 0;
        if self.banim == 0 {
            self.banim = e.torso_anim;
            self.idle = true;
        }
        let (old, frame, back);
        if !spawned {
            let first = anims.anims.get(1).map(|a| a.first_frame).unwrap_or(0);
            self.legs.old_frame_time = now;
            self.legs.frame_time = now;
            self.legs.animation_time = now;
            self.legs.old_frame = first;
            self.legs.frame = first;
            self.legs.backlerp = 0.0;
            self.old_banim = 0;
            (old, frame, back) = (first, first, 0.0);
        } else {
            if (self.old_banim ^ e.legs_anim) & k::ANIM_TOGGLEBIT != 0 {
                if self.banim == e.torso_anim || e.legs_anim & ANIM_FORCEBIT != 0 {
                    self.banim = e.legs_anim;
                    self.old_banim = e.legs_anim;
                    self.idle = false;
                } else {
                    self.banim = e.torso_anim;
                    self.old_banim = e.torso_anim;
                    self.idle = true;
                }
            } else if self.idle && self.banim != e.torso_anim {
                self.banim = e.torso_anim;
            }
            // CG_RunBuildableLerpFrame
            let new_anim = self.banim & !(k::ANIM_TOGGLEBIT | ANIM_FORCEBIT);
            if new_anim != self.legs.animation_number || self.legs.animation.is_none() {
                if self.legs.animation.is_none() {
                    self.legs.frame_time = now + 1000;
                }
                self.legs.set_animation(&anims, new_anim, 0);
                if let Some(&(enabled, looped)) = self.bsounds.get(new_anim as usize) {
                    if enabled && !looped {
                        out.push(SoundReq { path: self.anim_sound(new_anim), origin: e.origin, ent: i });
                    }
                }
            }
            self.legs.run(now);
            if self.legs.frame_time == now {
                self.banim = e.torso_anim;
                self.idle = true;
            }
            (old, frame, back) = (self.legs.old_frame, self.legs.frame, self.legs.backlerp);
        }
        // looping animation sound (trap_S_AddLoopingSound)
        let cur = self.legs.animation_number;
        let want_loop = spawned && self.bsounds.get(cur as usize).map(|s| s.0 && s.1).unwrap_or(false);
        if want_loop && self.loop_anim != cur {
            self.loop_anim = cur;
            out.push(SoundReq { path: format!("@loop:{}", self.anim_sound(cur)), origin: e.origin, ent: i });
        } else if !want_loop && self.loop_anim != -1 {
            self.loop_anim = -1;
            if let Some(l) = self.looping.as_mut() {
                l.stop();
            }
        }
        // placement: origin + zOffset along the surface normal, axis from normal and yaw
        let normal = if e.origin2 == [0.0; 3] { [0.0, 0.0, 1.0] } else { e.origin2 };
        let mut o = e.origin;
        for c in 0..3 {
            o[c] += normal[c] * self.z_offset;
        }
        let axis = buildable_axis(&e.angles, &normal);
        let base = Transform3D::new(scale_basis(tag_basis(&axis), self.scale), q2g(&o));
        let aim = e.turret_aim;
        let health = e.health;
        let p0 = &mut self.parts[0];
        p0.set_frames(old, frame, back);
        p0.set_transform(base);
        if spawned {
            p0.set_tint(Color::from_rgba(1.0, 1.0, 1.0, 1.0), 0.0);
        } else if self.team == Team::Humans {
            p0.set_tint(Color::from_rgba(0.55, 0.75, 1.0, 1.0), 0.6);
        }
        let _ = health;
        if self.parts.len() > 1 {
            let tag = self.parts[0].tag("tag_turret").unwrap_or(Transform3D::IDENTITY);
            let at = (base * tag).origin;
            let barrel = Transform3D::new(scale_basis(model_basis(&aim), self.scale), at);
            self.parts[1].set_frames(old, frame, back);
            self.parts[1].set_transform(barrel);
            if self.parts.len() > 2 {
                let top = Transform3D::new(scale_basis(model_basis(&[0.0, aim[1], 0.0]), self.scale), at);
                self.parts[2].set_frames(old, frame, back);
                self.parts[2].set_transform(top);
            }
        }
        if let Some(l) = self.looping.as_mut() {
            l.set_position(q2g(&e.origin));
        }
    }

    fn anim_sound(&self, anim: i32) -> String {
        const NAMES: [&str; 15] = ["", "construct1", "construct2", "idle1", "idle2", "idle3", "attack1", "attack2", "spawn1", "spawn2", "pain1", "pain2", "destroy1", "destroy2", "destroyed"];
        let n = NAMES.get(anim as usize).copied().unwrap_or("idle1");
        let team = if self.team == Team::Aliens { "alien" } else { "human" };
        format!("sound/buildables/{}/{n}.wav|sound/buildables/{team}/{n}.wav", self.bname)
    }

    pub fn looping_player(&mut self) -> Option<&mut Gd<AudioStreamPlayer3D>> {
        self.looping.as_mut()
    }
}

/// `CG_PositionAndOrientateBuildable` axis: up = surface normal, forward = yaw projected on it.
fn buildable_axis(angles: &Vec3, normal: &Vec3) -> [[f32; 3]; 3] {
    let (fwd, _, _) = trem_sim::kernel::qmath::angle_vectors(angles);
    let d = fwd[0] * normal[0] + fwd[1] * normal[1] + fwd[2] * normal[2];
    let mut x = [fwd[0] - d * normal[0], fwd[1] - d * normal[1], fwd[2] - d * normal[2]];
    let l = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
    if l < 1e-4 {
        x = [1.0, 0.0, 0.0];
    } else {
        x.iter_mut().for_each(|v| *v /= l);
    }
    let z = *normal;
    // y = -(x cross z) = z cross x  (left)
    let y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
    [x, y, z]
}

/// `CG_MapTorsoToWeaponFrame`.
pub fn map_torso_to_weapon_frame(anims: &AnimSet, frame: i32) -> i32 {
    let a = |n: PlayerAnim| anims.anims.get(n as usize).map(|x| x.first_frame).unwrap_or(-1000);
    let drop = a(PlayerAnim::TorsoDrop);
    if frame >= drop && frame < drop + 9 {
        return frame - drop + 6;
    }
    let at = a(PlayerAnim::TorsoAttack);
    if frame >= at && frame < at + 6 {
        return 1 + frame - at;
    }
    let at2 = a(PlayerAnim::TorsoAttack2);
    if frame >= at2 && frame < at2 + 6 {
        return 1 + frame - at2;
    }
    0
}

impl AnimSet {
    pub fn nonseg_or(&self, _d: bool) -> bool {
        self.nonsegmented
    }
}

/// The first-person hand + weapon (`CG_AddViewWeapon` / the `ps` case of
/// `CG_AddPlayerWeapon`): weaponModel's `_hand` model, the gun on its
/// tag_weapon, the spinning `_barrel` on the gun's tag_barrel and the
/// `_flash` on tag_flash. Weapons without a weaponModel (the aliens' in the
/// 1.1 data) have no view model, as in the original.
pub struct ViewHand {
    pub weapon: i32,
    hand: ModelInst,
    gun: Option<ModelInst>,
    barrel: Option<ModelInst>,
    flash: Option<ModelInst>,
    pub continuous_flash: [bool; 3],
    /// the weapon's animation.cfg and noDrift (`CG_ParseWeaponAnimationFile`)
    anims: Option<Rc<AnimSet>>,
    pub no_drift: bool,
    /// cent->pe.weapon (`CG_WeaponAnimation`)
    lf: LerpFrame,
}

impl ViewHand {
    pub fn new(lib: &mut Library, weapon: i32) -> Option<ViewHand> {
        let wid = WeaponId::from_num(weapon as u8).filter(|_| weapon > 0)?;
        let cfg = lib.weapon_cfg(wid.id());
        let model = cfg.model.clone()?;
        let stem = model.trim_end_matches(".md3");
        let hand_res = lib.model(&format!("{stem}_hand.md3"))?;
        let hand = lib.instance(&hand_res, None);
        let gun = lib.model(&model).map(|r| lib.instance(&r, None));
        let barrel = lib.model(&format!("{stem}_barrel.md3")).map(|r| lib.instance(&r, None));
        let flash = lib.model(&format!("{stem}_flash.md3")).map(|r| lib.instance(&r, None));
        let mut vh = ViewHand { weapon, hand, gun, barrel, flash, continuous_flash: cfg.continuous_flash, anims: cfg.anims.clone(), no_drift: cfg.no_drift, lf: LerpFrame::default() };
        // hand.renderfx = RF_DEPTHHACK | RF_FIRST_PERSON | RF_MINLIGHT
        vh.hand.set_depth_hack(true);
        for m in [&mut vh.gun, &mut vh.barrel, &mut vh.flash].into_iter().flatten() {
            m.set_depth_hack(true);
        }
        Some(vh)
    }
    /// World transform of a tag on the view weapon (tag_flash for muzzle effects).
    pub fn world_tag(&self, name: &str) -> Option<Transform3D> {
        let g = self.gun.as_ref()?;
        g.tag(name).map(|t| g.root.get_transform() * t)
    }
    pub fn nodes(&self) -> Vec<Gd<Node3D>> {
        let mut v = vec![self.hand.root.clone()];
        for m in [&self.gun, &self.barrel, &self.flash].into_iter().flatten() {
            v.push(m.root.clone());
        }
        v
    }
    pub fn set_light(&mut self, amb: [f32; 3], dcol: [f32; 3], ldir: Vector3) {
        self.hand.set_light(amb, dcol, ldir);
        for m in [&mut self.gun, &mut self.barrel].into_iter().flatten() {
            m.set_light(amb, dcol, ldir);
        }
    }

    /// Place the hand at `origin` / `axis` (`CG_AddViewWeapon`), the gun on
    /// its tag_weapon (`CG_PositionEntityOnTag`), the barrel rolled by
    /// `barrel_roll` and the flash rolled by `flash_roll` when it is shown
    /// (`CG_PositionRotatedEntityOnTag`).
    /// `CG_WeaponAnimation`: the gun's frames from es->weaponAnim
    /// (ps.weaponAnim) through the weapon's animation.cfg; frame 0 without one.
    fn weapon_animation(&mut self, weapon_anim: i32, now: i32) -> (i32, i32, f32) {
        let Some(set) = self.anims.clone() else { return (0, 0, 0.0) };
        // see if the animation sequence is switching (`CG_SetWeaponLerpFrameAnimation`)
        if weapon_anim != self.lf.animation_number || self.lf.animation.is_none() {
            self.lf.set_animation(&set, weapon_anim, k::ANIM_TOGGLEBIT);
        }
        self.lf.run(now);
        (self.lf.old_frame, self.lf.frame, self.lf.backlerp)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, origin: &Vec3, axis: &[Vec3; 3], frames: (i32, i32, f32), visible: bool, barrel_roll: f32, flash_roll: Option<f32>, weapon_anim: i32, now: i32) {
        let t = Transform3D::new(tag_basis(axis), q2g(origin));
        self.hand.set_frames(frames.0, frames.1, frames.2);
        self.hand.set_transform(t);
        self.hand.set_visible(visible);
        if self.gun.is_none() {
            return;
        }
        let tag = self.hand.tag("tag_weapon").unwrap_or(Transform3D::IDENTITY);
        let (old, new, back) = self.weapon_animation(weapon_anim, now);
        let Some(g) = self.gun.as_mut() else { return };
        g.set_frames(old, new, back);
        let gt = t * tag;
        g.set_transform(gt);
        g.set_visible(visible);
        if let Some(b) = self.barrel.as_mut() {
            let bt = g.tag("tag_barrel").unwrap_or(Transform3D::IDENTITY);
            b.set_frames(0, 0, 0.0);
            b.set_transform(gt * bt * Transform3D::new(model_basis(&[0.0, 0.0, barrel_roll]), Vector3::ZERO));
            b.set_visible(visible);
        }
        if let Some(f) = self.flash.as_mut() {
            match flash_roll {
                Some(roll) if visible => {
                    let ft = g.tag("tag_flash").unwrap_or(Transform3D::IDENTITY);
                    f.set_frames(0, 0, 0.0);
                    f.set_transform(gt * ft * Transform3D::new(model_basis(&[0.0, 0.0, roll]), Vector3::ZERO));
                    f.set_visible(true);
                }
                _ => f.set_visible(false),
            }
        }
    }
    pub fn free(self) {
        self.hand.free();
        for m in [self.gun, self.barrel, self.flash].into_iter().flatten() {
            m.free();
        }
    }
}

/// The build preview (`CG_GhostBuildable`): the structure's model 0 drawn
/// with cgs.media.greenBuildShader / redBuildShader (gfx/misc/greenbuild,
/// gfx/misc/redbuild: additive) as its custom shader.
pub struct GhostModel {
    pub buildable: BuildableId,
    green: ModelInst,
    red: ModelInst,
    scale: f32,
    pub z_offset: f32,
}

impl GhostModel {
    pub fn new(lib: &mut Library, b: BuildableId) -> Option<GhostModel> {
        let cfg = lib.buildable_cfg(b.id());
        let path = cfg.models[0].clone()?;
        let res = lib.model(&path)?;
        let mut mk = |shader: &str| {
            // ent.customShader: every surface gets the shader
            let skin: std::collections::HashMap<String, String> = res.surfs.iter().map(|s| (s.name.to_lowercase(), shader.to_string())).collect();
            lib.instance(&res, Some(&skin))
        };
        let green = mk("gfx/misc/greenbuild");
        let red = mk("gfx/misc/redbuild");
        Some(GhostModel { buildable: b, green, red, scale: cfg.scale, z_offset: cfg.z_offset })
    }
    pub fn nodes(&self) -> Vec<Gd<Node3D>> {
        vec![self.green.root.clone(), self.red.root.clone()]
    }
    /// Place it (axis rows forward, left, up; origin already offset by
    /// zOffset), scaled by modelScale; `valid` picks the green shader.
    pub fn update(&mut self, origin: &Vec3, axis: &[Vec3; 3], valid: bool) {
        let t = Transform3D::new(scale_basis(tag_basis(axis), self.scale), q2g(origin));
        for (m, show) in [(&mut self.green, valid), (&mut self.red, !valid)] {
            m.set_frames(0, 0, 0.0);
            m.set_transform(t);
            m.set_visible(show);
        }
    }
    pub fn hide(&mut self) {
        self.green.set_visible(false);
        self.red.set_visible(false);
    }
    pub fn free(self) {
        self.green.free();
        self.red.free();
    }
}

/// A projectile (`CG_Missile`): the weapon's missile model oriented along its
/// flight, or its additive sprite, plus the missile dlight.
pub struct MissileVis {
    pub key: String,
    model: Option<ModelInst>,
    sprite: Option<Gd<godot::classes::MeshInstance3D>>,
    light: Option<Gd<godot::classes::OmniLight3D>>,
}

impl MissileVis {
    pub fn new(lib: &mut Library, weapon: i32, mode: usize, key: String) -> Option<MissileVis> {
        use godot::classes::base_material_3d::{BlendMode, ShadingMode, Transparency};
        let wid = WeaponId::from_num(weapon as u8).filter(|_| weapon > 0)?;
        let cfg = lib.weapon_cfg(wid.id());
        let mode = mode.min(2);
        let model_path = cfg.missile_model[mode].clone().or_else(|| cfg.missile_model[0].clone());
        let sprite_def = cfg.missile_sprite[mode].clone().or_else(|| cfg.missile_sprite[0].clone());
        let model = model_path.and_then(|p| lib.model(&p)).map(|r| lib.instance(&r, None));
        let sprite = if model.is_none() {
            sprite_def.and_then(|(size, shader)| {
                let tex = lib.texture(&shader)?;
                let mut quad = godot::classes::QuadMesh::new_gd();
                quad.set_size(Vector2::splat(size * 2.0 * crate::geom::SCALE));
                let mut mat = godot::classes::StandardMaterial3D::new_gd();
                mat.set_texture(godot::classes::base_material_3d::TextureParam::ALBEDO, &tex);
                mat.set_billboard_mode(BillboardMode::ENABLED);
                mat.set_shading_mode(ShadingMode::UNSHADED);
                mat.set_blend_mode(BlendMode::ADD);
                mat.set_transparency(Transparency::ALPHA);
                let mut mi = godot::classes::MeshInstance3D::new_alloc();
                mi.set_mesh(&quad);
                mi.set_material_override(&mat);
                Some(mi)
            })
        } else {
            None
        };
        let light = cfg.missile_dlight[mode].or(cfg.missile_dlight[0]).map(|c| {
            let mut l = godot::classes::OmniLight3D::new_alloc();
            l.add_to_group(crate::gfx::DLIGHT_GROUP);
            l.set_color(Color::from_rgb(c[0], c[1], c[2]));
            l.set_param(godot::classes::light_3d::Param::RANGE, 6.0);
            l.set_param(godot::classes::light_3d::Param::ENERGY, 2.0);
            l
        });
        if model.is_none() && sprite.is_none() {
            return None;
        }
        Some(MissileVis { key, model, sprite, light })
    }

    pub fn nodes(&self) -> Vec<Gd<Node3D>> {
        let mut v = Vec::new();
        if let Some(m) = &self.model {
            v.push(m.root.clone());
        }
        if let Some(s) = &self.sprite {
            v.push(s.clone().upcast());
        }
        if let Some(l) = &self.light {
            v.push(l.clone().upcast());
        }
        v
    }

    pub fn update(&mut self, origin: &Vec3, velocity: &Vec3) {
        let p = q2g(origin);
        if let Some(m) = self.model.as_mut() {
            let ang = trem_sim::kernel::qmath::vectoangles(velocity);
            m.set_frames(0, 0, 0.0);
            m.set_transform(Transform3D::new(model_basis(&ang), p));
        }
        if let Some(s) = self.sprite.as_mut() {
            s.set_position(p);
        }
        if let Some(l) = self.light.as_mut() {
            l.set_position(p);
        }
    }

    pub fn free(self) {
        for mut n in self.nodes() {
            n.queue_free();
        }
    }
}
