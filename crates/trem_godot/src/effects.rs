//! The cgame glue between game events / entity state and the particle and
//! trail systems (`particles.rs`, `trails.rs`): which system is spawned where,
//! following cg_event.c, cg_weapons.c (impacts, muzzles, bleeding, shotgun
//! pattern), cg_ents.c (missiles, lev2 zap chains, tesla trails, particle
//! system entities), cg_buildable.c (damaged / destroyed structures) and
//! cg_players.c (jetpack). Presentation only: nothing here feeds the sim.

use crate::actors::{Actor, ViewHand};
use crate::geom::SCALE;
use crate::models::Library;
use crate::particles::{Attach, AttachPos, FxRenderer, ParticleLib, Particles};
use crate::trails::{TrailLib, Trails};
use godot::prelude::*;
use std::collections::HashMap;
use trem_sim::gen::buildables::BuildableId;
use trem_sim::gen::constants as k;
use trem_sim::gen::enums::{Event, SpectatorState, Stat, Team};
use trem_sim::gen::upgrades::UpgradeId;
use trem_sim::gen::weapons::WeaponId;
use trem_sim::kernel::bg;
use trem_sim::kernel::qmath::{self as qm, Vec3};
use trem_sim::kernel::world::{EType, GameEvent, World, MAX_CLIENTS};

