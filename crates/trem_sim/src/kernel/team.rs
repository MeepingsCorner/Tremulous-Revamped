//! Kernel `team` — the team-wide bookkeeping systems of game/g_main.c:
//! G_CountSpawns, G_CalculateBuildPoints (queues), G_CalculateStages,
//! G_CalculateAvgPlayers, CalculateRanks (team counts) and CheckExitRules.
//! Thresholds and max stages come from the constants sheet (g_* defaults).

use super::types::k;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::buildables::BuildableId;
use crate::gen::enums::{SpectatorState, Team};

pub const PLAYER_COUNT_MOD: f32 = 5.0;
/// g_local.h `INTERMISSION_DELAY_TIME`.
pub const INTERMISSION_DELAY_TIME: i32 = 1000;

/// `G_CountSpawns` system.
pub fn count_spawns(w: &mut World) {
    let (mut a, mut h) = (0, 0);
    for i in 1..w.num_entities {
        let e = &w.ents[i];
        if !e.inuse || e.etype != EType::Buildable || e.health <= 0 {
            continue;
        }
        match e.buildable {
            Some(BuildableId::Eggpod) => a += 1,
            Some(BuildableId::Telenode) => h += 1,
            _ => {}
        }
    }
    w.level.num_alien_spawns = a;
    w.level.num_human_spawns = h;
    calculate_ranks(w);
}

/// The team-count half of `CalculateRanks` (scoreboard sorting is presentation).
pub fn calculate_ranks(w: &mut World) {
    let l = &mut w.level;
    l.num_alien_clients = 0;
    l.num_human_clients = 0;
    l.num_live_alien_clients = 0;
    l.num_live_human_clients = 0;
    for i in 0..MAX_CLIENTS {
        let c = &w.clients[i];
        if !c.connected {
            continue;
        }
        let live = c.spectator == SpectatorState::Not as i32 && c.ps.stats[crate::gen::enums::Stat::Health as usize] > 0;
        match c.team {
            Team::Aliens => {
                l.num_alien_clients += 1;
                if live {
                    l.num_live_alien_clients += 1;
                }
            }
            Team::Humans => {
                l.num_human_clients += 1;
                if live {
                    l.num_live_human_clients += 1;
                }
            }
            _ => {}
        }
    }
}

/// `G_NextQueueTime`.
pub fn next_queue_time(queued_bp: i32, total_bp: i32, base_rate: i32) -> i32 {
    if total_bp == 0 {
        return 0;
    }
    let fraction = queued_bp as f32 / total_bp as f32;
    ((1.0f32 - fraction) * base_rate as f32) as i32
}

/// `G_CalculateBuildPoints` system: queues, sudden death, spent BP and the
/// repeater build-point zones.
/// `SP_worldspawn`'s warmup (g_spawn.c): CS_WARMUP is "-1", or with
/// g_doWarmup the time the match starts, g_warmup seconds from now, and
/// the "Warmup:" log line.
pub fn init_warmup(w: &mut World) {
    w.level.cs_warmup = -1;
    if w.level.do_warmup {
        w.level.warmup_time = w.level.time - w.level.start_time + w.level.warmup * 1000;
        w.level.cs_warmup = w.level.warmup_time;
        let line = format!("Warmup: {}\n", w.level.warmup);
        super::log::log_printf(w, &line);
    }
}

/// `G_RunFrame`'s `!g_doWarmup.integer || level.warmupTime <= level.time`:
/// build points, stages, spawning, the player averages and the zaps only
/// run once the warmup is over.
pub fn warmup_over(w: &World) -> bool {
    !w.level.do_warmup || w.level.warmup_time <= w.level.time
}

