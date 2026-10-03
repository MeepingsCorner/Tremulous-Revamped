//! VoIP end to end (dec038): a server and two clients over localhost UDP;
//! one client's synthetic microphone (48 kHz PCM) goes through
//! `CL_CaptureVoip` (Opus encode), `CL_WritePacket`'s clc_voipOpus,
//! `SV_UserVoip` / `SV_WriteVoipToClient` and `CL_ParseVoip` (Opus decode)
//! into the other client's raw streams; the decoded voice matches the
//! source (cl_voipCaptureMult applied) with a usable SNR. Targets (spatial,
//! all, a client list), voip ignore / muteall, VAD and the meter too.
use std::collections::HashMap;
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::cgview::Cgame;
use trem_sim::kernel::cl::{ConnState, NetClient};
use trem_sim::kernel::netchan::{Net, NetSrc};
use trem_sim::kernel::sv::{Server, ServerCvars};
use trem_sim::kernel::types::UserCmd;
use trem_sim::kernel::voip::{self, VoipHost, VoipPlayback, CL_VOIP_CVARS, MAX_CLIENTS};
use trem_sim::kernel::world::World;

/// The engine around a client: cvars with `modified` flags and a
/// synthetic capture device delivering 48 samples per millisecond.
struct TestHost {
    cvars: HashMap<String, (String, bool)>,
    capturing: bool,
    avail: usize,
    phase: usize,
    freq: f32,
    amp: f32,
    /// what S_Capture returned, in order
    captured: Vec<i16>,
    gain: f32,
    starts: usize,
}

impl TestHost {
    fn new(freq: f32, amp: f32) -> TestHost {
        let mut cvars = HashMap::new();
        for (n, v, _) in CL_VOIP_CVARS {
            cvars.insert(n.to_ascii_lowercase(), (v.to_string(), true));
        }
        cvars.insert("rate".into(), ("25000".into(), true));
        TestHost { cvars, capturing: false, avail: 0, phase: 0, freq, amp, captured: Vec::new(), gain: 1.0, starts: 0 }
    }
    fn set(&mut self, n: &str, v: &str) {
        VoipHost::cvar_set(self, n, v);
    }
    /// the sound card's clock
    fn tick(&mut self, ms: i32) {
        if self.capturing {
            self.avail += 48 * ms as usize;
        }
    }
}

impl VoipHost for TestHost {
    fn cvar_string(&self, name: &str) -> String {
        self.cvars.get(&name.to_ascii_lowercase()).map(|v| v.0.clone()).unwrap_or_default()
    }
    fn cvar_set(&mut self, name: &str, value: &str) {
        let e = self.cvars.entry(name.to_ascii_lowercase()).or_insert((String::new(), false));
        if e.0 != value {
            e.0 = value.to_string();
            e.1 = true;
        }
    }
    fn cvar_take_modified(&mut self, name: &str) -> bool {
        self.cvars.get_mut(&name.to_ascii_lowercase()).is_some_and(|v| std::mem::take(&mut v.1))
    }
    fn start_capture(&mut self) {
        self.capturing = true;
        self.avail = 0;
        self.starts += 1;
    }
    fn available_capture_samples(&mut self) -> usize {
        self.avail
    }
    fn capture(&mut self, samples: usize) -> Vec<i16> {
        let n = samples.min(self.avail);
        self.avail -= n;
        let out: Vec<i16> = (0..n)
            .map(|k| {
                let t = (self.phase + k) as f32 / 48000.0;
                // a tone with a slow amplitude wobble, like speech energy
                ((t * self.freq * std::f32::consts::TAU).sin() * self.amp * (0.75 + 0.25 * (t * 3.0 * std::f32::consts::TAU).sin())) as i16
            })
            .collect();
        self.phase += n;
        self.captured.extend_from_slice(&out);
        out
    }
    fn stop_capture(&mut self) {
        self.capturing = false;
    }
    fn master_gain(&mut self, gain: f32) {
        self.gain = gain;
    }
}

