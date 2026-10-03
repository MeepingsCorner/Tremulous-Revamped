//! Kernel `log` — the server log file (g_main.c `G_LogPrintf`,
//! `G_SendGameStat`, `LogExit`; g_admin.c `admin_log_start` / `admin_log` /
//! `admin_log_abort` / `admin_log_end`, `G_admin_authlog`; g_cmds.c
//! `ConcatArgsPrintable`). `g_logFile` (default games.log) is opened for
//! appending through the host's file system (`world::Fs::append`) when the
//! game starts; `g_logFileSync 1` writes every line at once, otherwise lines
//! are buffered and written at the end of each frame and at shutdown (the
//! FS_APPEND / FS_APPEND_SYNC difference). Lines carry the `%3i:%i%i` game
//! clock, colour codes are stripped (`G_DecolorString`), and with `dedicated`
//! set they are echoed to the server console (`G_UnEscapeString`).
//!
//! `G_SendGameStat` builds the same statistics line the C sends to the
//! master server; there is no master server in a single process, so the
//! line goes to the log as `GameStat: ...` (dec031).

use super::cmds::decolor_string;
use super::world::{World, MAX_CLIENTS};
use crate::gen::enums::{MeansOfDeath, Pers, Team};

/// `g_logFile` default.
pub const DEFAULT_LOG_FILE: &str = "games.log";
/// `g_tag` default.
pub const DEFAULT_TAG: &str = "main";
/// `Q3_VERSION` (q_shared.h: PRODUCT_NAME " " PRODUCT_VERSION).
pub const Q3_VERSION: &str = "tremulous 1.1.0";
/// `MAX_STRING_CHARS` / `BIG_INFO_STRING`.
const MAX_STRING_CHARS: usize = 1024;
const BIG_INFO_STRING: usize = 8192;

/// Truncate to fit a C buffer of `size` bytes (at a char boundary).
fn fit(s: &mut String, size: usize) {
    if s.len() > size - 1 {
        let mut k = size - 1;
        while !s.is_char_boundary(k) {
            k -= 1;
        }
        s.truncate(k);
    }
}

/// `G_UnEscapeString`: control characters other than newline removed.
pub fn unescape_string(s: &str) -> String {
    s.chars().filter(|&c| c >= ' ' || c == '\n').collect()
}

/// The `%3i:%i%i ` game clock `G_LogPrintf` prefixes.
pub fn log_time(level_time: i32, start_time: i32) -> String {
    let mut sec = (level_time - start_time) / 1000;
    let min = sec / 60;
    sec -= min * 60;
    let tens = sec / 10;
    sec -= tens * 10;
    format!("{min:3}:{tens}{sec} ")
}

/// `G_LogPrintf`: to the log file with a time stamp if it is open, and to
/// the server console (dedicated servers).
pub fn log_printf(w: &mut World, text: &str) {
    let mut string = log_time(w.level.time, w.level.start_time);
    string.push_str(text);
    fit(&mut string, MAX_STRING_CHARS);
    if w.level.dedicated {
        let un = unescape_string(&string);
        w.console.push_str(un.get(7..).unwrap_or(""));
    }
    write_log(w, &string);
}

/// The file half of `G_LogPrintf` for lines the port already echoes to the
/// server console on its own (dec026: the host console is a dedicated
/// console for those).
pub fn log_file_printf(w: &mut World, text: &str) {
    let mut string = log_time(w.level.time, w.level.start_time);
    string.push_str(text);
    fit(&mut string, MAX_STRING_CHARS);
    write_log(w, &string);
}

fn write_log(w: &mut World, string: &str) {
    if !w.level.log_open {
        return;
    }
    let decolored = decolor_string(string);
    w.level.log_buffer.push_str(&decolored);
    if w.level.log_sync {
        flush(w);
    }
}

/// Write what is buffered (trap_FS_Write on the FS_APPEND handle).
pub fn flush(w: &mut World) {
    if w.level.log_buffer.is_empty() {
        return;
    }
    let text = std::mem::take(&mut w.level.log_buffer);
    let path = w.level.log_file.clone();
    let ok = w.fs.as_mut().is_some_and(|fs| fs.append(&path, &text));
    if !ok {
        w.level.log_open = false;
    }
}

/// `log_flush` system: the buffered lines of this frame.
pub fn flush_frame(w: &mut World) {
    flush(w);
}

/// The `serverinfo` string of the InitGame line (trap_GetServerinfo: the
/// CVAR_SERVERINFO cvars this port models).
pub fn server_info(w: &World) -> String {
    let l = &w.level;
    format!(
        "\\version\\{}\\g_alienStage\\{}\\g_humanStage\\{}\\timelimit\\{}\\g_suddenDeathTime\\{}\\sv_cheats\\{}\\mapname\\{}",
        Q3_VERSION,
        l.alien_stage,
        l.human_stage,
        l.timelimit,
        l.sudden_death_time,
        l.cheats as i32,
        w.map_name
    )
}

