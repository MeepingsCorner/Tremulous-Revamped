//! Shared bot-match harness: runs a headless match on a world and measures
//! what the `bots` definition of done checks. It only observes the world
//! (events, client and entity state); it never changes it. Used by
//! `tests/bots.rs` and `examples/match_sim.rs`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use trem_sim::gen::enums::{Event, MeansOfDeath, SpectatorState, Stat, Team};
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::world::{EType, World};
use trem_sim::kernel::types::k;
use trem_sim::kernel::{bot, bsp, combat};

pub const OFFICIAL_MAPS: [&str; 8] = ["atcs", "arachnid2", "karith", "nexus6", "niveus", "transit", "tremor", "uncreation"];

/// A bot that stays within this radius while it has no target and is not at
/// its goal is "stuck".
pub const STUCK_RADIUS: f32 = 96.0;

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub map: String,
    pub ms: i32,
    /// kills of enemy players, [aliens, humans]
    pub kills: [i32; 2],
    /// deaths with no enemy player as the attacker, by means of death
    pub world_deaths: BTreeMap<String, i32>,
    pub mods: BTreeMap<String, i32>,
    /// deaths to the map: falling, trigger_hurt, crush, lava/slime/water,
    /// target_kill (MOD_TELEFRAG), target_laser
    pub hazard_deaths: i32,
    /// `/kill` (a wedged bot asks for it)
    pub suicides: i32,
    /// longest stuck spell of any bot (ms) and who/where
    pub max_stuck_ms: i32,
    pub max_stuck_at: String,
    pub stuck_ms_total: i64,
    /// health removed from the enemy's structures, [by aliens, by humans]
    pub struct_damage: [i32; 2],
    /// enemy structures destroyed, [by aliens, by humans]
    pub struct_kills: [i32; 2],
    /// alien evolutions / human purchases (credit spends)
    pub evolves: i32,
    pub buys: i32,
    pub winner: Option<Team>,
}

impl Report {
    pub fn line(&self) -> String {
        format!(
            "{:<11} {:>4}s kills A{:>3} H{:>3} | hazard {:>2} suicide {} world {:?} | stuck max {:>5.1}s total {:>6.1}s {} | sdmg A{:>6} H{:>6} skill A{:>2} H{:>2} | evolves {:>3} buys {:>3} | winner {:?}",
            self.map,
            self.ms / 1000,
            self.kills[0],
            self.kills[1],
            self.hazard_deaths,
            self.suicides,
            self.world_deaths,
            self.max_stuck_ms as f32 / 1000.0,
            self.stuck_ms_total as f32 / 1000.0,
            self.max_stuck_at,
            self.struct_damage[0],
            self.struct_damage[1],
            self.struct_kills[0],
            self.struct_kills[1],
            self.evolves,
            self.buys,
            self.winner
        )
    }
}

/// The official map BSP, from `godot/base/map-<name>-1.1.0.pk3` (or
/// `$TREM_BSP_DIR/<name>.bsp`); None when it is not there.
pub fn load_bsp(name: &str) -> Option<World> {
    let mut v = Vec::new();
    if let Ok(dir) = std::env::var("TREM_BSP_DIR") {
        if let Ok(d) = std::fs::read(PathBuf::from(dir).join(format!("{name}.bsp"))) {
            v = d;
        }
    }
    if v.is_empty() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../godot/base/map-{name}-1.1.0.pk3"));
        let f = std::fs::File::open(p).ok()?;
        let mut z = zip::ZipArchive::new(f).ok()?;
        let mut e = z.by_name(&format!("maps/{name}.bsp")).ok()?;
        e.read_to_end(&mut v).ok()?;
    }
    let b = bsp::parse(&v).ok()?;
    Some(World::from_bsp(&b, name))
}

pub fn load(map: &str) -> Option<World> {
    if map == "arena" {
        Some(World::new(MapId::Arena))
    } else if map.ends_with(".bsp") {
        let b = bsp::parse(&std::fs::read(map).ok()?).ok()?;
        let name = std::path::Path::new(map).file_stem()?.to_string_lossy().into_owned();
        Some(World::from_bsp(&b, &name))
    } else {
        load_bsp(map)
    }
}

fn tix(t: Team) -> Option<usize> {
    match t {
        Team::Aliens => Some(0),
        Team::Humans => Some(1),
        Team::None => None,
    }
}

/// Add `n` bots per team and run `ms` of game time in 16 ms frames (the
/// Godot host's tick), stopping at intermission.
pub fn run(w: &mut World, map: &str, n: usize, ms: i32, verbose: bool) -> Report {
    run_with(w, map, n, n, 0, ms, verbose)
}

