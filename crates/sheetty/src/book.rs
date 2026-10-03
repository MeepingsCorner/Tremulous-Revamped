//! The process layers the Spreadsheet-Driven Build prompt adds on top of
//! L0..L7:
//!
//! * groups (To-Do 4): every sheet belongs to exactly one group in
//!   00-groups.tsv, and the whole book is readable as one grouped JSON
//!   document (`book.json` in OUT_DIR, `sheetty json`).
//! * L8 completeness (To-Do 5): every row x column intersection must be
//!   checked. An intersection is unchecked when it holds a placeholder
//!   (TODO / TBD / FIXME / XXX / ?) or when its row carries a `Status` that is
//!   not final (Todo, Stub, Partial, Blocked). An unchecked intersection makes
//!   its row unfinished, and unfinished rows are errors: the build does not
//!   proceed until every intersection is checked.

use crate::check::{TSheet, Typed, Val};
use crate::diag::{Diag, Diagnostics, Layer, Sev};
use crate::schema::Ty;
use crate::tsv::Sheet;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Final values of the `Status` enum.
pub const FINAL_STATUS: &[&str] = &["Done", "Verified", "Replaced", "Dropped"];
const PLACEHOLDERS: &[&str] = &["TODO", "TBD", "FIXME", "XXX", "?", "??", "???"];

/// The completeness matrix of one sheet.
#[derive(Clone, Debug, Default)]
pub struct SheetMatrix {
    pub rows: usize,
    pub cols: usize,
    pub checked: usize,
    /// (row key, column, reason)
    pub unfinished: Vec<(String, String, String)>,
}

impl SheetMatrix {
    pub fn cells(&self) -> usize {
        self.rows * self.cols
    }
}

/// sheet -> group, from 00-groups.tsv.
pub fn groups(typed: &Typed) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(g) = typed.get("groups") {
        for r in &g.rows {
            let sh = g.get(r, "sheet").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let gr = g.get(r, "group").and_then(|v| v.as_str()).unwrap_or("").to_string();
            m.insert(sh, gr);
        }
    }
    m
}

/// L0: every sheet of the book is in exactly one group, and every grouped
/// sheet exists.
pub fn check_groups(book: &[Sheet], typed: &Typed, d: &mut Diagnostics) {
    if typed.get("groups").is_none() {
        d.push(Diag::new(Sev::E, Layer::L0, "E-L0-GROUP").msg("00-groups.tsv missing (To-Do 4: every sheet needs a group)"));
        return;
    }
    let g = groups(typed);
    for s in book {
        if !g.contains_key(&s.name) {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-GROUP").at(&s.name, 0).msg(format!("sheet `{}` has no row in 00-groups.tsv", s.name)));
        }
    }
    for sh in g.keys() {
        if !book.iter().any(|s| &s.name == sh) {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-GROUP").at("groups", 0).row(sh).msg("grouped sheet does not exist"));
        }
    }
}

fn is_placeholder(s: &str) -> bool {
    let t = s.trim();
    PLACEHOLDERS.iter().any(|p| t.eq_ignore_ascii_case(p))
        || t.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "TODO" || w == "FIXME" || w == "TBD")
}

fn val_placeholder(v: &Val) -> bool {
    match v {
        Val::Str(s) | Val::Enum(s) | Val::Fn(s) => is_placeholder(s),
        Val::List(xs) => xs.iter().any(val_placeholder),
        Val::Flags(xs) | Val::Vec(xs) => xs.iter().any(|x| is_placeholder(x)),
        _ => false,
    }
}

/// L8: the checkmark matrix of every sheet; unchecked intersections are
/// reported as errors (E-L8-UNCHECKED cell, E-L8-UNFINISHED row status).
pub fn completeness(typed: &Typed, d: &mut Diagnostics) -> BTreeMap<String, SheetMatrix> {
    let mut out = BTreeMap::new();
    for (name, t) in typed {
        let mut m = SheetMatrix { rows: t.rows.len(), cols: t.cols.len(), ..Default::default() };
        let status_cols: Vec<usize> = t.cols.iter().enumerate().filter(|(_, c)| matches!(&c.ty, Ty::Enum(e) if e == "Status")).map(|(i, _)| i).collect();
        for r in &t.rows {
            let row_status = status_cols.iter().find_map(|&i| match &r.vals[i] {
                Val::Enum(s) if !FINAL_STATUS.contains(&s.as_str()) => Some((t.cols[i].name.clone(), s.clone())),
                _ => None,
            });
            let mut row_ok = 0;
            for (ci, c) in t.cols.iter().enumerate() {
                if val_placeholder(&r.vals[ci]) {
                    m.unfinished.push((r.key.clone(), c.name.clone(), "placeholder".into()));
                    d.push(Diag::new(Sev::E, Layer::L8, "E-L8-UNCHECKED").at(name, r.line).row(&r.key).col(&c.name).msg("placeholder value: this intersection is not checked"));
                } else if row_status.is_none() {
                    row_ok += 1;
                }
            }
            if let Some((col, st)) = &row_status {
                m.unfinished.push((r.key.clone(), col.clone(), format!("status {st}")));
                d.push(Diag::new(Sev::E, Layer::L8, "E-L8-UNFINISHED").at(name, r.line).row(&r.key).col(col).msg(format!("status {st} is not final ({}): every intersection of this row is unchecked", FINAL_STATUS.join("/"))));
            }
            m.checked += row_ok;
        }
        out.insert(name.clone(), m);
    }
    let cells: usize = out.values().map(|m| m.cells()).sum();
    let checked: usize = out.values().map(|m| m.checked).sum();
    let rows: usize = out.values().map(|m| m.rows).sum();
    let unfinished_rows: usize = out.values().map(|m| m.unfinished.iter().map(|u| &u.0).collect::<std::collections::BTreeSet<_>>().len()).sum();
    d.note(Layer::L8, format!("checkmark matrix: {checked}/{cells} intersections checked, {unfinished_rows}/{rows} rows unfinished"));
    out
}

