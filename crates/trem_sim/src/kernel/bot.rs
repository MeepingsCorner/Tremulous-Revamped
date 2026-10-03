//! Kernel `bot` — not in the original (tremulous-legacy has no bots; dec015).
//! Bots are ordinary clients whose `UserCmd` is written here before the
//! client_think system runs, so they obey exactly the same Pmove/weapon
//! rules as a human player, and they evolve, buy and sell only through the
//! client commands a player types (`class`, `buy`, `sell`). Navigation is
//! the `nav` kernel's graph (the waypoints sheet on sheet maps, a generated
//! floor graph on BSP maps); target choice and shopping are small rules.
//! Every movement command passes a ledge guard that refuses to step where
//! there is no floor within a safe fall or where a lethal trigger is
//! (dec028); a bot that makes no progress along a link blocks that link for
//! a while and routes around it. Balance (dec036): human bots go back when
//! one of their structures is hurt, a human builder keeps machinegun
//! turrets up, and a structure (it does not dodge) is aimed at with a
//! quarter of the spread.

use super::bg;
use super::client;
use super::nav::{LinkKind, STEP};
use super::qmath::*;
use super::rng::Rng;
use super::types::*;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::buildables::BuildableId;
use crate::gen::classes::ClassId;
use crate::gen::enums::{SpectatorState, Stat, Team};
use crate::gen::weapons::WeaponId;

const REACH: f32 = 56.0;
const SIGHT: f32 = 2000.0;
/// A link the bot made no progress on for this long is blocked for `BLOCK_MS`.
const NO_PROGRESS_MS: i32 = 2500;
const BLOCK_MS: i32 = 30_000;
/// A builder that finds no valid spot for this long gives that structure up
/// for a minute.
const BUILD_GIVE_UP_MS: i32 = 15_000;
/// A bot that got nowhere (within `WEDGED_RADIUS`) for this long while
/// travelling is wedged in the map: it types `/kill` like a player would
/// (the suicide itself follows 20 s later, as for anyone).
const WEDGED_MS: i32 = 10_000;
const WEDGED_RADIUS: f32 = 96.0;
/// A raiding alien fights only players that are on it (within this)...
const RAID_SELF_DEFENCE: f32 = 60.0;
/// ...until it is this close to an enemy structure (dec028).
const RAID_ARRIVE: f32 = 500.0;
/// Human bots go back to defend a structure that was hurt this recently.
const ALARM_MS: i32 = 20_000;
/// Machinegun turrets a human builder keeps up.
const MG_TURRETS: usize = 3;

#[derive(Clone, Debug, Default)]
pub struct BotState {
    pub active: bool,
    pub team: Team,
    pub target: Option<usize>,
    pub path: Vec<usize>,
    pub goal: Option<Vec3>,
    pub next_repath: i32,
    pub next_target: i32,
    pub next_shop: i32,
    pub last_pos: Vec3,
    pub stuck_check: i32,
    pub unstick_until: i32,
    pub strafe: i8,
    pub strafe_until: i32,
    /// The last nav node reached (start of the link being walked).
    pub last_node: Option<usize>,
    /// The route to `goal` was walked to its end: the last leg is straight.
    pub arrived: bool,
    /// The path is a random link (there was no route to the goal).
    pub wandering: bool,
    /// Direction of the drop link being walked: the graph checked where it
    /// lands, so the ledge guard lets the bot step off along it.
    pub drop_dir: Option<Vec3>,
    /// Where the bot has been lingering, and for how long it has been
    /// trying to get somewhere from there (`WEDGED_MS` of it: `/kill`).
    pub anchor: Option<(Vec3, i32)>,
    /// Consecutive stuck checks without moving, and the way out being tried.
    pub stuck_count: i32,
    pub escape: Option<Vec3>,
    /// Progress along the current link: (node, best flat distance, since).
    pub progress: Option<(usize, f32, i32)>,
    /// Dretches guard their base: the structure being visited, and until when.
    pub patrol: Option<usize>,
    pub patrol_until: i32,
    /// When the bot started trying to place the structure it is building.
    pub build_since: Option<i32>,
    /// Reaction time: no firing at a new target before this.
    pub react_at: i32,
    /// Aim error (a fraction of the distance) and when it is picked again.
    pub aim_off: Vec3,
    pub aim_off_until: i32,
}

/// Per-world bot bookkeeping. A private RNG keeps bot decisions from
/// perturbing the game RNG sequence (which golden traces rely on).
pub struct Bots {
    pub states: Vec<BotState>,
    pub rng: Rng,
    /// Nav links found shut or untravellable: (from, to, until level.time).
    pub blocked: Vec<(usize, usize, i32)>,
    /// Structures a builder found no valid spot for: (team, structure, until).
    pub no_build: Vec<(Team, BuildableId, i32)>,
    /// Each structure's health last frame, and per team where one of its
    /// own structures was last hurt and when (a player sees their own
    /// structures' health: dec028).
    pub struct_health: Vec<i32>,
    pub alarm: [Option<(Vec3, i32)>; 3],
}
impl Default for Bots {
    fn default() -> Self {
        Bots { states: vec![BotState::default(); MAX_CLIENTS], rng: Rng::new(0xb075_eed5), blocked: Vec::new(), no_build: Vec::new(), struct_health: Vec::new(), alarm: [None; 3] }
    }
}

/// Connect a bot and put it on `team`.
pub fn add_bot(w: &mut World, name: &str, team: Team) -> Option<usize> {
    let i = w.connect(name, true)?;
    w.bots.states[i] = BotState { active: true, team, ..Default::default() };
    client::change_team(w, i, team);
    Some(i)
}

/// Remove one bot of `team` (or any team), returning its slot.
pub fn remove_bot(w: &mut World, team: Option<Team>) -> Option<usize> {
    let i = (0..MAX_CLIENTS).rev().find(|&i| w.clients[i].connected && w.clients[i].is_bot && team.map(|t| w.clients[i].team == t).unwrap_or(true))?;
    w.bots.states[i] = BotState::default();
    w.disconnect(i);
    Some(i)
}

/// `bots_think` system.
pub fn think_all(w: &mut World) {
    let any = (0..MAX_CLIENTS).any(|i| w.clients[i].connected && w.clients[i].is_bot);
    if any && w.nav.is_none() {
        // waypoints sheet for sheet maps, a generated floor graph for BSP maps
        w.nav = Some(if w.has_waypoints { super::nav::Nav::from_sheet(w) } else { super::nav::Nav::generate(w) });
    }
    let t = w.level.time;
    w.bots.blocked.retain(|b| b.2 > t);
    w.bots.no_build.retain(|b| b.2 > t);
    if any {
        watch_structures(w);
    }
    for i in 0..MAX_CLIENTS {
        if w.clients[i].connected && w.clients[i].is_bot && w.bots.states[i].active {
            think(w, i);
        }
    }
}