/// The objective scenario: `n` bots of `team` alone on the map, each
/// handed the team's credit cap (as the `give funds` cheat would), so the
/// economy and the attack on the enemy base are tested without a fight.
pub fn run_objective(w: &mut World, map: &str, team: Team, n: usize, ms: i32, verbose: bool) -> Report {
    let (a, h) = if team == Team::Aliens { (n, 0) } else { (0, n) };
    let cap = if team == Team::Aliens { k::ALIEN_MAX_CREDITS } else { k::HUMAN_MAX_CREDITS };
    run_with(w, map, a, h, cap, ms, verbose)
}

/// `aliens` and `humans` bots, each starting with `credits`.
pub fn run_with(w: &mut World, map: &str, aliens: usize, humans: usize, credits: i32, ms: i32, verbose: bool) -> Report {
    for k in 0..aliens.max(humans) {
        for (team, n, name) in [(Team::Aliens, aliens, "alien"), (Team::Humans, humans, "human")] {
            if k < n {
                if let Some(i) = bot::add_bot(w, &format!("{name}{k}"), team) {
                    combat::add_credit(w, i, credits, true);
                }
            }
        }
    }
    let msec = 16;
    let mut r = Report { map: map.to_string(), ..Default::default() };
    let mut health: Vec<i32> = vec![0; w.ents.len()];
    let mut alive_b: Vec<bool> = vec![false; w.ents.len()];
    let mut credit = [0i32; 64];
    let mut anchor: [Option<([f32; 3], i32)>; 64] = [None; 64];
    for i in 0..64 {
        credit[i] = w.clients[i].credit;
    }
    for e in 0..w.ents.len() {
        health[e] = w.ents[e].health;
        alive_b[e] = w.ents[e].inuse && w.ents[e].etype == EType::Buildable && w.ents[e].health > 0;
    }
    let start = w.level.time;
    while w.level.time - start < ms {
        w.frame(msec);
        let t = w.level.time;
        for ev in w.drain_events() {
            if ev.event != Event::Obituary {
                continue;
            }
            let m = MeansOfDeath::from_raw(ev.parm as u8);
            let ms_ = format!("{m:?}");
            *r.mods.entry(ms_.clone()).or_insert(0) += 1;
            let victim = ev.weapon as usize;
            let att = ev.other;
            let vt = w.clients.get(victim).map(|c| c.team).unwrap_or(Team::None);
            let enemy_kill = att >= 0 && (att as usize) < 64 && att as usize != victim && w.clients[att as usize].team != vt && w.clients[att as usize].team != Team::None;
            if enemy_kill {
                if let Some(ti) = tix(w.clients[att as usize].team) {
                    r.kills[ti] += 1;
                }
            } else {
                *r.world_deaths.entry(ms_).or_insert(0) += 1;
                if matches!(
                    m,
                    Some(MeansOfDeath::Falling | MeansOfDeath::TriggerHurt | MeansOfDeath::Crush | MeansOfDeath::Lava | MeansOfDeath::Slime | MeansOfDeath::Water | MeansOfDeath::Telefrag | MeansOfDeath::TargetLaser)
                ) {
                    r.hazard_deaths += 1;
                }
                if m == Some(MeansOfDeath::Suicide) {
                    r.suicides += 1;
                }
                if verbose {
                    let c = &w.clients[victim];
                    println!("  world death t={} {} {:?} at {:.0},{:.0},{:.0}", t, c.name, m, c.ps.origin[0], c.ps.origin[1], c.ps.origin[2]);
                }
            }
        }
        if health.len() < w.ents.len() {
            health.resize(w.ents.len(), 0);
            alive_b.resize(w.ents.len(), false);
        }
        for e in 0..w.num_entities {
            let x = &w.ents[e];
            let is_b = x.inuse && x.etype == EType::Buildable;
            if is_b && alive_b[e] {
                if let Some(ti) = tix(x.team) {
                    let by = 1 - ti;
                    let drop = health[e] - x.health.max(0);
                    if drop > 0 {
                        r.struct_damage[by] += drop;
                    }
                    if x.health <= 0 {
                        r.struct_kills[by] += 1;
                        if verbose {
                            println!("  structure lost t={} {:?} {:?} spawned {}", t, x.team, x.buildable, x.spawned);
                        }
                    }
                }
            }
            health[e] = x.health;
            alive_b[e] = is_b && x.health > 0;
        }
        for i in 0..64 {
            let c = &w.clients[i];
            if !c.connected {
                continue;
            }
            if c.credit < credit[i] {
                if verbose {
                    println!("  spend t={} {} class {} weapon {} credit {} -> {}", t, c.name, c.ps.stats[Stat::Class as usize], c.ps.stats[Stat::Weapon as usize], credit[i], c.credit);
                }
                match c.team {
                    Team::Aliens => r.evolves += 1,
                    Team::Humans => r.buys += 1,
                    _ => {}
                }
            }
            credit[i] = c.credit;
            if !c.is_bot {
                continue;
            }
            let alive = c.spectator == SpectatorState::Not as i32 && c.ps.stats[Stat::Health as usize] > 0;
            let st = &w.bots.states[i];
            let pos = c.ps.origin;
            let at_goal = st.goal.map(|g| ((g[0] - pos[0]).powi(2) + (g[1] - pos[1]).powi(2)).sqrt() < 160.0).unwrap_or(true);
            if !alive || st.target.is_some() || at_goal {
                anchor[i] = None;
                continue;
            }
            match anchor[i] {
                Some((a, since)) => {
                    let d = ((a[0] - pos[0]).powi(2) + (a[1] - pos[1]).powi(2) + (a[2] - pos[2]).powi(2)).sqrt();
                    if d > STUCK_RADIUS {
                        let dur = t - since;
                        if dur > 3000 {
                            r.stuck_ms_total += dur as i64;
                        }
                        anchor[i] = Some((pos, t));
                    } else {
                        let dur = t - since;
                        if dur > r.max_stuck_ms {
                            r.max_stuck_ms = dur;
                            r.max_stuck_at = format!("{}@{:.0},{:.0},{:.0}", c.name, a[0], a[1], a[2]);
                        }
                    }
                }
                None => anchor[i] = Some((pos, t)),
            }
        }
        if verbose && (t - start) % 60_000 < msec {
            println!("  t={:>4}s kills A{} H{} sdmg A{} H{} evolves {} buys {} stage A{} H{}", (t - start) / 1000, r.kills[0], r.kills[1], r.struct_damage[0], r.struct_damage[1], r.evolves, r.buys, w.level.alien_stage, w.level.human_stage);
        }
        if w.level.intermission_time != 0 {
            break;
        }
    }
    r.ms = w.level.time - start;
    r.winner = w.level.winner;
    r
}

