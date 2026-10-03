//! Pure servers and pk3 downloads over localhost UDP (dec037): a client
//! lacking the server's map pk3 downloads it (CL_InitDownloads /
//! CL_NextDownload / CL_ParseDownload against SV_WriteDownloadToClient's
//! MAX_DOWNLOAD_BLKSIZE blocks), writes it into its fs_homepath, restarts its search
//! path, gets the gamestate again and passes SV_VerifyPaks_f; the download
//! prompt and cl_allowDownload 0; a forged "cp" is refused as the C does.
use std::io::Write;
use std::path::{Path, PathBuf};
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::cgview::Cgame;
use trem_sim::kernel::cl::{ConnState, NetClient, DLP_SHOW, DLP_UDP};
use trem_sim::kernel::files::PakSet;
use trem_sim::kernel::netchan::{Net, NetSrc};
use trem_sim::kernel::sv::{ClientState, Server, ServerCvars, MAX_DOWNLOAD_BLKSIZE};
use trem_sim::kernel::types::UserCmd;
use trem_sim::kernel::world::World;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("trem_netfiles_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A stored (uncompressed) pk3 with these files.
fn write_pk3(path: &Path, files: &[(&str, Vec<u8>)]) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (n, d) in files {
        z.start_file(*n, o).unwrap();
        z.write_all(d).unwrap();
    }
    z.finish().unwrap();
}

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            (s >> 16) as u8
        })
        .collect()
}

struct Setup {
    srv_base: PathBuf,
    cli_base: PathBuf,
    cli_home: PathBuf,
    map_pk3: PathBuf,
}

/// The server has the data pk3 and the map pk3; the client only the data.
fn setup(tag: &str, map_size: usize) -> Setup {
    let root = tmp(tag);
    let (srv_base, cli_base, cli_home) = (root.join("srv/base"), root.join("cli/base"), root.join("home"));
    for d in [&srv_base, &cli_base, &cli_home] {
        std::fs::create_dir_all(d).unwrap();
    }
    let data = [("ui/hud.txt", b"hud".to_vec()), ("ui/menus.txt", b"menus".to_vec()), ("models/x.md3", noise(3000, 1))];
    write_pk3(&srv_base.join("data-test.pk3"), &data);
    write_pk3(&cli_base.join("data-test.pk3"), &data);
    let map_pk3 = srv_base.join("map-dltest-1.pk3");
    write_pk3(&map_pk3, &[("maps/arena.bsp", noise(map_size, 7)), ("textures/dl/a.tga", noise(500, 9))]);
    Setup { srv_base, cli_base, cli_home, map_pk3 }
}

fn spawn(s: &Setup, allow_download: i32) -> (Server, Net, u16) {
    let mut net = Net::new();
    let port = net.open(NetSrc::Server, "127.0.0.1", 0).unwrap();
    let fs = PakSet::new(&[(s.srv_base.clone(), "base".into())]);
    let cvars = ServerCvars { hostname: "pure".into(), allow_download, pure: 1, ..Default::default() };
    let sv = Server::spawn_fs(World::new(MapId::Arena), "arena", cvars, Some(fs));
    (sv, net, port)
}

struct Client {
    net: Net,
    cl: NetClient,
    cg: Option<Cgame>,
    view: Option<World>,
}

fn client(s: &Setup, port: u16, allow_download: i32) -> Client {
    let mut net = Net::new();
    net.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
    let mut cl = NetClient::new(0);
    cl.userinfo = "\\name\\Downloader\\cl_guid\\0123456789ABCDEF0123456789ABCDEF".into();
    cl.allow_download = allow_download;
    cl.home_path = Some(s.cli_home.clone());
    // basepath, then fs_homepath (the downloads), as FS_Startup adds them
    cl.fs = Some(PakSet::new(&[(s.cli_base.clone(), "base".into()), (s.cli_home.join("base"), "base".into())]));
    cl.connect(&mut net, &format!("127.0.0.1:{port}")).unwrap();
    Client { net, cl, cg: None, view: None }
}

