//! Kernel `spawn` — game/g_spawn.c (spawn vars, entity fields, G_CallSpawn,
//! SP_worldspawn, G_SpawnEntitiesFromString), G_FindTeams (g_main.c) and the
//! g_utils.c helpers map entities use (G_Find, G_PickTarget, G_UseTargets,
//! G_SetMovedir, G_SetOrigin, G_KillBox) plus TeleportPlayer (g_misc.c).
//!
//! The classname -> SP_* table is the `entity_spawns` sheet (a fn table, like
//! `buildable_behaviors`); spawn functions read `World::spawn_vars` exactly as
//! the originals read level.spawnVars.

use super::combat::{self, DAMAGE_NO_PROTECTION};
use super::qmath::*;
use super::types::*;
use super::world::{World, MAX_CLIENTS};
use crate::gen::buildables::BuildableId;
use crate::gen::enums::{MeansOfDeath as Mod, PmType, SpectatorState, Team, Traj};
use std::collections::BTreeMap;

pub const FRAMETIME: i32 = 100;
/// g_local.h / g_public.h / surfaceflags.h engine flags.
pub const FL_TEAMSLAVE: i32 = 0x400;
pub const FL_NO_BOTS: i32 = 0x2000;
pub const FL_NO_HUMANS: i32 = 0x4000;
pub const SVF_NOCLIENT: i32 = 0x1;
pub const SVF_BROADCAST: i32 = 0x20;
pub const SVF_PORTAL: i32 = 0x40;
pub const SVF_USE_CURRENT_ORIGIN: i32 = 0x80;
pub const CONTENTS_SOLID: i32 = 1;
pub const CONTENTS_TRIGGER: i32 = 0x4000_0000;

// ------------------------------------------------------------- C parsing

/// `atof`: the longest numeric prefix (leading whitespace skipped), else 0.
pub fn c_atof(s: &str) -> f64 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut end = 0;
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut digits = false;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits = true;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits = true;
        }
    }
    if digits {
        end = i;
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            let start = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                end = j;
            }
        }
    }
    t[..end].parse().unwrap_or(0.0)
}

/// `atoi`.
pub fn c_atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut i = 0;
    let neg = if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
        b[i - 1] == b'-'
    } else {
        false
    };
    let mut v: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        v = (v * 10 + (b[i] - b'0') as i64).min(i32::MAX as i64 + 1);
        i += 1;
    }
    (if neg { -v } else { v }) as i32
}

/// `sscanf(s, "%f %f %f ...")` into `out`, leaving unparsed slots unchanged.
pub fn scan_floats(s: &str, out: &mut [f32]) {
    for (slot, tok) in out.iter_mut().zip(s.split_whitespace()) {
        let v = c_atof(tok);
        if tok.trim_start().starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.') {
            *slot = v as f32;
        } else {
            return;
        }
    }
}

// ---------------------------------------------------------- spawn vars

/// `G_SpawnString`.
pub fn spawn_string(w: &World, key: &str, default: &str) -> (bool, String) {
    for (k, v) in &w.spawn_vars {
        if k.eq_ignore_ascii_case(key) {
            return (true, v.clone());
        }
    }
    (false, default.to_string())
}
/// `G_SpawnFloat`.
pub fn spawn_float(w: &World, key: &str, default: &str) -> (bool, f32) {
    let (p, s) = spawn_string(w, key, default);
    (p, c_atof(&s) as f32)
}
/// `G_SpawnInt`.
pub fn spawn_int(w: &World, key: &str, default: &str) -> (bool, i32) {
    let (p, s) = spawn_string(w, key, default);
    (p, c_atoi(&s))
}
/// `G_SpawnVector`.
pub fn spawn_vector(w: &World, key: &str, default: &str) -> (bool, Vec3) {
    let (p, s) = spawn_string(w, key, default);
    let mut v = [0.0f32; 3];
    scan_floats(&s, &mut v);
    (p, v)
}

fn lstring(v: &str) -> Option<String> {
    // G_NewString: "\n" escapes become line feeds
    Some(v.replace("\\n", "\n"))
}

/// Classnames are interned to the static ids of the spawn and buildable
/// tables (anything else has no spawn function and is freed).
fn intern_classname(v: &str) -> &'static str {
    let lower = v.to_ascii_lowercase();
    if let Some(d) = crate::gen::entity_spawns::by_id(&lower) {
        return d.id;
    }
    if let Some(b) = buildable_by_entity_name(v) {
        return b.cold().entity_name;
    }
    match lower.as_str() {
        "worldspawn" => "worldspawn",
        _ => "unknown",
    }
}

