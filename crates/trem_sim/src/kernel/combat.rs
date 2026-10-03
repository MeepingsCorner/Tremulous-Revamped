//! Kernel `combat` — port of game/g_combat.c: G_Damage, locational damage
//! modifiers (regions from damage_regions.tsv), radius damage, player_die,
//! G_RewardAttackers, G_AddCreditToClient.

use super::bg;
use super::qmath::*;
use super::types::*;
use super::world::{EType, World, MAX_CLIENTS};
use crate::gen::damage_regions::{DamageRegionDef, DAMAGE_REGIONS};
use crate::gen::enums::{Event, MeansOfDeath as Mod, NonSegAnim, Pers, PlayerAnim, PmType, RegionKind, Stat, Team};
use crate::gen::upgrades::UpgradeId;

pub const DAMAGE_RADIUS: i32 = 0x1;
pub const DAMAGE_NO_ARMOR: i32 = 0x2;
pub const DAMAGE_NO_KNOCKBACK: i32 = 0x4;
pub const DAMAGE_NO_PROTECTION: i32 = 0x8;
pub const DAMAGE_NO_LOCDAMAGE: i32 = 0x10;
pub const FL_NO_KNOCKBACK: i32 = 0x800;
pub const FL_GODMODE: i32 = 0x10;
pub const FL_NOTARGET: i32 = 0x20;

fn area(r: &DamageRegionDef) -> f32 {
    let mut angle_span = r.max_angle - r.min_angle;
    if angle_span < 0.0 {
        angle_span += 360.0;
    }
    angle_span /= 360.0;
    let mut height_span = r.max_height - r.min_height;
    if height_span < 0.0 {
        height_span = -height_span;
    }
    if height_span > 1.0 {
        height_span = 1.0;
    }
    let a = angle_span * height_span;
    if a == 0.0 { 0.00001 } else { a }
}

fn body_regions(class: i32) -> impl Iterator<Item = &'static DamageRegionDef> {
    let cid = bg::class_id(class);
    DAMAGE_REGIONS.iter().filter(move |r| r.kind == RegionKind::Body && r.class == Some(cid))
}
fn armour_regions(up: i32) -> impl Iterator<Item = &'static DamageRegionDef> {
    DAMAGE_REGIONS.iter().filter(move |r| r.kind == RegionKind::Armour && r.upgrade.map(|u| u.num() as i32) == Some(up))
}

/// `GetRegionDamageModifier`.
fn region_modifier(w: &World, targ: usize, overlap: &DamageRegionDef) -> f32 {
    let ps = &w.clients[targ].ps;
    let crouch = ps.pm_flags & k::PMF_DUCKED != 0;
    let mut modifier = 0.0f32;
    let mut area_sum = 0.0f32;
    for u in crate::gen::upgrades::UPGRADES.iter() {
        if !bg::inventory_contains_upgrade(u.num as i32, &ps.stats) {
            continue;
        }
        for r in armour_regions(u.num as i32) {
            if r.crouch != crouch {
                continue;
            }
            let mut overlap_max_a = overlap.max_angle - overlap.min_angle;
            if overlap_max_a < 0.0 {
                overlap_max_a += 360.0;
            }
            let mut region_min_a = r.min_angle - overlap.min_angle;
            if region_min_a < 0.0 {
                region_min_a += 360.0;
            }
            let mut region_max_a = r.max_angle - overlap.min_angle;
            if region_max_a < 0.0 {
                region_max_a += 360.0;
            }
            let mut angle_span;
            if region_min_a <= region_max_a {
                angle_span = 0.0;
                if region_min_a < overlap_max_a {
                    if region_max_a > overlap_max_a {
                        region_max_a = overlap_max_a;
                    }
                    angle_span = region_max_a - region_min_a;
                }
            } else {
                if region_max_a > overlap_max_a {
                    region_max_a = overlap_max_a;
                }
                angle_span = region_max_a;
                if region_min_a < overlap_max_a {
                    angle_span += overlap_max_a - region_min_a;
                }
            }
            angle_span /= 360.0;
            let height_span = (overlap.max_height.min(r.max_height) - overlap.min_height.max(r.min_height)).clamp(0.0, 1.0);
            let a = angle_span * height_span;
            area_sum += a;
            modifier += r.modifier * a;
        }
    }
    overlap.modifier * (area(overlap) + modifier - area_sum)
}

