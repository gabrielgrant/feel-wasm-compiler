//! Runtime type descriptors (10.3.2.9): `instance of`, conformance and the
//! implicit conversions used at invocation and binding boundaries.
//!
//! Type descriptors are carried as FEEL strings in the constant pool using the
//! rule 52 syntax (e.g. `list<number>`, `context<a: string>`,
//! `function<number>->boolean`). Item-definition types from DMN are lowered by
//! the DMN adapter into these structural forms (plus allowed-value tests).

use crate::value::*;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum RtType {
    Any,
    Null,
    Number,
    String,
    Boolean,
    Date,
    Time,
    DateTime,
    Dtd,
    Ymd,
    /// generic `duration` (either kind)
    Duration,
    List(Box<RtType>),
    Range(Box<RtType>),
    Context(Vec<(String, RtType)>),
    Function(Vec<RtType>, Box<RtType>),
    /// an unknown type name: `instance of` yields null
    Unknown(String),
}

pub fn parse_type(s: &str) -> RtType {
    let mut p = TP { s: s.trim(), i: 0 };
    let t = p.ty();
    p.ws();
    if p.i != p.s.len() {
        return RtType::Unknown(s.to_string());
    }
    t
}

struct TP<'a> {
    s: &'a str,
    i: usize,
}

impl TP<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s.as_bytes()[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, t: &str) -> bool {
        self.ws();
        if self.s[self.i..].starts_with(t) {
            self.i += t.len();
            true
        } else {
            false
        }
    }
    fn name(&mut self) -> String {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i..].chars().next().unwrap();
            if c == '<'
                || c == '>'
                || c == ','
                || c == ':'
                || c == '-' && self.s[self.i..].starts_with("->")
            {
                break;
            }
            self.i += c.len_utf8();
        }
        self.s[start..self.i]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn ty(&mut self) -> RtType {
        let n = self.name();
        match n.as_str() {
            "list" if self.eat("<") => {
                let t = self.ty();
                self.eat(">");
                RtType::List(Box::new(t))
            }
            "range" if self.eat("<") => {
                let t = self.ty();
                self.eat(">");
                RtType::Range(Box::new(t))
            }
            "context" if self.eat("<") => {
                let mut es = Vec::new();
                if !self.eat(">") {
                    loop {
                        let k = self.name();
                        let k = k.trim_matches('"').to_string();
                        self.eat(":");
                        let t = self.ty();
                        es.push((k, t));
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.eat(">");
                }
                RtType::Context(es)
            }
            "function" if self.eat("<") => {
                let mut ps = Vec::new();
                if !self.eat(">") {
                    loop {
                        ps.push(self.ty());
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.eat(">");
                }
                let r = if self.eat("->") {
                    self.ty()
                } else {
                    RtType::Any
                };
                RtType::Function(ps, Box::new(r))
            }
            _ => named_type(&n),
        }
    }
}

pub fn named_type(n: &str) -> RtType {
    match n {
        "Any" | "any" => RtType::Any,
        "Null" => RtType::Null,
        "number" | "feel:number" | "decimal" | "integer" | "double" | "long" | "int" => {
            RtType::Number
        }
        "string" | "feel:string" => RtType::String,
        "boolean" | "feel:boolean" => RtType::Boolean,
        "date" | "feel:date" => RtType::Date,
        "time" | "feel:time" => RtType::Time,
        "date and time" | "dateTime" | "feel:date and time" | "feel:dateTime" => RtType::DateTime,
        "days and time duration"
        | "dayTimeDuration"
        | "feel:days and time duration"
        | "feel:dayTimeDuration" => RtType::Dtd,
        "years and months duration"
        | "yearMonthDuration"
        | "feel:years and months duration"
        | "feel:yearMonthDuration" => RtType::Ymd,
        "duration" => RtType::Duration,
        "list" => RtType::List(Box::new(RtType::Any)),
        "context" => RtType::Context(Vec::new()),
        "range" => RtType::Range(Box::new(RtType::Any)),
        "function" => RtType::Function(Vec::new(), Box::new(RtType::Any)),
        other => RtType::Unknown(other.to_string()),
    }
}

impl std::fmt::Display for RtType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtType::Any => write!(f, "Any"),
            RtType::Null => write!(f, "Null"),
            RtType::Number => write!(f, "number"),
            RtType::String => write!(f, "string"),
            RtType::Boolean => write!(f, "boolean"),
            RtType::Date => write!(f, "date"),
            RtType::Time => write!(f, "time"),
            RtType::DateTime => write!(f, "date and time"),
            RtType::Dtd => write!(f, "days and time duration"),
            RtType::Ymd => write!(f, "years and months duration"),
            RtType::Duration => write!(f, "duration"),
            RtType::List(t) => write!(f, "list<{t}>"),
            RtType::Range(t) => write!(f, "range<{t}>"),
            RtType::Context(es) => {
                write!(f, "context<")?;
                for (i, (k, t)) in es.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {t}")?;
                }
                write!(f, ">")
            }
            RtType::Function(ps, r) => {
                write!(f, "function<")?;
                for (i, t) in ps.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ">->{r}")
            }
            RtType::Unknown(n) => write!(f, "{n}"),
        }
    }
}

