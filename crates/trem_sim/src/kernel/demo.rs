//! Kernel `demo` — client-side demos of client/cl_main.c and cl_cgame.c
//! (dec037): `CL_Record_f` (the gamestate message rebuilt from the client's
//! configstrings and baselines), `CL_WriteDemoMessage` (every server message
//! after the netchan header, once a non-delta snapshot arrived),
//! `CL_StopRecord_f`, `CL_DemoFilename`, `CL_PlayDemo_f` / `CL_WalkDemoExt`
//! (demos/<name>.dm_<protocol>), `CL_ReadDemoMessage`, `CL_DemoCompleted`
//! with the timedemo statistics (`CL_DemoFrameDurationSDev`,
//! cl_timedemoLog), the demo half of `CL_SetCGameTime` (timedemo's fixed 50
//! ms steps, reading messages until the cgame has a snapshot to
//! interpolate to) and `CL_Frame`'s cl_autoRecordDemo.
//!
//! The file format is the original's: per message the little-endian
//! sequence number and length, then the bytes; two -1 longs end it. Demo
//! files live in `DemoState::dir` (the host's user://demos).

use super::cl::{ConnState, NetClient};
use super::msg::{EntityState, Msg, MAX_GENTITIES, MAX_MSGLEN};
use super::netchan::{self, Net};
use super::sv::{svc, PROTOCOL_VERSION};
use super::svgame::{self, cs};
use std::io::Write;
use std::path::PathBuf;

/// `DEMOEXT`: the standard demo extension (followed by the protocol).
pub const DEMOEXT: &str = "dm_";
/// `demo_protocols[]` (common.c).
pub const DEMO_PROTOCOLS: [i32; 1] = [PROTOCOL_VERSION];
/// `MAX_TIMEDEMO_DURATIONS`.
pub const MAX_TIMEDEMO_DURATIONS: usize = 4096;

/// The demo members of `clientConnection_t` and the demo cvars.
#[derive(Debug, Default)]
pub struct DemoState {
    /// `clc.demorecording` / `clc.demoplaying`
    pub recording: bool,
    pub playing: bool,
    /// `clc.demowaiting`: don't record until a non-delta message is received
    pub waiting: bool,
    /// `clc.firstDemoFrameSkipped`
    pub first_frame_skipped: bool,
    /// `clc.demoName`
    pub name: String,
    /// the file being written (`clc.demofile` while recording)
    writer: Option<std::io::BufWriter<std::fs::File>>,
    /// the path being written
    pub path: Option<PathBuf>,
    /// the file being played (`clc.demofile` while playing), read whole
    data: Vec<u8>,
    pos: usize,
    /// where demos are (the host's "demos" folder)
    pub dir: Option<PathBuf>,
    /// `timedemo`, `cl_timedemoLog`, `cl_freezeDemo`, `cl_autoRecordDemo`
    pub timedemo: bool,
    pub timedemo_log: String,
    pub freeze: bool,
    pub auto_record: bool,
    /// `clc.timeDemo*`
    pub time_demo_frames: i32,
    pub time_demo_start: i32,
    pub time_demo_last_frame: i32,
    pub time_demo_base_time: i32,
    pub time_demo_min_duration: i32,
    pub time_demo_max_duration: i32,
    time_demo_durations: Vec<u8>,
    /// set by `CL_DemoCompleted`: the host goes back to the menu and runs
    /// `nextdemo` (CL_NextDemo)
    pub completed: bool,
    /// a `record [name]` the host asked for before the client was active
    /// (a single-player game restarted on the loopback, dec037): recorded
    /// at CA_ACTIVE ("" = the next free demoNNNN)
    pub pending: Option<String>,
}

impl DemoState {
    /// Close the demo file (`CL_Disconnect`'s FS_FCloseFile( clc.demofile )).
    pub fn close(&mut self) {
        if let Some(mut w) = self.writer.take() {
            let _ = w.flush();
        }
        self.recording = false;
        self.playing = false;
        self.waiting = false;
        self.data.clear();
        self.pos = 0;
    }

