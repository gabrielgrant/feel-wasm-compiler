//! FEEL-JSON: the precision-preserving interchange format used for inputs,
//! outputs, constants and host callbacks.
//!
//! * `null`, `true`, `false`, strings: as in JSON.
//! * numbers: JSON number tokens, parsed from their *text* as FEEL decimals
//!   (never via binary floating point); `{"@number": "1.10"}` is also accepted.
//!   Output numbers are always written as plain decimal JSON number tokens,
//!   preserving scale (e.g. `1.10`).
//! * lists: JSON arrays; contexts: JSON objects (entry order preserved).
//! * tagged values: an object with exactly one key starting with `@`:
//!   `{"@date": "2020-01-01"}`, `{"@time": "10:00:00Z"}`,
//!   `{"@dateTime": "2020-01-01T10:00:00@Europe/Paris"}`,
//!   `{"@duration": "P1DT2H"}` / `{"@duration": "P1Y2M"}`,
//!   `{"@range": {"start": .., "end": .., "startIncluded": b, "endIncluded": b}}`
//!   (a missing `start`/`end` key means unbounded),
//!   `{"@function": "<description>"}` (output only),
//!   `{"@context": {...}}` (escape for contexts whose only key starts with `@`).

use crate::value::*;
use feel_decimal::Decimal;
use std::fmt::Write as _;
use std::rc::Rc;

pub struct Decoder<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonError {
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid FEEL-JSON at byte {}: {}",
            self.offset, self.message
        )
    }
}

const MAX_DEPTH: u32 = 256;

impl<'a> Decoder<'a> {
    pub fn new(s: &'a str) -> Self {
        Decoder {
            s: s.as_bytes(),
            i: 0,
            depth: 0,
        }
    }
    fn err<T>(&self, m: &str) -> Result<T, JsonError> {
        Err(JsonError {
            offset: self.i,
            message: m.to_string(),
        })
    }
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    pub fn decode_document(mut self, rt: &mut Runtime) -> Result<H, JsonError> {
        self.ws();
        let h = self.value(rt)?;
        self.ws();
        if self.i != self.s.len() {
            return self.err("trailing characters");
        }
        Ok(h)
    }

