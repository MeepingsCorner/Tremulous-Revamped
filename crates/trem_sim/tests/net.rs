//! Network layer evidence (dec033).
//!
//! * `oracle_stream`: the same deterministic sequence as re/harness/net_ref.c
//!   (the original msg.c / huffman.c / net_chan.c compiled natively) produced
//!   with the port; the SHA-256 of the hex lines must equal re/traces/net.sha256.
//! * round trips: Huffman (adaptive and the netchan tree), bits, strings,
//!   usercmd / entityState / playerState deltas, netchan fragment reassembly.
use sha2::{Digest, Sha256};
use trem_sim::kernel::huffman;
use trem_sim::kernel::msg::*;
use trem_sim::kernel::netchan::*;
use trem_sim::kernel::types::{PlayerState, UserCmd, MAX_PERSISTANT, MAX_STATS};

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0 >> 8
    }
    fn next2(&mut self) -> u32 {
        let a = self.next();
        let b = self.next();
        a ^ (b << 8)
    }
    fn word(&mut self, bits: i32) -> i32 {
        let r = self.next() % 10;
        if r < 4 {
            return 0;
        }
        if bits == 0 {
            let f = if r < 7 { ((self.next() % 8000) as i32 - 4000) as f32 } else { ((self.next() % 2000000) as i32 - 1000000) as f32 / 64.0 };
            return f.to_bits() as i32;
        }
        let ab = bits.unsigned_abs();
        let mut v = if r < 7 { self.next() % 256 } else { self.next2() };
        if ab < 32 {
            v &= (1u32 << ab) - 1;
        }
        v as i32
    }
}

fn set<T>(s: &mut T, off: usize, v: i32) {
    unsafe { std::ptr::write((s as *mut T as *mut u8).add(off) as *mut i32, v) }
}

fn gen_es(r: &mut Lcg) -> EntityState {
    let mut es = EntityState { number: (r.next() % (MAX_GENTITIES as u32 - 1)) as i32, ..Default::default() };
    for f in ENTITY_STATE_FIELDS.iter() {
        let v = r.word(f.bits);
        set(&mut es, f.offset, v);
    }
    es
}

fn gen_ps(r: &mut Lcg) -> PlayerState {
    let mut ps = PlayerState::default();
    for f in PLAYER_STATE_FIELDS.iter() {
        let v = r.word(f.bits);
        set(&mut ps, f.offset, v);
    }
    for i in 0..MAX_STATS {
        ps.stats[i] = r.word(16) as i16 as i32;
    }
    for i in 0..MAX_PERSISTANT {
        ps.persistant[i] = r.word(16) as i16 as i32;
    }
    for i in 0..MAX_MISC {
        ps.misc[i] = r.word(32);
    }
    ps
}

fn gen_cmd(r: &mut Lcg, base: i32) -> UserCmd {
    let mut c = UserCmd { server_time: base + (r.next() % 600) as i32, ..Default::default() };
    c.angles[0] = (r.next() % 65536) as i32;
    c.angles[1] = (r.next() % 65536) as i32;
    c.angles[2] = if r.next() % 4 != 0 { 0 } else { (r.next() % 65536) as i32 };
    c.buttons = if r.next() % 4 != 0 { 0 } else { (r.next() % 65536) as i32 };
    c.weapon = (r.next() % 26) as u8;
    c.forwardmove = if r.next() % 3 != 0 { 127 } else { ((r.next() % 256) as i32 - 128) as i8 };
    c.rightmove = ((r.next() % 256) as i32 - 128) as i8;
    c.upmove = if r.next() % 2 != 0 { 0 } else { 127 };
    c
}

fn hex(out: &mut String, tag: &str, d: &[u8]) {
    out.push_str(&format!("{tag} {} ", d.len()));
    for b in d {
        out.push_str(&format!("{b:02x}"));
    }
    out.push('\n');
}