fn step(sv: &mut Server, snet: &mut Net, c: &mut Client) {
    sv.frame(snet, 16);
    c.cl.download_prompt_frame();
    c.cl.frame(&mut c.net, 16);
    if c.cl.gamestate_new {
        c.view = Some(World::client_view(MapId::Arena));
        c.cg = Some(Cgame::init(&c.cl));
        c.cl.cgame_loaded(&mut c.net);
    }
    if let (Some(cg), Some(v)) = (c.cg.as_mut(), c.view.as_mut()) {
        if c.cl.state == ConnState::Active {
            cg.frame(&mut c.cl, v);
        }
    }
    c.cl.create_cmd(UserCmd::default());
    c.cl.send_cmd(&mut c.net);
    std::thread::sleep(std::time::Duration::from_micros(150));
}

#[test]
fn client_downloads_the_missing_map_pk3() {
    let s = setup("dl", 70_000);
    let (mut sv, mut snet, port) = spawn(&s, 1);
    // what the server tells clients: pure, its paks, the referenced ones
    let si = sv.systeminfo();
    let v = |k: &str| trem_sim::kernel::svgame::info_value(&si, k);
    assert_eq!(v("sv_pure"), "1");
    assert_eq!(v("sv_pakNames"), "map-dltest-1 data-test");
    assert!(v("sv_referencedPakNames").contains("base/map-dltest-1"), "{si}");
    let mut c = client(&s, port, 1);
    let mut blocks_seen = 0;
    let mut restarted = false;
    for _ in 0..4000 {
        step(&mut sv, &mut snet, &mut c);
        if let Some(d) = sv.clients.iter().map(|c| c.download.current_block).max() {
            blocks_seen = blocks_seen.max(d);
        }
        restarted |= c.cl.fs_restarted;
        c.cl.fs_restarted = false;
        if c.cl.state == ConnState::Active && sv.active_clients() == 1 {
            break;
        }
    }
    assert_eq!(c.cl.state, ConnState::Active, "{:?}\n{}\n{}", c.cl.error, c.cl.print_out, sv.console);
    // the file arrived whole in fs_homepath/base, in MAX_DOWNLOAD_BLKSIZE blocks + EOF
    let got = std::fs::read(s.cli_home.join("base/map-dltest-1.pk3")).unwrap();
    let want = std::fs::read(&s.map_pk3).unwrap();
    assert_eq!(got, want);
    assert!(!s.cli_home.join("base/map-dltest-1.pk3.tmp").exists());
    assert_eq!(blocks_seen, want.len().div_ceil(MAX_DOWNLOAD_BLKSIZE) + 1, "the server's blocks: data + the EOF block");
    assert!(restarted, "FS_Restart after the download");
    assert!(c.cl.print_out.contains("Need paks: @base/map-dltest-1.pk3@base/map-dltest-1.pk3"), "{}", c.cl.print_out);
    assert!(sv.console.contains("clientDownload: 0 : beginning \"base/map-dltest-1.pk3\""), "{}", sv.console);
    // the search path holds it now, and the pure check passed
    assert!(c.cl.fs.as_ref().unwrap().paks.iter().any(|p| p.basename == "map-dltest-1"));
    let i = c.cl.client_num as usize;
    assert!(sv.clients[i].pure_authentic && sv.clients[i].got_cp);
    let _ = std::fs::remove_dir_all(s.srv_base.parent().unwrap().parent().unwrap());
}

