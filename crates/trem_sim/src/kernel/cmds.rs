//! Kernel `cmds` — game/g_cmds.c ClientCommand: the `cmds[]` table lives in
//! the `client_commands` sheet (id, flags, handler), this file holds the
//! gating ClientCommand does from the flags and the handlers that are not
//! gameplay rules already in client.rs (chat, cheats, misc).
//!
//! Handlers have the `sig::ClientCmd` shape: (world, client, argv) where
//! argv[0] is the command name, like trap_Argv.

use super::bg;
use super::client::{self, notify};
use super::combat::{self, FL_GODMODE, FL_NOTARGET};
use super::types::k;
use super::world::{World, CONSOLE, MAX_CLIENTS};
use crate::gen::enums::{cmd_flag as cf, MeansOfDeath as Mod, SpectatorState, Stat, Team};
use crate::gen::upgrades::UpgradeId;
use crate::gen::weapons::WeaponId;

/// `ClientCommand`: look the command up in the sheet table, apply the
/// CMD_* flag tests, run the handler. An unknown command goes to the admin
/// system (`G_admin_cmd_check`) before "Unknown command". Errors are also
/// printed to the client.
pub fn client_command(w: &mut World, i: usize, line: &str) -> Result<(), String> {
    if i >= MAX_CLIENTS || !w.clients[i].connected {
        return Err("not connected".into());
    }
    let argv: Vec<&str> = line.split_whitespace().collect();
    let Some(&name) = argv.first() else { return Ok(()) };
    let lower = name.to_lowercase();
    let Some(def) = crate::gen::client_commands::by_id(&lower) else {
        if super::admin::cmd_check(w, Some(i), &argv) {
            return Ok(());
        }
        let e = format!("Unknown command {name}");
        notify(w, i, e.clone());
        return Err(e);
    };
    // not valid during intermission or a pause: silently ignored, as in the C
    if def.flags & cf::INTERMISSION == 0 && (w.level.intermission_time != 0 || w.level.paused_time != 0) {
        return Ok(());
    }
    if def.flags & cf::CHEAT != 0 && !w.level.cheats {
        let e = "Cheats are not enabled on this server".to_string();
        notify(w, i, e.clone());
        return Err(e);
    }
    // muted players and flooders lose CMD_MESSAGE commands silently
    if def.flags & cf::MESSAGE != 0 && (w.clients[i].namelog.is_some_and(|n| w.namelogs[n].muted) || flood_limited(w, Some(i)) != 0) {
        return Ok(());
    }
    let r = gate(w, i, def.flags).and_then(|_| (def.handler)(w, i, &argv));
    if let Err(e) = &r {
        notify(w, i, e.clone());
    }
    r
}

/// The CMD_* checks of ClientCommand (G_TriggerMenu messages as errors).
fn gate(w: &World, i: usize, flags: u32) -> Result<(), String> {
    let c = &w.clients[i];
    if flags & cf::CHEAT != 0 && !w.level.cheats {
        return Err("Cheats are not enabled on this server".into());
    }
    if flags & cf::TEAM != 0 && c.team == Team::None {
        return Err("Join a team first".into());
    }
    if flags & cf::CHEAT_TEAM != 0 && !w.level.cheats && c.team != Team::None {
        return Err("Cheats are not enabled on this server, so you may only use this command as a spectator".into());
    }
    if flags & cf::SPEC != 0 && c.spectator == SpectatorState::Not as i32 {
        return Err("You can only use this command when spectating".into());
    }
    if flags & cf::ALIEN != 0 && c.team != Team::Aliens {
        return Err("Must be alien to use this command".into());
    }
    if flags & cf::HUMAN != 0 && c.team != Team::Humans {
        return Err("Must be human to use this command".into());
    }
    if flags & cf::LIVING != 0 && (c.ps.stats[Stat::Health as usize] <= 0 || c.spectator != SpectatorState::Not as i32) {
        return Err("Must be living to use this command".into());
    }
    Ok(())
}

pub fn arg<'a>(argv: &[&'a str], n: usize) -> &'a str {
    argv.get(n).copied().unwrap_or("")
}

/// `ConcatArgs(n)`.
pub fn concat(argv: &[&str], n: usize) -> String {
    argv.get(n..).map(|a| a.join(" ")).unwrap_or_default()
}

