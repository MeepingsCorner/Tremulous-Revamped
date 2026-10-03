//! Kernel `admin` — game/g_admin.c (the shrubbot admin system) and
//! game/g_namelog.c, in a single process (dec026).
//!
//! * admin.dat is the original text format; the host reads and writes it
//!   through `World::fs` (user://admin.dat), the kernel only sees text.
//! * Networked clients have their cl_guid and their address (pers.ip, from
//!   the server's "ip" userinfo, dec033): bans match the GUID or the
//!   IP/CIDR, and ban/showbans/namelog take the IP forms like the C. The
//!   host's player gets `LOCAL_GUID` and bots `BOT_<name>`; neither has an
//!   address, so banning one by name makes a single GUID ban (the C would
//!   create no ban at all for an address-less player).
//! * The host's player is the top admin level by default: on connect, if
//!   admin.dat has no [admin] for its GUID, one is added at the highest
//!   defined level (normally 5 "Server Operator": ALLFLAGS).
//! * The command table `g_admin_cmds[]` is the `admin_commands` sheet; the
//!   commands run from the client command path (`ClientCommand` falls back
//!   to `G_admin_cmd_check`), from chat lines starting with '!', and from the
//!   server console (`ConsoleCommand`).
//! * Output: `G_admin_print` (ADMP) goes to the client's print queue or the
//!   server console buffer (`World::console`); AP() is a print to everyone.
//! * The admin log lines (`admin_log*`, `G_admin_authlog`) are G_LogPrintf
//!   output and are dropped with the game log (dec017).

use super::cmds::{concat, decolor_string, is_color_string, print_strlen, sanitise_string};
use super::maprotation::Lexer;
use super::world::{World, MAX_CLIENTS};
use crate::gen::enums::{SpectatorState, Stat, Team};

pub const ADMF_IMMUNITY: &str = "IMMUNITY";
pub const ADMF_NOCENSORFLOOD: &str = "NOCENSORFLOOD";
pub const ADMF_SPEC_ALLCHAT: &str = "SPECALLCHAT";
pub const ADMF_FORCETEAMCHANGE: &str = "FORCETEAMCHANGE";
pub const ADMF_UNACCOUNTABLE: &str = "UNACCOUNTABLE";
pub const ADMF_NO_VOTE_LIMIT: &str = "NOVOTELIMIT";
pub const ADMF_CAN_PERM_BAN: &str = "CANPERMBAN";
pub const ADMF_CAN_IP_BAN: &str = "CANIPBAN";
pub const ADMF_ACTIVITY: &str = "ACTIVITY";
pub const ADMF_IMMUTABLE: &str = "IMMUTABLE";
pub const ADMF_INCOGNITO: &str = "INCOGNITO";
pub const ADMF_ALLFLAGS: &str = "ALLFLAGS";
pub const ADMF_ADMINCHAT: &str = "ADMINCHAT";

pub const MAX_ADMIN_LISTITEMS: i32 = 20;
pub const MAX_ADMIN_SHOWBANS: i32 = 10;
pub const MAX_ADMIN_CMD_LEN: usize = 20;
pub const MAX_ADMIN_BAN_REASON: usize = 50;
pub const MAX_NAMELOG_NAMES: usize = 5;
pub const MAX_NAMELOG_ADDRS: usize = 5;
const MAX_NAME_LENGTH: usize = super::cmds::MAX_NAME_LENGTH;

/// pers.guid of the host's own player (pers.localClient).
pub const LOCAL_GUID: &str = "LOCALHOST";

/// `addr_t` (g_local.h): a parsed IPv4 / IPv6 address with a CIDR mask;
/// `str` is the text it was parsed from (pers.ip.str).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Addr {
    pub ipv6: bool,
    pub addr: [u8; 16],
    pub mask: i32,
    pub str: String,
}

/// `addr4parse`: the rest of the string after the address, None on error.
fn addr4parse<'a>(s: &'a [u8], addr: &mut Addr) -> Option<&'a [u8]> {
    *addr = Addr::default();
    let mut octet = 0;
    let mut num: i32 = 0;
    let mut i = 0;
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    while octet < 4 {
        if at(i).is_ascii_digit() {
            num = num.saturating_mul(10).saturating_add((at(i) - b'0') as i32);
        } else {
            if !(0..=255).contains(&num) {
                return None;
            }
            addr.addr[octet] = num as u8;
            octet += 1;
            if at(i) != b'.' || at(i + 1) == b'.' {
                break;
            }
            num = 0;
        }
        i += 1;
    }
    if octet < 1 {
        return None;
    }
    Some(&s[i.min(s.len())..])
}

/// `addr6parse`.
fn addr6parse<'a>(s: &'a [u8], addr: &mut Addr) -> Option<&'a [u8]> {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut seen = false;
    let (mut a, mut b) = ([0u8; 16], [0u8; 16]);
    let (mut before, mut after) = (0usize, 0usize);
    let mut num: i32 = 0;
    let mut i = 0usize;
    // 8 hexadectets unless :: is present
    while before + after <= 8 {
        let c = at(i);
        if c.is_ascii_hexdigit() {
            num = num.saturating_mul(16).saturating_add((c as char).to_digit(16).unwrap() as i32);
        } else {
            if !(0..=65535).contains(&num) {
                return None;
            }
            if i == 0 {
            } else if seen {
                // :: has been seen already
                if after < 8 {
                    b[after * 2] = (num >> 8) as u8;
                    b[after * 2 + 1] = (num & 0xff) as u8;
                }
                after += 1;
            } else {
                if before < 8 {
                    a[before * 2] = (num >> 8) as u8;
                    a[before * 2 + 1] = (num & 0xff) as u8;
                }
                before += 1;
            }
            if c == 0 {
                break;
            }
            if c != b':' || before + after == 8 {
                break;
            }
            if at(i + 1) == b':' {
                // ::: or multiple ::
                if seen || at(i + 2) == b':' {
                    break;
                }
                seen = true;
                i += 1;
            } else if i == 0 {
                // starts with : but not ::
                return None;
            }
            num = 0;
        }
        i += 1;
    }
    if seen {
        // there have to be fewer than 8 hexadectets when :: is present
        if before + after >= 8 {
            return None;
        }
    } else if before + after < 8 {
        // require exactly 8 hexadectets
        return None;
    }
    *addr = Addr { ipv6: true, ..Default::default() };
    addr.addr[..before * 2].copy_from_slice(&a[..before * 2]);
    addr.addr[16 - 2 * after..].copy_from_slice(&b[..after * 2]);
    Some(&s[i.min(s.len())..])
}

/// `G_AddressParse`: (parsed, address). The address keeps `str` even when
/// the parse fails, like the C.
pub fn address_parse(text: &str) -> (bool, Addr) {
    let s = text.as_bytes();
    let mut addr = Addr::default();
    let (p, max) = if text.contains(':') {
        (addr6parse(s, &mut addr), 128)
    } else if text.contains('.') {
        (addr4parse(s, &mut addr), 32)
    } else {
        return (false, addr);
    };
    addr.str = text.chars().take(43).collect();
    let Some(p) = p else { return (false, addr) };
    if p.first() == Some(&b'/') {
        addr.mask = super::spawn::c_atoi(std::str::from_utf8(&p[1..]).unwrap_or(""));
        if addr.mask < 1 || addr.mask > max {
            addr.mask = max;
        }
    } else {
        if !p.is_empty() {
            return (false, addr);
        }
        addr.mask = max;
    }
    (true, addr)
}

/// `G_AddressCompare`: `b` is inside `a`'s network.
pub fn address_compare(a: &Addr, b: &Addr) -> bool {
    if a.ipv6 != b.ipv6 || a.str.is_empty() || b.str.is_empty() {
        return false;
    }
    let mut netmask = a.mask;
    let max = if a.ipv6 { 128 } else { 32 };
    if netmask < 1 || netmask > max {
        netmask = max;
    }
    let mut i = 0;
    while netmask > 7 {
        if a.addr[i] != b.addr[i] {
            return false;
        }
        i += 1;
        netmask -= 8;
    }
    if netmask != 0 {
        let m = (((1u32 << netmask) - 1) << (8 - netmask)) as u8;
        return (a.addr[i] & m) == (b.addr[i] & m);
    }
    true
}

/// The pseudo-GUID of a bot: stable across maps for the same name.
pub fn bot_guid(name: &str) -> String {
    format!("BOT_{}", sanitise_string(name).to_uppercase())
}

/// `g_admin_level_t`.
#[derive(Clone, Debug, Default)]
pub struct AdminLevel {
    pub level: i32,
    pub name: String,
    pub flags: String,
}

/// `g_admin_admin_t`.
#[derive(Clone, Debug, Default)]
pub struct Admin {
    pub level: i32,
    pub guid: String,
    pub name: String,
    pub flags: String,
}

/// `g_admin_ban_t`.
#[derive(Clone, Debug, Default)]
pub struct Ban {
    pub name: String,
    pub guid: String,
    pub ip: Addr,
    pub reason: String,
    pub made: String,
    pub expires: i32,
    pub banner: String,
    pub warn_count: i32,
}

/// `g_admin_command_t`: a custom [command] that runs a console line.
#[derive(Clone, Debug, Default)]
pub struct AdminCommand {
    pub command: String,
    pub exec: String,
    pub desc: String,
    pub flag: String,
}

/// g_admin_levels / g_admin_admins / g_admin_bans / g_admin_commands.
#[derive(Clone, Debug, Default)]
pub struct AdminState {
    pub levels: Vec<AdminLevel>,
    pub admins: Vec<Admin>,
    pub bans: Vec<Ban>,
    pub commands: Vec<AdminCommand>,
    /// `admin_level_maxname`.
    pub level_maxname: i32,
}

/// `namelog_t`.
#[derive(Clone, Debug, Default)]
pub struct NameLog {
    pub names: [String; MAX_NAMELOG_NAMES],
    /// `ip[ MAX_NAMELOG_ADDRS ]`: the addresses this player used
    pub ip: Vec<Addr>,
    pub guid: String,
    pub slot: i32,
    pub banned: bool,
    pub name_offset: usize,
    pub name_change_time: i32,
    pub name_changes: i32,
    pub vote_count: i32,
    pub muted: bool,
    pub deny_build: bool,
    pub score: i32,
    pub credits: i32,
    pub team: Team,
    pub id: i32,
}

impl NameLog {
    pub fn name(&self) -> &str {
        &self.names[self.name_offset]
    }
}

// ================================================================== output

fn now(w: &World) -> i32 {
    w.real_time
}

/// `G_admin_print` (ADMP): to the client's console, or (decoloured, as with
/// com_ansiColor 0) to the server console.
pub fn admp(w: &mut World, ent: Option<usize>, m: &str) {
    match ent {
        Some(i) => {
            let t = w.level.time;
            for line in m.trim_end_matches('\n').split('\n') {
                w.prints.push((t, None, Some(i), line.to_string()));
            }
        }
        None => w.console.push_str(&decolor_string(m)),
    }
}

/// `AP( "print ..." )`: a print to every client (and the console log).
pub fn ap(w: &mut World, m: &str) {
    let t = w.level.time;
    for line in m.trim_end_matches('\n').split('\n') {
        w.prints.push((t, None, None, line.to_string()));
    }
    w.console.push_str(&decolor_string(m));
}

/// `CPx( slot, "cp ..." )`.
fn cpx(w: &mut World, slot: i32, m: &str) {
    if slot >= 0 {
        let t = w.level.time;
        w.center_prints.push((t, None, Some(slot as usize), m.to_string()));
    }
}

/// `G_admin_buffer_begin` / `G_admin_buffer_print` / `G_admin_buffer_end`:
/// ADMBP output collected then printed at once (no 1022-byte packets here).
struct Admbp(String);
impl Admbp {
    fn begin() -> Self {
        Admbp(String::new())
    }
    fn print(&mut self, m: &str) {
        self.0.push_str(m);
    }
    fn end(self, w: &mut World, ent: Option<usize>) {
        admp(w, ent, &self.0);
    }
}

