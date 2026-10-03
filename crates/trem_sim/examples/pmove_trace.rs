//! usage: cargo run --example pmove_trace -- <map> [scenario] > re/traces/port/<map>.tsv
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let map = trem_sim::gen::maps::MapId::from_id(args.first().map(|s| s.as_str()).unwrap_or("arena")).expect("unknown map");
    print!("{}", trem_sim::parity::dump_scenarios(map, args.get(1).map(|s| s.as_str())));
}