    fn value(&mut self, rt: &mut Runtime) -> Result<H, JsonError> {
        self.ws();
        if rt.aborted() {
            return self.err("resource limit exceeded while decoding");
        }
        match self.s.get(self.i) {
            None => self.err("unexpected end of input"),
            Some(b'n') => self.lit(b"null", NULL),
            Some(b't') => self.lit(b"true", TRUE),
            Some(b'f') => self.lit(b"false", FALSE),
            Some(b'"') => {
                let s = self.string()?;
                Ok(rt.string(s))
            }
            Some(b'[') => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return self.err("nesting too deep");
                }
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                } else {
                    loop {
                        items.push(self.value(rt)?);
                        self.ws();
                        match self.s.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return self.err("expected ',' or ']'"),
                        }
                    }
                }
                self.depth -= 1;
                Ok(rt.list(items))
            }
            Some(b'{') => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return self.err("nesting too deep");
                }
                self.i += 1;
                let mut keys: Vec<String> = Vec::new();
                let mut vals: Vec<H> = Vec::new();
                let mut raw_starts: Vec<usize> = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        if self.s.get(self.i) != Some(&b'"') {
                            return self.err("expected object key");
                        }
                        let k = self.string()?;
                        self.ws();
                        if self.s.get(self.i) != Some(&b':') {
                            return self.err("expected ':'");
                        }
                        self.i += 1;
                        self.ws();
                        raw_starts.push(self.i);
                        // the payload of an `@context` escape is a plain context
                        // whose (single) key may itself start with '@'
                        let v = if k == "@context"
                            && keys.is_empty()
                            && self.s.get(self.i) == Some(&b'{')
                        {
                            self.raw_object(rt)?
                        } else {
                            self.value(rt)?
                        };
                        keys.push(k);
                        vals.push(v);
                        self.ws();
                        match self.s.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return self.err("expected ',' or '}'"),
                        }
                    }
                }
                self.depth -= 1;
                if keys.len() == 1 && keys[0].starts_with('@') {
                    return self.tagged(rt, &keys[0], vals[0]);
                }
                let mut c = Ctx::default();
                for (k, v) in keys.into_iter().zip(vals) {
                    if c.get(&k).is_some() {
                        return self.err("duplicate key in object");
                    }
                    c.put(Rc::from(k.as_str()), v);
                }
                Ok(rt.ctx(c))
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let start = self.i;
                if self.s[self.i] == b'-' {
                    self.i += 1;
                }
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_digit()
                        || matches!(self.s[self.i], b'.' | b'e' | b'E' | b'+' | b'-'))
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                match Decimal::parse(text) {
                    Some(d) => Ok(rt.num(d)),
                    None => Err(JsonError {
                        offset: start,
                        message: format!("number {text} is not representable as a FEEL number"),
                    }),
                }
            }
            _ => self.err("unexpected character"),
        }
    }

    /// An object decoded as a context without tag interpretation.
    fn raw_object(&mut self, rt: &mut Runtime) -> Result<H, JsonError> {
        self.i += 1;
        let mut c = Ctx::default();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(rt.ctx(c));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return self.err("expected object key");
            }
            let k = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return self.err("expected ':'");
            }
            self.i += 1;
            let v = self.value(rt)?;
            if c.get(&k).is_some() {
                return self.err("duplicate key in object");
            }
            c.put(Rc::from(k.as_str()), v);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(rt.ctx(c));
                }
                _ => return self.err("expected ',' or '}'"),
            }
        }
    }

    fn tagged(&self, rt: &mut Runtime, tag: &str, v: H) -> Result<H, JsonError> {
        let sval = rt.as_str(v);
        let bad = |what: &str| JsonError {
            offset: self.i,
            message: format!("invalid {what} payload for {tag}"),
        };
        match tag {
            "@number" => {
                let s = sval.ok_or_else(|| bad("string"))?;
                let d = Decimal::parse(s.trim()).ok_or_else(|| bad("number"))?;
                Ok(rt.num(d))
            }
            "@date" => {
                let s = sval.ok_or_else(|| bad("string"))?;
                let d = feel_temporal::Date::parse(&s).ok_or_else(|| bad("date"))?;
                Ok(rt.alloc(Value::Date(d)))
            }
            "@time" => {
                let s = sval.ok_or_else(|| bad("string"))?;
                let t = feel_temporal::Time::parse(&s).ok_or_else(|| bad("time"))?;
                Ok(rt.alloc(Value::Time(t)))
            }
            "@dateTime" | "@date and time" => {
                let s = sval.ok_or_else(|| bad("string"))?;
                let t = feel_temporal::DateTime::parse(&s).ok_or_else(|| bad("date and time"))?;
                Ok(rt.alloc(Value::DateTime(t)))
            }
            "@duration" => {
                let s = sval.ok_or_else(|| bad("string"))?;
                match feel_temporal::parse_duration(&s) {
                    Some(feel_temporal::Duration::DayTime(d)) => Ok(rt.alloc(Value::Dtd(d))),
                    Some(feel_temporal::Duration::YearMonth(d)) => Ok(rt.alloc(Value::Ymd(d))),
                    None => Err(bad("duration")),
                }
            }
            "@range" => {
                let c = rt.as_ctx(v).cloned().ok_or_else(|| bad("object"))?;
                let start = c.get("start");
                let end = c.get("end");
                let si = c
                    .get("startIncluded")
                    .and_then(|h| rt.as_bool(h))
                    .unwrap_or(start.is_some());
                let ei = c
                    .get("endIncluded")
                    .and_then(|h| rt.as_bool(h))
                    .unwrap_or(end.is_some());
                let ne = c
                    .get("notEqual")
                    .and_then(|h| rt.as_bool(h))
                    .unwrap_or(false);
                let eq = c.get("equal").and_then(|h| rt.as_bool(h)).unwrap_or(false);
                Ok(rt.alloc(Value::Range(Range {
                    start,
                    end,
                    start_incl: si,
                    end_incl: ei,
                    ne,
                    eq,
                })))
            }
            // function values cannot cross the boundary; their placeholder
            // decodes to null
            "@function" => Ok(NULL),
            "@context" => {
                if rt.as_ctx(v).is_some() {
                    Ok(v)
                } else {
                    Err(bad("object"))
                }
            }
            _ => Err(JsonError {
                offset: self.i,
                message: format!("unknown tag {tag}"),
            }),
        }
    }

    fn lit(&mut self, word: &[u8], h: H) -> Result<H, JsonError> {
        if self.s[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(h)
        } else {
            self.err("invalid literal")
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' {
                self.i += 1;
            }
            out.push_str(
                std::str::from_utf8(&self.s[start..self.i]).map_err(|_| JsonError {
                    offset: start,
                    message: "invalid UTF-8".into(),
                })?,
            );
            match self.s.get(self.i) {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => {
                    self.i += 1;
                    let c = match self.s.get(self.i) {
                        Some(c) => *c,
                        None => return self.err("bad escape"),
                    };
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                if self.s.get(self.i) == Some(&b'\\')
                                    && self.s.get(self.i + 1) == Some(&b'u')
                                {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        let full = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                        out.push(char::from_u32(full).unwrap_or('\u{FFFD}'));
                                    } else {
                                        out.push('\u{FFFD}');
                                        out.push(char::from_u32(lo).unwrap_or('\u{FFFD}'));
                                    }
                                } else {
                                    out.push('\u{FFFD}');
                                }
                            } else {
                                out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                            }
                        }
                        _ => return self.err("bad escape"),
                    }
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| JsonError {
            offset: self.i,
            message: "bad \\u".into(),
        })?;
        let s = std::str::from_utf8(h).map_err(|_| JsonError {
            offset: self.i,
            message: "bad \\u".into(),
        })?;
        let v = u32::from_str_radix(s, 16).map_err(|_| JsonError {
            offset: self.i,
            message: "bad \\u".into(),
        })?;
        self.i += 4;
        Ok(v)
    }
}