/// A UDP relay between one client and the server that drops `loss_pct` of
/// the datagrams each way (a fixed LCG, so runs repeat) and delays the rest
/// by `delay_ms` each way; returns its port.
fn lossy_relay(server_port: u16, loss_pct: u32, delay_ms: u64) -> (u16, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::net::{SocketAddr, UdpSocket};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    let front = UdpSocket::bind("127.0.0.1:0").unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = front.local_addr().unwrap().port();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    front.set_nonblocking(true).unwrap();
    back.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let server: SocketAddr = format!("127.0.0.1:{server_port}").parse().unwrap();
        let mut client: Option<SocketAddr> = None;
        let mut s = 0x1234_5678u32;
        let mut drop_it = || {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            (s >> 16) % 100 < loss_pct
        };
        // (due, toward the server, datagram)
        let mut queue: std::collections::VecDeque<(Instant, bool, Vec<u8>)> = Default::default();
        let delay = Duration::from_millis(delay_ms);
        let mut buf = vec![0u8; 65536];
        while !stop2.load(Ordering::Relaxed) {
            while let Ok((n, from)) = front.recv_from(&mut buf) {
                client = Some(from);
                if !drop_it() {
                    queue.push_back((Instant::now() + delay, true, buf[..n].to_vec()));
                }
            }
            while let Ok((n, _)) = back.recv_from(&mut buf) {
                if !drop_it() {
                    queue.push_back((Instant::now() + delay, false, buf[..n].to_vec()));
                }
            }
            let now = Instant::now();
            while queue.front().is_some_and(|q| q.0 <= now) {
                let (_, up, d) = queue.pop_front().unwrap();
                if up {
                    let _ = back.send_to(&d, server);
                } else if let Some(c) = client {
                    let _ = front.send_to(&d, c);
                }
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    });
    (port, stop)
}

#[test]
fn download_survives_packet_loss() {
    // burst sending puts a whole window of blocks on the wire at once; with
    // 10% of the datagrams lost each way and a 40 ms round trip the resends
    // must still finish it, in real time (LOSS / DELAY override)
    let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (loss, delay) = (env("LOSS", 10) as u32, env("DELAY", 20));
    let s = setup("lossy", 600_000);
    let (mut sv, mut snet, port) = spawn(&s, 1);
    let (relay, stop) = lossy_relay(port, loss, delay);
    let mut c = client(&s, relay, 1);
    let t0 = std::time::Instant::now();
    let mut last = t0;
    while t0.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        let now = std::time::Instant::now();
        let msec = (now - last).as_millis() as i32;
        if msec == 0 {
            continue;
        }
        last = now;
        sv.frame(&mut snet, msec);
        c.cl.frame(&mut c.net, msec);
        if c.cl.gamestate_new {
            c.cl.cgame_loaded(&mut c.net);
        }
        c.cl.create_cmd(UserCmd::default());
        c.cl.send_cmd(&mut c.net);
        if c.cl.state == ConnState::Active && sv.active_clients() == 1 {
            break;
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let svs: Vec<_> = sv.clients.iter().filter(|x| x.state != ClientState::Free).map(|x| (x.state, x.download.name.clone(), x.download.client_block)).collect();
    assert_eq!(c.cl.state, ConnState::Active, "{:?}\nserver clients {svs:?}\n{}\n{}", c.cl.error, c.cl.print_out, sv.console);
    let got = std::fs::read(s.cli_home.join("base/map-dltest-1.pk3")).unwrap();
    assert_eq!(got, std::fs::read(&s.map_pk3).unwrap(), "the pak arrived intact");
    eprintln!("{} bytes through {loss}% loss each way, {delay} ms each way, in {:?}", got.len(), t0.elapsed());
    let _ = std::fs::remove_dir_all(s.srv_base.parent().unwrap().parent().unwrap());
}

#[test]
fn download_prompt_and_disabled_downloads() {
    let s = setup("prompt", 5_000);
    let (mut sv, mut snet, port) = spawn(&s, 1);
    // cl_allowDownload 0: CL_NextDownload asks the ui (com_downloadPrompt)
    let mut c = client(&s, port, 0);
    for _ in 0..600 {
        sv.frame(&mut snet, 16);
        c.cl.frame(&mut c.net, 16);
        c.cl.send_cmd(&mut c.net);
        if c.cl.download_prompt & DLP_SHOW != 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(150));
    }
    assert_eq!(c.cl.download_prompt, DLP_SHOW);
    assert!(c.cl.download_prompt_text.starts_with("You are missing files required by the server. You must download these files or disconnect:"), "{}", c.cl.download_prompt_text);
    assert!(c.cl.download_prompt_text.contains("base/map-dltest-1.pk3"));
    // the ui's "Download" (uiScript downloadUDP): the download runs
    c.cl.download_prompt = DLP_UDP;
    for _ in 0..3000 {
        step(&mut sv, &mut snet, &mut c);
        if c.cl.state == ConnState::Active {
            break;
        }
    }
    assert_eq!(c.cl.state, ConnState::Active, "{:?} {}", c.cl.error, c.cl.print_out);
    assert!(c.cl.print_out.contains("Download prompt returned 4"));
    // a second client with no ui to answer: the prompt goes stale, is
    // ignored, and a pure server's missing pak ends the connection
    let s2 = setup("prompt2", 5_000);
    let (mut sv2, mut snet2, port2) = spawn(&s2, 1);
    let mut c2 = client(&s2, port2, 0);
    for _ in 0..600 {
        step(&mut sv2, &mut snet2, &mut c2);
        if c2.cl.error.is_some() {
            break;
        }
    }
    assert!(c2.cl.print_out.contains("WARNING: UI VM does not support download prompt"), "{}", c2.cl.print_out);
    assert_eq!(c2.cl.error.as_deref(), Some("Automatic downloads are disabled on your client (cl_allowDownload is 0). You can enable automatic downloads in the Options menu."));
    for d in [&s, &s2] {
        let _ = std::fs::remove_dir_all(d.srv_base.parent().unwrap().parent().unwrap());
    }
}