fn ent_name(w: &World, ent: Option<usize>) -> String {
    ent.map(|e| w.clients[e].name.clone()).unwrap_or_else(|| "console".into())
}

// ============================================================= lookups

/// `admin_permission`: does `flags` grant / deny `flag`? Returns (found, perm).
fn admin_permission(flags: &str, flag: &str) -> (bool, bool) {
    let mut lx = Lexer::new(flags);
    let (mut allflags, mut p, mut perm) = (false, false, false);
    loop {
        let tok = lx.parse();
        if tok.is_empty() {
            break;
        }
        perm = true;
        let mut t = tok.as_str();
        if let Some(r) = t.strip_prefix('-') {
            perm = false;
            t = r;
        } else if let Some(r) = t.strip_prefix('+') {
            t = r;
        }
        if t == flag {
            return (true, perm);
        }
        if t == ADMF_ALLFLAGS {
            allflags = true;
            p = perm;
        }
    }
    if allflags {
        perm = p;
    }
    (allflags, perm)
}

/// `G_admin_level`.
pub fn level_def(w: &World, l: i32) -> Option<&AdminLevel> {
    w.admin.levels.iter().find(|x| x.level == l)
}

/// `G_admin_admin`: the [admin] with this GUID.
pub fn admin_by_guid(w: &World, guid: &str) -> Option<usize> {
    w.admin.admins.iter().position(|a| a.guid.eq_ignore_ascii_case(guid))
}

/// `G_admin_command`: a custom [command].
fn custom_command(w: &World, cmd: &str) -> Option<usize> {
    w.admin.commands.iter().position(|c| c.command.eq_ignore_ascii_case(cmd))
}

/// `G_admin_cmd`: the built-in command table (admin_commands sheet).
pub fn builtin(cmd: &str) -> Option<&'static crate::gen::admin_commands::AdminCommandDef> {
    crate::gen::admin_commands::by_id(&cmd.to_lowercase())
}

/// A built-in or custom admin command by this name.
pub fn admin_command_exists(w: &World, cmd: &str) -> bool {
    builtin(cmd).is_some() || custom_command(w, cmd).is_some()
}

/// The `silent` column (commands whose use is not logged or echoed).
pub fn cmd_is_silent(cmd: &str) -> bool {
    builtin(cmd).is_some_and(|d| d.silent)
}

/// `G_admin_permission`: the console always wins; an admin's own flags,
/// then its level's flags (level 0 for non-admins).
pub fn permission(w: &World, ent: Option<usize>, flag: &str) -> bool {
    let Some(i) = ent else { return true };
    let l = match w.clients[i].admin.and_then(|a| w.admin.admins.get(a)) {
        Some(a) => {
            let (found, perm) = admin_permission(&a.flags, flag);
            if found {
                return perm;
            }
            level_def(w, a.level)
        }
        None => level_def(w, 0),
    };
    match l {
        Some(l) => {
            let (found, perm) = admin_permission(&l.flags, flag);
            found && perm
        }
        None => false,
    }
}

fn admin_level_of(w: &World, a: Option<usize>) -> i32 {
    a.and_then(|a| w.admin.admins.get(a)).map(|a| a.level).unwrap_or(0)
}

/// `admin_higher_admin`.
fn higher_admin(w: &World, a: Option<usize>, b: Option<usize>) -> bool {
    let Some(b) = b.and_then(|b| w.admin.admins.get(b)) else { return true };
    let (found, perm) = admin_permission(&b.flags, ADMF_IMMUTABLE);
    if found {
        return !perm;
    }
    b.level <= admin_level_of(w, a)
}

/// `admin_higher_guid`.
fn higher_guid(w: &World, admin_guid: &str, victim_guid: &str) -> bool {
    higher_admin(w, admin_by_guid(w, admin_guid), admin_by_guid(w, victim_guid))
}

/// `admin_higher`: the console always wins.
fn higher(w: &World, admin: Option<usize>, victim: usize) -> bool {
    match admin {
        None => true,
        Some(a) => higher_admin(w, w.clients[a].admin, w.clients[victim].admin),
    }
}

/// `G_admin_name_check`: reserved, numeric, letterless, duplicate or
/// registered-admin names are refused.
pub fn name_check(w: &World, ent: usize, name: &str) -> Result<(), String> {
    let name2 = sanitise_string(name);
    if name2 == "unnamedplayer" {
        return Ok(());
    }
    if name2 == "console" {
        return Err("The name 'console' is not allowed.".into());
    }
    let test = decolor_string(name);
    if test.as_bytes().first().is_some_and(|c| c.is_ascii_digit()) {
        return Err("Names cannot begin with numbers".into());
    }
    if !test.bytes().any(|c| c.is_ascii_alphabetic()) {
        return Err("Names must contain letters".into());
    }
    for i in 0..MAX_CLIENTS {
        if !w.clients[i].connected || i == ent {
            continue;
        }
        if sanitise_string(&w.clients[i].name) == name2 {
            return Err(format!("The name '{name}^7' is already in use"));
        }
    }
    for (k, a) in w.admin.admins.iter().enumerate() {
        if a.level < 1 {
            continue;
        }
        if sanitise_string(&a.name) == name2 && w.clients[ent].admin != Some(k) {
            return Err(format!("The name '{name}^7' belongs to an admin, please use another name"));
        }
    }
    Ok(())
}

/// `G_TeamFromString` (None: NUM_TEAMS).
pub fn team_from_string(s: &str) -> Option<Team> {
    match s.bytes().next().map(|c| c.to_ascii_lowercase()) {
        Some(b'0') | Some(b's') => Some(Team::None),
        Some(b'1') | Some(b'a') => Some(Team::Aliens),
        Some(b'2') | Some(b'h') => Some(Team::Humans),
        _ => None,
    }
}

/// `BG_TeamName`.
pub fn team_name(t: Team) -> &'static str {
    match t {
        Team::None => "spectator",
        Team::Aliens => "alien",
        Team::Humans => "human",
    }
}

// ============================================================== config file

/// `admin_writeconfig`: admin.dat through the host's file system.
pub fn writeconfig(w: &mut World) {
    if w.level.g_admin.is_empty() {
        w.console.push_str("WARNING: g_admin is not set.  configuration will not be saved to a file.\n");
        return;
    }
    let t = now(w);
    let mut f = String::new();
    // `admin_writeconfig_string` / `admin_writeconfig_int`
    let s = |f: &mut String, k: &str, v: &str| {
        f.push_str(k);
        f.push_str(v);
        f.push('\n');
    };
    for l in &w.admin.levels {
        f.push_str("[level]\n");
        s(&mut f, "level   = ", &l.level.to_string());
        s(&mut f, "name    = ", &l.name);
        s(&mut f, "flags   = ", &l.flags);
        f.push('\n');
    }
    for a in &w.admin.admins {
        // don't write level 0 users
        if a.level == 0 {
            continue;
        }
        f.push_str("[admin]\n");
        s(&mut f, "name    = ", &a.name);
        s(&mut f, "guid    = ", &a.guid);
        s(&mut f, "level   = ", &a.level.to_string());
        s(&mut f, "flags   = ", &a.flags);
        f.push('\n');
    }
    for b in &w.admin.bans {
        // don't write expired bans; expires 0 is a permanent ban
        if b.expires != 0 && b.expires <= t {
            continue;
        }
        f.push_str("[ban]\n");
        s(&mut f, "name    = ", &b.name);
        s(&mut f, "guid    = ", &b.guid);
        s(&mut f, "ip      = ", &b.ip.str);
        s(&mut f, "reason  = ", &b.reason);
        s(&mut f, "made    = ", &b.made);
        s(&mut f, "expires = ", &b.expires.to_string());
        s(&mut f, "banner  = ", &b.banner);
        f.push('\n');
    }
    for c in &w.admin.commands {
        f.push_str("[command]\n");
        s(&mut f, "command = ", &c.command);
        s(&mut f, "exec    = ", &c.exec);
        s(&mut f, "desc    = ", &c.desc);
        s(&mut f, "flag    = ", &c.flag);
        f.push('\n');
    }
    let name = w.level.g_admin.clone();
    let ok = w.fs.as_mut().is_some_and(|fs| fs.write(&name, &f));
    if !ok {
        w.console.push_str(&format!("admin_writeconfig: could not open g_admin file \"{name}\"\n"));
    }
}

/// `admin_readconfig_string`: the rest of the line after '='.
fn readconfig_string(lx: &mut Lexer, size: usize) -> String {
    let mut s = String::new();
    let t = lx.parse_ext(false);
    if t != "=" {
        s = t;
        s.truncate(size - 1);
    }
    loop {
        let t = lx.parse_ext(false);
        if t.is_empty() || t.len() + s.len() >= size {
            break;
        }
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&t);
    }
    s
}

/// `admin_readconfig_int`.
fn readconfig_int(lx: &mut Lexer) -> i32 {
    let mut t = lx.parse_ext(false);
    if t == "=" {
        t = lx.parse_ext(false);
    }
    super::spawn::c_atoi(&t)
}

/// `admin_default_levels`: the six levels a fresh install gets.
fn default_levels(w: &mut World) {
    let defs: [(&str, &str); 6] = [
        ("^4Unknown Player", "listplayers admintest adminhelp time"),
        ("^5Server Regular", "listplayers admintest adminhelp time"),
        ("^6Team Manager", "listplayers admintest adminhelp time putteam spec999"),
        ("^2Junior Admin", "listplayers admintest adminhelp time putteam spec999 kick mute ADMINCHAT"),
        ("^3Senior Admin", "listplayers admintest adminhelp time putteam spec999 kick mute showbans ban namelog ADMINCHAT"),
        ("^1Server Operator", "ALLFLAGS -IMMUTABLE -INCOGNITO"),
    ];
    w.admin.levels = defs.iter().enumerate().map(|(i, (n, f))| AdminLevel { level: i as i32, name: n.to_string(), flags: f.to_string() }).collect();
    w.admin.level_maxname = 15;
}

/// `G_admin_cleanup`.
pub fn cleanup(w: &mut World) {
    w.admin = AdminState::default();
}