struct Peer {
    net: Net,
    cl: NetClient,
    cg: Option<Cgame>,
    view: Option<World>,
    host: TestHost,
    heard: Vec<VoipPlayback>,
}

impl Peer {
    fn new(name: &str, guid: &str, server: &str, voip: bool, freq: f32) -> Peer {
        let mut net = Net::new();
        net.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
        let mut cl = NetClient::new(0);
        let proto = if voip { "\\cl_voipProtocol\\opus" } else { "" };
        cl.userinfo = format!("\\name\\{name}\\cl_guid\\{guid}\\rate\\25000\\snaps\\20{proto}");
        cl.connect(&mut net, server).unwrap();
        Peer { net, cl, cg: None, view: None, host: TestHost::new(freq, 6000.0), heard: Vec::new() }
    }

    /// `CL_Frame`: packets, cgame, usercmd, then `CL_CaptureVoip`.
    fn frame(&mut self, msec: i32) {
        self.host.tick(msec);
        self.cl.frame(&mut self.net, msec);
        if self.cl.gamestate_new {
            self.view = Some(World::client_view(MapId::Arena));
            self.cg = Some(Cgame::init(&self.cl));
            self.cl.cgame_loaded(&mut self.net);
        }
        if let (Some(cg), Some(v)) = (self.cg.as_mut(), self.view.as_mut()) {
            if self.cl.state == ConnState::Active {
                cg.frame(&mut self.cl, v);
            }
        }
        self.cl.create_cmd(UserCmd::default());
        self.cl.send_cmd(&mut self.net);
        self.cl.capture_voip(&mut self.host);
        self.heard.append(&mut self.cl.voip.playback);
    }
}

fn step(sv: &mut Server, snet: &mut Net, peers: &mut [Peer], ms: i32) {
    sv.frame(snet, ms);
    for p in peers.iter_mut() {
        p.frame(ms);
    }
    std::thread::sleep(std::time::Duration::from_micros(200));
}

fn setup(voip_b: bool) -> (Server, Net, Vec<Peer>) {
    let mut snet = Net::new();
    let port = snet.open(NetSrc::Server, "127.0.0.1", 0).unwrap();
    let w = World::new(MapId::Arena);
    let sv = Server::spawn(w, "arena", ServerCvars { hostname: "voip test".into(), ..Default::default() });
    let addr = format!("127.0.0.1:{port}");
    let peers = vec![Peer::new("Talker", "0123456789ABCDEF0123456789ABCDEF", &addr, true, 440.0), Peer::new("Listener", "FEDCBA9876543210FEDCBA9876543210", &addr, voip_b, 300.0)];
    let (mut sv, mut snet, mut peers) = (sv, snet, peers);
    for _ in 0..400 {
        step(&mut sv, &mut snet, &mut peers, 16);
        if peers.iter().all(|p| p.cl.state == ConnState::Active) && sv.active_clients() == 2 {
            break;
        }
    }
    for p in &peers {
        assert_eq!(p.cl.state, ConnState::Active, "{:?}", p.cl.error);
    }
    (sv, snet, peers)
}

/// Talk for `ms` milliseconds (+voiprecord held), then release and drain.
fn talk(sv: &mut Server, snet: &mut Net, peers: &mut [Peer], ms: i32) {
    peers[0].host.set("cl_voipSend", voip::voip_record(true));
    for _ in 0..ms / 16 {
        step(sv, snet, peers, 16);
    }
    peers[0].host.set("cl_voipSend", voip::voip_record(false));
    for _ in 0..40 {
        step(sv, snet, peers, 16);
    }
}

/// Best SNR (dB) of `got` against `src` over the codec delay.
fn snr(src: &[i16], got: &[i16]) -> f64 {
    let n = src.len().min(got.len());
    assert!(n > 20000, "only {n} samples");
    (0..1200)
        .map(|d| {
            let (mut sig, mut err) = (0f64, 0f64);
            for i in 3000..n - 1300 {
                let a = src[i] as f64;
                let b = got[i + d] as f64;
                sig += a * a;
                err += (a - b) * (a - b);
            }
            10.0 * (sig / err.max(1.0)).log10()
        })
        .fold(f64::MIN, f64::max)
}

