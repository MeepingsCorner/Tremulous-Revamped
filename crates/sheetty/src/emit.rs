//! Emission (MDD §3.3, §5.6, §5.7, §5.9): one module per sheet, a small
//! registry of PHF maps, every write gated on content equality.
//!
//! Generators (manifest `generator:`):
//!   table   struct per row + const table (+ dense enum if `dense:`), cold split
//!   consts  one `pub const` per row; `kind` column picks the type
//!   enums   the 01-enums.tsv domains as `#[repr]` enums / flag constants
//!   none    validated only

use crate::check::{topo_rows, TSheet, Typed, Val};
use crate::schema::{ColSpec, Schema, Ty};
use crate::tsv::Sheet;
use std::fmt::Write as _;
use std::path::Path;

pub const EMITTER_VERSION: &str = "1";

pub fn camel(id: &str) -> String {
    let mut s: String = id
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        s.insert(0, 'N');
    }
    s
}
pub fn upper(id: &str) -> String {
    let s = id.to_ascii_uppercase();
    if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        format!("N{s}")
    } else {
        s
    }
}
/// English-ish singular of a CamelCase sheet name: MapEntities -> MapEntity,
/// Brushes -> Brush, Classes -> Class, Weapons -> Weapon.
pub fn singular(s: &str) -> String {
    if let Some(b) = s.strip_suffix("ies") {
        return format!("{b}y");
    }
    for suf in ["sses", "shes", "ches", "xes"] {
        if s.ends_with(suf) {
            return s[..s.len() - 2].to_string();
        }
    }
    s.strip_suffix('s').unwrap_or(s).to_string()
}

fn snake(name: &str) -> String {
    let mut o = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            o.push('_');
        }
        o.push(c.to_ascii_lowercase());
    }
    o
}
fn rs_str(s: &str) -> String {
    format!("{s:?}")
}

pub struct Ctx<'a> {
    pub typed: &'a Typed,
    pub book: &'a [Sheet],
    pub schema: &'a Schema,
    pub fn_sig_path: String,
}

impl Ctx<'_> {
    fn dense_of(&self, sheet: &str) -> Option<String> {
        self.book.iter().find(|s| s.name == sheet).and_then(|s| s.meta("dense")).map(|s| s.to_string())
    }
    fn sheet(&self, name: &str) -> &Sheet {
        self.book.iter().find(|s| s.name == name).unwrap()
    }
}

/// Rust type for one column.
fn rust_ty(cx: &Ctx, c: &ColSpec) -> String {
    let base = |ty: &Ty, refd: bool| -> String {
        match ty {
            Ty::Str | Ty::Asset(_) => {
                if refd {
                    if let Some((ts, _)) = &c.reference {
                        if let Some(dn) = cx.dense_of(ts) {
                            return format!("super::{}::{dn}", ident(ts));
                        }
                    }
                }
                "&'static str".into()
            }
            Ty::Bool => "bool".into(),
            Ty::Int { .. } => ty.rust_int().unwrap(),
            Ty::F32 => "f32".into(),
            Ty::F64 => "f64".into(),
            Ty::Vec(n) => format!("[f32; {n}]"),
            Ty::Enum(e) => format!("super::enums::{e}"),
            Ty::Flags(_) => "u32".into(),
            Ty::Fn(sig) => format!("{}::{sig}", cx.fn_sig_path),
            Ty::List(_) => unreachable!(),
        }
    };
    let t = match &c.ty {
        Ty::List(inner) => format!("&'static [{}]", base(inner, true)),
        Ty::Flags(_) => return "u32".into(),
        other => base(other, true),
    };
    if c.optional && !matches!(c.ty, Ty::List(_)) {
        format!("Option<{t}>")
    } else {
        t
    }
}

