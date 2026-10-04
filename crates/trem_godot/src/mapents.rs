//! Map entities on screen (cg_ents.c / cg_animmapobj.c): brush movers
//! (CG_Mover, with their inline-model meshes and optional model2 md3), model
//! doors (CG_ModelDoor), animated map objects (CG_AnimMapObj), light flares
//! (CG_LightFlare), looping entity sounds (CG_EntityEffects) and auto
//! triggering speakers (CG_Speaker). Presentation only.

use crate::audio::new_3d;
use crate::geom::{q2g, SCALE};
use crate::md3::{Animation, LerpFrame};
use crate::models::{model_basis, Library, ModelInst};
use godot::classes::base_material_3d::{BillboardMode, BlendMode, Flags as MatFlags, ShadingMode, TextureParam, Transparency};
use godot::classes::{AudioStreamPlayer3D, MeshInstance3D, Node3D, QuadMesh, StandardMaterial3D};
use godot::prelude::*;
use std::collections::HashMap;
use trem_sim::gen::constants as k;
use trem_sim::kernel::world::{EType, World};

/// Brush model meshes by inline model number (built when the map loads).
pub type BrushMeshes = HashMap<i32, Vec<Gd<MeshInstance3D>>>;

struct Md3Vis {
    key: String,
    inst: ModelInst,
    lf: LerpFrame,
    door_state: i32,
    anim_init: bool,
    anim_playing: bool,
    anim_last: bool,
    misc_time: i32,
}

struct Flare {
    node: Gd<MeshInstance3D>,
    mat: Gd<StandardMaterial3D>,
    status: bool,
    last_time: i32,
    last_ratio: f32,
    last_radius: f32,
}

/// `cg_lightFlare` values (cg_ents.c).
pub const FLARE_OFF: i32 = 0;
#[allow(dead_code)]
pub const FLARE_NOFADE: i32 = 1;
pub const FLARE_TIMEFADE: i32 = 2;
pub const FLARE_REALFADE: i32 = 3;

#[derive(Default)]
pub struct MapEnts {
    brushes: HashMap<usize, Gd<Node3D>>,
    model2: HashMap<usize, ModelInst>,
    md3s: HashMap<usize, Md3Vis>,
    flares: HashMap<usize, Flare>,
    /// looping sounds by (entity, source): 0 the entity's own loopSound /
    /// the jetpack, 1 its weapon or projectile, 2 the lucifer cannon's
    /// overcharge warning (trap_S_AddLoopingSound keeps one per source)
    loops: HashMap<(usize, u8), (String, Gd<AudioStreamPlayer3D>)>,
    speaker_time: HashMap<usize, i32>,
    seed: u32,
    /// `cg_lightFlare` (None: the default, FLARE_REALFADE).
    pub light_flare: Option<i32>,
    /// `cg.clientFrame`: flares recalculate every other frame.
    client_frame: u32,
}

/// Where an entity's sounds come from (`CG_SetEntitySoundPosition`): brush
/// models use the middle of their inline model.
pub fn sound_origin(w: &World, i: usize) -> [f32; 3] {
    let e = &w.ents[i];
    if e.bmodel {
        if let Some(m) = w.cm.models.get(e.model_index.max(0) as usize) {
            return [e.origin[0] + (m.mins[0] + m.maxs[0]) * 0.5, e.origin[1] + (m.mins[1] + m.maxs[1]) * 0.5, e.origin[2] + (m.mins[2] + m.maxs[2]) * 0.5];
        }
    }
    if e.etype == EType::Speaker || e.etype == EType::ModelDoor {
        return e.s_origin;
    }
    e.origin
}

