//! A headless network client (dec033): connects to any protocol 71 server
//! (this port's or an original tremded), joins the humans with a rifle,
//! walks, fires, and prints what its view World holds.
//! `cargo run --release -p trem_sim --example net_probe -- 127.0.0.1:30720 [seconds] [--base=DIR]`

#[path = "../tests/common/bot_match.rs"]
#[allow(dead_code)]
mod bot_match;

use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use trem_sim::gen::constants as k;
use trem_sim::gen::enums::Stat;
use trem_sim::kernel::cl::ConnState;
use trem_sim::kernel::nethost::{HostEvent, NetHost};
use trem_sim::kernel::types::UserCmd;
use trem_sim::kernel::world::{EType, World};
use trem_sim::kernel::{bsp, msg};

fn map_view(base: &PathBuf, name: &str) -> Option<World> {
    let mut pk3s: Vec<PathBuf> = std::fs::read_dir(base).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "pk3")).collect();
    pk3s.sort();
    for p in pk3s.iter().rev() {
        let Ok(f) = std::fs::File::open(p) else { continue };
        let Ok(mut z) = zip::ZipArchive::new(f) else { continue };
        let mut v = Vec::new();
        let ok = match z.by_name(&format!("maps/{name}.bsp")) {
            Ok(mut e) => e.read_to_end(&mut v).is_ok(),
            Err(_) => false,
        };
        if ok {
            return Some(World::client_view_from_bsp(&bsp::parse(&v).ok()?, name));
        }
    }
    if name == "arena" {
        return Some(World::client_view(trem_sim::gen::maps::MapId::Arena));
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let server = args.first().cloned().unwrap_or_else(|| "127.0.0.1:30720".into());
    let secs: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let base = args.iter().find_map(|a| a.strip_prefix("--base=")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../godot/base"));
    let mut h = NetHost::new();
    // the search path for sv_pure servers' checks (CL_SendPureChecksums)
    h.cl.fs = Some(trem_sim::kernel::files::PakSet::new(&[(base.clone(), "base".into())]));
    h.cl.userinfo = "\\name\\RustProbe\\rate\\25000\\snaps\\20\\cg_unlagged\\1\\teamoverlay\\1".into();
    h.cl.qkey = Some((0..2048u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect());
    if let Err(e) = h.connect(&server) {
        eprintln!("{e}");
        return;
    }
    let mut view: Option<World> = None;
    let t0 = Instant::now();
    let mut active_at: Option<f32> = None;
    let mut last_print = 0.0f32;
    let mut cmd = UserCmd::default();
    let mut step = 0;
    let mut fired = 0;
    loop {
        let t = t0.elapsed().as_secs_f32();
        if t > secs {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
        let send = h.client_num().map(|_| cmd);
        for e in h.frame(16, view.as_mut(), send) {
            match e {
                HostEvent::LoadView { map } => {
                    println!("[{t:5.1}] gamestate: map {map}, clientNum {}", h.cl.client_num);
                    view = map_view(&base, &map);
                    if view.is_none() {
                        println!("map {map} not found in {}", base.display());
                        return;
                    }
                    h.view_loaded();
                }
                HostEvent::Dropped { reason } => {
                    println!("[{t:5.1}] dropped: {reason}");
                    return;
                }
                HostEvent::Rcon { .. } | HostEvent::FsRestart | HostEvent::DemoCompleted => {}
            }
        }
        for l in std::mem::take(&mut h.out).lines() {
            println!("[{t:5.1}] {l}");
        }
        if h.cl.state == ConnState::Active && active_at.is_none() {
            active_at = Some(t);
            println!("[{t:5.1}] active (serverTime {})", h.cl.server_time);
        }
        let Some(at) = active_at else { continue };
        let v = view.as_mut().unwrap();
        let me = h.cl.client_num as usize;
        fired += v.drain_events().iter().filter(|e| e.event == trem_sim::gen::enums::Event::FireWeapon && e.entity == me as i32).count();
        let ps = v.clients[me].ps;
        let alive = ps.stats[Stat::Health as usize] > 0 && ps.persistant[trem_sim::gen::enums::Pers::Specstate as usize] == 0;
        match step {
            0 if t - at > 1.0 => {
                h.cl.add_reliable_command("team humans", false).unwrap();
                step = 1;
            }
            1 if t - at > 2.5 => {
                h.cl.add_reliable_command("class rifle", false).unwrap();
                step = 2;
            }
            2 if alive => {
                println!("[{t:5.1}] spawned at {:?}", ps.origin);
                cmd.forwardmove = 127;
                step = 3;
            }
            3 if ps.stats[Stat::Health as usize] > 0 && t - at > 7.0 => {
                cmd.forwardmove = 0;
                // turn round and fire
                cmd.angles[1] = msg::angle2short(180.0) - ps.delta_angles[1];
                cmd.buttons = k::BUTTON_ATTACK;
                step = 4;
            }
            4 if t - at > 9.0 => {
                cmd.buttons = 0;
                step = 5;
            }
            _ => {}
        }
        if t - last_print > 2.0 {
            last_print = t;
            let ents = (0..v.ents.len()).filter(|&i| v.ents[i].inuse);
            let (mut players, mut buildables, mut other) = (0, 0, 0);
            for i in ents {
                match v.ents[i].etype {
                    EType::Player => players += 1,
                    EType::Buildable => buildables += 1,
                    _ => other += 1,
                }
            }
            let misses = h.cg.as_ref().map(|c| c.misses.len()).unwrap_or(0);
            let names: Vec<String> = v.clients.iter().filter(|c| c.connected).map(|c| c.name.clone()).collect();
            println!(
                "[{t:5.1}] snap {} ping {} | me {} hp {} origin {:.0},{:.0},{:.0} weapon {} ammo {} | ents: players {players} buildables {buildables} other {other} | clients {names:?} | stages A{} H{} | prediction misses {misses}, fire events {fired}",
                h.cl.snap.server_time, h.cl.snap.ping, v.clients[me].name, ps.stats[Stat::Health as usize], ps.origin[0], ps.origin[1], ps.origin[2], ps.weapon, ps.ammo, v.level.alien_stage, v.level.human_stage
            );
        }
    }
    h.disconnect();
    for _ in 0..5 {
        h.frame(16, view.as_mut(), None);
        std::thread::sleep(Duration::from_millis(16));
    }
    println!("disconnected");
}