/// Reseed the bots' decisions and the game RNG (balance runs over several
/// seeds; the default seeds are the ones every other measurement uses).
pub fn seed(w: &mut World, s: u32) {
    w.bots.rng = trem_sim::kernel::rng::Rng::new(0xb075_eed5 ^ s.wrapping_mul(0x9e37_79b9));
    w.rng = trem_sim::kernel::rng::Rng::new(0x5eed_1234 ^ s.wrapping_mul(0x85eb_ca6b));
}

/// Balance over match reports: per map and overall, wins of each team,
/// draws (time ran out) and the mean match length.
#[derive(Debug, Default, Clone, Copy)]
pub struct Balance {
    pub runs: i32,
    pub alien_wins: i32,
    pub human_wins: i32,
    pub mean_s: f32,
}

impl Balance {
    pub fn of<'a>(rs: impl Iterator<Item = &'a Report>) -> Balance {
        let mut b = Balance::default();
        let mut total = 0i64;
        for r in rs {
            b.runs += 1;
            total += r.ms as i64;
            match r.winner {
                Some(Team::Aliens) => b.alien_wins += 1,
                Some(Team::Humans) => b.human_wins += 1,
                _ => {}
            }
        }
        b.mean_s = if b.runs > 0 { total as f32 / b.runs as f32 / 1000.0 } else { 0.0 };
        b
    }
    /// The larger team's share of all runs.
    pub fn max_win_share(&self) -> f32 {
        self.alien_wins.max(self.human_wins) as f32 / self.runs.max(1) as f32
    }
    pub fn line(&self, what: &str) -> String {
        format!("{what:<11} runs {:>3}  aliens won {:>3}  humans won {:>3}  draws {:>3}  mean {:>5.0}s", self.runs, self.alien_wins, self.human_wins, self.runs - self.alien_wins - self.human_wins, self.mean_s)
    }
}

/// `Balance` per map and over all runs, one line each.
pub fn balance_summary(all: &[Report]) -> String {
    let mut maps: Vec<&str> = all.iter().map(|r| r.map.as_str()).collect();
    maps.dedup();
    let mut out = String::from("balance\n");
    for m in &maps {
        out += &Balance::of(all.iter().filter(|r| r.map == *m)).line(m);
        out.push('\n');
    }
    out += &Balance::of(all.iter()).line("all");
    out
}