/// `G_admin_readconfig`: parse admin.dat (default levels when it can't be
/// read or defines none), sort levels and admins by level, and re-map the
/// connected clients' admins.
pub fn readconfig(w: &mut World, ent: Option<usize>) -> bool {
    cleanup(w);
    if w.level.g_admin.is_empty() {
        admp(w, ent, "^3readconfig: g_admin is not set, not loading configuration from a file\n");
        return false;
    }
    let name = w.level.g_admin.clone();
    let Some(text) = w.fs.as_ref().and_then(|f| f.read(&name)) else {
        w.console.push_str(&format!("readconfig: could not open admin config file {name}\n"));
        default_levels(w);
        remap_clients(w);
        return false;
    };
    w.admin.level_maxname = 0;
    let (mut lc, mut ac, mut bc, mut cc) = (0, 0, 0, 0);
    #[derive(PartialEq)]
    enum Open {
        None,
        Level,
        Admin,
        Ban,
        Command,
    }
    let mut open = Open::None;
    let mut lx = Lexer::new(&text);
    loop {
        let t = lx.parse();
        if t.is_empty() {
            break;
        }
        let tl = t.to_ascii_lowercase();
        match tl.as_str() {
            "[level]" => {
                w.admin.levels.push(AdminLevel::default());
                open = Open::Level;
                lc += 1;
            }
            "[admin]" => {
                w.admin.admins.push(Admin::default());
                open = Open::Admin;
                ac += 1;
            }
            "[ban]" => {
                w.admin.bans.push(Ban::default());
                open = Open::Ban;
                bc += 1;
            }
            "[command]" => {
                w.admin.commands.push(AdminCommand::default());
                open = Open::Command;
                cc += 1;
            }
            _ => match open {
                Open::Level => {
                    let l = w.admin.levels.last_mut().unwrap();
                    match tl.as_str() {
                        "level" => l.level = readconfig_int(&mut lx),
                        "name" => {
                            l.name = readconfig_string(&mut lx, MAX_NAME_LENGTH);
                            let len = print_strlen(&l.name) as i32;
                            if len > w.admin.level_maxname {
                                w.admin.level_maxname = len;
                            }
                        }
                        "flags" => l.flags = readconfig_string(&mut lx, 1024),
                        _ => w.console.push_str(&format!("ERROR: [level] unrecognized token \"{t}\"\n")),
                    }
                }
                Open::Admin => {
                    let a = w.admin.admins.last_mut().unwrap();
                    match tl.as_str() {
                        "name" => a.name = readconfig_string(&mut lx, MAX_NAME_LENGTH),
                        "guid" => a.guid = readconfig_string(&mut lx, 33),
                        "level" => a.level = readconfig_int(&mut lx),
                        "flags" => a.flags = readconfig_string(&mut lx, 1024),
                        _ => w.console.push_str(&format!("ERROR: [admin] unrecognized token \"{t}\"\n")),
                    }
                }
                Open::Ban => {
                    let b = w.admin.bans.last_mut().unwrap();
                    match tl.as_str() {
                        "name" => b.name = readconfig_string(&mut lx, MAX_NAME_LENGTH),
                        "guid" => b.guid = readconfig_string(&mut lx, 33),
                        "ip" => b.ip = address_parse(&readconfig_string(&mut lx, 44)).1,
                        "reason" => b.reason = readconfig_string(&mut lx, MAX_ADMIN_BAN_REASON),
                        "made" => b.made = readconfig_string(&mut lx, 18),
                        "expires" => b.expires = readconfig_int(&mut lx),
                        "banner" => b.banner = readconfig_string(&mut lx, MAX_NAME_LENGTH),
                        _ => w.console.push_str(&format!("ERROR: [ban] unrecognized token \"{t}\"\n")),
                    }
                }
                Open::Command => {
                    let c = w.admin.commands.last_mut().unwrap();
                    match tl.as_str() {
                        "command" => c.command = readconfig_string(&mut lx, MAX_ADMIN_CMD_LEN),
                        "exec" => c.exec = readconfig_string(&mut lx, 64),
                        "desc" => c.desc = readconfig_string(&mut lx, 50),
                        "flag" => c.flag = readconfig_string(&mut lx, 20),
                        _ => w.console.push_str(&format!("ERROR: [command] unrecognized token \"{t}\"\n")),
                    }
                }
                Open::None => w.console.push_str(&format!("ERROR: unexpected token \"{t}\"\n")),
            },
        }
    }
    admp(w, ent, &format!("^3readconfig: ^7loaded {lc} levels, {ac} admins, {bc} bans, {cc} commands\n"));
    if lc == 0 {
        default_levels(w);
    } else {
        // llsort( cmplevel ): stable, highest level first
        w.admin.levels.sort_by(|a, b| b.level.cmp(&a.level));
        w.admin.admins.sort_by(|a, b| b.level.cmp(&a.level));
    }
    remap_clients(w);
    true
}

/// readconfig's "restore admin mapping".
fn remap_clients(w: &mut World) {
    for i in 0..MAX_CLIENTS {
        if w.clients[i].connected {
            let g = w.clients[i].guid.clone();
            w.clients[i].admin = admin_by_guid(w, &g);
            if w.clients[i].admin.is_some() {
                super::log::admin_authlog(w, i);
            }
        }
    }
}

/// dec026: the host's player becomes an admin of the highest defined level
/// when admin.dat has no [admin] for it (as if the console ran setlevel).
pub fn host_admin(w: &mut World, i: usize) {
    let Some(top) = w.admin.levels.iter().map(|l| l.level).max() else { return };
    let (guid, name) = (w.clients[i].guid.clone(), w.clients[i].name.clone());
    w.admin.admins.push(Admin { level: top, guid, name, flags: String::new() });
    w.clients[i].admin = Some(w.admin.admins.len() - 1);
    writeconfig(w);
}

// ======================================================== time and bans

/// `trap_RealTime( &qt )` fields from epoch seconds (UTC):
/// (year, month 0-11, mday, hour, min, sec).
pub fn qtime(t: i32) -> (i32, i32, i32, i32, i32, i32) {
    let t = t as i64;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil from days (proleptic Gregorian)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as i32 - 1, d as i32, (secs / 3600) as i32, (secs % 3600 / 60) as i32, (secs % 60) as i32)
}

/// `G_admin_duration`.
pub fn duration(secs: i32) -> String {
    if secs > 60 * 60 * 24 * 365 * 50 || secs < 0 {
        "PERMANENT".into()
    } else if secs >= 60 * 60 * 24 * 365 {
        format!("{:.1} years", secs as f32 / (60.0 * 60.0 * 24.0 * 365.0))
    } else if secs >= 60 * 60 * 24 * 90 {
        format!("{:.1} weeks", secs as f32 / (60.0 * 60.0 * 24.0 * 7.0))
    } else if secs >= 60 * 60 * 24 {
        format!("{:.1} days", secs as f32 / (60.0 * 60.0 * 24.0))
    } else if secs >= 60 * 60 {
        format!("{:.1} hours", secs as f32 / (60.0 * 60.0))
    } else if secs >= 60 {
        format!("{:.1} minutes", secs as f32 / 60.0)
    } else {
        format!("{secs} seconds")
    }
}

/// `G_admin_parse_time`: "1w2d3h4m5s" -> seconds, -1 if malformed.
pub fn parse_time(time: &str) -> i32 {
    let b = time.as_bytes();
    if b.is_empty() {
        return -1;
    }
    let (mut seconds, mut num, mut i) = (0i32, 0i32, 0);
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            return -1;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            num = num.wrapping_mul(10).wrapping_add((b[i] - b'0') as i32);
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let u = b[i];
        i += 1;
        // fall-through multipliers of the C switch
        match u {
            b'w' => num *= 7 * 24 * 60 * 60,
            b'd' => num *= 24 * 60 * 60,
            b'h' => num *= 60 * 60,
            b'm' => num *= 60,
            b's' => {}
            _ => return -1,
        }
        seconds += num;
        num = 0;
    }
    if num != 0 {
        seconds += num;
    }
    seconds
}

/// `G_admin_ban_message`: the disconnect reason.
fn ban_message(w: &World, b: &Ban) -> String {
    format!("You have been banned by {}^7 duration: {} reason: {}", b.banner, duration(b.expires - now(w)), b.reason)
}

/// `G_admin_ban_matches`: the GUID, or the address unless IMMUNITY.
fn ban_matches(w: &World, ban: &Ban, i: usize) -> bool {
    ban.guid.eq_ignore_ascii_case(&w.clients[i].guid) || (!permission(w, Some(i), ADMF_IMMUNITY) && address_compare(&ban.ip, &w.clients[i].ip))
}

/// `G_admin_match_ban`.
fn match_ban(w: &World, i: usize) -> Option<usize> {
    if w.clients[i].local_client {
        return None;
    }
    let t = now(w);
    w.admin.bans.iter().position(|b| !(b.expires != 0 && b.expires <= t) && ban_matches(w, b, i))
}

/// `G_admin_ban_check`: Some(reason) when client `i` is banned.
pub fn ban_check(w: &mut World, i: usize) -> Option<String> {
    if w.clients[i].local_client {
        return None;
    }
    let b = match_ban(w, i)?;
    let reason = ban_message(w, &w.admin.bans[b]);
    let name = if w.clients[i].name.is_empty() { w.admin.bans[b].name.clone() } else { w.clients[i].name.clone() };
    let warning = format!("^3Banned player {name}^3 tried to connect from {} (ban #{})", w.clients[i].ip.str, b + 1);
    let wc = w.admin.bans[b].warn_count;
    w.admin.bans[b].warn_count += 1;
    // don't spam admins, and don't fill the console
    if wc < 5 {
        super::cmds::admin_message(w, None, &warning);
    } else if wc + 1 < 10 {
        let tail = if wc + 2 == 10 { "^7 - future messages for this ban will be suppressed" } else { "" };
        w.console.push_str(&decolor_string(&format!("{warning}{tail}\n")));
    }
    Some(reason)
}

/// `trap_DropClient`: "<name> <reason>" to everyone, then ClientDisconnect.
pub fn drop_client(w: &mut World, i: usize, reason: &str) {
    if !w.clients[i].connected {
        return;
    }
    let name = w.clients[i].name.clone();
    ap(w, &format!("{name}^7 {reason}\n"));
    w.bots.states[i] = Default::default();
    // a networked client's connection is dropped by the server (dec033)
    if w.clients[i].async_cmds {
        w.drops.push((i, reason.to_string()));
    }
    w.disconnect(i);
}

/// `admin_create_ban`: add the ban and drop every client it matches.
fn create_ban(w: &mut World, ent: Option<usize>, netname: &str, guid: &str, ip: &Addr, seconds: i32, reason: &str) {
    let t = now(w);
    let (y, mo, d, h, mi, s) = qtime(t);
    let banner = match ent {
        Some(e) => match w.clients[e].admin.and_then(|a| w.admin.admins.get(a)) {
            Some(a) => a.name.clone(),
            None => w.clients[e].name.clone(),
        },
        None => "console".into(),
    };
    let mut reason = if reason.is_empty() { "banned by admin".to_string() } else { reason.to_string() };
    reason.truncate(MAX_ADMIN_BAN_REASON - 1);
    let b = Ban {
        name: netname.to_string(),
        guid: guid.to_string(),
        ip: ip.clone(),
        reason,
        made: format!("{:02}/{:02}/{:02} {:02}:{:02}:{:02}", mo + 1, d, y % 100, h, mi, s),
        expires: if seconds == 0 { 0 } else { t + seconds },
        banner,
        warn_count: 0,
    };
    w.admin.bans.push(b.clone());
    for i in 0..MAX_CLIENTS {
        if w.clients[i].connected && !w.clients[i].local_client && ban_matches(w, &b, i) {
            let msg = ban_message(w, &b);
            if w.clients[i].async_cmds {
                w.server_commands.push((Some(i), format!("disconnect \"{msg}\"")));
            } else {
                w.console.push_str(&format!("disconnect \"{}\"\n", decolor_string(&msg)));
            }
            drop_client(w, i, &format!("has been kicked by {}^7. reason: {}", b.banner, b.reason));
        }
    }
}

// ============================================================ namelog

/// `G_namelog_connect`.
pub fn namelog_connect(w: &mut World, i: usize) {
    let guid = w.clients[i].guid.clone();
    let n = match w.namelogs.iter().position(|n| n.slot == -1 && n.guid.eq_ignore_ascii_case(&guid)) {
        Some(n) => n,
        None => {
            let id = w.namelogs.last().map(|p| p.id + 1).unwrap_or(MAX_CLIENTS as i32);
            w.namelogs.push(NameLog { guid, id, slot: -1, ..Default::default() });
            w.namelogs.len() - 1
        }
    };
    w.clients[i].namelog = Some(n);
    w.namelogs[n].slot = i as i32;
    w.namelogs[n].banned = false;
    // If they're muted, copy in their last known name - this will stop people
    // reconnecting to get around the name change protection.
    let newname = w.namelogs[n].name().to_string();
    if w.namelogs[n].muted && name_check(w, i, &newname).is_ok() {
        w.clients[i].name = newname;
    }
    let ip = w.clients[i].ip.clone();
    let nl = &mut w.namelogs[n];
    let known = nl.ip.iter().take_while(|a| !a.str.is_empty()).position(|a| a.str == ip.str);
    if known.is_some() {
        return;
    }
    let mut k = nl.ip.iter().take_while(|a| !a.str.is_empty()).count();
    if k == MAX_NAMELOG_ADDRS {
        k -= 1;
    }
    if k < nl.ip.len() {
        nl.ip[k] = ip;
    } else {
        nl.ip.push(ip);
    }
}

/// `G_namelog_disconnect`.
pub fn namelog_disconnect(w: &mut World, i: usize) {
    if let Some(n) = w.clients[i].namelog.take() {
        w.namelogs[n].slot = -1;
    }
}

