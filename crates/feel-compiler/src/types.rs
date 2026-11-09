//! Static types for the checker and the schema language.
//!
//! The lattice follows 10.3.2.9 (Figure 10-26): `Any` on top, `Null` at the
//! bottom. In addition the checker tracks *nullability* (`Nullable(T)` = T or
//! null) and context fields may be *optional* (the key may be absent) and/or
//! nullable (present with a null value) — the two are distinct.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
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
    List(Box<Ty>),
    Range(Box<Ty>),
    Context(Vec<Field>, bool),
    Function(Vec<Ty>, Box<Ty>),
    Nullable(Box<Ty>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    pub optional: bool,
}

impl Ty {
    pub fn nullable(self) -> Ty {
        match self {
            Ty::Any | Ty::Null | Ty::Nullable(_) => self,
            t => Ty::Nullable(Box::new(t)),
        }
    }
    pub fn non_null(&self) -> Ty {
        match self {
            Ty::Nullable(t) => (**t).clone(),
            t => t.clone(),
        }
    }
    pub fn is_nullable(&self) -> bool {
        matches!(self, Ty::Nullable(_) | Ty::Null)
    }
    pub fn is_any(&self) -> bool {
        matches!(self, Ty::Any)
    }
    /// Base (non-null) type is known and not Any.
    pub fn known(&self) -> Option<Ty> {
        match self.non_null() {
            Ty::Any | Ty::Null => None,
            t => Some(t),
        }
    }
    pub fn is_list(&self) -> bool {
        matches!(self.non_null(), Ty::List(_))
    }
    pub fn kind(&self) -> &'static str {
        match self.non_null() {
            Ty::Any => "Any",
            Ty::Null => "Null",
            Ty::Number => "number",
            Ty::String => "string",
            Ty::Boolean => "boolean",
            Ty::Date => "date",
            Ty::Time => "time",
            Ty::DateTime => "date and time",
            Ty::Dtd => "days and time duration",
            Ty::Ymd => "years and months duration",
            Ty::List(_) => "list",
            Ty::Range(_) => "range",
            Ty::Context(..) => "context",
            Ty::Function(..) => "function",
            Ty::Nullable(_) => unreachable!(),
        }
    }
    /// Least upper bound (join) of two types.
    pub fn join(&self, o: &Ty) -> Ty {
        if self == o {
            return self.clone();
        }
        match (self, o) {
            (Ty::Null, t) | (t, Ty::Null) => t.clone().nullable(),
            (Ty::Any, _) | (_, Ty::Any) => Ty::Any,
            (Ty::Nullable(a), b) | (b, Ty::Nullable(a)) => a.join(&b.non_null()).nullable(),
            (Ty::List(a), Ty::List(b)) => Ty::List(Box::new(a.join(b))),
            (Ty::Range(a), Ty::Range(b)) => Ty::Range(Box::new(a.join(b))),
            (Ty::Context(a, oa), Ty::Context(b, ob)) => {
                // fields common to both
                let mut fs = Vec::new();
                for f in a {
                    if let Some(g) = b.iter().find(|g| g.name == f.name) {
                        fs.push(Field {
                            name: f.name.clone(),
                            ty: f.ty.join(&g.ty),
                            optional: f.optional || g.optional,
                        });
                    } else {
                        fs.push(Field {
                            name: f.name.clone(),
                            ty: f.ty.clone(),
                            optional: true,
                        });
                    }
                }
                for g in b {
                    if !a.iter().any(|f| f.name == g.name) {
                        fs.push(Field {
                            name: g.name.clone(),
                            ty: g.ty.clone(),
                            optional: true,
                        });
                    }
                }
                Ty::Context(fs, *oa || *ob)
            }
            _ => Ty::Any,
        }
    }
    /// Conformance `self <: t` (10.3.2.9.2), with nullability.
    pub fn conforms(&self, t: &Ty) -> bool {
        match (self, t) {
            (_, Ty::Any) => true,
            (Ty::Null, Ty::Nullable(_)) => true,
            (Ty::Null, _) => false,
            (Ty::Any, _) => false,
            (Ty::Nullable(a), Ty::Nullable(b)) => a.conforms(b),
            (Ty::Nullable(_), _) => false,
            (a, Ty::Nullable(b)) => a.conforms(b),
            (Ty::List(a), Ty::List(b)) => a.conforms(b),
            (Ty::Range(a), Ty::Range(b)) => a.conforms(b),
            (Ty::Context(a, _), Ty::Context(b, _)) => b.iter().all(|g| {
                a.iter().any(|f| {
                    f.name == g.name && f.ty.conforms(&g.ty) && (!f.optional || g.optional)
                }) || g.optional
            }),
            (Ty::Function(pa, ra), Ty::Function(pb, rb)) => {
                pa.len() == pb.len()
                    && pa.iter().zip(pb).all(|(x, y)| y.conforms(x))
                    && ra.conforms(rb)
            }
            (a, b) => a == b,
        }
    }
    /// Could a value of this type possibly conform to `t` (no definite mismatch)?
    pub fn compatible(&self, t: &Ty) -> bool {
        match (self.non_null(), t.non_null()) {
            (Ty::Any, _) | (_, Ty::Any) | (Ty::Null, _) | (_, Ty::Null) => true,
            (Ty::List(a), Ty::List(b)) => a.compatible(&b),
            (Ty::Range(a), Ty::Range(b)) => a.compatible(&b),
            (Ty::Context(..), Ty::Context(..)) => true,
            (Ty::Function(..), Ty::Function(..)) => true,
            (a, b) => a == b,
        }
    }
    pub fn field(&self, name: &str) -> Option<&Field> {
        match self.non_null() {
            Ty::Context(fs, _) => {
                // need a reference into self; re-match on self
                let _ = fs;
                match self {
                    Ty::Context(fs, _) => fs.iter().find(|f| f.name == name),
                    Ty::Nullable(t) => t.field(name),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// FEEL type-expression string used for runtime validation (optional
    /// fields are omitted because absence cannot be expressed in rule 52).
    pub fn to_runtime_string(&self) -> String {
        match self {
            Ty::Nullable(t) => t.to_runtime_string(),
            Ty::Context(fs, _) => {
                let parts: Vec<String> = fs
                    .iter()
                    .filter(|f| !f.optional)
                    .map(|f| format!("{}: {}", quote_key(&f.name), f.ty.to_runtime_string()))
                    .collect();
                format!("context<{}>", parts.join(", "))
            }
            Ty::List(t) => format!("list<{}>", t.to_runtime_string()),
            Ty::Range(t) => format!("range<{}>", t.to_runtime_string()),
            Ty::Function(ps, r) => {
                let parts: Vec<String> = ps.iter().map(|p| p.to_runtime_string()).collect();
                format!("function<{}>->{}", parts.join(", "), r.to_runtime_string())
            }
            other => other.to_string(),
        }
    }
}

fn quote_key(k: &str) -> String {
    if k.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == ' ')
    {
        k.to_string()
    } else {
        format!("\"{k}\"")
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Any => write!(f, "Any"),
            Ty::Null => write!(f, "Null"),
            Ty::Number => write!(f, "number"),
            Ty::String => write!(f, "string"),
            Ty::Boolean => write!(f, "boolean"),
            Ty::Date => write!(f, "date"),
            Ty::Time => write!(f, "time"),
            Ty::DateTime => write!(f, "date and time"),
            Ty::Dtd => write!(f, "days and time duration"),
            Ty::Ymd => write!(f, "years and months duration"),
            Ty::List(t) => write!(f, "list<{t}>"),
            Ty::Range(t) => write!(f, "range<{t}>"),
            Ty::Context(fs, _) => {
                write!(f, "context<")?;
                for (i, x) in fs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(
                        f,
                        "{}{}: {}",
                        x.name,
                        if x.optional { "?" } else { "" },
                        x.ty
                    )?;
                }
                write!(f, ">")
            }
            Ty::Function(ps, r) => {
                write!(f, "function<")?;
                for (i, x) in ps.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{x}")?;
                }
                write!(f, ">->{r}")
            }
            Ty::Nullable(t) => write!(f, "{t}?"),
        }
    }
}

