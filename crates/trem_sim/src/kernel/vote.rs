//! Votes (g_cmds.c Cmd_CallVote_f / Cmd_Vote_f, g_main.c G_Vote /
//! G_CheckVote / G_ExecuteVote) and the server commands they run
//! (g_svcmds.c evacuation / admitdefeat / suddendeath, G_BaseSelfDestruct).
//!
//! kick / mute / unmute / denybuild / allowbuild act on the namelog through
//! the admin commands (dec026): like the C, the passed vote runs the console
//! line ("mute <namelog id>", "ban <id> 1s<g_adminTempBan> vote kick ...")
//! from `World::exec`. The host reloads the map for map / map_restart via
//! `level.exited` + `level.next_map`.

use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::enums::{MeansOfDeath as Mod, Team};

/// `VOTE_TIME`: 30 seconds before a vote times out.
pub const VOTE_TIME: i32 = 30000;

/// What a passed vote runs (`level.voteString`).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum VoteAction {
    #[default]
    None,
    MapRestart,
    Map(String),
    NextMap(String),
    Evacuation,
    SuddenDeath(i32),
    AdmitDefeat(Team),
    /// A console line (`level.voteString` of the namelog votes).
    Exec(String),
}

/// `level.vote*[ team ]`.
#[derive(Clone, Debug, Default)]
pub struct VoteState {
    pub time: i32,
    pub execute_time: i32,
    pub delay: i32,
    pub threshold: i32,
    pub yes: i32,
    pub no: i32,
    pub action: VoteAction,
    pub display: String,
    pub caller: String,
}

/// `level.voteString`: the console command a passed vote runs, as the C
/// words it (the CallVote log line).
pub fn vote_string(a: &VoteAction) -> String {
    match a {
        VoteAction::None => String::new(),
        VoteAction::MapRestart => "map_restart".into(),
        VoteAction::Map(m) => format!("map \"{m}\""),
        VoteAction::NextMap(m) => format!("set g_nextMap \"{m}\""),
        VoteAction::Evacuation => "evacuation".into(),
        VoteAction::SuddenDeath(d) => format!("suddendeath {d}"),
        VoteAction::AdmitDefeat(t) => format!("admitdefeat {}", *t as i32),
        VoteAction::Exec(s) => s.clone(),
    }
}

fn ti(t: Team) -> usize {
    t as usize
}

/// `level.numVotingClients[ team ]` (CalculateRanks).
pub fn num_voting_clients(w: &World, team: Team) -> i32 {
    w.clients.iter().filter(|c| c.connected && (team == Team::None || c.team == team)).count() as i32
}

fn broadcast(w: &mut World, team: Team, msg: &str) {
    for i in 0..MAX_CLIENTS {
        if w.clients[i].connected && (team == Team::None || w.clients[i].team == team) {
            super::client::notify(w, i, msg.to_string());
        }
    }
}

/// `G_Vote`: count (or uncount) a client's vote.
pub fn vote(w: &mut World, i: usize, team: Team, voting: bool) {
    let t = ti(team);
    if w.level.votes[t].time == 0 {
        return;
    }
    let bit = 1 << t;
    if voting && w.clients[i].voted & bit != 0 {
        return;
    }
    if !voting && w.clients[i].voted & bit == 0 {
        return;
    }
    w.clients[i].voted |= bit;
    let d = if voting { 1 } else { -1 };
    if w.clients[i].vote & bit != 0 {
        w.level.votes[t].yes += d;
    } else {
        w.level.votes[t].no += d;
    }
}

/// `G_ClearVotes` (BeginIntermission).
pub fn clear_votes(w: &mut World) {
    for v in w.level.votes.iter_mut() {
        *v = VoteState::default();
    }
    for c in w.clients.iter_mut() {
        c.voted = 0;
    }
}

