//! Demos (dec037): a client records a networked session (CL_Record_f's
//! gamestate, then every server message from the first non-delta
//! snapshot), and playing the demo back drives the client exactly as the
//! connection did: every recorded snapshot comes back identical, the
//! configstrings and server commands too; timedemo reports its frames.
use std::collections::BTreeMap;
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::cgview::Cgame;
use trem_sim::kernel::cl::{ConnState, NetClient};
use trem_sim::kernel::netchan::{Net, NetSrc};
use trem_sim::kernel::sv::{Server, ServerCvars};
use trem_sim::kernel::types::UserCmd;
use trem_sim::kernel::world::World;

/// Every snapshot the client holds since `last` (CL_GetSnapshot), as text.
fn collect(cl: &NetClient, last: &mut i32, out: &mut BTreeMap<i32, String>) {
    let cur = cl.snap.message_num;
    for n in (*last + 1).max(cur - 31)..=cur {
        if let Some(s) = cl.get_snapshot(n) {
            out.insert(n, format!("{} {} {:?} {:?} {:?}", s.server_time, s.snap_flags, s.areamask, s.ps, s.entities));
        }
    }
    *last = cur;
}

#[test]
fn recorded_session_replays_the_same_snapshots() {
    let dir = std::env::temp_dir().join(format!("trem_demo_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut snet = Net::new();
    let port = snet.open(NetSrc::Server, "127.0.0.1", 0).unwrap();
    let mut w = World::new(MapId::Arena);
    w.level.cheats = true;
    let mut sv = Server::spawn(w, "arena", ServerCvars { hostname: "demo".into(), ..Default::default() });
    let mut net = Net::new();
    net.open(NetSrc::Client, "127.0.0.1", 0).unwrap();
    let mut cl = NetClient::new(0);
    cl.userinfo = "\\name\\Recorder\\cl_guid\\0123456789ABCDEF0123456789ABCDEF".into();
    cl.demo.dir = Some(dir.clone());
    cl.connect(&mut net, &format!("127.0.0.1:{port}")).unwrap();
    let mut view: Option<World> = None;
    let mut cg: Option<Cgame> = None;
    let mut recorded = BTreeMap::new();
    let mut last = 0;
    let mut cmds = Vec::new();
    let mut frame = 0;
    let mut record_at = None;
    let mut gamestate = Vec::new();
    loop {
        frame += 1;
        assert!(frame < 3000, "{:?} {}", cl.error, cl.print_out);
        sv.frame(&mut snet, 16);
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
        if cl.state == ConnState::Active && record_at.is_none() {
            // join and spawn, then record
            cl.add_reliable_command("team humans", false).unwrap();
            cl.add_reliable_command("class rifle", false).unwrap();
            record_at = Some(frame + 60);
        }
        if record_at == Some(frame) {
            let out = cl.record(Some("session"));
            gamestate = cl.configstrings.clone();
            assert!(out.contains("recording to demos/session.dm_71."), "{out}");
            last = cl.snap.message_num;
        }
        // walk and turn
        let mut cmd = UserCmd { forwardmove: 127, ..Default::default() };
        cmd.angles[1] = (frame * 300) & 0xffff;
        cl.create_cmd(cmd);
        cl.send_cmd(&mut net);
        if cl.demo.recording && !cl.demo.waiting {
            collect(&cl, &mut last, &mut recorded);
            let cur = cl.server_command_sequence;
            if cmds.len() < cur as usize {
                cmds.resize(cur as usize, String::new());
            }
            if cur > 0 {
                cmds[cur as usize - 1] = cl.server_commands[cur as usize & 63].clone();
            }
        }
        if recorded.len() >= 60 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    assert_eq!(cl.stop_record(), "Stopped demo.\n");
    cl.disconnect(&mut net);
    let file = dir.join("session.dm_71");
    assert!(std::fs::metadata(&file).unwrap().len() > 1000);
    // play it back: a fresh client fed from the file (CL_PlayDemo_f)
    let mut p = NetClient::new(0);
    p.demo.dir = Some(dir.clone());
    p.play_demo("session").unwrap();
    assert_eq!(p.state, ConnState::Loading);
    assert_eq!(p.configstrings, gamestate, "the gamestate of CL_Record_f");
    let mut pview = World::client_view(MapId::Arena);
    let mut pcg = Cgame::init(&p);
    let mut pnet = Net::new();
    p.cgame_loaded(&mut pnet);
    let mut replayed = BTreeMap::new();
    let mut plast = 0;
    let mut seen_active = false;
    for _ in 0..5000 {
        p.frame(&mut pnet, 16);
        if p.state == ConnState::Active {
            seen_active = true;
            pcg.frame(&mut p, &mut pview);
            collect(&p, &mut plast, &mut replayed);
        }
        if p.demo.completed || (seen_active && p.state == ConnState::Disconnected) {
            break;
        }
    }
    assert!(seen_active, "{}", p.print_out);
    assert!(p.state == ConnState::Disconnected && !p.demo.playing, "CL_DemoCompleted");
    assert!(pnet.packets_sent == 0, "a demo sends nothing");
    // every recorded snapshot came back identical
    let mut same = 0;
    for (n, s) in &recorded {
        let r = replayed.get(n).unwrap_or_else(|| panic!("snapshot {n} missing in playback ({} replayed)", replayed.len()));
        assert_eq!(r, s, "snapshot {n}");
        same += 1;
    }
    assert!(same >= 60);
    // the cgame of the playback saw the player walk
    let me = p.client_num as usize;
    assert!(pview.clients[me].ps.origin != [0.0; 3]);

    // timedemo: deterministic 50 ms steps and the frame report
    let mut t = NetClient::new(0);
    t.demo.dir = Some(dir.clone());
    t.demo.timedemo = true;
    t.play_demo("session.dm_71").unwrap();
    t.cgame_loaded(&mut pnet);
    for _ in 0..5000 {
        t.frame(&mut pnet, 1);
        // the report needs some real time to pass (time > 0)
        std::thread::sleep(std::time::Duration::from_millis(1));
        if t.state == ConnState::Disconnected {
            break;
        }
    }
    assert!(t.print_out.contains(" frames ") && t.print_out.contains(" fps "), "{}", t.print_out);
    let _ = std::fs::remove_dir_all(dir);
}
