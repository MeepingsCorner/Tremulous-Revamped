//! Kernel `layout` — game/g_buildable.c layouts (G_LayoutSave / G_LayoutList /
//! G_LayoutSelect / G_LayoutLoad / G_LayoutBuildItem) and the build log
//! (G_BuildLogNew / G_BuildLogSet / G_BuildLogAuto / G_BuildLogRevert /
//! G_BuildLogRevertThink).
//!
//! Layout files are "layouts/<map>/<name>.dat" read through `World::fs`
//! (the host searches its user folder, then the pk3s; dec026). The kernel
//! formats and parses the original text: one buildable per line,
//! `name ox oy oz ax ay az nx ny nz bx by bz`.

use super::qmath::*;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::buildables::BuildableId;
use crate::gen::enums::Team;

/// `MAX_BUILDLOG`.
pub const MAX_BUILDLOG: usize = 128;
/// `FRAMETIME`.
const FRAMETIME: i32 = 100;

/// `buildFate_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuildFate {
    #[default]
    Construct,
    Deconstruct,
    Replace,
    Destroy,
    Teamkill,
    Unpower,
    Auto,
}

/// `buildLog_t`: what is needed to revert one change of the layout.
#[derive(Clone, Debug, Default)]
pub struct BuildLog {
    pub time: i32,
    pub fate: BuildFate,
    /// namelog indices
    pub actor: Option<usize>,
    pub built_by: Option<usize>,
    pub buildable: Option<BuildableId>,
    pub deconstruct: bool,
    pub deconstruct_time: i32,
    pub origin: Vec3,
    pub angles: Vec3,
    pub origin2: Vec3,
    pub angles2: Vec3,
    pub power_source: Option<BuildableId>,
    pub power_value: i32,
}

// =================================================================== layouts

/// `G_LayoutSave`: the current structures as layout text, written to
/// "layouts/<map>/<name>.dat".
pub fn layout_save(w: &mut World, name: &str) -> Option<String> {
    let map = w.map_name.clone();
    if map.is_empty() {
        w.console.push_str("LayoutSave( ): no map is loaded\n");
        return None;
    }
    let file = format!("layouts/{map}/{name}.dat");
    let text = layout_text(w);
    w.console.push_str(&format!("layoutsave: saving layout to {file}\n"));
    let ok = w.fs.as_mut().is_some_and(|f| f.write(&file, &text));
    if !ok {
        w.console.push_str(&format!("layoutsave: could not open {file}\n"));
        return None;
    }
    Some(text)
}

/// The body G_LayoutSave writes: every ET_BUILDABLE entity.
pub fn layout_text(w: &World) -> String {
    let mut s = String::new();
    for i in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[i];
        if !e.inuse || e.etype != EType::Buildable {
            continue;
        }
        let Some(b) = e.buildable else { continue };
        let (o, a, n, a2) = (e.pos.tr_base, e.angles, e.origin2, e.angles2);
        s.push_str(&format!(
            "{} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6}\n",
            b.id(),
            o[0], o[1], o[2], a[0], a[1], a[2], n[0], n[1], n[2], a2[0], a2[1], a2[2]
        ));
    }
    s
}

/// `G_LayoutList`: "*BUILTIN* name1 name2 ..." and the count (incl. *BUILTIN*).
pub fn layout_list(w: &mut World, map: &str) -> (i32, String) {
    let files = w.fs.as_ref().map(|f| f.list(&format!("layouts/{map}"), ".dat")).unwrap_or_default();
    let mut layouts = String::from("*BUILTIN* ");
    let mut count = 0;
    for f in &files {
        // a name needs at least one character before ".dat"
        if f.len() < 5 {
            continue;
        }
        // list is full, stop trying to add to it (MAX_CVAR_VALUE_STRING)
        if layouts.len() + f.len() >= 256 {
            break;
        }
        layouts.push_str(&f[..f.len() - 4]);
        layouts.push(' ');
        count += 1;
    }
    if count != files.len() {
        w.console.push_str(&format!("WARNING: layout list was truncated to {count} layouts, but {} layout files exist in layouts/{map}/.\n", files.len()));
    }
    (count as i32 + 1, layouts)
}