/// The port's rendition of re/harness/net_ref.c.
fn oracle_text() -> String {
    let mut out = String::new();
    let mut r = Lcg(12345);
    // 1. raw bits
    let mut m = Msg::new(MAX_MSGLEN);
    for _ in 0..3000 {
        let bits = 1 + (r.next() % 32) as i32;
        let v = r.next2();
        if r.next() % 3 == 0 && bits < 32 {
            m.write_bits(v as i32, -bits);
        } else {
            m.write_bits(v as i32, bits);
        }
    }
    hex(&mut out, "bits", m.bytes());
    // 2. strings and scalars
    let mut m = Msg::new(MAX_MSGLEN);
    for _ in 0..200 {
        let n = r.next() % 40;
        let s: Vec<u8> = (0..n).map(|_| (1 + r.next() % 255) as u8).collect();
        m.write_string_bytes(&s);
        m.write_byte((r.next() % 256) as i32);
        m.write_short((r.next() % 65536) as i32 - 32768);
        m.write_long(r.next2() as i32);
        m.write_float(((r.next() % 100000) as i32 - 50000) as f32 / 16.0);
        m.write_angle16((r.next() % 3600) as f32 / 10.0);
    }
    hex(&mut out, "str", m.bytes());
    // 3. usercmds
    let mut m = Msg::new(MAX_MSGLEN);
    let mut a = UserCmd::default();
    for _ in 0..400 {
        let key = r.next2() as i32;
        let mut b = gen_cmd(&mut r, a.server_time);
        if r.next() % 4 == 0 {
            b = a;
            b.server_time += 16;
        }
        m.write_delta_usercmd_key(key, &a, &b);
        a = b;
    }
    hex(&mut out, "cmd", m.bytes());
    // 4. entities
    for _ in 0..40 {
        let mut m = Msg::new(MAX_MSGLEN);
        for _ in 0..20 {
            let a = gen_es(&mut r);
            let mut b = gen_es(&mut r);
            b.number = a.number;
            match r.next() % 4 {
                0 => m.write_delta_entity(Some(&a), Some(&b), false),
                1 => m.write_delta_entity(Some(&a), Some(&a), true),
                2 => m.write_delta_entity(Some(&a), None, true),
                _ => m.write_delta_entity(Some(&EntityState::default()), Some(&b), true),
            }
        }
        hex(&mut out, "ent", m.bytes());
    }
    // 5. playerstates
    for _ in 0..40 {
        let mut m = Msg::new(MAX_MSGLEN);
        let a = gen_ps(&mut r);
        let b = gen_ps(&mut r);
        if r.next() % 3 == 0 {
            m.write_delta_playerstate(None, &b);
        } else {
            m.write_delta_playerstate(Some(&a), &b);
        }
        hex(&mut out, "ps", m.bytes());
    }
    // 6. adaptive Huffman
    let text = b"\xff\xff\xff\xffconnect \"\\cl_guid\\0123456789ABCDEF0123456789ABCDEF\\name\\Player\\rate\\25000\\snaps\\20\\protocol\\71\\qport\\4242\\challenge\\-1234567\"";
    let mut d = text.to_vec();
    huffman::compress(&mut d, 12);
    hex(&mut out, "huff", &d);
    huffman::decompress(&mut d, 12, MAX_MSGLEN * 2);
    hex(&mut out, "unhuff", &d);
    // 7. netchan
    let mut net = Net::new();
    net.capture = Some(Vec::new());
    let adr = NetAdr { kind: NetAdrType::Ip, ..Default::default() };
    let mut chan = Netchan::setup(NetSrc::Client, adr, 4242, -1234567);
    for len in [0usize, 100, 1299, 1300, 2600, 4000, 16000] {
        let data: Vec<u8> = (0..len).map(|_| r.next() as u8).collect();
        chan.transmit(&mut net, &data, 0);
        while chan.unsent_fragments {
            chan.transmit_next_fragment(&mut net, 0);
        }
    }
    let mut chan = Netchan::setup(NetSrc::Server, adr, 4242, 98765);
    let data: Vec<u8> = (0..3000).map(|_| r.next() as u8).collect();
    chan.transmit(&mut net, &data, 0);
    while chan.unsent_fragments {
        chan.transmit_next_fragment(&mut net, 0);
    }
    for (_, p) in net.capture.take().unwrap() {
        hex(&mut out, "pkt", &p);
    }
    out
}

#[test]
fn oracle_stream() {
    let text = oracle_text();
    if let Ok(p) = std::env::var("NET_REF_OUT") {
        std::fs::write(p, &text).unwrap();
    }
    let got = format!("{:x}", Sha256::digest(text.as_bytes()));
    let want = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/traces/net.sha256")).unwrap();
    assert_eq!(got, want.split_whitespace().next().unwrap(), "msg/huffman/netchan stream differs from re/harness/net_ref (NET_REF_OUT=path to dump)");
}

#[test]
fn huffman_round_trips() {
    let mut r = Lcg(7);
    for len in [0usize, 1, 13, 200, 1400, 9000] {
        let mut d: Vec<u8> = b"\xff\xff\xff\xffconnect ".to_vec();
        let payload: Vec<u8> = (0..len).map(|i| if i % 3 == 0 { r.next() as u8 } else { b"\\name\\x"[i % 7] }).collect();
        d.extend_from_slice(&payload);
        let orig = d.clone();
        huffman::compress(&mut d, 12);
        huffman::decompress(&mut d, 12, MAX_MSGLEN * 2);
        assert_eq!(d, orig, "adaptive huffman len {len}");
    }
    // the netchan tree: every symbol decodes to itself
    let h = huffman::msg_huff();
    for c in 0..256 {
        let code = &h.codes[c];
        let mut i = 0;
        let s = h.tree.receive(&mut || {
            let b = code[i];
            i += 1;
            b
        });
        assert_eq!(s, c as i32);
        assert_eq!(i, code.len());
    }
}