#[test]
fn voice_goes_from_one_client_to_the_other() {
    let (mut sv, mut snet, mut peers) = setup(true);
    let (a, b) = (peers[0].cl.client_num as usize, peers[1].cl.client_num as usize);
    // the server announces opus in CS_SYSTEMINFO, the codec starts at the first snapshot
    assert!(peers[0].cl.voip.enabled && peers[0].cl.voip.codec_initialized);
    assert!(sv.clients[a].voip.has_voip && sv.clients[b].voip.has_voip);
    assert_eq!(peers[0].host.cvar_string("cl_voipSendTarget"), "spatial");
    // 2 s of talking with the default spatial target
    talk(&mut sv, &mut snet, &mut peers, 2000);
    assert_eq!(peers[0].host.starts, 1, "S_StartCapture once");
    assert!(!peers[0].host.capturing, "S_StopCapture on release");
    assert_eq!(peers[0].host.gain, 1.0, "S_MasterGain back to 1");
    assert!(peers[0].heard.is_empty(), "own voice is not sent back");
    let heard = std::mem::take(&mut peers[1].heard);
    assert!(!heard.is_empty(), "the listener got voice");
    assert!(heard.iter().all(|p| p.sender == a && p.entity == a as i32 && p.stream == a + MAX_CLIENTS + 1 && p.volume == 1.0), "spatial only");
    let got: Vec<i16> = heard.iter().flat_map(|p| p.samples.iter().copied()).collect();
    // cl_voipCaptureMult 2.0 scales the captured samples before encoding
    let mult = peers[0].host.cvar_value("cl_voipCaptureMult");
    let src: Vec<i16> = peers[0].host.captured.iter().map(|&s| (s as f32 * mult) as i16).collect();
    assert!(got.len() >= src.len() * 9 / 10, "{} of {} samples", got.len(), src.len());
    let db = snr(&src, &got);
    println!("VoIP over UDP: {} samples, SNR {db:.1} dB", got.len());
    assert!(db > 12.0, "SNR {db:.1} dB");

    // "all": direct streams at the listener's gain
    peers[0].host.set("cl_voipSendTarget", "all");
    talk(&mut sv, &mut snet, &mut peers, 400);
    let heard = std::mem::take(&mut peers[1].heard);
    assert!(!heard.is_empty() && heard.iter().all(|p| p.stream == a + 1 && p.entity == -1));
    // a client list without the listener and without spatial: nothing reaches it
    let other = (0..MAX_CLIENTS).find(|&i| i != a && i != b).unwrap();
    peers[0].host.set("cl_voipSendTarget", &format!("{other},200"));
    talk(&mut sv, &mut snet, &mut peers, 400);
    assert!(peers[1].heard.is_empty());
    assert!(peers[0].cl.print_out.contains("VoIP target 200 is not a valid client number") || peers[0].cl.print_out.is_empty());
    // the listener's own number plus spatial: both streams
    peers[0].host.set("cl_voipSendTarget", &format!("spatial,{b}"));
    talk(&mut sv, &mut snet, &mut peers, 400);
    let heard = std::mem::take(&mut peers[1].heard);
    assert!(heard.iter().any(|p| p.stream == a + 1) && heard.iter().any(|p| p.stream == a + MAX_CLIENTS + 1));

    // voip gain 0 on the listener: too quiet to play; voip ignore: the server stops relaying
    peers[1].cl.voip_f(&["voip".into(), "gain".into(), a.to_string(), "0".into()]);
    talk(&mut sv, &mut snet, &mut peers, 400);
    assert!(peers[1].heard.is_empty());
    peers[1].cl.voip_f(&["voip".into(), "gain".into(), a.to_string(), "1.5".into()]);
    peers[1].cl.voip_f(&["voip".into(), "ignore".into(), a.to_string()]);
    for _ in 0..20 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    assert!(sv.clients[b].voip.ignore_voip_from_client[a], "the reliable 'voip ignore' reached the server");
    talk(&mut sv, &mut snet, &mut peers, 400);
    assert!(peers[1].heard.is_empty());
    peers[1].cl.voip_f(&["voip".into(), "unignore".into(), a.to_string()]);
    peers[1].cl.voip_f(&["voip".into(), "muteall".into()]);
    for _ in 0..20 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    assert!(sv.clients[b].voip.mute_all_voip);
    talk(&mut sv, &mut snet, &mut peers, 400);
    assert!(peers[1].heard.is_empty());
    peers[1].cl.voip_f(&["voip".into(), "unmuteall".into()]);
    for _ in 0..20 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    talk(&mut sv, &mut snet, &mut peers, 400);
    let heard = std::mem::take(&mut peers[1].heard);
    assert!(!heard.is_empty() && heard.iter().all(|p| p.stream != a + 1 || p.volume == 1.5), "gain applies to the direct stream");
    let out = std::mem::take(&mut peers[1].cl.print_out);
    assert!(out.contains(&format!("VoIP: Now ignoring player #{a}")) && out.contains("VoIP: muting incoming voice"), "{out}");

    // the meter while talking, VAD below the threshold sends nothing
    peers[0].host.set("cl_voipSend", "1");
    for _ in 0..10 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    let meter = peers[0].cl.voip_meter(true, true, true).unwrap();
    assert!(meter.starts_with("VoIP: [*") && meter.len() == "VoIP: [          ]".len(), "{meter}");
    peers[0].host.set("cl_voipSend", "0");
    for _ in 0..20 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    assert!(peers[0].cl.voip_meter(true, false, true).is_none());
    peers[1].heard.clear();
    peers[0].host.amp = 50.0;
    peers[0].host.set("cl_voipUseVAD", "1");
    for _ in 0..60 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    assert_eq!(peers[0].host.cvar_integer("cl_voipSend"), 1, "VAD keeps cl_voipSend on");
    assert!(peers[1].heard.is_empty(), "silence under cl_voipVADThreshold is not sent");
    peers[0].host.amp = 6000.0;
    for _ in 0..60 {
        step(&mut sv, &mut snet, &mut peers, 16);
    }
    assert!(!peers[1].heard.is_empty(), "speech over the threshold is");
    peers[0].host.set("cl_voipUseVAD", "0");

    // disconnect: the capture stops, the codec goes
    let p = &mut peers[0];
    p.cl.disconnect(&mut p.net);
    p.cl.capture_voip(&mut p.host);
    assert!(!peers[0].host.capturing && peers[0].host.cvar_integer("cl_voipSend") == 0);
    assert!(!peers[0].cl.voip.codec_initialized && !peers[0].cl.voip.enabled);
}

