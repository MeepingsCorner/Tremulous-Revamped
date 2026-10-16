//! The internet server list end to end (dec039): a dedicated 2 server
//! heartbeats to a master, the master verifies it with getinfo from its
//! second port, a client asks the master for the list, pings the entry
//! for its info and joins it. The master here follows darklegion's
//! tremulous-master (the one behind master.tremulous.net): `getservers
//! <protocol> [empty] [full]`, the protocol matched against the
//! infoResponse's comma-separated `protocol`, the out port = in port + 1.
use std::net::UdpSocket;
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::cl::{ConnState, NetClient, AS_GLOBAL};
use trem_sim::kernel::netchan::{Net, NetSrc};
use trem_sim::kernel::sv::{Server, ServerCvars, PROTOCOL_VERSION};
use trem_sim::kernel::types::UserCmd;
use trem_sim::kernel::world::World;

const OOB: &[u8] = b"\xff\xff\xff\xff";

/// master.py's state machine over two localhost sockets.
struct Master {
    inp: UdpSocket,
    out: UdpSocket,
    challenge: String,
    pending: Option<std::net::SocketAddr>,
    /// verified servers and their protocol lists
    servers: Vec<(std::net::SocketAddrV4, Vec<String>)>,
    /// every getservers request seen, verbatim
    requests: Vec<String>,
}

impl Master {
    fn new() -> Master {
        // inPort, outPort = inPort + 1 (config.py)
        loop {
            let inp = UdpSocket::bind("127.0.0.1:0").unwrap();
            let p = inp.local_addr().unwrap().port();
            if let Ok(out) = UdpSocket::bind(("127.0.0.1", p + 1)) {
                inp.set_nonblocking(true).unwrap();
                out.set_nonblocking(true).unwrap();
                return Master { inp, out, challenge: "a1b2c3d4".into(), pending: None, servers: Vec::new(), requests: Vec::new() };
            }
        }
    }

    fn port(&self) -> u16 {
        self.inp.local_addr().unwrap().port()
    }

    fn poll(&mut self) {
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = self.inp.recv_from(&mut buf) {
            let d = &buf[..n];
            if !d.starts_with(OOB) {
                continue;
            }
            let text = String::from_utf8_lossy(&d[4..]).to_string();
            if text.starts_with("heartbeat") {
                // Server.send_challenge, from the out socket
                let mut m = OOB.to_vec();
                m.extend_from_slice(format!("getinfo {}", self.challenge).as_bytes());
                self.out.send_to(&m, from).unwrap();
                self.pending = Some(from);
            } else if text.starts_with("getservers") {
                self.requests.push(text.trim_end_matches('\0').to_string());
                let mut tokens = text.split_whitespace();
                let ext = tokens.next() == Some("getserversExt");
                if ext && tokens.next() != Some("Tremulous") {
                    continue; // ext but not Tremulous, ignored
                }
                let Some(protocol) = tokens.next() else { continue }; // no protocol specified
                let mut m = OOB.to_vec();
                m.extend_from_slice(if ext { b"getserversExtResponse" } else { b"getserversResponse" });
                if ext {
                    m.extend_from_slice(b"\x001\x001\x00");
                }
                for (a, protos) in &self.servers {
                    if protos.iter().any(|p| p == protocol) {
                        m.push(b'\\');
                        m.extend_from_slice(&a.ip().octets());
                        m.extend_from_slice(&a.port().to_be_bytes());
                    }
                }
                m.push(b'\\');
                self.inp.send_to(&m, from).unwrap();
            }
        }
        while let Ok((n, from)) = self.out.recv_from(&mut buf) {
            let text = String::from_utf8_lossy(&buf[4..n]).to_string();
            if let Some(info) = text.strip_prefix("infoResponse\n") {
                assert_eq!(Some(from), self.pending, "the infoResponse comes from the heartbeating address");
                let v = |k: &str| -> String {
                    let f: Vec<&str> = info.split('\\').collect();
                    f.iter().position(|x| *x == k).and_then(|i| f.get(i + 1)).map(|s| s.to_string()).unwrap_or_default()
                };
                assert_eq!(v("challenge"), self.challenge, "mismatched challenge");
                for key in ["hostname", "clients", "sv_maxclients"] {
                    assert!(!v(key).is_empty(), "info key missing: {key}");
                }
                let std::net::SocketAddr::V4(a) = from else { panic!() };
                let protos = v("protocol").split(',').map(str::to_string).collect();
                self.servers.retain(|(x, _)| *x != a);
                self.servers.push((a, protos));
            }
        }
    }
}