/// `G_namelog_update_score`.
pub fn namelog_update_score(w: &mut World, i: usize) {
    let Some(n) = w.clients[i].namelog else { return };
    let c = &w.clients[i];
    let (team, score, credit) = (c.team, c.score, c.credit);
    let nl = &mut w.namelogs[n];
    nl.team = team;
    nl.score = score;
    nl.credits = credit;
}

/// `G_namelog_update_name`.
pub fn namelog_update_name(w: &mut World, i: usize) {
    let Some(n) = w.clients[i].namelog else { return };
    let name = w.clients[i].name.clone();
    let nl = &mut w.namelogs[n];
    if !nl.names[nl.name_offset].is_empty() && sanitise_string(&name) != sanitise_string(&nl.names[nl.name_offset]) {
        nl.name_offset = (nl.name_offset + 1) % MAX_NAMELOG_NAMES;
    }
    nl.names[nl.name_offset] = name;
}

/// `G_namelog_restore`: team, score and credits of a reconnecting player.
pub fn namelog_restore(w: &mut World, i: usize) {
    let Some(n) = w.clients[i].namelog else { return };
    let (team, score, credits) = (w.namelogs[n].team, w.namelogs[n].score, w.namelogs[n].credits);
    super::client::change_team(w, i, team);
    w.clients[i].score = score;
    w.clients[i].credit = 0;
    w.clients[i].ps.persistant[crate::gen::enums::Pers::Credit as usize] = 0;
    super::combat::add_credit(w, i, credits, false);
}

/// `G_namelog_cleanup`.
pub fn namelog_cleanup(w: &mut World) {
    w.namelogs.clear();
}

/// `G_NamelogFromString`: a slot, a namelog id (>= MAX_CLIENTS) or a
/// unique name match; prints the candidates when ambiguous.
pub fn namelog_from_string(w: &mut World, ent: Option<usize>, s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|c| c.is_ascii_digit()) {
        let i = super::spawn::c_atoi(s);
        if (0..MAX_CLIENTS as i32).contains(&i) {
            if w.clients[i as usize].connected {
                return w.clients[i as usize].namelog;
            }
        } else if i >= MAX_CLIENTS as i32 {
            if let Some(p) = w.namelogs.iter().position(|n| n.id == i) {
                return Some(p);
            }
        }
        return None;
    }
    let s2 = sanitise_string(s);
    let (mut m, mut found) = (None, 0);
    for (p, n) in w.namelogs.iter().enumerate() {
        for (k, name) in n.names.iter().enumerate() {
            if name.is_empty() {
                break;
            }
            let n2 = sanitise_string(name);
            // if this is an exact match to a current player
            if k == n.name_offset && n.slot > -1 && s2 == n2 {
                return Some(p);
            }
            if n2.contains(&s2) {
                m = Some(p);
            }
        }
        if m == Some(p) {
            found += 1;
        }
    }
    if found == 1 {
        return m;
    }
    if found > 1 {
        namelog_search(w, ent, &s2, None, 0, -1);
    }
    None
}

// ================================================================ searches

/// `admin_search`: print the matches of a list, `limit` per page.
#[allow(clippy::too_many_arguments)]
fn admin_search(w: &mut World, ent: Option<usize>, cmd: &str, noun: &str, matches: &[bool], outs: &[String], arg: &str, mut start: i32, offset: i32, limit: i32) -> i32 {
    let total = matches.len() as i32;
    if start < 0 {
        start += total;
    } else {
        start -= offset;
    }
    if start < 0 || start > total {
        start = 0;
    }
    let (mut count, mut found, mut next, mut end) = (0, 0, 0, 0);
    let mut b = Admbp::begin();
    for i in 0..total {
        if matches[i as usize] {
            if i >= start && (limit < 1 || count < limit) {
                b.print(&format!("{:<3} {}\n", i + offset, outs[i as usize]));
                count += 1;
                end = i;
            } else if count == limit && next == 0 {
                next = i;
            }
            found += 1;
        }
    }
    if limit > 0 {
        b.print(&format!("^3{cmd}: ^7showing {count} of {found} {noun} {}-{}{}{}.", start + offset, end + offset, if arg.is_empty() { "" } else { " matching " }, arg));
        if next != 0 {
            b.print(&format!("  use '{cmd}{}{arg} {}' to see more", if arg.is_empty() { "" } else { " " }, next + offset));
        }
    }
    b.print("\n");
    b.end(w, ent);
    next + offset
}

/// `%-*s` / `%*s` with a byte width.
fn pad(s: &str, width: i32, left: bool) -> String {
    let n = s.len() as i32;
    if n >= width {
        return s.to_string();
    }
    let sp = " ".repeat((width - n) as usize);
    if left { format!("{s}{sp}") } else { format!("{sp}{s}") }
}

fn color_len(s: &str) -> i32 {
    let b = s.as_bytes();
    (0..b.len()).filter(|&i| is_color_string(b, i)).count() as i32 * 2
}

/// `admin_listadmins` (`admin_match` / `admin_out`).
fn listadmins(w: &mut World, ent: Option<usize>, start: i32, search: &str) -> i32 {
    let n2 = sanitise_string(search);
    let maxname = w.admin.level_maxname;
    let matches: Vec<bool> = w.admin.admins.iter().map(|a| n2.is_empty() || sanitise_string(&a.name).contains(&n2)).collect();
    let outs: Vec<String> = w
        .admin
        .admins
        .iter()
        .map(|a| {
            let l = level_def(w, a.level);
            let lncol = l.map(|l| color_len(&l.name)).unwrap_or(0);
            format!("{:<6} {}^7 {}", a.level, pad(l.map(|l| l.name.as_str()).unwrap_or("(null)"), maxname + lncol - 1, false), a.name)
        })
        .collect();
    admin_search(w, ent, "listadmins", "admins", &matches, &outs, search, start, MAX_CLIENTS as i32, MAX_ADMIN_LISTITEMS)
}

/// `namelog_matchip` / `namelog_matchname` / `namelog_out` over
/// level.namelogs (an IP search prints no "matching" text: the C passes the
/// addr_t whose first byte is the IPv4 type, 0).
fn namelog_search(w: &mut World, ent: Option<usize>, s2: &str, ip: Option<&Addr>, start: i32, limit: i32) -> i32 {
    let matches: Vec<bool> = match ip {
        Some(ip) => w.namelogs.iter().map(|n| n.ip.iter().take_while(|a| !a.str.is_empty()).any(|a| address_compare(a, ip) || address_compare(ip, a))).collect(),
        None => w.namelogs.iter().map(|n| n.names.iter().take_while(|x| !x.is_empty()).any(|x| sanitise_string(x).contains(s2))).collect(),
    };
    let outs: Vec<String> = w
        .namelogs
        .iter()
        .map(|n| {
            let (mut p, scolor) = if n.slot > -1 { (format!("^3{:<2}", n.slot), "^3") } else { ("- ".to_string(), "^7") };
            for a in n.ip.iter().take_while(|a| !a.str.is_empty()) {
                p.push_str(&format!(" {}", a.str));
            }
            for (k, name) in n.names.iter().enumerate().take_while(|(_, x)| !x.is_empty()) {
                p.push_str(&format!(" '^7{name}{scolor}'{}", if k == n.name_offset { "*" } else { "" }));
            }
            p
        })
        .collect();
    admin_search(w, ent, "namelog", "recent players", &matches, &outs, s2, start, MAX_CLIENTS as i32, limit)
}

// ======================================================= command dispatch

/// `G_admin_cmd_check`: a custom [command] or a built-in admin command.
/// Returns false when `argv[0]` is neither.
pub fn cmd_check(w: &mut World, ent: Option<usize>, argv: &[&str]) -> bool {
    let Some(first) = argv.first() else { return false };
    let mut command = first.to_lowercase();
    command.truncate(MAX_ADMIN_CMD_LEN - 1);
    if command.is_empty() {
        return false;
    }
    super::log::admin_log_start(w, ent, &command);
    if let Some(c) = custom_command(w, &command) {
        super::log::admin_log(w, &super::log::concat_args_printable(argv, 1));
        let (flag, exec, name) = (w.admin.commands[c].flag.clone(), w.admin.commands[c].exec.clone(), w.admin.commands[c].command.clone());
        let success = permission(w, ent, &flag);
        if success {
            if super::cmds::flood_limited(w, ent) != 0 {
                return true;
            }
            w.exec.push(exec);
        } else {
            admp(w, ent, &format!("^3{name}: ^7permission denied\n"));
        }
        super::log::admin_log_end(w, success);
        return true;
    }
    if let Some(d) = builtin(&command) {
        let mut success = permission(w, ent, d.flag);
        if success {
            if super::cmds::flood_limited(w, ent) != 0 {
                return true;
            }
            if cmd_is_silent(&command) {
                super::log::admin_log_abort(w);
            }
            success = (d.handler)(w, ent, argv);
            if !success {
                super::log::admin_log(w, &super::log::concat_args_printable(argv, 1));
            }
        } else {
            admp(w, ent, &format!("^3{}: ^7permission denied\n", d.id));
            super::log::admin_log(w, &super::log::concat_args_printable(argv, 1));
        }
        super::log::admin_log_end(w, success);
        return true;
    }
    false
}

/// `G_admin_cmdlist`: the admin commands this client may use (the "cmds"
/// completion list; the presentation reads it directly).
pub fn cmdlist(w: &World, ent: Option<usize>) -> Vec<&'static str> {
    crate::gen::admin_commands::ADMIN_COMMANDS.iter().filter(|d| permission(w, ent, d.flag)).map(|d| d.id).collect()
}

fn argc(a: &[&str]) -> usize {
    a.len()
}
fn argv<'a>(a: &[&'a str], n: usize) -> &'a str {
    a.get(n).copied().unwrap_or("")
}
fn all_digits(s: &str) -> bool {
    s.bytes().all(|c| c.is_ascii_digit())
}
fn atoi(s: &str) -> i32 {
    super::spawn::c_atoi(s)
}
fn trunc(s: &str, n: usize) -> String {
    let mut s = s.to_string();
    if s.len() > n - 1 {
        let mut k = n - 1;
        while !s.is_char_boundary(k) {
            k -= 1;
        }
        s.truncate(k);
    }
    s
}

// ============================================================== commands

/// `G_admin_time`.
pub fn cmd_time(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    let (_, _, _, h, m, s) = qtime(now(w));
    admp(w, ent, &format!("^3time: ^7local time is {h:02}:{m:02}:{s:02}\n"));
    true
}

