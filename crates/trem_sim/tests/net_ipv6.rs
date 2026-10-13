//! IPv6 (dec037): net_ip.c's address parsing and printing for ipv6
//! servers, the net_enabled family priority of NET_StringToAdr, and — when
//! the host has ipv6 (EAFNOSUPPORT skips, like NET_OpenIP's scan) — a
//! connection over [::1] and the LAN scan through the multicast group.
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::cl::{ConnState, NetClient};
use trem_sim::kernel::netchan::{self, compare_adr, compare_base_adr_mask, is_lan_address, string_to_adr, Net, NetAdr, NetAdrType, NetSrc, NET_ENABLEV4, NET_ENABLEV6, NET_PRIOV6, PORT_SERVER};
use trem_sim::kernel::sv::{Server, ServerCvars};
use trem_sim::kernel::world::World;

#[test]
fn ipv6_addresses_parse_print_and_compare() {
    // NET_StringToAdr: brackets with or without a port, bare addresses
    let (r, a) = string_to_adr("[::1]:30721", NetAdrType::Unspec);
    assert_eq!((r, a.kind, a.port), (1, NetAdrType::Ip6, 30721));
    assert_eq!(a.ip6, std::net::Ipv6Addr::LOCALHOST.octets());
    assert_eq!(a.to_string_port(), "[::1]:30721");
    assert_eq!(a.to_string_base(), "::1");
    let (r, b) = string_to_adr("[::1]", NetAdrType::Unspec);
    assert_eq!((r, b.port), (2, PORT_SERVER));
    let (r, c) = string_to_adr("2001:db8::7", NetAdrType::Ip6);
    assert_eq!((r, c.kind, c.port), (2, NetAdrType::Ip6, PORT_SERVER));
    assert_eq!(c.to_string_port(), "[2001:db8::7]:30720");
    let (r, d) = string_to_adr("[2001:db8::7]:5", NetAdrType::Unspec);
    assert_eq!((r, d.port), (1, 5));
    assert!(compare_adr(&string_to_adr("[2001:db8::7]:5", NetAdrType::Unspec).1, &d));
    // NET_CompareBaseAdrMask over a /32 and /64 prefix (bans, dec034)
    let (_, e) = string_to_adr("[2001:db8:0:0:ffff::1]:9", NetAdrType::Unspec);
    assert!(compare_base_adr_mask(&c, &e, 64));
    assert!(compare_base_adr_mask(&c, &e, 32));
    assert!(!compare_base_adr_mask(&c, &e, 128));
    // Sys_IsLANAddress: link-local, unique local, loopback
    for (s, lan) in [("[fe80::1]", true), ("[fd00::1]", true), ("[::1]", true), ("[2001:db8::1]", false)] {
        assert_eq!(is_lan_address(&string_to_adr(s, NetAdrType::Unspec).1), lan, "{s}");
    }
    // a v4-mapped peer is an ipv4 address (SockadrToNetadr)
    let m: std::net::SocketAddr = "[::ffff:10.1.2.3]:77".parse().unwrap();
    let n = NetAdr::from_socket(&m);
    assert_eq!((n.kind, n.ip, n.port), (NetAdrType::Ip, [10, 1, 2, 3], 77));
    // a scoped link-local address keeps its scope and prints it
    let s: std::net::SocketAddr = "[fe80::1%1]:3".parse().unwrap();
    let sa = NetAdr::from_socket(&s);
    assert_eq!(sa.scope_id, 1);
    assert!(sa.to_string_port().starts_with("[fe80::1%"), "{}", sa.to_string_port());
    // net_enabled: the family priority of Sys_StringToAdr. One test owns
    // the global, restored at the end.
    let old = netchan::net_enabled();
    netchan::set_net_enabled(NET_ENABLEV4);
    assert_eq!(string_to_adr("::1", NetAdrType::Unspec).0, 0, "ipv6 is off");
    assert_eq!(string_to_adr("127.0.0.1:5", NetAdrType::Unspec).1.kind, NetAdrType::Ip);
    netchan::set_net_enabled(NET_ENABLEV4 | NET_ENABLEV6 | NET_PRIOV6);
    assert_eq!(string_to_adr("::1", NetAdrType::Unspec).1.kind, NetAdrType::Ip6);
    netchan::set_net_enabled(old);
}

fn ipv6_available() -> bool {
    match std::net::UdpSocket::bind("[::1]:0") {
        Ok(_) => true,
        Err(e) => {
            eprintln!("skipped: no ipv6 on this host ({e})");
            false
        }
    }
}

#[test]
fn connect_over_ipv6_loopback() {
    if !ipv6_available() {
        return;
    }
    let mut snet = Net::new();
    let log = snet.open_ip(NetSrc::Server, "127.0.0.1", 0, "::1", 0);
    assert!(snet.is_open6(NetSrc::Server), "{log}");
    let port6 = snet.ports6[NetSrc::Server as usize];
    let mut sv = Server::spawn(World::new(MapId::Arena), "arena", ServerCvars::default());
    let mut net = Net::new();
    net.open_client();
    assert!(net.is_open6(NetSrc::Client));
    let mut cl = NetClient::new(0);
    cl.userinfo = "\\name\\Six\\cl_guid\\0123456789ABCDEF0123456789ABCDEF".into();
    cl.connect_family(&mut net, &format!("[::1]:{port6}"), NetAdrType::Ip6).unwrap();
    for _ in 0..600 {
        sv.frame(&mut snet, 16);
        cl.frame(&mut net, 16);
        if cl.gamestate_new {
            cl.cgame_loaded(&mut net);
        }
        cl.create_cmd(Default::default());
        cl.send_cmd(&mut net);
        if cl.state == ConnState::Active {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    assert_eq!(cl.state, ConnState::Active, "{:?} {}", cl.error, cl.print_out);
    assert_eq!(cl.netchan.remote_address.kind, NetAdrType::Ip6);
    let i = cl.client_num as usize;
    assert_eq!(sv.world.clients[i].ip.str, "::1");
    assert!(sv.status().contains(&format!("[::1]:{}", net.ports6[NetSrc::Client as usize])));
}

#[test]
fn lan_scan_finds_a_server_through_the_multicast_group() {
    if !ipv6_available() {
        return;
    }
    let mut snet = Net::new();
    // the scan goes to the four server ports: bind the first free of them
    let log = snet.open_ip(NetSrc::Server, "127.0.0.1", PORT_SERVER, "", PORT_SERVER);
    let log2 = snet.join_multicast6(NetSrc::Server);
    if !snet.in_multicast6(NetSrc::Server) {
        eprintln!("skipped: the multicast group cannot be joined here: {log}{log2}");
        return;
    }
    let p6 = snet.ports6[NetSrc::Server as usize];
    if !(PORT_SERVER..PORT_SERVER + netchan::NUM_SERVER_PORTS).contains(&p6) {
        eprintln!("skipped: no free server port");
        return;
    }
    let mut sv = Server::spawn(World::new(MapId::Arena), "arena", ServerCvars { hostname: "mcast".into(), ..Default::default() });
    let mut net = Net::new();
    let mut cl = NetClient::new(0);
    cl.local_servers(&mut net, false);
    for _ in 0..100 {
        sv.frame(&mut snet, 16);
        cl.frame(&mut net, 16);
        if cl.local_servers.iter().any(|s| s.adr.kind == NetAdrType::Ip6) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(cl.local_servers.iter().any(|s| s.adr.kind == NetAdrType::Ip6 && s.adr.port == p6), "{:?}", cl.local_servers);
}