/// `cgs.media` particle / trail handles (cg_main.c CG_RegisterGraphics).
#[derive(Default)]
struct Media {
    alien_bleed: Option<usize>,
    human_bleed: Option<usize>,
    alien_buildable_bleed: Option<usize>,
    human_buildable_bleed: Option<usize>,
    alien_evolve: Option<usize>,
    alien_acidtube: Option<usize>,
    human_damaged: Option<usize>,
    alien_damaged: Option<usize>,
    human_destroyed: Option<usize>,
    alien_destroyed: Option<usize>,
    jet_ascend: Option<usize>,
    jet_descend: Option<usize>,
    jet_hover: Option<usize>,
    poison_cloud_fp: Option<usize>,
    tesla_zap: Option<usize>,
    level2_zap: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Jps {
    Ascending,
    Descending,
    Hovering,
}

/// cg_tracerChance / cg_tracerWidth / cg_tracerLength defaults.
const TRACER_CHANCE: f32 = 0.4;
const TRACER_WIDTH: f32 = 1.0;
const TRACER_LENGTH: f32 = 100.0;

/// `cg_teslaTrailTime`.
const TESLA_TRAIL_TIME: i32 = 250;

pub struct Effects {
    plib: ParticleLib,
    tlib: TrailLib,
    ps: Particles,
    ts: Trails,
    fx: FxRenderer,
    media: Media,
    /// missile entity -> (weapon:mode key, particle id, trail id)
    missiles: HashMap<usize, (i32, u32, u32)>,
    /// cent->muzzlePS and the pending muzzlePsTrigger (weapon mode)
    muzzle: HashMap<usize, u32>,
    muzzle_trigger: HashMap<usize, usize>,
    jetpack: HashMap<usize, (u32, Jps)>,
    damaged: HashMap<usize, u32>,
    entity_ps: HashMap<usize, u32>,
    entity_ps_missing: std::collections::HashSet<usize>,
    /// (source, target) -> lev2 zap trail
    zaps: HashMap<(usize, usize), u32>,
    /// tesla source -> (trail, death time)
    tesla: HashMap<usize, (u32, i32)>,
    poison: Option<u32>,
    poisoned_last: bool,
    /// muzzle systems triggered this frame: (entity, weapon, mode)
    pending_muzzle: Vec<(usize, i32, usize)>,
    /// tracers to draw this frame: (start, finish, direction)
    tracers: Vec<(Vec3, Vec3, Vec3)>,
    marks: crate::marks::Marks,
    now: i32,
    rng: u32,
    /// cg_particles.c testPS / cg_trails.c testTS
    test_ps: u32,
    test_ts: u32,
}

/// Quake origin of a Godot position.
fn g2q(v: Vector3) -> Vec3 {
    [v.x / SCALE, -v.z / SCALE, v.y / SCALE]
}
fn g2q_dir(v: Vector3) -> Vec3 {
    [v.x, -v.z, v.y]
}

/// A Godot transform as an attachment position (rows forward, left, up).
fn transform_pos(t: Transform3D) -> AttachPos {
    let axis = |q: Vec3| {
        let g = t.basis * crate::geom::q2g_dir(&q);
        let mut d = g2q_dir(g);
        qm::normalize(&mut d);
        d
    };
    AttachPos { origin: g2q(t.origin), axis: [axis([1.0, 0.0, 0.0]), axis([0.0, 1.0, 0.0]), axis([0.0, 0.0, 1.0])], velocity: [0.0; 3] }
}

fn weapon_mode(generic1: i32) -> usize {
    // weaponMode <= WPM_NONE || >= WPM_NUM_WEAPONMODES -> WPM_PRIMARY
    if (1..=3).contains(&generic1) {
        (generic1 - 1) as usize
    } else {
        0
    }
}

impl Effects {
    /// Load every scripts/*.particle and scripts/*.trail from the data pk3s.
    pub fn new(lib: &mut Library, parent: &Gd<Node3D>) -> Self {
        let names: Vec<String> = lib.pk3.files().filter(|f| {
            let l = f.to_lowercase();
            l.starts_with("scripts/") && (l.ends_with(".particle") || l.ends_with(".trail"))
        }).cloned().collect();
        let files: Vec<(String, String)> = names.into_iter().filter_map(|n| lib.text(&n).map(|t| (n, t))).collect();
        let plib = ParticleLib::load(&files);
        let tlib = TrailLib::load(&files);
        for e in plib.errors.iter().chain(tlib.errors.iter()) {
            godot_warn!("fx script: {e}");
        }
        let h = |n: &str| plib.handle(n);
        let media = Media {
            alien_bleed: h("alienBleedPS"),
            human_bleed: h("humanBleedPS"),
            alien_buildable_bleed: h("alienBuildableBleedPS"),
            human_buildable_bleed: h("humanBuildableBleedPS"),
            alien_evolve: h("alienEvolvePS"),
            alien_acidtube: h("alienAcidTubePS"),
            human_damaged: h("humanBuildableDamagedPS"),
            alien_damaged: h("alienBuildableDamagedPS"),
            human_destroyed: h("humanBuildableDestroyedPS"),
            alien_destroyed: h("alienBuildableDestroyedPS"),
            jet_ascend: h("jetPackAscendPS"),
            jet_descend: h("jetPackDescendPS"),
            jet_hover: h("jetPackHoverPS"),
            poison_cloud_fp: h("firstPersonPoisonCloudPS"),
            tesla_zap: tlib.handle("models/buildables/tesla/zap"),
            level2_zap: tlib.handle("models/weapons/lev2zap/lightning"),
        };
        let fx = FxRenderer::new(lib, parent, &plib, &tlib);
        let marks = crate::marks::Marks::new(parent);
        Effects {
            plib,
            tlib,
            ps: Particles::new(),
            ts: Trails::new(),
            fx,
            media,
            missiles: HashMap::new(),
            muzzle: HashMap::new(),
            muzzle_trigger: HashMap::new(),
            jetpack: HashMap::new(),
            damaged: HashMap::new(),
            entity_ps: HashMap::new(),
            entity_ps_missing: Default::default(),
            zaps: HashMap::new(),
            tesla: HashMap::new(),
            poison: None,
            poisoned_last: false,
            pending_muzzle: Vec::new(),
            tracers: Vec::new(),
            marks,
            now: 0,
            rng: 0x1234_5678,
            test_ps: 0,
            test_ts: 0,
        }
    }

    /// Debug / test hook: spawn a named particle (or trail) system at a point.
    pub fn spawn_named(&mut self, name: &str, origin: Vec3, now: i32) -> bool {
        self.ps.set_time(now);
        if let Some(h) = self.plib.handle(name) {
            return self.spawn_point(Some(h), origin, Some([0.0, 0.0, 1.0])) != 0;
        }
        false
    }

    /// `CG_TestPS_f`: the named system 100 units in front of the view,
    /// pointing up (the last test system is destroyed first). False: no
    /// such system.
    pub fn test_ps(&mut self, name: &str, origin: Vec3, now: i32) -> bool {
        self.ps.set_time(now);
        let Some(h) = self.plib.handle(name) else { return false };
        self.destroy_test_ps();
        self.test_ps = self.spawn_point(Some(h), origin, Some([0.0, 0.0, 1.0]));
        true
    }

    /// `CG_DestroyTestPS_f`.
    pub fn destroy_test_ps(&mut self) {
        if self.ps.is_valid(self.test_ps) {
            self.ps.destroy(self.test_ps);
        }
        self.test_ps = 0;
    }

