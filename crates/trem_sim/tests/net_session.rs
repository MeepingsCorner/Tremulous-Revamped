//! A server and networked clients over localhost UDP (dec033): connect
//! (challenge, connect, gamestate, snapshots), join teams through reliable
//! commands, move with usercmds, see each other in snapshots, one shoots
//! the other, prediction stays within tolerance, a clean disconnect; plus
//! the listen-server path over the loopback, rcon, getinfo/getstatus and
//! the GUID / ban checks of ClientConnect.
use trem_sim::gen::constants as k;
use trem_sim::gen::enums::{SpectatorState, Stat, Team};
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::cgview::Cgame;
use trem_sim::kernel::cl::{ConnState, NetClient};
use trem_sim::kernel::netchan::{Net, NetSrc};
use trem_sim::kernel::sv::{ClientState, Server, ServerCvars};
use trem_sim::kernel::types::UserCmd;
use trem_sim::kernel::world::{EType, World};

struct Peer {
    net: Net,
    cl: NetClient,
    cg: Option<Cgame>,
    view: Option<World>,
    cmd: UserCmd,
}

impl Peer {
    fn new(name: &str, guid: &str, server: &str, loopback: Option<Net>) -> Peer {
        let mut net = loopback.unwrap_or_default();
        if !net.is_open(NetSrc::Client) && server != "localhost" {
            net.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
        }
        let mut cl = NetClient::new(0);
        cl.userinfo = format!("\\name\\{name}\\cl_guid\\{guid}\\rate\\25000\\snaps\\20\\cg_unlagged\\1\\teamoverlay\\1");
        cl.connect(&mut net, server).unwrap();
        Peer { net, cl, cg: None, view: None, cmd: UserCmd::default() }
    }

    /// One client frame: packets, cgame, a usercmd.
    fn frame(&mut self, msec: i32) {
        self.cl.frame(&mut self.net, msec);
        if self.cl.gamestate_new {
            // CL_DownloadsComplete -> CL_InitCGame: the map is the arena
            self.view = Some(World::client_view(MapId::Arena));
            self.cg = Some(Cgame::init(&self.cl));
            self.cl.cgame_loaded(&mut self.net);
        }
        if let (Some(cg), Some(v)) = (self.cg.as_mut(), self.view.as_mut()) {
            if self.cl.state == ConnState::Active {
                cg.frame(&mut self.cl, v);
            }
        }
        self.cl.create_cmd(self.cmd);
        self.cl.send_cmd(&mut self.net);
    }

    fn me(&self) -> usize {
        self.cl.client_num as usize
    }
}

fn server(port_out: &mut u16) -> (Server, Net) {
    let mut net = Net::new();
    let port = net.open(NetSrc::Server, "127.0.0.1", 0).unwrap();
    *port_out = port;
    let mut w = World::new(MapId::Arena);
    w.level.cheats = true;
    // both test players are humans
    w.level.friendly_fire = true;
    let cvars = ServerCvars { hostname: "test server".into(), rcon_password: "secret".into(), ..Default::default() };
    (Server::spawn(w, "arena", cvars), net)
}

const GUID_A: &str = "0123456789ABCDEF0123456789ABCDEF";
const GUID_B: &str = "FEDCBA9876543210FEDCBA9876543210";

fn step(sv: &mut Server, snet: &mut Net, peers: &mut [Peer], ms: i32) {
    sv.frame(snet, ms);
    for p in peers.iter_mut() {
        p.frame(ms);
    }
    // let the kernel deliver the datagrams
    std::thread::sleep(std::time::Duration::from_micros(200));
}

