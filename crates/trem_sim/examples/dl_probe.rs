//! A real-server download check: connects to a server with a fresh
//! fs_homepath (so every pak the server references beyond the given base is
//! downloaded), waits for the gamestate, then disconnects without joining a
//! team. Prints what was downloaded and how long it took.
//! `cargo run --release -p trem_sim --example dl_probe -- HOST:PORT [--base=DIR] [--secs=N]`

use std::path::PathBuf;
use std::time::{Duration, Instant};
use trem_sim::kernel::cl::{ConnState, NetClient};
use trem_sim::kernel::files::PakSet;
use trem_sim::kernel::netchan::{Net, NetSrc};
use trem_sim::kernel::types::UserCmd;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let server = args.first().cloned().expect("HOST:PORT");
    let base = args.iter().find_map(|a| a.strip_prefix("--base=")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Tremulous"));
    let secs: u64 = args.iter().find_map(|a| a.strip_prefix("--secs=")).and_then(|s| s.parse().ok()).unwrap_or(120);
    let home = std::env::temp_dir().join(format!("trem_dlprobe_{}", std::process::id()));
    std::fs::create_dir_all(home.join("base")).unwrap();
    let mut net = Net::new();
    net.open(NetSrc::Client, "0.0.0.0", 0).unwrap();
    let mut cl = NetClient::new(0);
    cl.userinfo = "\\name\\DownloadProbe\\rate\\25000\\snaps\\20".into();
    cl.allow_download = 1;
    cl.home_path = Some(home.clone());
    cl.fs = Some(PakSet::new(&[(base.join("base"), "base".into()), (home.join("base"), "base".into())]));
    cl.connect(&mut net, &server).unwrap();
    let t0 = Instant::now();
    let mut last = t0;
    let mut state = cl.state;
    let mut dl_start: Option<Instant> = None;
    let mut printed = 0;
    while t0.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(2));
        let now = Instant::now();
        let msec = (now - last).as_millis() as i32;
        if msec == 0 {
            continue;
        }
        last = now;
        cl.download_prompt_frame();
        cl.frame(&mut net, msec);
        if cl.gamestate_new {
            cl.cgame_loaded(&mut net);
        }
        cl.create_cmd(UserCmd::default());
        cl.send_cmd(&mut net);
        if dl_start.is_none() && cl.download.as_ref().is_some_and(|d| d.block > 0) {
            dl_start = Some(now);
        }
        if cl.print_out.len() > printed {
            print!("{}", &cl.print_out[printed..]);
            printed = cl.print_out.len();
        }
        if cl.state != state {
            println!("[{:>6} ms] {:?}", t0.elapsed().as_millis(), cl.state);
            state = cl.state;
        }
        if let Some(e) = cl.error.as_ref() {
            println!("error: {e}");
            break;
        }
        if cl.state == ConnState::Active {
            break;
        }
    }
    for e in std::fs::read_dir(home.join("base")).unwrap().flatten() {
        println!("downloaded {} ({} bytes)", e.file_name().to_string_lossy(), e.metadata().map(|m| m.len()).unwrap_or(0));
    }
    if let Some(s) = dl_start {
        println!("from the first block to the end: {:?}", s.elapsed());
    }
    println!("total {:?}, protocol {}", t0.elapsed(), cl.protocol);
    cl.disconnect(&mut net);
    std::thread::sleep(Duration::from_millis(200));
    let _ = std::fs::remove_dir_all(&home);
}