#[test]
fn server_refuses_downloads_and_unpure_clients() {
    // sv_allowDownload 0: the server's refusal ends the client's connection
    let s = setup("refuse", 5_000);
    let (mut sv, mut snet, port) = spawn(&s, 0);
    let mut c = client(&s, port, 1);
    for _ in 0..600 {
        step(&mut sv, &mut snet, &mut c);
        if c.cl.error.is_some() {
            break;
        }
    }
    let e = c.cl.error.clone().unwrap_or_default();
    assert!(e.starts_with("Could not download \"base/map-dltest-1.pk3\" because autodownloading is disabled on the server.\n\nYou will need to get this file elsewhere before you can connect to this pure server."), "{e}");
    // a client with every pak joins; a forged "cp" afterwards is refused
    std::fs::copy(&s.map_pk3, s.cli_base.join("map-dltest-1.pk3")).unwrap();
    let (mut sv, mut snet, port) = spawn(&s, 0);
    let mut c = client(&s, port, 0);
    for _ in 0..600 {
        step(&mut sv, &mut snet, &mut c);
        if c.cl.state == ConnState::Active && sv.active_clients() == 1 {
            break;
        }
    }
    assert_eq!(c.cl.state, ConnState::Active, "{:?} {}", c.cl.error, c.cl.print_out);
    let i = c.cl.client_num as usize;
    assert!(sv.clients[i].pure_authentic);
    let forged = format!("cp {} 1 2 @ 3 4 5", c.cl.server_id);
    c.cl.add_reliable_command(&forged, false).unwrap();
    for _ in 0..200 {
        step(&mut sv, &mut snet, &mut c);
        if c.cl.error.is_some() {
            break;
        }
    }
    assert!(matches!(sv.clients[i].state, ClientState::Zombie | ClientState::Free));
    assert!(sv.console.contains("Unpure client detected. Invalid .PK3 files referenced!") || c.cl.error.as_deref().is_some_and(|e| e.contains("Unpure Client")), "{} {:?}", sv.console, c.cl.error);
    assert_eq!(c.cl.error.as_deref(), Some("Server disconnected - Unpure Client. You may need to enable in-game downloads to connect to this server (set cl_allowDownload 1)"));
    // a pak name the server did not reference cannot be downloaded
    let mut c3 = client(&s, port, 1);
    for _ in 0..600 {
        step(&mut sv, &mut snet, &mut c3);
        if c3.cl.state == ConnState::Active {
            break;
        }
    }
    c3.cl.begin_download("base/x.pk3", "base/notreferenced.pk3");
    for _ in 0..200 {
        step(&mut sv, &mut snet, &mut c3);
        if c3.cl.error.is_some() {
            break;
        }
    }
    assert_eq!(c3.cl.error.as_deref(), Some("File \"base/notreferenced.pk3\" is not referenced and cannot be downloaded."));
    let _ = std::fs::remove_dir_all(s.srv_base.parent().unwrap().parent().unwrap());
}