/// `G_admin_setlevel`.
pub fn cmd_setlevel(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    if argc(a) < 3 {
        admp(w, ent, "^3setlevel: ^7usage: setlevel [name|slot#] [level]\n");
        return false;
    }
    let testname = trunc(argv(a, 1), MAX_NAME_LENGTH);
    let lstr = trunc(argv(a, 2), 12);
    let Some(lvl) = level_def(w, atoi(&lstr)).map(|l| l.level) else {
        admp(w, ent, "^3setlevel: ^7level is not defined\n");
        return false;
    };
    if let Some(e) = ent {
        if lvl > admin_level_of(w, w.clients[e].admin) {
            admp(w, ent, "^3setlevel: ^7you may not use setlevel to set a level higher than your current level\n");
            return false;
        }
    }
    let na = w.admin.admins.len() as i32;
    let mut vic: Option<usize> = None;
    let mut adm: Option<usize> = None;
    let mut name = String::new();
    if all_digits(&testname) {
        let id = atoi(&testname);
        if id < MAX_CLIENTS as i32 {
            if !w.clients[id as usize].connected {
                admp(w, ent, &format!("^3setlevel: ^7no player connected in slot {id}\n"));
                return false;
            }
            vic = Some(id as usize);
        } else if id < na + MAX_CLIENTS as i32 {
            adm = Some((id - MAX_CLIENTS as i32) as usize);
        } else {
            admp(w, ent, &format!("^3setlevel: ^7{testname} not in range 1-{}\n", na + MAX_CLIENTS as i32 - 1));
            return false;
        }
    } else {
        name = sanitise_string(&testname);
    }
    if let Some(v) = vic {
        adm = w.clients[v].admin;
    } else if adm.is_none() {
        let mut matches = 0;
        for (k, wa) in w.admin.admins.iter().enumerate() {
            if matches >= 2 {
                break;
            }
            if sanitise_string(&wa.name).contains(&name) {
                adm = Some(k);
                matches += 1;
            }
        }
        for i in 0..MAX_CLIENTS {
            if matches >= 2 {
                break;
            }
            if !w.clients[i].connected {
                continue;
            }
            if matches != 0 && w.clients[i].admin.is_some() && w.clients[i].admin == adm {
                vic = Some(i);
                continue;
            }
            if sanitise_string(&w.clients[i].name).contains(&name) {
                vic = Some(i);
                adm = w.clients[i].admin;
                matches += 1;
            }
        }
        if matches == 0 {
            admp(w, ent, "^3setlevel:^7 no match.  use listplayers or listadmins to find an appropriate number to use instead of name.\n");
            return false;
        }
        if matches > 1 {
            admp(w, ent, "^3setlevel:^7 more than one match.  Use the admin number instead:\n");
            listadmins(w, ent, 0, &name);
            return false;
        }
    }
    if let Some(e) = ent {
        if !higher_admin(w, w.clients[e].admin, adm) {
            admp(w, ent, "^3setlevel: ^7sorry, but your intended victim has a higher admin level than you\n");
            return false;
        }
    }
    let k = match (adm, vic) {
        (Some(k), _) => k,
        (None, Some(v)) => {
            let guid = w.clients[v].guid.clone();
            w.admin.admins.push(Admin { guid, ..Default::default() });
            let k = w.admin.admins.len() - 1;
            w.clients[v].admin = Some(k);
            k
        }
        (None, None) => return false,
    };
    w.admin.admins[k].level = lvl;
    if let Some(v) = vic {
        w.admin.admins[k].name = w.clients[v].name.clone();
    }
    let l = format!("{} ({}) \"{}^7\"", lvl, w.admin.admins[k].guid, w.admin.admins[k].name);
    super::log::admin_log(w, &l);
    let msg = format!("^3setlevel: ^7{}^7 was given level {} admin rights by {}\n", w.admin.admins[k].name, lvl, ent_name(w, ent));
    ap(w, &msg);
    writeconfig(w);
    if let Some(v) = vic {
        super::log::admin_authlog(w, v);
    }
    true
}

/// `G_admin_kick`: a g_adminTempBan ban that drops the player.
pub fn cmd_kick(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let minargc = if permission(w, ent, ADMF_UNACCOUNTABLE) { 2 } else { 3 };
    if argc(a) < minargc {
        admp(w, ent, "^3kick: ^7usage: kick [name] [reason]\n");
        return false;
    }
    let reason = concat(a, 2);
    let pid = match super::client::client_number_from_string(w, argv(a, 1)) {
        Ok(p) => p,
        Err(e) => {
            admp(w, ent, &format!("^3kick: ^7{e}"));
            return false;
        }
    };
    if !higher(w, ent, pid) {
        admp(w, ent, "^3kick: ^7sorry, but your intended victim has a higher admin level than you\n");
        return false;
    }
    if w.clients[pid].local_client {
        admp(w, ent, "^3kick: ^7disconnecting the host would end the game\n");
        return false;
    }
    let (name, guid, ip) = (w.clients[pid].name.clone(), w.clients[pid].guid.clone(), w.clients[pid].ip.clone());
    super::log::admin_log(w, &format!("{pid} ({guid}) \"{name}^7\": \"{reason}^7\""));
    let secs = parse_time(&w.level.admin_temp_ban).max(1);
    create_ban(w, ent, &name, &guid, &ip, secs, if reason.is_empty() { "kicked by admin" } else { &reason });
    writeconfig(w);
    true
}

/// `G_admin_ban` (by slot, name or namelog id; GUID bans only).
pub fn cmd_ban(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    if argc(a) < 2 {
        admp(w, ent, "^3ban: ^7usage: ban [name|slot|IP(/mask)] [duration] [reason]\n");
        return false;
    }
    let search = trunc(argv(a, 1), MAX_NAME_LENGTH);
    let mut seconds = parse_time(argv(a, 2));
    let reason = if seconds <= 0 {
        seconds = 0;
        concat(a, 2)
    } else {
        concat(a, 3)
    };
    if reason.is_empty() && !permission(w, ent, ADMF_UNACCOUNTABLE) {
        admp(w, ent, "^3ban: ^7you must specify a reason\n");
        return false;
    }
    if !permission(w, ent, ADMF_CAN_PERM_BAN) {
        let maximum = parse_time(&w.level.admin_max_ban).max(1);
        if seconds == 0 || seconds > maximum {
            admp(w, ent, "^3ban: ^7you may not issue permanent bans\n");
            seconds = maximum;
        }
    }
    let (is_ip, ip) = address_parse(&search);
    let m: Option<usize>;
    if is_ip {
        let max = if ip.ipv6 { 128 } else { 32 };
        let cidr = permission(w, ent, ADMF_CAN_IP_BAN);
        let min = if cidr { if ent.is_some() { max / 2 } else { 1 } } else { max };
        if ip.mask < min || ip.mask > max {
            admp(w, ent, &format!("^3ban: ^7invalid netmask ({} is not in the range {min}-{max})\n", ip.mask));
            return false;
        }
        // skip players in the namelog who have already been banned
        m = w.namelogs.iter().position(|n| !n.banned && n.ip.iter().take_while(|a| !a.str.is_empty()).any(|a| address_compare(&ip, a)));
        if m.is_none() {
            if cidr {
                admp(w, ent, "^3ban: ^7no player found by that IP address; banning anyway\n");
            } else {
                admp(w, ent, "^3ban: ^7no player found by that IP address\n");
                return false;
            }
        }
    } else {
        m = match namelog_from_string(w, ent, &search) {
            Some(m) if !w.namelogs[m].banned => Some(m),
            _ => {
                admp(w, ent, "^3ban: ^7no match\n");
                return false;
            }
        };
    }
    if let (Some(e), Some(m)) = (ent, m) {
        let (eg, mg) = (w.clients[e].guid.clone(), w.namelogs[m].guid.clone());
        if !higher_guid(w, &eg, &mg) {
            admp(w, ent, "^3ban: ^7sorry, but your intended victim has a higher admin level than you\n");
            return false;
        }
    }
    let slot = m.map(|m| w.namelogs[m].slot).unwrap_or(-1);
    if slot > -1 && w.clients[slot as usize].local_client {
        admp(w, ent, "^3ban: ^7disconnecting the host would end the game\n");
        return false;
    }
    let dur = duration(if seconds != 0 { seconds } else { -1 });
    let victim = m.map(|m| w.namelogs[m].name().to_string());
    let msg = format!("^3ban:^7 {}^7 has been banned by {}^7 duration: {}, reason: {}\n", victim.as_deref().unwrap_or("an IP address"), ent_name(w, ent), dur, if reason.is_empty() { "banned by admin" } else { &reason });
    ap(w, &msg);
    let guid = m.map(|m| w.namelogs[m].guid.clone()).unwrap_or_default();
    let l = format!("{} ({}) \"{}^7\": \"{}^7\"", seconds, guid, victim.as_deref().unwrap_or("IP ban"), reason);
    super::log::admin_log(w, &l);
    let name = match m {
        Some(m) if slot == -1 => w.namelogs[m].name().to_string(),
        Some(_) => w.clients[slot as usize].name.clone(),
        None => "IP ban".to_string(),
    };
    if is_ip {
        create_ban(w, ent, &name, &guid, &ip, seconds, &reason);
        super::log::admin_log(w, &format!("[{}]", ip.str));
    } else {
        // ban all IP addresses used by this player
        let m = m.unwrap();
        let ips: Vec<Addr> = w.namelogs[m].ip.iter().take_while(|a| !a.str.is_empty()).cloned().collect();
        for a in &ips {
            create_ban(w, ent, &name, &guid, a, seconds, &reason);
            super::log::admin_log(w, &format!("[{}]", a.str));
        }
        // the host's player and bots have no address: one GUID ban (dec033)
        if ips.is_empty() {
            create_ban(w, ent, &name, &guid, &Addr::default(), seconds, &reason);
        }
    }
    if let Some(m) = m {
        w.namelogs[m].banned = true;
    }
    if w.level.g_admin.is_empty() {
        admp(w, ent, "^3ban: ^7WARNING g_admin not set, not saving ban to a file\n");
    } else {
        writeconfig(w);
    }
    true
}

/// `G_admin_unban`.
pub fn cmd_unban(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let time = now(w);
    if argc(a) < 2 {
        admp(w, ent, "^3unban: ^7usage: unban [ban#]\n");
        return false;
    }
    let bnum = atoi(&trunc(argv(a, 1), 5));
    if bnum < 1 || bnum as usize > w.admin.bans.len() {
        admp(w, ent, "^3unban: ^7invalid ban#\n");
        return false;
    }
    let b = (bnum - 1) as usize;
    let exp = w.admin.bans[b].expires;
    if exp > 0 && exp - time <= 0 {
        admp(w, ent, "^3unban: ^7ban already expired\n");
        return false;
    }
    if !permission(w, ent, ADMF_CAN_PERM_BAN) && (exp == 0 || exp - time > parse_time(&w.level.admin_max_ban).max(1)) {
        admp(w, ent, "^3unban: ^7you cannot remove permanent bans\n");
        return false;
    }
    let bn = &w.admin.bans[b];
    let l = format!("{} ({}) \"{}^7\": \"{}^7\": [{}]", if bn.expires != 0 { bn.expires - time } else { 0 }, bn.guid, bn.name, bn.reason, bn.ip.str);
    super::log::admin_log(w, &l);
    let msg = format!("^3unban: ^7ban #{bnum} for {}^7 has been removed by {}\n", w.admin.bans[b].name, ent_name(w, ent));
    ap(w, &msg);
    w.admin.bans[b].expires = time;
    writeconfig(w);
    true
}

