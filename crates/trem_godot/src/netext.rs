//! Godot glue of dec037 (a child module of game.rs, so it reaches
//! TremGame's state): pk3 downloads into user://base with the search path
//! restart, the pure search path of a pure server's client, the ui's
//! download prompt (com_downloadPrompt / download_popmenu) and the connect
//! screen's download line (`UI_DisplayDownloadInfo`), demos (`record`,
//! `stoprecord`, `demo`, timedemo, cl_autoRecordDemo; files in
//! user://demos), the single-player game recorded over the loopback,
//! `connect -4 / -6`, the net_ip.c cvars, and the headless dedicated server
//! (`godot --headless --path godot -- +set dedicated 1 +map atcs`: no
//! presentation, the server console on stdin as `Sys_ConsoleInput`,
//! heartbeats with `dedicated 2`).

use super::*;
use std::path::PathBuf;
use trem_sim::kernel::cl::{ConnState, DLP_SHOW};
use trem_sim::kernel::files::{PakSet, BASEGAME};
use trem_sim::kernel::nethost::{self, HostEvent};

/// This glue's state on `TremGame`.
#[derive(Default)]
pub(crate) struct NetExt {
    /// `com_dedicated`: no client, no presentation
    pub dedicated: bool,
    /// `Sys_ConsoleInput`: stdin lines from a reader thread
    stdin: Option<std::sync::mpsc::Receiver<String>>,
    /// the download prompt text for the ui (`com_downloadPromptText`)
    prompt: Option<String>,
}

/// A folder of the user data (user://<sub>), globalized.
pub(crate) fn user_dir(sub: &str) -> PathBuf {
    PathBuf::from(godot::classes::ProjectSettings::singleton().globalize_path(&format!("user://{sub}")).to_string())
}

/// FS_Startup's game folders for the pk3 list: the folders of the pk3s
/// (lowest priority last in the hosts' list), and user://base (fs_homepath,
/// where downloads go) as the highest even while it is empty.
fn pak_dirs(pk3_paths: &[String]) -> Vec<(PathBuf, String)> {
    let home = user_dir(BASEGAME);
    let mut dirs: Vec<(PathBuf, String)> = Vec::new();
    for p in pk3_paths {
        let d = std::path::Path::new(p).parent().map(|d| d.to_path_buf()).unwrap_or_default();
        if d != home && !dirs.iter().any(|(x, _)| *x == d) {
            dirs.push((d, BASEGAME.into()));
        }
    }
    dirs.reverse();
    dirs.push((home, BASEGAME.into()));
    dirs
}

impl TremGame {
    /// `NET_Init` / `CL_Init` / `FS_InitFilesystem` parts of dec037 on the
    /// new network host: the search paths, fs_homepath, the demos folder,
    /// the client cvars of downloads and demos.
    pub(super) fn netext_setup(&mut self) {
        use crate::cmd::{CVAR_ARCHIVE, CVAR_ROM};
        const CVAR_TEMP: i32 = 0x0100; // q_shared.h (not kept by the port's store)
        for (n, v, f) in [
            ("cl_allowDownload", "0", CVAR_ARCHIVE),
            // the network mode (dec040): 0 Auto, 71 legacy 1.3, 69 vanilla 1.1.0
            ("cl_netProtocol", "0", CVAR_ARCHIVE),
            ("cl_autoRecordDemo", "0", CVAR_ARCHIVE),
            ("cl_timedemoLog", "", CVAR_ARCHIVE),
            ("timedemo", "0", 0),
            ("cl_freezeDemo", "0", CVAR_TEMP),
            ("com_downloadPrompt", "0", CVAR_ROM),
            ("com_downloadPromptText", "", CVAR_TEMP),
        ] {
            self.client.cvars.get(n, v, f);
        }
        // the old defaults (25000 / 30 / 20) throttled downloads and updates:
        // a saved old default becomes the new one (servers clamp them)
        for (n, old, new) in [("rate", "25000", "90000"), ("cl_maxpackets", "30", "125"), ("snaps", "20", "40")] {
            if self.client.cvars.find(n).is_some_and(|v| v.string == old) {
                self.client.cvars.set(n, new);
            }
        }
        let dirs = pak_dirs(&self.pk3_paths);
        let Some(nh) = self.nethost.as_mut() else { return };
        nh.cl.fs = Some(PakSet::new(&dirs));
        nh.cl.home_path = Some(user_dir(""));
        nh.cl.demo.dir = Some(user_dir("demos"));
        nh.server_fs = Some(PakSet::new(&dirs));
    }

