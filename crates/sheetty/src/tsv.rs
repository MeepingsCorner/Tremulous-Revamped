//! Canonical TSV reader (MDD §5.2) with the L0 structural checks that can only
//! be done on raw bytes: UTF-8, BOM, LF, manifest, header, quoting.

use crate::diag::{Diag, Diagnostics, Layer, Sev};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Row {
    pub line: usize,
    pub cells: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Sheet {
    pub name: String,
    pub path: PathBuf,
    pub manifest: Vec<(String, String)>,
    pub header: Vec<String>,
    pub header_line: usize,
    pub rows: Vec<Row>,
    pub bytes: usize,
}

impl Sheet {
    pub fn meta(&self, k: &str) -> Option<&str> {
        self.manifest.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
    pub fn col(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| h == name)
    }
    pub fn key_cols(&self) -> Vec<usize> {
        match self.meta("key") {
            Some(k) => k.split(',').filter_map(|c| self.col(c.trim())).collect(),
            None => self.col("id").into_iter().collect(),
        }
    }
    pub fn key_of(&self, r: &Row) -> String {
        self.key_cols().iter().map(|&i| r.cells.get(i).map(|s| s.as_str()).unwrap_or("")).collect::<Vec<_>>().join(":")
    }
    pub fn cell<'a>(&self, r: &'a Row, col: &str) -> Option<&'a str> {
        self.col(col).and_then(|i| r.cells.get(i)).map(|s| s.as_str())
    }
    pub fn generator(&self) -> &str {
        self.meta("generator").unwrap_or("none")
    }
}

/// Split one line on TAB with RFC-4180 style quoting (no embedded newlines).
fn split_cells(line: &str, quoted: &mut bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut in_q = false;
    let mut at_start = true;
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_q = false;
                }
            } else {
                cur.push(c);
            }
            continue;
        }
        match c {
            '\t' => {
                out.push(std::mem::take(&mut cur));
                at_start = true;
                continue;
            }
            '"' if at_start => {
                in_q = true;
                *quoted = true;
            }
            c => cur.push(c),
        }
        at_start = false;
    }
    out.push(cur);
    out
}

pub fn load_sheet(path: &Path, rel: &str, d: &mut Diagnostics) -> Option<Sheet> {
    let stem = path.file_stem().unwrap().to_string_lossy().to_string();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-IO").at(&stem, 0).msg(format!("{rel}: {e}")));
            return None;
        }
    };
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        d.push(Diag::new(Sev::E, Layer::L0, "E-L0-BOM").at(&stem, 1).msg("UTF-8 BOM present"));
    }
    let text = match String::from_utf8(bytes.clone()) {
        Ok(t) => t,
        Err(e) => {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-UTF8").at(&stem, 0).msg(e.to_string()));
            return None;
        }
    };
    let mut manifest = Vec::new();
    let mut header: Option<(usize, Vec<String>)> = None;
    let mut rows = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let ln = i + 1;
        if raw.ends_with('\r') {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-EOL").at(&stem, ln).msg("non-LF line ending"));
        }
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with("//") {
            continue;
        }
        if header.is_none() && line.starts_with('#') {
            let body = line[1..].trim();
            let body = body.split("  #").next().unwrap_or(body);
            match body.split_once(':') {
                Some((k, v)) => manifest.push((k.trim().to_string(), v.trim().to_string())),
                None => d.push(Diag::new(Sev::E, Layer::L0, "E-L0-MANIFEST").at(&stem, ln).msg(format!("unparsable manifest line {line:?}"))),
            }
            continue;
        }
        let mut quoted = false;
        let cells = split_cells(line, &mut quoted);
        if header.is_none() {
            if cells.len() < 2 && !line.contains('\t') {
                d.push(Diag::new(Sev::E, Layer::L0, "E-L0-DELIM").at(&stem, ln).msg("header row is not TAB delimited"));
            }
            header = Some((ln, cells));
            continue;
        }
        if quoted {
            d.push(Diag::new(Sev::W, Layer::L1, "W-L1-QUOTED").at(&stem, ln).msg("quoted cell (merge hazard)"));
        }
        rows.push(Row { line: ln, cells });
    }
    let (header_line, header) = match header {
        Some(h) => h,
        None => {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-HEADER").at(&stem, 0).msg("no header row"));
            return None;
        }
    };
    let name = manifest.iter().find(|(k, _)| k == "sheet").map(|(_, v)| v.clone());
    match &name {
        None => d.push(Diag::new(Sev::E, Layer::L0, "E-L0-MANIFEST").at(&stem, 1).msg("manifest missing `sheet:`")),
        Some(n) if n != &stem && !stem.ends_with(&format!("-{n}")) => d.push(
            Diag::new(Sev::E, Layer::L0, "E-L0-MANIFEST").at(&stem, 1).msg(format!("manifest sheet `{n}` does not match file name `{stem}`")),
        ),
        _ => {}
    }
    let name = name.unwrap_or(stem);
    for r in &rows {
        if r.cells.len() != header.len() {
            d.push(
                Diag::new(Sev::E, Layer::L0, "E-L0-WIDTH")
                    .at(&name, r.line)
                    .msg(format!("{} cells, header has {}", r.cells.len(), header.len())),
            );
        }
    }
    Some(Sheet { name, path: path.to_path_buf(), manifest, header, header_line, rows, bytes: bytes.len() })
}

/// Load every `*.tsv` under `dir`, sorted by path for determinism (D9).
pub fn load_book(dir: &Path, d: &mut Diagnostics) -> Vec<Sheet> {
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    let mut out: Vec<Sheet> = Vec::new();
    for f in files {
        let rel = f.strip_prefix(dir).unwrap_or(&f).to_string_lossy().to_string();
        if let Some(s) = load_sheet(&f, &rel, d) {
            if out.iter().any(|o| o.name == s.name) {
                d.push(Diag::new(Sev::E, Layer::L0, "E-L0-DUPSHEET").at(&s.name, 0).msg(format!("sheet name defined twice ({rel})")));
                continue;
            }
            out.push(s);
        }
    }
    out
}

pub fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "tsv").unwrap_or(false) {
                out.push(p);
            }
        }
    }
}