/// `GetNonLocDamageModifier`.
fn non_loc_modifier(w: &World, targ: usize, class: i32) -> f32 {
    let crouch = w.clients[targ].ps.pm_flags & k::PMF_DUCKED != 0;
    let (mut modifier, mut ar, mut sc) = (0.0f32, 0.0f32, 0.0f32);
    for r in body_regions(class) {
        if r.crouch != crouch {
            continue;
        }
        modifier += region_modifier(w, targ, r);
        sc += r.modifier * area(r);
        ar += area(r);
    }
    if sc == 0.0 { 1.0 } else { 1.0 + (modifier / sc - 1.0) * ar }
}

/// `GetPointDamageModifier`.
fn point_modifier<'a>(crouch: bool, regions: impl Iterator<Item = &'a DamageRegionDef>, angle: f32, height: f32) -> f32 {
    let mut m = 1.0f32;
    for r in regions {
        if r.crouch != crouch {
            continue;
        }
        if (r.min_angle <= r.max_angle && (angle < r.min_angle || angle > r.max_angle)) || (r.min_angle > r.max_angle && angle > r.max_angle && angle < r.min_angle) {
            continue;
        }
        if height < r.min_height || height > r.max_height {
            continue;
        }
        m *= r.modifier;
    }
    m
}

/// `G_CalcDamageModifier`.
fn calc_damage_modifier(w: &World, point: Option<&Vec3>, targ: usize, class: i32, dflags: i32) -> f32 {
    let Some(point) = point else { return 1.0 };
    if dflags & DAMAGE_NO_LOCDAMAGE != 0 {
        return non_loc_modifier(w, targ, class);
    }
    let e = &w.ents[targ];
    let ps = &w.clients[targ].ps;
    // the unlagged position the shooter saw (G_UnlaggedCalc)
    let targ_origin = if w.level.unlagged && targ < super::world::MAX_CLIENTS && w.clients[targ].unlagged.calc.used { w.clients[targ].unlagged.calc.origin } else { e.origin };
    let normal = bg::client_normal(ps);
    let floor = ma(&targ_origin, e.mins[2], &normal);
    let p_minus_floor = sub(point, &floor);
    let mut client_height = e.maxs[2] - e.mins[2];
    if client_height == 0.0 {
        client_height = 1.0;
    }
    let hit_relative = (dot(&normal, &p_minus_floor) / length(&normal)).clamp(0.0, client_height);
    let hit_ratio = hit_relative / client_height;
    let bullet_angle = vectoangles(&sub(point, &targ_origin));
    let hit_rotation = angle_normalize360(ps.viewangles[YAW] - bullet_angle[YAW]) as i32 as f32;
    let crouch = ps.pm_flags & k::PMF_DUCKED != 0;
    let mut m = point_modifier(crouch, body_regions(class), hit_rotation, hit_ratio);
    for u in crate::gen::upgrades::UPGRADES.iter() {
        if bg::inventory_contains_upgrade(u.num as i32, &ps.stats) {
            m *= point_modifier(crouch, armour_regions(u.num as i32), hit_rotation, hit_ratio);
        }
    }
    m
}