    /// `CG_TestTS_f`: the named trail system with its front on entity 0.
    pub fn test_ts(&mut self, name: &str, now: i32) -> bool {
        self.ts.set_time(now);
        let Some(h) = self.tlib.handle(name) else { return false };
        self.destroy_test_ts();
        self.test_ts = self.ts.spawn(&self.tlib, h, Attach::Entity(0), None);
        true
    }

    /// `CG_DestroyTestTS_f`.
    pub fn destroy_test_ts(&mut self) {
        if self.ts.is_valid(self.test_ts) {
            self.ts.destroy(self.test_ts);
        }
        self.test_ts = 0;
    }

    fn spawn_point(&mut self, h: Option<usize>, origin: Vec3, normal: Option<Vec3>) -> u32 {
        let Some(h) = h else { return 0 };
        let id = self.ps.spawn(&self.plib, h, Attach::Point(origin));
        if id != 0 {
            if let Some(n) = normal {
                self.ps.set_normal(id, n);
            }
        }
        id
    }

    /// The map's draw surfaces, which impact marks, shadows and creep are
    /// clipped to (`R_MarkFragments`).
    pub fn set_mark_world(&mut self, world: trem_sim::kernel::marks::MarkWorld) {
        self.marks.set_world(world);
    }

    /// `CG_MissileHitWall` (particle half; sounds stay in game.rs).
    fn missile_hit_wall(&mut self, lib: &mut Library, weapon: i32, mode: usize, origin: Vec3, dir: Vec3, charge: i32) {
        let Some(wid) = WeaponId::from_num(weapon as u8).filter(|_| weapon > 0) else { return };
        let cfg = lib.weapon_cfg(wid.id());
        if let Some(name) = cfg.impact_ps[mode].clone() {
            let h = self.plib.handle(&name);
            let id = self.spawn_point(h, origin, Some(dir));
            if id != 0 {
                self.ps.set_charge(id, charge);
            }
        }
        // impact mark, random orientation
        if let Some((radius, shader)) = cfg.impact_mark[mode].clone() {
            let orientation = self.ps_random() * 360.0;
            self.marks.impact(lib, &shader, origin, dir, orientation, radius, self.now);
        }
    }

    /// `CG_CalcMuzzlePoint`.
    fn muzzle_point(w: &World, ent: i32) -> Option<Vec3> {
        if ent < 0 || ent as usize >= w.num_entities || !w.ents[ent as usize].inuse {
            return None;
        }
        let i = ent as usize;
        let (mut m, angles, crouch) = if i < MAX_CLIENTS && w.ents[i].client {
            let ps = &w.clients[i].ps;
            let mut o = ps.origin;
            o[2] += ps.viewheight as f32;
            (o, ps.viewangles, None)
        } else {
            let e = &w.ents[i];
            let anim = e.legs_anim & !k::ANIM_TOGGLEBIT;
            let cr = anim == trem_sim::gen::enums::PlayerAnim::LegsWalkcr as i32 || anim == trem_sim::gen::enums::PlayerAnim::LegsIdlecr as i32;
            (e.pos.tr_base, e.angles, Some(cr))
        };
        if let Some(cr) = crouch {
            m[2] += if cr { k::CROUCH_VIEWHEIGHT } else { k::DEFAULT_VIEWHEIGHT } as f32;
        }
        let (f, _, _) = qm::angle_vectors(&angles);
        Some(qm::ma(&m, 14.0, &f))
    }

    /// `CG_Tracer`: a one-frame streak somewhere along the bullet's path.
    fn tracer(&mut self, source: Vec3, dest: Vec3) {
        let mut fwd = qm::sub(&dest, &source);
        let len = qm::normalize(&mut fwd);
        // start at least a little ways from the muzzle
        if len < 100.0 {
            return;
        }
        let begin = 50.0 + self.ps_random() * (len - 60.0);
        let end = (begin + TRACER_LENGTH).min(len);
        self.tracers.push((qm::ma(&source, begin, &fwd), qm::ma(&source, end, &fwd), fwd));
    }

    /// `CG_Bleed`.
    fn bleed(&mut self, w: &World, origin: Vec3, normal: Vec3, ent: i32) {
        if ent < 0 || ent as usize >= w.num_entities {
            return;
        }
        let e = &w.ents[ent as usize];
        let h = match e.etype {
            EType::Player => match w.team_of(ent as usize) {
                Team::Aliens => self.media.alien_bleed,
                Team::Humans => self.media.human_bleed,
                Team::None => return,
            },
            EType::Buildable => match e.team {
                Team::Aliens => self.media.alien_buildable_bleed,
                Team::Humans => self.media.human_buildable_bleed,
                Team::None => return,
            },
            _ => return,
        };
        let id = self.spawn_point(h, origin, Some(normal));
        if id != 0 {
            self.ps.set_cent(id, ent);
        }
    }