// ------------------------------------------------- wrappers onto client.rs
pub fn team(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    client::cmd_team(w, i, arg(a, 1))
}
pub fn class(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    client::cmd_class(w, i, arg(a, 1))
}
pub fn buy(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    client::cmd_buy(w, i, arg(a, 1))
}
pub fn sell(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    client::cmd_sell(w, i, arg(a, 1))
}
pub fn build(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    build_revoked(w, i)?;
    client::cmd_build(w, i, arg(a, 1))
}
pub fn reload(w: &mut World, i: usize, _a: &[&str]) -> Result<(), String> {
    client::cmd_reload(w, i)
}
/// `Cmd_Destroy_f`: "deconstruct" marks/removes, "destroy" (cheat) blows up.
pub fn destroy(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    build_revoked(w, i)?;
    client::cmd_deconstruct(w, i, arg(a, 0).eq_ignore_ascii_case("destroy"))
}
/// `pers.namelog->denyBuild` test of Cmd_Build_f / Cmd_Destroy_f (MN_B_REVOKED).
fn build_revoked(w: &World, i: usize) -> Result<(), String> {
    if w.clients[i].namelog.is_some_and(|n| w.namelogs[n].deny_build) {
        return Err("Your teammates have lost faith in your ability to build for the team. You will not be allowed to build until your team votes to reinstate your building rights.".into());
    }
    Ok(())
}
pub fn follow(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    client::cmd_follow(w, i, a.get(1).copied())
}
/// `Cmd_FollowCycle_f`.
pub fn follow_cycle(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    let dir = if arg(a, 0).eq_ignore_ascii_case("followprev") { -1 } else { 1 };
    if w.clients[i].spectator != SpectatorState::Not as i32 {
        client::follow_new_client(w, i, dir);
    }
    Ok(())
}
pub fn callvote(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    super::vote::cmd_callvote(w, i, arg(a, 0).eq_ignore_ascii_case("callteamvote"), arg(a, 1), arg(a, 2), &concat(a, 3))
}
pub fn vote(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    super::vote::cmd_vote(w, i, arg(a, 0).eq_ignore_ascii_case("teamvote"), arg(a, 1))
}

// ------------------------------------------------------------------ items
/// `Cmd_ActivateItem_f`.
pub fn item_act(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    let s = arg(a, 1);
    // "weapon" aliased to whatever weapon you have
    if s.eq_ignore_ascii_case("weapon") {
        if w.clients[i].ps.weapon == WeaponId::Blaster as i32 && bg::player_can_change_weapon(&w.clients[i].ps) {
            client::force_weapon_change(w, i, 0);
        }
        return Ok(());
    }
    let stats = w.clients[i].ps.stats;
    if let Some(up) = UpgradeId::from_id(s).filter(|u| bg::inventory_contains_upgrade(u.num() as i32, &stats)) {
        bg::activate_upgrade(up.num() as i32, &mut w.clients[i].ps.stats);
    } else if let Some(wp) = WeaponId::from_id(s).filter(|x| bg::inventory_contains_weapon(x.num() as i32, &stats)) {
        if w.clients[i].ps.weapon != wp.num() as i32 && bg::player_can_change_weapon(&w.clients[i].ps) {
            client::force_weapon_change(w, i, wp.num() as i32);
        }
    } else {
        return Err(format!("You don't have the {s}"));
    }
    Ok(())
}

/// `Cmd_DeActivateItem_f`.
pub fn item_deact(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    let s = arg(a, 1);
    match UpgradeId::from_id(s) {
        Some(up) if bg::inventory_contains_upgrade(up.num() as i32, &w.clients[i].ps.stats) => {
            bg::deactivate_upgrade(up.num() as i32, &mut w.clients[i].ps.stats);
            Ok(())
        }
        _ => Err(format!("You don't have the {s}")),
    }
}

