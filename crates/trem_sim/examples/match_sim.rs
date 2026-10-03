//! Headless bot match: `cargo run --release -p trem_sim --example match_sim -- [seconds] [bots_per_team] [arena|<official map name>|path/to/x.bsp]...`.
//! Prints the measurements the `bots` definition of done checks (tests/bots.rs).
//! `bots_per_team` as `a2` / `h2` runs the objective scenario instead: two
//! alien (human) bots alone with full credits. `TREM_SEEDS=1,2,3` reruns
//! every map with those bot RNG seeds (`bot_match::seed`) and prints the
//! balance summary the `bots` definition of done checks.
#[path = "../tests/common/bot_match.rs"]
mod bot_match;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(120);
    let spec = args.get(2).cloned().unwrap_or_else(|| "3".into());
    let objective = match spec.chars().next() {
        Some('a') => Some(trem_sim::gen::enums::Team::Aliens),
        Some('h') => Some(trem_sim::gen::enums::Team::Humans),
        _ => None,
    };
    let n: usize = spec.trim_start_matches(['a', 'h']).parse().unwrap_or(3);
    let maps: Vec<String> = if args.len() > 3 { args[3..].to_vec() } else { vec!["arena".into()] };
    let seeds: Vec<Option<u32>> = match std::env::var("TREM_SEEDS") {
        Ok(s) => s.split(',').filter_map(|x| x.trim().parse().ok()).map(Some).collect(),
        Err(_) => vec![None],
    };
    let verbose = seeds.len() == 1;
    let mut all = Vec::new();
    for m in maps {
        for &seed in &seeds {
            let Some(mut w) = bot_match::load(&m) else {
                println!("{m}: not found");
                continue;
            };
            if let Some(s) = seed {
                bot_match::seed(&mut w, s);
            }
            let t0 = std::time::Instant::now();
            let r = match objective {
                Some(team) => bot_match::run_objective(&mut w, &m, team, n, secs * 1000, verbose),
                None => bot_match::run(&mut w, &m, n, secs * 1000, verbose),
            };
            println!("{} seed {:?} ({:.1}s wall)", r.line(), seed, t0.elapsed().as_secs_f32());
            if verbose {
                println!("  mods {:?}", r.mods);
            }
            all.push(r);
        }
    }
    if all.len() > 1 {
        println!("{}", bot_match::balance_summary(&all));
    }
}
