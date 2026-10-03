//! cm_query MAP.bsp N SEED OUTDIR: writes OUTDIR/queries.txt and the port's
//! answers OUTDIR/port.txt (same format as re/harness/cm_ref).
use trem_sim::{cmq, kernel::bsp};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bsp = bsp::parse(&std::fs::read(&a[1]).unwrap()).unwrap();
    let cm = bsp.clip_map();
    let n: usize = a[2].parse().unwrap();
    let seed: u64 = a[3].parse().unwrap();
    let q = cmq::queries(&cm, n, seed);
    let mut out = cmq::patch_lines(&cm);
    out.extend(q.iter().map(|l| cmq::answer(&cm, l)));
    std::fs::write(format!("{}/queries.txt", a[4]), q.join("\n") + "\n").unwrap();
    std::fs::write(format!("{}/port.txt", a[4]), out.join("\n") + "\n").unwrap();
}