#[test]
fn two_clients_play_over_udp() {
    let mut port = 0;
    let (mut sv, mut snet) = server(&mut port);
    let addr = format!("127.0.0.1:{port}");
    let mut peers = vec![Peer::new("Alice", GUID_A, &addr, None), Peer::new("Bob", GUID_B, &addr, None)];
    // connect: challenge, connect, gamestate, first snapshot
    for _ in 0..400 {
        step(&mut sv, &mut snet, &mut peers, 16);
        if peers.iter().all(|p| p.cl.state == ConnState::Active) && sv.active_clients() == 2 {
            break;
        }
    }
    for p in &peers {
        assert_eq!(p.cl.state, ConnState::Active, "{:?}: {}", p.cl.error, p.cl.print_out);
    }
    let (a, b) = (peers[0].me(), peers[1].me());
    assert_ne!(a, b);
    assert_eq!(sv.world.clients[a].name, "Alice");
    assert_eq!(sv.world.clients[b].name, "Bob");
    assert_eq!(sv.world.clients[a].guid, GUID_A);
    assert!(!sv.world.clients[a].local_client && sv.world.clients[a].admin.is_none(), "remote players are not admins by default");
    assert_eq!(sv.world.clients[a].ip.str, "127.0.0.1");
    // the configstrings reached both views: names from CS_PLAYERS
    for p in &peers {
        let v = p.view.as_ref().unwrap();
        assert_eq!(v.clients[a].name, "Alice");
        assert_eq!(v.clients[b].name, "Bob");
    }
    // join the humans with a rifle (reliable client commands)
    for p in peers.iter_mut() {
        p.cl.add_reliable_command("team humans", false).unwrap();
        p.cl.add_reliable_command("class rifle", false).unwrap();
    }
    for _ in 0..600 {
        step(&mut sv, &mut snet, &mut peers, 16);
        if [a, b].iter().all(|&i| sv.world.clients[i].spectator == SpectatorState::Not as i32) && peers.iter().all(|p| p.view.as_ref().unwrap().clients[p.me()].ps.stats[Stat::Health as usize] > 0 && p.view.as_ref().unwrap().clients[p.me()].ps.persistant[trem_sim::gen::enums::Pers::Specstate as usize] == 0) {
            break;
        }
    }
    assert_eq!(sv.world.clients[a].team, Team::Humans);
    assert_eq!(sv.world.clients[a].spectator, SpectatorState::Not as i32, "Alice spawned");
    assert_eq!(sv.world.clients[b].spectator, SpectatorState::Not as i32, "Bob spawned");
    // move: Alice walks forward for a second; her prediction must agree with the server
    let before = sv.world.clients[a].ps.origin;
    peers[0].cmd.forwardmove = 127;
    for _ in 0..62 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    peers[0].cmd.forwardmove = 0;
    for _ in 0..30 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    let after = sv.world.clients[a].ps.origin;
    let moved = ((after[0] - before[0]).powi(2) + (after[1] - before[1]).powi(2)).sqrt();
    assert!(moved > 100.0, "Alice moved {moved} units on the server");
    // her predicted origin matches the server's once she stands still
    let pv = peers[0].view.as_ref().unwrap().clients[a].ps.origin;
    let d = ((pv[0] - after[0]).powi(2) + (pv[1] - after[1]).powi(2) + (pv[2] - after[2]).powi(2)).sqrt();
    assert!(d < 1.0, "prediction {pv:?} vs server {after:?}");
    let cg = peers[0].cg.as_ref().unwrap();
    let worst = cg.misses.iter().cloned().fold(0.0f32, f32::max);
    assert!(worst < 8.0, "prediction misses {:?}", cg.misses);
    // Bob sees Alice where the server has her
    let bv = peers[1].view.as_ref().unwrap();
    assert!(bv.ents[a].inuse && bv.ents[a].etype == EType::Player, "Bob's snapshot has Alice");
    let seen = bv.clients[a].ps.origin;
    assert!(((seen[0] - after[0]).powi(2) + (seen[1] - after[1]).powi(2)).sqrt() < 2.0, "Bob sees Alice at {seen:?}, server {after:?}");
    let av = peers[0].view.as_ref().unwrap();
    assert!(av.ents[b].inuse && av.ents[b].etype == EType::Player, "Alice's snapshot has Bob");
    // shoot: place them face to face (setviewpos is a cheat; sv_cheats on),
    // Alice holds the trigger
    peers[0].cl.add_reliable_command("setviewpos -300 -300 40 0", false).unwrap();
    peers[1].cl.add_reliable_command("setviewpos -100 -300 40 180", false).unwrap();
    for _ in 0..20 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    let hp0 = sv.world.clients[b].ps.stats[Stat::Health as usize];
    peers[0].cmd.buttons = k::BUTTON_ATTACK;
    let mut hits = 0;
    let mut fired = 0;
    for _ in 0..60 {
        step(&mut sv, &mut snet, &mut peers, 16);
        let v = peers[1].view.as_mut().unwrap();
        hits += v.drain_events().iter().filter(|e| matches!(e.event, trem_sim::gen::enums::Event::BulletHitFlesh | trem_sim::gen::enums::Event::Pain)).count();
        let v = peers[0].view.as_mut().unwrap();
        fired += v.drain_events().iter().filter(|e| e.event == trem_sim::gen::enums::Event::FireWeapon && e.entity == a as i32).count();
    }
    peers[0].cmd.buttons = 0;
    for _ in 0..10 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    let hp1 = sv.world.clients[b].ps.stats[Stat::Health as usize];
    assert!(hp1 < hp0, "Bob took damage on the server ({hp0} -> {hp1})");
    let bob_view_hp = peers[1].view.as_ref().unwrap().clients[b].ps.stats[Stat::Health as usize];
    assert_eq!(bob_view_hp, hp1, "Bob's own view shows his health");
    assert!(fired > 0, "Alice's own fire events were predicted");
    assert!(hits > 0, "Bob saw the hits");
    // pings were measured
    assert!(sv.clients[a].ping < 999);
    let m = &peers[0].cg.as_ref().unwrap().misses;
    eprintln!("two_clients_play_over_udp: Alice moved {moved:.0}u, prediction misses {} (worst {worst:.2}u), ping {} ms, hits seen {hits}, fire events predicted {fired}, server sent {} packets", m.len(), sv.clients[a].ping, snet.packets_sent);
    // a clean disconnect
    let Peer { net, cl, .. } = &mut peers[0];
    cl.disconnect(net);
    for _ in 0..30 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    assert!(!sv.world.clients[a].connected, "the server dropped Alice");
    assert!(matches!(sv.clients[a].state, ClientState::Zombie | ClientState::Free));
    let bv = peers[1].view.as_ref().unwrap();
    assert!(!bv.clients[a].connected, "Bob's view lost Alice");
    assert!(bv.prints.iter().any(|p| p.3.contains("Alice") && p.3.contains("disconnected")) || peers[1].cg.as_ref().unwrap().print_out.contains("disconnected"));
}