    /// `CG_MissileHitEntity`.
    fn missile_hit_entity(&mut self, w: &World, lib: &mut Library, weapon: i32, mode: usize, origin: Vec3, dir: Vec3, ent: i32, charge: i32) {
        let normal = qm::scale(&dir, -1.0);
        self.bleed(w, origin, normal, ent);
        let Some(wid) = WeaponId::from_num(weapon as u8).filter(|_| weapon > 0) else { return };
        if lib.weapon_cfg(wid.id()).always_impact[mode] {
            self.missile_hit_wall(lib, weapon, mode, origin, dir, charge);
        }
    }

    fn ps_random(&mut self) -> f32 {
        // cgame random(): presentation only, no parity needed
        self.rng = self.rng.wrapping_mul(1103515245).wrapping_add(12345);
        ((self.rng >> 16) & 0x7fff) as f32 / 32767.0
    }

    /// Event-driven systems (CG_EntityEvent).
    pub fn on_event(&mut self, w: &World, lib: &mut Library, e: &GameEvent, now: i32) {
        self.now = now;
        self.ps.set_time(now);
        self.ts.set_time(now);
        let mode = weapon_mode(e.generic1);
        match e.event {
            Event::MissileHit => {
                let dir = qm::byte_to_dir(e.parm);
                self.missile_hit_entity(w, lib, e.weapon, mode, e.origin, dir, e.other, e.charge);
            }
            Event::MissileMiss | Event::MissileMissMetal => {
                let dir = qm::byte_to_dir(e.parm);
                self.missile_hit_wall(lib, e.weapon, mode, e.origin, dir, e.charge);
            }
            Event::BulletHitWall | Event::BulletHitFlesh => {
                // CG_Bullet: a tracer from the shooter's muzzle (cg_tracerChance 0.4)
                if let Some(src) = Self::muzzle_point(w, e.other) {
                    if self.ps_random() < TRACER_CHANCE {
                        self.tracer(src, e.origin);
                    }
                }
                if e.event == Event::BulletHitFlesh {
                    self.bleed(w, e.origin, [0.0, 0.0, 1.0], e.parm);
                    return;
                }
                // the impact of WP_MACHINEGUN primary
                let dir = qm::byte_to_dir(e.parm);
                self.missile_hit_wall(lib, WeaponId::Rifle as i32, 0, e.origin, dir, 0);
            }
            Event::Shotgun => self.shotgun_pattern(w, lib, e),
            Event::HumanBuildableExplosion | Event::AlienBuildableExplosion => {
                let dir = qm::byte_to_dir(e.parm);
                let h = if e.event == Event::AlienBuildableExplosion { self.media.alien_destroyed } else { self.media.human_destroyed };
                self.spawn_point(h, e.origin, Some(dir));
            }
            Event::AlienEvolve if e.entity >= 0 => {
                if let Some(h) = self.media.alien_evolve {
                    self.ps.spawn(&self.plib, h, Attach::Entity(e.entity));
                }
            }
            Event::AlienAcidtube if e.entity >= 0 => {
                if let Some(h) = self.media.alien_acidtube {
                    let id = self.ps.spawn(&self.plib, h, Attach::Entity(e.entity));
                    if id != 0 {
                        self.ps.set_normal(id, qm::byte_to_dir(e.parm));
                    }
                }
            }
            Event::Teslatrail => {
                // other = target, parm = source (the tesla)
                let (source, target) = (e.parm, e.other);
                if source < 0 || target < 0 {
                    return;
                }
                let valid = self.tesla.get(&(source as usize)).is_some_and(|t| self.ts.is_valid(t.0));
                if !valid {
                    if let Some(h) = self.media.tesla_zap {
                        let id = self.ts.spawn(&self.tlib, h, Attach::Entity(source), Some(Attach::Entity(target)));
                        if id != 0 {
                            self.ts.set_offset(id, true, [0.0, 0.0, 28.0]);
                            self.tesla.insert(source as usize, (id, now + TESLA_TRAIL_TIME));
                        }
                    }
                }
            }
            Event::FireWeapon | Event::FireWeapon2 | Event::FireWeapon3 if e.entity >= 0 => {
                // CG_FireWeapon: muzzlePsTrigger
                let m = e.event as usize - Event::FireWeapon as usize;
                let Some(wid) = WeaponId::from_num(e.weapon as u8).filter(|_| e.weapon > 0) else { return };
                if lib.weapon_cfg(wid.id()).muzzle_ps[m].is_some() {
                    let i = e.entity as usize;
                    let live = self.muzzle.get(&i).is_some_and(|&id| self.ps.is_valid(id) && self.ps.is_infinite(id));
                    if !live {
                        self.muzzle_trigger.insert(i, m);
                    }
                }
            }
            _ => {}
        }
    }