/// `BG_BuildableByEntityName` (over the legacy structures: the 1.1-only
/// presentation rows, in no stage, are not legacy entities, dec041).
pub fn buildable_by_entity_name(name: &str) -> Option<BuildableId> {
    crate::gen::buildables::BUILDABLES.iter().filter(|d| d.stages != 0).filter_map(|d| BuildableId::from_num(d.num)).find(|id| id.cold().entity_name.eq_ignore_ascii_case(name))
}

/// `G_ParseField` for the fields[] table.
fn parse_field(w: &mut World, key: &str, value: &str, i: usize) {
    let e = &mut w.ents[i];
    let vec = |v: &str| {
        let mut o = [0.0f32; 3];
        scan_floats(v, &mut o);
        o
    };
    match key.to_ascii_lowercase().as_str() {
        "alpha" => e.pos1 = vec(value),
        "angle" => e.s_angles = [0.0, c_atof(value) as f32, 0.0],
        "angles" => e.s_angles = vec(value),
        "animation" => scan_floats(value, &mut e.animation),
        "classname" => e.classname = intern_classname(value),
        "count" => e.count = c_atoi(value),
        "dmg" => e.damage = c_atoi(value),
        "health" => e.health = c_atoi(value),
        "message" => e.message = lstring(value),
        "model" => e.model = lstring(value),
        "model2" => e.model2 = lstring(value),
        "origin" => e.s_origin = vec(value),
        "radius" => e.pos2 = vec(value),
        "random" => e.random = c_atof(value) as f32,
        "rotatorangle" => e.rotator_angle = c_atof(value) as f32,
        "spawnflags" => e.spawnflags = c_atoi(value),
        "speed" => e.speed = c_atof(value) as f32,
        "target" => e.target = lstring(value),
        "targetname" => e.targetname = lstring(value),
        "targetshadername" => e.target_shader_name = lstring(value),
        "targetshadernewname" => e.target_shader_new_name = lstring(value),
        "team" => e.team_name = lstring(value),
        "wait" => e.wait = c_atof(value) as f32,
        _ => {} // acceleration, bounce, light: unused by this port / F_IGNORE
    }
}

/// `SP_worldspawn`.
pub fn worldspawn(w: &mut World) {
    let (p, s) = spawn_string(w, "gravity", "");
    if p {
        w.level.g_gravity = c_atof(&s) as f32;
    }
    let (p, s) = spawn_string(w, "humanMaxStage", "");
    if p {
        w.level.human_max_stage = c_atoi(&s);
    }
    let (p, s) = spawn_string(w, "alienMaxStage", "");
    if p {
        w.level.alien_max_stage = c_atoi(&s);
    }
    // humanBuildPoints / alienBuildPoints are read by G_InitGame-time cvars
    // in 1.2 maps (g_humanBuildPoints set from the worldspawn key by the server cfg)
    let (p, s) = spawn_string(w, "humanBuildPoints", "");
    if p {
        w.level.human_bp = c_atoi(&s);
    }
    let (p, s) = spawn_string(w, "alienBuildPoints", "");
    if p {
        w.level.alien_bp = c_atoi(&s);
    }
    // disabledEquipment / disabledClasses / disabledBuildables -> g_disabled*,
    // which G_InitGame's BG_InitAllowedGameElements reads
    let (_, eq) = spawn_string(w, "disabledEquipment", "");
    let (_, cl) = spawn_string(w, "disabledClasses", "");
    let (_, bu) = spawn_string(w, "disabledBuildables", "");
    for warning in super::bg::init_allowed_game_elements(&eq, &cl, &bu) {
        w.console.push_str(&format!("{warning}\n"));
    }
    let world = ENTITYNUM_WORLD as usize;
    w.ents[world].classname = "worldspawn";
    let (_, msg) = spawn_string(w, "message", "");
    w.ents[world].message = Some(msg);
}