pub fn decode(rt: &mut Runtime, s: &str) -> Result<H, JsonError> {
    Decoder::new(s).decode_document(rt)
}

pub fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn encode(rt: &Runtime, h: H) -> String {
    let mut out = String::new();
    encode_into(rt, h, &mut out, 0);
    out
}

pub fn encode_into(rt: &Runtime, h: H, out: &mut String, depth: u32) {
    if depth > 512 {
        out.push_str("null");
        return;
    }
    match rt.get(h) {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(d) => {
            let _ = write!(out, "{d}");
        }
        Value::Str(s) => write_json_string(out, s),
        Value::Date(d) => {
            out.push_str("{\"@date\":");
            write_json_string(out, &d.to_string());
            out.push('}');
        }
        Value::Time(t) => {
            out.push_str("{\"@time\":");
            write_json_string(out, &t.to_string());
            out.push('}');
        }
        Value::DateTime(t) => {
            out.push_str("{\"@dateTime\":");
            write_json_string(out, &t.to_string());
            out.push('}');
        }
        Value::Dtd(d) => {
            out.push_str("{\"@duration\":");
            write_json_string(out, &d.to_string());
            out.push('}');
        }
        Value::Ymd(d) => {
            out.push_str("{\"@duration\":");
            write_json_string(out, &d.to_string());
            out.push('}');
        }
        Value::List(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode_into(rt, *x, out, depth + 1);
            }
            out.push(']');
        }
        Value::Ctx(c) => {
            let escape = c.len() == 1 && c.keys[0].starts_with('@');
            if escape {
                out.push_str("{\"@context\":");
            }
            out.push('{');
            for (i, (k, v)) in c.keys.iter().zip(c.vals.iter()).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(out, k);
                out.push(':');
                encode_into(rt, *v, out, depth + 1);
            }
            out.push('}');
            if escape {
                out.push('}');
            }
        }
        Value::Range(r) => {
            out.push_str("{\"@range\":{");
            let mut first = true;
            if let Some(s) = r.start {
                out.push_str("\"start\":");
                encode_into(rt, s, out, depth + 1);
                first = false;
            }
            if let Some(e) = r.end {
                if !first {
                    out.push(',');
                }
                out.push_str("\"end\":");
                encode_into(rt, e, out, depth + 1);
                first = false;
            }
            if !first {
                out.push(',');
            }
            let _ = write!(
                out,
                "\"startIncluded\":{},\"endIncluded\":{}",
                r.start_incl, r.end_incl
            );
            if r.ne {
                out.push_str(",\"notEqual\":true");
            }
            if r.eq {
                out.push_str(",\"equal\":true");
            }
            out.push_str("}}");
        }
        Value::Func(f) => {
            out.push_str("{\"@function\":");
            let desc = match f {
                Func::Builtin(id) => format!("builtin {}", crate::builtins::name_of(*id)),
                Func::User { index, .. } => format!("user function #{index}"),
                Func::External { mapping, .. } => {
                    format!("external {}", crate::builtins::external_key(rt, *mapping))
                }
                Func::Host { name, .. } => format!("host function {name}"),
            };
            write_json_string(out, &desc);
            out.push('}');
        }
    }
}