fn rust_val(cx: &Ctx, c: &ColSpec, ty: &Ty, v: &Val) -> String {
    match (ty, v) {
        (Ty::Flags(e), Val::Flags(fs)) => fs.iter().map(|f| format!("super::enums::{}::{}", snake(e), upper(&snake(f)))).collect::<Vec<_>>().join(" | "),
        (Ty::Flags(_), Val::Null) => "0".into(),
        (Ty::List(inner), Val::List(xs)) => format!("&[{}]", xs.iter().map(|x| rust_val(cx, c, inner, x)).collect::<Vec<_>>().join(", ")),
        (Ty::List(_), Val::Null) => "&[]".into(),
        (_, Val::Str(s)) => {
            if let Some((ts, _)) = &c.reference {
                if let Some(dn) = cx.dense_of(ts) {
                    return format!("super::{}::{dn}::{}", ident(ts), camel(s));
                }
            }
            rs_str(s)
        }
        (_, Val::Bool(b)) => b.to_string(),
        (Ty::Int { .. }, Val::Int(i)) => i.to_string(),
        (Ty::F64, Val::Float(f)) => format!("{f}_f64"),
        (_, Val::Float(f)) => format!("{f}_f32"),
        (_, Val::Vec(xs)) => format!("[{}]", xs.iter().map(|x| format!("{x}_f32")).collect::<Vec<_>>().join(", ")),
        (Ty::Enum(e), Val::Enum(x)) => format!("super::enums::{e}::{x}"),
        (_, Val::Fn(p)) => p.clone(),
        (_, Val::Null) => "None".into(),
        (t, v) => panic!("emit: no projection for {t:?} {v:?}"),
    }
}

fn opt_wrap(c: &ColSpec, v: &Val, s: String) -> String {
    if c.optional && !matches!(c.ty, Ty::List(_) | Ty::Flags(_)) && *v != Val::Null {
        format!("Some({s})")
    } else {
        s
    }
}