/// `G_CallSpawn`.
fn call_spawn(w: &mut World, i: usize) -> bool {
    let class = w.ents[i].classname;
    if let Some(b) = buildable_by_entity_name(class) {
        // don't spawn built-in buildings if we are using a custom layout
        if super::layout::custom_layout(w) {
            return false;
        }
        if b == BuildableId::Eggpod || b == BuildableId::Telenode {
            let a = &mut w.ents[i].s_angles;
            a[1] += 180.0;
            a[1] = angle_normalize360(a[1]);
        }
        let (o, a) = (w.ents[i].s_origin, w.ents[i].s_angles);
        // G_SpawnBuildable defers to the third frame so buildables can ride
        // trains; the port builds immediately in entity order (no trains
        // carry buildables in the shipped maps).
        w.free(i);
        super::buildable::spawn_from_map(w, b, o, a);
        return true;
    }
    if let Some(d) = crate::gen::entity_spawns::by_id(class) {
        (d.spawn)(w, i);
        return true;
    }
    false
}

/// `G_SpawnGEntityFromSpawnVars`.
fn spawn_from_vars(w: &mut World) {
    let i = w.spawn();
    let vars = w.spawn_vars.clone();
    for (k, v) in &vars {
        parse_field(w, k, v, i);
    }
    if spawn_int(w, "notq3a", "0").1 != 0 {
        w.free(i);
        return;
    }
    let o = w.ents[i].s_origin;
    w.ents[i].pos.tr_base = o;
    w.ents[i].origin = o;
    if !call_spawn(w, i) {
        w.free(i);
    }
}

/// `G_SpawnEntitiesFromString` + `G_FindTeams` over a parsed entity lump.
pub fn spawn_entities(w: &mut World, ents: &[BTreeMap<String, String>]) {
    let Some((first, rest)) = ents.split_first() else { return };
    w.spawn_vars = first.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    worldspawn(w);
    for e in rest {
        w.spawn_vars = e.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        // the BTreeMap sorted the keys; restore "classname" as found by key
        spawn_from_vars(w);
    }
    w.spawn_vars.clear();
    find_teams(w);
}

/// `G_FindTeams`.
pub fn find_teams(w: &mut World) {
    for i in 1..w.num_entities {
        if !w.ents[i].inuse || w.ents[i].team_name.is_none() || w.ents[i].flags & FL_TEAMSLAVE != 0 {
            continue;
        }
        w.ents[i].teammaster = Some(i);
        for j in i + 1..w.num_entities {
            if !w.ents[j].inuse || w.ents[j].team_name.is_none() || w.ents[j].flags & FL_TEAMSLAVE != 0 {
                continue;
            }
            if w.ents[i].team_name == w.ents[j].team_name {
                w.ents[j].teamchain = w.ents[i].teamchain;
                w.ents[i].teamchain = Some(j);
                w.ents[j].teammaster = Some(i);
                w.ents[j].flags |= FL_TEAMSLAVE;
                if let Some(tn) = w.ents[j].targetname.take() {
                    w.ents[i].targetname = Some(tn);
                }
            }
        }
    }
}

// --------------------------------------------------------------- g_utils

/// `G_Find(from, FOFS(targetname), match)`.
pub fn find_targetname(w: &World, from: Option<usize>, name: &str) -> Option<usize> {
    let start = from.map(|f| f + 1).unwrap_or(0);
    (start..w.num_entities).find(|&i| w.ents[i].inuse && w.ents[i].targetname.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(name)))
}

/// `G_PickTarget`.
pub fn pick_target(w: &mut World, name: Option<&str>) -> Option<usize> {
    let name = name?;
    let mut choices = Vec::new();
    let mut e = None;
    while let Some(i) = find_targetname(w, e, name) {
        choices.push(i);
        e = Some(i);
        if choices.len() == 32 {
            break;
        }
    }
    if choices.is_empty() {
        return None;
    }
    let n = choices.len() as i32;
    Some(choices[(w.rng.rand() / (super::rng::RAND_MAX / n + 1)) as usize])
}

/// `G_UseTargets`.
pub fn use_targets(w: &mut World, ent: usize, activator: Option<usize>) {
    if let (Some(a), Some(b)) = (w.ents[ent].target_shader_name.clone(), w.ents[ent].target_shader_new_name.clone()) {
        let f = (w.level.time as f64 * 0.001) as f32;
        add_remap(w, &a, &b, f);
    }
    let Some(target) = w.ents[ent].target.clone() else { return };
    let mut t = None;
    while let Some(i) = find_targetname(w, t, &target) {
        t = Some(i);
        if i != ent {
            if let Some(u) = w.ents[i].use_ {
                u(w, i, Some(ent), activator);
            }
        }
        if !w.ents[ent].inuse {
            return;
        }
    }
}