/// `G_admin_adjustban`.
pub fn cmd_adjustban(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let time = now(w);
    if argc(a) < 3 {
        admp(w, ent, "^3adjustban: ^7usage: adjustban [ban#] [/mask] [duration] [reason]\n");
        return false;
    }
    let bnum = atoi(&trunc(argv(a, 1), 5));
    if bnum < 1 || bnum as usize > w.admin.bans.len() {
        admp(w, ent, "^3adjustban: ^7invalid ban#\n");
        return false;
    }
    let b = (bnum - 1) as usize;
    let maximum = parse_time(&w.level.admin_max_ban).max(1);
    let exp0 = w.admin.bans[b].expires;
    if !permission(w, ent, ADMF_CAN_PERM_BAN) && (exp0 == 0 || exp0 - time > maximum) {
        admp(w, ent, "^3adjustban: ^7you cannot modify permanent bans\n");
        return false;
    }
    let mut skiparg: i32 = 0;
    let mut mask = 0;
    let mut secs = argv(a, 2).to_string();
    if secs.starts_with('/') {
        let max = if w.admin.bans[b].ip.ipv6 { 128 } else { 32 };
        let min = if permission(w, ent, ADMF_CAN_IP_BAN) { if ent.is_some() { max / 2 } else { 1 } } else { max };
        mask = atoi(&secs[1..]);
        if mask < min || mask > max {
            admp(w, ent, &format!("^3adjustban: ^7invalid netmask ({mask} is not in range {min}-{max})\n"));
            return false;
        }
        secs = argv(a, (3 + skiparg) as usize).to_string();
        skiparg += 1;
    }
    let mode = secs.chars().next().filter(|c| *c == '+' || *c == '-');
    let mut length = parse_time(if mode.is_some() { &secs[1..] } else { &secs });
    let mut dur = String::new();
    if length < 0 {
        skiparg -= 1;
    } else {
        let mut expires;
        if length != 0 {
            if exp0 == 0 && mode.is_some() {
                admp(w, ent, "^3adjustban: ^7new duration must be explicit\n");
                return false;
            }
            expires = match mode {
                Some('+') => exp0 + length,
                Some('-') => exp0 - length,
                _ => time + length,
            };
            if expires <= time {
                admp(w, ent, "^3adjustban: ^7ban duration must be positive\n");
                return false;
            }
        } else {
            length = 0;
            expires = 0;
        }
        if !permission(w, ent, ADMF_CAN_PERM_BAN) && (length == 0 || length > maximum) {
            admp(w, ent, "^3adjustban: ^7you may not issue permanent bans\n");
            expires = time + maximum;
        }
        w.admin.bans[b].expires = expires;
        dur = duration(if expires != 0 { expires - time } else { -1 });
    }
    if mask != 0 {
        let ip = &mut w.admin.bans[b].ip;
        let p = ip.str.find('/').unwrap_or(ip.str.len());
        ip.str.truncate(p);
        if mask != if ip.ipv6 { 128 } else { 32 } {
            ip.str.push_str(&format!("/{mask}"));
        }
        ip.mask = mask;
    }
    let reason = concat(a, (3 + skiparg).max(0) as usize);
    if !reason.is_empty() {
        w.admin.bans[b].reason = trunc(&reason, MAX_ADMIN_BAN_REASON);
    }
    let bn = &w.admin.bans[b];
    let l = format!("{} ({}) \"{}^7\": \"{}^7\": [{}]", if bn.expires != 0 { bn.expires - time } else { 0 }, bn.guid, bn.name, bn.reason, bn.ip.str);
    super::log::admin_log(w, &l);
    let msg = format!(
        "^3adjustban: ^7ban #{bnum} for {}^7 has been updated by {}^7 {}{}{}{}{}{}\n",
        w.admin.bans[b].name,
        ent_name(w, ent),
        if mask != 0 { format!("netmask: /{mask}{}", if length >= 0 || !reason.is_empty() { ", " } else { "" }) } else { String::new() },
        if length >= 0 { "duration: " } else { "" },
        dur,
        if length >= 0 && !reason.is_empty() { ", " } else { "" },
        if !reason.is_empty() { "reason: " } else { "" },
        reason
    );
    ap(w, &msg);
    if let Some(e) = ent {
        w.admin.bans[b].banner = w.clients[e].name.clone();
    }
    writeconfig(w);
    true
}

/// `G_admin_putteam`.
pub fn cmd_putteam(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    if argc(a) < 3 {
        admp(w, ent, "^3putteam: ^7usage: putteam [name] [h|a|s]\n");
        return false;
    }
    let pid = match super::client::client_number_from_string(w, argv(a, 1)) {
        Ok(p) => p,
        Err(e) => {
            admp(w, ent, &format!("^3putteam: ^7{e}"));
            return false;
        }
    };
    if !higher(w, ent, pid) {
        admp(w, ent, "^3putteam: ^7sorry, but your intended victim has a higher  admin level than you\n");
        return false;
    }
    let team = trunc(argv(a, 2), "spectators".len() + 1);
    let Some(t) = team_from_string(&team) else {
        admp(w, ent, &format!("^3putteam: ^7unknown team {team}\n"));
        return false;
    };
    if w.clients[pid].team == t {
        return false;
    }
    let l = format!("{pid} ({}) \"{}^7\"", w.clients[pid].guid, w.clients[pid].name);
    super::log::admin_log(w, &l);
    super::client::change_team(w, pid, t);
    let msg = format!("^3putteam: ^7{}^7 put {}^7 on to the {} team\n", ent_name(w, ent), w.clients[pid].name, team_name(t));
    ap(w, &msg);
    true
}

fn map_exists(w: &World, map: &str) -> bool {
    w.level.map_list.iter().any(|m| m.eq_ignore_ascii_case(map)) || w.fs.as_ref().is_some_and(|f| f.exists(&format!("maps/{map}.bsp")))
}
fn fs_exists(w: &World, path: &str) -> bool {
    w.fs.as_ref().is_some_and(|f| f.exists(path))
}

/// `G_admin_changemap`.
pub fn cmd_changemap(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    if argc(a) < 2 {
        admp(w, ent, "^3changemap: ^7usage: changemap [map] (layout)\n");
        return false;
    }
    let map = argv(a, 1).to_string();
    if !map_exists(w, &map) {
        admp(w, ent, &format!("^3changemap: ^7invalid map name '{map}'\n"));
        return false;
    }
    let mut layout = String::new();
    if argc(a) > 2 {
        layout = argv(a, 2).to_string();
        if layout.eq_ignore_ascii_case("*BUILTIN*") || fs_exists(w, &format!("layouts/{map}/{layout}.dat")) {
            w.exec.push(format!("set g_layouts \"{layout}\""));
        } else {
            admp(w, ent, &format!("^3changemap: ^7invalid layout name '{layout}'\n"));
            return false;
        }
    }
    super::log::admin_log(w, &map);
    super::log::admin_log(w, &layout);
    w.exec.push(format!("map {map}"));
    let msg = format!("^3changemap: ^7map '{map}' started by {}^7 {}\n", ent_name(w, ent), if layout.is_empty() { String::new() } else { format!("(forcing layout '{layout}')") });
    ap(w, &msg);
    true
}

/// `G_admin_mute` (mute / unmute).
pub fn cmd_mute(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let command = trunc(&argv(a, 0).to_lowercase(), MAX_ADMIN_CMD_LEN);
    if argc(a) < 2 {
        admp(w, ent, &format!("^3{command}: ^7usage: {command} [name|slot#]\n"));
        return false;
    }
    let Some(vic) = namelog_from_string(w, ent, &trunc(argv(a, 1), MAX_NAME_LENGTH)) else {
        admp(w, ent, &format!("^3{command}: ^7no match\n"));
        return false;
    };
    if let Some(e) = ent {
        let vg = w.namelogs[vic].guid.clone();
        if !higher_admin(w, w.clients[e].admin, admin_by_guid(w, &vg)) {
            admp(w, ent, &format!("^3{command}: ^7sorry, but your intended victim has a higher admin level than you\n"));
            return false;
        }
    }
    let slot = w.namelogs[vic].slot;
    let vname = w.namelogs[vic].name().to_string();
    if w.namelogs[vic].muted {
        if command == "mute" {
            admp(w, ent, "^3mute: ^7player is already muted\n");
            return false;
        }
        w.namelogs[vic].muted = false;
        cpx(w, slot, "^1You have been unmuted");
        let msg = format!("^3unmute: ^7{vname}^7 has been unmuted by {}\n", ent_name(w, ent));
        ap(w, &msg);
    } else {
        if command == "unmute" {
            admp(w, ent, "^3unmute: ^7player is not currently muted\n");
            return false;
        }
        w.namelogs[vic].muted = true;
        cpx(w, slot, "^1You've been muted");
        let msg = format!("^3mute: ^7{vname}^7 has been muted by ^7{}\n", ent_name(w, ent));
        ap(w, &msg);
    }
    let l = format!("{slot} ({}) \"{vname}^7\"", w.namelogs[vic].guid);
    super::log::admin_log(w, &l);
    true
}

/// `G_admin_denybuild` (denybuild / allowbuild).
pub fn cmd_denybuild(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let command = trunc(&argv(a, 0).to_lowercase(), MAX_ADMIN_CMD_LEN);
    if argc(a) < 2 {
        admp(w, ent, &format!("^3{command}: ^7usage: {command} [name|slot#]\n"));
        return false;
    }
    let Some(vic) = namelog_from_string(w, ent, &trunc(argv(a, 1), MAX_NAME_LENGTH)) else {
        admp(w, ent, &format!("^3{command}: ^7no match\n"));
        return false;
    };
    if let Some(e) = ent {
        let vg = w.namelogs[vic].guid.clone();
        if !higher_admin(w, w.clients[e].admin, admin_by_guid(w, &vg)) {
            admp(w, ent, &format!("^3{command}: ^7sorry, but your intended victim has a higher admin level than you\n"));
            return false;
        }
    }
    let slot = w.namelogs[vic].slot;
    let vname = w.namelogs[vic].name().to_string();
    if w.namelogs[vic].deny_build {
        if command == "denybuild" {
            admp(w, ent, "^3denybuild: ^7player already has no building rights\n");
            return false;
        }
        w.namelogs[vic].deny_build = false;
        cpx(w, slot, "^1You've regained your building rights");
        let msg = format!("^3allowbuild: ^7building rights for ^7{vname}^7 restored by {}\n", ent_name(w, ent));
        ap(w, &msg);
    } else {
        if command == "allowbuild" {
            admp(w, ent, "^3allowbuild: ^7player already has building rights\n");
            return false;
        }
        w.namelogs[vic].deny_build = true;
        if slot > -1 {
            w.clients[slot as usize].ps.stats[Stat::Buildable as usize] = 0;
        }
        cpx(w, slot, "^1You've lost your building rights");
        let msg = format!("^3denybuild: ^7building rights for ^7{vname}^7 revoked by ^7{}\n", ent_name(w, ent));
        ap(w, &msg);
    }
    let l = format!("{slot} ({}) \"{vname}^7\"", w.namelogs[vic].guid);
    super::log::admin_log(w, &l);
    true
}