/// The log half of `G_InitGame`: open `g_logFile` for appending and write
/// the InitGame / RealTime lines.
pub fn init_game(w: &mut World) {
    open_log(w);
    // G_SpawnEntitiesFromString -> SP_worldspawn's warmup, after the InitGame line
    super::team::init_warmup(w);
    flush(w);
}

/// The G_InitGame log opening: "InitGame:" and "RealTime:".
fn open_log(w: &mut World) {
    if w.level.log_file.is_empty() {
        w.console.push_str("Not logging to disk\n");
        return;
    }
    // FS_APPEND(_SYNC): the file can be appended to
    let path = w.level.log_file.clone();
    let ok = w.fs.as_mut().is_some_and(|fs| fs.append(&path, ""));
    w.level.log_open = ok;
    if !ok {
        w.console.push_str(&format!("WARNING: Couldn't open logfile: {path}\n"));
        return;
    }
    log_printf(w, "------------------------------------------------------------\n");
    let info = server_info(w);
    log_printf(w, &format!("InitGame: {info}\n"));
    let (y, mo, d, h, mi, s) = real_time(w.real_time);
    log_printf(w, &format!("RealTime: {y:04}/{mo:02}/{d:02} {h:02}:{mi:02}:{s:02}\n"));
    flush(w);
}

/// The log half of `G_ShutdownGame`.
pub fn shutdown_game(w: &mut World) {
    if w.level.log_open {
        log_printf(w, "ShutdownGame:\n");
        log_printf(w, "------------------------------------------------------------\n");
        flush(w);
        w.level.log_open = false;
    }
}

/// trap_RealTime as (year, month, day, hour, minute, second), UTC.
pub fn real_time(secs: i32) -> (i64, u32, u32, u32, u32, u32) {
    let secs = secs.max(0) as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
}

/// `modNames[]` (g_combat.c): "MOD_" + the means of death in upper snake case.
pub fn mod_name(m: i32) -> String {
    match MeansOfDeath::from_raw(m.clamp(0, 255) as u8) {
        Some(x) if m >= 0 => {
            let name = format!("{x:?}");
            let mut out = String::from("MOD_");
            let b = name.as_bytes();
            for (i, &c) in b.iter().enumerate() {
                if i > 0 && c.is_ascii_uppercase() && (b[i - 1].is_ascii_lowercase() || b[i - 1].is_ascii_digit()) {
                    out.push('_');
                }
                out.push(c.to_ascii_uppercase() as char);
            }
            out
        }
        // fall back on the number
        _ => m.to_string(),
    }
}

/// `BG_TeamName`.
pub fn team_name(t: Team) -> &'static str {
    super::admin::team_name(t)
}

/// `ConcatArgsPrintable`: the arguments from `start`, quoting those with spaces.
pub fn concat_args_printable(a: &[&str], start: usize) -> String {
    let mut line = String::new();
    let c = a.len();
    for i in start..c {
        let arg = a[i];
        let print_arg = if arg.contains(' ') { format!("\"{arg}\"") } else { arg.to_string() };
        if line.len() + print_arg.len() >= MAX_STRING_CHARS - 1 {
            break;
        }
        line.push_str(&print_arg);
        if line.len() == MAX_STRING_CHARS - 1 {
            break;
        }
        if i != c - 1 {
            line.push(' ');
        }
    }
    line
}

/// `admin_log_start`.
pub fn admin_log_start(w: &mut World, admin: Option<usize>, cmd: &str) {
    let name = admin.map(|i| w.clients[i].name.clone()).unwrap_or_else(|| "console".into());
    let a = admin.and_then(|i| w.clients[i].admin).and_then(|x| w.admin.admins.get(x));
    let mut s = format!(
        "{} \"{}^7\" \"{}^7\" [{}] ({}): {}",
        admin.map(|i| i as i32).unwrap_or(-1),
        name,
        a.map(|x| x.name.clone()).unwrap_or_else(|| name.clone()),
        a.map(|x| x.level).unwrap_or(0),
        admin.map(|i| w.clients[i].guid.clone()).unwrap_or_default(),
        cmd
    );
    fit(&mut s, MAX_STRING_CHARS);
    w.level.admin_log = s;
}

/// `admin_log`: one more ": str" on the line being built.
pub fn admin_log(w: &mut World, s: &str) {
    if !w.level.admin_log.is_empty() {
        let mut l = std::mem::take(&mut w.level.admin_log);
        l.push_str(&format!(": {s}"));
        fit(&mut l, MAX_STRING_CHARS);
        w.level.admin_log = l;
    }
}

