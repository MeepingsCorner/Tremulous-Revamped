//! `hdpack`: shrinks a 4K texture pack losslessly (dec055).
//!
//! ```text
//! hdpack stat  <dir>                         sizes by kind and version
//! hdpack bench <dir> [files] [levels...]     zstd ratio and speed on a sample
//! hdpack pack  <dir> [--level N] [--threads N] [--dry-run]
//! ```
//!
//! `pack` replaces byte-identical duplicates (same header fields and mip
//! chain) with links to the first of them, and rewrites every TREMHD01
//! file as TREMHD02. Each new file is read back and compared with the
//! original texture before it replaces it (written beside it, then
//! renamed), so an interrupted run leaves a valid pack.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use trem_hdpack::*;

fn files(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else if p.extension().is_some_and(|x| x == "hdtex" || x == "hdnrm") {
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

fn kind(b: &[u8]) -> &'static str {
    match b.get(..8) {
        Some(m) if m == MAGIC_V1 => "v1",
        Some(m) if m == MAGIC_V2 => "v2",
        Some(m) if m == MAGIC_LINK => "link",
        _ => "other",
    }
}

fn head(p: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut b = vec![0u8; 8];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut b)).unwrap_or(0);
    b.truncate(n);
    b
}

fn gb(n: u64) -> String {
    format!("{:.3} GB", n as f64 / 1e9)
}