/// `Cmd_ToggleItem_f`.
pub fn item_toggle(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    let s = arg(a, 1);
    if WeaponId::from_id(s).is_some() {
        if !bg::player_can_change_weapon(&w.clients[i].ps) {
            return Ok(());
        }
        // special case to allow switching between the blaster and the primary weapon
        let target = if w.clients[i].ps.weapon != WeaponId::Blaster as i32 { WeaponId::Blaster as i32 } else { 0 };
        client::force_weapon_change(w, i, target);
        return Ok(());
    }
    let stats = &mut w.clients[i].ps.stats;
    match UpgradeId::from_id(s) {
        Some(up) if bg::inventory_contains_upgrade(up.num() as i32, stats) => {
            let n = up.num() as i32;
            if bg::upgrade_is_active(n, stats) {
                bg::deactivate_upgrade(n, stats);
            } else {
                bg::activate_upgrade(n, stats);
            }
            Ok(())
        }
        _ => Err(format!("You don't have the {s}")),
    }
}

/// `Cmd_Kill_f`: instant with cheats, otherwise a 20 second (cancellable) timer.
pub fn kill(w: &mut World, i: usize, _a: &[&str]) -> Result<(), String> {
    if w.level.cheats {
        w.ents[i].flags &= !FL_GODMODE;
        w.ents[i].health = 0;
        w.clients[i].ps.stats[Stat::Health as usize] = 0;
        combat::player_die(w, i, i, i, 100000, Mod::Suicide as i32);
        return Ok(());
    }
    let t = w.level.time;
    let c = &mut w.clients[i];
    if c.suicide_time == 0 {
        c.suicide_time = t + 20000;
        notify(w, i, "You will suicide in 20 seconds");
    } else if c.suicide_time > t {
        c.suicide_time = 0;
        notify(w, i, "Suicide cancelled");
    }
    Ok(())
}

// ----------------------------------------------------------------- cheats
/// `Cmd_Give_f`.
pub fn give(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    if a.len() < 2 {
        return Err("usage: give [what]: all, health, funds [amount], stamina, poison, gas, ammo".into());
    }
    let name = concat(a, 1);
    let all = name.eq_ignore_ascii_case("all");
    let t = w.level.time;
    if all || name.eq_ignore_ascii_case("health") {
        w.ents[i].health = w.clients[i].ps.stats[Stat::MaxHealth as usize];
        bg::add_upgrade(UpgradeId::Medkit.num() as i32, &mut w.clients[i].ps.stats);
    }
    if all || name.len() >= 5 && name[..5].eq_ignore_ascii_case("funds") {
        let credits = if all || a.len() < 3 {
            30000.0
        } else {
            let mult = if w.clients[i].team == Team::Aliens { k::ALIEN_CREDITS_PER_KILL as f32 } else { 1.0 };
            let c = super::spawn::c_atof(name.get(6..).unwrap_or("")) as f32 * mult;
            if c > i16::MAX as f32 {
                30000.0
            } else if c < i16::MIN as f32 {
                -30000.0
            } else {
                c
            }
        };
        combat::add_credit(w, i, credits as i16 as i32, true);
    }
    if all || name.eq_ignore_ascii_case("stamina") {
        w.clients[i].ps.stats[Stat::Stamina as usize] = k::STAMINA_MAX;
    }
    if name.eq_ignore_ascii_case("poison") {
        let c = &mut w.clients[i];
        if c.team == Team::Humans {
            c.ps.stats[Stat::State as usize] |= k::SS_POISONED;
            c.last_poison_time = t;
            c.last_poison_client = i as i32;
        } else {
            c.ps.stats[Stat::State as usize] |= k::SS_BOOSTED;
            c.boosted_time = t;
        }
    }
    if name.eq_ignore_ascii_case("gas") {
        let c = &mut w.clients[i];
        c.ps.e_flags |= k::EF_POISONCLOUDED;
        c.last_poison_clouded_time = t;
    }
    if all || name.eq_ignore_ascii_case("ammo") {
        let ps = &mut w.clients[i].ps;
        let Some(wp) = WeaponId::from_num(ps.weapon as u8).filter(|_| ps.weapon > 0) else { return Ok(()) };
        let d = wp.def();
        if wp != WeaponId::Level3upg && d.infinite_ammo {
            return Ok(());
        }
        ps.ammo = d.max_ammo;
        ps.clips = d.max_clips;
        if d.uses_energy && bg::inventory_contains_upgrade(UpgradeId::Battpack.num() as i32, &ps.stats) {
            ps.ammo = (ps.ammo as f32 * k::BATTPACK_MODIFIER) as i32;
        }
    }
    Ok(())
}

