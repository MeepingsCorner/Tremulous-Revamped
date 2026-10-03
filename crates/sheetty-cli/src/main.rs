//! `sheetty` command line (MDD Appendix F).
//!
//!   sheetty preflight [--strict] [--json]
//!   sheetty report unimplemented [--limit N]
//!   sheetty overlap <sheetA> <sheetB> [--key col]
//!   sheetty view <sheet> --cols a,b,c [--where col=value] [--out file]
//!   sheetty patch <file.patch> [--dry-run]
//!   sheetty parity <ref.tsv> <port.tsv> [--key tick,entity,key]
//!   sheetty verify-determinism <out_dir>
//!   sheetty check-generated          (D6: no generated banner outside target/)
//!   sheetty json [group|sheet]       (To-Do 4: the book as grouped JSON)
//!   sheetty matrix [--all]           (To-Do 5: checkmark matrix per sheet)

use sheetty::{check, diag, emit, tsv, Config};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::exit;

fn root() -> PathBuf {
    let mut p = std::env::current_dir().unwrap();
    loop {
        if p.join("sheets/00-doctrine.tsv").exists() {
            return p;
        }
        if !p.pop() {
            eprintln!("sheetty: not inside a sheet book (no sheets/00-doctrine.tsv)");
            exit(2);
        }
    }
}

fn cfg(strict: bool) -> Config {
    let r = root();
    Config {
        sheets: r.join("sheets"),
        root: r.clone(),
        out: None,
        strict,
        fn_sig_path: "crate::kernel::sig".into(),
        kernel_dir: Some(r.join("crates/trem_sim/src/kernel")),
        emit_cargo_hints: false,
    }
}

fn flag(args: &[String], f: &str) -> bool {
    args.iter().any(|a| a == f)
}
fn opt<'a>(args: &'a [String], f: &str) -> Option<&'a str> {
    args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("preflight");
    match cmd {
        "preflight" => preflight(&args),
        "report" => report(&args),
        "overlap" => overlap(&args),
        "view" => view(&args),
        "patch" => patch(&args),
        "parity" => parity(&args),
        "verify-determinism" => verify(&args),
        "check-generated" => check_generated(),
        "json" => json(&args),
        "matrix" => matrix(&args),
        _ => {
            eprintln!("usage: sheetty preflight|report|overlap|view|patch|parity|verify-determinism|check-generated|json|matrix");
            exit(2)
        }
    }
}

fn preflight(args: &[String]) {
    let c = cfg(flag(args, "--strict"));
    let (d, book, _, _, _) = sheetty::preflight(&c);
    let rows: usize = book.iter().map(|s| s.rows.len()).sum();
    let cols: usize = book.iter().map(|s| s.header.len()).sum();
    if flag(args, "--json") {
        println!("{}", d.json());
    } else {
        println!("{}", d.human(&format!("PREFLIGHT  sheets/  {} sheets, {} rows, {} columns", book.len(), rows, cols)));
    }
    exit(if d.errors() > 0 { 1 } else { 0 });
}

fn report(args: &[String]) {
    let c = cfg(false);
    let (_, _, _, typed, cov) = sheetty::preflight(&c);
    let limit: usize = opt(args, "--limit").and_then(|s| s.parse().ok()).unwrap_or(50);
    match args.get(1).map(|s| s.as_str()) {
        Some("symbols") => {
            let sym = &typed["symbols"];
            let want = opt(args, "--status").unwrap_or("Todo");
            let sub = opt(args, "--subsystem");
            println!("id\tref_src\trust_module");
            for r in sym.rows.iter().filter(|r| sym.get(r, "status").and_then(|v| v.as_str()) == Some(want)) {
                if let Some(s) = sub {
                    if sym.get(r, "subsystem").and_then(|v| v.as_str()) != Some(s) {
                        continue;
                    }
                }
                println!("{}\t{}\t{}", r.key, sym.get(r, "ref_src").and_then(|v| v.as_str()).unwrap_or("-"), sym.get(r, "rust_module").and_then(|v| v.as_str()).unwrap_or("-"));
            }
        }
        _ => {
            println!("# unimplemented, ranked by blast radius (inbound refs x10 + priority)");
            println!("sheet:id\trank\tkind");
            for (k, r, kind) in cov.unimplemented.iter().take(limit) {
                println!("{k}\t{r}\t{kind}");
            }
            println!("# {} unimplemented, {} covered, {} orphan", cov.unimplemented.len(), cov.covered.len(), cov.orphan.len());
            for (sub, st) in &cov.symbols {
                println!("# symbols[{sub}] {st:?}");
            }
        }
    }
}