#[test]
fn internet_server_list_through_a_master() {
    let mut master = Master::new();
    let mut snet = Net::new();
    let sport = snet.open(NetSrc::Server, "127.0.0.1", 0).unwrap();
    let mut cv = ServerCvars { hostname: "port server".into(), ..Default::default() };
    cv.dedicated = 2;
    cv.masters[0] = format!("127.0.0.1:{}", master.port());
    let mut sv = Server::spawn(World::new(MapId::Arena), "arena", cv);

    let mut cnet = Net::new();
    cnet.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
    let mut cl = NetClient::new(0);
    cl.userinfo = "\\name\\Browser\\cl_guid\\0123456789ABCDEF0123456789ABCDEF".into();
    cl.master_names[0] = format!("127.0.0.1:{}", master.port());

    let step = |sv: &mut Server, cl: &mut NetClient, master: &mut Master, snet: &mut Net, cnet: &mut Net| {
        sv.frame(snet, 16);
        master.poll();
        cl.frame(cnet, 16);
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    // the heartbeat, the master's getinfo, the verified entry
    for _ in 0..200 {
        step(&mut sv, &mut cl, &mut master, &mut snet, &mut cnet);
        if !master.servers.is_empty() {
            break;
        }
    }
    assert_eq!(master.servers.len(), 1, "the master verified the heartbeating server");
    assert_eq!(master.servers[0].0.port(), sport);
    assert_eq!(master.servers[0].1, vec![PROTOCOL_VERSION.to_string()]);

    // the ui's "globalservers 0 full empty"
    cl.global_servers(&mut cnet, 0, "full empty").unwrap();
    for _ in 0..200 {
        step(&mut sv, &mut cl, &mut master, &mut snet, &mut cnet);
        if cl.num_global_servers >= 0 && !cl.global_servers.is_empty() {
            break;
        }
    }
    // both protocols are asked for (dec040)
    assert_eq!(master.requests, vec![format!("getservers {PROTOCOL_VERSION} full empty"), "getservers 69 full empty".to_string()]);
    assert_eq!(cl.global_servers.len(), 1, "the master's list reached the client");
    assert_eq!(cl.global_servers[0].adr.port, sport);

    // LAN_UpdateVisiblePings: the ping brings the hostname and map
    cl.global_servers[0].visible = true;
    let mut pinging = true;
    for _ in 0..200 {
        pinging = cl.update_visible_pings(&mut cnet, AS_GLOBAL);
        step(&mut sv, &mut cl, &mut master, &mut snet, &mut cnet);
        if !pinging {
            break;
        }
    }
    assert!(!pinging);
    let s = &cl.global_servers[0];
    assert_eq!(s.host_name, "port server");
    assert_eq!(s.map_name, "arena");
    assert!(s.ping > 0);

    // join it from the list
    let adr = format!("127.0.0.1:{sport}");
    cl.connect(&mut cnet, &adr).unwrap();
    let mut cg_loaded = false;
    for _ in 0..400 {
        sv.frame(&mut snet, 16);
        cl.frame(&mut cnet, 16);
        if cl.gamestate_new && !cg_loaded {
            cl.cgame_loaded(&mut cnet);
            cg_loaded = true;
        }
        cl.create_cmd(UserCmd::default());
        cl.send_cmd(&mut cnet);
        std::thread::sleep(std::time::Duration::from_micros(300));
        if cl.state == ConnState::Active {
            break;
        }
    }
    assert_eq!(cl.state, ConnState::Active, "the client joined the listed server");
    assert_eq!(sv.active_clients(), 1);
}

#[test]
fn a_master_that_drops_the_gamename_form_still_lists() {
    // the C's own request ("getservers Tremulous full empty") gets nothing
    // from this master: its first word is taken as the protocol
    let mut master = Master::new();
    master.servers.push(("127.0.0.1:30720".parse().unwrap(), vec![PROTOCOL_VERSION.to_string()]));
    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    probe.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();
    let mut count = |q: &str| -> usize {
        let mut m = OOB.to_vec();
        m.extend_from_slice(q.as_bytes());
        probe.send_to(&m, ("127.0.0.1", master.port())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        master.poll();
        let mut buf = [0u8; 1400];
        let (n, _) = probe.recv_from(&mut buf).unwrap();
        // "\\" + 6 bytes per entry, then the closing "\\"
        (n - 4 - "getserversResponse".len() - 1) / 7
    };
    assert_eq!(count("getservers Tremulous full empty"), 0);
    assert_eq!(count(&format!("getservers {PROTOCOL_VERSION} full empty")), 1);
}

#[test]
fn the_menu_refresh_asks_for_full_and_empty_servers() {
    use trem_sim::kernel::nethost::NetHost;
    let mut master = Master::new();
    let mut h = NetHost::new();
    h.cl.master_names[0] = format!("127.0.0.1:{}", master.port());
    // ui_main.c UI_StartServerRefresh: "globalservers 0 full empty"
    let argv: Vec<String> = ["globalservers", "0", "full", "empty"].iter().map(|s| s.to_string()).collect();
    let out = h.command(&argv).unwrap_or_default();
    assert!(out.starts_with("Requesting servers from master"), "{out}");
    for _ in 0..200 {
        h.frame(16, None, None);
        master.poll();
        std::thread::sleep(std::time::Duration::from_millis(1));
        if !master.requests.is_empty() {
            break;
        }
    }
    for _ in 0..50 {
        master.poll();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(master.requests, vec![format!("getservers {PROTOCOL_VERSION} full empty"), "getservers 69 full empty".to_string()]);
}

/// A Tremulous 1.1.0 server as the browser sees it: getinfo answered with
/// the infoResponse of a real 1.1.0 tremded (no gamename, protocol 69).
struct Vanilla {
    sock: UdpSocket,
}

impl Vanilla {
    fn new() -> Vanilla {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_nonblocking(true).unwrap();
        Vanilla { sock }
    }
    fn addr(&self) -> std::net::SocketAddrV4 {
        match self.sock.local_addr().unwrap() {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        }
    }
    fn info(challenge: &str) -> Vec<u8> {
        let mut m = OOB.to_vec();
        m.extend_from_slice(format!("infoResponse\n\\pure\\0\\sv_maxclients\\8\\clients\\0\\mapname\\atcs\\hostname\\noname\\protocol\\69\\challenge\\{challenge}").as_bytes());
        m
    }
    fn poll(&self) {
        let mut buf = [0u8; 1400];
        while let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            let t = String::from_utf8_lossy(&buf[4..n]).to_string();
            if let Some(c) = t.strip_prefix("getinfo ") {
                self.sock.send_to(&Self::info(c.trim_end_matches('\0')), from).unwrap();
            }
        }
    }
}

#[test]
fn the_browser_merges_legacy_and_vanilla_servers() {
    // dec040: the master lists 71 and 69 servers apart; the client asks
    // for both, merges them, and each server's infoResponse tells its protocol
    let mut master = Master::new();
    let mut snet = Net::new();
    let sport = snet.open(NetSrc::Server, "127.0.0.1", 0).unwrap();
    let cv = ServerCvars { hostname: "legacy server".into(), ..Default::default() };
    let mut sv = Server::spawn(World::new(MapId::Arena), "arena", cv);
    let vanilla = Vanilla::new();
    let legacy: std::net::SocketAddrV4 = format!("127.0.0.1:{sport}").parse().unwrap();
    // a server running both protocols is listed by both queries, once in the browser
    let both = Vanilla::new();
    master.servers.push((legacy, vec![PROTOCOL_VERSION.to_string()]));
    master.servers.push((vanilla.addr(), vec!["69".into()]));
    master.servers.push((both.addr(), vec!["69".into(), PROTOCOL_VERSION.to_string()]));

    let mut cnet = Net::new();
    cnet.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
    let mut cl = NetClient::new(0);
    cl.master_names[0] = format!("127.0.0.1:{}", master.port());
    cl.global_servers(&mut cnet, 0, "full empty").unwrap();
    for _ in 0..300 {
        sv.frame(&mut snet, 16);
        master.poll();
        vanilla.poll();
        both.poll();
        cl.frame(&mut cnet, 16);
        std::thread::sleep(std::time::Duration::from_millis(1));
        if master.requests.len() == 2 && cl.global_servers.len() == 3 {
            break;
        }
    }
    assert_eq!(master.requests, vec![format!("getservers {PROTOCOL_VERSION} full empty"), "getservers 69 full empty".to_string()]);
    assert_eq!(cl.global_servers.len(), 3, "the two answers merged, the shared server once: {:?}", cl.global_servers.iter().map(|s| s.adr.to_string_port()).collect::<Vec<_>>());
    for s in cl.global_servers.iter_mut() {
        s.visible = true;
    }
    let mut pinging = true;
    for _ in 0..300 {
        pinging = cl.update_visible_pings(&mut cnet, AS_GLOBAL);
        sv.frame(&mut snet, 16);
        vanilla.poll();
        both.poll();
        cl.frame(&mut cnet, 16);
        std::thread::sleep(std::time::Duration::from_millis(1));
        if !pinging {
            break;
        }
    }
    assert!(!pinging);
    let find = |port: u16| cl.global_servers.iter().find(|s| s.adr.port == port).unwrap();
    let l = find(sport);
    assert_eq!((l.protocol, l.host_name.as_str(), l.map_name.as_str()), (71, "legacy server", "arena"));
    let v = find(vanilla.addr().port());
    assert_eq!((v.protocol, v.host_name.as_str(), v.map_name.as_str()), (69, "noname", "atcs"));
    assert!(v.ping > 0);
}

#[test]
fn the_lan_scan_takes_vanilla_servers() {
    // CL_ServerInfoPacket on the LAN list: a 1.1.0 infoResponse has no
    // gamename and protocol 69; a legacy one without gamename is ignored
    let vanilla = Vanilla::new();
    let mut cnet = Net::new();
    let cport = cnet.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
    let mut cl = NetClient::new(0);
    cl.local_servers(&mut cnet);
    vanilla.sock.send_to(&Vanilla::info("xxx"), ("127.0.0.1", cport)).unwrap();
    let odd = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut m = OOB.to_vec();
    m.extend_from_slice(b"infoResponse\n\\protocol\\71\\hostname\\nogamename");
    odd.send_to(&m, ("127.0.0.1", cport)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    for _ in 0..10 {
        cl.frame(&mut cnet, 16);
    }
    assert_eq!(cl.local_servers.len(), 1, "{:?}", cl.local_servers);
    assert_eq!(cl.local_servers[0].adr.port, vanilla.addr().port());
    assert_eq!(cl.local_servers[0].protocol, 69);
}

#[test]
fn the_lan_scan_finds_a_server_by_its_network_broadcast() {
    // dec045: the scan also broadcasts on each network by its own address
    // (Windows sends 255.255.255.255 out of one adapter only); a server
    // bound to every address answers it from its network address
    let locals = trem_sim::kernel::netchan::local_ipv4();
    let Some(me) = locals.first().copied() else { return };
    let srv = UdpSocket::bind(("0.0.0.0", 0)).unwrap();
    srv.set_nonblocking(true).unwrap();
    let sport = srv.local_addr().unwrap().port();
    let mut cnet = Net::new();
    cnet.open(NetSrc::Client, "", 0).unwrap();
    // the scan's message to this network's broadcast on the server's port
    let b = trem_sim::kernel::netchan::lan_broadcasts(&[me])[0];
    let to = trem_sim::kernel::netchan::NetAdr { kind: trem_sim::kernel::netchan::NetAdrType::Ip, ip: b, port: sport, ..Default::default() };
    cnet.send_packet(NetSrc::Client, b"\xff\xff\xff\xffgetinfo xxx", &to);
    let mut buf = [0u8; 256];
    let mut got = None;
    for _ in 0..200 {
        if let Ok((n, _)) = srv.recv_from(&mut buf) {
            got = Some(buf[..n].to_vec());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(&got.expect("no broadcast arrived")[4..], b"getinfo xxx");
}

/// sv_main.c's SV_MasterHeartbeat: each master resolved for ipv4 and for
/// ipv6 on its own, and the heartbeat sent to both (dec071). A fake resolver
/// stands in for DNS; nothing leaves the process (Net::capture).
fn fake_resolve(name: &str, family: trem_sim::kernel::netchan::NetAdrType) -> (i32, trem_sim::kernel::netchan::NetAdr) {
    use trem_sim::kernel::netchan::{NetAdr, NetAdrType};
    let a = |s: &str| NetAdr::from_socket(&s.parse().unwrap());
    match (name, family) {
        ("master.example", NetAdrType::Ip) => (2, a("192.0.2.10:1")),
        ("master.example", NetAdrType::Ip6) => (2, a("[2001:db8::10]:1")),
        ("v4only.example:31000", NetAdrType::Ip) => (1, a("192.0.2.20:31000")),
        _ => (0, NetAdr::default()),
    }
}

fn heartbeats_with(enabled: i32) -> (Vec<std::net::SocketAddr>, String) {
    let mut net = Net::new();
    net.enabled = enabled;
    net.capture = Some(Vec::new());
    let mut cv = ServerCvars { hostname: "port server".into(), ..Default::default() };
    cv.dedicated = 2;
    cv.masters[0] = "master.example".into();
    cv.masters[1] = "v4only.example:31000".into();
    cv.masters[2] = "nowhere.example".into();
    let mut sv = Server::spawn(World::new(MapId::Arena), "arena", cv);
    sv.master_resolve = fake_resolve;
    let mut console = String::new();
    let mut to = Vec::new();
    for _ in 0..200 {
        sv.frame(&mut net, 16);
        console.push_str(&std::mem::take(&mut sv.console));
        for (a, d) in net.capture.as_mut().unwrap().drain(..) {
            if d.starts_with(b"\xff\xff\xff\xffheartbeat ") {
                to.push(a.to_socket().unwrap());
            }
        }
        if console.contains("Sending heartbeat to v4only.example") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    to.sort();
    (to, console)
}

#[test]
fn heartbeats_go_to_the_ipv4_and_the_ipv6_address_of_each_master() {
    use trem_sim::kernel::netchan::{NET_ENABLEV4, NET_ENABLEV6, PORT_MASTER};
    let (to, console) = heartbeats_with(NET_ENABLEV4 | NET_ENABLEV6);
    let mut want: Vec<std::net::SocketAddr> = vec![format!("192.0.2.10:{PORT_MASTER}").parse().unwrap(), format!("[2001:db8::10]:{PORT_MASTER}").parse().unwrap(), "192.0.2.20:31000".parse().unwrap()];
    want.sort();
    assert_eq!(to, want, "{console}");
    for l in ["Resolving master.example (IPv4)", "Resolving master.example (IPv6)", "v4only.example:31000 has no IPv6 address.", "nowhere.example has no IPv4 address.", "Couldn't resolve address: nowhere.example"] {
        assert!(console.contains(l), "{l}: {console}");
    }
    // net_enabled 1: ipv4 only, as the C
    let (to, console) = heartbeats_with(NET_ENABLEV4);
    assert!(to.iter().all(|a| a.is_ipv4()) && to.len() == 2, "{to:?}");
    assert!(!console.contains("(IPv6)"), "{console}");
}