/// `G_admin_listadmins`.
pub fn cmd_listadmins(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let mut search = String::new();
    let mut start = MAX_CLIENTS as i32;
    if argc(a) == 3 {
        start = atoi(argv(a, 2));
    }
    if argc(a) > 1 {
        let s = trunc(argv(a, 1), MAX_NAME_LENGTH);
        let b = s.as_bytes();
        let mut i = 0;
        if argc(a) == 2 {
            i = (b.first() == Some(&b'-')) as usize;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i != 0 && i == b.len() {
            start = atoi(&s);
        } else {
            search = sanitise_string(&s);
        }
    }
    listadmins(w, ent, start, &search);
    true
}

/// `G_admin_listlayouts`.
pub fn cmd_listlayouts(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let map = if argc(a) == 2 { argv(a, 1).to_string() } else { w.map_name.clone() };
    let (count, list) = super::layout::layout_list(w, &map);
    let mut b = Admbp::begin();
    b.print(&format!("^3listlayouts:^7 {count} layouts found for '{map}':\n"));
    for l in list.split(' ').filter(|l| !l.is_empty()) {
        b.print(&format!(" {l}\n"));
    }
    b.end(w, ent);
    true
}

/// `G_admin_listplayers`.
pub fn cmd_listplayers(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    let canset = permission(w, ent, "setlevel");
    super::team::calculate_ranks(w);
    let n = w.clients.iter().filter(|c| c.connected).count();
    let mut b = Admbp::begin();
    b.print(&format!("^3listplayers: ^7{n} players connected:\n"));
    let maxname = w.admin.level_maxname;
    for i in 0..MAX_CLIENTS {
        let p = &w.clients[i];
        if !p.connected {
            continue;
        }
        let t = team_name(p.team).chars().next().unwrap().to_ascii_uppercase();
        let c = match p.team {
            Team::Humans => '5',
            Team::Aliens => '1',
            Team::None => '7',
        };
        let nl = p.namelog.map(|n| &w.namelogs[n]);
        let muted = if nl.is_some_and(|n| n.muted) { 'M' } else { ' ' };
        let denied = if nl.is_some_and(|n| n.deny_build) { 'B' } else { ' ' };
        let mut l = level_def(w, 0);
        let mut registered = None;
        let mut hint = canset;
        if let Some(adm) = p.admin.and_then(|a| w.admin.admins.get(a)) {
            if hint {
                hint = higher(w, ent, i);
            }
            if hint || !permission(w, Some(i), ADMF_INCOGNITO) {
                l = level_def(w, adm.level);
                if !sanitise_string(&p.name).eq_ignore_ascii_case(&sanitise_string(&adm.name)) {
                    registered = Some(adm.name.clone());
                }
            }
        }
        let lname = l.map(|l| l.name.clone()).unwrap_or_default();
        let colorlen = color_len(&lname);
        b.print(&format!(
            "{:>2} ^{c}{t}^7 {:<2}^2{}^7 {}^7 ^1{muted}{denied}^7 {}^7 {}{}{}\n",
            i,
            l.map(|l| l.level).unwrap_or(0),
            if hint { '*' } else { ' ' },
            pad(&lname, maxname + colorlen, false),
            p.name,
            if registered.is_some() { "(a.k.a. " } else { "" },
            registered.as_deref().unwrap_or(""),
            if registered.is_some() { "^7)" } else { "" }
        ));
    }
    b.end(w, ent);
    true
}

/// `G_admin_showbans` (`ban_matchname` / `ban_out`).
pub fn cmd_showbans(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let mut start = 1;
    let mut name_match = String::new();
    let mut ip: Option<Addr> = None;
    if argc(a) == 3 {
        start = atoi(argv(a, 2));
    }
    if argc(a) > 1 {
        let f = trunc(argv(a, 1), MAX_NAME_LENGTH);
        let b = f.as_bytes();
        let mut i = 0;
        if argc(a) == 2 {
            i = (b.first() == Some(&b'-')) as usize;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i == b.len() {
            start = atoi(&f);
        } else {
            let (ok, a) = address_parse(&f);
            if ok {
                ip = Some(a);
            } else {
                name_match = sanitise_string(&f);
            }
        }
    }
    let t = now(w);
    // ban_matchip / ban_matchname
    let matches: Vec<bool> = match &ip {
        Some(ip) => w.admin.bans.iter().map(|b| address_compare(&b.ip, ip) || address_compare(ip, &b.ip)).collect(),
        None => w.admin.bans.iter().map(|b| sanitise_string(&b.name).contains(&name_match)).collect(),
    };
    let outs: Vec<String> = w
        .admin
        .bans
        .iter()
        .map(|b| {
            let date: String = b.made.chars().take_while(|c| *c != ' ').take(10).collect();
            let (dur, dcol) = if b.expires == 0 || b.expires - t > 0 { (duration(if b.expires != 0 { b.expires - t } else { -1 }), "^7") } else { ("expired".to_string(), "^5") };
            format!(
                "{} {}{} ^7{} {}\n     \\__ {dcol}{} ^7{}",
                pad(&b.name, MAX_NAME_LENGTH as i32 + color_len(&b.name) - 1, true),
                if b.ip.str.contains('/') { "^1" } else { "^7" },
                pad(&b.ip.str, 15, true),
                pad(&date, 8, true),
                b.banner,
                pad(&dur, 13 - 1, true),
                b.reason
            )
        })
        .collect();
    admin_search(w, ent, "showbans", "bans", &matches, &outs, &name_match, start, 1, MAX_ADMIN_SHOWBANS);
    true
}

/// `G_admin_adminhelp`.
pub fn cmd_adminhelp(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    if argc(a) < 2 {
        let mut count = 0;
        let mut b = Admbp::begin();
        for d in crate::gen::admin_commands::ADMIN_COMMANDS.iter() {
            if permission(w, ent, d.flag) {
                b.print(&format!("^3{}", pad(d.id, 12, true)));
                count += 1;
                // show 6 commands per line
                if count % 6 == 0 {
                    b.print("\n");
                }
            }
        }
        for c in w.admin.commands.clone() {
            if !permission(w, ent, &c.flag) {
                continue;
            }
            b.print(&format!("^3{}", pad(&c.command, 12, true)));
            count += 1;
            if count % 6 == 0 {
                b.print("\n");
            }
        }
        if count % 6 != 0 {
            b.print("\n");
        }
        b.print(&format!("^3adminhelp: ^7{count} available commands\n"));
        b.print("run adminhelp [^3command^7] for adminhelp with a specific command.\n");
        b.end(w, ent);
        return true;
    }
    let param = trunc(argv(a, 1), MAX_ADMIN_CMD_LEN);
    let mut denied = false;
    let mut b = Admbp::begin();
    if let Some(c) = custom_command(w, &param).map(|c| w.admin.commands[c].clone()) {
        if permission(w, ent, &c.flag) {
            b.print(&format!("^3adminhelp: ^7help for '{}':\n", c.command));
            b.print(&format!(" ^3Description: ^7{}\n", c.desc));
            b.print(&format!(" ^3Syntax: ^7{}\n", c.command));
            b.print(&format!(" ^3Flag: ^7'{}'\n", c.flag));
            b.end(w, ent);
            return true;
        }
        denied = true;
    }
    if let Some(d) = builtin(&param) {
        if permission(w, ent, d.flag) {
            b.print(&format!("^3adminhelp: ^7help for '{}':\n", d.id));
            b.print(&format!(" ^3Description: ^7{}\n", d.function));
            b.print(&format!(" ^3Syntax: ^7{} {}\n", d.id, d.syntax.unwrap_or("")));
            b.print(&format!(" ^3Flag: ^7'{}'\n", d.flag));
            b.end(w, ent);
            return true;
        }
        denied = true;
    }
    b.print(&format!("^3adminhelp: ^7{} '{param}'\n", if denied { "you do not have permission to use" } else { "no help found for" }));
    b.end(w, ent);
    false
}

/// `G_admin_admintest`.
pub fn cmd_admintest(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    let Some(e) = ent else {
        admp(w, ent, "^3admintest: ^7you are on the console.\n");
        return true;
    };
    let l = level_def(w, admin_level_of(w, w.clients[e].admin)).cloned();
    let msg = match l {
        Some(l) => format!("^3admintest: ^7{}^7 is a level {} admin ({}^7)\n", w.clients[e].name, l.level, l.name),
        None => format!("^3admintest: ^7{}^7 is a level 0 admin ^7\n", w.clients[e].name),
    };
    ap(w, &msg);
    true
}

/// `G_admin_allready`.
pub fn cmd_allready(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    if w.level.intermission_time == 0 {
        admp(w, ent, "^3allready: ^7this command is only valid during intermission\n");
        return false;
    }
    for c in w.clients.iter_mut() {
        if c.connected && c.team != Team::None {
            c.ready_to_exit = true;
        }
    }
    let msg = format!("^3allready:^7 {}^7 says everyone is READY now\n", ent_name(w, ent));
    ap(w, &msg);
    true
}

/// `G_admin_endvote` (cancelvote / passvote).
pub fn cmd_endvote(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let command = trunc(&argv(a, 0).to_lowercase(), MAX_ADMIN_CMD_LEN);
    let cancel = command == "cancelvote";
    let team_s = if argc(a) == 2 { trunc(argv(a, 1), "spectators".len() + 1) } else { "s".into() };
    let Some(team) = team_from_string(&team_s) else {
        admp(w, ent, &format!("^3{command}: ^7invalid team '{team_s}'\n"));
        return false;
    };
    let t = team as usize;
    if w.level.votes[t].time == 0 {
        admp(w, ent, &format!("^3{command}: ^7no vote in progress\n"));
        return false;
    }
    super::log::admin_log(w, super::log::team_name(team));
    let n = super::vote::num_voting_clients(w, team);
    w.level.votes[t].no = if cancel { n } else { 0 };
    w.level.votes[t].yes = if cancel { 0 } else { n };
    super::vote::check_vote(w, team);
    let msg = format!("^3{command}: ^7{}^7 decided that everyone voted {}", ent_name(w, ent), if cancel { "No" } else { "Yes" });
    let tm = w.level.time;
    if team == Team::None {
        ap(w, &format!("{msg}\n"));
    } else {
        w.prints.push((tm, Some(team), None, msg));
    }
    true
}

/// `G_admin_spec999`: there is no ping here, so nobody is ever at 999.
pub fn cmd_spec999(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    for i in 0..MAX_CLIENTS {
        let c = &w.clients[i];
        if !c.connected || c.team == Team::None {
            continue;
        }
        // ps.ping is always 0 in the single-process port
        if c.ps.ping == 999 {
            super::client::change_team(w, i, Team::None);
            let msg = format!("^3spec999: ^7{}^7 moved {}^7 to spectators\n", ent_name(w, ent), w.clients[i].name);
            ap(w, &msg);
        }
    }
    true
}

/// `G_admin_rename`.
pub fn cmd_rename(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    if argc(a) < 3 {
        admp(w, ent, "^3rename: ^7usage: rename [name] [newname]\n");
        return false;
    }
    let newname = trunc(&concat(a, 2), MAX_NAME_LENGTH);
    let pid = match super::client::client_number_from_string(w, argv(a, 1)) {
        Ok(p) => p,
        Err(e) => {
            admp(w, ent, &format!("^3rename: ^7{e}"));
            return false;
        }
    };
    if !higher(w, ent, pid) {
        admp(w, ent, "^3rename: ^7sorry, but your intended victim has a higher admin level than you\n");
        return false;
    }
    if let Err(e) = name_check(w, pid, &newname) {
        admp(w, ent, &format!("^3rename: ^7{e}\n"));
        return false;
    }
    let l = format!("{pid} ({}) \"{}^7\"", w.clients[pid].guid, w.clients[pid].name);
    super::log::admin_log(w, &l);
    super::log::admin_log(w, &newname);
    let msg = format!("^3rename: ^7{}^7 has been renamed to {newname}^7 by {}\n", w.clients[pid].name, ent_name(w, ent));
    ap(w, &msg);
    super::client::userinfo_changed(w, pid, &newname, true);
    true
}

/// `G_admin_restart`: map_restart, optionally with a layout and keeping or
/// switching teams (sess.restartTeam, g_lockTeamsAtStart).
pub fn cmd_restart(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let mut layout = String::new();
    let mut teampref = String::new();
    let is_pref = |s: &str| ["keepteams", "keepteamslock", "switchteams", "switchteamslock"].iter().any(|p| p.eq_ignore_ascii_case(s));
    if argc(a) > 1 {
        let map = w.map_name.clone();
        layout = argv(a, 1).to_string();
        if !is_pref(&layout) {
            if layout.eq_ignore_ascii_case("*BUILTIN*") || fs_exists(w, &format!("layouts/{map}/{layout}.dat")) {
                w.exec.push(format!("set g_layouts \"{layout}\""));
            } else {
                admp(w, ent, &format!("^3restart: ^7layout '{layout}' does not exist\n"));
                return false;
            }
        } else {
            layout.clear();
            teampref = argv(a, 1).to_string();
        }
    }
    if argc(a) > 2 {
        teampref = argv(a, 2).to_string();
    }
    super::log::admin_log(w, &layout);
    super::log::admin_log(w, &teampref);
    let tp = teampref.to_ascii_lowercase();
    if tp.starts_with("keepteams") {
        for c in w.clients.iter_mut().filter(|c| c.connected && c.team != Team::None) {
            c.restart_team = c.team;
        }
    } else if tp.starts_with("switchteams") {
        for c in w.clients.iter_mut().filter(|c| c.connected) {
            match c.team {
                Team::Humans => c.restart_team = Team::Aliens,
                Team::Aliens => c.restart_team = Team::Humans,
                Team::None => {}
            }
        }
    }
    if tp == "switchteamslock" || tp == "keepteamslock" {
        w.exec.push("set g_lockTeamsAtStart \"1\"".into());
    }
    w.exec.push("map_restart".into());
    let msg = format!(
        "^3restart: ^7map restarted by {} {} {}\n",
        ent_name(w, ent),
        if layout.is_empty() { String::new() } else { format!("^7(forcing layout '{layout}^7')") },
        if teampref.is_empty() { String::new() } else { format!("^7(with teams option: '{teampref}^7')") }
    );
    ap(w, &msg);
    true
}

/// `LogExit` for an evacuation-style end (no winner).
pub fn log_exit_evacuation(w: &mut World, why: &str) {
    let l = &mut w.level;
    l.last_win = Some(Team::None);
    l.winner = Some(Team::None);
    super::log::log_exit(w, why);
}

/// `G_admin_nextmap`.
pub fn cmd_nextmap(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    let msg = format!("^3nextmap: ^7{}^7 decided to load the next map\n", ent_name(w, ent));
    ap(w, &msg);
    let why = format!("nextmap was run by {}", ent_name(w, ent));
    log_exit_evacuation(w, &why);
    true
}

/// `G_admin_namelog`.
pub fn cmd_namelog(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let mut start = MAX_CLIENTS as i32;
    let mut s2 = String::new();
    let mut ip: Option<Addr> = None;
    if argc(a) == 3 {
        start = atoi(argv(a, 2));
    }
    if argc(a) > 1 {
        let search = trunc(argv(a, 1), MAX_NAME_LENGTH);
        let b = search.as_bytes();
        let mut i = 0;
        if argc(a) == 2 {
            i = (b.first() == Some(&b'-')) as usize;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i == b.len() {
            start = atoi(&search);
        } else {
            let (ok, a) = address_parse(&search);
            if ok {
                ip = Some(a);
            } else {
                s2 = sanitise_string(&search);
            }
        }
    }
    namelog_search(w, ent, &s2, ip.as_ref(), start, MAX_ADMIN_LISTITEMS);
    true
}

/// `G_admin_lock` (lock / unlock).
pub fn cmd_lock(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    let command = trunc(&argv(a, 0).to_lowercase(), MAX_ADMIN_CMD_LEN);
    if argc(a) < 2 {
        admp(w, ent, &format!("^3{command}: ^7usage: {command} [a|h]\n"));
        return false;
    }
    let lock = command == "lock";
    let team_s = trunc(argv(a, 1), "aliens".len() + 1);
    let team = team_from_string(&team_s);
    let slot = match team {
        Some(Team::Aliens) => &mut w.level.alien_team_locked,
        Some(Team::Humans) => &mut w.level.human_team_locked,
        _ => {
            admp(w, ent, &format!("^3{command}: ^7invalid team: '{team_s}'\n"));
            return false;
        }
    };
    let fail = *slot == lock;
    if !fail {
        *slot = lock;
    }
    let team = team.unwrap();
    if fail {
        admp(w, ent, &format!("^3{command}: ^7the {} team is {} locked\n", team_name(team), if lock { "already" } else { "not currently" }));
        return false;
    }
    super::log::admin_log(w, team_name(team));
    let msg = format!("^3{command}: ^7the {} team has been {}locked by {}\n", team_name(team), if lock { "" } else { "un" }, ent_name(w, ent));
    ap(w, &msg);
    true
}

/// `G_admin_builder`: who built the structure under the crosshair.
pub fn cmd_builder(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    let Some(e) = ent else {
        admp(w, ent, "^3builder: ^7console can't aim.\n");
        return false;
    };
    let buildlog = permission(w, ent, "buildlog");
    let (forward, start) = if w.clients[e].team != Team::None && w.clients[e].spectator == SpectatorState::Not as i32 {
        let am = super::weapon::aim(w, e);
        (am.forward, am.muzzle)
    } else {
        let (f, _, _) = super::qmath::angle_vectors(&w.clients[e].ps.viewangles);
        (f, w.clients[e].ps.origin)
    };
    let end = super::qmath::ma(&start, 1000.0, &forward);
    let tr = w.trace(&start, None, None, &end, e as i32, super::types::k::MASK_PLAYERSOLID);
    let te = tr.entity_num;
    if tr.fraction < 1.0 && te >= 0 && (te as usize) < w.ents.len() && w.ents[te as usize].etype == super::world::EType::Buildable {
        let t = te as usize;
        if !buildlog && w.clients[e].team != Team::None && w.clients[e].team != w.ents[t].team {
            admp(w, ent, "^3builder: ^7structure not owned by your team\n");
            return false;
        }
        let mut logid = String::new();
        if buildlog {
            for i in 0..w.level.num_build_logs {
                let idx = (w.level.build_id - i - 1).rem_euclid(super::layout::MAX_BUILDLOG as i32) as usize;
                let log = &w.level.build_log[idx];
                if log.fate != super::layout::BuildFate::Construct || w.ents[t].buildable != log.buildable {
                    continue;
                }
                let d = super::qmath::sub(&w.ents[t].pos.tr_base, &log.origin);
                if super::qmath::dot(&d, &d) < 2.0 {
                    logid = format!(", buildlog #{}", MAX_CLIENTS as i32 + w.level.build_id - i - 1);
                }
            }
        }
        let hname = w.ents[t].buildable.map(|b| b.cold().display_name).unwrap_or("");
        let by = w.ents[t].built_by.map(|n| w.namelogs[n].name().to_string());
        let msg = format!(
            "^3builder: ^7{hname}{}{}^7{}\n",
            if by.is_some() { " built by " } else { "" },
            by.as_deref().unwrap_or(""),
            if buildlog { if logid.is_empty() { ", not in buildlog".to_string() } else { logid } } else { String::new() }
        );
        admp(w, ent, &msg);
    } else {
        admp(w, ent, "^3builder: ^7no structure found under crosshair\n");
    }
    true
}

/// `G_admin_pause`.
pub fn cmd_pause(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    let t = w.level.time;
    if w.level.paused_time == 0 {
        let msg = format!("^3pause: ^7{}^7 paused the game.\n", ent_name(w, ent));
        ap(w, &msg);
        w.level.paused_time = 1;
        w.center_prints.push((t, None, None, "The game has been paused. Please wait.".into()));
    } else {
        // Prevent accidental pause->unpause race conditions by two admins
        if w.level.paused_time < 1000 {
            admp(w, ent, "^3pause: ^7Unpausing so soon assumed accidental and ignored.\n");
            return false;
        }
        let msg = format!("^3pause: ^7{}^7 unpaused the game (Paused for {} sec) \n", ent_name(w, ent), (w.level.paused_time as f32 / 1000.0) as i32);
        ap(w, &msg);
        w.center_prints.push((t, None, None, "The game has been unpaused!".into()));
        w.level.paused_time = 0;
    }
    true
}

/// `G_admin_buildlog`.
pub fn cmd_buildlog(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    use super::layout::{BuildFate, MAX_BUILDLOG};
    let (bid, nbl) = (w.level.build_id, w.level.num_build_logs);
    if bid == 0 {
        admp(w, ent, "^3buildlog: ^7log is empty\n");
        return true;
    }
    let mut start = MAX_CLIENTS as i32 + bid - nbl;
    let mut id = -1;
    let mut search = String::new();
    let mut s = String::new();
    if argc(a) == 3 {
        start = atoi(argv(a, 2));
    }
    if argc(a) > 1 {
        search = trunc(argv(a, 1), MAX_NAME_LENGTH);
        let b = search.as_bytes();
        let mut i = (b.first() == Some(&b'-')) as usize;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i != 0 && i == b.len() {
            id = atoi(&search);
            if argc(a) == 2 && (id < 0 || id >= MAX_CLIENTS as i32) {
                start = id;
                id = -1;
            } else if id < 0 || id >= MAX_CLIENTS as i32 || !w.clients[id as usize].connected {
                admp(w, ent, "^3buildlog: ^7invalid client id\n");
                return false;
            }
        } else {
            s = sanitise_string(&search);
        }
    } else {
        start = (-MAX_ADMIN_LISTITEMS).max(-bid);
    }
    if start < 0 {
        start = (bid - nbl).max(start + bid);
    } else {
        start -= MAX_CLIENTS as i32;
    }
    if start < bid - nbl || start >= bid {
        admp(w, ent, "^3buildlog: ^7invalid build ID\n");
        return false;
    }
    if let Some(e) = ent.filter(|&e| w.clients[e].team != Team::None) {
        let msg = format!("^3buildlog: ^7{}^7 requested a log of recent building activity\n", w.clients[e].name);
        ap(w, &msg);
    }
    let fates = ["^2built^7", "^3deconstructed^7", "^7replaced^7", "^3destroyed^7", "^1TEAMKILLED^7", "^7unpowered^7", "removed"];
    let mut b = Admbp::begin();
    let mut printed = 0;
    let mut i = start;
    while i < bid && printed < MAX_ADMIN_LISTITEMS {
        let log = w.level.build_log[(i as usize) % MAX_BUILDLOG].clone();
        if id >= 0 && id < MAX_CLIENTS as i32 {
            if log.actor != w.clients[id as usize].namelog {
                i += 1;
                continue;
            }
        } else if !s.is_empty() {
            let Some(act) = log.actor else {
                i += 1;
                continue;
            };
            if !w.namelogs[act].names.iter().take_while(|n| !n.is_empty()).any(|n| sanitise_string(n).contains(&s)) {
                i += 1;
                continue;
            }
        }
        printed += 1;
        let time = (log.time - w.level.start_time) / 1000;
        let stamp = format!("{:>3}:{:02}", time / 60, time % 60);
        let indirect = matches!(log.fate, BuildFate::Replace | BuildFate::Unpower);
        let hname = log.buildable.map(|x| x.cold().display_name).unwrap_or("");
        let by = log.built_by.filter(|_| log.fate != BuildFate::Construct).map(|n| w.namelogs[n].name().to_string());
        let actor = log.actor.map(|n| w.namelogs[n].name().to_string());
        b.print(&format!(
            "^2{}^7{:<3} {} {}^7{}{}{} {}{}{}\n",
            if log.actor.is_some() && !indirect { '*' } else { ' ' },
            i + MAX_CLIENTS as i32,
            if log.actor.is_some() && indirect { "    \\_".to_string() } else { stamp },
            hname,
            if by.is_some() { " (built by " } else { "" },
            by.as_deref().unwrap_or(""),
            if by.is_some() { "^7)" } else { "" },
            fates[log.fate as usize],
            if actor.is_some() { " by " } else { "" },
            actor.as_deref().unwrap_or("")
        ));
        i += 1;
    }
    let more = if i < bid { format!("run 'buildlog {search}{}{}' to see more", if search.is_empty() { "" } else { " " }, i + MAX_CLIENTS as i32) } else { String::new() };
    b.print(&format!(
        "^3buildlog: ^7showing {printed} build logs {} - {} of {} - {}.  {more}\n",
        start + MAX_CLIENTS as i32,
        i + MAX_CLIENTS as i32 - 1,
        bid + MAX_CLIENTS as i32 - nbl,
        bid + MAX_CLIENTS as i32 - 1
    ));
    b.end(w, ent);
    true
}

/// `G_admin_revert`.
pub fn cmd_revert(w: &mut World, ent: Option<usize>, a: &[&str]) -> bool {
    use super::layout::{BuildFate, MAX_BUILDLOG};
    if argc(a) != 2 {
        admp(w, ent, "^3revert: ^7usage: revert [id]\n");
        return false;
    }
    let id = atoi(argv(a, 1)) - MAX_CLIENTS as i32;
    let (bid, nbl) = (w.level.build_id, w.level.num_build_logs);
    if id < bid - nbl || id >= bid {
        admp(w, ent, "^3revert: ^7invalid id\n");
        return false;
    }
    let log = &w.level.build_log[(id as usize) % MAX_BUILDLOG];
    if log.actor.is_none() || matches!(log.fate, BuildFate::Replace | BuildFate::Unpower) {
        admp(w, ent, "^3revert: ^7you can only revert direct player actions, indicated by ^2* ^7in buildlog\n");
        return false;
    }
    let time = duration((w.level.time - log.time) / 1000);
    super::log::admin_log(w, argv(a, 1));
    let n = bid - id;
    let msg = format!("^3revert: ^7{}^7 reverted {n} {} over the past {time}\n", ent_name(w, ent), if n > 1 { "changes" } else { "change" });
    ap(w, &msg);
    super::layout::build_log_revert(w, id);
    true
}

/// `G_admin_readconfig` as an admin command.
pub fn cmd_readconfig(w: &mut World, ent: Option<usize>, _a: &[&str]) -> bool {
    readconfig(w, ent)
}