fn overlap(args: &[String]) {
    let (Some(a), Some(b)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: sheetty overlap <sheetA> <sheetB>");
        exit(2)
    };
    let c = cfg(false);
    let mut d = diag::Diagnostics::default();
    let book = tsv::load_book(&c.sheets, &mut d);
    let get = |n: &str| book.iter().find(|s| s.name == n).unwrap_or_else(|| {
        eprintln!("no sheet {n}");
        exit(2)
    });
    let (sa, sb) = (get(a), get(b));
    let ka: BTreeMap<String, &tsv::Row> = sa.rows.iter().map(|r| (sa.key_of(r), r)).collect();
    let kb: BTreeMap<String, &tsv::Row> = sb.rows.iter().map(|r| (sb.key_of(r), r)).collect();
    let shared: Vec<&String> = sa.header.iter().filter(|h| sb.header.contains(h) && *h != "id").collect();
    println!("key\tclass\tdetail");
    for k in ka.keys().chain(kb.keys().filter(|k| !ka.contains_key(*k))) {
        let class = match (ka.get(k), kb.get(k)) {
            (Some(ra), Some(rb)) => {
                let diff: Vec<String> = shared
                    .iter()
                    .filter_map(|h| {
                        let va = sa.cell(ra, h)?;
                        let vb = sb.cell(rb, h)?;
                        (va != vb).then(|| format!("{h}: {va} != {vb}"))
                    })
                    .collect();
                if diff.is_empty() { ("covered".to_string(), String::new()) } else { ("divergent".to_string(), diff.join("; ")) }
            }
            (Some(_), None) => ("unimplemented".into(), String::new()),
            (None, Some(_)) => ("orphan".into(), String::new()),
            _ => unreachable!(),
        };
        println!("{k}\t{}\t{}", class.0, class.1);
    }
}

fn view(args: &[String]) {
    let Some(name) = args.get(1) else {
        eprintln!("usage: sheetty view <sheet> --cols a,b [--where col=value]");
        exit(2)
    };
    let c = cfg(false);
    let mut d = diag::Diagnostics::default();
    let book = tsv::load_book(&c.sheets, &mut d);
    let Some(sh) = book.iter().find(|s| &s.name == name) else {
        eprintln!("no sheet {name}");
        exit(2)
    };
    let cols: Vec<String> = opt(args, "--cols").map(|s| s.split(',').map(|x| x.to_string()).collect()).unwrap_or_else(|| sh.header.clone());
    let filt = opt(args, "--where").and_then(|w| w.split_once('='));
    let mut out = String::new();
    out.push_str(&cols.join("\t"));
    out.push('\n');
    for r in &sh.rows {
        if let Some((k, v)) = filt {
            if sh.cell(r, k) != Some(v) {
                continue;
            }
        }
        let cells: Vec<&str> = cols.iter().map(|c| sh.cell(r, c).unwrap_or("")).collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    let budget: Option<usize> = opt(args, "--max-tokens").and_then(|s| s.parse().ok());
    if let Some(b) = budget {
        if out.len() / 4 > b {
            eprintln!("E-L6-BUDGET view {name}: ~{} tokens > {b}", out.len() / 4);
            exit(1);
        }
    }
    match opt(args, "--out") {
        Some(p) => std::fs::write(p, out).unwrap(),
        None => print!("{out}"),
    }
}

/// Row-delta patch language (MDD §11.4):
///   = <sheet> <key> <col> <value>     set cell
///   + <sheet> <cell>\t<cell>...       add row (tab separated after the sheet name)
///   - <sheet> <key>                   delete row
fn patch(args: &[String]) {
    let Some(file) = args.get(1) else {
        eprintln!("usage: sheetty patch <file> [--dry-run]");
        exit(2)
    };
    let dry = flag(args, "--dry-run");
    let c = cfg(false);
    let mut d = diag::Diagnostics::default();
    let book = tsv::load_book(&c.sheets, &mut d);
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("{file}: {e}");
        exit(2)
    });
    let mut edits: BTreeMap<String, Vec<String>> = BTreeMap::new(); // path -> lines
    let mut applied = 0;
    for (ln, line) in text.lines().enumerate() {
        let line = line.split(" #").next().unwrap_or(line).trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (op, rest) = line.split_at(1);
        let rest = rest.trim_start();
        let (sheet, rest) = rest.split_once(' ').unwrap_or((rest, ""));
        let Some(sh) = book.iter().find(|s| s.name == sheet) else {
            eprintln!("patch:{}: no sheet {sheet}", ln + 1);
            exit(1)
        };
        let path = sh.path.to_string_lossy().to_string();
        let lines = edits.entry(path.clone()).or_insert_with(|| std::fs::read_to_string(&sh.path).unwrap().split('\n').map(|s| s.to_string()).collect());
        let find = |lines: &Vec<String>, key: &str| -> Option<usize> {
            lines.iter().position(|l| {
                if l.starts_with('#') || l.is_empty() {
                    return false;
                }
                let cells: Vec<&str> = l.split('\t').collect();
                let k: Vec<&str> = sh.key_cols().iter().map(|&i| cells.get(i).copied().unwrap_or("")).collect();
                k.join(":") == key
            })
        };
        match op {
            "=" => {
                let mut it = rest.splitn(3, ' ');
                let (key, col, val) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
                let Some(ci) = sh.col(col) else {
                    eprintln!("patch:{}: no column {col} in {sheet}", ln + 1);
                    exit(1)
                };
                let Some(li) = find(lines, key) else {
                    eprintln!("patch:{}: no row {key} in {sheet}", ln + 1);
                    exit(1)
                };
                let mut cells: Vec<String> = lines[li].split('\t').map(|s| s.to_string()).collect();
                cells[ci] = val.to_string();
                lines[li] = cells.join("\t");
            }
            "+" => {
                let row = rest.to_string();
                let insert_at = lines.iter().rposition(|l| !l.is_empty()).map(|i| i + 1).unwrap_or(lines.len());
                lines.insert(insert_at, row);
            }
            "-" => {
                let Some(li) = find(lines, rest.trim()) else {
                    eprintln!("patch:{}: no row {rest} in {sheet}", ln + 1);
                    exit(1)
                };
                lines.remove(li);
            }
            _ => {
                eprintln!("patch:{}: unknown op {op}", ln + 1);
                exit(1)
            }
        }
        applied += 1;
    }
    if dry {
        println!("{applied} edit(s) parsed and applicable (dry run)");
        return;
    }
    let backup: Vec<(String, String)> = edits.keys().map(|p| (p.clone(), std::fs::read_to_string(p).unwrap())).collect();
    for (p, lines) in &edits {
        std::fs::write(p, lines.join("\n")).unwrap();
    }
    // validated by preflight; roll back atomically on failure
    let (d, ..) = sheetty::preflight(&c);
    if d.errors() > 0 {
        for (p, b) in backup {
            std::fs::write(p, b).unwrap();
        }
        println!("{}", d.human("PATCH REJECTED, rolled back"));
        exit(1);
    }
    println!("{applied} edit(s) applied, preflight clean");
}

