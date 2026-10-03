//! A command-line check of the port's networking (dec039), the same client
//! code the game runs, without Godot:
//!
//! - `cargo run -p trem_sim --example server_list -- lan`: the LAN scan
//!   (`localservers`), then a ping of each server found;
//! - `... -- master [host[:port]]`: the internet list from a master
//!   (default master.tremulous.net), then a ping of each listed server;
//!
//! (`net_probe` joins a server and plays.)
use std::time::{Duration, Instant};
use trem_sim::kernel::cl::{NetClient, AS_GLOBAL, AS_LOCAL};
use trem_sim::kernel::netchan::Net;

fn pump(cl: &mut NetClient, net: &mut Net, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        cl.frame(net, 16);
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn ping_all(cl: &mut NetClient, net: &mut Net, source: i32) {
    let list = if source == AS_LOCAL { &mut cl.local_servers } else { &mut cl.global_servers };
    for s in list.iter_mut() {
        s.visible = true;
    }
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        let pinging = cl.update_visible_pings(net, source);
        cl.frame(net, 16);
        std::thread::sleep(Duration::from_millis(16));
        if !pinging {
            break;
        }
    }
    let list = if source == AS_LOCAL { &cl.local_servers } else { &cl.global_servers };
    println!("{} server(s):", list.len());
    for s in list {
        let name: String = s.host_name.chars().filter(|c| !c.is_control()).collect();
        if s.ping > 0 {
            println!("  {:<24} ping {:>4}  {:>2}/{:<2}  {:<12} {}", s.adr.to_string_port(), s.ping, s.clients, s.max_clients, s.map_name, name);
        } else {
            println!("  {:<24} no compatible answer (protocol 71 / gamename Tremulous)", s.adr.to_string_port());
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut net = Net::new();
    let mut cl = NetClient::new(0);
    cl.userinfo = "\\name\\netprobe\\rate\\25000\\snaps\\20\\cl_guid\\5E1F0A2B3C4D5E6F708192A3B4C5D6E7".into();
    match args.first().map(String::as_str) {
        Some("lan") => {
            cl.local_servers(&mut net);
            pump(&mut cl, &mut net, 1500);
            ping_all(&mut cl, &mut net, AS_LOCAL);
        }
        Some("master") => {
            if let Some(m) = args.get(1) {
                cl.master_names[0] = m.clone();
            }
            cl.global_servers(&mut net, 0, "full empty").unwrap();
            pump(&mut cl, &mut net, 4000);
            if cl.num_global_servers < 0 {
                println!("no answer from the master {}", cl.master_names[0]);
            }
            ping_all(&mut cl, &mut net, AS_GLOBAL);
        }
        _ => {
            eprintln!("usage: server_list lan | master [host[:port]]");
            std::process::exit(2);
        }
    }
}