/// `Cmd_CallVote_f` (callvote / callteamvote).
pub fn cmd_callvote(w: &mut World, i: usize, team_vote: bool, kind: &str, arg: &str, reason: &str) -> Result<(), String> {
    let cmd = if team_vote { "callteamvote" } else { "callvote" };
    let team = if team_vote { w.clients[i].team } else { Team::None };
    let t = ti(team);
    if !w.level.allow_vote {
        return Err(format!("{cmd}: voting not allowed here"));
    }
    if w.level.votes[t].time != 0 {
        return Err(format!("{cmd}: a vote is already in progress"));
    }
    if w.level.votes[t].execute_time != 0 {
        execute_vote(w, team);
    }
    let mut delay = 0;
    let mut threshold = 50;
    let kind = kind.to_lowercase();
    let nl = w.clients[i].namelog;
    if w.level.vote_limit > 0 && nl.is_some_and(|n| w.namelogs[n].vote_count >= w.level.vote_limit) && !super::admin::permission(w, Some(i), super::admin::ADMF_NO_VOTE_LIMIT) {
        return Err(format!("{cmd}: you have already called the maximum number of votes ({})", w.level.vote_limit));
    }
    let reason = super::cmds::decolor_string(reason);
    // kick, mute, unmute, denybuild, allowbuild
    let mut target = None;
    if matches!(kind.as_str(), "kick" | "mute" | "unmute" | "denybuild" | "allowbuild") {
        if arg.is_empty() {
            return Err(format!("{cmd}: no target"));
        }
        let c = super::client::client_number_from_string(w, arg).map_err(|e| format!("{cmd}: {e}"))?;
        if matches!(kind.as_str(), "kick" | "mute" | "denybuild") {
            if super::admin::permission(w, Some(c), super::admin::ADMF_IMMUNITY) {
                let msg = format!("^7{}^3 attempted {cmd} {kind} on immune admin ^7{}^3 for: {}", w.clients[i].name, w.clients[c].name, if reason.is_empty() { "no reason" } else { &reason });
                super::cmds::admin_message(w, None, &msg);
                return Err(format!("{cmd}: admin is immune"));
            }
            if team != Team::None && w.clients[i].team != w.clients[c].team {
                return Err(format!("{cmd}: player is not on your team"));
            }
            if reason.is_empty() && !super::admin::permission(w, Some(i), super::admin::ADMF_UNACCOUNTABLE) {
                return Err(format!("{cmd}: You must provide a reason"));
            }
        }
        let id = w.clients[c].namelog.map(|n| w.namelogs[n].id).unwrap_or(-1);
        target = Some((c, super::cmds::decolor_string(&w.clients[c].name), id));
    }
    let for_reason = if reason.is_empty() { String::new() } else { format!(" for '{reason}'") };
    let map_exists = |w: &World, m: &str| w.level.map_list.iter().any(|x| x.eq_ignore_ascii_case(m));
    let (action, display) = match (team, kind.as_str()) {
        (_, "kick") => {
            let (c, name, id) = target.unwrap();
            if w.clients[c].local_client {
                return Err(format!("{cmd}: admin is immune"));
            }
            // "ban <ip> 1s<g_adminTempBan> vote kick (reason)": the namelog id stands in for the IP
            (VoteAction::Exec(format!("ban {id} \"1s{}\" vote kick ({reason})", w.level.admin_temp_ban)), format!("Kick player '{name}'{for_reason}"))
        }
        (Team::None, "mute") => {
            let (c, name, id) = target.unwrap();
            if w.clients[c].namelog.is_some_and(|n| w.namelogs[n].muted) {
                return Err(format!("{cmd}: player is already muted"));
            }
            (VoteAction::Exec(format!("mute {id}")), format!("Mute player '{name}'{for_reason}"))
        }
        (Team::None, "unmute") => {
            let (c, name, id) = target.unwrap();
            if !w.clients[c].namelog.is_some_and(|n| w.namelogs[n].muted) {
                return Err(format!("{cmd}: player is not currently muted"));
            }
            (VoteAction::Exec(format!("unmute {id}")), format!("Unmute player '{name}'"))
        }
        (Team::None, "map_restart") => (VoteAction::MapRestart, "Restart current map".to_string()),
        (Team::None, "map") => {
            if !map_exists(w, arg) {
                return Err(format!("{cmd}: 'maps/{arg}.bsp' could not be found on the server"));
            }
            delay = 3000;
            (VoteAction::Map(arg.to_string()), format!("Change to map '{arg}'"))
        }
        (Team::None, "nextmap") => {
            if let Some(m) = w.level.next_map.clone() {
                return Err(format!("{cmd}: the next map is already set to '{m}'"));
            }
            if !map_exists(w, arg) {
                return Err(format!("{cmd}: 'maps/{arg}.bsp' could not be found on the server"));
            }
            (VoteAction::NextMap(arg.to_string()), format!("Set the next map to '{arg}'"))
        }
        (Team::None, "draw") => {
            delay = 3000;
            (VoteAction::Evacuation, "End match in a draw".to_string())
        }
        (Team::None, "sudden_death") => {
            if w.level.sudden_death_vote_percent == 0 {
                return Err("Sudden Death votes have been disabled".into());
            }
            let tsd = super::buildable::time_til_sudden_death(w);
            if tsd <= 0 {
                return Err("callvote: Sudden Death has already begun".into());
            }
            if w.level.sudden_death_begin_time > 0 && tsd <= w.level.sudden_death_vote_delay * 1000 {
                return Err("callvote: Sudden Death is imminent".into());
            }
            threshold = w.level.sudden_death_vote_percent;
            let d = w.level.sudden_death_vote_delay;
            (VoteAction::SuddenDeath(d), format!("Begin sudden death in {d} seconds"))
        }
        (Team::None, _) => return Err("Invalid vote string. Valid vote commands are: map, nextmap, map_restart, draw, sudden_death, kick, mute and unmute".into()),
        (_, "denybuild") => {
            let (c, name, id) = target.unwrap();
            if w.clients[c].namelog.is_some_and(|n| w.namelogs[n].deny_build) {
                return Err(format!("{cmd}: player already lost building rights"));
            }
            (VoteAction::Exec(format!("denybuild {id}")), format!("Take away building rights from '{name}'{for_reason}"))
        }
        (_, "allowbuild") => {
            let (c, name, id) = target.unwrap();
            if !w.clients[c].namelog.is_some_and(|n| w.namelogs[n].deny_build) {
                return Err(format!("{cmd}: player already has building rights"));
            }
            (VoteAction::Exec(format!("allowbuild {id}")), format!("Allow '{name}' to build"))
        }
        (_, "admitdefeat") => {
            delay = 3000;
            (VoteAction::AdmitDefeat(team), "Admit Defeat".to_string())
        }
        _ => return Err("Invalid vote string. Valid team vote commands are: kick, denybuild, allowbuild and admitdefeat".into()),
    };
    let name = w.clients[i].name.clone();
    let now = w.level.time;
    let line = format!("{}: {} \"{}^7\": {}\n", if team == Team::None { "CallVote" } else { "CallTeamVote" }, i, name, vote_string(&action));
    super::log::log_printf(w, &line);
    let v = &mut w.level.votes[t];
    v.delay = delay;
    v.threshold = threshold;
    v.action = action;
    v.display = display.clone();
    v.caller = name.clone();
    v.time = now;
    let msg = if team == Team::None { format!("{name} called a vote: {display}") } else { format!("{name} called a team vote: {display}") };
    broadcast(w, team, &msg);
    if let Some(n) = nl {
        w.namelogs[n].vote_count += 1;
    }
    w.clients[i].vote |= 1 << t;
    vote(w, i, team, true);
    Ok(())
}