/// `G_Damage`.
pub fn damage(w: &mut World, targ: usize, inflictor: Option<usize>, attacker: Option<usize>, dir: Option<Vec3>, point: Option<Vec3>, damage: i32, mut dflags: i32, mod_: Mod) {
    if !w.ents[targ].takedamage || w.ents[targ].health <= 0 || w.level.intermission_queued != 0 {
        return;
    }
    let inflictor = inflictor.unwrap_or(ENTITYNUM_WORLD as usize);
    let attacker = attacker.unwrap_or(ENTITYNUM_WORLD as usize);
    // shootable doors / buttons don't actually have any health
    if w.ents[targ].etype == super::world::EType::Mover {
        let st = w.ents[targ].mover_state;
        if let Some(u) = w.ents[targ].use_ {
            if st == super::world::MoverState::Pos1 || st == super::world::MoverState::RotatorPos1 {
                u(w, targ, Some(inflictor), Some(attacker));
            }
        }
        return;
    }
    let is_client = targ < MAX_CLIENTS && w.ents[targ].client;
    let mut dirv = [0.0f32; 3];
    match dir {
        None => dflags |= DAMAGE_NO_KNOCKBACK,
        Some(d) => {
            dirv = d;
            normalize(&mut dirv);
        }
    }
    let mut knockback = damage;
    if w.ents[inflictor].weapon != 0 {
        knockback = (knockback as f32 * bg::weapon(w.ents[inflictor].weapon).knockback_scale) as i32;
    }
    if is_client {
        knockback = (knockback as f32 * bg::class(w.clients[targ].ps.stat(Stat::Class)).knockback_scale) as i32;
    }
    if mod_ == Mod::Falling && knockback > 50 {
        knockback = 50;
    }
    knockback = knockback.min(200);
    if w.ents[targ].flags & FL_NO_KNOCKBACK != 0 || dflags & DAMAGE_NO_KNOCKBACK != 0 {
        knockback = 0;
    }
    if knockback != 0 && is_client {
        let mass = 200.0f32;
        let kvel = scale(&dirv, w.level.g_knockback * knockback as f32 / mass);
        let ps = &mut w.clients[targ].ps;
        ps.velocity = add(&ps.velocity, &kvel);
        if ps.pm_time == 0 {
            ps.pm_time = (knockback * 2).clamp(50, 200);
            ps.pm_flags |= k::PMF_TIME_KNOCKBACK;
        }
    }
    let movement_attack = matches!(mod_, Mod::Level4Trample | Mod::Level3Pounce | Mod::Level4Crush);
    if movement_attack && w.ents[targ].etype == EType::Buildable && w.ents[targ].team == Team::Aliens {
        return;
    }
    if dflags & DAMAGE_NO_PROTECTION == 0 {
        if targ != attacker && w.on_same_team(targ, attacker) {
            if movement_attack || !w.level.friendly_fire {
                return;
            }
        }
        if w.ents[targ].etype == EType::Buildable
            && attacker < MAX_CLIENTS
            && w.ents[attacker].client
            && !matches!(mod_, Mod::Deconstruct | Mod::Suicide | Mod::Replace | Mod::Nocreep)
        {
            // g_friendlyBuildableFire 0
            if w.ents[targ].team == w.clients[attacker].team {
                return;
            }
            // base is under attack warning if DCC'd
            if w.ents[targ].team == Team::Humans && super::buildable::find_dcc(w, targ) != 0 && w.level.time > w.level.human_base_attack_timer {
                w.level.human_base_attack_timer = w.level.time + k::DC_ATTACK_PERIOD;
                w.temp_event([0.0; 3], Event::DccAttack, -1, 0, 0);
            }
        }
        // check for godmode
        if w.ents[targ].flags & FL_GODMODE != 0 {
            return;
        }
    }
    if attacker < MAX_CLIENTS && w.ents[attacker].client && targ != attacker && w.ents[targ].health > 0 && w.ents[targ].etype != EType::Missile && w.ents[targ].etype != EType::General {
        let d = if w.on_same_team(targ, attacker) { -1 } else { 1 };
        w.clients[attacker].ps.persistant[Pers::Hits as usize] += d;
    }
    let mut take = damage;
    if is_client {
        w.clients[targ].ps.persistant[Pers::Attacker as usize] = attacker as i32;
        w.clients[targ].damage_flash += take;
        match dir {
            Some(d) => {
                w.clients[targ].damage_from = d;
                w.clients[targ].damage_from_world = false;
            }
            None => {
                w.clients[targ].damage_from = w.ents[targ].origin;
                w.clients[targ].damage_from_world = true;
            }
        }
        w.clients[targ].last_attacker = attacker as i32;
        let class = w.clients[targ].ps.stat(Stat::Class);
        take = (take as f32 * calc_damage_modifier(w, point.as_ref(), targ, class, dflags) + 0.5) as i32;
        if attacker < MAX_CLIENTS && w.ents[attacker].client && w.clients[attacker].ps.stat(Stat::State) & k::SS_BOOSTED != 0 {
            let t = w.level.time;
            let c = &mut w.clients[targ];
            if c.ps.stat(Stat::Team) == Team::Humans as i32 && !matches!(mod_, Mod::Level2Zap | Mod::Poison | Mod::Level1Pcloud | Mod::Hspawn | Mod::Aspawn) && c.poison_immunity_time < t {
                c.ps.stats[Stat::State as usize] |= k::SS_POISONED;
                c.last_poison_time = t;
                c.last_poison_client = attacker as i32;
            }
        }
    }
    if take < 1 {
        take = 1;
    }
    let t = w.level.time;
    let e = &mut w.ents[targ];
    e.health -= take;
    e.last_damage_time = t;
    e.next_regen_time = t + k::ALIEN_REGEN_DAMAGE_TIME;
    if is_client {
        w.clients[targ].ps.stats[Stat::Health as usize] = w.ents[targ].health;
    }
    if attacker < MAX_CLIENTS && w.ents[attacker].client && attacker != targ {
        w.ents[targ].credits[attacker] += take;
    }
    if w.ents[targ].health <= 0 {
        if is_client {
            w.ents[targ].flags |= FL_NO_KNOCKBACK;
        }
        if w.ents[targ].health < -999 {
            w.ents[targ].health = -999;
        }
        w.ents[targ].enemy = Some(attacker);
        if is_client {
            player_die(w, targ, inflictor, attacker, take, mod_ as i32);
        } else if let Some(die) = w.ents[targ].die {
            die(w, targ, inflictor, attacker, take, mod_ as i32);
        }
    } else if let Some(pain) = w.ents[targ].pain {
        if take != 0 {
            pain(w, targ, attacker, take);
        }
    }
}

