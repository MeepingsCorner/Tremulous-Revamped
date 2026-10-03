//! MDD §7.5: the committed artifact is the hash of the C oracle's golden trace
//! (re/traces/<map>.sha256). The port must reproduce it byte for byte.
use sha2::{Digest, Sha256};

#[test]
fn pmove_golden_trace_matches_c_oracle() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let want = std::fs::read_to_string(root.join("re/traces/arena.sha256")).expect("re/traces/arena.sha256");
    let want = want.split_whitespace().next().unwrap().to_string();
    let trace = trem_sim::parity::dump_scenarios(trem_sim::gen::maps::MapId::Arena, None);
    let got = format!("{:x}", Sha256::digest(trace.as_bytes()));
    if got != want {
        let _ = std::fs::create_dir_all(root.join("re/traces/port"));
        let _ = std::fs::write(root.join("re/traces/port/arena.tsv"), &trace);
        panic!("golden trace diverged (port {got} != reference {want}); run `sheetty parity re/traces/ref/arena.tsv re/traces/port/arena.tsv --first-divergence`");
    }
}