/// A base under attack: a team's finished structure lost health.
fn watch_structures(w: &mut World) {
    let t = w.level.time;
    let n = w.num_entities;
    if w.bots.struct_health.len() < n {
        w.bots.struct_health.resize(n, 0);
    }
    for e in MAX_CLIENTS..n {
        let x = &w.ents[e];
        let h = if x.inuse && x.etype == EType::Buildable { x.health } else { 0 };
        if h > 0 && h < w.bots.struct_health[e] && x.spawned {
            w.bots.alarm[x.team as usize] = Some((x.origin, t));
        }
        w.bots.struct_health[e] = h;
    }
}

/// Where the team's base was last hurt, if within `ALARM_MS`.
fn alarm(w: &World, team: Team) -> Option<Vec3> {
    w.bots.alarm[team as usize].filter(|a| w.level.time - a.1 < ALARM_MS).map(|a| a.0)
}

fn enemy_of(t: Team) -> Team {
    match t {
        Team::Aliens => Team::Humans,
        Team::Humans => Team::Aliens,
        Team::None => Team::None,
    }
}

fn center(w: &World, e: usize) -> Vec3 {
    let x = &w.ents[e];
    let mut c = x.origin;
    for k_ in 0..3 {
        c[k_] += (x.mins[k_] + x.maxs[k_]) * 0.5;
    }
    c
}

fn eye(w: &World, i: usize) -> Vec3 {
    bg::client_view_origin(&w.clients[i].ps)
}

fn can_see(w: &World, i: usize, e: usize) -> bool {
    let tr = w.trace(&eye(w, i), None, None, &center(w, e), i as i32, k::MASK_SHOT);
    tr.entity_num == e as i32 || tr.fraction == 1.0
}

fn pick_target(w: &World, i: usize) -> Option<usize> {
    let me = eye(w, i);
    let enemy = enemy_of(w.clients[i].team);
    // dretch bites only hurt structures that are still building (CheckVenomAttack)
    let bites_only = w.clients[i].ps.weapon == WeaponId::Level0 as i32;
    let raid = raiding(w, i);
    let mut best: Option<(f32, usize)> = None;
    for e in 0..w.num_entities {
        let x = &w.ents[e];
        if !x.inuse || !x.linked || !x.takedamage || x.health <= 0 {
            continue;
        }
        let is_enemy = if e < MAX_CLIENTS && x.client {
            w.clients[e].team == enemy && w.clients[e].spectator == SpectatorState::Not as i32
        } else {
            x.etype == EType::Buildable && x.team == enemy && !(bites_only && x.spawned)
        };
        if !is_enemy {
            continue;
        }
        let mut d = distance(&me, &center(w, e));
        if d > SIGHT || !can_see(w, i, e) {
            continue;
        }
        if raid {
            // a raider slips past players on the way: it fights only what
            // is on it, and goes for the structures once there
            if x.etype != EType::Buildable && d > RAID_SELF_DEFENCE {
                continue;
            }
        } else if x.etype == EType::Buildable && !(w.clients[i].team == Team::Aliens && !bites_only && d < 300.0) {
            d += 400.0; // prefer players (an evolved alien hits what is at hand)
        }
        if best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, e));
        }
    }
    best.map(|b| b.1)
}

fn nearest_waypoint(w: &World, p: &Vec3, need_sight: bool) -> Option<usize> {
    w.nav.as_ref()?.nearest(w, p, need_sight)
}

/// A route avoiding blocked links, or (when they cut the goal off) any route.
fn route(w: &World, i: usize, from: usize, to: usize) -> Vec<usize> {
    let t = w.level.time;
    // (usize::MAX, n) blocks every link into n
    let blocked = |a: usize, b: usize| w.bots.blocked.iter().any(|&(x, y, until)| until > t && (x == a || x == usize::MAX) && y == b);
    let Some(n) = w.nav.as_ref() else { return Vec::new() };
    let fall = max_fall(w, i);
    let r = n.route_avoiding(from, to, fall, &blocked);
    if r.is_empty() {
        n.route_avoiding(from, to, fall, &|_, _| false)
    } else {
        r
    }
}

/// The deepest drop this bot's class takes without falling damage.
fn max_fall(w: &World, i: usize) -> f32 {
    let class = bg::class(w.clients[i].ps.stats[Stat::Class as usize]);
    match w.nav.as_ref() {
        Some(n) if class.fall_damage > 0.0 => n.safe_drop,
        _ => super::nav::MAX_FALL,
    }
}

/// The deepest drop the ledge guard lets this bot take (aliens take no
/// falling damage).
fn guard_fall(w: &World, i: usize) -> f32 {
    let class = bg::class(w.clients[i].ps.stats[Stat::Class as usize]);
    match w.nav.as_ref() {
        // a wounded human only takes falls that do not hurt
        Some(n) if class.fall_damage > 0.0 && hurt(w, i) => n.safe_drop,
        Some(n) if class.fall_damage > 0.0 => n.hurt_drop,
        _ => super::nav::MAX_FALL,
    }
}

fn node(w: &World, n: usize) -> Vec3 {
    w.nav.as_ref().map(|x| x.nodes[n]).unwrap_or([0.0; 3])
}

fn hurt(w: &World, i: usize) -> bool {
    let ps = &w.clients[i].ps;
    ps.stats[Stat::Health as usize] * 10 < ps.stats[Stat::MaxHealth as usize] * 4
}

/// How many standing (or still building) structures of kind `b` there are.
fn structure_count(w: &World, b: BuildableId) -> usize {
    (MAX_CLIENTS..w.num_entities).filter(|&e| w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].buildable == Some(b) && w.ents[e].health > 0).count()
}

/// A standing (or still building) structure of kind `b`.
fn structure(w: &World, b: BuildableId) -> Option<usize> {
    (MAX_CLIENTS..w.num_entities).find(|&e| w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].buildable == Some(b) && w.ents[e].health > 0)
}

