//! Diagnostics: stable code, location, human + JSON forms (MDD §5.8, §6.3).

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sev {
    E,
    W,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    L0,
    L1,
    L2,
    L3,
    L4,
    L5,
    L6,
    L7,
    L8,
}

impl Layer {
    pub const ALL: [Layer; 9] = [
        Layer::L0,
        Layer::L1,
        Layer::L2,
        Layer::L3,
        Layer::L4,
        Layer::L5,
        Layer::L6,
        Layer::L7,
        Layer::L8,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Layer::L0 => "structural",
            Layer::L1 => "types",
            Layer::L2 => "refs",
            Layer::L3 => "coverage",
            Layer::L4 => "assets",
            Layer::L5 => "divergence",
            Layer::L6 => "budget",
            Layer::L7 => "determinism",
            Layer::L8 => "completeness",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub sev: Sev,
    pub layer: Layer,
    pub code: String,
    pub sheet: String,
    pub line: usize,
    pub row: String,
    pub col: String,
    pub msg: String,
}

impl Diag {
    pub fn new(sev: Sev, layer: Layer, code: &str) -> Self {
        Diag {
            sev,
            layer,
            code: code.to_string(),
            sheet: String::new(),
            line: 0,
            row: String::new(),
            col: String::new(),
            msg: String::new(),
        }
    }
    pub fn at(mut self, sheet: &str, line: usize) -> Self {
        self.sheet = sheet.to_string();
        self.line = line;
        self
    }
    pub fn row(mut self, row: &str) -> Self {
        self.row = row.to_string();
        self
    }
    pub fn col(mut self, col: &str) -> Self {
        self.col = col.to_string();
        self
    }
    pub fn msg(mut self, m: impl Into<String>) -> Self {
        self.msg = m.into();
        self
    }
    pub fn human(&self) -> String {
        let mut s = format!("{:<16}", self.code);
        if !self.sheet.is_empty() {
            if self.line > 0 {
                let _ = write!(s, " {}.tsv:{}", self.sheet, self.line);
            } else {
                let _ = write!(s, " {}", self.sheet);
            }
        }
        if !self.row.is_empty() {
            let _ = write!(s, "  {}", self.row);
            if !self.col.is_empty() {
                let _ = write!(s, ".{}", self.col);
            }
        } else if !self.col.is_empty() {
            let _ = write!(s, "  col={}", self.col);
        }
        if !self.msg.is_empty() {
            let _ = write!(s, "  {}", self.msg);
        }
        s
    }
    pub fn json(&self) -> String {
        format!(
            "{{\"sev\":\"{:?}\",\"layer\":\"{:?}\",\"code\":{},\"sheet\":{},\"line\":{},\"row\":{},\"col\":{},\"msg\":{}}}",
            self.sev,
            self.layer,
            js(&self.code),
            js(&self.sheet),
            self.line,
            js(&self.row),
            js(&self.col),
            js(&self.msg)
        )
    }
}

pub fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
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
    o
}

/// Collected findings for one preflight run. Never short-circuits on the first
/// error: one invocation reports the whole picture (MDD §6.1).
#[derive(Default, Debug, Clone)]
pub struct Diagnostics {
    pub items: Vec<Diag>,
    pub notes: Vec<(Layer, String)>,
}

impl Diagnostics {
    pub fn push(&mut self, d: Diag) {
        self.items.push(d);
    }
    pub fn note(&mut self, l: Layer, s: impl Into<String>) {
        self.notes.push((l, s.into()));
    }
    pub fn errors(&self) -> usize {
        self.items.iter().filter(|d| d.sev == Sev::E).count()
    }
    pub fn warnings(&self) -> usize {
        self.items.iter().filter(|d| d.sev == Sev::W).count()
    }
    pub fn has_errors_in(&self, l: Layer) -> bool {
        self.items.iter().any(|d| d.sev == Sev::E && d.layer == l)
    }
    pub fn human(&self, header: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "{header}");
        for l in Layer::ALL {
            let e = self.items.iter().filter(|d| d.layer == l && d.sev == Sev::E).count();
            let w = self.items.iter().filter(|d| d.layer == l && d.sev == Sev::W).count();
            let status = match (e, w) {
                (0, 0) => "ok".to_string(),
                (0, w) => format!("{w} warning(s)"),
                (e, 0) => format!("{e} error(s)"),
                (e, w) => format!("{e} error(s), {w} warning(s)"),
            };
            let label = format!("{:?} {} ", l, l.name());
            let _ = writeln!(s, "  {:.<28} {}", label, status);
            for (nl, n) in &self.notes {
                if *nl == l {
                    let _ = writeln!(s, "      {n}");
                }
            }
            let mut shown = 0;
            for d in self.items.iter().filter(|d| d.layer == l) {
                if shown == 40 {
                    let _ = writeln!(s, "      ... (+{} more, see preflight.json)", self.items.iter().filter(|d| d.layer == l).count() - 40);
                    break;
                }
                let _ = writeln!(s, "      {}", d.human());
                shown += 1;
            }
        }
        let _ = writeln!(
            s,
            "\n{}  {} error(s), {} warning(s)",
            if self.errors() == 0 { "PASSED" } else { "FAILED" },
            self.errors(),
            self.warnings()
        );
        s
    }
    pub fn json(&self) -> String {
        let items: Vec<String> = self.items.iter().map(|d| d.json()).collect();
        let notes: Vec<String> = self
            .notes
            .iter()
            .map(|(l, n)| format!("{{\"layer\":\"{:?}\",\"note\":{}}}", l, js(n)))
            .collect();
        format!(
            "{{\"errors\":{},\"warnings\":{},\"items\":[{}],\"notes\":[{}]}}",
            self.errors(),
            self.warnings(),
            items.join(","),
            notes.join(",")
        )
    }
}