#[test]
fn listen_server_loopback_client_is_host_admin() {
    let mut port = 0;
    let (mut sv, mut net) = server(&mut port);
    // the listen server's own client shares the server's Net (loopback)
    let mut cl = NetClient::new(0);
    cl.userinfo = format!("\\name\\Host\\cl_guid\\{GUID_A}");
    cl.connect(&mut net, "localhost").unwrap();
    let mut view = None;
    let mut cg = None;
    for _ in 0..200 {
        sv.frame(&mut net, 16);
        cl.frame(&mut net, 16);
        if cl.gamestate_new {
            view = Some(World::client_view(MapId::Arena));
            cg = Some(Cgame::init(&cl));
            cl.cgame_loaded(&mut net);
        }
        if let (Some(c), Some(v)) = (cg.as_mut(), view.as_mut()) {
            if cl.state == ConnState::Active {
                c.frame(&mut cl, v);
            }
        }
        cl.create_cmd(UserCmd::default());
        cl.send_cmd(&mut net);
        if cl.state == ConnState::Active && sv.active_clients() == 1 {
            break;
        }
    }
    assert_eq!(cl.state, ConnState::Active);
    let i = cl.client_num as usize;
    assert!(sv.world.clients[i].local_client, "ip localhost: the host's player");
    assert!(sv.world.clients[i].admin.is_some(), "the listen-server host is the top admin (dec026)");
    // G_AddressParse("localhost") fails, so pers.ip.str stays empty like the C
    assert_eq!(sv.world.clients[i].ip.str, "");
    // the view shows the map's structures
    let v = view.unwrap();
    assert!(v.ents.iter().any(|e| e.inuse && e.etype == EType::Buildable && e.spawned), "structures in the snapshot");
}