/// What `team`'s builder should put up next: the main structure, then a
/// spawn, then (humans) an armoury and a medistat, if the build points allow.
fn needed(w: &World, team: Team) -> Option<BuildableId> {
    let (bp, list): (i32, &[BuildableId]) = match team {
        Team::Aliens => (w.level.alien_bp_left, &[BuildableId::Overmind, BuildableId::Eggpod]),
        Team::Humans => (w.level.human_bp_left, &[BuildableId::Reactor, BuildableId::Telenode, BuildableId::Arm, BuildableId::Medistat, BuildableId::Mgturret]),
        Team::None => return None,
    };
    let stage = if team == Team::Aliens { w.level.alien_stage } else { w.level.human_stage };
    let t = w.level.time;
    let gave_up = |b: BuildableId| w.bots.no_build.iter().any(|&(tm, x, until)| tm == team && x == b && until > t);
    // one of each, and a few machinegun turrets to hold the base
    let wanted = |b: BuildableId| if b == BuildableId::Mgturret { MG_TURRETS } else { 1 };
    list.iter().copied().find(|&b| structure_count(w, b) < wanted(b) && b.def().build_points <= bp && bg::buildable_allowed_in_stage(b, stage) && !gave_up(b))
}

/// An alien fighter that can hurt structures (not a dretch or a builder).
fn evolved(w: &World, i: usize) -> bool {
    w.clients[i].team == Team::Aliens && !is_builder(w, i) && w.clients[i].ps.weapon != WeaponId::Level0 as i32
}

/// Does this alien stay home: dretches always, evolved aliens until a
/// second one is alive (a dragoon or tyrant goes alone).
fn guards(w: &World, i: usize) -> bool {
    if w.clients[i].team != Team::Aliens || is_builder(w, i) {
        return false;
    }
    if !evolved(w, i) {
        return true;
    }
    let class = bg::class_id(w.clients[i].ps.stats[Stat::Class as usize]);
    let strong = matches!(class, ClassId::Level3 | ClassId::Level3upg | ClassId::Level4);
    let mates = (0..MAX_CLIENTS).filter(|&j| w.clients[j].connected && w.clients[j].spectator == SpectatorState::Not as i32 && w.clients[j].ps.stats[Stat::Health as usize] > 0 && evolved(w, j)).count();
    !strong && mates < 2
}

/// An evolved alien on its way to the human base (not guarding) is a
/// raider while it is still far from the enemy's structures: it avoids
/// fights with players that keep their distance (dec028).
fn raiding(w: &World, i: usize) -> bool {
    if !evolved(w, i) || guards(w, i) {
        return false;
    }
    let me = w.clients[i].ps.origin;
    nearest_enemy_structure(w, i).is_some_and(|e| distance(&me, &w.ents[e].origin) > RAID_ARRIVE)
}

/// The closest standing enemy structure.
fn nearest_enemy_structure(w: &World, i: usize) -> Option<usize> {
    let me = w.clients[i].ps.origin;
    let enemy = enemy_of(w.clients[i].team);
    (MAX_CLIENTS..w.num_entities)
        .filter(|&e| w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].team == enemy && w.ents[e].health > 0)
        .min_by(|&a, &b| distance(&me, &w.ents[a].origin).total_cmp(&distance(&me, &w.ents[b].origin)))
}

fn is_builder(w: &World, i: usize) -> bool {
    matches!(WeaponId::from_num(w.clients[i].ps.weapon as u8), Some(WeaponId::Ckit | WeaponId::Abuild | WeaponId::Abuildupg))
}

/// Where a builder puts structures: by the team's main structure, else
/// where it stands.
fn build_site(w: &World, i: usize) -> Vec3 {
    let main = if w.clients[i].team == Team::Aliens { w.overmind().or_else(|| structure(w, BuildableId::Eggpod)) } else { w.reactor().or_else(|| structure(w, BuildableId::Telenode)) };
    main.map(|m| w.ents[m].origin).unwrap_or(w.clients[i].ps.origin)
}

fn goal(w: &World, i: usize) -> Option<Vec3> {
    let c = &w.clients[i];
    let team = c.team;
    if is_builder(w, i) {
        if needed(w, team).is_some() {
            return Some(build_site(w, i));
        }
        // a human builder with nothing to build trades the kit for a rifle,
        // an alien one goes home to evolve
        if let Some(a) = w.find_buildable(BuildableId::Arm).filter(|_| team == Team::Humans) {
            return Some(w.ents[a].origin);
        }
        return Some(build_site(w, i));
    }
    // wounded: aliens regenerate on creep near the overmind, humans at a medistat
    if hurt(w, i) {
        let home = if team == Team::Aliens { w.overmind().or_else(|| w.find_buildable(BuildableId::Booster)) } else { w.find_buildable(BuildableId::Medistat) };
        // an evolved alien closer to the enemy's structures than to home presses on
        let me = c.ps.origin;
        let committed = team == Team::Aliens && evolved(w, i) && !guards(w, i) && home.zip(nearest_enemy_structure(w, i)).is_some_and(|(hm, e)| distance(&me, &w.ents[e].origin) < distance(&me, &w.ents[hm].origin));
        if let Some(hm) = home.filter(|_| !committed) {
            return Some(w.ents[hm].origin);
        }
    }
    // humans defend their base when it is under attack
    if team == Team::Humans {
        if let Some(a) = alarm(w, team) {
            return Some(a);
        }
    }
    // humans with money go shopping first
    if team == Team::Humans && c.credit >= 150 && c.ps.stats[Stat::Weapon as usize] == WeaponId::Rifle as i32 {
        if let Some(a) = w.find_buildable(BuildableId::Arm) {
            return Some(w.ents[a].origin);
        }
    }
    // dretches cannot hurt finished structures: they guard their own base,
    // ambushing humans there until they can evolve; evolved aliens wait
    // there for a second one before they attack
    if let Some(p) = w.bots.states[i].patrol.filter(|_| team == Team::Aliens && guards(w, i)) {
        return Some(w.ents[p].origin);
    }
    // evolved aliens take the base apart from the outside in
    if team == Team::Aliens {
        let me = c.ps.origin;
        let near = (MAX_CLIENTS..w.num_entities)
            .filter(|&e| w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].team == Team::Humans && w.ents[e].health > 0)
            .min_by(|&a, &b| distance(&me, &w.ents[a].origin).total_cmp(&distance(&me, &w.ents[b].origin)));
        if let Some(e) = near {
            return Some(w.ents[e].origin);
        }
    }
    let main = if team == Team::Aliens { w.reactor() } else { w.overmind() };
    if let Some(m) = main {
        return Some(w.ents[m].origin);
    }
    let enemy = enemy_of(team);
    (MAX_CLIENTS..w.num_entities).find(|&e| w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].team == enemy && w.ents[e].health > 0).map(|e| w.ents[e].origin)
}