fn toggle_flag(w: &mut World, i: usize, flag: i32, what: &str) -> Result<(), String> {
    w.ents[i].flags ^= flag;
    let on = w.ents[i].flags & flag != 0;
    notify(w, i, format!("{what} {}", if on { "ON" } else { "OFF" }));
    Ok(())
}
/// `Cmd_God_f`.
pub fn god(w: &mut World, i: usize, _a: &[&str]) -> Result<(), String> {
    toggle_flag(w, i, FL_GODMODE, "godmode")
}
/// `Cmd_Notarget_f`.
pub fn notarget(w: &mut World, i: usize, _a: &[&str]) -> Result<(), String> {
    toggle_flag(w, i, FL_NOTARGET, "notarget")
}
/// `Cmd_Noclip_f`.
pub fn noclip(w: &mut World, i: usize, _a: &[&str]) -> Result<(), String> {
    let on = !w.clients[i].noclip;
    w.clients[i].noclip = on;
    notify(w, i, format!("noclip {}", if on { "ON" } else { "OFF" }));
    Ok(())
}

/// `Cmd_SetViewpos_f`.
pub fn setviewpos(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    if a.len() != 5 {
        return Err("usage: setviewpos x y z yaw".into());
    }
    let o = [super::spawn::c_atof(a[1]) as f32, super::spawn::c_atof(a[2]) as f32, super::spawn::c_atof(a[3]) as f32];
    let angles = [0.0, super::spawn::c_atof(a[4]) as f32, 0.0];
    super::spawn::teleport_player(w, i, &o, &angles);
    Ok(())
}

/// `Cmd_Damage_f`: hurt yourself (testing).
pub fn damage(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    let mut dmg = 100;
    let (mut dx, mut dy, mut dz) = (0.0f32, 0.0f32, 100.0f32);
    let mut nonloc = true;
    if a.len() > 1 {
        dmg = super::spawn::c_atoi(a[1]);
    }
    if a.len() > 4 {
        dx = super::spawn::c_atof(a[2]) as f32;
        dy = super::spawn::c_atof(a[3]) as f32;
        dz = super::spawn::c_atof(a[4]) as f32;
        nonloc = false;
    }
    let o = w.ents[i].origin;
    let point = [o[0] + dx, o[1] + dy, o[2] + dz];
    combat::damage(w, i, None, None, None, Some(point), dmg, if nonloc { combat::DAMAGE_NO_LOCDAMAGE } else { 0 }, Mod::TargetLaser);
    Ok(())
}

/// `Cmd_Where_f`.
pub fn where_(w: &mut World, i: usize, _a: &[&str]) -> Result<(), String> {
    let o = w.ents[i].origin;
    notify(w, i, format!("origin: {:.6} {:.6} {:.6}", o[0], o[1], o[2]));
    Ok(())
}

/// `Cmd_ListMaps_f` (one page: the host's map list, optionally filtered).
pub fn listmaps(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    let search = arg(a, 1).to_lowercase();
    let maps: Vec<String> = w.level.map_list.iter().filter(|m| m.to_lowercase().contains(&search)).cloned().collect();
    notify(w, i, format!("Listing {} maps: {}", maps.len(), maps.join(" ")));
    Ok(())
}

/// Commands whose effect is only on the network client (ScoreboardMessage;
/// the presentation reads the world directly), and `Cmd_Test_f`, which is
/// empty in the C as well.
pub fn nothing(_w: &mut World, _i: usize, _a: &[&str]) -> Result<(), String> {
    Ok(())
}

/// Commands that need a subsystem this port does not have (admin system,
/// voice menus, levelshots); the sheet marks them Dropped.
pub fn unavailable(_w: &mut World, _i: usize, a: &[&str]) -> Result<(), String> {
    Err(format!("{}: not available in this port", arg(a, 0)))
}

// ------------------------------------------------------------------- chat
/// `saymode_t`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SayMode {
    All,
    Team,
    Area,
    Private,
    TeamPrivate,
    Admins,
    AdminsPublic,
    Raw,
}

fn sender_name(w: &World, from: usize) -> String {
    if from == CONSOLE { "console".into() } else { w.clients[from].name.clone() }
}