impl Runtime {
    /// Parse (and cache) the type descriptor held by a string constant.
    pub fn type_of_const(&mut self, h: H) -> Rc<RtType> {
        if let Some((_, t)) = self.type_cache.iter().find(|(k, _)| *k == h) {
            return t.clone();
        }
        let t = Rc::new(match self.as_str(h) {
            Some(s) => parse_type(&s),
            None => RtType::Any,
        });
        self.type_cache.push((h, t.clone()));
        t
    }
}

/// The (most specific) type of a value, used for function value conformance.
pub fn value_conforms(rt: &Runtime, h: H, t: &RtType) -> Option<bool> {
    use RtType as T;
    let v = rt.get(h);
    Some(match (t, v) {
        (T::Unknown(_), _) => return None,
        (T::Any, _) => true,
        (_, Value::Null) => true,
        (T::Null, _) => false,
        (T::Number, Value::Num(_)) => true,
        (T::String, Value::Str(_)) => true,
        (T::Boolean, Value::Bool(_)) => true,
        (T::Date, Value::Date(_)) => true,
        (T::Time, Value::Time(_)) => true,
        (T::DateTime, Value::DateTime(_)) => true,
        (T::Dtd, Value::Dtd(_)) => true,
        (T::Ymd, Value::Ymd(_)) => true,
        (T::Duration, Value::Dtd(_) | Value::Ymd(_)) => true,
        (T::List(et), Value::List(items)) => {
            let items = items.clone();
            for it in items {
                if rt.is_null(it) {
                    continue;
                }
                match value_conforms(rt, it, et) {
                    Some(true) => {}
                    other => return other,
                }
            }
            true
        }
        (T::Range(et), Value::Range(r)) => {
            let r = r.clone();
            for p in [r.start, r.end].into_iter().flatten() {
                match value_conforms(rt, p, et) {
                    Some(true) => {}
                    other => return other,
                }
            }
            true
        }
        (T::Context(es), Value::Ctx(c)) => {
            for (k, et) in es {
                match c.get(k) {
                    None => return Some(false),
                    Some(x) => match value_conforms(rt, x, et) {
                        Some(true) => {}
                        other => return other,
                    },
                }
            }
            true
        }
        (T::Function(ps, ret), Value::Func(f)) => {
            if ps.is_empty() && **ret == T::Any {
                return Some(true);
            }
            match f {
                Func::User { sig, .. } | Func::External { sig, .. } | Func::Host { sig, .. } => {
                    let sig_items = rt.list_items(*sig).unwrap_or_default();
                    if sig_items.len() != ps.len() {
                        return Some(false);
                    }
                    // contravariant parameters: declared parameter types of the
                    // value must accept the requested parameter types
                    for (i, p) in ps.iter().enumerate() {
                        let pair = rt.list_items(sig_items[i]).unwrap_or_default();
                        let declared = pair
                            .get(1)
                            .and_then(|h| rt.as_str(*h))
                            .map(|s| parse_type(&s))
                            .unwrap_or(T::Any);
                        if !type_conforms(p, &declared) {
                            return Some(false);
                        }
                    }
                    true
                }
                Func::Builtin(_) => true,
            }
        }
        _ => false,
    })
}