/// Golden-trace parity (MDD §7.5): full outer join on the key columns.
fn parity(args: &[String]) {
    let (Some(a), Some(b)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: sheetty parity <ref.tsv> <port.tsv>");
        exit(2)
    };
    let keys: Vec<String> = opt(args, "--key").unwrap_or("tick,entity,key").split(',').map(|s| s.to_string()).collect();
    let tol: f64 = opt(args, "--tolerance").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let load = |p: &str| -> (Vec<String>, BTreeMap<Vec<String>, String>) {
        let t = std::fs::read_to_string(p).unwrap_or_else(|e| {
            eprintln!("{p}: {e}");
            exit(2)
        });
        let mut lines = t.lines().filter(|l| !l.starts_with('#'));
        let hdr: Vec<String> = lines.next().unwrap_or("").split('\t').map(|s| s.to_string()).collect();
        let vi = hdr.iter().position(|h| h == "value").expect("trace needs a value column");
        let ki: Vec<usize> = keys.iter().map(|k| hdr.iter().position(|h| h == k).unwrap_or_else(|| panic!("trace lacks key column {k}"))).collect();
        let mut m = BTreeMap::new();
        for l in lines {
            let c: Vec<&str> = l.split('\t').collect();
            m.insert(ki.iter().map(|&i| c[i].to_string()).collect::<Vec<_>>(), c[vi].to_string());
        }
        (hdr, m)
    };
    let (_, ra) = load(a);
    let (_, rb) = load(b);
    let num_eq = |x: &str, y: &str| -> bool {
        if x == y {
            return true;
        }
        let xs: Vec<f64> = x.split(' ').filter_map(|v| v.parse().ok()).collect();
        let ys: Vec<f64> = y.split(' ').filter_map(|v| v.parse().ok()).collect();
        tol > 0.0 && !xs.is_empty() && xs.len() == ys.len() && xs.iter().zip(&ys).all(|(p, q)| (p - q).abs() <= tol)
    };
    let tick_of = |k: &Vec<String>| k.first().and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
    let (mut matching, mut divergent, mut missing, mut extra) = (0usize, Vec::new(), 0usize, 0usize);
    let mut per_key: BTreeMap<String, usize> = BTreeMap::new();
    for (k, v) in &ra {
        match rb.get(k) {
            Some(w) if num_eq(v, w) => matching += 1,
            Some(w) => {
                *per_key.entry(k.last().cloned().unwrap_or_default()).or_insert(0) += 1;
                divergent.push((k.clone(), v.clone(), w.clone()));
            }
            None => missing += 1,
        }
    }
    for k in rb.keys() {
        if !ra.contains_key(k) {
            extra += 1;
        }
    }
    divergent.sort_by_key(|(k, _, _)| tick_of(k));
    println!("PARITY {a} vs {b}  key={}", keys.join(","));
    println!("  matching   {matching}");
    println!("  divergent  {}  {:?}", divergent.len(), per_key);
    println!("  missing    {missing}");
    println!("  extra      {extra}");
    if let Some((k, v, w)) = divergent.first() {
        println!("  FIRST DIVERGENCE  {}  reference={v}  port={w}", k.join(" "));
    }
    let ok = divergent.is_empty() && missing == 0 && extra == 0;
    println!("{}", if ok { "PARITY OK" } else { "PARITY FAILED" });
    exit(if ok { 0 } else { 1 });
}