fn stat(dir: &Path) {
    let mut by: HashMap<(String, &'static str), (usize, u64)> = HashMap::new();
    let mut total = 0u64;
    for f in files(dir) {
        let p = dir.join(&f);
        let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let ext = f.rsplit('.').next().unwrap_or("").to_string();
        let e = by.entry((ext, kind(&head(&p)))).or_default();
        e.0 += 1;
        e.1 += len;
        total += len;
    }
    let mut keys: Vec<_> = by.keys().cloned().collect();
    keys.sort();
    for k in keys {
        let (n, b) = by[&k];
        println!("{:6} {:5} {:5} files {}", k.0, k.1, n, gb(b));
    }
    println!("total {}", gb(total));
}

fn bench(dir: &Path, n: usize, levels: &[i32]) {
    let all: Vec<String> = files(dir).into_iter().filter(|f| kind(&head(&dir.join(f))) == "v1").collect();
    let step = (all.len() / n.max(1)).max(1);
    let sample: Vec<Vec<u8>> = all.iter().step_by(step).take(n).filter_map(|f| std::fs::read(dir.join(f)).ok()).collect();
    let raw: usize = sample.iter().map(|b| b.len()).sum();
    println!("{} files, {}", sample.len(), gb(raw as u64));
    for &l in levels {
        let t = std::time::Instant::now();
        let enc: Vec<Vec<u8>> = sample.iter().map(|b| encode_v2(b, l).unwrap()).collect();
        let ct = t.elapsed().as_secs_f64();
        let out: usize = enc.iter().map(|b| b.len()).sum();
        // decode speed through the game's reader
        let d = std::env::temp_dir().join(format!("hdpack_bench_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut dt = 0.0;
        for (i, e) in enc.iter().enumerate() {
            let name = format!("{i}.hdtex");
            std::fs::write(d.join(&name), e).unwrap();
            let t = std::time::Instant::now();
            let r = read(&d, &name, usize::MAX).unwrap();
            dt += t.elapsed().as_secs_f64();
            assert_eq!(r.data, sample[i][HEADER..], "level {l}: not lossless");
        }
        let _ = std::fs::remove_dir_all(&d);
        println!("level {l:2}: {:.1}% of the size, compress {:.0} MB/s, read {:.0} MB/s", out as f64 * 100.0 / raw as f64, raw as f64 / 1e6 / ct, raw as f64 / 1e6 / dt.max(1e-9));
    }
}

/// The texture's identity for duplicates: header fields and full mip chain.
fn content_key(t: &Texture) -> (u64, usize) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (t.header.w, t.header.h, t.header.fmt, t.header.mips, t.header.alpha).hash(&mut h);
    t.header.mean.map(f32::to_bits).hash(&mut h);
    t.data.hash(&mut h);
    (h.finish(), t.data.len())
}

fn same(a: &Texture, b: &Texture) -> bool {
    a.header.w == b.header.w && a.header.h == b.header.h && a.header.fmt == b.header.fmt && a.header.mips == b.header.mips && a.header.alpha == b.header.alpha && a.header.mean.map(f32::to_bits) == b.header.mean.map(f32::to_bits) && a.data == b.data
}

/// Writes `bytes` beside `p` and renames it over `p`.
fn replace(p: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = p.with_extension(format!("{}.tmp", p.extension().unwrap().to_string_lossy()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, p)
}

fn pack(dir: &Path, level: i32, threads: usize, dry: bool) -> Result<(), String> {
    let list = files(dir);
    let before: u64 = list.iter().map(|f| std::fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0)).sum();
    println!("{} files, {}", list.len(), gb(before));
    // 1. duplicates: by content, the first path keeps the data
    let keys: Mutex<Vec<(String, (u64, usize))>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(f) = list.get(i) else { break };
                if kind(&head(&dir.join(f))) == "link" {
                    continue;
                }
                if let Some(t) = read(dir, f, usize::MAX) {
                    keys.lock().unwrap().push((f.clone(), content_key(&t)));
                }
            });
        }
    });
    let mut keys = keys.into_inner().unwrap();
    keys.sort();
    let mut first: HashMap<(u64, usize), String> = HashMap::new();
    let mut links = Vec::new();
    for (f, k) in &keys {
        match first.get(k) {
            Some(canon) => links.push((f.clone(), canon.clone())),
            None => {
                first.insert(*k, f.clone());
            }
        }
    }
    let mut linked = 0u64;
    for (f, canon) in &links {
        let (a, b) = (read(dir, f, usize::MAX), read(dir, canon, usize::MAX));
        if !matches!((&a, &b), (Some(a), Some(b)) if same(a, b)) {
            println!("hash match but different bytes, kept: {f}");
            continue;
        }
        linked += std::fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0);
        if !dry {
            replace(&dir.join(f), &link_bytes(canon)).map_err(|e| format!("{f}: {e}"))?;
        }
    }
    println!("duplicates: {} files ({}) become links", links.len(), gb(linked));
    // 2. every remaining v1 file to v2, verified
    let todo: Vec<String> = first.values().filter(|f| kind(&head(&dir.join(f))) == "v1").cloned().collect();
    let done = AtomicUsize::new(0);
    let saved = Mutex::new((0u64, 0u64));
    let failed: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let t0 = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(f) = todo.get(i) else { break };
                let p = dir.join(f);
                let r = (|| -> Result<(u64, u64), String> {
                    let raw = std::fs::read(&p).map_err(|e| e.to_string())?;
                    let enc = encode_v2(&raw, level).map_err(|e| e.to_string())?;
                    // read back through the game's reader before replacing
                    let check = std::env::temp_dir().join(format!("hdpack_check_{}_{i}", std::process::id()));
                    std::fs::create_dir_all(&check).map_err(|e| e.to_string())?;
                    std::fs::write(check.join("t.hdtex"), &enc).map_err(|e| e.to_string())?;
                    let back = read(&check, "t.hdtex", usize::MAX);
                    let _ = std::fs::remove_dir_all(&check);
                    if back.map(|t| t.data) != Some(raw[HEADER..].to_vec()) {
                        return Err("read-back differs".into());
                    }
                    if !dry {
                        replace(&p, &enc).map_err(|e| e.to_string())?;
                    }
                    Ok((raw.len() as u64, enc.len() as u64))
                })();
                match r {
                    Ok((a, b)) => {
                        let mut s = saved.lock().unwrap();
                        s.0 += a;
                        s.1 += b;
                    }
                    Err(e) => failed.lock().unwrap().push(format!("{f}: {e}")),
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 50 == 0 || n == todo.len() {
                    println!("  {n}/{} compressed ({:.0} s)", todo.len(), t0.elapsed().as_secs_f64());
                }
            });
        }
    });
    let (a, b) = saved.into_inner().unwrap();
    println!("compressed: {} -> {} ({:.1}%)", gb(a), gb(b), b as f64 * 100.0 / a.max(1) as f64);
    let failed = failed.into_inner().unwrap();
    for f in &failed {
        println!("left as it was: {f}");
    }
    let after: u64 = list.iter().map(|f| std::fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0)).sum();
    println!("{}: {} -> {}", if dry { "dry run (nothing written)" } else { "pack" }, gb(before), gb(if dry { before - linked - a + b } else { after }));
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("{} files could not be converted", failed.len()))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: hdpack stat <dir> | bench <dir> [files] [levels...] | pack <dir> [--level N] [--threads N] [--dry-run]";
    let (Some(cmd), Some(dir)) = (args.first(), args.get(1)) else {
        eprintln!("{usage}");
        std::process::exit(2);
    };
    let dir = PathBuf::from(dir);
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<i64>().ok());
    match cmd.as_str() {
        "stat" => stat(&dir),
        "bench" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(12);
            let levels: Vec<i32> = args.iter().skip(3).filter_map(|v| v.parse().ok()).collect();
            bench(&dir, n, if levels.is_empty() { &[3, 9, 15, 19] } else { &levels });
        }
        "pack" => {
            let threads = opt("--threads").map(|t| t as usize).unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4));
            let level = opt("--level").unwrap_or(15) as i32;
            if let Err(e) = pack(&dir, level, threads.max(1), args.iter().any(|a| a == "--dry-run")) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
}