    /// `CG_ShotgunPattern`: redo the server's pellet traces for the splashes.
    fn shotgun_pattern(&mut self, w: &World, lib: &mut Library, e: &GameEvent) {
        let mut seed = e.parm;
        let mut forward = [0.0; 3];
        qm::normalize2(&e.origin2, &mut forward);
        let right = qm::perpendicular_vector(&forward);
        let up = qm::cross(&forward, &right);
        let shotgun = WeaponId::Shotgun as i32;
        for _ in 0..k::SHOTGUN_PELLETS {
            let r = trem_sim::kernel::weapon::q_crandom(&mut seed) * k::SHOTGUN_SPREAD as f32 * 16.0;
            let u = trem_sim::kernel::weapon::q_crandom(&mut seed) * k::SHOTGUN_SPREAD as f32 * 16.0;
            let mut end = qm::ma(&e.origin, 8192.0 * 16.0, &forward);
            end = qm::ma(&end, r, &right);
            end = qm::ma(&end, u, &up);
            let tr = w.trace(&e.origin, None, None, &end, e.other, k::MASK_SHOT);
            if tr.surface_flags & trem_sim::gen::enums::surface::NOIMPACT as i32 != 0 {
                continue;
            }
            let t = tr.entity_num;
            let hit_ent = t >= 0 && (t as usize) < w.num_entities && matches!(w.ents[t as usize].etype, EType::Player | EType::Buildable);
            if hit_ent {
                self.missile_hit_entity(w, lib, shotgun, 0, tr.endpos, tr.plane.normal, t, 0);
            } else {
                self.missile_hit_wall(lib, shotgun, 0, tr.endpos, tr.plane.normal, 0);
            }
        }
    }

    /// Per-frame entity-driven systems (CG_AddPacketEntities' share).
    pub fn sync(&mut self, w: &World, lib: &mut Library, me: Option<usize>, now: i32) {
        self.ps.set_time(now);
        self.ts.set_time(now);
        let mut seen_missiles = Vec::new();
        for i in 0..w.num_entities {
            let e = &w.ents[i];
            if !e.inuse || e.free_after_event {
                continue;
            }
            match e.etype {
                EType::Missile => {
                    // CG_Missile: missileParticleSystem / missileTrailSystem
                    seen_missiles.push(i);
                    let key = e.weapon * 4 + e.generic1.clamp(0, 3);
                    if self.missiles.get(&i).is_some_and(|m| m.0 == key) {
                        continue;
                    }
                    if let Some((_, p, t)) = self.missiles.remove(&i) {
                        self.ps.destroy(p);
                        self.ts.destroy(t);
                    }
                    let Some(wid) = WeaponId::from_num(e.weapon as u8).filter(|_| e.weapon > 0) else { continue };
                    let cfg = lib.weapon_cfg(wid.id());
                    let mode = weapon_mode(e.generic1);
                    let mut p = 0;
                    if let Some(h) = cfg.missile_ps[mode].as_deref().and_then(|n| self.plib.handle(n)) {
                        p = self.ps.spawn(&self.plib, h, Attach::Entity(i as i32));
                        if p != 0 {
                            self.ps.set_charge(p, e.torso_anim);
                        }
                    }
                    let mut t = 0;
                    if let Some(h) = cfg.missile_ts[mode].as_deref().and_then(|n| self.tlib.handle(n)) {
                        t = self.ts.spawn(&self.tlib, h, Attach::Entity(i as i32), None);
                    }
                    self.missiles.insert(i, (key, p, t));
                }
                EType::Buildable => self.buildable_effects(w, i),
                EType::ParticleSystem => self.particle_system_entity(w, i),
                EType::Player => {
                    self.jetpack_effects(w, i);
                    self.muzzle_effects(w, i);
                }
                _ => {}
            }
        }
        // missiles that exploded or vanished
        let gone: Vec<usize> = self.missiles.keys().copied().filter(|i| !seen_missiles.contains(i)).collect();
        for i in gone {
            let (_, p, t) = self.missiles.remove(&i).unwrap();
            self.ps.destroy(p);
            self.ts.destroy(t);
        }
        // structures / players that vanished
        for map in [&mut self.damaged, &mut self.entity_ps, &mut self.muzzle] {
            let gone: Vec<usize> = map.keys().copied().filter(|&i| !w.ents[i].inuse).collect();
            for i in gone {
                let id = map.remove(&i).unwrap();
                self.ps.destroy(id);
            }
        }
        let gone: Vec<usize> = self.jetpack.keys().copied().filter(|&i| !w.ents[i].inuse || w.ents[i].etype != EType::Player).collect();
        for i in gone {
            let (id, _) = self.jetpack.remove(&i).unwrap();
            self.ps.destroy(id);
        }
        self.zap_chains(w);
        self.tesla_trails(w, now);
        self.poison_cloud(w, me);
    }