#[test]
fn bits_and_strings_round_trip() {
    let mut r = Lcg(99);
    let mut m = Msg::new(MAX_MSGLEN);
    let mut want = Vec::new();
    for _ in 0..2000 {
        let bits = 1 + (r.next() % 32) as i32;
        // MSG_ReadBits sign-extends from bit (bits & ~7) - 1 when bits is
        // not a multiple of 8 (the C subtracts the raw low bits first); the
        // tables only use signed 8 / 16 bit fields, so test those
        let signed = bits < 32 && bits % 8 == 0 && r.next() % 2 == 0;
        let mut v = r.next2() as i32;
        if bits < 32 {
            v &= ((1u64 << bits) - 1) as i32;
            if signed && v & (1 << (bits - 1)) != 0 {
                v |= (-1i64 ^ ((1i64 << bits) - 1)) as i32;
            }
        }
        m.write_bits(v, if signed { -bits } else { bits });
        want.push((v, if signed { -bits } else { bits }));
    }
    m.write_string("hello \u{1}world");
    m.write_big_string(&"x".repeat(3000));
    m.write_float(-1.5);
    m.write_angle16(90.0);
    let mut rd = Msg::from_bytes(m.bytes());
    for (v, b) in want {
        assert_eq!(rd.read_bits(b), v, "bits {b}");
    }
    assert_eq!(rd.read_string(), "hello \u{1}world");
    assert_eq!(rd.read_big_string().len(), 3000);
    assert_eq!(rd.read_float(), -1.5);
    assert_eq!(rd.read_angle16(), 90.0);
    // past the end: -1
    assert_eq!(rd.read_byte(), -1);
    // OOB mode is plain little endian
    let mut o = Msg::new_oob(64);
    o.write_long(-1);
    o.write_short(0x1234);
    o.write_byte(7);
    assert_eq!(o.bytes(), &[0xff, 0xff, 0xff, 0xff, 0x34, 0x12, 7]);
}

#[test]
fn usercmd_delta_round_trip() {
    let mut r = Lcg(5);
    let mut m = Msg::new(MAX_MSGLEN);
    let mut cmds = Vec::new();
    let mut a = UserCmd::default();
    for _ in 0..300 {
        let mut b = gen_cmd(&mut r, a.server_time);
        if b.forwardmove == -128 {
            b.forwardmove = -127;
        }
        if b.rightmove == -128 {
            b.rightmove = -127;
        }
        cmds.push(b);
        a = b;
    }
    let key = 0x1234_5678;
    let mut prev = UserCmd::default();
    for c in &cmds {
        m.write_delta_usercmd_key(key, &prev, c);
        prev = *c;
    }
    let mut rd = Msg::from_bytes(m.bytes());
    let mut prev = UserCmd::default();
    for c in &cmds {
        let got = rd.read_delta_usercmd_key(key, &prev);
        assert_eq!(&got, c);
        prev = got;
    }
}

/// A value that survives the field's bit width (what the game puts there).
fn fit(v: i32, bits: i32) -> i32 {
    if bits == 0 || bits.abs() >= 32 {
        return v;
    }
    let ab = bits.abs();
    let m = v & ((1 << ab) - 1);
    if bits < 0 && m & (1 << (ab - 1)) != 0 {
        m | (-1 ^ ((1 << ab) - 1))
    } else {
        m
    }
}

#[test]
fn entity_and_playerstate_delta_round_trip() {
    let mut r = Lcg(77);
    for _ in 0..200 {
        let mut a = gen_es(&mut r);
        let mut b = gen_es(&mut r);
        b.number = a.number;
        for f in ENTITY_STATE_FIELDS.iter() {
            for s in [&mut a, &mut b] {
                let v = unsafe { std::ptr::read((s as *const EntityState as *const u8).add(f.offset) as *const i32) };
                set(s, f.offset, fit(v, f.bits));
            }
        }
        let mut m = Msg::new(MAX_MSGLEN);
        m.write_delta_entity(Some(&a), Some(&b), true);
        m.write_delta_entity(Some(&a), None, true);
        let mut rd = Msg::from_bytes(m.bytes());
        let n = rd.read_bits(GENTITYNUM_BITS);
        assert_eq!(n, b.number);
        assert_eq!(rd.read_delta_entity(&a, n), b);
        let n = rd.read_bits(GENTITYNUM_BITS);
        assert_eq!(rd.read_delta_entity(&a, n).number, MAX_GENTITIES - 1, "removed");
    }
    for _ in 0..200 {
        let mut a = gen_ps(&mut r);
        let mut b = gen_ps(&mut r);
        for f in PLAYER_STATE_FIELDS.iter() {
            for s in [&mut a, &mut b] {
                let v = unsafe { std::ptr::read((s as *const PlayerState as *const u8).add(f.offset) as *const i32) };
                set(s, f.offset, fit(v, f.bits));
            }
        }
        let mut m = Msg::new(MAX_MSGLEN);
        m.write_delta_playerstate(Some(&a), &b);
        m.write_delta_playerstate(None, &b);
        let mut rd = Msg::from_bytes(m.bytes());
        assert_eq!(rd.read_delta_playerstate(Some(&a)), b);
        assert_eq!(rd.read_delta_playerstate(None), b);
    }
}

