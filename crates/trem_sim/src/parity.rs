//! Golden-trace driver (MDD §7.5), port side. Glue, not a kernel: it only
//! sets up player states exactly like re/harness/pmove_ref.c and dumps the
//! same (tick, entity, key, value) relation.
use crate::cm::{self, ClipMap, Trace};
use crate::gen::maps::MapId;
use crate::gen::scenarios::SCENARIOS;
use crate::pmove::Pmove;
use crate::qmath::*;
use crate::rng::Rng;
use crate::types::*;
use crate::{bg, gen};

struct W(ClipMap);
impl PmWorld for W {
    fn trace(&self, s: &Vec3, mins: Option<&Vec3>, maxs: Option<&Vec3>, e: &Vec3, _pass: i32, mask: i32) -> Trace {
        let mut t = self.0.box_trace(s, e, mins, maxs, mask);
        t.entity_num = if t.fraction != 1.0 { ENTITYNUM_WORLD } else { ENTITYNUM_NONE };
        t
    }
    fn point_contents(&self, p: &Vec3, _pass: i32) -> i32 {
        self.0.point_contents(p)
    }
}

fn hx(v: &Vec3) -> String {
    format!("{:08x} {:08x} {:08x}", v[0].to_bits(), v[1].to_bits(), v[2].to_bits())
}

/// Run every scenarios.tsv row (or one) on `map` and return the golden-trace TSV.
pub fn dump_scenarios(map: MapId, only: Option<&str>) -> String {
    dump_scenarios_on(cm::from_sheet(map), only)
}

/// Same, on any clip map (e.g. the arena round-tripped through an IBSP file).
pub fn dump_scenarios_on(clip: ClipMap, only: Option<&str>) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let w = W(clip);
    out.push_str("tick\tentity\tkey\tvalue\n");
    for sc in SCENARIOS.iter() {
        if only.map(|o| o != sc.id).unwrap_or(false) {
            continue;
        }
        let mut rng = Rng::default();
        let class = sc.class;
        let cd = class.def();
        let mut ps = PlayerState::default();
        let mut pmext = PmoveExt::default();
        let mut cmd = UserCmd::default();
        ps.stats[gen::enums::Stat::Class as usize] = class.num() as i32;
        ps.stats[gen::enums::Stat::Team as usize] = cd.team as i32;
        let hp = if cd.health != 0 { cd.health } else { 100 };
        ps.stats[gen::enums::Stat::Health as usize] = hp;
        ps.stats[gen::enums::Stat::MaxHealth as usize] = hp;
        ps.stats[gen::enums::Stat::Stamina as usize] = k::STAMINA_MAX;
        ps.speed = (320.0f32 * cd.speed) as i32;
        ps.gravity = k::DEFAULT_GRAVITY;
        ps.viewheight = cd.viewheight;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.grapple_point = [0.0, 0.0, 1.0];
        ps.pm_type = sc.pm_type as i32;
        ps.persistant[gen::enums::Pers::State as usize] = sc.ps_state as i32;
        if let Some(wp) = sc.weapon {
            let wd = wp.def();
            ps.weapon = wp.num() as i32;
            ps.stats[gen::enums::Stat::Weapon as usize] = ps.weapon;
            ps.ammo = wd.max_ammo;
            ps.clips = wd.max_clips;
        }
        for u in sc.items {
            bg::add_upgrade(u.num() as i32, &mut ps.stats);
        }
        ps.origin = sc.origin;
        cmd.angles[YAW] = angle2short_f(sc.yaw);
        
        let mut tick = 0;
        for tok in sc.script.split(' ') {
            let (n, rest) = tok.split_once('*').expect("script token");
            let n: i32 = n.parse().unwrap();
            let (mut f, mut r, mut u, mut b, mut y, mut p, mut wsel) = (0, 0, 0, 0, 0, 0, 0);
            let bytes = rest.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let kc = bytes[i] as char;
                i += 1;
                let st = i;
                if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let v: i32 = rest[st..i].parse().unwrap();
                match kc {
                    'f' => f = v,
                    'r' => r = v,
                    'u' => u = v,
                    'b' => b = v,
                    'y' => y = v,
                    'p' => p = v,
                    'w' => wsel = v,
                    _ => panic!("bad script key {kc}"),
                }
            }
            for _ in 0..n {
                tick += 1;
                cmd.server_time += sc.msec as i32;
                cmd.forwardmove = f as i8;
                cmd.rightmove = r as i8;
                cmd.upmove = u as i8;
                cmd.buttons = b;
                cmd.weapon = wsel as u8;
                cmd.angles[YAW] += y;
                cmd.angles[PITCH] += p;
                let mask = if ps.stats[gen::enums::Stat::Health as usize] > 0 { k::MASK_PLAYERSOLID } else { k::MASK_DEADSOLID };
                let numtouch;
                let waterlevel;
                {
                    let mut pm = Pmove::new(&mut ps, &mut pmext, cmd, &w, &mut rng);
                    pm.tracemask = mask;
                    pm.pmove();
                    numtouch = pm.numtouch;
                    waterlevel = pm.waterlevel;
                }
                let id = sc.id;
                let _ = writeln!(out, "{tick}\t{id}\torigin\t{}", hx(&ps.origin));
                let _ = writeln!(out, "{tick}\t{id}\tvelocity\t{}", hx(&ps.velocity));
                let _ = writeln!(out, "{tick}\t{id}\tviewangles\t{}", hx(&ps.viewangles));
                let _ = writeln!(out, "{tick}\t{id}\tgrapple\t{}", hx(&ps.grapple_point));
                let _ = writeln!(out, "{tick}\t{id}\tdelta_angles\t{} {} {}", ps.delta_angles[0], ps.delta_angles[1], ps.delta_angles[2]);
                let s = |x: gen::enums::Stat| ps.stats[x as usize];
                use gen::enums::Stat as S;
                for (key, v) in [
                    ("pm_flags", ps.pm_flags), ("pm_time", ps.pm_time), ("ground", ps.ground_entity_num), ("eflags", ps.e_flags),
                    ("stat_state", s(S::State)), ("stat_misc", s(S::Misc)), ("stamina", s(S::Stamina)), ("falldist", s(S::Falldist)),
                    ("weapon", ps.weapon), ("weapon_time", ps.weapon_time), ("weaponstate", ps.weaponstate), ("ammo", ps.ammo), ("clips", ps.clips),
                    ("legs_anim", ps.legs_anim), ("torso_anim", ps.torso_anim), ("weapon_anim", ps.weapon_anim),
                ] {
                    let _ = writeln!(out, "{tick}\t{id}\t{key}\t{v}");
                }
                let _ = writeln!(out, "{tick}\t{id}\tevents\t{} {} {}", ps.event_sequence, ps.events[0], ps.events[1]);
                for (key, v) in [
                    ("bob_cycle", ps.bob_cycle), ("viewheight", ps.viewheight), ("movement_dir", ps.movement_dir), ("generic1", ps.generic1),
                    ("pounce_payload", pmext.pounce_payload), ("touch", numtouch as i32), ("waterlevel", waterlevel),
                ] {
                    let _ = writeln!(out, "{tick}\t{id}\t{key}\t{v}");
                }
            }
        }
    }
    out
}

