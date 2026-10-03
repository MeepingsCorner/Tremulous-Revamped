//! L0 (header/schema/keys) + L1 (types, domains, empty-vs-NULL) + L2 (foreign
//! keys, cycles, topological order). Collects every finding before returning.

use crate::diag::{Diag, Diagnostics, Layer, Sev};
use crate::schema::{ColSpec, Schema, Ty};
use crate::tsv::Sheet;
use petgraph::graphmap::DiGraphMap;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Str(String),
    Bool(bool),
    Int(i64),
    /// Canonical decimal text, kept verbatim so emission is exact.
    Float(String),
    Vec(Vec<String>),
    Enum(String),
    Flags(Vec<String>),
    List(Vec<Val>),
    Fn(String),
}

impl Val {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) | Val::Enum(s) | Val::Fn(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TRow {
    pub line: usize,
    pub key: String,
    pub vals: Vec<Val>,
}

#[derive(Clone, Debug)]
pub struct TSheet {
    pub name: String,
    pub cols: Vec<ColSpec>,
    pub rows: Vec<TRow>,
}

impl TSheet {
    pub fn ci(&self, c: &str) -> Option<usize> {
        self.cols.iter().position(|x| x.name == c)
    }
    pub fn get<'a>(&self, r: &'a TRow, c: &str) -> Option<&'a Val> {
        self.ci(c).map(|i| &r.vals[i])
    }
}

pub type Typed = BTreeMap<String, TSheet>;

