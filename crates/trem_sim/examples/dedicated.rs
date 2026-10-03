//! A dedicated server (`tremded`) on the port's kernel (dec033):
//! `cargo run --release -p trem_sim --example dedicated -- +set sv_hostname "My server" +map atcs`.
//!
//! Command line: `+set <cvar> <value>`, `+map <name>` / `+devmap <name>`
//! (`arena` is the built-in sheet map; other maps are read from the
//! map-*.pk3 files in `--base=DIR`, default godot/base), any other `+cmd`
//! runs on the console once the map is up. Then the console reads stdin:
//! engine commands (heartbeat, serverinfo, systeminfo, killserver, map,
//! devmap, map_restart, quit), the game's server commands (status,
//! listplayers, say, kick, ban, ...), else `<cvar> [value]`.
//! Cvars: net_enabled (1: ipv4; 3 adds ipv6 and the LAN multicast group),
//! net_ip, net_port (30720), net_ip6, net_port6, net_mcast6addr, sv_pure
//! (1: clients must have the server's paks), sv_allowDownload (1: clients
//! may download the referenced pk3s), dedicated (1 LAN, 2 Internet with
//! heartbeats to sv_master1..5), sv_hostname, sv_maxclients, rconPassword,
//! g_* game cvars, bots_per_team (0), fs_homepath (admin.dat, layouts,
//! games.log; default ./dedicated-home).

use std::collections::BTreeMap;
use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use trem_sim::gen::enums::Team;
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::files::PakSet;
use trem_sim::kernel::nethost::{self, HostEvent, NetHost};
use trem_sim::kernel::world::{Fs, World};
use trem_sim::kernel::{admin, bot, bsp, client, log, session, svcmds};

/// fs_homepath for writes, then the pk3s for reads (`trap_FS_*`).
struct DirFs {
    home: PathBuf,
    pk3s: Vec<PathBuf>,
}

impl DirFs {
    fn path(&self, p: &str) -> Option<PathBuf> {
        (!p.starts_with(['/', '\\']) && !p.split(['/', '\\']).any(|c| c == "..") && !p.contains(':')).then(|| self.home.join(p))
    }
    fn pk3_read(&self, path: &str) -> Option<Vec<u8>> {
        for p in self.pk3s.iter().rev() {
            let Ok(f) = std::fs::File::open(p) else { continue };
            let Ok(mut z) = zip::ZipArchive::new(f) else { continue };
            let mut v = Vec::new();
            let ok = match z.by_name(path) {
                Ok(mut e) => e.read_to_end(&mut v).is_ok(),
                Err(_) => false,
            };
            if ok {
                return Some(v);
            }
        }
        None
    }
}

impl Fs for DirFs {
    fn read(&self, path: &str) -> Option<String> {
        if let Some(t) = self.path(path).and_then(|p| std::fs::read(p).ok()) {
            return Some(String::from_utf8_lossy(&t).into_owned());
        }
        self.pk3_read(path).map(|d| String::from_utf8_lossy(&d).into_owned())
    }
    fn write(&mut self, path: &str, text: &str) -> bool {
        let Some(p) = self.path(path) else { return false };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(p, text).is_ok()
    }
    fn list(&self, dir: &str, ext: &str) -> Vec<String> {
        let Some(rd) = self.path(dir).and_then(|p| std::fs::read_dir(p).ok()) else { return Vec::new() };
        rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.to_lowercase().ends_with(&ext.to_lowercase())).collect()
    }
}

struct Ded {
    host: NetHost,
    cvars: BTreeMap<String, String>,
    base: PathBuf,
    map: String,
    cheats: bool,
}

fn real_time() -> i32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i32).unwrap_or(0)
}

impl Ded {
    fn cvar(&self, n: &str) -> Option<String> {
        self.cvars.get(&n.to_ascii_lowercase()).cloned()
    }