pub fn calculate_build_points(w: &mut World) {
    if !warmup_over(w) {
        return;
    }
    const SUDDENDEATHWARNING: i32 = 60000;
    const TW_IMMINENT: i32 = 1;
    const TW_PASSED: i32 = 2;
    {
        let l = &mut w.level;
        while l.alien_bp_queue > 0 && l.alien_next_queue_time < l.time {
            l.alien_bp_queue -= 1;
            l.alien_next_queue_time += next_queue_time(l.alien_bp_queue, l.alien_bp, l.alien_queue_time);
        }
        while l.human_bp_queue > 0 && l.human_next_queue_time < l.time {
            l.human_bp_queue -= 1;
            l.human_next_queue_time += next_queue_time(l.human_bp_queue, l.human_bp, l.human_queue_time);
        }
    }
    let tsd = super::buildable::time_til_sudden_death(w);
    if tsd <= 0 && w.level.sudden_death_warning < TW_PASSED {
        super::log::log_printf(w, "Beginning Sudden Death\n");
        w.level.sudden_death_warning = TW_PASSED;
        let t = w.level.time;
        w.center_prints.push((t, None, None, "Sudden Death!".into()));
        super::buildable::clear_decon_marks(w);
        for i in 0..MAX_CLIENTS {
            if w.clients[i].connected {
                w.clients[i].ps.stats[crate::gen::enums::Stat::Buildable as usize] = 0;
            }
        }
    } else if tsd <= SUDDENDEATHWARNING && w.level.sudden_death_warning < TW_IMMINENT {
        let t = w.level.time;
        w.center_prints.push((t, None, None, format!("Sudden Death in {} seconds!", tsd / 1000)));
        w.level.sudden_death_warning = TW_IMMINENT;
    }
    w.level.human_bp_left = w.level.human_bp - w.level.human_bp_queue;
    w.level.alien_bp_left = w.level.alien_bp - w.level.alien_bp_queue;
    let rbp = w.level.repeater_bp;
    for z in w.level.bp_zones.iter_mut() {
        z.active = false;
        z.total = rbp;
    }
    for i in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[i];
        if e.etype != super::world::EType::Buildable || !e.inuse || e.e_flags & k::EF_DEAD != 0 {
            continue;
        }
        let Some(b) = e.buildable else { continue };
        if e.uses_bp_zone {
            let zi = e.bp_zone;
            if let Some(z) = w.level.bp_zones.get_mut(zi) {
                z.active = true;
            }
        }
        let cost = b.def().build_points;
        let e = &w.ents[i];
        if e.team == Team::Aliens {
            w.level.alien_bp_left -= cost;
        }
        if b == BuildableId::Repeater {
            w.level.human_bp_left -= cost;
        } else if b != BuildableId::Reactor {
            if let Some(p) = super::buildable::power_entity_for_entity(w, i) {
                let pe = &w.ents[p];
                if pe.buildable == Some(BuildableId::Reactor) {
                    w.level.human_bp_left -= cost;
                } else if pe.buildable == Some(BuildableId::Repeater) && pe.uses_bp_zone {
                    let zi = pe.bp_zone;
                    if let Some(z) = w.level.bp_zones.get_mut(zi) {
                        z.total -= cost;
                    }
                }
            }
        }
    }
    let rqt = w.level.repeater_queue_time;
    for i in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[i];
        if e.etype != super::world::EType::Buildable || e.e_flags & k::EF_DEAD != 0 || e.team != Team::Humans || e.buildable != Some(BuildableId::Repeater) {
            continue;
        }
        if !e.uses_bp_zone {
            continue;
        }
        let zi = e.bp_zone;
        let t = w.level.time;
        let Some(z) = w.level.bp_zones.get_mut(zi).filter(|z| z.active) else { continue };
        if tsd > 0 {
            while z.queued > 0 && z.next_queue_time < t {
                z.queued -= 1;
                z.next_queue_time += next_queue_time(z.queued, z.total, rqt);
            }
        } else {
            z.total = 0;
            z.queued = 0;
        }
    }
    w.level.human_bp_left = w.level.human_bp_left.max(0);
    w.level.alien_bp_left = w.level.alien_bp_left.max(0);
}

fn threshold(base: i32, mod_: f32) -> i32 {
    (base as f32 * mod_).ceil() as i32
}

/// `G_CalculateStages` system.
pub fn calculate_stages(w: &mut World) {
    if !warmup_over(w) {
        return;
    }
    let before = (w.level.alien_stage, w.level.human_stage);
    calculate_stages_inner(w);
    // G_Checktrigger_stages on every change of g_alienStage / g_humanStage
    if w.level.alien_stage != before.0 {
        let s = w.level.alien_stage;
        super::trigger::check_trigger_stages(w, Team::Aliens, s);
    }
    if w.level.human_stage != before.1 {
        let s = w.level.human_stage;
        super::trigger::check_trigger_stages(w, Team::Humans, s);
    }
}

fn calculate_stages_inner(w: &mut World) {
    let before = (w.level.alien_stage, w.level.human_stage);
    calculate_stages_rules(w);
    // the Stage log lines and level.*StageNTime
    let t = w.level.time;
    if w.level.alien_stage > before.0 {
        for s in before.0 + 1..=w.level.alien_stage {
            if s == 1 {
                w.level.alien_stage2_time = t;
            } else {
                w.level.alien_stage3_time = t;
            }
            super::log::log_printf(w, &format!("Stage: A {}: Aliens reached Stage {}\n", s + 1, s + 1));
        }
    }
    if w.level.human_stage > before.1 {
        for s in before.1 + 1..=w.level.human_stage {
            if s == 1 {
                w.level.human_stage2_time = t;
            } else {
                w.level.human_stage3_time = t;
            }
            super::log::log_printf(w, &format!("Stage: H {}: Humans reached Stage {}\n", s + 1, s + 1));
        }
    }
}