/// Bot turn rate (degrees per second) and aim wobble: bots are not aimbots.
const TURN_RATE: f32 = 540.0;
const AIM_JITTER: f32 = 4.0;
/// Bots aim where the target was this long ago (seconds)...
const AIM_LAG: f32 = 0.15;
/// ...plus a wandering error of up to this fraction of the distance (human
/// weapons hit at range, so human bots are the less accurate ones).
const AIM_SPREAD_ALIEN: f32 = 0.08;
const AIM_SPREAD_HUMAN: f32 = 0.16;

/// Turn towards `at` at a limited rate; returns the remaining angular error.
fn aim_cmd(w: &mut World, i: usize, at: &Vec3) -> f32 {
    let dir = sub(at, &eye(w, i));
    let want = vectoangles(&dir);
    let step = TURN_RATE * w.level.msec.max(1) as f32 / 1000.0;
    let jitter = [(w.bots.rng.crandom() as f32) * AIM_JITTER, (w.bots.rng.crandom() as f32) * AIM_JITTER];
    let c = &mut w.clients[i];
    let mut err = 0.0f32;
    for k_ in 0..2 {
        let cur = c.ps.viewangles[k_];
        let mut d = want[k_] - cur;
        while d > 180.0 {
            d -= 360.0;
        }
        while d < -180.0 {
            d += 360.0;
        }
        err = err.max(d.abs());
        let next = cur + d.clamp(-step, step) + jitter[k_] * (d.abs() / 90.0).min(1.0) + jitter[k_] * 0.15;
        c.cmd.angles[k_] = angle2short_f(next) - c.ps.delta_angles[k_];
    }
    err
}

fn shop(w: &mut World, i: usize) {
    let t = w.level.time;
    if w.bots.states[i].next_shop > t {
        return;
    }
    w.bots.states[i].next_shop = t + 2000;
    let team = w.clients[i].team;
    if is_builder(w, i) && needed(w, team).is_none() {
        if team == Team::Aliens {
            let _ = client::cmd_class(w, i, ClassId::Level0.id());
            return;
        }
        let origin = w.clients[i].ps.origin;
        if super::buildable::buildable_range(w, origin, 100.0, BuildableId::Arm) && client::cmd_sell(w, i, "weapon").is_ok() {
            let _ = client::cmd_buy(w, i, "rifle");
        }
        return;
    }
    match team {
        Team::Humans => {
            let origin = w.clients[i].ps.origin;
            if !super::buildable::buildable_range(w, origin, 100.0, BuildableId::Arm) {
                return;
            }
            let _ = client::cmd_buy(w, i, "ammo");
            for wp in ["lcannon", "chaingun", "prifle", "mdriver", "lgun", "shotgun"] {
                if w.clients[i].ps.stats[Stat::Weapon as usize] != WeaponId::Rifle as i32 {
                    break;
                }
                if w.clients[i].credit >= WeaponId::from_id(wp).map(|x| x.def().price).unwrap_or(i32::MAX) {
                    let _ = client::cmd_sell(w, i, "weapon");
                    if client::cmd_buy(w, i, wp).is_err() {
                        let _ = client::cmd_buy(w, i, "rifle");
                    }
                }
            }
            for up in ["helmet", "larmour"] {
                let _ = client::cmd_buy(w, i, up);
            }
        }
        Team::Aliens => {
            let cur = bg::class_id(w.clients[i].ps.stats[Stat::Class as usize]);
            if cur == ClassId::Builder || cur == ClassId::Builderupg {
                return;
            }
            let credits = w.clients[i].credit;
            let stage = w.level.alien_stage;
            for to in [ClassId::Level4, ClassId::Level3upg, ClassId::Level3, ClassId::Level2upg, ClassId::Level2, ClassId::Level1upg, ClassId::Level1] {
                if to == cur {
                    break;
                }
                if bg::class_can_evolve_from_to(cur, to, credits, stage, 0) >= 0 && w.bots.rng.random() < 0.5 {
                    if client::cmd_class(w, i, to.id()).is_ok() {
                        break;
                    }
                }
            }
        }
        Team::None => {}
    }
}

/// The yaw (degrees) the command's angles will give the player.
fn cmd_yaw(w: &World, i: usize, cmd: &UserCmd) -> f32 {
    short2angle(cmd.angles[1] + w.clients[i].ps.delta_angles[1]) as f32
}

/// Where a bot moving `dist` from `pos` along horizontal `dir` ends up:
/// Some(fall depth) on standable floor within `guard_fall` that is clear of
/// lethal volumes and lava/slime (0 when a wall is in the way), None
/// otherwise.
fn landing(w: &World, i: usize, pos: &Vec3, dir: &Vec3, dist: f32) -> Option<f32> {
    let nav = w.nav.as_ref()?;
    let (lo, hi) = ([-8.0, -8.0, 0.0], [8.0, 8.0, 8.0]);
    let feet = w.clients[i].ps.origin[2] + bg::class(w.clients[i].ps.stats[Stat::Class as usize]).mins[2];
    let start = [pos[0], pos[1], feet + STEP];
    let ahead = ma(&start, dist, dir);
    let tr = w.trace(&start, Some(&lo), Some(&hi), &ahead, i as i32, k::MASK_PLAYERSOLID);
    if tr.startsolid {
        return Some(0.0);
    }
    let p = tr.endpos;
    let floor_z = feet - guard_fall(w, i) - 4.0;
    let down = [p[0], p[1], floor_z];
    let tr2 = w.trace(&p, Some(&lo), Some(&hi), &down, i as i32, k::MASK_PLAYERSOLID);
    // no floor, or a slope too steep to stand on (it slides the bot away)
    if tr2.fraction == 1.0 || tr2.startsolid || tr2.plane.normal[2] < 0.7 {
        return None;
    }
    let land = tr2.endpos;
    let human_origin = [land[0], land[1], land[2] + 24.0];
    let fall = ([land[0] - 16.0, land[1] - 16.0, land[2]], [land[0] + 16.0, land[1] + 16.0, feet]);
    if nav.danger.iter().any(|(a, b)| bounds_intersect(&fall.0, &fall.1, a, b)) || !nav.safe_at(w, &human_origin) {
        return None;
    }
    Some((feet - land[2]).max(0.0))
}

