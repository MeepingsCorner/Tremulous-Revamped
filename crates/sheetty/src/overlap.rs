//! L3 coverage (spec ⋈ impl full outer join), L4 assets, L5 divergence vs
//! reference evidence, L6 budget / dead columns. MDD §3.4, §6.4, §6.5.

use crate::check::{TSheet, Typed, Val};
use crate::diag::{Diag, Diagnostics, Layer, Sev};
use crate::schema::Ty;
use crate::tsv::Sheet;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Coverage {
    pub covered: Vec<(String, String)>,
    pub unimplemented: Vec<(String, u32, String)>, // key, blast radius, kind
    pub orphan: Vec<String>,
    pub divergent: Vec<(String, String)>,
    pub status_counts: BTreeMap<String, usize>,
    pub symbols: BTreeMap<String, BTreeMap<String, usize>>, // subsystem -> status -> n
}

fn s<'a>(t: &TSheet, r: &'a crate::check::TRow, c: &str) -> &'a str {
    t.get(r, c).and_then(|v| v.as_str()).unwrap_or("")
}

/// Inbound reference count for every `sheet:id` in the book.
pub fn inbound(typed: &Typed) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    for t in typed.values() {
        for (ci, c) in t.cols.iter().enumerate() {
            let Some((ts, _)) = &c.reference else { continue };
            for r in &t.rows {
                let mut f = |v: &str| *m.entry(format!("{ts}:{v}")).or_insert(0) += 1;
                match &r.vals[ci] {
                    Val::Str(x) => f(x),
                    Val::List(l) => l.iter().filter_map(|x| x.as_str()).for_each(f),
                    _ => {}
                }
            }
        }
    }
    m
}

pub fn coverage(typed: &Typed, strict: bool, d: &mut Diagnostics) -> Coverage {
    let mut cov = Coverage::default();
    let (Some(plan), Some(imp)) = (typed.get("plan"), typed.get("impl")) else {
        d.push(Diag::new(Sev::E, Layer::L3, "E-L3-NOPLAN").msg("02-plan.tsv and 03-impl.tsv are required"));
        return cov;
    };
    let inb = inbound(typed);
    // plan targets must exist
    for r in &plan.rows {
        let sh = s(plan, r, "sheet");
        let id = s(plan, r, "id");
        if sh == "feature" {
            continue;
        }
        match typed.get(sh) {
            None => d.push(Diag::new(Sev::E, Layer::L2, "E-L2-REF").at("plan", r.line).row(&r.key).col("sheet").msg(format!("no sheet `{sh}`"))),
            Some(t) => {
                if !t.rows.iter().any(|x| x.key == id) {
                    d.push(Diag::new(Sev::E, Layer::L2, "E-L2-REF").at("plan", r.line).row(&r.key).col("id").msg(format!("`{id}` not in {sh}")));
                }
            }
        }
    }
    let impl_rows: BTreeMap<&str, &crate::check::TRow> = imp.rows.iter().map(|r| (r.key.as_str(), r)).collect();
    let plan_keys: BTreeSet<&str> = plan.rows.iter().map(|r| r.key.as_str()).collect();
    for r in &plan.rows {
        match impl_rows.get(r.key.as_str()) {
            Some(ir) => {
                let st = s(imp, ir, "status").to_string();
                *cov.status_counts.entry(st.clone()).or_insert(0) += 1;
                let item = imp.get(ir, "rust_item").cloned().unwrap_or(Val::Null);
                if matches!(st.as_str(), "Done" | "Verified") && item == Val::Null {
                    cov.divergent.push((r.key.clone(), "status done but rust_item NULL".into()));
                    d.push(Diag::new(Sev::E, Layer::L3, "E-L3-DIVERGE").at("impl", ir.line).row(&r.key).col("rust_item").msg("status Done/Verified requires rust_item"));
                }
                cov.covered.push((r.key.clone(), st));
            }
            None => {
                let target = r.key.replacen(':', ":", 1);
                let br = inb.get(&target).copied().unwrap_or(0);
                let prio = plan.get(r, "priority").and_then(|v| v.as_int()).unwrap_or(3) as u32;
                cov.unimplemented.push((r.key.clone(), br * 10 + (10 - prio.min(9)), s(plan, r, "kind").to_string()));
            }
        }
    }
    for r in &imp.rows {
        if !plan_keys.contains(r.key.as_str()) {
            cov.orphan.push(r.key.clone());
            d.push(Diag::new(Sev::W, Layer::L3, "W-L3-ORPHAN").at("impl", r.line).row(&r.key).msg("implementation row with no plan row"));
        }
    }
    cov.unimplemented.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let sev = if strict { Sev::E } else { Sev::W };
    for (k, br, kind) in &cov.unimplemented {
        d.push(Diag::new(sev, Layer::L3, if strict { "E-L3-COVERAGE" } else { "W-L3-COVERAGE" }).at("plan", 0).row(k).msg(format!("unimplemented {kind} (rank {br})")));
    }
    // symbol map backlog (MDD §7.3)
    if let Some(sym) = typed.get("symbols") {
        for r in &sym.rows {
            *cov.symbols.entry(s(sym, r, "subsystem").to_string()).or_default().entry(s(sym, r, "status").to_string()).or_insert(0) += 1;
        }
    }
    d.note(
        Layer::L3,
        format!(
            "plan {} rows: {} covered, {} unimplemented, {} orphan, {} divergent; impl status {:?}",
            plan.rows.len(),
            cov.covered.len(),
            cov.unimplemented.len(),
            cov.orphan.len(),
            cov.divergent.len(),
            cov.status_counts
        ),
    );
    for (sub, st) in &cov.symbols {
        d.note(Layer::L3, format!("symbols[{sub}] {st:?}"));
    }
    cov
}