/// `G_SayTo`. `from` is CONSOLE for the server console.
fn say_to(w: &mut World, from: usize, to: usize, mode: SayMode, text: &str) -> bool {
    if to >= MAX_CLIENTS || !w.clients[to].connected || (from != CONSOLE && w.clients[to].ignored.contains(&from)) {
        return false;
    }
    // OnSameTeam compares teamSelection, so spectators are team mates
    if from != CONSOLE && w.clients[from].team != w.clients[to].team && matches!(mode, SayMode::Team | SayMode::Area | SayMode::TeamPrivate) {
        if w.clients[to].team != Team::None {
            return false;
        }
        // specs with ADMF_SPEC_ALLCHAT flag can see team chat
        if !super::admin::permission(w, Some(to), super::admin::ADMF_SPEC_ALLCHAT) && mode != SayMode::TeamPrivate {
            return false;
        }
    }
    let t = w.level.time;
    let name = sender_name(w, from);
    let line = match mode {
        SayMode::Raw => text.to_string(),
        SayMode::Admins => format!("[ADMIN]{name}: {text}"),
        SayMode::AdminsPublic => format!("[PLAYER]{name}: {text}"),
        _ => format!("{name}: {text}"),
    };
    w.chat.push((t, to, from, mode, line));
    true
}

/// `G_Say`. `from` is CONSOLE for the server console.
pub fn say_all(w: &mut World, from: usize, mut mode: SayMode, text: &str) {
    let ent = (from != CONSOLE).then_some(from);
    // check if blocked by g_specChat 0
    if !w.level.spec_chat && mode != SayMode::Team && from != CONSOLE && w.clients[from].team == Team::None && !super::admin::permission(w, ent, super::admin::ADMF_NOCENSORFLOOD) {
        notify(w, from, "say: Global chatting for spectators has been disabled. You may only use team chat.");
        mode = SayMode::Team;
    }
    let name = sender_name(w, from);
    let id = ent.map(|e| e as i32).unwrap_or(-1);
    match mode {
        SayMode::All => super::log::log_printf(w, &format!("Say: {id} \"{name}^7\": ^2{text}\n")),
        // console say_team is handled in svcmds
        SayMode::Team if ent.is_some() => super::log::log_printf(w, &format!("SayTeam: {id} \"{name}^7\": ^5{text}\n")),
        SayMode::Raw if ent.is_none() => super::log::log_printf(w, &format!("Chat: -1 \"console\": {text}\n")),
        _ => {}
    }
    let text = censor_string(w, text, ent);
    for j in 0..MAX_CLIENTS {
        say_to(w, from, j, mode, &text);
    }
}

/// `Cmd_Say_f` (say / say_team). A line starting with '!' is first tried as
/// an admin command (the shrubbot g_adminParseSay convention, dec026).
pub fn say(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    if a.len() < 2 {
        return Ok(());
    }
    let mode = if arg(a, 0).eq_ignore_ascii_case("say_team") { SayMode::Team } else { SayMode::All };
    let text = concat(a, 1);
    if let Some(cmd) = text.strip_prefix('!') {
        let argv: Vec<&str> = cmd.split_whitespace().collect();
        if !argv.is_empty() && super::admin::admin_command_exists(w, argv[0]) {
            // silent commands (adminhelp, listplayers, ...) are not echoed
            if !super::admin::cmd_is_silent(argv[0]) {
                say_all(w, i, mode, &text);
            }
            super::admin::cmd_check(w, Some(i), &argv);
            return Ok(());
        }
    }
    say_all(w, i, mode, &text);
    Ok(())
}

/// `Cmd_SayArea_f`: team mates within g_sayAreaRange (1000), and spectators
/// with ADMF_SPEC_ALLCHAT.
pub fn say_area(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    if a.len() < 2 {
        return Err("usage: say_area [message]".into());
    }
    let text = concat(a, 1);
    let line = format!("SayArea: {i} \"{}^7\": ^4{text}\n", w.clients[i].name);
    super::log::log_printf(w, &line);
    let o = w.ents[i].origin;
    let r = 1000.0;
    let list = w.entities_in_box(&[o[0] - r, o[1] - r, o[2] - r], &[o[0] + r, o[1] + r, o[2] + r]);
    for e in list {
        if e < MAX_CLIENTS {
            say_to(w, i, e, SayMode::Area, &text);
        }
    }
    for j in 0..MAX_CLIENTS {
        if w.clients[j].connected && w.clients[j].team == Team::None && super::admin::permission(w, Some(j), super::admin::ADMF_SPEC_ALLCHAT) {
            say_to(w, i, j, SayMode::Area, &text);
        }
    }
    Ok(())
}