/// `Cmd_Vote_f` (vote / teamvote yes|no).
pub fn cmd_vote(w: &mut World, i: usize, team_vote: bool, choice: &str) -> Result<(), String> {
    let cmd = if team_vote { "teamvote" } else { "vote" };
    let team = if team_vote { w.clients[i].team } else { Team::None };
    let t = ti(team);
    if w.level.votes[t].time == 0 {
        return Err(format!("{cmd}: no vote in progress"));
    }
    if w.clients[i].voted & (1 << t) != 0 {
        return Err(format!("{cmd}: vote already cast"));
    }
    super::client::notify(w, i, format!("{cmd}: vote cast"));
    if choice.starts_with('y') {
        w.clients[i].vote |= 1 << t;
    } else {
        w.clients[i].vote &= !(1 << t);
    }
    vote(w, i, team, true);
    Ok(())
}

/// `G_ExecuteVote`.
pub fn execute_vote(w: &mut World, team: Team) {
    let t = ti(team);
    w.level.votes[t].execute_time = 0;
    let action = w.level.votes[t].action.clone();
    match action {
        VoteAction::None => {}
        VoteAction::MapRestart => {
            w.level.next_map = None;
            w.level.exited = true;
        }
        VoteAction::Map(m) => {
            w.level.next_map = Some(m);
            w.level.exited = true;
        }
        VoteAction::NextMap(m) => w.level.next_map = Some(m),
        VoteAction::Evacuation => evacuation(w),
        VoteAction::SuddenDeath(secs) => sudden_death(w, secs),
        VoteAction::AdmitDefeat(tm) => admit_defeat(w, tm),
        VoteAction::Exec(line) => w.exec.push(line),
    }
}