fn is_int(s: &str) -> bool {
    let b = s.strip_prefix('-').unwrap_or(s);
    !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()) && !(b.len() > 1 && b.starts_with('0'))
}
fn is_float(s: &str) -> bool {
    let b = s.strip_prefix('-').unwrap_or(s);
    match b.split_once('.') {
        Some((i, f)) => !i.is_empty() && !f.is_empty() && i.bytes().all(|c| c.is_ascii_digit()) && f.bytes().all(|c| c.is_ascii_digit()) && b.parse::<f64>().map(|v| v.is_finite()).unwrap_or(false),
        None => false,
    }
}
pub fn is_ident_path(s: &str) -> bool {
    !s.is_empty() && s.split("::").all(|p| !p.is_empty() && p.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false) && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

pub fn parse_cell(raw: &str, ty: &Ty, schema: &Schema) -> Result<Val, String> {
    match ty {
        Ty::Str | Ty::Asset(_) => Ok(Val::Str(raw.to_string())),
        Ty::Bool => match raw {
            "0" => Ok(Val::Bool(false)),
            "1" => Ok(Val::Bool(true)),
            _ => Err(format!("`{raw}` is not bool (0/1)")),
        },
        Ty::Int { bits, signed } => {
            if !is_int(raw) {
                return Err(if is_float(raw) { format!("`{raw}` is float-form, column is integer") } else { format!("`{raw}` is not an integer") });
            }
            let v: i128 = raw.parse().map_err(|_| format!("`{raw}` out of range"))?;
            let (lo, hi): (i128, i128) = if *signed { (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1) } else { (0, (1i128 << bits) - 1) };
            if v < lo || v > hi {
                return Err(format!("`{raw}` out of range for {}{bits}", if *signed { 'i' } else { 'u' }));
            }
            Ok(Val::Int(v as i64))
        }
        Ty::F32 | Ty::F64 => {
            if !is_float(raw) {
                return Err(if is_int(raw) { format!("`{raw}` has no fractional part (write `{raw}.0`)") } else { format!("`{raw}` is not a float") });
            }
            Ok(Val::Float(raw.to_string()))
        }
        Ty::Vec(n) => {
            let parts: Vec<&str> = raw.split(' ').collect();
            if parts.len() != *n as usize {
                return Err(format!("expected {n} space-separated components, got {}", parts.len()));
            }
            for p in &parts {
                if !is_float(p) {
                    return Err(format!("vector component `{p}` is not a float"));
                }
            }
            Ok(Val::Vec(parts.iter().map(|s| s.to_string()).collect()))
        }
        Ty::Enum(e) => {
            let spec = schema.enums.get(e).ok_or_else(|| format!("enum `{e}` not declared in 01-enums.tsv"))?;
            if spec.variants.iter().any(|(v, _)| v == raw) {
                Ok(Val::Enum(raw.to_string()))
            } else {
                Err(format!("`{raw}` not in {e}"))
            }
        }
        Ty::Flags(e) => {
            let spec = schema.enums.get(e).ok_or_else(|| format!("flags `{e}` not declared in 01-enums.tsv"))?;
            let mut out = Vec::new();
            for f in raw.split('|') {
                if !spec.variants.iter().any(|(v, _)| v == f) {
                    return Err(format!("`{f}` not in {e}"));
                }
                out.push(f.to_string());
            }
            Ok(Val::Flags(out))
        }
        Ty::List(inner) => {
            let mut out = Vec::new();
            for p in raw.split(' ') {
                out.push(parse_cell(p, inner, schema)?);
            }
            Ok(Val::List(out))
        }
        Ty::Fn(_) => {
            if is_ident_path(raw) {
                Ok(Val::Fn(raw.to_string()))
            } else {
                Err(format!("`{raw}` is not a Rust path"))
            }
        }
    }
}

/// Built-in sheets that are not described by 01-schema.tsv.
pub const META_SHEETS: &[&str] = &["schema", "enums"];

pub fn check(book: &[Sheet], schema: &Schema, d: &mut Diagnostics) -> Typed {
    let mut typed = Typed::new();
    for sh in book {
        if META_SHEETS.contains(&sh.name.as_str()) {
            continue;
        }
        let Some(cols) = schema.cols(&sh.name) else {
            d.push(Diag::new(Sev::E, Layer::L0, "E-SCHEMA-DRIFT").at(&sh.name, sh.header_line).msg("sheet has no rows in 01-schema.tsv"));
            continue;
        };
        // L0: header == schema (names and order)
        let want: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        let have: Vec<&str> = sh.header.iter().map(|s| s.as_str()).collect();
        if want != have {
            let missing: Vec<_> = want.iter().filter(|w| !have.contains(w)).collect();
            let extra: Vec<_> = have.iter().filter(|h| !want.contains(h)).collect();
            d.push(Diag::new(Sev::E, Layer::L0, "E-SCHEMA-DRIFT").at(&sh.name, sh.header_line).msg(format!(
                "header != schema; missing {missing:?}, extra {extra:?}{}",
                if missing.is_empty() && extra.is_empty() { " (order differs)" } else { "" }
            )));
            continue;
        }
        for h in &sh.header {
            if !h.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_') {
                d.push(Diag::new(Sev::E, Layer::L0, "E-L0-NAME").at(&sh.name, sh.header_line).col(h).msg("column name must be snake_case"));
            }
        }
        // L0: keys
        let kc = sh.key_cols();
        if kc.is_empty() {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-KEY").at(&sh.name, sh.header_line).msg("no `id` column and no `key:` manifest entry"));
        }
        let mut seen = BTreeMap::new();
        for r in &sh.rows {
            let key = sh.key_of(r);
            for &i in &kc {
                let v = r.cells.get(i).map(|s| s.as_str()).unwrap_or("");
                if v.is_empty() || !v.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_') {
                    d.push(Diag::new(Sev::E, Layer::L0, "E-L0-ID").at(&sh.name, r.line).row(v).msg("key must match ^[a-z0-9_]+$"));
                }
            }
            if let Some(prev) = seen.insert(key.clone(), r.line) {
                d.push(Diag::new(Sev::E, Layer::L0, "E-L0-DUPKEY").at(&sh.name, r.line).row(&key).msg(format!("duplicate key (first at line {prev})")));
            }
        }
        // L1: types
        let mut rows = Vec::new();
        for r in &sh.rows {
            let key = sh.key_of(r);
            let mut vals = Vec::with_capacity(cols.len());
            for (i, c) in cols.iter().enumerate() {
                let raw = r.cells.get(i).map(|s| s.as_str()).unwrap_or("");
                let v = if raw == "NULL" {
                    if !c.optional && !matches!(c.ty, Ty::Flags(_)) {
                        d.push(Diag::new(Sev::E, Layer::L1, "E-L1-NULL").at(&sh.name, r.line).row(&key).col(&c.name).msg("NULL in a required column"));
                    }
                    Val::Null
                } else if raw.is_empty() {
                    match &c.default {
                        Some(def) => parse_cell(def, &c.ty, schema).unwrap_or(Val::Null),
                        None if c.optional => Val::Null,
                        None => {
                            d.push(Diag::new(Sev::E, Layer::L1, "E-L1-EMPTY").at(&sh.name, r.line).row(&key).col(&c.name).msg("empty cell, column is required and has no default"));
                            Val::Null
                        }
                    }
                } else {
                    match parse_cell(raw, &c.ty, schema) {
                        Ok(v) => v,
                        Err(e) => {
                            let code = match c.ty {
                                Ty::Enum(_) | Ty::Flags(_) => "E-L1-ENUM",
                                _ => "E-L1-TYPE",
                            };
                            d.push(Diag::new(Sev::E, Layer::L1, code).at(&sh.name, r.line).row(&key).col(&c.name).msg(e));
                            Val::Null
                        }
                    }
                };
                vals.push(v);
            }
            rows.push(TRow { line: r.line, key, vals });
        }
        typed.insert(sh.name.clone(), TSheet { name: sh.name.clone(), cols: cols.to_vec(), rows });
    }
    for name in schema.sheets.keys() {
        if !book.iter().any(|s| &s.name == name) {
            d.push(Diag::new(Sev::E, Layer::L0, "E-SCHEMA-DRIFT").at(name, 0).msg("declared in 01-schema.tsv but no such sheet"));
        }
    }
    refs(book, &typed, d);
    typed
}