/// `Cmd_PrivateMessage_f` (m / mt): to every player whose name matches.
/// `ent` None is the server console.
pub fn private_message_from(w: &mut World, ent: Option<usize>, a: &[&str]) {
    let from = ent.unwrap_or(CONSOLE);
    if !w.level.private_messages && ent.is_some() {
        super::admin::admp(w, ent, "Sorry, but private messages have been disabled\n");
        return;
    }
    let cmd = arg(a, 0);
    if a.len() < 3 {
        super::admin::admp(w, ent, &format!("usage: {cmd} [name|slot#] [message]\n"));
        return;
    }
    let teamonly = cmd.eq_ignore_ascii_case("mt");
    let name = a[1];
    let msg = concat(a, 2);
    let pids = client_numbers_from_string(w, name, MAX_CLIENTS);
    let text = censor_string(w, &msg, ent);
    let mut recipients = Vec::new();
    for p in pids {
        if say_to(w, from, p, if teamonly { SayMode::TeamPrivate } else { SayMode::Private }, &text) {
            recipients.push(format!("{}^7", w.clients[p].name));
        }
    }
    let color = if teamonly { '5' } else { '3' };
    if recipients.is_empty() {
        super::admin::admp(w, ent, &format!("^3No player matching ^7'{name}^7' ^3to send message to.\n"));
    } else {
        super::admin::admp(w, ent, &format!("^{color}Private message: ^7{text}\n"));
        let n = recipients.len();
        super::admin::admp(w, ent, &format!("^{color}sent to {n} player{}: ^7{}\n", if n == 1 { "" } else { "s" }, recipients.join(", ")));
        let line = format!("{}: {} \"{}^7\" \"{name}\": ^{color}{msg}\n", if teamonly { "TPrivMsg" } else { "PrivMsg" }, ent.map(|e| e as i32).unwrap_or(-1), sender_name(w, from));
        super::log::log_printf(w, &line);
    }
}

/// `Cmd_PrivateMessage_f` for a client.
pub fn private_message(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    private_message_from(w, Some(i), a);
    Ok(())
}

/// `Cmd_AdminMessage_f` ("a"): message to every admin with ADMF_ADMINCHAT.
pub fn admin_message_cmd(w: &mut World, ent: Option<usize>, a: &[&str]) {
    if !super::admin::permission(w, ent, super::admin::ADMF_ADMINCHAT) {
        if !w.level.public_admin_messages {
            super::admin::admp(w, ent, "Sorry, but use of /a by non-admins has been disabled.\n");
            return;
        }
        super::admin::admp(w, ent, "Your message has been sent to any available admins and to the server logs.\n");
    }
    if a.len() < 2 {
        super::admin::admp(w, ent, "usage: a [message]\n");
        return;
    }
    admin_message(w, ent, &concat(a, 1));
}

/// "a" client command.
pub fn admin_msg(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    admin_message_cmd(w, Some(i), a);
    Ok(())
}

/// `G_AdminMessage`: to all admins with ADMF_ADMINCHAT (and the server console).
pub fn admin_message(w: &mut World, ent: Option<usize>, msg: &str) {
    let mode = if super::admin::permission(w, ent, super::admin::ADMF_ADMINCHAT) { SayMode::Admins } else { SayMode::AdminsPublic };
    let from = ent.unwrap_or(CONSOLE);
    let name = sender_name(w, from);
    let t = w.level.time;
    for j in 0..MAX_CLIENTS {
        if w.clients[j].connected && super::admin::permission(w, Some(j), super::admin::ADMF_ADMINCHAT) {
            let line = format!("{}{name}: {msg}", if mode == SayMode::Admins { "[ADMIN]" } else { "[PLAYER]" });
            w.chat.push((t, j, from, mode, line));
        }
    }
    let line = format!("{}: {} \"{}\": {}\n", if mode == SayMode::Admins { "AdminMsg" } else { "AdminMsgPublic" }, ent.map(|e| e as i32).unwrap_or(-1), name, msg);
    w.console.push_str(&decolor_string(&line));
    // the log file gets the C's own format (G_LogPrintf)
    let logline = format!("{}: {} \"{}^7\": ^6{}\n", if mode == SayMode::Admins { "AdminMsg" } else { "AdminMsgPublic" }, ent.map(|e| e as i32).unwrap_or(-1), name, msg);
    super::log::log_file_printf(w, &logline);
}