/// `G_CheckVote`.
pub fn check_vote(w: &mut World, team: Team) {
    let t = ti(team);
    let now = w.level.time;
    if w.level.votes[t].execute_time != 0 && w.level.votes[t].execute_time < now {
        execute_vote(w, team);
    }
    if w.level.votes[t].time == 0 {
        return;
    }
    let voting = num_voting_clients(w, team);
    let v = &w.level.votes[t];
    let thr = v.threshold as f32 / 100.0;
    let pass;
    if now - v.time >= VOTE_TIME || v.yes + v.no == voting {
        pass = v.yes != 0 && v.yes as f32 / (v.yes as f32 + v.no as f32) > thr;
    } else if v.yes as f32 > voting as f32 * thr {
        pass = true;
    } else if v.no as f32 <= voting as f32 * (1.0 - thr) {
        return;
    } else {
        pass = false;
    }
    let (yes, no, delay) = (v.yes, v.no, v.delay);
    if pass {
        w.level.votes[t].execute_time = now + delay;
    }
    let line = format!("EndVote: {} {} {} {} {}\n", if team == Team::None { "global" } else { super::log::team_name(team) }, if pass { "pass" } else { "fail" }, yes, no, voting);
    super::log::log_printf(w, &line);
    let msg = format!("{}ote {}ed ({} - {})", if team == Team::None { "V" } else { "Team v" }, if pass { "pass" } else { "fail" }, yes, no);
    broadcast(w, team, &msg);
    let v = &mut w.level.votes[t];
    v.time = 0;
    v.yes = 0;
    v.no = 0;
    for c in w.clients.iter_mut() {
        c.voted &= !(1 << t);
    }
}

/// System: G_CheckVote for every team (G_RunFrame).
pub fn check_votes(w: &mut World) {
    for team in [Team::None, Team::Aliens, Team::Humans] {
        check_vote(w, team);
    }
}

/// `Svcmd_Evacuation_f`: end in a draw.
pub fn evacuation(w: &mut World) {
    let t = w.level.time;
    w.center_prints.push((t, None, None, "Evacuation ordered".into()));
    w.level.last_win = Some(Team::None);
    w.level.winner = Some(Team::None);
    super::log::log_exit(w, "Evacuation.");
}

/// `Svcmd_SuddenDeath_f`.
pub fn sudden_death(w: &mut World, secs: i32) {
    w.level.sudden_death_begin_time = w.level.time - w.level.start_time + secs * 1000;
    let t = w.level.time;
    w.center_prints.push((t, None, None, format!("Sudden Death will begin in {secs} second{}", if secs == 1 { "" } else { "s" })));
}

/// `Svcmd_AdmitDefeat_f`.
pub fn admit_defeat(w: &mut World, team: Team) {
    let t = w.level.time;
    match team {
        Team::Aliens => {
            w.center_prints.push((t, Some(Team::Aliens), None, "Hivemind Link Broken".into()));
            w.prints.push((t, None, None, "Alien team has admitted defeat".into()));
        }
        Team::Humans => {
            w.center_prints.push((t, Some(Team::Humans), None, "Life Support Terminated".into()));
            w.prints.push((t, None, None, "Human team has admitted defeat".into()));
        }
        Team::None => return,
    }
    w.level.surrender_team = Some(team);
    base_self_destruct(w, team);
}

/// `G_BaseSelfDestruct`.
pub fn base_self_destruct(w: &mut World, team: Team) {
    for i in MAX_CLIENTS..w.num_entities {
        let e = &w.ents[i];
        if e.health <= 0 || e.etype != EType::Buildable || e.team != team {
            continue;
        }
        super::combat::damage(w, i, None, None, None, None, 10000, 0, Mod::Suicide);
    }
}