    fn pk3s(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.base).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pk3"))).collect()).unwrap_or_default();
        v.sort();
        v
    }

    fn fs(&self) -> DirFs {
        DirFs { home: PathBuf::from(self.cvar("fs_homepath").unwrap_or_else(|| "dedicated-home".into())), pk3s: self.pk3s() }
    }

    /// `SV_SpawnServer` + `G_InitGame`: the World of `name`, with the cvars,
    /// admin.dat, the game log and the bots.
    fn load(&mut self, name: &str, cheats: bool) -> Result<(), String> {
        let fs = self.fs();
        let mut w = if name.eq_ignore_ascii_case("arena") {
            World::new(MapId::Arena)
        } else {
            let data = fs.pk3_read(&format!("maps/{name}.bsp")).ok_or_else(|| format!("Can't find map maps/{name}.bsp"))?;
            let b = bsp::parse(&data).map_err(|e| format!("{name}: {e}"))?;
            let layouts = self.cvars.remove("g_layouts").unwrap_or_default();
            let auto = self.cvar("g_layoutauto").is_none_or(|v| v.trim() != "0");
            World::from_bsp_with(&b, name, Some(Box::new(self.fs())), Some((&layouts, auto)))
        };
        w.fs = Some(Box::new(fs));
        for (k, v) in &self.cvars {
            w.set_cvar(k, v);
        }
        w.level.cheats = cheats;
        w.real_time = real_time();
        log::init_game(&mut w);
        client::load_emoticons(&mut w);
        admin::readconfig(&mut w, None);
        session::init_game_locks(&mut w);
        w.level.map_list = self.pk3s().iter().filter_map(|p| p.file_name()?.to_str()?.strip_prefix("map-")?.split('-').next().map(str::to_string)).collect();
        let n = self.cvar("bots_per_team").map(|v| trem_sim::kernel::spawn::c_atoi(&v)).unwrap_or(0);
        for t in [Team::Aliens, Team::Humans] {
            for k in 0..n {
                let name = format!("{}{}", if t == Team::Aliens { "Xeno" } else { "Marine" }, k + 1);
                bot::add_bot(&mut w, &name, t);
            }
        }
        print!("{}", std::mem::take(&mut w.console));
        let cv = self.cvars.clone();
        let get = |n: &str| cv.get(&n.to_ascii_lowercase()).cloned();
        let mut sc = nethost::server_cvars(&get);
        if self.cvar("dedicated").is_none() {
            sc.dedicated = 1;
        }
        sc.cheats = cheats;
        let ip = self.cvar("net_ip").unwrap_or_default();
        let port = self.cvar("net_port").map(|v| trem_sim::kernel::spawn::c_atoi(&v) as u16).unwrap_or(trem_sim::kernel::netchan::PORT_SERVER);
        // FS_Startup: the base folder, then fs_homepath/base (pure
        // checksums, referenced paks, downloads)
        let home = PathBuf::from(self.cvar("fs_homepath").unwrap_or_else(|| "dedicated-home".into()));
        self.host.server_fs = Some(PakSet::new(&[(self.base.clone(), "base".into()), (home.join("base"), "base".into())]));
        nethost::apply_net_cvars(&mut self.host, &get, true);
        self.host.spawn_server(w, name, sc, &ip, port, false);
        self.map = name.to_string();
        self.cheats = cheats;
        Ok(())
    }

    /// One console line (stdin, rcon, the game's EXEC_APPEND).
    fn console(&mut self, line: &str) -> String {
        let argv = svcmds::tokenize(line);
        let Some(a0) = argv.first().map(|s| s.to_ascii_lowercase()) else { return String::new() };
        if let Some(out) = self.host.command(&argv) {
            return out;
        }
        match a0.as_str() {
            "map" | "devmap" if argv.len() > 1 => {
                let m = argv[1].clone();
                match self.load(&m, a0 == "devmap") {
                    Ok(()) => String::new(),
                    Err(e) => format!("{e}\n"),
                }
            }
            "map_restart" => {
                let (m, c) = (self.map.clone(), self.cheats);
                match self.load(&m, c) {
                    Ok(()) => String::new(),
                    Err(e) => format!("{e}\n"),
                }
            }
            "quit" => {
                self.host.shutdown_server("Server quit");
                print!("{}", std::mem::take(&mut self.host.out));
                std::process::exit(0);
            }
            "set" | "seta" | "sets" | "setu" if argv.len() > 2 => self.set(&argv[1], &argv[2..].join(" ")),
            _ => {
                if let Some(sv) = self.host.server.as_mut() {
                    let w = &mut sv.world;
                    w.real_time = real_time();
                    w.console.clear();
                    if svcmds::console_command(w, line) {
                        let mut out = std::mem::take(&mut w.console);
                        out.push_str(&self.run_exec());
                        return out;
                    }
                }
                match argv.len() {
                    1 => match self.cvar(&argv[0]) {
                        Some(v) => format!("\"{}\" is:\"{v}^7\"\n", argv[0]),
                        None => format!("Unknown command \"{}\"\n", argv[0]),
                    },
                    _ => self.set(&argv[0], &argv[1..].join(" ")),
                }
            }
        }
    }

    fn set(&mut self, name: &str, value: &str) -> String {
        self.cvars.insert(name.to_ascii_lowercase(), value.to_string());
        if let Some(sv) = self.host.server.as_mut() {
            sv.world.set_cvar(name, value);
            let get = |n: &str| self.cvars.get(&n.to_ascii_lowercase()).cloned();
            let mut sc = nethost::server_cvars(&get);
            if !self.cvars.contains_key("dedicated") {
                sc.dedicated = 1;
            }
            sc.cheats = self.cheats;
            sv.cvars = sc;
        }
        String::new()
    }

    /// `trap_SendConsoleCommand( EXEC_APPEND )` lines of the game.
    fn run_exec(&mut self) -> String {
        let mut out = String::new();
        for _ in 0..16 {
            let Some(sv) = self.host.server.as_mut() else { break };
            let lines = svcmds::run_exec(&mut sv.world);
            out.push_str(&std::mem::take(&mut sv.world.console));
            if lines.is_empty() {
                break;
            }
            for l in lines {
                out.push_str(&self.console(&l));
            }
        }
        out
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../godot/base");
    let mut cvars = BTreeMap::new();
    let mut map: Option<(String, bool)> = None;
    let mut later: Vec<String> = Vec::new();
    // Com_ParseCommandLine: "+" starts a command
    let mut cmds: Vec<Vec<String>> = Vec::new();
    for a in &args {
        if let Some(b) = a.strip_prefix("--base=") {
            base = PathBuf::from(b);
        } else if let Some(c) = a.strip_prefix('+') {
            cmds.push(vec![c.to_string()]);
        } else if let Some(c) = cmds.last_mut() {
            c.push(a.clone());
        }
    }
    for c in cmds {
        match c[0].to_ascii_lowercase().as_str() {
            "set" | "seta" if c.len() > 2 => {
                cvars.insert(c[1].to_ascii_lowercase(), c[2..].join(" "));
            }
            "map" | "devmap" if c.len() > 1 => map = Some((c[1].clone(), c[0].eq_ignore_ascii_case("devmap"))),
            _ => later.push(c.iter().map(|x| if x.contains(' ') { format!("\"{x}\"") } else { x.clone() }).collect::<Vec<_>>().join(" ")),
        }
    }
    let mut d = Ded { host: NetHost::new(), cvars, base, map: String::new(), cheats: false };
    let (name, cheats) = map.unwrap_or(("arena".into(), false));
    if let Err(e) = d.load(&name, cheats) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    for l in later {
        print!("{}", d.console(&l));
    }
    // stdin console
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for l in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let mut last = Instant::now();
    loop {
        // NET_Sleep: wait for the next server frame
        std::thread::sleep(Duration::from_millis(1));
        let now = Instant::now();
        let msec = now.duration_since(last).as_millis() as i32;
        if msec < 1 {
            continue;
        }
        last = now;
        while let Ok(l) = rx.try_recv() {
            print!("{}", d.console(&l));
        }
        for ev in d.host.frame(msec.min(200), None, None) {
            if let HostEvent::Rcon { from, line } = ev {
                let out = d.console(&line);
                d.host.rcon_reply(&from, &out);
            }
        }
        print!("{}", d.run_exec());
        print!("{}", std::mem::take(&mut d.host.out));
        // ExitLevel: g_nextMap, else the same map again
        let exited = d.host.server.as_ref().map(|s| (s.world.level.exited, s.world.level.next_map.clone()));
        if let Some((true, next)) = exited {
            let m = next.unwrap_or_else(|| d.map.clone());
            let c = d.cheats;
            if let Err(e) = d.load(&m, c) {
                eprintln!("{e}");
                let (m, c) = (d.map.clone(), d.cheats);
                let _ = d.load(&m, c);
            }
        }
    }
}
