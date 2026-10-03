//! The authoritative column declarations (`01-schema.tsv`) and enum domains
//! (`01-enums.tsv`). Appendix B type tokens.

use crate::diag::{Diag, Diagnostics, Layer, Sev};
use crate::tsv::Sheet;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Str,
    Bool,
    Int { bits: u8, signed: bool },
    F32,
    F64,
    Vec(u8),
    Enum(String),
    Flags(String),
    List(Box<Ty>),
    Fn(String),
    Asset(String),
}

impl Ty {
    pub fn parse(s: &str) -> Option<Ty> {
        Some(match s {
            "string" => Ty::Str,
            "bool" => Ty::Bool,
            "i8" => Ty::Int { bits: 8, signed: true },
            "i16" => Ty::Int { bits: 16, signed: true },
            "i32" => Ty::Int { bits: 32, signed: true },
            "i64" => Ty::Int { bits: 64, signed: true },
            "u8" => Ty::Int { bits: 8, signed: false },
            "u16" => Ty::Int { bits: 16, signed: false },
            "u32" => Ty::Int { bits: 32, signed: false },
            "u64" => Ty::Int { bits: 64, signed: false },
            "f32" => Ty::F32,
            "f64" => Ty::F64,
            "vec2" => Ty::Vec(2),
            "vec3" => Ty::Vec(3),
            "vec4" => Ty::Vec(4),
            _ => {
                let (k, v) = s.split_once(':')?;
                if v.is_empty() {
                    return None;
                }
                match k {
                    "enum" => Ty::Enum(v.to_string()),
                    "flags" => Ty::Flags(v.to_string()),
                    "list" => Ty::List(Box::new(Ty::parse(v)?)),
                    "fn" => Ty::Fn(v.to_string()),
                    "asset" => Ty::Asset(v.to_string()),
                    _ => return None,
                }
            }
        })
    }
    pub fn rust_int(&self) -> Option<String> {
        match self {
            Ty::Int { bits, signed } => Some(format!("{}{}", if *signed { 'i' } else { 'u' }, bits)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ColSpec {
    pub name: String,
    pub ty: Ty,
    pub unit: String,
    pub optional: bool,
    pub reference: Option<(String, String)>,
    pub default: Option<String>,
    pub cold: bool,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct EnumSpec {
    pub name: String,
    pub flags: bool,
    pub variants: Vec<(String, i64)>,
}

#[derive(Default, Debug, Clone)]
pub struct Schema {
    pub sheets: BTreeMap<String, Vec<ColSpec>>,
    pub enums: BTreeMap<String, EnumSpec>,
}

impl Schema {
    pub fn cols(&self, sheet: &str) -> Option<&[ColSpec]> {
        self.sheets.get(sheet).map(|v| v.as_slice())
    }
}

pub fn load(book: &[Sheet], d: &mut Diagnostics) -> Schema {
    let mut s = Schema::default();
    let Some(sch) = book.iter().find(|x| x.name == "schema") else {
        d.push(Diag::new(Sev::E, Layer::L0, "E-L0-NOSCHEMA").msg("sheets/01-schema.tsv missing"));
        return s;
    };
    let need = ["sheet", "column", "type", "unit", "optional", "ref", "default", "cold"];
    for n in need {
        if sch.col(n).is_none() {
            d.push(Diag::new(Sev::E, Layer::L0, "E-L0-HEADER").at("schema", sch.header_line).msg(format!("schema missing column `{n}`")));
            return s;
        }
    }
    for r in &sch.rows {
        let g = |c: &str| sch.cell(r, c).unwrap_or("").to_string();
        let ty = match Ty::parse(&g("type")) {
            Some(t) => t,
            None => {
                d.push(Diag::new(Sev::E, Layer::L1, "E-L1-TYPE").at("schema", r.line).row(&g("sheet")).col(&g("column")).msg(format!("unknown type `{}`", g("type"))));
                continue;
            }
        };
        let reference = match g("ref").as_str() {
            "-" | "" | "NULL" => None,
            x => match x.split_once('.') {
                Some((a, b)) => Some((a.to_string(), b.to_string())),
                None => {
                    d.push(Diag::new(Sev::E, Layer::L1, "E-L1-REFSPEC").at("schema", r.line).msg(format!("ref `{x}` is not sheet.column")));
                    None
                }
            },
        };
        let bool01 = |c: &str, d: &mut Diagnostics| match g(c).as_str() {
            "0" => false,
            "1" => true,
            v => {
                d.push(Diag::new(Sev::E, Layer::L1, "E-L1-BOOL").at("schema", r.line).col(c).msg(format!("`{v}` is not 0/1")));
                false
            }
        };
        let spec = ColSpec {
            name: g("column"),
            unit: g("unit"),
            optional: bool01("optional", d),
            cold: bool01("cold", d),
            default: match g("default").as_str() {
                "-" | "" => None,
                v => Some(v.to_string()),
            },
            ty,
            reference,
            line: r.line,
        };
        if spec.unit.is_empty() {
            d.push(Diag::new(Sev::E, Layer::L1, "E-L1-UNIT").at("schema", r.line).col(&spec.name).msg("unit tag empty (use `-` for unitless)"));
        }
        s.sheets.entry(g("sheet")).or_default().push(spec);
    }
    if let Some(en) = book.iter().find(|x| x.name == "enums") {
        for r in &en.rows {
            let g = |c: &str| en.cell(r, c).unwrap_or("").to_string();
            let name = g("enum");
            let flags = g("kind") == "flags";
            let v: i64 = match g("value").parse() {
                Ok(v) => v,
                Err(_) => {
                    d.push(Diag::new(Sev::E, Layer::L1, "E-L1-TYPE").at("enums", r.line).msg(format!("value `{}` is not an integer", g("value"))));
                    continue;
                }
            };
            let e = s.enums.entry(name.clone()).or_insert(EnumSpec { name, flags, variants: vec![] });
            if e.variants.iter().any(|(n, _)| *n == g("variant")) {
                d.push(Diag::new(Sev::E, Layer::L0, "E-L0-DUPKEY").at("enums", r.line).row(&g("variant")).msg("duplicate enum variant"));
            }
            e.variants.push((g("variant"), v));
        }
    }
    s
}