/// The Tremulous 1.1.0 golden trace (dec041): every scenarios11.tsv row (or
/// one) through `kernel::pmove11`, set up and dumped exactly like
/// re/harness/pmove11_ref.c.
pub fn dump_scenarios11(map: MapId, only: Option<&str>) -> String {
    use crate::gen::constants11 as k11;
    use crate::gen::enums::{Pers11, Stat11};
    use crate::kernel::{bg11, pmove11::Pmove11, v11::PlayerState11};
    use std::fmt::Write as _;
    let w = W(cm::from_sheet(map));
    let mut out = String::new();
    out.push_str("tick\tentity\tkey\tvalue\n");
    let st = |s: Stat11| s as usize;
    for sc in crate::gen::scenarios11::SCENARIOS11.iter() {
        if only.map(|o| o != sc.id).unwrap_or(false) {
            continue;
        }
        let mut rng = Rng::default();
        let pclass = sc.class.num() as i32;
        let mut ps = PlayerState11::default();
        let mut cmd = UserCmd::default();
        ps.stats[st(Stat11::Pclass)] = pclass;
        ps.stats[st(Stat11::Pteam)] = if pclass >= crate::gen::classes11::Class11Id::HumanBase.num() as i32 { 2 } else if pclass == 0 { 0 } else { 1 };
        ps.persistant[Pers11::Team as usize] = if pclass == 0 { 1 } else { 0 };
        let health = bg11::health(pclass);
        let hp = if health != 0 { health } else { 100 };
        ps.stats[st(Stat11::Health)] = hp;
        ps.stats[st(Stat11::MaxHealth)] = hp;
        ps.stats[st(Stat11::Stamina)] = k11::MAX_STAMINA;
        ps.speed = (320.0f32 * bg11::speed(pclass)) as i32;
        ps.gravity = k11::DEFAULT_GRAVITY;
        ps.viewheight = bg11::class_viewheight(pclass).0;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.grapple_point = [0.0, 0.0, 1.0];
        ps.pm_type = sc.pm_type as i32;
        ps.persistant[Pers11::State as usize] = sc.ps_state as i32 & (k11::PS_NONSEGMODEL | k11::PS_WALLCLIMBINGTOGGLE | k11::PS_WALLCLIMBINGFOLLOW);
        if ps.stats[st(Stat11::Pteam)] == 2 {
            bg11::add_weapon_to_inventory(crate::gen::weapons11::Weapon11Id::Blaster as i32, &mut ps.stats);
        }
        if let Some(wp) = sc.weapon {
            let n = wp.num() as i32;
            bg11::add_weapon_to_inventory(n, &mut ps.stats);
            ps.weapon = n;
            let (ma, mc) = bg11::ammo_for_weapon(n).unwrap_or((0, 0));
            bg11::pack_ammo_array(n, &mut ps.ammo, &mut ps.powerups, ma, mc);
        }
        for u in sc.items {
            bg11::add_upgrade_to_inventory(u.num() as i32, &mut ps.stats);
        }
        ps.origin = sc.origin;
        cmd.angles[YAW] = angle2short_f(sc.yaw);
        let mut tick = 0;
        for tok in sc.script.split(' ') {
            let (n, rest) = tok.split_once('*').expect("script token");
            let n: i32 = n.parse().unwrap();
            let (mut f, mut r, mut u, mut b, mut y, mut p, mut wsel) = (0, 0, 0, 0, 0, 0, 0);
            let (mut misc, mut sor, mut sclr) = (None, 0, 0);
            let bytes = rest.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let kc = bytes[i] as char;
                i += 1;
                let st0 = i;
                if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let v: i32 = rest[st0..i].parse().unwrap();
                match kc {
                    'f' => f = v,
                    'r' => r = v,
                    'u' => u = v,
                    'b' => b = v,
                    'y' => y = v,
                    'p' => p = v,
                    'w' => wsel = v,
                    'm' => misc = Some(v),
                    's' => sor = v,
                    'c' => sclr = v,
                    _ => panic!("bad script key {kc}"),
                }
            }
            for _ in 0..n {
                tick += 1;
                if let Some(m) = misc {
                    ps.stats[st(Stat11::Misc)] = m;
                }
                ps.stats[st(Stat11::State)] |= sor;
                ps.stats[st(Stat11::State)] &= !sclr;
                cmd.server_time += sc.msec as i32;
                cmd.forwardmove = f as i8;
                cmd.rightmove = r as i8;
                cmd.upmove = u as i8;
                cmd.buttons = b;
                cmd.weapon = wsel as u8;
                cmd.angles[YAW] += y;
                cmd.angles[PITCH] += p;
                let mask = if ps.stats[st(Stat11::Health)] > 0 { k11::MASK_PLAYERSOLID } else { k11::MASK_DEADSOLID };
                let (numtouch, waterlevel);
                {
                    let mut pm = Pmove11::new(&mut ps, cmd, &w, &mut rng);
                    pm.tracemask = mask;
                    pm.pmove();
                    numtouch = pm.numtouch;
                    waterlevel = pm.waterlevel;
                }
                let id = sc.id;
                let _ = writeln!(out, "{tick}\t{id}\torigin\t{}", hx(&ps.origin));
                let _ = writeln!(out, "{tick}\t{id}\tvelocity\t{}", hx(&ps.velocity));
                let _ = writeln!(out, "{tick}\t{id}\tviewangles\t{}", hx(&ps.viewangles));
                let _ = writeln!(out, "{tick}\t{id}\tgrapple\t{}", hx(&ps.grapple_point));
                let _ = writeln!(out, "{tick}\t{id}\tdelta_angles\t{} {} {}", ps.delta_angles[0], ps.delta_angles[1], ps.delta_angles[2]);
                let s = |x: Stat11| ps.stats[x as usize];
                let (ammo, clips) = bg11::unpack_ammo_array(ps.weapon, &ps.ammo, &ps.powerups);
                for (key, v) in [
                    ("pm_flags", ps.pm_flags), ("pm_time", ps.pm_time), ("ground", ps.ground_entity_num), ("eflags", ps.e_flags),
                    ("stat_state", s(Stat11::State)), ("stat_misc", s(Stat11::Misc)), ("stamina", s(Stat11::Stamina)), ("falldist", s(Stat11::Falldist)),
                    ("activeitems", s(Stat11::Activeitems)), ("buildable", s(Stat11::Buildable)),
                    ("weapon", ps.weapon), ("weapon_time", ps.weapon_time), ("weaponstate", ps.weaponstate), ("ammo", ammo), ("clips", clips),
                    ("newweapon", ps.persistant[Pers11::Newweapon as usize]),
                    ("legs_anim", ps.legs_anim), ("torso_anim", ps.torso_anim), ("legs_timer", ps.legs_timer), ("torso_timer", ps.torso_timer),
                ] {
                    let _ = writeln!(out, "{tick}\t{id}\t{key}\t{v}");
                }
                let _ = writeln!(out, "{tick}\t{id}\tevents\t{} {} {}", ps.event_sequence, ps.events[0], ps.events[1]);
                for (key, v) in [
                    ("bob_cycle", ps.bob_cycle), ("viewheight", ps.viewheight), ("movement_dir", ps.movement_dir), ("generic1", ps.generic1),
                    ("touch", numtouch as i32), ("waterlevel", waterlevel),
                ] {
                    let _ = writeln!(out, "{tick}\t{id}\t{key}\t{v}");
                }
            }
        }
    }
    out
}