/// `AddRemap`.
fn add_remap(w: &mut World, old: &str, new: &str, t: f32) {
    for r in w.level.shader_remaps.iter_mut() {
        if r.0.eq_ignore_ascii_case(old) {
            r.1 = new.to_string();
            r.2 = t;
            return;
        }
    }
    if w.level.shader_remaps.len() < 128 {
        w.level.shader_remaps.push((old.to_string(), new.to_string(), t));
    }
}

/// `G_SetMovedir`: returns movedir and clears `angles`.
pub fn set_movedir(angles: &mut Vec3) -> Vec3 {
    let movedir = if *angles == [0.0, -1.0, 0.0] {
        [0.0, 0.0, 1.0]
    } else if *angles == [0.0, -2.0, 0.0] {
        [0.0, 0.0, -1.0]
    } else {
        angle_vectors(angles).0
    };
    *angles = [0.0; 3];
    movedir
}

/// `G_SetOrigin`.
pub fn set_origin(w: &mut World, i: usize, o: Vec3) {
    let e = &mut w.ents[i];
    e.pos = super::bg::Trajectory { tr_type: Traj::Stationary as i32, tr_time: 0, tr_duration: 0, tr_base: o, tr_delta: [0.0; 3] };
    e.origin = o;
    e.s_origin = o;
}

/// `G_KillBox`.
pub fn kill_box(w: &mut World, i: usize) {
    let mins = add(&w.ents[i].origin, &w.ents[i].mins);
    let maxs = add(&w.ents[i].origin, &w.ents[i].maxs);
    for h in w.entities_in_box(&mins, &maxs) {
        if !w.ents[h].client || h == i {
            continue;
        }
        combat::damage(w, h, Some(i), Some(i), None, None, 100000, DAMAGE_NO_PROTECTION, Mod::Telefrag);
    }
}

/// `G_SetClientViewAngle`.
pub fn set_client_view_angle(w: &mut World, i: usize, angles: &Vec3) {
    let c = &mut w.clients[i];
    for k in 0..3 {
        let cmd_angle = angle2short_f(angles[k]);
        c.ps.delta_angles[k] = cmd_angle - c.cmd.angles[k];
    }
    w.ents[i].angles = *angles;
    w.clients[i].ps.viewangles = *angles;
}

/// `TeleportPlayer` (g_misc.c).
pub fn teleport_player(w: &mut World, p: usize, origin: &Vec3, angles: &Vec3) {
    w.unlink(p);
    let c = &mut w.clients[p];
    c.ps.origin = *origin;
    c.ps.origin[2] += 1.0;
    let (fwd, _, _) = angle_vectors(angles);
    c.ps.velocity = scale(&fwd, 400.0);
    c.ps.pm_time = 160;
    c.ps.pm_flags |= k::PMF_TIME_KNOCKBACK;
    c.ps.e_flags ^= k::EF_TELEPORT_BIT;
    super::unlagged::clear(w, p);
    super::weapon::clear_player_zap_effects(w, p);
    set_client_view_angle(w, p, angles);
    super::client::ps_to_es(w, p, true);
    let o = w.clients[p].ps.origin;
    w.ents[p].origin = o;
    if w.clients[p].spectator == SpectatorState::Not as i32 {
        kill_box(w, p);
        w.link(p);
    }
}

/// Spawn-point flags read by G_SelectSpawnPoint style code.
pub fn info_player_deathmatch(w: &mut World, i: usize) {
    if spawn_int(w, "nobots", "0").1 != 0 {
        w.ents[i].flags |= FL_NO_BOTS;
    }
    if spawn_int(w, "nohumans", "0").1 != 0 {
        w.ents[i].flags |= FL_NO_HUMANS;
    }
}
/// `SP_info_player_start`.
pub fn info_player_start(w: &mut World, i: usize) {
    w.ents[i].classname = "info_player_deathmatch";
    info_player_deathmatch(w, i);
}
/// `SP_info_player_intermission` / alien / human: positional only.
pub fn info_intermission(_w: &mut World, _i: usize) {}

/// Live clients that can trigger/pmove (used by several touch functions).
pub fn is_live_client(w: &World, i: usize) -> bool {
    i < MAX_CLIENTS && w.ents[i].client && w.clients[i].ps.pm_type != PmType::Dead as i32
}

/// The team a client plays on (STAT_TEAM).
pub fn client_team(w: &World, i: usize) -> Team {
    if i < MAX_CLIENTS && w.ents[i].client {
        w.clients[i].team
    } else {
        Team::None
    }
}