pub fn assets(typed: &Typed, root: &Path, d: &mut Diagnostics) {
    let mut n = 0;
    for t in typed.values() {
        for (ci, c) in t.cols.iter().enumerate() {
            if !matches!(c.ty, Ty::Asset(_)) {
                continue;
            }
            for r in &t.rows {
                if let Val::Str(p) = &r.vals[ci] {
                    n += 1;
                    if !root.join(p).exists() {
                        d.push(Diag::new(Sev::E, Layer::L4, "E-L4-ASSET").at(&t.name, r.line).row(&r.key).col(&c.name).msg(format!("{p} missing")));
                    }
                }
            }
        }
    }
    d.note(Layer::L4, format!("{n} asset reference(s) checked"));
}

/// L5: every (sheet,row,column,value) recorded from the original program must
/// equal the sheet cell, unless the sheet overrides it with a decision.
pub fn divergence(book: &[Sheet], evidence: &Path, d: &mut Diagnostics) {
    let Ok(text) = std::fs::read_to_string(evidence) else {
        d.note(Layer::L5, format!("no evidence file at {} (skipped)", evidence.display()));
        return;
    };
    let mut checked = 0;
    let mut diverged = 0;
    for line in text.lines().filter(|l| !l.starts_with('#')).skip(1) {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 4 {
            continue;
        }
        let (sheet, row, col, val) = (c[0], c[1], c[2], c[3]);
        let Some(sh) = book.iter().find(|s| s.name == sheet) else { continue };
        let Some(ci) = sh.col(col) else {
            continue;
        };
        checked += 1;
        match sh.rows.iter().find(|r| sh.key_of(r) == row) {
            None => {
                diverged += 1;
                d.push(Diag::new(Sev::W, Layer::L5, "W-L5-MISSING").at(sheet, 0).row(row).msg("row exists in original, absent from sheet"));
            }
            Some(r) => {
                if r.cells.get(ci).map(|s| s.as_str()) != Some(val) {
                    diverged += 1;
                    d.push(Diag::new(Sev::W, Layer::L5, "W-L5-DIVERGE").at(sheet, r.line).row(row).col(col).msg(format!("sheet={} original={val}", r.cells.get(ci).map(|s| s.as_str()).unwrap_or(""))));
                }
            }
        }
    }
    d.note(Layer::L5, format!("{checked} cells compared with original program, {diverged} diverge"));
}

pub fn budget(book: &[Sheet], typed: &Typed, d: &mut Diagnostics) {
    let mut largest = ("", 0usize);
    for sh in book {
        let tok = sh.bytes / 4; // heuristic recorded in decisions.tsv (dec006)
        let cap: usize = sh.meta("budget").and_then(|b| b.parse().ok()).unwrap_or(32_000);
        if tok > cap {
            d.push(Diag::new(Sev::E, Layer::L6, "E-L6-BUDGET").at(&sh.name, 0).msg(format!("~{tok} tokens > budget {cap}")));
        }
        if tok > largest.1 {
            largest = (&sh.name, tok);
        }
    }
    for t in typed.values() {
        if t.rows.len() < 2 {
            continue;
        }
        for (ci, c) in t.cols.iter().enumerate() {
            if t.rows.iter().all(|r| r.vals[ci] == Val::Null) {
                d.push(Diag::new(Sev::W, Layer::L6, "W-L6-DEADCOL").at(&t.name, 0).col(&c.name).msg("every cell is NULL: column buys nothing (token tax)"));
            }
        }
    }
    d.note(Layer::L6, format!("largest sheet: {}=~{} tok", largest.0, largest.1));
}