fn key_set(t: &TSheet, col: &str) -> Option<BTreeSet<String>> {
    let i = t.ci(col)?;
    Some(
        t.rows
            .iter()
            .filter_map(|r| match &r.vals[i] {
                Val::Str(s) | Val::Enum(s) => Some(s.clone()),
                Val::Int(v) => Some(v.to_string()),
                _ => None,
            })
            .collect(),
    )
}

fn each_str(v: &Val, f: &mut dyn FnMut(&str)) {
    match v {
        Val::Str(s) => f(s),
        Val::List(l) => l.iter().for_each(|x| each_str(x, f)),
        _ => {}
    }
}

/// L2: foreign keys, sheet-level `requires` acyclicity, row-level self-reference
/// acyclicity (topological order exists).
fn refs(book: &[Sheet], typed: &Typed, d: &mut Diagnostics) {
    let mut sheet_graph: DiGraphMap<&str, ()> = DiGraphMap::new();
    for t in typed.values() {
        sheet_graph.add_node(&t.name);
        let declared: BTreeSet<String> = book
            .iter()
            .find(|s| s.name == t.name)
            .and_then(|s| s.meta("requires"))
            .map(|r| r.split(',').map(|x| x.trim().to_string()).filter(|x| x != "-" && !x.is_empty()).collect())
            .unwrap_or_default();
        let mut used = BTreeSet::new();
        for (ci, c) in t.cols.iter().enumerate() {
            let Some((ts, tc)) = &c.reference else { continue };
            let Some(target) = typed.get(ts) else {
                d.push(Diag::new(Sev::E, Layer::L2, "E-L2-REFSHEET").at(&t.name, 0).col(&c.name).msg(format!("references unknown sheet `{ts}`")));
                continue;
            };
            let Some(keys) = key_set(target, tc) else {
                d.push(Diag::new(Sev::E, Layer::L2, "E-L2-REFCOL").at(&t.name, 0).col(&c.name).msg(format!("references unknown column `{ts}.{tc}`")));
                continue;
            };
            if ts != &t.name {
                used.insert(ts.clone());
                sheet_graph.add_edge(&t.name, &target.name, ());
            }
            let mut self_edges: Vec<(String, String)> = Vec::new();
            for r in &t.rows {
                each_str(&r.vals[ci], &mut |v: &str| {
                    if !keys.contains(v) {
                        d.push(Diag::new(Sev::E, Layer::L2, "E-L2-REF").at(&t.name, r.line).row(&r.key).col(&c.name).msg(format!("`{v}` not in {ts}.{tc}")));
                    } else if ts == &t.name {
                        self_edges.push((r.key.clone(), v.to_string()));
                    }
                });
            }
            let cyclic_ok = book
                .iter()
                .find(|s| s.name == t.name)
                .and_then(|s| s.meta("cyclic"))
                .map(|v| v.split(',').any(|x| x.trim() == c.name))
                .unwrap_or(false);
            if !self_edges.is_empty() && !cyclic_ok {
                let mut g: DiGraphMap<&str, ()> = DiGraphMap::new();
                for (a, b) in &self_edges {
                    g.add_edge(a.as_str(), b.as_str(), ());
                }
                if let Err(cy) = petgraph::algo::toposort(&g, None) {
                    d.push(Diag::new(Sev::E, Layer::L2, "E-L2-CYCLE").at(&t.name, 0).col(&c.name).msg(format!("row reference cycle through `{}`", cy.node_id())));
                }
            }
        }
        for u in &used {
            if !declared.contains(u) {
                d.push(Diag::new(Sev::W, Layer::L2, "W-L2-REQUIRES").at(&t.name, 1).msg(format!("references `{u}` but manifest `requires:` does not list it")));
            }
        }
    }
    if let Err(cy) = petgraph::algo::toposort(&sheet_graph, None) {
        d.push(Diag::new(Sev::E, Layer::L2, "E-L2-CYCLE").msg(format!("sheet reference cycle through `{}`", cy.node_id())));
    }
}