#[test]
fn netchan_fragments_reassemble() {
    let mut net = Net::new();
    let mut srv = Netchan::setup(NetSrc::Server, NetAdr::loopback(), 0, 555);
    let mut cl = Netchan::setup(NetSrc::Client, NetAdr::loopback(), 1234, 555);
    for len in [0usize, 10, FRAGMENT_SIZE - 1, FRAGMENT_SIZE, FRAGMENT_SIZE * 2, 9000, 16000] {
        let data: Vec<u8> = (0..len).map(|i| (i * 7 + len) as u8).collect();
        srv.transmit(&mut net, &data, 0);
        while srv.unsent_fragments {
            srv.transmit_next_fragment(&mut net, 0);
        }
        let mut got = None;
        while let Some((_, p)) = net.get_packet(NetSrc::Client) {
            let mut m = Msg::from_bytes(&p);
            if cl.process(&mut m) {
                got = Some(m.data[m.readcount..m.cursize].to_vec());
            }
        }
        assert_eq!(got.as_deref(), Some(&data[..]), "len {len}");
    }
    // a lost middle fragment drops the message; the next one still arrives
    let data = vec![9u8; FRAGMENT_SIZE * 3];
    srv.transmit(&mut net, &data, 0);
    let mut n = 0;
    while srv.unsent_fragments {
        srv.transmit_next_fragment(&mut net, 0);
        n += 1;
    }
    assert!(n >= 2);
    let mut pkts = Vec::new();
    while let Some((_, p)) = net.get_packet(NetSrc::Client) {
        pkts.push(p);
    }
    pkts.remove(1);
    for p in pkts {
        let mut m = Msg::from_bytes(&p);
        assert!(!cl.process(&mut m));
    }
    srv.transmit(&mut net, b"after", 0);
    let (_, p) = net.get_packet(NetSrc::Client).unwrap();
    let mut m = Msg::from_bytes(&p);
    assert!(cl.process(&mut m));
    assert_eq!(&m.data[m.readcount..m.cursize], b"after");
    assert_eq!(cl.dropped, 1);
    // client -> server carries the qport; a bad checksum is refused
    // (sequence 1's checksum is 0 for any challenge, so use the second)
    cl.transmit(&mut net, b"first", 0);
    let (_, p) = net.get_packet(NetSrc::Server).unwrap();
    assert!(srv.process(&mut Msg::from_bytes(&p)));
    cl.transmit(&mut net, b"cmd", 0);
    let (_, p) = net.get_packet(NetSrc::Server).unwrap();
    let mut m = Msg::from_bytes(&p);
    let mut bad = Netchan::setup(NetSrc::Server, NetAdr::loopback(), 1234, 556);
    assert!(!bad.process(&mut m.clone()));
    assert!(srv.process(&mut m));
    assert_eq!(&m.data[m.readcount..m.cursize], b"cmd");
}

#[test]
fn addresses() {
    let (r, a) = string_to_adr("127.0.0.1:30721", NetAdrType::Unspec);
    assert_eq!(r, 1);
    assert_eq!(a.to_string_port(), "127.0.0.1:30721");
    let (r, b) = string_to_adr("127.0.0.1", NetAdrType::Unspec);
    assert_eq!((r, b.port), (2, PORT_SERVER));
    assert!(compare_base_adr(&a, &b) && !compare_adr(&a, &b));
    assert_eq!(string_to_adr("localhost", NetAdrType::Unspec).1.kind, NetAdrType::Loopback);
    let (_, c) = string_to_adr("192.168.1.77", NetAdrType::Unspec);
    let (_, d) = string_to_adr("192.168.1.200", NetAdrType::Unspec);
    assert!(compare_base_adr_mask(&c, &d, 24) && !compare_base_adr_mask(&c, &d, 25));
    assert!(is_lan_address(&c));
    let (_, e) = string_to_adr("[::1]:30720", NetAdrType::Unspec);
    assert_eq!(e.kind, NetAdrType::Ip6);
    assert_eq!(e.to_string_port(), "[::1]:30720");
    assert_eq!(string_to_adr("no.such.host.invalid", NetAdrType::Unspec).0, 0);
}