/// `CanDamage`.
pub fn can_damage(w: &World, targ: usize, origin: &Vec3) -> bool {
    let e = &w.ents[targ];
    let mid = scale(&add(&e.absmin(), &e.absmax()), 0.5);
    let tr = w.trace(origin, None, None, &mid, ENTITYNUM_NONE, k::MASK_SOLID);
    if tr.fraction == 1.0 || tr.entity_num == targ as i32 {
        return true;
    }
    for (dx, dy) in [(15.0, 15.0), (15.0, -15.0), (-15.0, 15.0), (-15.0, -15.0)] {
        let dest = [mid[0] + dx, mid[1] + dy, mid[2]];
        if w.trace(origin, None, None, &dest, ENTITYNUM_NONE, k::MASK_SOLID).fraction == 1.0 {
            return true;
        }
    }
    false
}

/// `G_RadiusDamage` / `G_SelectiveRadiusDamage` (team = Some: only clients not on `team`).
pub fn radius_damage(w: &mut World, origin: Vec3, attacker: Option<usize>, dmg: f32, mut radius: f32, ignore: Option<usize>, mod_: Mod, team: Option<Team>) -> bool {
    if radius < 1.0 {
        radius = 1.0;
    }
    let mins = [origin[0] - radius, origin[1] - radius, origin[2] - radius];
    let maxs = [origin[0] + radius, origin[1] + radius, origin[2] + radius];
    let mut hit_client = false;
    for i in w.entities_in_box(&mins, &maxs) {
        if Some(i) == ignore || !w.ents[i].takedamage {
            continue;
        }
        if team.is_some() && w.ents[i].flags & FL_NOTARGET != 0 {
            continue;
        }
        let (amin, amax) = (w.ents[i].absmin(), w.ents[i].absmax());
        let mut v = [0.0f32; 3];
        for k2 in 0..3 {
            v[k2] = if origin[k2] < amin[k2] {
                amin[k2] - origin[k2]
            } else if origin[k2] > amax[k2] {
                origin[k2] - amax[k2]
            } else {
                0.0
            };
        }
        let dist = length(&v);
        if dist >= radius {
            continue;
        }
        let points = (dmg as f64 * (1.0 - (dist / radius) as f64)) as f32;
        let team_ok = match team {
            None => true,
            Some(t) => i < MAX_CLIENTS && w.ents[i].client && w.clients[i].ps.stat(Stat::Team) != t as i32,
        };
        if team_ok && can_damage(w, i, &origin) {
            let mut dir = sub(&w.ents[i].origin, &origin);
            dir[2] += 24.0;
            hit_client = true;
            damage(w, i, None, attacker, Some(dir), Some(origin), points as i32, DAMAGE_RADIUS | DAMAGE_NO_LOCDAMAGE, mod_);
        }
    }
    hit_client
}