    fn write(&mut self, b: &[u8]) {
        if let Some(w) = self.writer.as_mut() {
            let _ = w.write_all(b);
        }
    }

    /// `FS_Read` of 4 bytes.
    fn read_long(&mut self) -> Option<i32> {
        let b = self.data.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// `CL_DemoFilename`: "demo%i%i%i%i".
pub fn demo_filename(number: i32) -> String {
    let n = if !(0..=9999).contains(&number) { 9999 } else { number };
    format!("demo{}{}{}{}", n / 1000, n / 100 % 10, n / 10 % 10, n % 10)
}

/// `Com_RealTime` as "%04d%02d%02d%02d%02d%02d" (local time).
pub fn real_time_string() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    #[cfg(unix)]
    {
        // SAFETY: localtime_r fills the zeroed tm from a valid time_t
        unsafe {
            let t: libc::time_t = secs as libc::time_t;
            let mut tm: libc::tm = std::mem::zeroed();
            if !libc::localtime_r(&t, &mut tm).is_null() {
                return format!("{:04}{:02}{:02}{:02}{:02}{:02}", 1900 + tm.tm_year, 1 + tm.tm_mon, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec);
            }
        }
    }
    // UTC (civil from days, H. Hinnant)
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}{:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

impl NetClient {
    fn demo_path(&self, file: &str) -> Option<PathBuf> {
        self.demo.dir.as_ref().map(|d| d.join(file))
    }

    /// `CL_Record_f`: `record [demoname]`; begins recording a demo from the
    /// current position. Returns the console output.
    pub fn record(&mut self, name: Option<&str>) -> String {
        let mut out = String::new();
        if self.demo.recording {
            return "Already recording.\n".into();
        }
        if self.state != ConnState::Active {
            return "You must be in a level to record.\n".into();
        }
        // the demo format is legacy's (dm_71); a protocol 69 connection's
        // messages are not recorded (dec040)
        if self.vanilla() {
            return "Demos of protocol 69 (Tremulous 1.1.0) servers are not recorded.\n".into();
        }
        // sync 0 doesn't prevent recording, so not forcing it off .. everyone does g_sync 1 ; record ; g_sync 0 ..
        if netchan::is_local_address(&self.server_address) {
            out.push_str("^3WARNING: You should set 'g_synchronousClients 1' for smoother demo recording\n");
        }
        let Some(dir) = self.demo.dir.clone() else {
            out.push_str("ERROR: couldn't open.\n");
            return out;
        };
        let demo_name = match name {
            Some(n) => n.to_string(),
            None => {
                // scan for a free demo name
                let mut n = demo_filename(9999);
                for number in 0..=9999 {
                    n = demo_filename(number);
                    if !dir.join(format!("{n}.{DEMOEXT}{PROTOCOL_VERSION}")).exists() {
                        break; // file doesn't exist
                    }
                }
                n
            }
        };
        let file = format!("{demo_name}.{DEMOEXT}{PROTOCOL_VERSION}");
        // open the demo file
        out.push_str(&format!("recording to demos/{file}.\n"));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(&file);
        match std::fs::File::create(&path) {
            Ok(f) => self.demo.writer = Some(std::io::BufWriter::new(f)),
            Err(_) => {
                out.push_str("ERROR: couldn't open.\n");
                return out;
            }
        }
        self.demo.path = Some(path);
        self.demo.recording = true;
        self.demo.name = demo_name;
        // don't start saving messages until a non-delta compressed message is received
        self.demo.waiting = true;
        // write out the gamestate message
        let mut buf = Msg::new(MAX_MSGLEN);
        buf.bitstream();
        // NOTE, MRE: all server->client messages now acknowledge
        buf.write_long(self.reliable_sequence);
        buf.write_byte(svc::GAMESTATE);
        buf.write_long(self.server_command_sequence);
        // configstrings
        for i in 0..cs::MAX_CONFIGSTRINGS {
            if self.configstrings[i].is_empty() {
                continue;
            }
            buf.write_byte(svc::CONFIGSTRING);
            buf.write_short(i as i32);
            buf.write_big_string(&self.configstrings[i]);
        }
        // baselines
        let null = EntityState::default();
        for i in 0..MAX_GENTITIES as usize {
            let ent = self.entity_baselines[i];
            if ent.number == 0 {
                continue;
            }
            buf.write_byte(svc::BASELINE);
            buf.write_delta_entity(Some(&null), Some(&ent), true);
        }
        buf.write_byte(svc::EOF);
        // finished writing the gamestate stuff
        // write the client num
        buf.write_long(self.client_num);
        // write the checksum feed
        buf.write_long(self.checksum_feed);
        // finished writing the client packet
        buf.write_byte(svc::EOF);
        // write it to the demo file
        let seq = self.server_message_sequence - 1;
        self.demo.write(&seq.to_le_bytes());
        self.demo.write(&(buf.cursize as i32).to_le_bytes());
        let b = buf.bytes().to_vec();
        self.demo.write(&b);
        // the rest of the demo file will be copied from net messages
        out
    }

    /// `CL_WriteDemoMessage`: dumps the current net message, prefixed by
    /// the sequence and the length.
    pub(crate) fn write_demo_message(&mut self, m: &Msg, header_bytes: usize) {
        // write the packet sequence
        let seq = self.server_message_sequence;
        self.demo.write(&seq.to_le_bytes());
        // skip the packet sequencing information
        let len = m.cursize.saturating_sub(header_bytes);
        self.demo.write(&(len as i32).to_le_bytes());
        let b = m.data[header_bytes..header_bytes + len].to_vec();
        self.demo.write(&b);
    }

    /// `CL_StopRecord_f`.
    pub fn stop_record(&mut self) -> String {
        if !self.demo.recording {
            return "Not recording a demo.\n".into();
        }
        // finish up
        self.demo.write(&(-1i32).to_le_bytes());
        self.demo.write(&(-1i32).to_le_bytes());
        if let Some(mut w) = self.demo.writer.take() {
            let _ = w.flush();
        }
        self.demo.recording = false;
        "Stopped demo.\n".into()
    }

    /// `CL_WalkDemoExt`: demos/<arg>.dm_<protocol> for the protocols demos
    /// are kept for; the protocol, or -1.
    fn walk_demo_ext(&mut self, arg: &str) -> (i32, String) {
        let name = format!("demos/{arg}.{DEMOEXT}{PROTOCOL_VERSION}");
        if let Some(d) = self.demo_path(&format!("{arg}.{DEMOEXT}{PROTOCOL_VERSION}")).and_then(|p| std::fs::read(p).ok()) {
            self.printf(&format!("Demo file: {name}\n"));
            self.demo.data = d;
            return (PROTOCOL_VERSION, name);
        }
        self.printf(&format!("Not found: {name}\n"));
        for &p in DEMO_PROTOCOLS.iter().filter(|&&p| p != PROTOCOL_VERSION) {
            let n = format!("demos/{arg}.{DEMOEXT}{p}");
            if let Some(d) = self.demo_path(&format!("{arg}.{DEMOEXT}{p}")).and_then(|p| std::fs::read(p).ok()) {
                self.printf(&format!("Demo file: {n}\n"));
                self.demo.data = d;
                return (p, n);
            }
            self.printf(&format!("Not found: {n}\n"));
        }
        (-1, name)
    }

    /// `CL_PlayDemo_f` after the host's `CL_Disconnect` (and killing a
    /// local server): open the demo, read messages until the gamestate is
    /// in. Err: `Com_Error( ERR_DROP, "couldn't open %s" )`.
    pub fn play_demo(&mut self, arg: &str) -> Result<(), String> {
        self.demo.data.clear();
        self.demo.pos = 0;
        self.demo.completed = false;
        // check for an extension .DEMOEXT_?? (?? is protocol)
        let name = match arg.rfind('.') {
            Some(dot) if arg[dot + 1..].len() >= DEMOEXT.len() && arg[dot + 1..dot + 1 + DEMOEXT.len()].eq_ignore_ascii_case(DEMOEXT) => {
                let protocol = super::spawn::c_atoi(&arg[dot + 1 + DEMOEXT.len()..]);
                if DEMO_PROTOCOLS.contains(&protocol) || protocol == PROTOCOL_VERSION {
                    if let Some(d) = self.demo_path(arg).and_then(|p| std::fs::read(p).ok()) {
                        self.demo.data = d;
                    }
                    format!("demos/{arg}")
                } else {
                    self.printf(&format!("Protocol {protocol} not supported for demos\n"));
                    self.walk_demo_ext(&arg[..dot]).1
                }
            }
            _ => self.walk_demo_ext(arg).1,
        };
        if self.demo.data.is_empty() {
            return Err(format!("couldn't open {name}"));
        }
        self.demo.name = arg.to_string();
        self.state = ConnState::Connected;
        self.demo.playing = true;
        self.voip.demoplaying = true; // clc.demoplaying for voip.rs and `video`
        self.servername = arg.to_string();
        self.demo.time_demo_frames = 0;
        self.demo.time_demo_start = 0;
        self.demo.time_demo_durations.clear();
        // read demo messages until connected (the gamestate: the host loads
        // the map before the cgame takes the first snapshot)
        while self.state >= ConnState::Connected && self.state < ConnState::Loading && self.demo.playing {
            self.read_demo_message();
        }
        // don't get the first snapshot this frame, to prevent the long
        // time from the gamestate load from messing causing a time skip
        self.demo.first_frame_skipped = false;
        Ok(())
    }

    /// `CL_ReadDemoMessage`.
    pub fn read_demo_message(&mut self) {
        if self.demo.data.is_empty() {
            self.demo_completed();
            return;
        }
        // get the sequence number
        let Some(s) = self.demo.read_long() else {
            self.demo_completed();
            return;
        };
        self.server_message_sequence = s;
        // get the length
        let Some(len) = self.demo.read_long() else {
            self.demo_completed();
            return;
        };
        if len == -1 {
            self.demo_completed();
            return;
        }
        if len < 0 || len as usize > MAX_MSGLEN {
            self.drop_error("CL_ReadDemoMessage: demoMsglen > MAX_MSGLEN");
            self.demo_completed();
            return;
        }
        let Some(b) = self.demo.data.get(self.demo.pos..self.demo.pos + len as usize).map(|b| b.to_vec()) else {
            self.printf("Demo file was truncated.\n");
            self.demo_completed();
            return;
        };
        self.demo.pos += len as usize;
        self.last_packet_time = self.realtime;
        let mut buf = Msg::from_bytes(&b);
        // a demo has no downloads: nothing is sent from here
        let mut none = Net::new();
        self.parse_server_message(&mut none, &mut buf);
    }

    /// `CL_DemoFrameDurationSDev`.
    fn demo_frame_duration_sdev(&self) -> f32 {
        let n = ((self.demo.time_demo_frames - 1).max(0) as usize).min(MAX_TIMEDEMO_DURATIONS).min(self.demo.time_demo_durations.len());
        if n == 0 {
            return 0.0;
        }
        let d = &self.demo.time_demo_durations[..n];
        let mean = d.iter().map(|&x| x as f32).sum::<f32>() / n as f32;
        let variance = d.iter().map(|&x| (x as f32 - mean) * (x as f32 - mean)).sum::<f32>() / n as f32;
        variance.sqrt()
    }

    /// `CL_DemoCompleted`: the timedemo report, then `CL_Disconnect` (the
    /// host shows the main menu and runs `nextdemo`).
    pub fn demo_completed(&mut self) {
        if self.demo.timedemo {
            let time = super::cl::sys_milliseconds() - self.demo.time_demo_start;
            if time > 0 {
                // Millisecond times are frame durations:
                // minimum/average/maximum/std deviation
                let f = self.demo.time_demo_frames;
                let buffer = format!(
                    "{} frames {:.1} seconds {:.1} fps {}.0/{:.1}/{}.0/{:.1} ms\n",
                    f,
                    time as f64 / 1000.0,
                    f as f64 * 1000.0 / time as f64,
                    self.demo.time_demo_min_duration,
                    time as f32 / f.max(1) as f32,
                    self.demo.time_demo_max_duration,
                    self.demo_frame_duration_sdev()
                );
                self.printf(&buffer);
                // Write a log of all the frame durations
                if !self.demo.timedemo_log.is_empty() {
                    let n = ((f - 1).max(0) as usize).min(MAX_TIMEDEMO_DURATIONS).min(self.demo.time_demo_durations.len());
                    let mut log = format!("# {buffer}");
                    for d in &self.demo.time_demo_durations[..n] {
                        log.push_str(&format!("{d}\n"));
                    }
                    let path = self.home_path.as_ref().map(|h| h.join(&self.demo.timedemo_log)).unwrap_or_else(|| PathBuf::from(&self.demo.timedemo_log));
                    if std::fs::write(&path, log).is_ok() {
                        self.printf(&format!("{} written\n", self.demo.timedemo_log));
                    } else {
                        self.printf(&format!("Couldn't open {} for writing\n", self.demo.timedemo_log));
                    }
                }
            }
        }
        self.demo.completed = true;
        let mut none = Net::new();
        self.disconnect(&mut none);
    }

    /// The demo half of `CL_SetCGameTime`: timedemo's deterministic time
    /// samples, then read messages until the cgame has valid snapshots to
    /// interpolate between.
    pub(crate) fn demo_time_frame(&mut self) {
        // a timedemo will always use a deterministic set of time samples
        // no matter what speed machine it is run on,
        // while a normal demo may have different time samples
        // each time it is played back
        if self.demo.timedemo {
            let now = super::cl::sys_milliseconds();
            if self.demo.time_demo_start == 0 {
                self.demo.time_demo_start = now;
                self.demo.time_demo_last_frame = now;
                self.demo.time_demo_min_duration = i32::MAX;
                self.demo.time_demo_max_duration = 0;
            }
            let mut frame_duration = now - self.demo.time_demo_last_frame;
            self.demo.time_demo_last_frame = now;
            // Ignore the first measurement as it'll always be 0
            if self.demo.time_demo_frames > 0 {
                self.demo.time_demo_max_duration = self.demo.time_demo_max_duration.max(frame_duration);
                self.demo.time_demo_min_duration = self.demo.time_demo_min_duration.min(frame_duration);
                // 255 ms = about 4fps
                frame_duration = frame_duration.min(255);
                let k = (self.demo.time_demo_frames - 1) as usize % MAX_TIMEDEMO_DURATIONS;
                if self.demo.time_demo_durations.len() <= k {
                    self.demo.time_demo_durations.resize(k + 1, 0);
                }
                self.demo.time_demo_durations[k] = frame_duration as u8;
            }
            self.demo.time_demo_frames += 1;
            self.server_time = self.demo.time_demo_base_time + self.demo.time_demo_frames * 50;
        }
        while self.server_time >= self.snap.server_time {
            // feed another messag, which should change
            // the contents of cl.snap
            self.read_demo_message();
            if self.state != ConnState::Active {
                return; // end of demo
            }
        }
    }

    /// `CL_Frame`'s cl_autoRecordDemo: start a demo named after the time,
    /// the server and the map when active, stop it when not.
    pub(crate) fn auto_record_frame(&mut self) {
        if self.state == ConnState::Active && !self.demo.playing {
            if let Some(p) = self.demo.pending.take() {
                let s = self.record(if p.is_empty() { None } else { Some(&p) });
                self.printf(&s);
            }
        }
        if !self.demo.auto_record {
            return;
        }
        if self.state == ConnState::Active && !self.demo.recording && !self.demo.playing {
            // If not recording a demo, and we should be, start one
            let now = real_time_string();
            // Replace the ":" in the address as it is not a valid
            // file name character
            let server = self.servername.replacen(':', ".", 1);
            let map = svgame::info_value(&self.configstrings[cs::SERVERINFO], "mapname");
            let map = map.rsplit(['/', '\\']).next().unwrap_or("").to_string();
            let map = map.split('.').next().unwrap_or("").to_string();
            let s = self.record(Some(&format!("{now}-{server}-{map}")));
            self.printf(&s);
        } else if self.state != ConnState::Active && self.demo.recording {
            // Recording, but not CA_ACTIVE, so stop recording
            let s = self.stop_record();
            self.printf(&s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_filenames() {
        assert_eq!(demo_filename(0), "demo0000");
        assert_eq!(demo_filename(42), "demo0042");
        assert_eq!(demo_filename(1234), "demo1234");
        assert_eq!(demo_filename(-5), "demo9999");
        assert_eq!(real_time_string().len(), 14);
    }

    /// A demo file is the original's: (sequence, length, bytes)* then -1 -1.
    #[test]
    fn demo_write_read_round_trip() {
        let dir = std::env::temp_dir().join(format!("trem_demo_rt_{}", std::process::id()));
        let mut cl = NetClient::new(1);
        cl.demo.dir = Some(dir.clone());
        // a fake active connection with some gamestate
        cl.state = ConnState::Active;
        cl.configstrings[cs::SERVERINFO] = "\\mapname\\arena\\sv_hostname\\x".into();
        cl.configstrings[cs::SYSTEMINFO] = "\\sv_serverid\\7".into();
        cl.entity_baselines[5].number = 5;
        cl.entity_baselines[5].e_type = 3;
        cl.client_num = 2;
        cl.checksum_feed = 99;
        cl.server_message_sequence = 10;
        let out = cl.record(Some("rt"));
        assert!(out.contains("recording to demos/rt.dm_71."), "{out}");
        assert!(cl.demo.waiting);
        // a message as received: the 4-byte netchan header (out of band
        // bytes) then a server command in the bit stream
        let mut m = Msg::new_oob(MAX_MSGLEN);
        m.write_long(11);
        let header = m.cursize;
        m.bitstream();
        m.write_long(0);
        m.write_byte(svc::SERVER_COMMAND);
        m.write_long(1);
        m.write_string("print \"hello\"");
        m.write_byte(svc::EOF);
        cl.demo.waiting = false;
        cl.server_message_sequence = 11;
        cl.write_demo_message(&m, header);
        assert_eq!(cl.stop_record(), "Stopped demo.\n");
        let bytes = std::fs::read(dir.join("rt.dm_71")).unwrap();
        // gamestate: seq = 9, then the message, then -1 -1
        assert_eq!(&bytes[..4], &9i32.to_le_bytes());
        assert_eq!(&bytes[bytes.len() - 8..], &[0xff; 8]);
        // play it back: the gamestate restores configstrings and baselines
        let mut p = NetClient::new(2);
        p.demo.dir = Some(dir.clone());
        p.play_demo("rt").unwrap();
        assert!(p.demo.playing);
        assert_eq!(p.state, ConnState::Loading);
        assert!(p.gamestate_new);
        assert_eq!(p.client_num, 2);
        assert_eq!(p.checksum_feed, 99);
        assert_eq!(svgame::info_value(&p.configstrings[cs::SERVERINFO], "mapname"), "arena");
        assert_eq!(p.server_id, 7);
        assert_eq!(p.entity_baselines[5].e_type, 3);
        // the next message holds the server command, then the end
        p.read_demo_message();
        assert_eq!(p.server_message_sequence, 11);
        assert_eq!(p.server_command_sequence, 1);
        assert_eq!(p.server_commands[1], "print \"hello\"");
        p.read_demo_message();
        assert!(p.demo.completed && !p.demo.playing && p.state == ConnState::Disconnected);
        // with the extension, and a missing demo
        let mut q = NetClient::new(3);
        q.demo.dir = Some(dir.clone());
        assert!(q.play_demo("rt.dm_71").is_ok());
        let mut q = NetClient::new(4);
        q.demo.dir = Some(dir.clone());
        assert_eq!(q.play_demo("nope"), Err("couldn't open demos/nope.dm_71".into()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