const ALLOW: &str = "#[allow(clippy::all, dead_code, non_upper_case_globals, non_camel_case_types, unused_imports)] ";
const KEYWORDS: &[&str] = &["as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen"];
/// A sheet name as a Rust module identifier (raw identifier for keywords).
pub fn ident(name: &str) -> String {
    if KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

fn banner(sheet: &str) -> String {
    format!("// GENERATED BY sheetty v{EMITTER_VERSION} FROM {sheet} - DO NOT EDIT\n")
}

fn emit_enums(cx: &Ctx) -> String {
    let mut o = banner("01-enums.tsv");
    for e in cx.schema.enums.values() {
        if e.flags {
            let _ = writeln!(o, "pub mod {} {{", snake(&e.name));
            for (v, n) in &e.variants {
                let _ = writeln!(o, "    pub const {}: u32 = {:#x};", upper(&snake(v)), n);
            }
            let _ = writeln!(o, "    pub const ALL: &[(&str, u32)] = &[{}];", e.variants.iter().map(|(v, _)| format!("({:?}, {})", v, upper(&snake(v)))).collect::<Vec<_>>().join(", "));
            let _ = writeln!(o, "}}");
        } else {
            let max = e.variants.iter().map(|x| x.1).max().unwrap_or(0);
            let min = e.variants.iter().map(|x| x.1).min().unwrap_or(0);
            let repr = if min >= 0 && max < 256 { "u8" } else { "i32" };
            let _ = writeln!(o, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]\n#[repr({repr})]\npub enum {} {{", e.name);
            // the zero-valued variant (else the first) is Default, like a memset C struct
            let dflt = e.variants.iter().position(|x| x.1 == 0).unwrap_or(0);
            for (k, (v, n)) in e.variants.iter().enumerate() {
                let attr = if k == dflt { "#[default] " } else { "" };
                let _ = writeln!(o, "    {attr}{v} = {n},");
            }
            let _ = writeln!(o, "}}\nimpl {} {{", e.name);
            let _ = writeln!(o, "    pub const ALL: [Self; {}] = [{}];", e.variants.len(), e.variants.iter().map(|(v, _)| format!("Self::{v}")).collect::<Vec<_>>().join(", "));
            let _ = writeln!(o, "    pub fn name(self) -> &'static str {{ match self {{ {} }} }}", e.variants.iter().map(|(v, _)| format!("Self::{v} => {v:?}")).collect::<Vec<_>>().join(", "));
            let _ = writeln!(o, "    pub fn from_raw(n: {repr}) -> Option<Self> {{ match n {{ {} _ => None }} }}", e.variants.iter().map(|(v, n)| format!("{n} => Some(Self::{v}),")).collect::<Vec<_>>().join(" "));
            let _ = writeln!(o, "}}");
        }
    }
    o
}

fn emit_consts(cx: &Ctx, t: &TSheet) -> String {
    let mut o = banner(&format!("{}.tsv", t.name));
    let ki = t.ci("kind").expect("consts sheet needs `kind`");
    let vi = t.ci("value").expect("consts sheet needs `value`");
    for r in &t.rows {
        let kind = r.vals[ki].as_str().unwrap_or("I32");
        let val = r.vals[vi].as_str().unwrap_or("0");
        let ty = match kind {
            "F32" => "f32",
            "F64" => "f64",
            "I32" => "i32",
            "U32" => "u32",
            "Str" => "&str",
            k => panic!("const kind {k}"),
        };
        let lit = match ty {
            "&str" => rs_str(val),
            "f32" => format!("{val}_f32"),
            "f64" => format!("{val}_f64"),
            _ => val.to_string(),
        };
        let _ = writeln!(o, "pub const {}: {} = {};", upper(&r.key), ty, lit);
    }
    let _ = cx;
    o
}

fn emit_table(cx: &Ctx, t: &TSheet, reg: &mut String) -> String {
    let sh = cx.sheet(&t.name);
    let stem = camel(&t.name);
    let row = singular(&stem);
    let def = format!("{row}Def");
    let cold = format!("{row}Cold");
    let up = upper(&t.name);
    let mut o = banner(&format!("{}.tsv", t.name));
    // row order: topological if requested, else sheet order
    let order: Vec<usize> = match sh.meta("order_after") {
        Some(col) => topo_rows(t, col).expect("topo order validated at L2"),
        None => (0..t.rows.len()).collect(),
    };
    let hot: Vec<(usize, &ColSpec)> = t.cols.iter().enumerate().filter(|(_, c)| !c.cold).collect();
    let coldc: Vec<(usize, &ColSpec)> = t.cols.iter().enumerate().filter(|(_, c)| c.cold).collect();
    let dense = sh.meta("dense").map(|s| s.to_string());
    let numc = t.ci("num");

    let _ = writeln!(o, "/// Hot columns of `{}` (one strut per row).\n#[derive(Clone, Copy, Debug)]\npub struct {def} {{", t.name);
    for (_, c) in &hot {
        let ty = if c.name == "id" { "&'static str".to_string() } else { rust_ty(cx, c) };
        let _ = writeln!(o, "    pub {}: {},", ident(&c.name), ty);
    }
    let _ = writeln!(o, "}}");
    if !coldc.is_empty() {
        let _ = writeln!(o, "/// Cold columns of `{}` (UI, provenance): kept out of the hot table (MDD §9.2).\n#[derive(Clone, Copy, Debug)]\npub struct {cold} {{", t.name);
        for (_, c) in &coldc {
            let _ = writeln!(o, "    pub {}: {},", ident(&c.name), rust_ty(cx, c));
        }
        let _ = writeln!(o, "}}");
    }
    let n = order.len();
    let _ = writeln!(o, "pub const {up}_COUNT: usize = {n};");
    let _ = writeln!(o, "pub static {up}: [{def}; {n}] = [");
    for &i in &order {
        let r = &t.rows[i];
        let _ = write!(o, "    {def} {{");
        for (ci, c) in &hot {
            let v = &r.vals[*ci];
            let lit = if c.name == "id" { rs_str(&r.key) } else { opt_wrap(c, v, rust_val(cx, c, &c.ty, v)) };
            let _ = write!(o, " {}: {},", ident(&c.name), lit);
        }
        let _ = writeln!(o, " }}, // {}.tsv:{}", t.name, r.line);
    }
    let _ = writeln!(o, "];");
    if !coldc.is_empty() {
        let _ = writeln!(o, "pub static {up}_COLD: [{cold}; {n}] = [");
        for &i in &order {
            let r = &t.rows[i];
            let _ = write!(o, "    {cold} {{");
            for (ci, c) in &coldc {
                let v = &r.vals[*ci];
                let _ = write!(o, " {}: {},", ident(&c.name), opt_wrap(c, v, rust_val(cx, c, &c.ty, v)));
            }
            let _ = writeln!(o, " }},");
        }
        let _ = writeln!(o, "];");
    }
    // dense index (MDD §5.7 row 2)
    if let Some(dn) = &dense {
        let max_disc = order.iter().enumerate().map(|(p, &i)| numc.and_then(|c| t.rows[i].vals[c].as_int()).unwrap_or(p as i64)).max().unwrap_or(0);
        let repr = if max_disc < 256 { "u8" } else { "u16" };
        let _ = writeln!(o, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]\n#[repr({repr})]\npub enum {dn} {{");
        for (pos, &i) in order.iter().enumerate() {
            let r = &t.rows[i];
            let disc = numc.and_then(|c| r.vals[c].as_int()).unwrap_or(pos as i64);
            let _ = writeln!(o, "    {} = {},", camel(&r.key), disc);
        }
        let _ = writeln!(o, "}}\nimpl {dn} {{\n    pub const COUNT: usize = {n};");
        let _ = writeln!(o, "    pub const ALL: [Self; {n}] = [{}];", order.iter().map(|&i| format!("Self::{}", camel(&t.rows[i].key))).collect::<Vec<_>>().join(", "));
        let _ = writeln!(o, "    #[inline] pub const fn index(self) -> usize {{ match self {{ {} }} }}", order.iter().enumerate().map(|(p, &i)| format!("Self::{} => {p},", camel(&t.rows[i].key))).collect::<Vec<_>>().join(" "));
        let _ = writeln!(o, "    #[inline] pub const fn num(self) -> {repr} {{ self as {repr} }}");
        let _ = writeln!(o, "    pub const fn from_num(n: {repr}) -> Option<Self> {{ match n {{ {} _ => None }} }}", order.iter().enumerate().map(|(pos, &i)| {
            let r = &t.rows[i];
            let disc = numc.and_then(|c| r.vals[c].as_int()).unwrap_or(pos as i64);
            format!("{disc} => Some(Self::{}),", camel(&r.key))
        }).collect::<Vec<_>>().join(" "));
        let _ = writeln!(o, "    #[inline] pub fn def(self) -> &'static {def} {{ &{up}[self.index()] }}");
        if !coldc.is_empty() {
            let _ = writeln!(o, "    #[inline] pub fn cold(self) -> &'static {cold} {{ &{up}_COLD[self.index()] }}");
        }
        let _ = writeln!(o, "    #[inline] pub fn id(self) -> &'static str {{ {up}[self.index()].id }}");
        let _ = writeln!(o, "    pub fn from_id(s: &str) -> Option<Self> {{ super::registry::{up}_BY_ID.get(s).copied() }}");
        let _ = writeln!(o, "}}");
        // PHF registry entry (cold path, MDD §5.7 row 1)
        let mut m = phf_codegen::Map::new();
        for &i in &order {
            m.entry(t.rows[i].key.clone(), format!("super::{}::{}::{}", ident(&t.name), dn, camel(&t.rows[i].key)));
        }
        let _ = writeln!(reg, "pub static {up}_BY_ID: phf::Map<&'static str, super::{}::{dn}> = {};", ident(&t.name), m.build());
    } else if let Some(Some((ts, _))) = t.ci("id").map(|i| t.cols[i].reference.clone()) {
        // keyed by another sheet's id: emit a by-target lookup array
        if let Some(tdn) = cx.dense_of(&ts) {
            let target = cx.typed.get(&ts).unwrap();
            let mut slots = vec!["None".to_string(); target.rows.len()];
            let tord: Vec<String> = match cx.sheet(&ts).meta("order_after") {
                Some(col) => topo_rows(target, col).unwrap().into_iter().map(|i| target.rows[i].key.clone()).collect(),
                None => target.rows.iter().map(|r| r.key.clone()).collect(),
            };
            for (p, &i) in order.iter().enumerate() {
                if let Some(ti) = tord.iter().position(|k| *k == t.rows[i].key) {
                    slots[ti] = format!("Some({p})");
                }
            }
            let _ = writeln!(o, "static {up}_BY_TARGET: [Option<u16>; {}] = [{}];", slots.len(), slots.join(", "));
            let _ = writeln!(o, "pub fn for_target(t: super::{}::{tdn}) -> Option<&'static {def}> {{ {up}_BY_TARGET[t.index()].map(|i| &{up}[i as usize]) }}", ident(&ts));
        }
    } else if sh.meta("index") == Some("by_id") {
        let mut m = phf_codegen::Map::new();
        for (p, &i) in order.iter().enumerate() {
            m.entry(t.rows[i].key.clone(), format!("{p}u16"));
        }
        let _ = writeln!(reg, "pub static {up}_BY_ID: phf::Map<&'static str, u16> = {};", m.build());
        let _ = writeln!(o, "pub fn by_id(s: &str) -> Option<&'static {def}> {{ super::registry::{up}_BY_ID.get(s).map(|&i| &{up}[i as usize]) }}");
    }
    o
}

/// `paths`: every `fn:` cell must name a real Rust item; `use <path> as _;`
/// makes rustc prove it (an impl row that points nowhere fails the build).
fn emit_paths(t: &TSheet) -> String {
    let mut o = banner(&format!("{}.tsv", t.name));
    for (ci, c) in t.cols.iter().enumerate() {
        if !matches!(c.ty, Ty::Fn(_)) {
            continue;
        }
        for r in &t.rows {
            if let Val::Fn(p) = &r.vals[ci] {
                let _ = writeln!(o, "#[allow(unused_imports)] use {p} as _; // {}.tsv:{} {}", t.name, r.line, r.key);
            }
        }
    }
    o
}

/// Produce every generated file in memory: (relative path, contents).
pub fn render(cx: &Ctx) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut reg = banner("registry (all sheets)");
    let mut mods = String::from("// GENERATED BY sheetty - module tree for every emitted sheet\n");
    if !cx.schema.enums.is_empty() {
        files.push(("sheets/enums.rs".into(), emit_enums(cx)));
        let _ = writeln!(mods, "{ALLOW}pub mod enums {{ include!(concat!(env!(\"OUT_DIR\"), \"/sheets/enums.rs\")); }}");
    }
    for (name, t) in cx.typed.iter() {
        let sh = cx.sheet(name);
        let body = match sh.generator() {
            "table" => emit_table(cx, t, &mut reg),
            "consts" => emit_consts(cx, t),
            "paths" => emit_paths(t),
            _ => continue,
        };
        files.push((format!("sheets/{name}.rs"), body));
        let _ = writeln!(mods, "{ALLOW}pub mod {} {{ include!(concat!(env!(\"OUT_DIR\"), \"/sheets/{name}.rs\")); }}", ident(name));
    }
    let _ = writeln!(mods, "{ALLOW}pub mod registry {{ include!(concat!(env!(\"OUT_DIR\"), \"/registry.rs\")); }}");
    files.push(("registry.rs".into(), reg));
    files.push(("gen_mods.rs".into(), mods));
    files
}

/// Hash-gated write (MDD §5.9 rule 2): untouched bytes keep their mtime.
pub fn write_all(out: &Path, files: &[(String, String)]) -> std::io::Result<usize> {
    let mut written = 0;
    for (rel, body) in files {
        let p = out.join(rel);
        if let Ok(old) = std::fs::read(&p) {
            if old == body.as_bytes() {
                continue;
            }
        }
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&p, body)?;
        written += 1;
    }
    Ok(written)
}

pub fn digest(files: &[(String, String)]) -> u64 {
    // FNV-1a over (path, body): stable across runs and platforms.
    let mut h: u64 = 0xcbf29ce484222325;
    for (p, b) in files {
        for byte in p.bytes().chain([0u8]).chain(b.bytes()) {
            h ^= byte as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