    /// `CG_BuildableParticleEffects` + the EF_NODRAW destroy of CG_Buildable.
    fn buildable_effects(&mut self, w: &World, i: usize) {
        let e = &w.ents[i];
        let Some(b) = e.buildable else { return };
        if e.e_flags & k::EF_NODRAW != 0 {
            if let Some(id) = self.damaged.remove(&i) {
                self.ps.destroy(id);
            }
            return;
        }
        if e.e_flags & k::EF_B_SPAWNED == 0 {
            return;
        }
        let frac = e.generic1 as f32 / BuildableId::def(b).health as f32;
        let valid = self.damaged.get(&i).is_some_and(|&id| self.ps.is_valid(id));
        if frac < 0.33 && !valid {
            let h = if b.def().team == Team::Aliens { self.media.alien_damaged } else { self.media.human_damaged };
            if let Some(h) = h {
                let id = self.ps.spawn(&self.plib, h, Attach::Entity(i as i32));
                if id != 0 {
                    if b.def().team == Team::Aliens {
                        self.ps.set_normal(id, e.origin2);
                    }
                    self.damaged.insert(i, id);
                }
            }
        } else if frac >= 0.33 && valid {
            let id = self.damaged.remove(&i).unwrap();
            self.ps.destroy(id);
        }
    }

    /// `CG_ParticleSystemEntity` (misc_particle_system).
    fn particle_system_entity(&mut self, w: &World, i: usize) {
        let e = &w.ents[i];
        if e.e_flags & k::EF_NODRAW != 0 {
            if let Some(&id) = self.entity_ps.get(&i) {
                if self.ps.is_valid(id) && self.ps.is_infinite(id) {
                    self.ps.destroy(id);
                    self.entity_ps.remove(&i);
                }
            }
            return;
        }
        let valid = self.entity_ps.get(&i).is_some_and(|&id| self.ps.is_valid(id));
        if !valid && !self.entity_ps_missing.contains(&i) {
            let name = w.particle_names.get(e.model_index.max(0) as usize).cloned().unwrap_or_default();
            match self.plib.handle(&name) {
                Some(h) => {
                    let id = self.ps.spawn(&self.plib, h, Attach::Point(e.origin));
                    if id != 0 {
                        self.ps.set_cent(id, i as i32);
                        self.entity_ps.insert(i, id);
                    }
                }
                None => {
                    self.entity_ps_missing.insert(i);
                }
            }
        }
    }

    /// The jetpack half of `CG_PlayerUpgrades`.
    fn jetpack_effects(&mut self, w: &World, i: usize) {
        let ps = &w.clients[i].ps;
        let held = bg::inventory_contains_upgrade(UpgradeId::Jetpack.num() as i32, &ps.stats);
        let active = held && bg::upgrade_is_active(UpgradeId::Jetpack.num() as i32, &ps.stats) && ps.stats[Stat::Health as usize] > 0;
        if !active {
            if let Some((id, _)) = self.jetpack.remove(&i) {
                self.ps.destroy(id);
            }
            return;
        }
        let vz = w.ents[i].pos.tr_delta[2];
        let (state, h) = if vz > 10.0 {
            (Jps::Ascending, self.media.jet_ascend)
        } else if vz < -10.0 {
            (Jps::Descending, self.media.jet_descend)
        } else {
            (Jps::Hovering, self.media.jet_hover)
        };
        if self.jetpack.get(&i).is_some_and(|j| j.1 == state) {
            return;
        }
        if let Some((id, _)) = self.jetpack.remove(&i) {
            self.ps.destroy(id);
        }
        if let Some(h) = h {
            let id = self.ps.spawn(&self.plib, h, Attach::Tag { entity: i as i32, tag: "jetpack:tag_flash".into() });
            if id != 0 {
                self.jetpack.insert(i, (id, state));
            }
        }
    }