// ----------------------------------------------------- strings and limits
/// `Q_IsColorString` at byte `i`.
pub fn is_color_string(s: &[u8], i: usize) -> bool {
    s.get(i) == Some(&b'^') && s.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric())
}

/// `MAX_NAME_LENGTH`.
pub const MAX_NAME_LENGTH: usize = 32;

/// `G_SanitiseString` into a MAX_NAME_LENGTH buffer: colour codes and
/// non-alphanumerics removed, lower case.
pub fn sanitise_string(s: &str) -> String {
    let b = s.as_bytes();
    let mut len = MAX_NAME_LENGTH - 1;
    let (mut i, mut out) = (0, String::new());
    while i < b.len() && len > 0 {
        if is_color_string(b, i) {
            i += 2;
            continue;
        }
        if b[i].is_ascii_alphanumeric() {
            out.push(b[i].to_ascii_lowercase() as char);
            len -= 1;
        }
        i += 1;
    }
    out
}

/// `DECOLOR_OFF` / `DECOLOR_ON`.
pub const DECOLOR_OFF: u8 = 0x0e;
pub const DECOLOR_ON: u8 = 0x0f;

/// `G_DecolorString` (unbounded output).
pub fn decolor_string(s: &str) -> String {
    let b = s.as_bytes();
    let mut decolor = true;
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == DECOLOR_OFF || b[i] == DECOLOR_ON {
            decolor = b[i] == DECOLOR_ON;
            i += 1;
            continue;
        }
        if decolor && is_color_string(b, i) {
            i += 2;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `Q_PrintStrlen`.
pub fn print_strlen(s: &str) -> usize {
    let b = s.as_bytes();
    let (mut i, mut n) = (0, 0);
    while i < b.len() {
        if is_color_string(b, i) {
            i += 2;
            continue;
        }
        i += 1;
        n += 1;
    }
    n
}

/// `G_ClientNumbersFromString`: slot number, else every partial name match.
pub fn client_numbers_from_string(w: &World, s: &str, max: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if max == 0 || s.is_empty() {
        return out;
    }
    // if a number is provided, it is a clientnum (strtol accepts a sign)
    let num = s.strip_prefix(['+', '-']).unwrap_or(s);
    if !num.is_empty() && num.bytes().all(|c| c.is_ascii_digit()) {
        if let Ok(i) = s.parse::<i64>() {
            if (0..MAX_CLIENTS as i64).contains(&i) && w.clients[i as usize].connected {
                out.push(i as usize);
            }
        }
        return out;
    }
    let s2 = sanitise_string(s);
    if s2.is_empty() {
        return out;
    }
    for i in 0..MAX_CLIENTS {
        if out.len() >= max {
            break;
        }
        if w.clients[i].connected && sanitise_string(&w.clients[i].name).contains(&s2) {
            out.push(i);
        }
    }
    out
}

/// `G_LoadCensors`: "term replacement" pairs from the g_censorship file.
pub fn load_censors(w: &mut World) {
    w.censors.clear();
    if w.level.censorship.is_empty() {
        return;
    }
    let name = w.level.censorship.clone();
    let Some(text) = w.fs.as_ref().and_then(|f| f.read(&name)) else {
        w.console.push_str(&format!("ERROR: Censors file {name} doesn't exist\n"));
        return;
    };
    if text.is_empty() || text.len() >= 20000 - 1 {
        w.console.push_str(&format!("ERROR: Censors file {name} is {}\n", if text.is_empty() { "empty" } else { "too long" }));
        return;
    }
    let mut lx = super::maprotation::Lexer::new(&text);
    loop {
        let term = lx.parse();
        if term.is_empty() {
            break;
        }
        let repl = lx.parse_ext(false);
        w.censors.push((term.to_lowercase(), repl));
    }
    w.console.push_str(&format!("Parsed {} string replacements\n", w.censors.len()));
}

/// `G_CensorString`: replace censored terms (matched ignoring colour codes
/// and punctuation) unless the speaker has ADMF_NOCENSORFLOOD.
pub fn censor_string(w: &World, input: &str, ent: Option<usize>) -> String {
    if w.censors.is_empty() || super::admin::permission(w, ent, super::admin::ADMF_NOCENSORFLOOD) {
        return input.to_string();
    }
    let b = input.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if is_color_string(b, i) {
            out.extend_from_slice(&b[i..i + 2]);
            i += 2;
            continue;
        }
        if !b[i].is_ascii_alphanumeric() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let mut matched = false;
        for (term, repl) in &w.censors {
            let m = term.as_bytes();
            let (mut s, mut k) = (i, 0);
            while s < b.len() && k < m.len() {
                if is_color_string(b, s) {
                    s += 2;
                    continue;
                }
                if !b[s].is_ascii_alphanumeric() {
                    s += 1;
                    continue;
                }
                if b[s].to_ascii_lowercase() != m[k] {
                    break;
                }
                s += 1;
                k += 1;
            }
            if k == m.len() {
                i = s;
                out.extend_from_slice(repl.as_bytes());
                matched = true;
                break;
            }
        }
        if !matched {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `G_FloodLimited`: adjust the flood demerits; returns the msec until the
/// client may speak again (0: not limited).
pub fn flood_limited(w: &mut World, ent: Option<usize>) -> i32 {
    if w.level.flood_min_time <= 0 {
        return 0;
    }
    // handles !ent
    if super::admin::permission(w, ent, super::admin::ADMF_NOCENSORFLOOD) {
        return 0;
    }
    let Some(i) = ent else { return 0 };
    let t = w.level.time;
    let c = &mut w.clients[i];
    let deltatime = t - c.flood_time;
    c.flood_demerits += w.level.flood_min_time - deltatime;
    if c.flood_demerits < 0 {
        c.flood_demerits = 0;
    }
    c.flood_time = t;
    let ms = c.flood_demerits - w.level.flood_max_demerits;
    if ms <= 0 {
        return 0;
    }
    notify(w, i, format!("You are flooding: please wait {} second{} before trying again", (ms + 999) / 1000, if ms > 1000 { "s" } else { "" }));
    ms
}

/// `Cmd_Ignore_f` (ignore / unignore).
pub fn ignore(w: &mut World, i: usize, a: &[&str]) -> Result<(), String> {
    let cmd = arg(a, 0);
    if a.len() < 2 {
        return Err(format!("usage: {cmd} [clientNum | partial name match]"));
    }
    let n = client::client_number_from_string(w, &concat(a, 1)).map_err(|e| format!("{cmd}: {e}"))?;
    let name = w.clients[n].name.clone();
    let list = &mut w.clients[i].ignored;
    if cmd.eq_ignore_ascii_case("ignore") {
        if list.contains(&n) {
            return Err(format!("ignore: player {name} is already on your ignore list"));
        }
        list.push(n);
        notify(w, i, format!("ignore: added {name} to your ignore list"));
    } else {
        if !list.contains(&n) {
            return Err(format!("unignore: player {name} is not on your ignore list"));
        }
        list.retain(|&x| x != n);
        notify(w, i, format!("unignore: removed {name} from your ignore list"));
    }
    Ok(())
}

/// `G_ListCommands` (ClientBegin): the client commands the client learns
/// for tab completion — the "cmds" server command (never advertising
/// cheats) and `G_admin_cmdlist`'s admin commands this client may use;
/// the cgame registers both (`CG_GameCmds_f`).
pub fn list_commands(w: &World, i: usize) -> Vec<String> {
    use crate::gen::enums::cmd_flag;
    let mut out: Vec<String> = crate::gen::client_commands::CLIENT_COMMANDS.iter().filter(|c| c.flags & cmd_flag::CHEAT == 0).map(|c| c.id.to_string()).collect();
    out.extend(super::admin::cmdlist(w, Some(i)).into_iter().map(String::from));
    out
}