#[test]
fn connectionless_queries_and_refusals() {
    let mut port = 0;
    let (mut sv, mut snet) = server(&mut port);
    let mut net = Net::new();
    net.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
    let to = trem_sim::kernel::netchan::string_to_adr(&format!("127.0.0.1:{port}"), trem_sim::kernel::netchan::NetAdrType::Unspec).1;
    net.out_of_band_print(NetSrc::Client, &to, "getinfo xxx");
    net.out_of_band_print(NetSrc::Client, &to, "getstatus yyy");
    net.out_of_band_print(NetSrc::Client, &to, "rcon secret status");
    net.out_of_band_print(NetSrc::Client, &to, "rcon wrong status");
    let mut got = Vec::new();
    for _ in 0..50 {
        sv.frame(&mut snet, 16);
        while let Some((_, d)) = net.get_packet(NetSrc::Client) {
            got.push(String::from_utf8_lossy(&d[4..]).into_owned());
        }
        if got.len() >= 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let info = got.iter().find(|g| g.starts_with("infoResponse")).expect("infoResponse");
    assert!(info.contains("\\challenge\\xxx") && info.contains("\\hostname\\test server") && info.contains("\\protocol\\71") && info.contains("\\mapname\\arena"), "{info}");
    let status = got.iter().find(|g| g.starts_with("statusResponse")).expect("statusResponse");
    assert!(status.contains("\\challenge\\yyy"), "{status}");
    assert!(got.iter().any(|g| g.starts_with("print\nBad rconpassword")), "{got:?}");
    assert_eq!(sv.rcon_queue.len(), 1);
    assert_eq!(sv.rcon_queue[0].1, "status");
    // an invalid GUID is refused by ClientConnect
    let mut peer = Peer::new("NoGuid", "not-hex", &format!("127.0.0.1:{port}"), None);
    for _ in 0..300 {
        sv.frame(&mut snet, 16);
        peer.frame(16);
        std::thread::sleep(std::time::Duration::from_micros(200));
        if !peer.cl.server_message.is_empty() {
            break;
        }
    }
    assert_eq!(peer.cl.server_message, "Invalid GUID");
    assert_ne!(peer.cl.state, ConnState::Active);
}

/// One frame on a simulated clock: every Net holds its packets
/// `cl_packetdelay` / `sv_packetdelay` ms (NET_QueuePacket) before sending.
fn step_lagged(sv: &mut Server, snet: &mut Net, peers: &mut [Peer], clock: &mut i32, ms: i32) {
    *clock += ms;
    snet.clock = Some(*clock);
    snet.flush_packet_queue();
    sv.frame(snet, ms);
    for p in peers.iter_mut() {
        p.net.clock = Some(*clock);
        p.net.flush_packet_queue();
        p.frame(ms);
    }
    std::thread::sleep(std::time::Duration::from_micros(200));
}

/// Alice tracks a strafing Bob through 100 ms of lag each way; returns
/// (server damage Bob took, worst prediction miss of Alice, Alice's ping).
fn lagged_duel(unlagged: bool) -> (i32, f32, i32) {
    let mut port = 0;
    let (mut sv, mut snet) = server(&mut port);
    sv.world.level.unlagged = unlagged;
    snet.packet_delay[NetSrc::Server as usize] = 100;
    let addr = format!("127.0.0.1:{port}");
    let mut peers = vec![Peer::new("Alice", GUID_A, &addr, None), Peer::new("Bob", GUID_B, &addr, None)];
    for p in peers.iter_mut() {
        p.net.packet_delay[NetSrc::Client as usize] = 100;
    }
    let mut clock = 0;
    for _ in 0..600 {
        step_lagged(&mut sv, &mut snet, &mut peers, &mut clock, 16);
        if peers.iter().all(|p| p.cl.state == ConnState::Active) && sv.active_clients() == 2 {
            break;
        }
    }
    let (a, b) = (peers[0].me(), peers[1].me());
    for p in peers.iter_mut() {
        p.cl.add_reliable_command("team humans", false).unwrap();
        p.cl.add_reliable_command("class rifle", false).unwrap();
    }
    for _ in 0..600 {
        step_lagged(&mut sv, &mut snet, &mut peers, &mut clock, 16);
        if [a, b].iter().all(|&i| sv.world.clients[i].spectator == SpectatorState::Not as i32 && sv.world.clients[i].ps.stats[Stat::Health as usize] > 0) {
            break;
        }
    }
    peers[0].cl.add_reliable_command("setviewpos -300 -300 40 0", false).unwrap();
    peers[1].cl.add_reliable_command("setviewpos -100 -300 40 180", false).unwrap();
    for _ in 0..40 {
        step_lagged(&mut sv, &mut snet, &mut peers, &mut clock, 16);
    }
    peers[0].cg.as_mut().unwrap().misses.clear();
    let hp0 = sv.world.clients[b].ps.stats[Stat::Health as usize];
    // Bob strafes across Alice's line of fire; Alice aims where she sees him
    for f in 0..50 {
        peers[1].cmd.rightmove = if (f / 12) % 2 == 0 { 127 } else { -127 };
        let v = peers[0].view.as_ref().unwrap();
        let me = v.clients[a].ps.origin;
        let him = v.ents[b].origin;
        let yaw = (him[1] - me[1]).atan2(him[0] - me[0]).to_degrees();
        let delta = v.clients[a].ps.delta_angles;
        peers[0].cmd.angles = [0 - delta[0], trem_sim::kernel::msg::angle2short(yaw) - delta[1], 0 - delta[2]];
        peers[0].cmd.buttons = k::BUTTON_ATTACK;
        step_lagged(&mut sv, &mut snet, &mut peers, &mut clock, 16);
    }
    peers[0].cmd.buttons = 0;
    peers[1].cmd.rightmove = 0;
    for _ in 0..10 {
        step_lagged(&mut sv, &mut snet, &mut peers, &mut clock, 16);
    }
    let hp1 = sv.world.clients[b].ps.stats[Stat::Health as usize];
    let worst = peers[0].cg.as_ref().unwrap().misses.iter().cloned().fold(0.0f32, f32::max);
    (hp0 - hp1, worst, sv.clients[a].ping)
}

#[test]
fn lagged_prediction_and_unlagged_hits() {
    let (dmg_on, worst_on, ping) = lagged_duel(true);
    let (dmg_off, worst_off, _) = lagged_duel(false);
    eprintln!("lagged_prediction_and_unlagged_hits: ping {ping} ms, damage with unlagged {dmg_on}, without {dmg_off}, worst prediction miss {worst_on:.2}u / {worst_off:.2}u");
    assert!(ping >= 190, "the packet delays show in the ping ({ping})");
    assert!(worst_on < 8.0 && worst_off < 8.0, "prediction stays within tolerance");
    assert!(dmg_on > 0, "the shooter hits what she saw");
    assert!(dmg_on > dmg_off, "unlagged makes the lagged shots hit ({dmg_on} vs {dmg_off})");
}

/// Two `NetHost`s as the Godot host drives them: a listen server (its
/// own player over the loopback) found by the other's LAN scan, pinged,
/// joined; both views see both players; the remote one leaves.
#[test]
fn nethost_listen_server_lan_browser_and_join() {
    use trem_sim::kernel::cl::AS_LOCAL;
    use trem_sim::kernel::nethost::{HostEvent, NetHost};
    let mut a = NetHost::new();
    let mut w = World::new(MapId::Arena);
    w.level.cheats = true;
    let mut cv = trem_sim::kernel::sv::ServerCvars { hostname: "listen host".into(), ..Default::default() };
    cv.dedicated = 0;
    a.cl.userinfo = format!("\\name\\Host\\cl_guid\\{GUID_A}");
    a.spawn_server(w, "arena", cv, "127.0.0.1", trem_sim::kernel::netchan::PORT_SERVER + 1, true);
    let port = a.net.ports[NetSrc::Server as usize];
    let mut b = NetHost::new();
    b.cl.userinfo = "\\name\\Remote".into();
    b.cl.qkey = Some((0..2048u32).map(|i| (i * 7 + 3) as u8).collect());
    let mut views: [Option<World>; 2] = [None, None];
    let mut step = |a: &mut NetHost, b: &mut NetHost, views: &mut [Option<World>; 2]| {
        for (k, h) in [a, b].into_iter().enumerate() {
            let cmd = h.client_num().map(|_| UserCmd::default());
            for e in h.frame(16, views[k].as_mut(), cmd) {
                match e {
                    HostEvent::LoadView { map } => {
                        assert_eq!(map, "arena");
                        views[k] = Some(World::client_view(MapId::Arena));
                        h.view_loaded();
                    }
                    HostEvent::Dropped { reason } => panic!("dropped: {reason}"),
                    HostEvent::Rcon { .. } | HostEvent::FsRestart | HostEvent::DemoCompleted => {}
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_micros(300));
    };
    // the LAN scan finds the listen server (it answers on one of the four server ports)
    assert_eq!(b.command(&["localservers".to_string()]).as_deref(), Some("Scanning for servers on the local network...\n"));
    for _ in 0..50 {
        step(&mut a, &mut b, &mut views);
    }
    let found = b.cl.local_servers.iter().position(|s| s.adr.port == port);
    if port < trem_sim::kernel::netchan::PORT_SERVER + trem_sim::kernel::netchan::NUM_SERVER_PORTS {
        let n = found.expect("the LAN scan found the listen server");
        // trap_LAN_MarkServerVisible + LAN_UpdateVisiblePings: the ping brings the info
        for s in b.cl.local_servers.iter_mut() {
            s.visible = true;
        }
        let mut pinging = true;
        for _ in 0..100 {
            pinging = b.cl.update_visible_pings(&mut b.net, AS_LOCAL);
            step(&mut a, &mut b, &mut views);
            if !pinging {
                break;
            }
        }
        assert!(!pinging);
        let s = &b.cl.local_servers[n];
        assert_eq!(s.host_name, "listen host");
        assert_eq!(s.map_name, "arena");
        assert!(s.ping > 0, "ping {}", s.ping);
    }
    // join it
    b.connect(&format!("127.0.0.1:{port}")).unwrap();
    for _ in 0..300 {
        step(&mut a, &mut b, &mut views);
        if a.client_num().is_some() && b.client_num().is_some() && a.server.as_ref().unwrap().active_clients() == 2 {
            break;
        }
    }
    let (ia, ib) = (a.client_num().unwrap(), b.client_num().unwrap());
    let sv = a.server.as_ref().unwrap();
    assert!(sv.world.clients[ia].local_client && sv.world.clients[ia].admin.is_some(), "the host's player is the listen server's admin");
    assert!(!sv.world.clients[ib].local_client && sv.world.clients[ib].admin.is_none());
    // cl_guid from the qkey (CL_UpdateGUID with cl_guidServerUniq)
    let guid = &sv.world.clients[ib].guid;
    assert_eq!(guid.len(), 32);
    assert_eq!(*guid, trem_sim::kernel::md5::md5_file(b.cl.qkey.as_ref().unwrap(), 2048, format!("127.0.0.1:{port}").as_bytes()));
    for _ in 0..20 {
        step(&mut a, &mut b, &mut views);
    }
    for v in views.iter() {
        let v = v.as_ref().unwrap();
        assert_eq!(v.clients[ia].name, "Host");
        assert_eq!(v.clients[ib].name, "Remote");
    }
    // the remote player leaves
    b.disconnect();
    for _ in 0..20 {
        step(&mut a, &mut b, &mut views);
    }
    assert!(!a.server.as_ref().unwrap().world.clients[ib].connected);
    assert!(!views[0].as_ref().unwrap().clients[ib].connected);
}

/// dec083: a client that joins as a spectator is looking along the
/// intermission point's angles (G_SelectSpectatorSpawnPoint ->
/// G_SetClientViewAngle), whatever its own cl.viewangles are when its first
/// usercmd arrives: the previous game's view, mouse movement while the map
/// loaded. The spawn runs at connect time in this port, before any cmd, so
/// the delta_angles have to be taken against the first cmd.
#[test]
fn spectator_join_keeps_the_intermission_view() {
    let mut port = 0;
    let (mut sv, mut snet) = server(&mut port);
    let addr = format!("127.0.0.1:{port}");
    let mut peers = vec![Peer::new("Alice", GUID_A, &addr, None), Peer::new("Bob", GUID_B, &addr, None)];
    // Alice starts with cl.viewangles 0; Bob arrives with a view he turned before (yaw 275, pitch 20)
    peers[1].cmd.angles = [trem_sim::kernel::msg::angle2short(20.0), trem_sim::kernel::msg::angle2short(275.0), 0];
    for _ in 0..400 {
        step(&mut sv, &mut snet, &mut peers, 16);
        if peers.iter().all(|p| p.cl.state == ConnState::Active) && sv.active_clients() == 2 {
            break;
        }
    }
    for _ in 0..40 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    let diff = |a: f32, b: f32| ((a - b + 540.0).rem_euclid(360.0) - 180.0).abs();
    let want = sv.world.level.intermission_angle;
    eprintln!("spectator_join_keeps_the_intermission_view: intermission angles {want:?}");
    assert!(diff(want[1], 275.0) > 10.0 && diff(want[0], 20.0) > 10.0, "Bob's own view differs from the spawn view, so the test can tell");
    for (n, p) in peers.iter().enumerate() {
        let i = p.me();
        assert_eq!(sv.world.clients[i].spectator, SpectatorState::Free as i32);
        let sv_view = sv.world.clients[i].ps.viewangles;
        assert!(diff(sv_view[1], want[1]) < 0.1 && diff(sv_view[0], want[0]) < 0.1, "client {n}: server view {sv_view:?}, intermission {want:?}");
        // and the client's own predicted view agrees
        let cl_view = p.view.as_ref().unwrap().clients[i].ps.viewangles;
        assert!(diff(cl_view[1], want[1]) < 0.1 && diff(cl_view[0], want[0]) < 0.1, "client {n}: predicted view {cl_view:?}, intermission {want:?}");
    }
}

/// dec083, the listen-server case ("Host Game"): the
/// host's own player joins over the loopback, its first usercmd carries
/// whatever cl.viewangles the host had (here yaw 275, pitch 20), and it
/// must still look along the intermission point; again after the next map
/// (SV_SpawnServer on the running server: every client gets a new gamestate
/// and enters the world on its first cmd, as after map_restart).
#[test]
fn listen_host_keeps_the_intermission_view_after_map_change() {
    use trem_sim::kernel::msg::angle2short;
    use trem_sim::kernel::nethost::{HostEvent, NetHost};
    let mut a = NetHost::new();
    let mut cv = ServerCvars { hostname: "listen host".into(), ..Default::default() };
    cv.dedicated = 0;
    a.cl.userinfo = format!("\\name\\Host\\cl_guid\\{GUID_A}");
    a.spawn_server(World::new(MapId::Arena), "arena", cv.clone(), "127.0.0.1", trem_sim::kernel::netchan::PORT_SERVER + 1, true);
    let mut view: Option<World> = None;
    let diff = |x: f32, y: f32| ((x - y + 540.0).rem_euclid(360.0) - 180.0).abs();
    for (round, own) in [(20.0f32, 275.0f32), (-30.0, 30.0)].into_iter().enumerate() {
        let cmd = UserCmd { angles: [angle2short(own.0), angle2short(own.1), 0], ..Default::default() };
        let mut loaded = false;
        for n in 0..600 {
            let c = a.client_num().map(|_| cmd);
            for e in a.frame(16, view.as_mut(), c) {
                match e {
                    HostEvent::LoadView { .. } => {
                        view = Some(World::client_view(MapId::Arena));
                        a.view_loaded();
                        loaded = true;
                    }
                    HostEvent::Dropped { reason } => panic!("dropped: {reason}"),
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_micros(300));
            if loaded && n > 100 && a.client_num().is_some() && a.server.as_ref().unwrap().active_clients() == 1 {
                break;
            }
        }
        assert!(loaded, "round {round}: the host got a gamestate");
        for _ in 0..40 {
            let c = a.client_num().map(|_| cmd);
            a.frame(16, view.as_mut(), c);
        }
        let i = a.client_num().unwrap();
        let sv = a.server.as_ref().unwrap();
        assert!(sv.world.clients[i].local_client);
        let want = sv.world.level.intermission_angle;
        assert!(diff(want[1], own.1) > 10.0, "the host's own yaw differs from the spawn view, so the test can tell");
        let sv_view = sv.world.clients[i].ps.viewangles;
        assert!(diff(sv_view[1], want[1]) < 0.1 && diff(sv_view[0], want[0]) < 0.1, "round {round}: server view {sv_view:?}, intermission {want:?}");
        let cl_view = view.as_ref().unwrap().clients[i].ps.viewangles;
        assert!(diff(cl_view[1], want[1]) < 0.1 && diff(cl_view[0], want[0]) < 0.1, "round {round}: host's predicted view {cl_view:?}, intermission {want:?}");
        if round == 0 {
            // the next map on the running server (change_map)
            a.spawn_server(World::new(MapId::Arena), "arena", cv.clone(), "127.0.0.1", trem_sim::kernel::netchan::PORT_SERVER + 1, true);
        }
    }
}
