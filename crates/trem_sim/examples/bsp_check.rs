//! Load real IBSP maps, report sizes and run a bot match: bsp_check <file.bsp>...
use trem_sim::gen::enums::{EntityKind, Team};
use trem_sim::kernel::{bot, bsp, world::World};
fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap();
        let b = match bsp::parse(&bytes) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}: ERROR {e}");
                continue;
            }
        };
        let mut w = World::from_bsp(&b, &path);
        let spots = |k: EntityKind| w.spots.iter().filter(|s| s.kind == k).count();
        println!("{path}: {} brushes {} surfaces {} shaders; spots buildable={} start={} inter={}; spawns a={} h={}",
            b.brushes.len(), b.surfaces.len(), b.shaders.len(), spots(EntityKind::Buildable), spots(EntityKind::PlayerStart), spots(EntityKind::Intermission), 0, 0);
        for k in 0..2 {
            bot::add_bot(&mut w, &format!("a{k}"), Team::Aliens);
            bot::add_bot(&mut w, &format!("h{k}"), Team::Humans);
        }
        let t0 = std::time::Instant::now();
        let nav = trem_sim::kernel::nav::Nav::generate(&w);
        let links: usize = nav.links.iter().map(|l| l.len()).sum();
        let om = w.overmind().map(|o| w.ents[o].origin);
        let re = w.reactor().map(|o| w.ents[o].origin);
        let conn = match (om, re) {
            (Some(a), Some(b)) => {
                let (na, nb) = (nav.nearest(&w, &a, false), nav.nearest(&w, &b, false));
                format!("{:?}->{:?} route {} nodes", na, nb, na.zip(nb).map(|(x, y)| nav.route(x, y).len()).unwrap_or(0))
            }
            _ => "no main structures".into(),
        };
        println!("   nav: {} nodes {} links in {:?}; overmind->reactor {}", nav.nodes.len(), links, t0.elapsed(), conn);
        let tele: Vec<String> = (64..w.num_entities).filter(|&i| w.ents[i].buildable == Some(trem_sim::gen::buildables::BuildableId::Telenode)).map(|i| {
            let e = &w.ents[i];
            format!("{:?} g={} sp={} {:?}", e.origin.map(|x| x as i32), e.ground_entity, e.spawned, trem_sim::kernel::buildable::check_spawn_point(&w, i as i32, e.origin, e.origin2, trem_sim::gen::buildables::BuildableId::Telenode).map(|v| v.map(|x| x as i32)))
        }).collect();
        println!("   telenodes: {tele:?}");
        let mut deaths = 0;
        for _ in 0..(30_000 / 16) {
            w.frame(16);
            for e in w.drain_events() {
                if e.event == trem_sim::gen::enums::Event::Obituary {
                    deaths += 1;
                    if std::env::var("OBIT").is_ok() {
                        println!("   t={} obit victim={} attacker={} mod={:?} at {:?}", w.level.time, e.weapon, e.other, trem_sim::gen::enums::MeansOfDeath::from_raw(e.parm as u8), e.origin.map(|x| x as i32));
                    }
                }
            }
        }
        let live: Vec<String> = (0..4).map(|i| format!("{:.0},{:.0},{:.0}", w.clients[i].ps.origin[0], w.clients[i].ps.origin[1], w.clients[i].ps.origin[2])).collect();
        println!("   after 30s: alien spawns {} human spawns {} live a={} h={} deaths={} pos={:?}", w.level.num_alien_spawns, w.level.num_human_spawns, w.level.num_live_alien_clients, w.level.num_live_human_clients, deaths, live);
    }
}