/// `G_AddCreditToClient`.
pub fn add_credit(w: &mut World, c: usize, credit: i32, cap: bool) {
    let cl = &mut w.clients[c];
    if cap && credit > 0 {
        let cap_amount = if cl.team == Team::Aliens { k::ALIEN_MAX_CREDITS } else { k::HUMAN_MAX_CREDITS };
        if cl.credit < cap_amount {
            cl.credit = (cl.credit + credit).min(cap_amount);
        }
    } else {
        cl.credit += credit;
    }
    if cl.credit < 0 {
        cl.credit = 0;
    }
    cl.ps.persistant[Pers::Credit as usize] = cl.credit;
}

pub fn add_score(w: &mut World, c: usize, s: i32) {
    w.clients[c].score += s;
    w.clients[c].ps.persistant[Pers::Score as usize] = w.clients[c].score;
}

/// `G_RewardAttackers`.
pub fn reward_attackers(w: &mut World, this: usize) -> f32 {
    let mut total = 0.0f32;
    let this_team = w.team_of(this);
    for i in 0..MAX_CLIENTS {
        if !w.clients[i].connected {
            continue;
        }
        if w.clients[i].team != this_team {
            total += w.ents[this].credits[i] as f32;
        }
    }
    if total <= 0.0 {
        return 0.0;
    }
    let (mut value, team, max_health);
    if this < MAX_CLIENTS && w.ents[this].client {
        value = bg::value_of_player(&w.clients[this].ps) as f32;
        team = w.clients[this].team;
        max_health = w.clients[this].ps.stat(Stat::MaxHealth);
    } else if let (EType::Buildable, Some(b)) = (w.ents[this].etype, w.ents[this].buildable) {
        value = b.def().value as f32;
        if !w.ents[this].spawned {
            value *= (w.level.time - w.ents[this].build_time) as f32 / b.def().build_time as f32;
        }
        team = w.ents[this].team;
        max_health = b.def().health;
    } else {
        return total;
    }
    let (mut alien_c, mut human_c) = (0, 0);
    for i in 0..MAX_CLIENTS {
        if !w.clients[i].connected {
            continue;
        }
        let mut stage_value = (value * w.ents[this].credits[i] as f32 / total) as i32;
        if w.clients[i].team != team {
            if total < max_health as f32 {
                stage_value = (stage_value as f32 * (total / max_health as f32)) as i32;
            }
            if w.ents[this].credits[i] == 0 || w.clients[i].ps.stat(Stat::Team) == team as i32 {
                continue;
            }
            add_score(w, i, stage_value);
            if w.ents[this].etype != EType::Buildable {
                add_credit(w, i, stage_value, true);
                if w.clients[i].ps.stat(Stat::Team) == Team::Aliens as i32 {
                    alien_c += stage_value;
                } else if w.clients[i].ps.stat(Stat::Team) == Team::Humans as i32 {
                    human_c += stage_value;
                }
            }
        }
        w.ents[this].credits[i] = 0;
    }
    w.level.alien_credits += alien_c;
    w.level.human_credits += human_c;
    total
}