/// Static type conformance `s <: t` (10.3.2.9.2).
pub fn type_conforms(s: &RtType, t: &RtType) -> bool {
    use RtType as T;
    match (s, t) {
        (_, T::Any) => true,
        (T::Null, _) => true,
        (T::Dtd | T::Ymd, T::Duration) => true,
        (T::List(a), T::List(b)) => type_conforms(a, b),
        (T::Range(a), T::Range(b)) => type_conforms(a, b),
        (T::Context(a), T::Context(b)) => b
            .iter()
            .all(|(k, tb)| a.iter().any(|(ka, ta)| ka == k && type_conforms(ta, tb))),
        (T::Function(pa, ra), T::Function(pb, rb)) => {
            pa.len() == pb.len()
                && pa.iter().zip(pb.iter()).all(|(x, y)| type_conforms(y, x))
                && type_conforms(ra, rb)
        }
        (a, b) => a == b,
    }
}

/// `e instance of T` (Table 61): null for unknown types; `null instance of T`
/// is true only for `Null` (and `Any`? no: Any excludes the special case per
/// TCK — null is an instance of Null only).
pub fn instance_of(rt: &Runtime, h: H, t: &RtType) -> Option<bool> {
    if let RtType::Unknown(_) = t {
        return None;
    }
    if rt.is_null(h) {
        return Some(matches!(t, RtType::Null));
    }
    value_conforms(rt, h, t)
}

/// Implicit conversions towards a target type (10.3.2.9.4): from singleton
/// list, to singleton list, date to date and time. Returns the converted value
/// and whether it conforms afterwards.
pub fn convert_to(rt: &mut Runtime, h: H, t: &RtType) -> (H, bool) {
    if let Some(true) = value_conforms(rt, h, t) {
        return (h, true);
    }
    // from singleton list
    if let Value::List(items) = rt.get(h) {
        if items.len() == 1 && !matches!(t, RtType::List(_)) {
            let x = items[0];
            if let Some(true) = value_conforms(rt, x, t) {
                return (x, true);
            }
        }
    }
    // to singleton list
    if let RtType::List(et) = t {
        if !matches!(rt.get(h), Value::List(_)) {
            if let Some(true) = value_conforms(rt, h, et) {
                let l = rt.list(vec![h]);
                return (l, true);
            }
        }
    }
    // date to date and time
    if matches!(t, RtType::DateTime) {
        if let Value::Date(d) = rt.get(h) {
            let dt = crate::ops::date_to_datetime(d);
            let x = rt.alloc(Value::DateTime(dt));
            return (x, true);
        }
    }
    (h, false)
}

/// Binding conversion (DMN typeRef): implicit conversions, then the
/// "conforms to" conversion (non-conforming → null).
pub fn coerce(rt: &mut Runtime, h: H, t: &RtType, site: u32) -> H {
    if let RtType::Unknown(_) = t {
        return h;
    }
    let (x, ok) = convert_to(rt, h, t);
    if ok {
        x
    } else {
        let k = rt.kind_name(h);
        rt.diag(
            "FEEL-E330",
            site,
            format!("value of type {k} does not conform to {t}; bound to null"),
        );
        NULL
    }
}