/// Topological order of rows by a self-referencing list column (e.g. systems.after).
/// Kahn's algorithm with key tie-break so the order is deterministic (D9).
pub fn topo_rows(t: &TSheet, after_col: &str) -> Result<Vec<usize>, String> {
    let ci = t.ci(after_col).ok_or_else(|| format!("no column {after_col}"))?;
    let idx: BTreeMap<&str, usize> = t.rows.iter().enumerate().map(|(i, r)| (r.key.as_str(), i)).collect();
    let n = t.rows.len();
    let mut indeg = vec![0usize; n];
    let mut out_edges: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, r) in t.rows.iter().enumerate() {
        let mut deps = Vec::new();
        each_str(&r.vals[ci], &mut |v| deps.push(v.to_string()));
        for dep in deps {
            let j = *idx.get(dep.as_str()).ok_or_else(|| format!("{} after unknown {}", r.key, dep))?;
            out_edges[j].push(i);
            indeg[i] += 1;
        }
    }
    let mut ready: BTreeSet<(usize, &str)> = BTreeSet::new();
    for i in 0..n {
        if indeg[i] == 0 {
            ready.insert((i, t.rows[i].key.as_str()));
        }
    }
    let mut order = Vec::with_capacity(n);
    while let Some(&first) = ready.iter().next() {
        ready.remove(&first);
        let i = first.0;
        order.push(i);
        for &j in &out_edges[i] {
            indeg[j] -= 1;
            if indeg[j] == 0 {
                ready.insert((j, t.rows[j].key.as_str()));
            }
        }
    }
    if order.len() != n {
        return Err("cycle in `after` ordering".into());
    }
    Ok(order)
}
