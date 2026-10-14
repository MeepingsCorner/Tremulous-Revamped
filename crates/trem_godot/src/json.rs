//! A small JSON reader for the release list (dec065): enough of RFC 8259
//! to read GitHub's releases answer and the static `releases.json`, so the
//! client needs no JSON crate. Only reads; never builds JSON. Nesting is
//! limited to `MAX_DEPTH`, so a hostile answer cannot overflow the stack.

const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        if let Value::Arr(a) = self {
            Some(a)
        } else {
            None
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
    /// The member `k` of an object (the last one if repeated, as most parsers do).
    pub fn get(&self, k: &str) -> Option<&Value> {
        if let Value::Obj(m) = self {
            m.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v)
        } else {
            None
        }
    }
}

/// Parse a whole document; `None` on any syntax error or trailing garbage.
pub fn parse(body: &[u8]) -> Option<Value> {
    let s = std::str::from_utf8(body).ok()?;
    let mut p = P {
        b: s.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    (p.i == p.b.len()).then_some(v)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Option<()> {
        (self.b.get(self.i) == Some(&c)).then(|| self.i += 1)
    }
    fn lit(&mut self, w: &str) -> Option<()> {
        self.b[self.i..]
            .starts_with(w.as_bytes())
            .then(|| self.i += w.len())
    }
    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        match *self.b.get(self.i)? {
            b'n' => self.lit("null").map(|_| Value::Null),
            b't' => self.lit("true").map(|_| Value::Bool(true)),
            b'f' => self.lit("false").map(|_| Value::Bool(false)),
            b'"' => self.string().map(Value::Str),
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.eat(b']').is_some() {
                    return Some(Value::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b']')?;
                    return Some(Value::Arr(a));
                }
            }
            b'{' => {
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.eat(b'}').is_some() {
                    return Some(Value::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    self.eat(b':')?;
                    self.ws();
                    m.push((k, self.value(depth + 1)?));
                    self.ws();
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b'}')?;
                    return Some(Value::Obj(m));
                }
            }
            b'-' | b'0'..=b'9' => {
                let st = self.i;
                while matches!(
                    self.b.get(self.i),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[st..self.i])
                    .ok()?
                    .parse::<f64>()
                    .ok()
                    .map(Value::Num)
            }
            _ => None,
        }
    }
    fn hex4(&mut self) -> Option<u32> {
        let h = self.b.get(self.i..self.i + 4)?;
        let v = u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
        self.i += 4;
        Some(v)
    }
    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                0..=0x1f => return None,
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut u = self.hex4()?;
                            if (0xd800..0xdc00).contains(&u) {
                                // a surrogate pair; a lone surrogate becomes U+FFFD
                                if self.b[self.i..].starts_with(b"\\u") {
                                    let save = self.i;
                                    self.i += 2;
                                    match self.hex4() {
                                        Some(lo) if (0xdc00..0xe000).contains(&lo) => {
                                            u = 0x10000 + ((u - 0xd800) << 10) + (lo - 0xdc00)
                                        }
                                        _ => {
                                            self.i = save;
                                            u = 0xfffd;
                                        }
                                    }
                                } else {
                                    u = 0xfffd;
                                }
                            }
                            char::from_u32(u).unwrap_or('\u{fffd}')
                        }
                        _ => return None,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_github_sends() {
        let v = parse(br#" [ {"tag_name":"v1.0","draft":false,"n":-1.5e2,"author":{"login":"a"},"body":"x\ny \"q\" \u00e9 \ud83d\ude00","z":null} ] "#).unwrap();
        let r = &v.as_array().unwrap()[0];
        assert_eq!(r.get("tag_name").and_then(|x| x.as_str()), Some("v1.0"));
        assert_eq!(r.get("draft").and_then(|x| x.as_bool()), Some(false));
        assert_eq!(
            r.get("author")
                .and_then(|a| a.get("login"))
                .and_then(|x| x.as_str()),
            Some("a")
        );
        assert_eq!(
            r.get("body").and_then(|x| x.as_str()),
            Some("x\ny \"q\" \u{e9} \u{1f600}")
        );
        assert_eq!(r.get("n"), Some(&Value::Num(-150.0)));
        assert_eq!(r.get("z"), Some(&Value::Null));
        assert_eq!(r.get("missing"), None);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "[",
            "[1,]",
            "{\"a\"}",
            "{\"a\":1,}",
            "[1] x",
            "tru",
            "\"a",
            "\"\u{1}\"",
            "[\"\\x\"]",
            "nul",
            "{1:2}",
            "'a'",
        ] {
            assert!(parse(bad.as_bytes()).is_none(), "{bad:?}");
        }
        assert!(parse(&[b'[', 0xff, b']']).is_none());
    }

    #[test]
    fn a_lone_surrogate_is_replaced_and_deep_nesting_refused() {
        assert_eq!(parse(br#""\ud800""#), Some(Value::Str("\u{fffd}".into())));
        assert_eq!(
            parse(br#""\ud800\u0041""#),
            Some(Value::Str("\u{fffd}A".into()))
        );
        let deep = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse(deep.as_bytes()).is_none());
        let ok = format!("{}{}", "[".repeat(30), "]".repeat(30));
        assert!(parse(ok.as_bytes()).is_some());
    }

    #[test]
    fn the_last_repeated_member_wins() {
        assert_eq!(
            parse(br#"{"a":1,"a":2}"#).unwrap().get("a"),
            Some(&Value::Num(2.0))
        );
    }
}