/// Parse a schema type: a FEEL type expression (rule 52) extended with
/// `?` suffixes for nullable types (`number?`) and optional context fields
/// (`context<a?: number>`), and named type aliases.
pub fn parse_schema_type(s: &str, aliases: &[(String, String)]) -> Result<Ty, String> {
    let mut p = P {
        s: s.as_bytes(),
        src: s,
        i: 0,
        aliases,
        depth: 0,
    };
    let t = p.ty()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("unexpected text in type '{s}' at {}", p.i));
    }
    Ok(t)
}

struct P<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    aliases: &'a [(String, String)],
    depth: u32,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, t: &str) -> bool {
        self.ws();
        if self.src[self.i..].starts_with(t) {
            self.i += t.len();
            true
        } else {
            false
        }
    }
    fn name(&mut self) -> String {
        self.ws();
        if self.i < self.s.len() && self.s[self.i] == b'"' {
            let start = self.i + 1;
            let mut j = start;
            while j < self.s.len() && self.s[j] != b'"' {
                j += 1;
            }
            self.i = (j + 1).min(self.s.len());
            return self.src[start..j].to_string();
        }
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b'<'
                || c == b'>'
                || c == b','
                || c == b':'
                || c == b'?'
                || (c == b'-' && self.s.get(self.i + 1) == Some(&b'>'))
            {
                break;
            }
            self.i += 1;
        }
        self.src[start..self.i]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn ty(&mut self) -> Result<Ty, String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err("type nesting too deep".into());
        }
        let n = self.name();
        let mut t = match n.as_str() {
            "list" if self.eat("<") => {
                let t = self.ty()?;
                if !self.eat(">") {
                    return Err("expected '>'".into());
                }
                Ty::List(Box::new(t))
            }
            "range" if self.eat("<") => {
                let t = self.ty()?;
                if !self.eat(">") {
                    return Err("expected '>'".into());
                }
                Ty::Range(Box::new(t))
            }
            "context" if self.eat("<") => {
                let mut fs = Vec::new();
                let mut open = false;
                if !self.eat(">") {
                    loop {
                        if self.eat("...") {
                            open = true;
                        } else {
                            let k = self.name();
                            let optional = self.eat("?");
                            if !self.eat(":") {
                                return Err(format!("expected ':' after field '{k}'"));
                            }
                            let t = self.ty()?;
                            fs.push(Field {
                                name: k,
                                ty: t,
                                optional,
                            });
                        }
                        if !self.eat(",") {
                            break;
                        }
                    }
                    if !self.eat(">") {
                        return Err("expected '>'".into());
                    }
                }
                Ty::Context(fs, open)
            }
            "function" if self.eat("<") => {
                let mut ps = Vec::new();
                if !self.eat(">") {
                    loop {
                        ps.push(self.ty()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    if !self.eat(">") {
                        return Err("expected '>'".into());
                    }
                }
                let r = if self.eat("->") { self.ty()? } else { Ty::Any };
                Ty::Function(ps, Box::new(r))
            }
            other => match named(other) {
                Some(t) => t,
                None => match self.aliases.iter().find(|(a, _)| a == other) {
                    Some((_, def)) => {
                        let def = def.clone();
                        let mut q = P {
                            s: def.as_bytes(),
                            src: &def,
                            i: 0,
                            aliases: self.aliases,
                            depth: self.depth,
                        };
                        q.ty()?
                    }
                    None => return Err(format!("unknown type '{other}'")),
                },
            },
        };
        if self.eat("?") {
            t = t.nullable();
        }
        self.depth -= 1;
        Ok(t)
    }
}

