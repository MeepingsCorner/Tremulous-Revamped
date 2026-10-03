//! Trace one bot's path until it falls (debugging aid): fall_trace MAP.bsp CLIENT
use trem_sim::gen::enums::Team;
use trem_sim::kernel::{bot, bsp, world::World};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = bsp::parse(&std::fs::read(&a[1]).unwrap()).unwrap();
    let watch: usize = a[2].parse().unwrap();
    let mut w = World::from_bsp(&b, "x");
    for k in 0..2 {
        bot::add_bot(&mut w, &format!("a{k}"), Team::Aliens);
        bot::add_bot(&mut w, &format!("h{k}"), Team::Humans);
    }
    let mut last = [0.0f32; 3];
    for step in 0..(8000 / 16) {
        w.frame(16);
        let ps = &w.clients[watch].ps;
        if step % 10 == 0 || ps.origin[2] < -900.0 {
            let o = ps.origin;
            if (o[0] - last[0]).abs() + (o[1] - last[1]).abs() + (o[2] - last[2]).abs() > 1.0 {
                println!("t={} o={:?} v={:?} ground={} class={} pmt={} cmd f{} r{} u{}", w.level.time, o.map(|x| x as i32), ps.velocity.map(|x| x as i32), ps.ground_entity_num, ps.stats[0], ps.pm_type, w.clients[watch].cmd.forwardmove, w.clients[watch].cmd.rightmove, w.clients[watch].cmd.upmove);
                last = o;
            }
            if ps.origin[2] < -1500.0 {
                break;
            }
        }
    }
}