#[test]
fn no_voip_without_support_or_rate() {
    // a listener without cl_voipProtocol gets nothing; sv_voip 0 relays nothing
    let (mut sv, mut snet, mut peers) = setup(false);
    let b = peers[1].cl.client_num as usize;
    assert!(!sv.clients[b].voip.has_voip);
    talk(&mut sv, &mut snet, &mut peers, 400);
    assert!(peers[1].heard.is_empty());
    // a too slow rate turns cl_voip off
    peers[0].host.set("rate", "8000");
    step(&mut sv, &mut snet, &mut peers, 16);
    assert_eq!(peers[0].host.cvar_integer("cl_voip"), 0);
    assert_eq!(peers[0].host.cvar_string("cl_voipProtocol"), "");
    assert!(peers[0].cl.print_out.contains("Your network rate is too slow for VoIP."));
    // the voip command refuses without a VoIP server
    sv.cvars.voip = 0;
    peers[1].cl.voip.enabled = false;
    peers[1].cl.voip_f(&["voip".into(), "muteall".into()]);
    assert!(peers[1].cl.print_out.contains("VoIP: command ignored: Server doesn't support VoIP"));
    assert!(!sv.systeminfo().contains("sv_voipProtocol"));
    sv.cvars.voip = 1;
    assert!(sv.systeminfo().contains("\\sv_voipProtocol\\opus"));
}