pub fn named(n: &str) -> Option<Ty> {
    Some(match n {
        "Any" | "any" | "dynamic" => Ty::Any,
        "Null" => Ty::Null,
        "number" => Ty::Number,
        "string" => Ty::String,
        "boolean" => Ty::Boolean,
        "date" => Ty::Date,
        "time" => Ty::Time,
        "date and time" | "dateTime" => Ty::DateTime,
        "days and time duration" | "dayTimeDuration" => Ty::Dtd,
        "years and months duration" | "yearMonthDuration" => Ty::Ymd,
        "list" => Ty::List(Box::new(Ty::Any)),
        "context" => Ty::Context(Vec::new(), true),
        "range" => Ty::Range(Box::new(Ty::Any)),
        "function" => Ty::Function(Vec::new(), Box::new(Ty::Any)),
        _ => return None,
    })
}

/// JSON schema accepted by `compile`/`check` (see docs/API.md).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Schema {
    /// Declared inputs: name → type (schema type syntax).
    pub inputs: Vec<InputSpec>,
    /// Named type aliases (e.g. DMN item definitions): name → type.
    pub types: Vec<(String, String)>,
    /// Host functions callable by name.
    pub functions: Vec<HostFunctionSpec>,
    /// Type of the unary-test candidate `?`.
    pub candidate: Option<String>,
    /// Expected result type.
    pub expected: Option<String>,
    /// If true, names not declared in the schema are errors; otherwise they
    /// become dynamically-typed inputs.
    pub closed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InputSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HostFunctionSpec {
    pub name: String,
    pub params: Vec<InputSpec>,
    pub returns: Option<String>,
}
