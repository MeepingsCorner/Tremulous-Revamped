//! Kernel `session` — game/g_session.c: the client data that survives a map
//! change. The original keeps it in "session<N>" cvars; here the host keeps
//! the per-slot strings across World reloads (like the map rotation state,
//! dec022/dec026) and hands them back when the slot reconnects.
//!
//! Only `restartTeam` (admin `restart keepteams|switchteams`), the ignore
//! list and `spectatorTime` matter after a map change: ClientBegin's
//! ClientSpawn resets the spectator state and followed client anyway.

use super::world::{World, MAX_CLIENTS};
use crate::gen::enums::{SpectatorState, Team};

/// `clientSession_t`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientSession {
    pub spectator_time: i32,
    pub spectator_state: i32,
    pub spectator_client: i32,
    pub restart_team: Team,
    pub ignore_list: Vec<usize>,
}

/// `Com_ClientListString`: the 64-bit client mask as 16 hex digits.
fn client_list_string(list: &[usize]) -> String {
    let mut hi = 0u32;
    let mut lo = 0u32;
    for &c in list.iter().filter(|&&c| c < MAX_CLIENTS) {
        if c < 32 {
            lo |= 1 << c;
        } else {
            hi |= 1 << (c - 32);
        }
    }
    format!("{hi:08x}{lo:08x}")
}

/// `Com_ClientListParse`.
fn client_list_parse(s: &str) -> Vec<usize> {
    let s = s.get(..16).unwrap_or(s);
    let (h, l) = s.split_at(s.len().min(8));
    let hi = u32::from_str_radix(h, 16).unwrap_or(0);
    let lo = u32::from_str_radix(l, 16).unwrap_or(0);
    (0..MAX_CLIENTS).filter(|&c| if c < 32 { lo & (1 << c) != 0 } else { hi & (1 << (c - 32)) != 0 }).collect()
}

fn team_num(t: Team) -> i32 {
    t as i32
}

impl ClientSession {
    /// The "session<N>" cvar text: "%i %i %i %i %s".
    pub fn to_cvar(&self) -> String {
        format!("{} {} {} {} {}", self.spectator_time, self.spectator_state, self.spectator_client, team_num(self.restart_team), client_list_string(&self.ignore_list))
    }
    /// sscanf( "%i %i %i %i %16s" ) of G_ReadSessionData.
    pub fn from_cvar(s: &str) -> Self {
        let mut it = s.split_ascii_whitespace();
        let mut n = || super::spawn::c_atoi(it.next().unwrap_or("0"));
        let spectator_time = n();
        let spectator_state = n();
        let spectator_client = n();
        let restart_team = match n() {
            1 => Team::Aliens,
            2 => Team::Humans,
            _ => Team::None,
        };
        let ignore_list = client_list_parse(s.split_ascii_whitespace().nth(4).unwrap_or(""));
        ClientSession { spectator_time, spectator_state, spectator_client, restart_team, ignore_list }
    }
}

/// `G_WriteClientSessionData`.
pub fn write_client_session_data(w: &World, i: usize) -> String {
    let c = &w.clients[i];
    ClientSession { spectator_time: c.spectator_time, spectator_state: c.spectator, spectator_client: c.spectator_client, restart_team: c.restart_team, ignore_list: c.ignored.clone() }.to_cvar()
}

/// `G_WriteSessionData`: "session<N>" for every connected slot (None: empty).
pub fn write_session_data(w: &World) -> Vec<Option<String>> {
    (0..MAX_CLIENTS).map(|i| w.clients[i].connected.then(|| write_client_session_data(w, i))).collect()
}

/// `G_InitSessionData`: a first-time connect.
pub fn init_session_data(w: &mut World, i: usize) {
    let t = w.level.time;
    let c = &mut w.clients[i];
    c.restart_team = Team::None;
    c.spectator = SpectatorState::Free as i32;
    c.spectator_time = t;
    c.spectator_client = -1;
    c.ignored.clear();
}

/// `G_ReadSessionData` for a slot carried over from the previous map, then
/// ClientBegin's restartTeam (`!restart keepteams` / `switchteams`).
pub fn read_session_data(w: &mut World, i: usize, cvar: &str) {
    let s = ClientSession::from_cvar(cvar);
    let c = &mut w.clients[i];
    c.spectator_time = s.spectator_time;
    c.restart_team = s.restart_team;
    c.ignored = s.ignore_list.into_iter().filter(|&x| x != i).collect();
    // ClientSpawn: "if client is dead and following teammate, stop following"
    c.spectator_client = -1;
    client_begin(w, i);
}

/// The restartTeam part of `ClientBegin`.
pub fn client_begin(w: &mut World, i: usize) {
    let rt = w.clients[i].restart_team;
    if rt != Team::None {
        super::client::change_team(w, i, rt);
        w.clients[i].restart_team = Team::None;
    }
}

/// G_InitGame's g_lockTeamsAtStart: lock both teams once, then clear the
/// cvar (pushed to the console so the host forgets it too).
pub fn init_game_locks(w: &mut World) {
    if w.level.lock_teams_at_start {
        w.level.alien_team_locked = true;
        w.level.human_team_locked = true;
        w.level.lock_teams_at_start = false;
        w.exec.push("set g_lockTeamsAtStart \"0\"".into());
    }
}