/// `G_LayoutSelect`: level.layout from g_layouts (one-time use: the host
/// clears it) or, with g_layoutAuto, from every layout of the map; one of
/// the existing ones at random.
pub fn layout_select(w: &mut World, g_layouts: &str) {
    let map = w.map_name.clone();
    let mut layouts = g_layouts.to_string();
    // pick an included layout at random if no list has been provided
    if layouts.is_empty() && w.level.layout_auto {
        layouts = layout_list(w, &map).1;
    }
    if layouts.is_empty() {
        return;
    }
    let mut ok: Vec<String> = Vec::new();
    let mut lx = super::maprotation::Lexer::new(&layouts);
    loop {
        let s = lx.parse_ext(false);
        if s.is_empty() {
            break;
        }
        if s.eq_ignore_ascii_case("*BUILTIN*") || w.fs.as_ref().is_some_and(|f| f.exists(&format!("layouts/{map}/{s}.dat"))) {
            ok.push(s);
        } else {
            w.console.push_str(&format!("WARNING: layout \"{s}\" does not exist\n"));
        }
    }
    if ok.is_empty() {
        w.console.push_str("ERROR: none of the specified layouts could be found, using map default\n");
        return;
    }
    let cnt = ok.len() as i32;
    // RAND_MAX / 1 + 1 wraps to INT_MIN in the C (rand() / INT_MIN == 0)
    let layout_num = w.rng.rand() / (super::rng::RAND_MAX / cnt).wrapping_add(1) + 1;
    w.level.layout = ok[(layout_num - 1).clamp(0, cnt - 1) as usize].clone();
    let list: String = ok.iter().map(|s| format!("{s} ")).collect();
    w.console.push_str(&format!("using layout \"{}\" from list ({list})\n", w.level.layout));
}

/// Does level.layout replace the map's own structures (G_CallSpawn)?
pub fn custom_layout(w: &World) -> bool {
    !w.level.layout.is_empty() && !w.level.layout.eq_ignore_ascii_case("*BUILTIN*")
}

/// `G_LayoutBuildItem`: place one structure as a builder-less G_Build
/// would (the port spawns immediately, like map structures).
fn layout_build_item(w: &mut World, b: BuildableId, origin: Vec3, angles: Vec3, origin2: Vec3, angles2: Vec3) {
    super::buildable::finish_spawning_buildable(w, b, origin, angles, origin2, angles2, None, false);
}

/// `G_LayoutLoad`: spawn the structures of level.layout.
pub fn layout_load(w: &mut World) {
    if !custom_layout(w) {
        return;
    }
    let file = format!("layouts/{}/{}.dat", w.map_name, w.level.layout);
    let Some(text) = w.fs.as_ref().and_then(|f| f.read(&file)) else {
        w.console.push_str(&format!("ERROR: layout {} could not be opened\n", w.level.layout));
        return;
    };
    layout_load_text(w, &text);
}

/// The parse loop of G_LayoutLoad: only '\n'-terminated lines count, and
/// like sscanf a short line keeps the previous line's values.
pub fn layout_load_text(w: &mut World, text: &str) {
    let mut build_name = String::new();
    let mut v = [0.0f32; 12];
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            break;
        }
        if line.len() >= 1024 - 1 {
            w.console.push_str(&format!("ERROR: line overflow in layouts/{}/{}.dat\n", w.map_name, w.level.layout));
            break;
        }
        let mut toks = line.split_ascii_whitespace();
        if let Some(n) = toks.next() {
            build_name = n.to_string();
            for x in v.iter_mut() {
                match toks.next().and_then(|t| scanf_float(t)) {
                    Some(f) => *x = f,
                    None => break,
                }
            }
        }
        match BuildableId::from_id(&build_name) {
            Some(b) => layout_build_item(w, b, [v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]], [v[9], v[10], v[11]]),
            None => w.console.push_str(&format!("WARNING: bad buildable name ({build_name}) in layout. skipping\n")),
        }
    }
}

/// `%f` of sscanf: the longest numeric prefix.
fn scanf_float(t: &str) -> Option<f32> {
    let b = t.as_bytes();
    let mut end = 0;
    let mut best = None;
    while end < b.len() {
        end += 1;
        if let Ok(f) = t[..end].parse::<f32>() {
            best = Some(f);
        }
    }
    best
}

// ================================================================= build log

/// `G_BuildLogNew`: the next ring slot.
pub fn build_log_new(w: &mut World, actor: Option<usize>, fate: BuildFate) -> usize {
    let idx = (w.level.build_id as usize) % MAX_BUILDLOG;
    w.level.build_id += 1;
    if w.level.num_build_logs < MAX_BUILDLOG as i32 {
        w.level.num_build_logs += 1;
    }
    let actor = actor.filter(|&a| a < MAX_CLIENTS && w.ents[a].client).and_then(|a| w.clients[a].namelog);
    let t = w.level.time;
    let log = &mut w.level.build_log[idx];
    *log = BuildLog { time: t, fate, actor, ..Default::default() };
    idx
}

/// `G_BuildLogSet`.
pub fn build_log_set(w: &mut World, log: usize, ent: usize) {
    let value = super::buildable::queue_value(w, ent);
    let src = w.ents[ent].parent_node.and_then(|p| w.ents[p].buildable);
    let e = &w.ents[ent];
    let l = &mut w.level.build_log[log];
    l.buildable = e.buildable;
    l.deconstruct = e.deconstruct;
    l.deconstruct_time = e.deconstruct_time;
    l.built_by = e.built_by;
    l.origin = e.pos.tr_base;
    l.angles = e.angles;
    l.origin2 = e.origin2;
    l.angles2 = e.angles2;
    l.power_source = src;
    l.power_value = value;
}