fn verify(args: &[String]) {
    let c = cfg(false);
    let (d, book, schema, typed, _) = sheetty::preflight(&c);
    if d.errors() > 0 {
        println!("{}", d.human("preflight failed"));
        exit(1);
    }
    let cx = emit::Ctx { typed: &typed, book: &book, schema: &schema, fn_sig_path: c.fn_sig_path.clone() };
    let files = emit::render(&cx);
    let Some(out) = args.get(1) else {
        println!("digest {:016x}", emit::digest(&files));
        return;
    };
    let mut bad = 0;
    for (rel, body) in &files {
        let p = Path::new(out).join(rel);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(body.as_str()) {
            println!("DIFFERS {}", p.display());
            bad += 1;
        }
    }
    println!("{} file(s) compared, {bad} differ; digest {:016x}", files.len(), emit::digest(&files));
    let _ = check::META_SHEETS;
    exit(if bad > 0 { 1 } else { 0 });
}

fn check_generated() {
    let r = root();
    let mut hits = 0;
    let mut stack = vec![r.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                if name != "target" && name != ".git" && name != "re" && name != ".godot" {
                    stack.push(p);
                }
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    if t.starts_with("// GENERATED BY sheetty") {
                        println!("E-L0-GENERATED {} (D6: generated code must not be in the tree)", p.display());
                        hits += 1;
                    }
                }
            }
        }
    }
    println!("{hits} generated file(s) outside target/");
    exit(if hits > 0 { 1 } else { 0 });
}

/// To-Do 4: print the sheet book (or one group / sheet) as grouped JSON.
fn json(args: &[String]) {
    let c = cfg(false);
    let (d, _, _, typed, _) = sheetty::preflight(&c);
    if d.errors() > 0 {
        eprintln!("{}", d.human("PREFLIGHT"));
        exit(1);
    }
    print!("{}", sheetty::book::json(&typed, args.get(1).map(|s| s.as_str())));
}

/// To-Do 5: the row x column checkmark matrix of every sheet.
fn matrix(args: &[String]) {
    let c = cfg(false);
    let (_, _, _, typed, _) = sheetty::preflight(&c);
    let mut d = diag::Diagnostics::default();
    let m = sheetty::book::completeness(&typed, &mut d);
    let groups = sheetty::book::groups(&typed);
    println!("{:<22} {:<13} {:>6} {:>5} {:>9} {:>9}  status", "sheet", "group", "rows", "cols", "cells", "checked");
    let (mut cells, mut checked) = (0, 0);
    for (name, s) in &m {
        cells += s.cells();
        checked += s.checked;
        let st = if s.unfinished.is_empty() { "ok" } else { "UNFINISHED" };
        println!("{:<22} {:<13} {:>6} {:>5} {:>9} {:>9}  {st}", name, groups.get(name).map(|s| s.as_str()).unwrap_or("-"), s.rows, s.cols, s.cells(), s.checked);
        let lim = if flag(args, "--all") { usize::MAX } else { 10 };
        for (row, col, why) in s.unfinished.iter().take(lim) {
            println!("    {row} x {col}: {why}");
        }
    }
    println!("TOTAL {checked}/{cells} intersections checked");
    exit(if checked == cells { 0 } else { 1 });
}