    /// Every frame with the other client cvars: the net_ip.c cvars.
    pub(super) fn netext_sync(&mut self) {
        let dedicated = self.netext.dedicated;
        let Some(mut nh) = self.nethost.take() else { return };
        {
            let get = |n: &str| self.net_cvar(n);
            nethost::apply_net_cvars(&mut nh, &get, dedicated);
        }
        self.nethost = Some(nh);
    }

    /// `map` makes a listen server on the loopback to record a single
    /// player game (cl_autoRecordDemo, or a `record` waiting for it).
    pub(super) fn wants_loopback_server(&self) -> bool {
        self.net_cvar("cl_autoRecordDemo").is_some_and(|v| trem_sim::kernel::spawn::c_atoi(&v) != 0) || self.nethost.as_ref().is_some_and(|n| n.cl.demo.pending.is_some())
    }

    /// The pk3s a map of a server is loaded from: a pure server's paks only
    /// (`FS_PureServerSetLoadedPaks`), else all.
    pub(super) fn client_load_paths(&self) -> Vec<String> {
        match self.nethost.as_ref().and_then(|n| n.cl.fs.as_ref().filter(|_| n.cl.connected_to_pure)) {
            Some(fs) => fs.effective_paths(),
            None => self.pk3_paths.clone(),
        }
    }

    /// `CL_Connect_f`'s arguments: [-4|-6] server [-1|-69|-71] (the
    /// protocol flag of dec040; without it cl_netProtocol decides).
    pub(super) fn netext_connect(&mut self, args: &[String]) -> String {
        let Some((family, server, protocol)) = nethost::parse_connect_args(args) else {
            return "usage: connect [-4|-6] server [-1|-69|-71]\n".into();
        };
        self.ensure_nethost();
        if self.world.is_some() || self.loading.is_some() || self.net_active() {
            self.loading = None;
            self.shutdown_map();
        }
        self.net_error = None;
        self.sync_net_cvars(true);
        let nh = self.nethost.as_mut().unwrap();
        let r = nh.connect_protocol(&server, family, protocol);
        let mut out = std::mem::take(&mut nh.out);
        if let Err(e) = r {
            out.push_str(&format!("{e}\n"));
        }
        self.update_connect_screen();
        out
    }