    /// Muzzle systems of `CG_AddPlayerWeapon`.
    fn muzzle_effects(&mut self, w: &World, i: usize) {
        let e = &w.ents[i];
        let firing = e.e_flags & (k::EF_FIRING | k::EF_FIRING2 | k::EF_FIRING3) != 0 && e.e_flags & k::EF_DEAD == 0;
        if let Some(&id) = self.muzzle.get(&i) {
            // if the PS is infinite disable it when not firing
            if self.ps.is_valid(id) && !firing && self.ps.is_infinite(id) {
                self.ps.destroy(id);
                self.muzzle.remove(&i);
            }
        }
        if e.e_flags & k::EF_DEAD != 0 {
            if let Some(id) = self.muzzle.remove(&i) {
                self.ps.destroy(id);
            }
        }
        let Some(mode) = self.muzzle_trigger.remove(&i) else { return };
        // the handle was checked when the trigger was set
        self.pending_muzzle.push((i, e.weapon, mode));
    }

    /// `CG_Lev2ZapChain`: creator -> first target, first target -> the rest.
    fn zap_chains(&mut self, w: &World) {
        let Some(h) = self.media.level2_zap else { return };
        let mut want = Vec::new();
        for z in &w.zaps {
            for (n, &t) in z.targets.iter().enumerate() {
                let source = if n == 0 { z.creator } else { z.targets[0] };
                want.push((source, t));
            }
        }
        let stale: Vec<(usize, usize)> = self.zaps.keys().copied().filter(|k_| !want.contains(k_)).collect();
        for k_ in stale {
            let id = self.zaps.remove(&k_).unwrap();
            self.ts.destroy(id);
        }
        for (s, t) in want {
            let valid = self.zaps.get(&(s, t)).is_some_and(|&id| self.ts.is_valid(id));
            if !valid {
                let id = self.ts.spawn(&self.tlib, h, Attach::Entity(s as i32), Some(Attach::Entity(t as i32)));
                if id != 0 {
                    self.zaps.insert((s, t), id);
                }
            }
        }
    }

    /// cg_ents.c tesla trail lifetime and range check.
    fn tesla_trails(&mut self, w: &World, now: i32) {
        let max = k::TESLAGEN_RANGE as f32 * 1.732_050_8;
        let mut dead = Vec::new();
        for (&s, &(id, death)) in self.tesla.iter() {
            let far = self.ts.endpoints(id).is_some_and(|(a, b)| b.is_some_and(|b| qm::distance(&a, &b) > max));
            if now > death || far || !w.ents[s].inuse {
                dead.push(s);
            }
        }
        for s in dead {
            let (id, _) = self.tesla.remove(&s).unwrap();
            self.ts.destroy(id);
        }
    }

    /// "poisoncloud" server command -> firstPersonPoisonCloudPS on the local player.
    fn poison_cloud(&mut self, w: &World, me: Option<usize>) {
        let Some(me) = me else { return };
        let ps = &w.clients[me].ps;
        let clouded = ps.e_flags & k::EF_POISONCLOUDED != 0;
        if clouded && !self.poisoned_last && !self.poison.is_some_and(|id| self.ps.is_valid(id)) {
            if let Some(h) = self.media.poison_cloud_fp {
                let id = self.ps.spawn(&self.plib, h, Attach::Entity(me as i32));
                self.poison = (id != 0).then_some(id);
            }
        }
        self.poisoned_last = clouded;
        if ps.stats[Stat::Health as usize] <= 0 || w.clients[me].spectator != SpectatorState::Not as i32 {
            if let Some(id) = self.poison.take() {
                self.ps.destroy(id);
            }
        }
    }