/// Is it safe to move from `pos` towards horizontal direction `dir`: the
/// floor `dist` ahead is a safe `landing`, and when that is a drop, so is
/// the spot the bot's momentum carries it to while it falls.
fn safe_ahead(w: &World, i: usize, pos: &Vec3, dir: &Vec3, dist: f32) -> bool {
    let Some(depth) = landing(w, i, pos, dir, dist) else { return false };
    if depth <= STEP + 4.0 {
        return true;
    }
    let v = w.clients[i].ps.velocity;
    let speed = (v[0] * v[0] + v[1] * v[1]).sqrt().max(250.0);
    let t = (2.0 * depth / w.level.g_gravity.max(1.0)).sqrt();
    let carry = dist + speed * t;
    landing(w, i, pos, dir, carry).is_some() && landing(w, i, pos, dir, (dist + carry) * 0.5).is_some()
}

/// Ledge guard: drop the parts of the move that would walk off a deadly or
/// damaging edge; with nothing safe left, stand still and do not jump.
fn guard(w: &World, i: usize, cmd: &mut UserCmd) {
    let ps = &w.clients[i].ps;
    let pos = ps.origin;
    let yaw = cmd_yaw(w, i, cmd);
    let (f, r, _) = angle_vectors(&[0.0, yaw, 0.0]);
    let v = ps.velocity;
    let speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if ps.ground_entity_num == k::ENTITYNUM_NONE {
        // airborne: if the jump or fall is heading somewhere deadly, use
        // what air control there is to pull back
        if speed > 80.0 && landing(w, i, &pos, &[v[0] / speed, v[1] / speed, 0.0], speed * 0.3).is_none() {
            let back = [-v[0] / speed, -v[1] / speed, 0.0];
            cmd.forwardmove = (dot(&back, &f) * 127.0) as i8;
            cmd.rightmove = (dot(&back, &r) * 127.0) as i8;
            cmd.upmove = 0;
        }
        return;
    }
    // friction needs about a quarter of the speed in units to stop
    let dist = 32.0 + speed * 0.25;
    let brake = |cmd: &mut UserCmd| {
        cmd.forwardmove = 0;
        cmd.rightmove = 0;
        cmd.upmove = cmd.upmove.min(0);
        if speed > 60.0 {
            let back = [-v[0] / speed, -v[1] / speed, 0.0];
            cmd.forwardmove = (dot(&back, &f) * 127.0) as i8;
            cmd.rightmove = (dot(&back, &r) * 127.0) as i8;
        }
    };
    let ok = |dir: &Vec3| safe_ahead(w, i, &pos, dir, dist) || (w.bots.states[i].drop_dir.is_some_and(|d| dot(&d, dir) > 0.8) && landing(w, i, &pos, dir, dist).is_some());
    // already sliding towards an edge
    if speed > 80.0 && !ok(&[v[0] / speed, v[1] / speed, 0.0]) {
        brake(cmd);
        return;
    }
    if cmd.forwardmove == 0 && cmd.rightmove == 0 {
        return;
    }
    let (fm, rm) = (cmd.forwardmove as f32, cmd.rightmove as f32);
    let wish = normalized(&[f[0] * fm + r[0] * rm, f[1] * fm + r[1] * rm, 0.0]);
    // a jump carries the bot much further than a step: check where it
    // comes down (the class's jump, at least running speed)
    if cmd.upmove > 0 {
        let class = bg::class(w.clients[i].ps.stats[Stat::Class as usize]);
        let air = 2.0 * class.jump_magnitude / w.level.g_gravity.max(1.0);
        let reach = speed.max(320.0 * class.speed) * air + 32.0;
        let vdir = if speed > 80.0 { [v[0] / speed, v[1] / speed, 0.0] } else { wish };
        if ![wish, vdir].iter().all(|d| [0.33f32, 0.66, 1.0].iter().all(|&f| safe_ahead(w, i, &pos, d, (reach * f).max(dist)))) {
            cmd.upmove = 0;
        }
    }
    if ok(&wish) {
        return;
    }
    let fwd = scale(&[f[0], f[1], 0.0], fm.signum());
    let right = scale(&[r[0], r[1], 0.0], rm.signum());
    if fm != 0.0 && rm != 0.0 && safe_ahead(w, i, &pos, &fwd, dist) {
        cmd.rightmove = 0;
    } else if fm != 0.0 && rm != 0.0 && safe_ahead(w, i, &pos, &right, dist) {
        cmd.forwardmove = 0;
    } else {
        brake(cmd);
    }
    cmd.upmove = cmd.upmove.min(0);
}

/// A wall or ledge right in front at step height (one Pmove cannot step).
fn blocked_ahead(w: &World, i: usize, dir: &Vec3) -> bool {
    let pos = w.clients[i].ps.origin;
    let (lo, hi) = ([-12.0, -12.0, 0.0], [12.0, 12.0, 4.0]);
    let start = [pos[0], pos[1], pos[2] - 24.0 + STEP + 2.0];
    let end = ma(&start, 28.0, dir);
    let tr = w.trace(&start, Some(&lo), Some(&hi), &end, i as i32, k::MASK_PLAYERSOLID);
    tr.fraction < 1.0 && tr.plane.normal[2] < 0.7
}

/// The centre of a structure or player within `dist` along `dir`, if one
/// is in the bot's way.
fn obstacle(w: &World, i: usize, dir: &Vec3, dist: f32) -> Option<Vec3> {
    let pos = w.clients[i].ps.origin;
    let (lo, hi) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 8.0]);
    let feet = pos[2] + bg::class(w.clients[i].ps.stats[Stat::Class as usize]).mins[2];
    let s = [pos[0], pos[1], feet + STEP];
    let tr = w.trace(&s, Some(&lo), Some(&hi), &ma(&s, dist, dir), i as i32, k::MASK_PLAYERSOLID);
    let e = tr.entity_num;
    if tr.fraction == 1.0 || e < 0 || e as usize >= w.num_entities {
        return None;
    }
    let x = &w.ents[e as usize];
    (x.etype == EType::Buildable || (e as usize) < MAX_CLIENTS).then(|| center(w, e as usize))
}

/// Can the bot walk straight at `at` (no wall at step height in between)?
fn direct(w: &World, i: usize, at: &Vec3) -> bool {
    let pos = w.clients[i].ps.origin;
    if (at[2] - pos[2]).abs() > 96.0 {
        return false;
    }
    let (lo, hi) = ([-8.0, -8.0, 0.0], [8.0, 8.0, 8.0]);
    let s = [pos[0], pos[1], pos[2] + STEP];
    let e = [at[0], at[1], at[2].max(pos[2]) + STEP];
    let tr = w.trace(&s, Some(&lo), Some(&hi), &e, i as i32, k::MASK_PLAYERSOLID);
    tr.fraction == 1.0 || tr.entity_num >= MAX_CLIENTS as i32 && w.ents.get(tr.entity_num as usize).map(|x| x.etype == EType::Buildable).unwrap_or(false) || (tr.entity_num >= 0 && (tr.entity_num as usize) < MAX_CLIENTS)
}