    /// The engine commands of dec037: record, stoprecord, demo, and
    /// fs_openedList / fs_referencedList (`CL_OpenedPK3List_f`,
    /// `CL_ReferencedPK3List_f`).
    pub(super) fn netext_command(&mut self, argv: &[String]) -> Option<String> {
        let c = argv.first()?.to_ascii_lowercase();
        match c.as_str() {
            "record" => {
                if argv.len() > 2 {
                    return Some("record <demoname>\n".into());
                }
                let active = self.nethost.as_ref().is_some_and(|n| n.cl.state == ConnState::Active);
                if active || self.net_active() {
                    self.sync_net_cvars(false);
                    let nh = self.nethost.as_mut().unwrap();
                    let mut out = nh.command(argv).unwrap_or_default();
                    out.push_str(&std::mem::take(&mut nh.out));
                    return Some(out);
                }
                // the single-process game: the original records the loopback
                // client of its local server, so the map restarts on one (dec037)
                let CurrentMap::Bsp(paths, map) = &self.current else { return Some("You must be in a level to record.\n".into()) };
                if self.world.is_none() {
                    return Some("You must be in a level to record.\n".into());
                }
                let (paths, map) = (paths.clone(), map.clone());
                let cheats = self.world.as_ref().is_some_and(|w| w.level.cheats);
                self.ensure_nethost();
                if let Some(nh) = self.nethost.as_mut() {
                    nh.cl.demo.pending = Some(argv.get(1).cloned().unwrap_or_default());
                }
                let err = self.begin_load(&paths, &map, Vec::new(), cheats);
                if !err.is_empty() {
                    if let Some(nh) = self.nethost.as_mut() {
                        nh.cl.demo.pending = None;
                    }
                    return Some(format!("{err}\n"));
                }
                Some(format!("record: {map} restarts on the local server's loopback to be recorded\n"))
            }
            "fs_openedlist" | "fs_referencedlist" => {
                let fs = self.nethost.as_ref().and_then(|n| n.cl.fs.as_ref());
                let (names, label) = match (c.as_str(), fs) {
                    ("fs_openedlist", Some(f)) => (f.loaded_pak_names(), "Opened"),
                    (_, Some(f)) => (f.referenced_pak_names(), "Referenced"),
                    ("fs_openedlist", None) => (String::new(), "Opened"),
                    _ => (String::new(), "Referenced"),
                };
                Some(format!("{label} PK3 Names: {names}\n"))
            }
            "stoprecord" => {
                let nh = self.nethost.as_mut()?;
                nh.cl.demo.pending = None;
                Some(nh.cl.stop_record())
            }
            "demo" => {
                if argv.len() != 2 {
                    return Some("demo <demoname>\n".into());
                }
                self.ensure_nethost();
                self.sync_net_cvars(false);
                // CL_Disconnect, and a local server is killed
                self.loading = None;
                self.shutdown_map();
                self.net_error = None;
                let nh = self.nethost.as_mut().unwrap();
                let r = nh.play_demo(&argv[1]);
                let mut out = std::mem::take(&mut nh.out);
                if let Err(e) = r {
                    // Com_Error( ERR_DROP, "couldn't open %s" )
                    self.net_drop(e.clone());
                    out.push_str(&format!("{e}\n"));
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// The events of dec037; true when the frame must end (the map went).
    pub(super) fn netext_event(&mut self, ev: &HostEvent) -> bool {
        match ev {
            HostEvent::FsRestart => {
                // FS_Restart: a downloaded pak joins the search path
                self.pk3_paths = find_pk3s();
                // the server's fs_game folder (a mod's downloaded paks)
                if let Some(fs) = self.nethost.as_ref().and_then(|nh| nh.cl.fs.as_ref()) {
                    for p in fs.game_pak_paths() {
                        if !self.pk3_paths.contains(&p) {
                            self.pk3_paths.push(p);
                        }
                    }
                }
                self.client_fs = Some(crate::hostfs::HostFs::new(&self.pk3_paths));
                godot_print!("trem: search path restarted: {} pk3 file(s)", self.pk3_paths.len());
                false
            }
            HostEvent::DemoCompleted => {
                // CL_Disconnect( qtrue ) (the ui comes back), then `CL_NextDemo`
                self.shutdown_map();
                let next = self.client.cvars.find("nextdemo").map(|v| v.string.clone()).unwrap_or_default();
                if !next.is_empty() {
                    self.client.cvars.set("nextdemo", "");
                    let _ = self.exec(GString::from(next.as_str()));
                }
                true
            }
            _ => false,
        }
    }

    /// Once per rendered frame of a network session: `CL_Frame`'s download
    /// prompt check; a prompt to show is kept for the ui.
    pub(super) fn netext_frame(&mut self) {
        let Some(nh) = self.nethost.as_mut() else { return };
        nh.ui_frame();
        for l in std::mem::take(&mut nh.out).lines() {
            godot_print!("{l}");
        }
        if nh.cl.download_prompt & DLP_SHOW != 0 && self.netext.prompt.is_none() {
            self.netext.prompt = Some(nh.cl.download_prompt_text.clone());
        }
    }

    /// The ui's `UI_DrawConnectScreen` at CA_CONNECTED: a prompt to show
    /// opens download_popmenu (`com_downloadPrompt` goes to 0 meanwhile).
    pub(crate) fn take_download_prompt(&mut self) -> Option<String> {
        let t = self.netext.prompt.take()?;
        if let Some(nh) = self.nethost.as_mut() {
            if nh.cl.download_prompt & DLP_SHOW == 0 {
                return None; // already answered or gone stale
            }
            nh.cl.download_prompt = 0;
        }
        self.client.cvars.set("com_downloadPromptText", &t);
        Some(t)
    }

    /// The ui's `trap_Cvar_Set( "com_downloadPrompt" )` (uiScript
    /// downloadIgnore / downloadCURL / downloadUDP): to the client.
    pub(super) fn netext_ui_cvar(&mut self, name: &str, value: &str) -> bool {
        if !name.eq_ignore_ascii_case("com_downloadPrompt") {
            return false;
        }
        if let Some(nh) = self.nethost.as_mut() {
            nh.cl.download_prompt = trem_sim::kernel::spawn::c_atoi(value);
        }
        true
    }

    /// `trap_GetConfigString( CS_SERVERINFO )` while connecting (the
    /// gamestate is in, no map yet).
    pub(crate) fn connecting_serverinfo(&self) -> Vec<(String, String)> {
        let Some(nh) = self.nethost.as_ref().filter(|n| n.cl.state >= ConnState::Connected) else { return Vec::new() };
        let si = &nh.cl.engine_cs(trem_sim::kernel::svgame::cs::SERVERINFO);
        trem_sim::kernel::svgame::info_pairs(si)
    }

    /// `UI_DisplayDownloadInfo`'s lines for the connect screen: the file
    /// and its percentage, bytes copied, transfer rate and estimated time
    /// left, plus how many files are still queued; and the fraction done
    /// (drawn as the loading screen's progress bar).
    pub(super) fn download_info(&self) -> Option<(String, f32)> {
        let nh = self.nethost.as_ref()?;
        let d = nh.cl.download.as_ref().filter(|d| !d.remote_name.is_empty())?;
        let readable = |n: f64| -> String {
            // UI_ReadableSize
            if n > 1024.0 * 1024.0 * 1024.0 {
                format!("{:.2} GB", n / (1024.0 * 1024.0 * 1024.0))
            } else if n > 1024.0 * 1024.0 {
                format!("{:.2} MB", n / (1024.0 * 1024.0))
            } else if n > 1024.0 {
                format!("{} KB", (n / 1024.0) as i64)
            } else {
                format!("{} bytes", n as i64)
            }
        };
        let frac = if d.size > 0 { (d.count as f32 / d.size as f32).clamp(0.0, 1.0) } else { 0.0 };
        let mut s = if d.size > 0 {
            format!("Downloading: {} ({}%)\n{} of {} copied", d.remote_name, (frac * 100.0) as i32, readable(d.count as f64), readable(d.size as f64))
        } else {
            format!("Downloading: {}\n{} copied", d.remote_name, readable(d.count as f64))
        };
        // UI_PrintTime: transfer rate and estimated time left
        let elapsed = (nh.cl.realtime - d.time).max(1) as f64 / 1000.0;
        if d.count > 0 && elapsed > 0.5 {
            let rate = d.count as f64 / elapsed;
            s.push_str(&format!("\nTransfer rate: {}/sec", readable(rate)));
            if d.size > d.count && rate > 0.0 {
                let left = ((d.size - d.count) as f64 / rate) as i64;
                s.push_str(&format!("   Estimated time left: {}:{:02}", left / 60, left % 60));
            }
        }
        // the download list is "@remote@local" pairs still to fetch
        let queued = nh.cl.download_list.matches('@').count() / 2;
        if queued > 0 {
            s.push_str(&format!("\n{queued} more file{} queued", if queued == 1 { "" } else { "s" }));
        }
        Some((s, frac))
    }

    // ================================================================ dedicated

    /// `com_dedicated` from the command line (`+set dedicated 1|2`).
    pub(super) fn dedicated_wanted(&self) -> bool {
        self.net_cvar("dedicated").is_some_and(|v| trem_sim::kernel::spawn::c_atoi(&v) != 0)
    }

    /// Com_Init of a dedicated server: no client and no presentation; the
    /// console reads stdin (`Sys_ConsoleInput`).
    pub(super) fn dedicated_start(&mut self) {
        self.netext.dedicated = true;
        self.input_enabled = false;
        self.bots_per_team = self.net_cvar("bots_per_team").map(|v| trem_sim::kernel::spawn::c_atoi(&v)).unwrap_or(0);
        self.ensure_nethost();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for l in std::io::stdin().lock().lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        self.netext.stdin = Some(rx);
        godot_print!("Tremulous dedicated server (Godot + Rust port): the console reads stdin");
    }

    /// `SV_SpawnServer` of a dedicated server: the map's World only (no
    /// loading screen, no media). "" or the error.
    pub(super) fn dedicated_load(&mut self, paths: &[String], name: &str, then: Vec<String>, cheats: bool) -> String {
        let mut set = crate::assets::Pk3Set::open(paths);
        let Some(data) = set.read(&format!("maps/{name}.bsp")) else { return format!("Can't find map maps/{name}.bsp") };
        let bsp = match trem_sim::kernel::bsp::parse(data.as_slice()) {
            Ok(b) => b,
            Err(e) => return format!("{name}: {e}"),
        };
        let layouts = self.take_host_cvar("g_layouts").unwrap_or_default();
        let auto = self.cvars.iter().find(|(k_, _)| k_.eq_ignore_ascii_case("g_layoutAuto")).is_none_or(|(_, v)| v.trim() != "0");
        let fs: Box<dyn trem_sim::kernel::world::Fs> = Box::new(crate::hostfs::HostFs::new(&self.pk3_paths));
        let mut w = World::from_bsp_with(&bsp, name, Some(fs), Some((&layouts, auto)));
        if let Some(old) = self.nethost.as_mut().and_then(|n| n.server.as_mut()) {
            trem_sim::kernel::log::shutdown_game(&mut old.world);
        }
        self.prepare_game_world(&mut w, None, false);
        w.level.cheats = cheats;
        let mut sc = self.server_cvars();
        if sc.dedicated == 0 {
            sc.dedicated = 1;
        }
        sc.cheats = cheats;
        let ip = self.net_cvar("net_ip").unwrap_or_default();
        let port = self.net_cvar("net_port").map(|p| trem_sim::kernel::spawn::c_atoi(&p) as u16).filter(|p| *p != 0).unwrap_or(trem_sim::kernel::netchan::PORT_SERVER);
        self.sync_net_cvars(true);
        let nh = self.nethost.as_mut().unwrap();
        nh.loopback_only = false;
        nh.spawn_server(w, name, sc, &ip, port, false);
        for l in std::mem::take(&mut nh.out).lines() {
            godot_print!("{l}");
        }
        self.current = CurrentMap::Bsp(paths.to_vec(), name.to_string());
        for l in &then {
            self.server_command(l);
        }
        String::new()
    }

    /// One frame of the dedicated server: the stdin console, then the
    /// server (`Com_Frame` with no client).
    pub(super) fn dedicated_process(&mut self, delta: f64) {
        let lines: Vec<String> = self.netext.stdin.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for l in lines {
            let l = l.trim().to_string();
            if l.is_empty() {
                continue;
            }
            if l.eq_ignore_ascii_case("quit") {
                if let Some(nh) = self.nethost.as_mut() {
                    nh.shutdown_server("Server quit");
                    for o in std::mem::take(&mut nh.out).lines() {
                        godot_print!("{o}");
                    }
                }
                self.base().get_tree().quit();
                return;
            }
            let out = self.console_line(&l);
            let more = self.run_exec_output();
            for o in out.lines().chain(more.lines()) {
                godot_print!("{o}");
            }
        }
        if self.net_active() {
            self.net_process(delta);
        }
    }
}