fn js(s: &str, o: &mut String) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

fn jv(v: &Val, o: &mut String) {
    match v {
        Val::Null => o.push_str("null"),
        Val::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
        Val::Int(i) => {
            let _ = write!(o, "{i}");
        }
        Val::Float(f) => o.push_str(f),
        Val::Str(s) | Val::Enum(s) | Val::Fn(s) => js(s, o),
        Val::Vec(xs) => {
            o.push('[');
            o.push_str(&xs.join(","));
            o.push(']');
        }
        Val::Flags(xs) => {
            o.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                js(x, o);
            }
            o.push(']');
        }
        Val::List(xs) => {
            o.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                jv(x, o);
            }
            o.push(']');
        }
    }
}

fn sheet_json(t: &TSheet, o: &mut String) {
    o.push_str("{\"columns\":[");
    for (i, c) in t.cols.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        js(&c.name, o);
    }
    o.push_str("],\"rows\":[");
    for (ri, r) in t.rows.iter().enumerate() {
        if ri > 0 {
            o.push(',');
        }
        o.push('{');
        for (ci, c) in t.cols.iter().enumerate() {
            if ci > 0 {
                o.push(',');
            }
            js(&c.name, o);
            o.push(':');
            jv(&r.vals[ci], o);
        }
        o.push('}');
    }
    o.push_str("]}");
}

/// The whole book as one JSON document, sheets grouped by 00-groups.tsv
/// (To-Do 4). Deterministic: groups and sheets in name order, rows in sheet
/// order, columns in schema order. `only` limits it to one group or sheet.
pub fn json(typed: &Typed, only: Option<&str>) -> String {
    let g = groups(typed);
    let mut by: BTreeMap<String, Vec<&TSheet>> = BTreeMap::new();
    for (name, t) in typed {
        let grp = g.get(name).cloned().unwrap_or_else(|| "Ungrouped".into());
        if let Some(f) = only {
            if !f.eq_ignore_ascii_case(&grp) && f != name {
                continue;
            }
        }
        by.entry(grp).or_default().push(t);
    }
    let mut o = String::from("{\"groups\":{");
    for (gi, (grp, sheets)) in by.iter().enumerate() {
        if gi > 0 {
            o.push(',');
        }
        js(grp, &mut o);
        o.push_str(":{");
        for (si, t) in sheets.iter().enumerate() {
            if si > 0 {
                o.push(',');
            }
            js(&t.name, &mut o);
            o.push(':');
            sheet_json(t, &mut o);
        }
        o.push('}');
    }
    o.push_str("}}\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::TRow;
    use crate::schema::ColSpec;

    fn col(name: &str, ty: Ty) -> ColSpec {
        ColSpec { name: name.into(), ty, unit: "-".into(), optional: false, reference: None, default: None, cold: false, line: 0 }
    }

    fn sheet(rows: Vec<Vec<Val>>) -> Typed {
        let cols = vec![col("id", Ty::Str), col("notes", Ty::Str), col("status", Ty::Enum("Status".into()))];
        let rows = rows.into_iter().enumerate().map(|(i, vals)| TRow { line: i + 1, key: vals[0].as_str().unwrap().to_string(), vals }).collect();
        let mut t = Typed::new();
        t.insert("things".into(), TSheet { name: "things".into(), cols, rows });
        t
    }

    #[test]
    fn every_intersection_must_be_checked() {
        let s = |x: &str| Val::Str(x.into());
        let e = |x: &str| Val::Enum(x.into());
        let typed = sheet(vec![
            vec![s("a"), s("fine"), e("Done")],
            vec![s("b"), s("TODO later"), e("Verified")],
            vec![s("c"), s("ok"), e("Partial")],
            vec![s("d"), Val::Null, e("Dropped")],
        ]);
        let mut d = Diagnostics::default();
        let m = completeness(&typed, &mut d);
        let t = &m["things"];
        assert_eq!((t.rows, t.cols, t.cells()), (4, 3, 12));
        // a: 3, b: 2 (the TODO cell is unchecked), c: 0 (unfinished row), d: 3
        assert_eq!(t.checked, 8);
        let codes: Vec<&str> = d.items.iter().map(|x| x.code.as_str()).collect();
        assert_eq!(codes, ["E-L8-UNCHECKED", "E-L8-UNFINISHED"]);
        assert_eq!(d.errors(), 2);
    }

    #[test]
    fn json_groups_like_items() {
        let typed = sheet(vec![vec![Val::Str("a".into()), Val::Str("say \"hi\"".into()), Val::Enum("Done".into())]]);
        let j = json(&typed, None);
        assert_eq!(j, "{\"groups\":{\"Ungrouped\":{\"things\":{\"columns\":[\"id\",\"notes\",\"status\"],\"rows\":[{\"id\":\"a\",\"notes\":\"say \\\"hi\\\"\",\"status\":\"Done\"}]}}}}\n");
        assert_eq!(json(&typed, Some("nothing")), "{\"groups\":{}}\n");
    }
}