/// Choose a new route to `g` (or wander one link when there is none).
fn repath(w: &mut World, i: usize, g: Option<Vec3>) {
    let t = w.level.time;
    let pos = w.clients[i].ps.origin;
    let s = &mut w.bots.states[i];
    s.next_repath = t + 3000;
    s.goal = g;
    // a start node the bot just failed to reach is skipped
    let from = w.nav.as_ref().and_then(|n| n.nearest_where(w, &pos, true, &|id| !w.bots.blocked.iter().any(|&(a, b, until)| a == usize::MAX && b == id && until > t)));
    let mut path = match (g, from) {
        (Some(g), Some(from)) => nearest_waypoint(w, &g, false).map(|to| route(w, i, from, to)).unwrap_or_default(),
        _ => Vec::new(),
    };
    let arrived = g.is_some() && path.len() <= 1 && from.is_some() && g.and_then(|g| nearest_waypoint(w, &g, false)) == from;
    let wandering = path.is_empty() && !arrived;
    if wandering {
        // no route: take a random link out of here and try again soon
        if let (Some(from), Some(nav)) = (from, w.nav.as_ref()) {
            let fall = max_fall(w, i);
            let outs: Vec<usize> = nav.links[from].iter().filter(|l| l.fall <= fall).map(|l| l.to as usize).collect();
            path.push(from);
            if !outs.is_empty() {
                let pick = (w.bots.rng.random() * outs.len() as f32) as usize % outs.len();
                path.push(outs[pick]);
            }
        }
        w.bots.states[i].next_repath = t + 1500;
    }
    let s = &mut w.bots.states[i];
    s.wandering = wandering;
    s.arrived = arrived;
    s.path = path;
}

/// Walk the path; returns false when there is nowhere to go.
fn follow_path(w: &mut World, i: usize, cmd: &mut UserCmd) -> bool {
    let t = w.level.time;
    let pos = w.clients[i].ps.origin;
    // generated BSP graphs are dense: cut fewer corners near ledges
    let reach = if w.has_waypoints { REACH } else { 28.0 };
    while let Some(&n) = w.bots.states[i].path.first() {
        let o = node(w, n);
        let flat = [o[0] - pos[0], o[1] - pos[1], 0.0];
        if length(&flat) < reach && (o[2] - pos[2]).abs() < 64.0 {
            let s = &mut w.bots.states[i];
            s.last_node = Some(n);
            s.path.remove(0);
            s.progress = None;
            if s.path.is_empty() && s.goal.is_some() && !s.wandering {
                s.arrived = true;
            }
        } else {
            break;
        }
    }
    if let Some(&n) = w.bots.states[i].path.first() {
        let target = node(w, n);
        let flat_d = length(&[target[0] - pos[0], target[1] - pos[1], 0.0]);
        // progress along the link; none for a while: block it and route around
        match w.bots.states[i].progress {
            Some((pn, best, since)) if pn == n => {
                if flat_d < best - 24.0 {
                    w.bots.states[i].progress = Some((n, flat_d, t));
                } else if t - since > NO_PROGRESS_MS {
                    let from = w.bots.states[i].last_node.filter(|&f| f != n).unwrap_or(usize::MAX);
                    w.bots.blocked.push((from, n, t + BLOCK_MS));
                    w.bots.states[i].last_node = None;
                    w.bots.states[i].next_repath = t;
                    w.bots.states[i].progress = None;
                }
            }
            _ => w.bots.states[i].progress = Some((n, flat_d, t)),
        }
        let mut o = target;
        // the graph ignores structures and players: steer around one in the way
        let dir = normalized(&[target[0] - pos[0], target[1] - pos[1], 0.0]);
        if let Some(c) = obstacle(w, i, &dir, flat_d.min(96.0)) {
            let away = [pos[0] - c[0], pos[1] - c[1], 0.0];
            let mut perp = [-dir[1], dir[0], 0.0];
            if dot(&perp, &away) < 0.0 {
                perp = neg(&perp);
            }
            o = ma(&pos, 100.0, &normalized(&ma(&dir, 1.5, &perp)));
        }
        o[2] = eye(w, i)[2];
        let err = aim_cmd(w, i, &o);
        cmd.angles = w.clients[i].cmd.angles;
        cmd.forwardmove = if err > 60.0 { 0 } else { 127 };
        let kind = w.bots.states[i].last_node.and_then(|a| w.nav.as_ref().and_then(|nv| nv.link(a, n)));
        let jump_link = kind == Some(LinkKind::Jump);
        if kind == Some(LinkKind::Drop) {
            w.bots.states[i].drop_dir = Some(dir);
            // no run-up: step off gently so the fall lands where the graph says
            cmd.forwardmove = cmd.forwardmove.min(80);
        }
        if cmd.forwardmove > 0 && (jump_link || target[2] > pos[2] + STEP) && blocked_ahead(w, i, &dir) {
            cmd.upmove = 127;
        }
        return true;
    }
    if let (Some(g), true) = (w.bots.states[i].goal, w.bots.states[i].arrived) {
        // last leg: walk straight onto the goal (e.g. stand on a medistat)
        let flat = [g[0] - pos[0], g[1] - pos[1], 0.0];
        if length(&flat) > 12.0 && length(&flat) < 400.0 {
            let mut o = g;
            o[2] = eye(w, i)[2];
            aim_cmd(w, i, &o);
            cmd.angles = w.clients[i].cmd.angles;
            cmd.forwardmove = if length(&flat) > 64.0 { 127 } else { 64 };
            return true;
        }
    }
    false
}