/// `player_die`.
pub fn player_die(w: &mut World, this: usize, _inflictor: usize, attacker: usize, _damage: i32, mod_: i32) {
    if w.clients[this].ps.pm_type == PmType::Dead as i32 {
        return;
    }
    w.clients[this].ps.pm_type = PmType::Dead as i32;
    for u in crate::gen::upgrades::UPGRADES.iter() {
        bg::deactivate_upgrade(u.num as i32, &mut w.clients[this].ps.stats);
    }
    // the obituary log line
    let killer = attacker as i32;
    let killer_name = if attacker < MAX_CLIENTS && w.ents[attacker].client { w.clients[attacker].name.clone() } else { "<world>".to_string() };
    let line = format!("Die: {killer} {this} {}: {killer_name}^7 killed {}\n", super::log::mod_name(mod_), w.clients[this].name);
    super::log::log_printf(w, &line);
    let origin = w.ents[this].origin;
    w.temp_event(origin, Event::Obituary, attacker as i32, mod_, this as i32);
    w.ents[this].enemy = Some(attacker);
    w.clients[this].ps.persistant[Pers::Killed as usize] += 1;
    w.clients[this].deaths += 1;
    if attacker < MAX_CLIENTS && w.ents[attacker].client {
        if attacker == this || w.on_same_team(this, attacker) {
            if mod_ != Mod::Hspawn as i32 {
                let pen = if w.clients[attacker].team == Team::Aliens { k::ALIEN_TK_SUICIDE_PENALTY } else { k::HUMAN_TK_SUICIDE_PENALTY };
                add_credit(w, attacker, -pen, true);
                add_score(w, attacker, -pen);
            }
        } else {
            w.clients[attacker].kills += 1;
        }
    } else if w.ents[attacker].etype != EType::Buildable {
        let pen = if w.clients[this].team == Team::Aliens { k::ALIEN_TK_SUICIDE_PENALTY } else { k::HUMAN_TK_SUICIDE_PENALTY };
        add_score(w, this, -pen);
    }
    reward_attackers(w, this);
    w.clients[this].last_death_location = origin;
    // s.angles = (0, yaw, 0); LookAtKiller; ps.viewangles = s.angles
    let inf = _inflictor;
    let c = &mut w.clients[this];
    c.ps.stats[Stat::Viewlock as usize] = if attacker != this { attacker as i32 } else if inf != this { inf as i32 } else { this as i32 };
    c.ps.viewangles = [0.0, c.ps.viewangles[YAW], 0.0];
    let e = &mut w.ents[this];
    e.takedamage = false;
    e.weapon = 0;
    e.contents = crate::gen::enums::contents::CORPSE as i32;
    e.maxs[2] = -8.0;
    let t = w.level.time;
    let c = &mut w.clients[this];
    c.respawn_time = t + 1700;
    c.ps.misc = [0; 16];
    let i = w.level.death_cycle;
    w.level.death_cycle = (i + 1) % 3;
    let c = &mut w.clients[this];
    let nonseg = c.ps.persistant[Pers::State as usize] & k::PS_NONSEGMODEL != 0;
    let anim = if !nonseg { [PlayerAnim::BothDeath1, PlayerAnim::BothDeath2, PlayerAnim::BothDeath3][i as usize] as i32 } else { [NonSegAnim::Death1, NonSegAnim::Death2, NonSegAnim::Death3][i as usize] as i32 };
    c.ps.legs_anim = ((c.ps.legs_anim & 0x80) ^ 0x80) | anim;
    if !nonseg {
        c.ps.torso_anim = ((c.ps.torso_anim & 0x80) ^ 0x80) | anim;
    }
    let ev = [Event::Death1, Event::Death2, Event::Death3][i as usize];
    w.add_event(this, ev, attacker as i32);
    let _ = UpgradeId::Medkit;
}