fn calculate_stages_rules(w: &mut World) {
    let l = &mut w.level;
    let amod = (l.avg_alien_clients / PLAYER_COUNT_MOD).max(0.1);
    let hmod = (l.avg_human_clients / PLAYER_COUNT_MOD).max(0.1);
    let (s1, s2) = (0, 1);
    if l.alien_credits >= threshold(l.alien_stage2_thresh, amod) && l.alien_stage == s1 && l.alien_max_stage > s1 {
        l.alien_stage = s2;
        l.stage_events.push((Team::Aliens, 2));
    }
    if l.alien_credits >= threshold(l.alien_stage3_thresh, amod) && l.alien_stage == s2 && l.alien_max_stage > s2 {
        l.alien_stage = 2;
        l.stage_events.push((Team::Aliens, 3));
    }
    if l.human_credits >= threshold(l.human_stage2_thresh, hmod) && l.human_stage == s1 && l.human_max_stage > s1 {
        l.human_stage = s2;
        l.stage_events.push((Team::Humans, 2));
    }
    if l.human_credits >= threshold(l.human_stage3_thresh, hmod) && l.human_stage == s2 && l.human_max_stage > s2 {
        l.human_stage = 2;
        l.stage_events.push((Team::Humans, 3));
    }
    let next = |stage_: i32, max: i32, t2: i32, t3: i32, m: f32| -> i32 {
        let n = if stage_ == 0 && max > 0 {
            threshold(t2, m)
        } else if stage_ == 1 && max > 1 {
            threshold(t3, m)
        } else {
            -1
        };
        if n > 0 { ((n as f32 / 100.0).ceil() * 100.0) as i32 } else { n }
    };
    l.alien_next_stage = next(l.alien_stage, l.alien_max_stage, l.alien_stage2_thresh, l.alien_stage3_thresh, amod);
    l.human_next_stage = next(l.human_stage, l.human_max_stage, l.human_stage2_thresh, l.human_stage3_thresh, hmod);
}

/// `G_CalculateAvgPlayers` system.
pub fn calculate_avg_players(w: &mut World) {
    if !warmup_over(w) {
        return;
    }
    let l = &mut w.level;
    if l.num_alien_clients == 0 {
        l.num_alien_samples = 0;
        l.alien_credits = 0;
    }
    if l.num_human_clients == 0 {
        l.num_human_samples = 0;
        l.human_credits = 0;
    }
    l.avg_alien_clients = (l.avg_alien_clients * l.num_alien_samples as f32 + l.num_alien_clients as f32) / (l.num_alien_samples + 1) as f32;
    l.num_alien_samples += 1;
    l.avg_human_clients = (l.avg_human_clients * l.num_human_samples as f32 + l.num_human_clients as f32) / (l.num_human_samples + 1) as f32;
    l.num_human_samples += 1;
}

/// `timeWarning_t`.
const TW_IMMINENT: i32 = 1;
const TW_PASSED: i32 = 2;

/// `CheckExitRules` system (+ LogExit / BeginIntermission state changes).
pub fn check_exit_rules(w: &mut World) {
    if w.level.exited {
        return;
    }
    if w.level.intermission_time != 0 {
        super::client::check_intermission_exit(w);
        return;
    }
    let l = &mut w.level;
    if l.intermission_queued != 0 {
        if l.time - l.intermission_queued >= INTERMISSION_DELAY_TIME {
            l.intermission_queued = 0;
            super::client::begin_intermission(w);
        }
        return;
    }
    if l.timelimit > 0 {
        let t = l.time;
        if l.time - l.start_time >= l.timelimit * 60000 {
            l.winner = Some(Team::None);
            l.last_win = Some(Team::None);
            super::log::log_exit(w, "Timelimit hit.");
            return;
        } else if l.time - l.start_time >= (l.timelimit - 5) * 60000 && l.timelimit_warning < TW_IMMINENT {
            l.timelimit_warning = TW_IMMINENT;
            w.center_prints.push((t, None, None, "5 minutes remaining!".into()));
        } else if l.time - l.start_time >= (l.timelimit - 1) * 60000 && l.timelimit_warning < TW_PASSED {
            l.timelimit_warning = TW_PASSED;
            w.center_prints.push((t, None, None, "1 minute remaining!".into()));
        }
    }
    let l = &mut w.level;
    let humans = l.uncond_human_win || (!l.uncond_alien_win && l.time > l.start_time + 1000 && l.num_alien_spawns == 0 && l.num_live_alien_clients == 0);
    let aliens = !humans && (l.uncond_alien_win || (l.time > l.start_time + 1000 && l.num_human_spawns == 0 && l.num_live_human_clients == 0));
    let won = if humans { Team::Humans } else if aliens { Team::Aliens } else { return };
    // LogExit: queue the intermission and fire the matching trigger_win
    l.winner = Some(won);
    l.last_win = Some(won);
    super::log::log_exit(w, if won == Team::Humans { "Humans win." } else { "Aliens win." });
}

/// `Team_GetLocation`: the nearest target_location in the player's PVS.
pub fn get_location(w: &World, i: usize) -> Option<usize> {
    let o = w.ents[i].origin;
    let mut best = None;
    let mut bestlen = 3.0f32 * 8192.0 * 8192.0;
    let mut e = w.level.location_head;
    while let Some(l) = e {
        let d = super::qmath::sub(&o, &w.ents[l].origin);
        let len = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if len <= bestlen && w.in_pvs(&o, &w.ents[l].origin) {
            bestlen = len;
            best = Some(l);
        }
        e = w.ents[l].next_train;
    }
    best
}