fn think(w: &mut World, i: usize) {
    let t = w.level.time;
    w.bots.states[i].drop_dir = None;
    let want = w.bots.states[i].team;
    if w.clients[i].team != want {
        client::change_team(w, i, want);
    }
    let mut cmd = UserCmd { server_time: t, angles: w.clients[i].cmd.angles, ..Default::default() };
    if w.clients[i].spectator != SpectatorState::Not as i32 {
        // (the first frames run before the build points are counted)
        if w.clients[i].class_selection.is_none() && w.level.time >= 500 {
            // one builder per team while something is missing
            let other_builder = (0..MAX_CLIENTS).any(|j| {
                j != i && w.clients[j].connected && w.clients[j].team == want && (is_builder(w, j) && w.clients[j].spectator == SpectatorState::Not as i32 || w.clients[j].human_item == Some(WeaponId::Ckit) && w.clients[j].spectator != SpectatorState::Not as i32 || matches!(w.clients[j].class_selection, Some(ClassId::Builder | ClassId::Builderupg)) && w.clients[j].spectator != SpectatorState::Not as i32)
            });
            let build = needed(w, want).is_some() && !other_builder;
            let choice = match (want, build) {
                (Team::Aliens, true) => "builder",
                (Team::Aliens, false) => "level0",
                (Team::Humans, true) => "ckit",
                (Team::Humans, false) => "rifle",
                (Team::None, _) => "",
            };
            if !choice.is_empty() {
                let _ = client::cmd_class(w, i, choice);
            }
        }
        let s = &mut w.bots.states[i];
        s.path.clear();
        s.last_node = None;
        s.progress = None;
        s.next_repath = t;
        w.clients[i].cmd = cmd;
        return;
    }
    if w.clients[i].ps.stats[Stat::Health as usize] <= 0 {
        w.clients[i].cmd = cmd;
        return;
    }
    shop(w, i);
    if w.clients[i].spectator != SpectatorState::Not as i32 {
        return; // evolving re-spawned us this frame
    }

    // ---- target
    if t >= w.bots.states[i].next_target {
        w.bots.states[i].next_target = t + 250;
        let old = w.bots.states[i].target;
        let mut tg = if is_builder(w, i) { None } else { pick_target(w, i) };
        // wounded bots only fight what is close
        if hurt(w, i) {
            if let Some(e) = tg {
                if distance(&w.clients[i].ps.origin, &w.ents[e].origin) > 400.0 {
                    tg = None;
                }
            }
        }
        if tg.is_some() && tg != old {
            w.bots.states[i].react_at = t + 200 + (w.bots.rng.random() * 400.0) as i32;
        }
        w.bots.states[i].target = tg;
    }
    if let Some(e) = w.bots.states[i].target {
        if !w.ents[e].inuse || w.ents[e].health <= 0 || !w.ents[e].takedamage {
            w.bots.states[i].target = None;
        }
    }
    let alien = w.clients[i].team == Team::Aliens;
    let pos = w.clients[i].ps.origin;
    let mut moving = true;
    w.clients[i].cmd = cmd;
    let mut navigate = true;
    if let Some(e) = w.bots.states[i].target {
        let mut at = center(w, e);
        let d = distance(&eye(w, i), &at);
        if e < MAX_CLIENTS {
            at = ma(&at, -AIM_LAG, &w.clients[e].ps.velocity);
        }
        if t >= w.bots.states[i].aim_off_until {
            let r = &mut w.bots.rng;
            let off = [r.crandom() as f32, r.crandom() as f32, r.crandom() as f32];
            let until = t + 300 + (r.random() * 400.0) as i32;
            w.bots.states[i].aim_off = off;
            w.bots.states[i].aim_off_until = until;
        }
        let spread = if alien { AIM_SPREAD_ALIEN } else { AIM_SPREAD_HUMAN };
        // a structure does not dodge: a quarter of the error
        let spread = if e >= MAX_CLIENTS { spread * 0.25 } else { spread };
        at = ma(&at, d * spread, &w.bots.states[i].aim_off);
        // aliens charge what they can run at; otherwise they path towards it
        let charge = !alien || direct(w, i, &w.ents[e].origin.clone()) || d < 160.0;
        if charge {
            navigate = false;
            let err = aim_cmd(w, i, &at);
            cmd.angles = w.clients[i].cmd.angles;
            let range = if alien {
                WeaponId::from_num(w.clients[i].ps.weapon as u8).and_then(crate::gen::weapon_behaviors::for_target).and_then(|b| b.range).unwrap_or(64.0) + 48.0
            } else {
                1200.0
            };
            if d < range && err < 12.0 && t >= w.bots.states[i].react_at {
                cmd.buttons |= k::BUTTON_ATTACK;
            }
            if alien {
                cmd.forwardmove = 127;
                // zig-zag on the approach so rifles miss more
                if d > 150.0 {
                    if t >= w.bots.states[i].strafe_until {
                        w.bots.states[i].strafe_until = t + 300 + (w.bots.rng.random() * 500.0) as i32;
                        w.bots.states[i].strafe = if w.bots.states[i].strafe > 0 { -127 } else { 127 };
                    }
                    cmd.rightmove = w.bots.states[i].strafe;
                }
                if d < 300.0 && w.bots.rng.random() < 0.02 {
                    cmd.upmove = 127;
                }
            } else {
                // humans keep their distance and strafe
                cmd.forwardmove = if d > 500.0 { 127 } else if d < 250.0 { -127 } else { 0 };
                if t >= w.bots.states[i].strafe_until {
                    w.bots.states[i].strafe_until = t + 600 + (w.bots.rng.random() * 900.0) as i32;
                    w.bots.states[i].strafe = if w.bots.rng.random() < 0.5 { -127 } else { 127 };
                }
                cmd.rightmove = w.bots.states[i].strafe;
                moving = cmd.forwardmove != 0;
            }
            // whatever the route was, it starts again from here
            w.bots.states[i].progress = None;
            w.bots.states[i].next_repath = w.bots.states[i].next_repath.min(t + 500);
        } else if t >= w.bots.states[i].next_repath || w.bots.states[i].goal.map(|g| distance(&g, &w.ents[e].origin) > 200.0).unwrap_or(true) {
            let g = w.ents[e].origin;
            repath(w, i, Some(g));
            w.bots.states[i].next_repath = t + 1000;
        }
    }
    // ---- build: near the site, select the structure and place it where
    // the server marks the spot valid; otherwise turn and look elsewhere
    if navigate && is_builder(w, i) {
        if let Some(b) = needed(w, w.clients[i].team) {
            let site = build_site(w, i);
            if length(&[site[0] - pos[0], site[1] - pos[1], 0.0]) < 400.0 {
                navigate = false;
                moving = false;
                let since = *w.bots.states[i].build_since.get_or_insert(t);
                if t - since > BUILD_GIVE_UP_MS {
                    // no valid spot here: leave it to a later try
                    let team = w.clients[i].team;
                    w.bots.no_build.push((team, b, t + 60_000));
                    w.bots.states[i].build_since = None;
                }
                let sel = w.clients[i].ps.stats[Stat::Buildable as usize];
                if sel & !k::SB_VALID_TOGGLEBIT != b.num() as i32 {
                    if client::cmd_build(w, i, b.id()).is_err() {
                        // refused outright (stage, surrender...): not now
                        let team = w.clients[i].team;
                        w.bots.no_build.push((team, b, t + 60_000));
                    }
                } else if sel & k::SB_VALID_TOGGLEBIT != 0 && w.clients[i].ps.stats[Stat::Misc as usize] <= 0 {
                    cmd.buttons |= k::BUTTON_ATTACK;
                } else if t >= w.bots.states[i].strafe_until {
                    // look somewhere else: turn, and step away from the site
                    w.bots.states[i].strafe_until = t + 500;
                    let yaw = w.clients[i].ps.viewangles[1] + 40.0 + w.bots.rng.random() * 60.0;
                    let away = sub(&pos, &site);
                    w.bots.states[i].escape = Some(normalized(&[away[0], away[1], 0.0]));
                    let c = &mut w.clients[i];
                    c.cmd.angles[1] = angle2short_f(yaw) - c.ps.delta_angles[1];
                    cmd.angles = c.cmd.angles;
                }
                if sel & k::SB_VALID_TOGGLEBIT == 0 && t < w.bots.states[i].strafe_until {
                    // walk outwards a little while looking around
                    if let Some(dir) = w.bots.states[i].escape {
                        let (f, r, _) = angle_vectors(&[0.0, cmd_yaw(w, i, &cmd), 0.0]);
                        cmd.forwardmove = (dot(&dir, &f) * 64.0) as i8;
                        cmd.rightmove = (dot(&dir, &r) * 64.0) as i8;
                    }
                }
            }
        }
    }
    if navigate && alien && guards(w, i) {
        let valid = w.bots.states[i].patrol.is_some_and(|p| w.ents[p].inuse && w.ents[p].etype == EType::Buildable && w.ents[p].team == Team::Aliens && w.ents[p].health > 0);
        if !valid || t >= w.bots.states[i].patrol_until {
            let own: Vec<usize> = (MAX_CLIENTS..w.num_entities).filter(|&e| w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].team == Team::Aliens && w.ents[e].health > 0).collect();
            let pick = if own.is_empty() { None } else { Some(own[(w.bots.rng.random() * own.len() as f32) as usize % own.len()]) };
            let s = &mut w.bots.states[i];
            s.patrol = pick;
            s.patrol_until = t + 8000 + (w.bots.rng.random() * 8000.0) as i32;
            s.next_repath = t;
        }
    }
    if navigate {
        w.bots.states[i].build_since = None;
        if w.bots.states[i].target.is_none() && (t >= w.bots.states[i].next_repath || (w.bots.states[i].path.is_empty() && w.bots.states[i].goal.is_none())) {
            let g = goal(w, i);
            repath(w, i, g);
        }
        moving = follow_path(w, i, &mut cmd);
    }

    // ---- unstick
    if moving {
        if t >= w.bots.states[i].stuck_check {
            if distance(&w.bots.states[i].last_pos, &pos) < 16.0 {
                // a shut door on the way to the next node: route around it for a while
                if let (Some(&next), Some(from)) = (w.bots.states[i].path.first(), w.bots.states[i].last_node) {
                    let o = node(w, next);
                    let tr = w.trace(&pos, None, None, &o, i as i32, k::MASK_PLAYERSOLID);
                    if tr.fraction < 1.0 && tr.entity_num >= 0 && (tr.entity_num as usize) < w.ents.len() && w.ents[tr.entity_num as usize].etype == EType::Mover {
                        w.bots.blocked.push((from, next, t + 20_000));
                    }
                }
                // a way out: the first of eight directions (from a random one)
                // that is open and safe
                let first = (w.bots.rng.random() * 8.0) as usize;
                let mut escape = None;
                for k_ in 0..8 {
                    let a = ((first + k_) % 8) as f32 * std::f32::consts::FRAC_PI_4;
                    let dir = [a.cos(), a.sin(), 0.0];
                    if !blocked_ahead(w, i, &dir) && safe_ahead(w, i, &pos, &dir, 64.0) && safe_ahead(w, i, &pos, &dir, 144.0) {
                        escape = Some(dir);
                        break;
                    }
                }
                let side = if w.bots.rng.random() < 0.5 { -127i8 } else { 127 };
                let s = &mut w.bots.states[i];
                s.stuck_count += 1;
                s.escape = escape;
                s.unstick_until = t + 700;
                s.strafe = side;
                s.next_repath = s.next_repath.min(t + 1000);
            } else {
                w.bots.states[i].stuck_count = 0;
            }
            let s = &mut w.bots.states[i];
            s.last_pos = pos;
            s.stuck_check = t + 1000;
        }
        let s = &w.bots.states[i];
        if t < s.unstick_until {
            cmd.upmove = 127;
            match s.escape {
                // alternate: the way out, then a jump-strafe
                Some(dir) if s.stuck_count % 2 == 1 => {
                    let (f, r, _) = angle_vectors(&[0.0, cmd_yaw(w, i, &cmd), 0.0]);
                    cmd.forwardmove = (dot(&dir, &f) * 127.0) as i8;
                    cmd.rightmove = (dot(&dir, &r) * 127.0) as i8;
                }
                _ => cmd.rightmove = s.strafe,
            }
        }
    }
    // ---- wedged: lingering in one spot while trying to travel
    let travelling = navigate && moving && w.bots.states[i].target.is_none() && !w.bots.states[i].arrived;
    let msec = w.level.msec;
    match w.bots.states[i].anchor {
        Some((a, spent)) if distance(&a, &pos) <= WEDGED_RADIUS => {
            let spent = spent + if travelling { msec } else { 0 };
            w.bots.states[i].anchor = Some((a, spent));
            if spent > WEDGED_MS && w.clients[i].suicide_time == 0 {
                let _ = client::cmd_kill(w, i);
            }
        }
        _ => {
            if w.clients[i].suicide_time != 0 {
                let _ = client::cmd_kill(w, i); // free again: cancel
            }
            w.bots.states[i].anchor = Some((pos, 0));
        }
    }
    guard(w, i, &mut cmd);
    // Pmove only jumps again once the jump key was let go
    if cmd.upmove > 0 && w.clients[i].ps.pm_flags & k::PMF_JUMP_HELD != 0 && w.clients[i].ps.ground_entity_num != k::ENTITYNUM_NONE {
        cmd.upmove = 0;
    }
    w.clients[i].cmd = cmd;
}