/// `admin_log_abort`.
pub fn admin_log_abort(w: &mut World) {
    w.level.admin_log.clear();
}

/// `admin_log_end`: the AdminExec line.
pub fn admin_log_end(w: &mut World, ok: bool) {
    if !w.level.admin_log.is_empty() {
        let l = w.level.admin_log.clone();
        log_printf(w, &format!("AdminExec: {}: {}\n", if ok { "ok" } else { "fail" }, l));
    }
    admin_log_abort(w);
}

/// `G_admin_authlog`: an admin was recognised (connect / setlevel / readconfig).
pub fn admin_authlog(w: &mut World, i: usize) {
    let Some(a) = w.clients[i].admin.and_then(|x| w.admin.admins.get(x)).cloned() else { return };
    let level = super::admin::level_def(w, a.level).map(|l| l.flags.clone()).unwrap_or_default();
    let aflags = format!("{} {}", a.flags, level);
    let line = format!("AdminAuth: {} \"{}^7\" \"{}^7\" [{}] ({}): {}\n", i, w.clients[i].name, a.name, a.level, w.clients[i].guid, aflags);
    log_printf(w, &line);
}

/// `level.sortedClients` (`CalculateRanks`: the connected clients sorted by
/// score with `SortRanks`).
pub fn sorted_clients(w: &World) -> Vec<usize> {
    let mut v: Vec<usize> = (0..MAX_CLIENTS).filter(|&i| w.clients[i].connected).collect();
    v.sort_by(|&a, &b| w.clients[b].ps.persistant[Pers::Score as usize].cmp(&w.clients[a].ps.persistant[Pers::Score as usize]));
    v
}

/// `G_SendGameStat`: the statistics line of a finished game.
pub fn game_stat(w: &World, team: Team) -> Option<String> {
    // games with cheats enabled are not very good for balance statistics
    if w.level.cheats {
        return None;
    }
    let l = &w.level;
    let team_char = match team {
        Team::Aliens => 'A',
        Team::Humans => 'H',
        Team::None => 'L',
    };
    let sorted = sorted_clients(w);
    let mut data = format!(
        "{} {} T:{} A:{:.6} H:{:.6} M:{} D:{} SD:{} AS:{} AS2T:{} AS3T:{} HS:{} HS2T:{} HS3T:{} CL:{}",
        Q3_VERSION,
        l.tag,
        team_char,
        l.avg_alien_clients,
        l.avg_human_clients,
        w.map_name,
        l.time - l.start_time,
        super::buildable::time_til_sudden_death(w),
        l.alien_stage,
        l.alien_stage2_time - l.start_time,
        l.alien_stage3_time - l.start_time,
        l.human_stage,
        l.human_stage2_time - l.start_time,
        l.human_stage3_time - l.start_time,
        sorted.len()
    );
    fit(&mut data, BIG_INFO_STRING);
    for &i in &sorted {
        let cl = &w.clients[i];
        // ps.ping is always 0 in one process; nobody is still connecting
        let ping = cl.ps.ping.min(999);
        let tc = match cl.team {
            Team::Aliens => 'A',
            Team::Humans => 'H',
            Team::None => 'S',
        };
        let mut entry = format!(" \"{}\" {} {} {} {}", cl.name, tc, cl.ps.persistant[Pers::Score as usize], ping, (l.time - cl.enter_time) / 60000);
        fit(&mut entry, MAX_STRING_CHARS);
        if data.len() + entry.len() >= BIG_INFO_STRING {
            break;
        }
        data.push_str(&entry);
    }
    Some(data)
}

/// `G_SendGameStat` (trap_SendGameStat goes to the log, see the module doc).
pub fn send_game_stat(w: &mut World, team: Team) {
    if let Some(data) = game_stat(w, team) {
        log_printf(w, &format!("GameStat: {data}\n"));
    }
}

/// `LogExit`: the Exit line, queue the intermission, the score lines, fire
/// the winning team's trigger_win and send the game statistics.
pub fn log_exit(w: &mut World, string: &str) {
    log_printf(w, &format!("Exit: {string}\n"));
    w.level.intermission_queued = w.level.time;
    // don't send more than 32 scores (FIXME?)
    let sorted: Vec<usize> = sorted_clients(w).into_iter().take(32).collect();
    for i in sorted {
        let cl = &w.clients[i];
        if cl.team == Team::None {
            continue;
        }
        let ping = cl.ps.ping.min(999);
        let line = format!("score: {}  ping: {}  client: {} {}\n", cl.ps.persistant[Pers::Score as usize], ping, i, cl.name);
        log_printf(w, &line);
    }
    let won = w.level.last_win.unwrap_or(Team::None);
    super::trigger::fire_trigger_win(w, won);
    send_game_stat(w, won);
}