/// `G_BuildLogAuto`.
pub fn build_log_auto(w: &mut World, actor: Option<usize>, buildable: usize, fate: BuildFate) {
    let l = build_log_new(w, actor, fate);
    build_log_set(w, l, buildable);
}

/// `G_BuildLogRevertThink`: push players out of the way (30 tries), then
/// rebuild the structure and telefrag whatever still blocks it.
pub fn build_log_revert_think(w: &mut World, ent: usize) {
    let Some(b) = w.ents[ent].buildable else {
        w.free(ent);
        return;
    };
    if w.ents[ent].suicide_time > 0 {
        let d = b.def();
        let o = w.ents[ent].pos.tr_base;
        let mut victims = 0;
        for t in w.entities_in_box(&add(&o, &d.mins), &add(&o, &d.maxs)) {
            if t < MAX_CLIENTS && w.ents[t].client {
                let val = if w.clients[t].ps.e_flags & super::types::k::EF_WALLCLIMB != 0 { 300.0 } else { 150.0 };
                let push = [w.rng.crandom() as f32 * val, w.rng.crandom() as f32 * val, w.rng.random() * val];
                let v = &mut w.clients[t].ps.velocity;
                *v = add(v, &push);
                victims += 1;
            }
        }
        w.ents[ent].suicide_time -= 1;
        if victims != 0 {
            // still a blocker
            w.ents[ent].nextthink = w.level.time + FRAMETIME;
            return;
        }
    }
    let e = &w.ents[ent];
    let (o, a, o2, a2, by, dec, dect) = (e.pos.tr_base, e.angles, e.origin2, e.angles2, e.built_by, e.deconstruct, e.deconstruct_time);
    if let Some(built) = super::buildable::finish_spawning_buildable(w, b, o, a, o2, a2, by, true) {
        w.ents[built].deconstruct = dec;
        if dec {
            w.ents[built].deconstruct_time = dect;
        }
        w.ents[built].build_time = 0;
        w.ents[built].s_time = 0;
        super::spawn::kill_box(w, built);
        super::log::log_printf(w, &format!("revert: restore {built} {}\n", b.def().id));
    }
    w.free(ent);
}

/// `G_BuildLogRevert`: undo every logged change from `id` on, newest first.
pub fn build_log_revert(w: &mut World, id: i32) {
    w.level.num_for_removal = 0;
    w.level.num_build_logs -= w.level.build_id - id;
    while w.level.build_id > id {
        w.level.build_id -= 1;
        let log = w.level.build_log[(w.level.build_id as usize) % MAX_BUILDLOG].clone();
        if log.fate == BuildFate::Construct {
            for i in MAX_CLIENTS..w.num_entities {
                let e = &w.ents[i];
                let live = e.inuse && ((e.etype == EType::Buildable && e.health > 0) || (e.etype == EType::General && e.classname == "buildlog_revert"));
                if live && e.buildable == log.buildable {
                    let d = sub(&e.pos.tr_base, &log.origin);
                    if dot(&d, &d) <= 2.0 {
                        if let Some(bt) = w.ents[i].buildable.filter(|_| w.ents[i].etype == EType::Buildable) {
                            super::log::log_printf(w, &format!("revert: remove {i} {}\n", bt.def().id));
                        }
                        w.free(i);
                        break;
                    }
                }
            }
        } else {
            let b = w.spawn();
            let t = w.level.time;
            let e = &mut w.ents[b];
            e.etype = EType::General;
            e.classname = "buildlog_revert";
            e.pos.tr_base = log.origin;
            e.angles = log.angles;
            e.origin2 = log.origin2;
            e.angles2 = log.angles2;
            e.buildable = log.buildable;
            e.deconstruct = log.deconstruct;
            e.deconstruct_time = log.deconstruct_time;
            e.built_by = log.built_by;
            e.think = Some(build_log_revert_think);
            e.nextthink = t + FRAMETIME;
            // Number of thinks before giving up and killing players in the way
            e.suicide_time = 30;
            if matches!(log.fate, BuildFate::Destroy | BuildFate::Teamkill) {
                let value = log.power_value;
                let team = log.buildable.map(|b| b.def().team).unwrap_or(Team::None);
                if team == Team::Aliens {
                    w.level.alien_bp_queue = (w.level.alien_bp_queue - value).max(0);
                } else if log.power_source == Some(BuildableId::Reactor) {
                    w.level.human_bp_queue = (w.level.human_bp_queue - value).max(0);
                } else if log.power_source == Some(BuildableId::Repeater) {
                    if let Some(src) = super::buildable::power_entity_for_point(w, log.origin) {
                        if w.ents[src].uses_bp_zone {
                            let zi = w.ents[src].bp_zone;
                            if let Some(z) = w.level.bp_zones.get_mut(zi) {
                                z.queued = (z.queued - value).max(0);
                            }
                        }
                    }
                }
            }
        }
    }
}