    /// Advance and draw. `actors`/`hand` give model tags; the camera is in
    /// Quake units (origin + view angles).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, w: &World, lib: &mut Library, actors: &HashMap<usize, Actor>, hand: Option<&ViewHand>, me: Option<usize>, first_person: bool, now: i32, cam_origin: Vec3, cam_angles: Vec3) {
        // spawn the triggered muzzle systems now that tags can be resolved
        for (i, weapon, mode) in std::mem::take(&mut self.pending_muzzle) {
            let Some(wid) = WeaponId::from_num(weapon as u8) else { continue };
            let cfg = lib.weapon_cfg(wid.id());
            let Some(h) = cfg.muzzle_ps[mode].as_deref().and_then(|n| self.plib.handle(n)) else { continue };
            let tag = if cfg.model.is_some() { "tag_flash" } else { "tag_weapon" };
            let id = self.ps.spawn(&self.plib, h, Attach::Tag { entity: i as i32, tag: tag.into() });
            if id != 0 {
                if let Some(old) = self.muzzle.insert(i, id) {
                    self.ps.destroy(old);
                }
            }
        }
        let fp = if first_person { me.map(|m| m as i32) } else { None };
        self.ps.first_person_client = fp;
        self.ts.first_person_client = fp;
        let resolve = |a: &Attach| -> Option<AttachPos> {
            match a {
                Attach::Entity(n) => {
                    let n = *n;
                    if n < 0 || n as usize >= w.num_entities {
                        return None;
                    }
                    let i = n as usize;
                    let e = &w.ents[i];
                    if !e.inuse || e.free_after_event {
                        return None;
                    }
                    let (origin, angles, velocity) = if i < MAX_CLIENTS && e.client {
                        let ps = &w.clients[i].ps;
                        (ps.origin, ps.viewangles, ps.velocity)
                    } else {
                        (e.origin, e.angles, e.pos.tr_delta)
                    };
                    let ax = qm::angles_to_axis(&angles);
                    Some(AttachPos { origin, axis: ax, velocity })
                }
                Attach::Tag { entity, tag } => {
                    let i = (*entity).max(0) as usize;
                    if i >= w.num_entities || !w.ents[i].inuse {
                        return None;
                    }
                    let t = if Some(i) == me && first_person && !tag.starts_with("jetpack:") {
                        hand.and_then(|h| h.world_tag("tag_flash"))
                    } else {
                        actors.get(&i).and_then(|a| a.world_tag(tag).or_else(|| a.world_tag("tag_weapon")))
                    };
                    match t {
                        Some(t) => {
                            let mut p = transform_pos(t);
                            p.velocity = w.ents[i].pos.tr_delta;
                            Some(p)
                        }
                        // no model (sheet arena / no data): the eye
                        None if i < MAX_CLIENTS => Some(AttachPos::at(bg::client_view_origin(&w.clients[i].ps))),
                        None => Some(AttachPos::at(w.ents[i].origin)),
                    }
                }
                _ => None,
            }
        };
        self.marks.update(w, lib, now);
        self.ps.update(&self.plib, &self.tlib, now, &w.cm, &resolve, &mut self.ts);
        self.ts.update(&self.tlib, now, &resolve, &self.ps);
        let ax = qm::angles_to_axis(&cam_angles);
        let mut quads = self.ps.quads(&self.plib, cam_origin, ax);
        quads.extend(self.ts.quads(&self.tlib, cam_origin));
        // tracers face the camera (line = forward projected on the view's left/up)
        if !self.tracers.is_empty() {
            let mut verts = Vec::new();
            for (start, finish, fwd) in std::mem::take(&mut self.tracers) {
                let line = [qm::dot(&fwd, &ax[1]), qm::dot(&fwd, &ax[2])];
                let mut right = qm::scale(&ax[1], line[1]);
                right = qm::ma(&right, -line[0], &ax[2]);
                qm::normalize(&mut right);
                let v = |p: Vec3, uv: [f32; 2]| crate::particles::FxVert { pos: p, uv, rgba: [255; 4] };
                verts.push(v(qm::ma(&finish, TRACER_WIDTH, &right), [0.0, 1.0]));
                verts.push(v(qm::ma(&finish, -TRACER_WIDTH, &right), [1.0, 0.0]));
                verts.push(v(qm::ma(&start, -TRACER_WIDTH, &right), [1.0, 1.0]));
                verts.push(v(qm::ma(&start, TRACER_WIDTH, &right), [0.0, 0.0]));
            }
            quads.push(crate::particles::QuadBatch { shader: "gfx/misc/tracer".into(), verts });
        }
        let models = self.ps.models(&self.plib);
        let lights = self.ps.lights(&self.plib);
        if let Some(path) = std::env::var_os("TREM_FX_DEBUG") {
            use std::io::Write;
            let verts: usize = quads.iter().map(|q| q.verts.len()).sum();
            let big: Vec<String> = quads.iter().map(|q| format!("{}:{}", q.shader, q.verts.len())).collect();
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "fx t={now}: ps {:?} ts {:?} batches {} verts {} models {} lights {} marks {:?} {:?}", self.ps.counts(), self.ts.counts(), quads.len(), verts, models.len(), lights.len(), self.marks.count(), big);
            }
        }
        self.fx.draw(lib, &quads, &models, &lights);
    }
}
