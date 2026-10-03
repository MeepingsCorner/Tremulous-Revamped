//! sheetty: the Spreadsheet Method machinery (MDD Appendix D).
//!
//! `run` is the whole pipeline: ingest -> validate (L0..L8) -> emit. Emission is
//! impossible when validation has errors (D5).

pub mod book;
pub mod check;
pub mod diag;
pub mod emit;
pub mod overlap;
pub mod schema;
pub mod tsv;

use diag::{Diagnostics, Layer, Sev};
use std::path::PathBuf;

pub struct Config {
    /// `sheets/` directory.
    pub sheets: PathBuf,
    /// Repository root (assets, evidence, kernel walk).
    pub root: PathBuf,
    pub out: Option<PathBuf>,
    pub strict: bool,
    /// Rust path of the module that defines the `fn:` signature aliases.
    pub fn_sig_path: String,
    /// Directory whose `*.rs` files must all be listed in kernel.tsv (check 21).
    pub kernel_dir: Option<PathBuf>,
    pub emit_cargo_hints: bool,
}

pub struct Outcome {
    pub diags: Diagnostics,
    pub coverage: overlap::Coverage,
    pub written: usize,
    pub digest: u64,
}

pub fn preflight(cfg: &Config) -> (Diagnostics, Vec<tsv::Sheet>, schema::Schema, check::Typed, overlap::Coverage) {
    let mut d = Diagnostics::default();
    let book = tsv::load_book(&cfg.sheets, &mut d);
    let schema = schema::load(&book, &mut d);
    let typed = check::check(&book, &schema, &mut d);
    doctrine(&book, &mut d);
    let cov = overlap::coverage(&typed, cfg.strict, &mut d);
    overlap::assets(&typed, &cfg.root, &mut d);
    overlap::divergence(&book, &cfg.root.join("re/evidence/l5_reference.tsv"), &mut d);
    overlap::budget(&book, &typed, &mut d);
    if let Some(k) = &cfg.kernel_dir {
        kernel_allowlist(&typed, k, &cfg.root, &mut d);
    }
    book::check_groups(&book, &typed, &mut d);
    book::completeness(&typed, &mut d);
    (d, book, schema, typed, cov)
}

/// L0: emitter version recorded in doctrine must match this emitter (R12).
fn doctrine(book: &[tsv::Sheet], d: &mut Diagnostics) {
    let Some(doc) = book.iter().find(|s| s.name == "doctrine") else {
        d.push(diag::Diag::new(Sev::E, Layer::L0, "E-L0-DOCTRINE").msg("00-doctrine.tsv missing"));
        return;
    };
    let ver = doc.rows.iter().find(|r| doc.cell(r, "key") == Some("emitter_version")).and_then(|r| doc.cell(r, "value"));
    if ver != Some(emit::EMITTER_VERSION) {
        d.push(diag::Diag::new(Sev::E, Layer::L0, "E-L0-EMITTER").at("doctrine", 0).msg(format!("doctrine emitter_version {:?} != sheetty {}", ver, emit::EMITTER_VERSION)));
    }
}

/// Check 21: every hand-written module under the kernel dir is in kernel.tsv.
fn kernel_allowlist(typed: &check::Typed, dir: &std::path::Path, root: &std::path::Path, d: &mut Diagnostics) {
    let Some(k) = typed.get("kernel") else {
        d.push(diag::Diag::new(Sev::E, Layer::L0, "E-L0-KERNEL").msg("kernel.tsv missing"));
        return;
    };
    let listed: std::collections::BTreeSet<String> = k.rows.iter().filter_map(|r| k.get(r, "path").and_then(|v| v.as_str()).map(|s| s.to_string())).collect();
    let mut files = Vec::new();
    walk_rs(dir, &mut files);
    files.sort();
    for f in files {
        // both canonical: on Windows a canonical root is `\\?\C:\...` and a
        // file under a non-canonical kernel dir would never strip it
        let fc = f.canonicalize().unwrap_or_else(|_| f.clone());
        let rc = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let rel = fc.strip_prefix(&rc).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        if !listed.contains(&rel) {
            d.push(diag::Diag::new(Sev::E, Layer::L0, "E-L0-KERNEL").at("kernel", 0).msg(format!("{rel} is not listed in kernel.tsv (D4)")));
        }
    }
    for p in &listed {
        if !root.join(p).exists() {
            d.push(diag::Diag::new(Sev::E, Layer::L0, "E-L0-KERNEL").at("kernel", 0).row(p).msg("listed in kernel.tsv but file missing"));
        }
    }
}

fn walk_rs(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_rs(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

pub fn run(cfg: &Config) -> Result<Outcome, Diagnostics> {
    let (mut d, book, schema, typed, cov) = preflight(cfg);
    if d.errors() > 0 {
        return Err(d);
    }
    let cx = emit::Ctx { typed: &typed, book: &book, schema: &schema, fn_sig_path: cfg.fn_sig_path.clone() };
    let mut files = emit::render(&cx);
    // To-Do 4: the grouped JSON view of the book next to the generated code
    files.push(("book.json".into(), book::json(&typed, None)));
    // L7: emitter determinism, two independent renders must be byte-identical.
    let mut again = emit::render(&cx);
    again.push(("book.json".into(), book::json(&typed, None)));
    let digest = emit::digest(&files);
    if digest != emit::digest(&again) {
        d.push(diag::Diag::new(Sev::E, Layer::L7, "E-L7-NONDET").msg("two renders of the same sheet book differ"));
        return Err(d);
    }
    d.note(Layer::L7, format!("emitter output digest {digest:016x} ({} files)", files.len()));
    let mut written = 0;
    if let Some(out) = &cfg.out {
        written = emit::write_all(out, &files).map_err(|e| {
            let mut d2 = d.clone();
            d2.push(diag::Diag::new(Sev::E, Layer::L7, "E-EMIT-IO").msg(e.to_string()));
            d2
        })?;
        let _ = std::fs::write(out.join("preflight.json"), d.json());
    }
    if cfg.emit_cargo_hints {
        for s in &book {
            println!("cargo::rerun-if-changed={}", s.path.display());
        }
        println!("cargo::rerun-if-changed={}", cfg.sheets.display());
        println!("cargo::rerun-if-changed={}", cfg.root.join("re/evidence/l5_reference.tsv").display());
        if let Some(k) = &cfg.kernel_dir {
            println!("cargo::rerun-if-changed={}", k.display());
        }
    }
    Ok(Outcome { diags: d, coverage: cov, written, digest })
}