impl MapEnts {
    fn crandom(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1103515245).wrapping_add(12345);
        ((self.seed >> 16) & 0x7fff) as f32 / 0x7fff as f32 * 2.0 - 1.0
    }

    /// Update every map entity; returns nodes to add to the scene and
    /// (sound path, origin) pairs for one-shot speaker sounds.
    #[allow(clippy::too_many_arguments)]
    /// The drawn node of mover `i` (its brush model), if it is drawn.
    pub fn brush_node(&self, i: usize) -> Option<Gd<Node3D>> {
        self.brushes.get(&i).cloned()
    }

    pub fn sync(&mut self, w: &World, lib: Option<&mut Library>, meshes: &mut BrushMeshes, now: i32, view: [f32; 3], view_fwd: [f32; 3], me: Option<usize>, light: impl Fn(&[f32; 3]) -> ([f32; 3], [f32; 3], Vector3)) -> (Vec<Gd<Node3D>>, Vec<(String, [f32; 3])>) {
        let mut add: Vec<Gd<Node3D>> = Vec::new();
        let mut sounds = Vec::new();
        let mut lib = lib;
        let mut alive_brush = Vec::new();
        let mut alive_md3 = Vec::new();
        let mut alive_flare = Vec::new();
        let mut alive_loop = Vec::new();
        self.client_frame = self.client_frame.wrapping_add(1);
        for i in 0..w.num_entities {
            let e = &w.ents[i];
            if !e.inuse {
                continue;
            }
            match e.etype {
                EType::Mover if e.bmodel => {
                    if !self.brushes.contains_key(&i) {
                        let mut root = Node3D::new_alloc();
                        root.set_name(&format!("mover_{i}"));
                        if let Some(list) = meshes.remove(&e.model_index) {
                            for mi in list {
                                root.add_child(&mi);
                            }
                        }
                        add.push(root.clone());
                        self.brushes.insert(i, root);
                        if e.model_index2 > 0 {
                            if let Some(l) = lib.as_deref_mut() {
                                if let Some(res) = w.model_names.get(e.model_index2 as usize).and_then(|p| l.model(p)) {
                                    let inst = l.instance(&res, None);
                                    add.push(inst.root.clone());
                                    self.model2.insert(i, inst);
                                }
                            }
                        }
                    }
                    let t = Transform3D::new(model_basis(&e.angles), q2g(&e.origin));
                    if let Some(n) = self.brushes.get_mut(&i) {
                        n.set_transform(t);
                    }
                    if let Some(m) = self.model2.get_mut(&i) {
                        m.set_transform(t);
                        let (a, d, l) = light(&e.origin);
                        m.set_light(a, d, l);
                    }
                    alive_brush.push(i);
                }
                EType::ModelDoor | EType::AnimMapObj => {
                    let Some(l) = lib.as_deref_mut() else { continue };
                    if e.model_index <= 0 || (e.etype == EType::AnimMapObj && e.e_flags & k::EF_NODRAW != 0) {
                        continue;
                    }
                    let path = w.model_names.get(e.model_index as usize).cloned().unwrap_or_default();
                    let key = format!("{path}:{:?}", e.etype);
                    if self.md3s.get(&i).map(|v| v.key != key).unwrap_or(true) {
                        if let Some(old) = self.md3s.remove(&i) {
                            old.inst.free();
                        }
                        let Some(res) = l.model(&path) else { continue };
                        let inst = l.instance(&res, None);
                        add.push(inst.root.clone());
                        let lf = LerpFrame { frame_time: now, old_frame_time: now, frame: e.misc, old_frame: e.misc, ..Default::default() };
                        self.md3s.insert(i, Md3Vis { key, inst, lf, door_state: e.legs_anim, anim_init: false, anim_playing: false, anim_last: false, misc_time: now });
                    }
                    let v = self.md3s.get_mut(&i).unwrap();
                    let origin = e.pos.tr_base;
                    let mut basis = model_basis(&e.s_angles);
                    if e.etype == EType::ModelDoor {
                        // CG_ModelDoor
                        let fl = if e.torso_anim != 0 { 1000 / e.torso_anim } else { 1000 };
                        let anim = Animation { first_frame: e.misc, num_frames: e.weapon, loop_frames: 0, frame_lerp: fl, initial_lerp: fl, reversed: e.legs_anim == 0, flipflop: false };
                        if e.legs_anim != v.door_state {
                            v.lf.animation_time = v.lf.frame_time + anim.initial_lerp;
                            v.door_state = e.legs_anim;
                        }
                        v.lf.animation = Some(anim);
                        v.lf.run(now);
                        let s = e.origin2;
                        basis = Basis::from_cols(basis.col_a() * s[0], basis.col_b() * s[2], basis.col_c() * s[1]);
                    } else {
                        // CG_AnimMapObj
                        let mut anim = Animation { first_frame: e.misc, num_frames: e.weapon, loop_frames: e.torso_anim, ..Default::default() };
                        if anim.num_frames < 0 {
                            anim.num_frames = -anim.num_frames;
                            anim.reversed = true;
                        }
                        let fl = if e.legs_anim == 0 { 1000 } else { 1000 / e.legs_anim };
                        anim.frame_lerp = fl;
                        anim.initial_lerp = fl;
                        let running = e.e_flags & k::EF_MOVER_STOP == 0;
                        if anim.loop_frames == 0 {
                            anim.num_frames += 2;
                            if !v.anim_init {
                                v.anim_init = true;
                                v.anim_playing = running;
                            } else if v.anim_last != running {
                                v.anim_playing = true;
                                v.lf.animation_time = now;
                                v.lf.frame_time = now;
                            }
                            v.anim_last = running;
                        }
                        if v.lf.animation.is_none() {
                            v.lf.animation_time = now + anim.initial_lerp;
                        }
                        v.lf.animation = Some(anim);
                        if running || v.anim_playing {
                            let delta = now - v.misc_time;
                            if delta > 900 {
                                v.lf.old_frame_time += delta;
                                v.lf.frame_time += delta;
                            }
                            v.lf.run(now);
                            v.misc_time = now;
                        }
                        if e.angles2[0] != 0.0 {
                            let sc = e.angles2[0];
                            basis = Basis::from_cols(basis.col_a() * sc, basis.col_b() * sc, basis.col_c() * sc);
                        }
                    }
                    v.inst.set_frames(v.lf.old_frame, v.lf.frame, v.lf.backlerp);
                    v.inst.set_transform(Transform3D::new(basis, q2g(&origin)));
                    let (a, d, ld) = light(&origin);
                    v.inst.set_light(a, d, ld);
                    alive_md3.push(i);
                }
                EType::LightFlare => {
                    if e.e_flags & k::EF_NODRAW != 0 {
                        continue;
                    }
                    let Some(l) = lib.as_deref_mut() else { continue };
                    if !self.flares.contains_key(&i) {
                        let shader = w.shader_names.get(e.model_index as usize).cloned().unwrap_or_default();
                        let info = l.shader_info(&shader);
                        let tex = l.texture(&info.image.clone().unwrap_or(shader.clone())).or_else(|| l.texture(&shader));
                        let mut mat = StandardMaterial3D::new_gd();
                        mat.set_shading_mode(ShadingMode::UNSHADED);
                        mat.set_transparency(Transparency::ALPHA);
                        mat.set_blend_mode(BlendMode::ADD);
                        mat.set_billboard_mode(BillboardMode::ENABLED);
                        mat.set_flag(MatFlags::DISABLE_DEPTH_TEST, true);
                        if let Some(t) = tex {
                            mat.set_texture(TextureParam::ALBEDO, &t);
                        }
                        let mut q = QuadMesh::new_gd();
                        q.set_size(Vector2::new(2.0, 2.0));
                        let mut node = MeshInstance3D::new_alloc();
                        node.set_mesh(&q);
                        node.set_material_override(&mat);
                        add.push(node.clone().upcast());
                        self.flares.insert(i, Flare { node, mat, status: false, last_time: now, last_ratio: 1.0, last_radius: 0.0 });
                    }
                    let mode = self.light_flare.unwrap_or(FLARE_REALFADE);
                    let recalc = self.client_frame % 2 == 0;
                    let f = self.flares.get_mut(&i).unwrap();
                    f.node.set_visible(false);
                    alive_flare.push(i);
                    if mode == FLARE_OFF {
                        continue;
                    }
                    let entity_num = me.map(|m| m as i32).unwrap_or(-1);
                    // no line of sight between the view and the flare source
                    let tr = w.trace(&view, None, None, &e.angles2, entity_num, k::MASK_SHOT);
                    if tr.fraction < 1.0 || tr.allsolid {
                        continue;
                    }
                    let src = e.pos.tr_base;
                    let delta = [src[0] - view[0], src[1] - view[1], src[2] - view[2]];
                    let len = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
                    let dn = if len > 0.0 { [delta[0] / len, delta[1] / len, delta[2] / len] } else { [0.0; 3] };
                    // too close to the camera, or behind the view plane
                    if len < e.generic1 as f32 || dn[0] * view_fwd[0] + dn[1] * view_fwd[1] + dn[2] * view_fwd[2] < 0.0 {
                        continue;
                    }
                    let mut ratio = 1.0f32;
                    let mut radius;
                    if recalc {
                        radius = len / e.origin2[0];
                        let src_radius = if e.origin2[2] == 0.0 { radius / 2.0 } else { len / e.origin2[2] };
                        let max_angle = e.origin2[1];
                        if max_angle > 0.0 {
                            let (fwd, _, _) = trem_sim::kernel::qmath::angle_vectors(&e.s_angles);
                            let d = (dn[0] * fwd[0] + dn[1] * fwd[1] + dn[2] * fwd[2]).clamp(-1.0, 1.0);
                            radius *= (1.0 - (180.0 - d.acos().to_degrees()) / max_angle).max(0.0);
                        }
                        if !(radius >= 0.0) {
                            radius = 0.0;
                        }
                        let end = [src[0] - radius * dn[0], src[1] - radius * dn[1], src[2] - radius * dn[2]];
                        let start = [view[0] + radius * dn[0], view[1] + radius * dn[1], view[2] + radius * dn[2]];
                        if mode == FLARE_REALFADE {
                            // "correct" flares
                            let tr = w.bisphere_trace(&view, &end, 1.0, src_radius, entity_num, k::MASK_SHOT);
                            ratio = if tr.fraction < 1.0 { tr.lateral_fraction } else { 1.0 };
                        } else {
                            let (mn, mx) = ([-src_radius; 3], [src_radius; 3]);
                            let tr = w.trace(&start, Some(&mn), Some(&mx), &end, entity_num, k::MASK_SHOT);
                            let blocked = tr.fraction < 1.0 || tr.startsolid;
                            if mode == FLARE_TIMEFADE {
                                if blocked && f.status {
                                    f.status = false;
                                    f.last_time = now;
                                } else if !blocked && !f.status {
                                    f.status = true;
                                    f.last_time = now;
                                }
                                let fade = e.s_time;
                                if f.status {
                                    if f.last_time + fade > now {
                                        ratio = (now - f.last_time) as f32 / fade as f32;
                                    }
                                } else if f.last_time + fade > now {
                                    ratio = 1.0 - (now - f.last_time) as f32 / fade as f32;
                                } else {
                                    ratio = 0.0;
                                }
                            } else if blocked {
                                ratio = 0.0; // FLARE_NOFADE
                            }
                        }
                    } else {
                        ratio = f.last_ratio;
                        radius = f.last_radius;
                    }
                    f.last_ratio = ratio;
                    f.last_radius = radius;
                    let mut alpha = 1.0;
                    if ratio < 1.0 {
                        radius *= ratio;
                        alpha = ratio;
                    }
                    if !(radius > 0.0) {
                        continue;
                    }
                    f.node.set_visible(true);
                    f.node.set_position(q2g(&src));
                    let s = radius * SCALE;
                    f.node.set_scale(Vector3::new(s, s, s));
                    f.mat.set_albedo(Color::from_rgba(1.0, 1.0, 1.0, alpha));
                }
                EType::Speaker => {
                    // CG_Speaker: auto-triggering speakers repeat client side
                    if e.s_client_num != 0 || e.frame != 0 {
                        let next = *self.speaker_time.get(&i).unwrap_or(&0);
                        if e.s_client_num != 0 && now >= next {
                            if let Some(p) = w.sound_name(e.event_parm) {
                                sounds.push((p.to_string(), e.s_origin));
                            }
                            let c = self.crandom();
                            self.speaker_time.insert(i, now + e.frame * 100 + (e.s_client_num as f32 * 100.0 * c) as i32);
                        }
                    }
                }
                _ => {}
            }
            // looping sounds (CG_EntityEffects)
            // CG_PlayerUpgrades: the jetpack hums by vertical speed (players)
            let jet = if e.etype == EType::Player && i < trem_sim::kernel::world::MAX_CLIENTS {
                let st = &w.clients[i].ps.stats;
                let j = trem_sim::gen::upgrades::UpgradeId::Jetpack.num() as i32;
                if trem_sim::kernel::bg::inventory_contains_upgrade(j, st) && trem_sim::kernel::bg::upgrade_is_active(j, st) {
                    let vz = e.pos.tr_delta[2];
                    Some(if vz > 10.0 { "sound/upgrades/jetpack/hi.wav" } else if vz < -10.0 { "sound/upgrades/jetpack/low.wav" } else { "sound/upgrades/jetpack/idle.wav" })
                } else {
                    None
                }
            } else {
                None
            };
            let mut wants: Vec<(u8, String)> = Vec::new();
            if let Some(p) = jet.map(str::to_string).or_else(|| if e.loop_sound > 0 && e.etype != EType::Buildable { w.sound_name(e.loop_sound).map(str::to_string) } else { None }) {
                wants.push((0, p));
            }
            // CG_AddPlayerWeapon: firingSound while the current mode fires, else
            // the weapon's idleSound; CG_Missile: the projectile's missileSound
            if let Some(l) = lib.as_deref_mut() {
                let player = e.etype == EType::Player && i < trem_sim::kernel::world::MAX_CLIENTS;
                let (weapon, mode, flags) = if player { let ps = &w.clients[i].ps; (ps.weapon, ps.generic1, ps.e_flags) } else { (e.weapon, e.generic1, e.e_flags) };
                let mode = if (1..=3).contains(&mode) { mode as usize - 1 } else { 0 };
                let cfg = trem_sim::gen::weapons::WeaponId::from_num(weapon.clamp(0, 255) as u8).filter(|_| weapon > 0).map(|wid| l.weapon_cfg(wid.id()));
                if let Some(cfg) = cfg {
                    if player && w.clients[i].ps.stats[trem_sim::gen::enums::Stat::Health as usize] > 0 {
                        let firing = [k::EF_FIRING, k::EF_FIRING2, k::EF_FIRING3][mode] & flags != 0;
                        if let Some(p) = cfg.firing_sound[mode].clone().filter(|_| firing).or_else(|| cfg.idle_sound.clone()) {
                            wants.push((1, p));
                        }
                        // the lucifer cannon's overcharge warning, unheard by aliens
                        let viewer_team = me.map(|m| w.clients[m].ps.stats[trem_sim::gen::enums::Stat::Team as usize]).unwrap_or(0);
                        if weapon == trem_sim::gen::weapons::WeaponId::Lcannon.num() as i32 && flags & k::EF_WARN_CHARGE != 0 && viewer_team != trem_sim::gen::enums::Team::Aliens as i32 {
                            wants.push((2, if Some(i) == me { "models/weapons/lcannon/warning.wav".to_string() } else { "models/weapons/lcannon/warning2.wav|models/weapons/lcannon/warning.wav".to_string() }));
                        }
                    } else if e.etype == EType::Missile {
                        let m = (e.generic1 - 1).clamp(0, 2) as usize;
                        if let Some(p) = cfg.missile_sound[m].clone() {
                            wants.push((1, p));
                        }
                    }
                }
            }
            for (slot, want) in wants {
                let Some(l) = lib.as_deref_mut() else { continue };
                let origin = sound_origin(w, i);
                let key = (i, slot);
                if self.loops.get(&key).map(|(s, _)| *s != want).unwrap_or(true) {
                    if let Some((_, mut p)) = self.loops.remove(&key) {
                        p.queue_free();
                    }
                    let Some(stream) = want.split('|').find_map(|p| l.sound(p)) else { continue };
                    if let Ok(mut wav) = stream.clone().try_cast::<godot::classes::AudioStreamWav>() {
                        wav.set_loop_mode(godot::classes::audio_stream_wav::LoopMode::FORWARD);
                        let frames = wav.get_length() * wav.get_mix_rate() as f64;
                        wav.set_loop_end(frames as i32);
                    }
                    let mut p = new_3d();
                    p.set_stream(&stream);
                    p.set_autoplay(true);
                    add.push(p.clone().upcast());
                    self.loops.insert(key, (want, p));
                }
                if let Some((_, p)) = self.loops.get_mut(&key) {
                    p.set_position(q2g(&origin));
                    if !p.is_playing() && p.is_inside_tree() {
                        p.play();
                    }
                }
                alive_loop.push(key);
            }
        }
        let gone: Vec<usize> = self.brushes.keys().copied().filter(|i| !alive_brush.contains(i)).collect();
        for i in gone {
            if let Some(mut n) = self.brushes.remove(&i) {
                n.queue_free();
            }
            if let Some(m) = self.model2.remove(&i) {
                m.free();
            }
        }
        let gone: Vec<usize> = self.md3s.keys().copied().filter(|i| !alive_md3.contains(i)).collect();
        for i in gone {
            if let Some(v) = self.md3s.remove(&i) {
                v.inst.free();
            }
        }
        let gone: Vec<usize> = self.flares.keys().copied().filter(|i| !alive_flare.contains(i)).collect();
        for i in gone {
            if let Some(mut f) = self.flares.remove(&i) {
                f.node.queue_free();
            }
        }
        let gone: Vec<(usize, u8)> = self.loops.keys().copied().filter(|i| !alive_loop.contains(i)).collect();
        for i in gone {
            if let Some((_, mut p)) = self.loops.remove(&i) {
                p.queue_free();
            }
        }
        (add, sounds)
    }
}
