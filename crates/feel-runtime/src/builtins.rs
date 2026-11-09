//! The FEEL standard builtin catalog (DMN 1.5 clause 10.3.4) and its
//! implementation.
//!
//! The catalog is a table of `(name, signature-spec, implementation)`. The
//! signature spec is shared with the compiler's checker and is also used to
//! map named arguments onto positions (10.3.2.13.5). Overloads are separated
//! by `|`; `?` marks an optional parameter and `...` a variadic one.

use crate::ops::{self, tri_of, T_FALSE, T_NULL, T_TRUE};
use crate::types::{self, RtType};
use crate::value::*;
use feel_decimal::{Decimal, RoundingMode};
use feel_temporal as tm;
use std::cmp::Ordering;
use std::rc::Rc;

/// Callbacks the runtime needs from its host while executing builtins.
pub trait Cx {
    fn rt(&mut self) -> &mut Runtime;
    /// Invoke a user function defined in the generated module with an
    /// already-bound positional argument list.
    fn apply(&mut self, f: H, args: H) -> H;
    /// Invoke an external/host function with a positional argument list.
    fn call_host(&mut self, f: H, args: H) -> H;
}

/// A trivial context for evaluation without callbacks (constant folding,
/// tests): user/external functions yield null.
pub struct NoCallbacks<'a>(pub &'a mut Runtime);
impl Cx for NoCallbacks<'_> {
    fn rt(&mut self) -> &mut Runtime {
        self.0
    }
    fn apply(&mut self, _f: H, _args: H) -> H {
        self.0.diag("FEEL-E321", 0, "user function call not available in this context");
        NULL
    }
    fn call_host(&mut self, _f: H, _args: H) -> H {
        self.0.diag("FEEL-E321", 0, "host function call not available in this context");
        NULL
    }
}

type Impl = fn(&mut dyn Cx, &[H], u32) -> H;

pub struct Builtin {
    pub name: &'static str,
    pub spec: &'static str,
    pub imp: Impl,
    /// Non-deterministic (clock) builtins are never constant-folded.
    pub deterministic: bool,
    /// Extension builtins are only visible when the extension is enabled.
    pub extension: u32,
}

macro_rules! b {
    ($n:expr, $s:expr, $f:expr) => {
        Builtin { name: $n, spec: $s, imp: $f, deterministic: true, extension: 0 }
    };
}

pub static BUILTINS: &[Builtin] = &[
    // 10.3.4.1 conversion
    b!("date", "from: Any -> date | year: number, month: number, day: number -> date", bi_date),
    b!("date and time", "from: Any -> date and time | date: Any, time: time -> date and time", bi_date_and_time),
    b!("time", "from: Any -> time | hour: number, minute: number, second: number, offset?: Any -> time", bi_time),
    b!("number", "from: string, grouping separator?: string, decimal separator?: string -> number", bi_number),
    b!("string", "from: Any -> string", bi_string),
    b!("duration", "from: string -> duration", bi_duration),
    b!("years and months duration", "from: Any, to: Any -> years and months duration", bi_ymd),
    b!("range", "from: string -> range<Any>", bi_range),
    // 10.3.4.2 boolean
    b!("not", "negand: Any -> boolean", bi_not),
    // 10.3.4.3 string
    b!("substring", "string: string, start position: number, length?: number -> string", bi_substring),
    b!("string length", "string: string -> number", bi_string_length),
    b!("upper case", "string: string -> string", bi_upper),
    b!("lower case", "string: string -> string", bi_lower),
    b!("substring before", "string: string, match: string -> string", bi_substring_before),
    b!("substring after", "string: string, match: string -> string", bi_substring_after),
    b!("replace", "input: string, pattern: string, replacement: string, flags?: string -> string", bi_replace),
    b!("contains", "string: string, match: string -> boolean", bi_contains),
    b!("starts with", "string: string, match: string -> boolean", bi_starts_with),
    b!("ends with", "string: string, match: string -> boolean", bi_ends_with),
    b!("matches", "input: string, pattern: string, flags?: string -> boolean", bi_matches),
    b!("split", "string: string, delimiter: string -> list<string>", bi_split),
    b!("string join", "list: list<Any>, delimiter?: Any -> string", bi_string_join),
    // 10.3.4.4 list
    b!("list contains", "list: list<Any>, element: Any -> boolean", bi_list_contains),
    b!("count", "list: list<Any> -> number", bi_count),
    b!("min", "list: list<Any> -> Any | c: Any... -> Any", bi_min),
    b!("max", "list: list<Any> -> Any | c: Any... -> Any", bi_max),
    b!("sum", "list: list<number> -> number | n: number... -> number", bi_sum),
    b!("mean", "list: list<number> -> number | n: number... -> number", bi_mean),
    b!("all", "list: list<boolean> -> boolean | b: boolean... -> boolean", bi_all),
    b!("any", "list: list<boolean> -> boolean | b: boolean... -> boolean", bi_any),
    b!("sublist", "list: list<Any>, start position: number, length?: number -> list<Any>", bi_sublist),
    b!("append", "list: list<Any>, item: Any... -> list<Any>", bi_append),
    b!("concatenate", "list: list<Any>... -> list<Any>", bi_concatenate),
    b!("insert before", "list: list<Any>, position: number, newItem: Any -> list<Any>", bi_insert_before),
    b!("remove", "list: list<Any>, position: number -> list<Any>", bi_remove),
    b!(
        "list replace",
        "list: list<Any>, position: number, newItem: Any -> list<Any> | list: list<Any>, match: function<Any, Any>->boolean, newItem: Any -> list<Any>",
        bi_list_replace
    ),
    b!("reverse", "list: list<Any> -> list<Any>", bi_reverse),
    b!("index of", "list: list<Any>, match: Any -> list<number>", bi_index_of),
    b!("union", "list: list<Any>... -> list<Any>", bi_union),
    b!("distinct values", "list: list<Any> -> list<Any>", bi_distinct_values),
    b!("flatten", "list: list<Any> -> list<Any>", bi_flatten),
    b!("product", "list: list<number> -> number | n: number... -> number", bi_product),
    b!("median", "list: list<number> -> number | n: number... -> number", bi_median),
    b!("stddev", "list: list<number> -> number | n: number... -> number", bi_stddev),
    b!("mode", "list: list<number> -> list<number> | n: number... -> list<number>", bi_mode),
    // 10.3.4.5 numeric
    b!("decimal", "n: number, scale: number -> number", bi_decimal),
    b!("floor", "n: number, scale?: number -> number", bi_floor),
    b!("ceiling", "n: number, scale?: number -> number", bi_ceiling),
    b!("round up", "n: number, scale: number -> number", bi_round_up),
    b!("round down", "n: number, scale: number -> number", bi_round_down),
    b!("round half up", "n: number, scale: number -> number", bi_round_half_up),
    b!("round half down", "n: number, scale: number -> number", bi_round_half_down),
    b!("abs", "n: Any -> Any", bi_abs),
    b!("modulo", "dividend: number, divisor: number -> number", bi_modulo),
    b!("sqrt", "number: number -> number", bi_sqrt),
    b!("log", "number: number -> number", bi_log),
    b!("exp", "number: number -> number", bi_exp),
    b!("odd", "number: number -> boolean", bi_odd),
    b!("even", "number: number -> boolean", bi_even),
    // 10.3.4.6 date and time
    b!("is", "value1: Any, value2: Any -> boolean", bi_is),
    // 10.3.4.7 range functions
    b!(
        "before",
        "point1: Any, point2: Any -> boolean | point: Any, range: range<Any> -> boolean | range: range<Any>, point: Any -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_before
    ),
    b!(
        "after",
        "point1: Any, point2: Any -> boolean | point: Any, range: range<Any> -> boolean | range: range<Any>, point: Any -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_after
    ),
    b!("meets", "range1: range<Any>, range2: range<Any> -> boolean", bi_meets),
    b!("met by", "range1: range<Any>, range2: range<Any> -> boolean", bi_met_by),
    b!("overlaps", "range1: range<Any>, range2: range<Any> -> boolean", bi_overlaps),
    b!("overlaps before", "range1: range<Any>, range2: range<Any> -> boolean", bi_overlaps_before),
    b!("overlaps after", "range1: range<Any>, range2: range<Any> -> boolean", bi_overlaps_after),
    b!(
        "finishes",
        "point: Any, range: range<Any> -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_finishes
    ),
    b!(
        "finished by",
        "range: range<Any>, point: Any -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_finished_by
    ),
    b!(
        "includes",
        "range: range<Any>, point: Any -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_includes
    ),
    b!(
        "during",
        "point: Any, range: range<Any> -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_during
    ),
    b!(
        "starts",
        "point: Any, range: range<Any> -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_starts
    ),
    b!(
        "started by",
        "range: range<Any>, point: Any -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_started_by
    ),
    b!(
        "coincides",
        "point1: Any, point2: Any -> boolean | range1: range<Any>, range2: range<Any> -> boolean",
        bi_coincides
    ),
    // 10.3.4.8 temporal
    b!("day of year", "date: Any -> number", bi_day_of_year),
    b!("day of week", "date: Any -> string", bi_day_of_week),
    b!("month of year", "date: Any -> string", bi_month_of_year),
    b!("week of year", "date: Any -> number", bi_week_of_year),
    // 10.3.4.9 sort
    b!("sort", "list: list<Any>, precedes?: function<Any, Any>->boolean -> list<Any>", bi_sort),
    // 10.3.4.10 context
    b!("get value", "m: context, key: string -> Any", bi_get_value),
    b!("get entries", "m: context -> list<context<key: string, value: Any>>", bi_get_entries),
    b!("context", "entries: list<Any> -> context", bi_context),
    b!(
        "context put",
        "context: context, key: string, value: Any -> context | context: context, keys: list<string>, value: Any -> context",
        bi_context_put
    ),
    b!("context merge", "contexts: list<context> -> context", bi_context_merge),
    // 10.3.4.11 misc (host clock capability)
    Builtin { name: "now", spec: " -> date and time", imp: bi_now, deterministic: false, extension: 0 },
    Builtin { name: "today", spec: " -> date", imp: bi_today, deterministic: false, extension: 0 },
    // extensions (outside the DMN 1.5 claim; opt-in)
    Builtin {
        name: "has key",
        spec: "context: context, key: string -> boolean",
        imp: bi_has_key,
        deterministic: true,
        extension: EXT_HAS_KEY,
    },
];

pub fn lookup(name: &str) -> Option<u16> {
    BUILTINS.iter().position(|b| b.name == name).map(|i| i as u16)
}

pub fn name_of(id: u16) -> &'static str {
    BUILTINS.get(id as usize).map(|b| b.name).unwrap_or("?")
}

// ------------------------------------------------------------------ signatures

#[derive(Clone, Debug, PartialEq)]
pub struct SigParam {
    pub name: String,
    pub ty: String,
    pub optional: bool,
    pub variadic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Overload {
    pub params: Vec<SigParam>,
    pub ret: String,
}

impl Overload {
    pub fn min_args(&self) -> usize {
        // a variadic parameter takes one or more arguments (N > 0)
        self.params.iter().filter(|p| !p.optional).count()
    }
    pub fn max_args(&self) -> Option<usize> {
        if self.params.iter().any(|p| p.variadic) {
            None
        } else {
            Some(self.params.len())
        }
    }
}

/// Split at top-level occurrences of `sep` (not inside `<...>`).
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '<' {
            depth += 1;
        } else if c == '>' && !(i > 0 && cs[i - 1] == '-') {
            depth -= 1;
        }
        if c == sep && depth == 0 {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
        i += 1;
    }
    out.push(cur.trim().to_string());
    out
}

pub fn parse_spec(spec: &str) -> Vec<Overload> {
    let mut res = Vec::new();
    for ov in split_top(spec, '|') {
        // last top-level "->" separates the return type
        let idx = rfind_top_arrow(&ov);
        let (ps, ret) = match idx {
            Some(i) => (&ov[..i], ov[i + 2..].trim()),
            None => (ov.as_str(), "Any"),
        };
        let mut params = Vec::new();
        if !ps.trim().is_empty() {
            for p in split_top(ps, ',') {
                let (n, t) = match p.find(':') {
                    Some(c) => (p[..c].trim().to_string(), p[c + 1..].trim().to_string()),
                    None => (p.trim().to_string(), "Any".to_string()),
                };
                let (n, optional) = match n.strip_suffix('?') {
                    Some(x) => (x.to_string(), true),
                    None => (n, false),
                };
                let (t, variadic) = match t.strip_suffix("...") {
                    Some(x) => (x.trim().to_string(), true),
                    None => (t, false),
                };
                params.push(SigParam { name: n, ty: t, optional, variadic });
            }
        }
        res.push(Overload { params, ret: ret.to_string() });
    }
    res
}

fn rfind_top_arrow(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut last = None;
    let mut i = 0;
    while i + 1 < b.len() {
        match b[i] {
            b'<' => depth += 1,
            b'>' if !(i > 0 && b[i - 1] == b'-') => depth -= 1,
            b'-' if b[i + 1] == b'>' && depth == 0 => last = Some(i),
            _ => {}
        }
        i += 1;
    }
    last
}

thread_local! {
    static SIGS: Vec<Vec<Overload>> = BUILTINS.iter().map(|b| parse_spec(b.spec)).collect();
}

pub fn overloads(id: u16) -> Vec<Overload> {
    SIGS.with(|s| s.get(id as usize).cloned().unwrap_or_default())
}

// ------------------------------------------------------------------ dispatch

fn is_list_type(t: &str) -> bool {
    t.starts_with("list")
}

/// Invoke builtin `id` with positional arguments.
pub fn call(cx: &mut dyn Cx, id: u16, args: &[H], site: u32) -> H {
    let Some(b) = BUILTINS.get(id as usize) else {
        cx.rt().diag("FEEL-E320", site, "unknown builtin");
        return NULL;
    };
    if b.extension != 0 && cx.rt().extensions & b.extension == 0 {
        cx.rt().diag("FEEL-E322", site, format!("builtin '{}' is an extension that is not enabled", b.name));
        return NULL;
    }
    let ovs = overloads(id);
    let n = args.len();
    let ok = ovs.iter().any(|o| n >= o.min_args() && o.max_args().map(|m| n <= m).unwrap_or(true));
    if !ok {
        cx.rt().diag(
            "FEEL-E320",
            site,
            format!("wrong number of arguments ({n}) for builtin '{}'", b.name),
        );
        return NULL;
    }
    // implicit conversion "from singleton list" for non-list parameters, when
    // every overload of this arity expects a non-list there
    let mut conv: Vec<H> = args.to_vec();
    for (i, a) in conv.iter_mut().enumerate() {
        let all_scalar = ovs.iter().filter(|o| n >= o.min_args() && o.max_args().map(|m| n <= m).unwrap_or(true)).all(|o| {
            let p = o.params.get(i).or_else(|| o.params.last().filter(|p| p.variadic));
            match p {
                Some(p) => !is_list_type(&p.ty) && p.ty != "Any",
                None => false,
            }
        });
        if all_scalar {
            let single = cx.rt().as_list(*a).and_then(|l| if l.len() == 1 { Some(l[0]) } else { None });
            if let Some(x) = single {
                *a = x;
            }
        }
    }
    // implicit conversion "to singleton list" for list parameters
    for (i, a) in conv.iter_mut().enumerate() {
        let all_list = ovs.iter().filter(|o| n >= o.min_args() && o.max_args().map(|m| n <= m).unwrap_or(true)).all(|o| {
            let p = o.params.get(i).or_else(|| o.params.last().filter(|p| p.variadic));
            match p {
                Some(p) => is_list_type(&p.ty) && !p.variadic,
                None => false,
            }
        });
        if all_list && !matches!(cx.rt().get(*a), Value::List(_) | Value::Null) {
            let l = cx.rt().list(vec![*a]);
            *a = l;
        }
    }
    (b.imp)(cx, &conv, site)
}

fn arg_conforms(rt: &mut Runtime, h: H, ty: &str) -> bool {
    if ty == "Any" || rt.is_null(h) {
        return true;
    }
    let t = types::parse_type(ty);
    if let Some(true) = types::value_conforms(rt, h, &t) {
        return true;
    }
    // implicit singleton conversions
    match (&t, rt.get(h).clone()) {
        (RtType::List(et), v) if !matches!(v, Value::List(_)) => types::value_conforms(rt, h, et) == Some(true),
        (t, Value::List(items)) if items.len() == 1 && !matches!(t, RtType::List(_)) => {
            types::value_conforms(rt, items[0], t) == Some(true)
        }
        (RtType::DateTime, Value::Date(_)) => true,
        _ => false,
    }
}

/// Invoke builtin `id` with named arguments given as a context.
pub fn call_named(cx: &mut dyn Cx, id: u16, named: H, site: u32) -> H {
    let Some(b) = BUILTINS.get(id as usize) else {
        return NULL;
    };
    let ctx = match cx.rt().as_ctx(named) {
        Some(c) => c.clone(),
        None => return NULL,
    };
    let ovs = overloads(id);
    let name_match: Vec<&Overload> = ovs.iter().filter(|o| ctx.keys.iter().all(|k| o.params.iter().any(|p| p.name == **k))).collect();
    // prefer an overload whose parameter types accept the named arguments
    let typed: Vec<&Overload> = name_match
        .iter()
        .copied()
        .filter(|o| {
            ctx.keys.iter().zip(ctx.vals.iter()).all(|(k, v)| {
                let p = o.params.iter().find(|p| p.name == **k).unwrap();
                arg_conforms(cx.rt(), *v, &p.ty)
            })
        })
        .collect();
    if typed.is_empty() && !name_match.is_empty() {
        cx.rt().diag("FEEL-E325", site, format!("named arguments of '{}' do not conform to its parameter types", b.name));
        return NULL;
    }
    for o in typed {
        {
            // required params must be supplied? Unsupplied are bound to null.
            let mut args = Vec::new();
            let last_supplied = o.params.iter().rposition(|p| ctx.get(&p.name).is_some());
            for (i, p) in o.params.iter().enumerate() {
                match ctx.get(&p.name) {
                    Some(v) => args.push(v),
                    None => {
                        if p.optional && last_supplied.map(|l| i > l).unwrap_or(true) {
                            break;
                        }
                        args.push(NULL);
                    }
                }
            }
            return call(cx, id, &args, site);
        }
    }
    let names: Vec<String> = ctx.keys.iter().map(|k| k.to_string()).collect();
    cx.rt().diag(
        "FEEL-E323",
        site,
        format!("builtin '{}' has no parameters named {}", b.name, names.join(", ")),
    );
    NULL
}

/// Call an arbitrary function value with a positional argument list.
pub fn call_value(cx: &mut dyn Cx, f: H, args: H, named: bool, site: u32) -> H {
    let fv = match cx.rt().get(f) {
        Value::Func(fv) => fv.clone(),
        _ => {
            let k = cx.rt().kind_name(f);
            cx.rt().diag("FEEL-E324", site, format!("cannot invoke a value of type {k}"));
            return NULL;
        }
    };
    match fv {
        Func::Builtin(id) => {
            if named {
                call_named(cx, id, args, site)
            } else {
                let a = cx.rt().list_items(args).unwrap_or_default();
                call(cx, id, &a, site)
            }
        }
        Func::User { .. } => {
            let bound = bind_args(cx.rt(), f, args, named, site);
            if bound == ABSENT {
                return NULL;
            }
            cx.apply(f, bound)
        }
        Func::External { .. } | Func::Host { .. } => {
            let bound = bind_args(cx.rt(), f, args, named, site);
            if bound == ABSENT {
                return NULL;
            }
            cx.call_host(f, bound)
        }
    }
}

/// Bind actual arguments to a user/external function's formal parameters
/// (Table 63, 10.3.2.13.2): arity / names, then implicit conversions and
/// type conformance. Returns a positional list handle, or `ABSENT` when the
/// invocation must yield null.
pub fn bind_args(rt: &mut Runtime, f: H, args: H, named: bool, site: u32) -> H {
    let sig = match rt.get(f) {
        Value::Func(Func::User { sig, .. }) | Value::Func(Func::External { sig, .. }) | Value::Func(Func::Host { sig, .. }) => {
            *sig
        }
        _ => return ABSENT,
    };
    let params: Vec<(Rc<str>, Option<Rc<str>>)> = rt
        .list_items(sig)
        .unwrap_or_default()
        .iter()
        .map(|p| {
            let pair = rt.list_items(*p).unwrap_or_default();
            let n = pair.first().and_then(|h| rt.as_str(*h)).unwrap_or_else(|| Rc::from(""));
            let t = pair.get(1).and_then(|h| rt.as_str(*h));
            (n, t)
        })
        .collect();
    let mut vals: Vec<H> = Vec::with_capacity(params.len());
    if named {
        let ctx = match rt.as_ctx(args) {
            Some(c) => c.clone(),
            None => return ABSENT,
        };
        for k in &ctx.keys {
            if !params.iter().any(|(n, _)| n == k) {
                rt.diag("FEEL-E323", site, format!("function has no parameter named '{k}'"));
                return ABSENT;
            }
        }
        for (n, _) in &params {
            vals.push(ctx.get(n).unwrap_or(NULL));
        }
    } else {
        let items = rt.list_items(args).unwrap_or_default();
        if items.len() != params.len() {
            rt.diag(
                "FEEL-E320",
                site,
                format!("function expects {} arguments but {} were given", params.len(), items.len()),
            );
            return ABSENT;
        }
        vals = items;
    }
    for (i, (n, t)) in params.iter().enumerate() {
        if let Some(t) = t {
            let ty = types::parse_type(t);
            let (x, ok) = types::convert_to(rt, vals[i], &ty);
            if !ok {
                let k = rt.kind_name(vals[i]);
                rt.diag(
                    "FEEL-E325",
                    site,
                    format!("argument '{n}' of type {k} does not conform to parameter type {ty}"),
                );
                return ABSENT;
            }
            vals[i] = x;
        }
    }
    rt.list(vals)
}

// ------------------------------------------------------------------ helpers

fn bad(cx: &mut dyn Cx, site: u32, name: &str, why: &str) -> H {
    cx.rt().diag("FEEL-E340", site, format!("{name}: {why}"));
    NULL
}

fn num(cx: &mut dyn Cx, h: H) -> Option<Decimal> {
    cx.rt().as_num(h)
}
fn int(cx: &mut dyn Cx, h: H) -> Option<i64> {
    cx.rt().as_num(h).and_then(|d| d.to_i64())
}
/// Integer argument with truncation of a fractional part (Java `intValue()`
/// semantics used by the reference implementations for positions, lengths
/// and scales).
fn int_trunc(cx: &mut dyn Cx, h: H) -> Option<i64> {
    cx.rt().as_num(h).and_then(|d| d.trunc().to_i64())
}
fn s(cx: &mut dyn Cx, h: H) -> Option<Rc<str>> {
    cx.rt().as_str(h)
}
fn list(cx: &mut dyn Cx, h: H) -> Option<Vec<H>> {
    cx.rt().list_items(h)
}

/// List argument or variadic arguments (min, max, sum, ...).
fn list_or_varargs(cx: &mut dyn Cx, a: &[H]) -> Option<Vec<H>> {
    if a.len() == 1 {
        match cx.rt().get(a[0]) {
            Value::List(v) => Some(v.clone()),
            _ => Some(vec![a[0]]),
        }
    } else {
        Some(a.to_vec())
    }
}

fn nums(cx: &mut dyn Cx, items: &[H]) -> Option<Vec<Decimal>> {
    items.iter().map(|h| cx.rt().as_num(*h)).collect()
}

fn opt_num(cx: &mut dyn Cx, d: Option<Decimal>, site: u32, name: &str) -> H {
    match d {
        Some(d) => cx.rt().num(d),
        None => bad(cx, site, name, "result is not a FEEL number"),
    }
}

// ------------------------------------------------------------------ conversion

fn bi_date(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    if a.len() == 1 {
        let v = cx.rt().get(a[0]).clone();
        let r = match v {
            Value::Str(s) => tm::Date::parse(&s),
            Value::DateTime(dt) => Some(dt.date),
            Value::Date(d) => Some(d),
            Value::Null => return NULL,
            _ => None,
        };
        return match r {
            Some(d) => cx.rt().alloc(Value::Date(d)),
            None => bad(cx, site, "date", "invalid date"),
        };
    }
    let (Some(y), Some(m), Some(d)) = (int(cx, a[0]), int(cx, a[1]), int(cx, a[2])) else {
        return bad(cx, site, "date", "year, month and day must be integers");
    };
    match tm::Date::from_ymd(y, m, d) {
        Some(d) => cx.rt().alloc(Value::Date(d)),
        None => bad(cx, site, "date", "invalid date"),
    }
}

fn bi_date_and_time(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    if a.len() == 1 {
        let v = cx.rt().get(a[0]).clone();
        let r = match v {
            Value::Str(s) => tm::DateTime::parse(&s),
            Value::DateTime(dt) => Some(dt),
            Value::Date(d) => Some(tm::DateTime::from_date(&d)),
            Value::Null => return NULL,
            _ => None,
        };
        return match r {
            Some(d) => cx.rt().alloc(Value::DateTime(d)),
            None => bad(cx, site, "date and time", "invalid date and time"),
        };
    }
    let d = match cx.rt().get(a[0]).clone() {
        Value::Date(d) => d,
        Value::DateTime(dt) => dt.date,
        _ => return bad(cx, site, "date and time", "first argument must be a date"),
    };
    let t = match cx.rt().get(a[1]).clone() {
        Value::Time(t) => t,
        _ => return bad(cx, site, "date and time", "second argument must be a time"),
    };
    let dt = tm::DateTime::from_date_time(&d, &t);
    cx.rt().alloc(Value::DateTime(dt))
}

fn bi_time(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    if a.len() == 1 {
        let v = cx.rt().get(a[0]).clone();
        let r = match v {
            Value::Str(s) => tm::Time::parse(&s),
            Value::DateTime(dt) => Some(dt.time()),
            Value::Time(t) => Some(t),
            Value::Date(_) => tm::Time::new(0, 0, 0, Some(tm::Zone::Offset(0))),
            Value::Null => return NULL,
            _ => None,
        };
        return match r {
            Some(t) => cx.rt().alloc(Value::Time(t)),
            None => bad(cx, site, "time", "invalid time"),
        };
    }
    let (Some(h), Some(m), Some(sec)) = (int(cx, a[0]), int(cx, a[1]), num(cx, a[2])) else {
        return bad(cx, site, "time", "hour and minute must be integers and second a number");
    };
    let nanos = sec.mul(&Decimal::from_i64(1_000_000_000)).and_then(|x| x.set_scale(0, RoundingMode::Down)).and_then(|x| x.to_i64());
    let Some(nanos) = nanos else { return bad(cx, site, "time", "invalid second") };
    let zone = if a.len() > 3 {
        match cx.rt().get(a[3]).clone() {
            Value::Null => None,
            Value::Dtd(d) => {
                if d.nanos % 1_000_000_000 != 0 {
                    return bad(cx, site, "time", "offset must be whole seconds");
                }
                Some(tm::Zone::Offset((d.nanos / 1_000_000_000) as i32))
            }
            _ => return bad(cx, site, "time", "offset must be a days and time duration"),
        }
    } else {
        None
    };
    match tm::Time::new(h, m, nanos, zone) {
        Some(t) => cx.rt().alloc(Value::Time(t)),
        None => bad(cx, site, "time", "invalid time components"),
    }
}

fn bi_number(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(from) = s(cx, a[0]) else { return bad(cx, site, "number", "from must be a string") };
    let sep = |cx: &mut dyn Cx, i: usize| -> Result<Option<Rc<str>>, ()> {
        if i >= a.len() || cx.rt().is_null(a[i]) {
            Ok(None)
        } else {
            match cx.rt().as_str(a[i]) {
                Some(x) => Ok(Some(x)),
                None => Err(()),
            }
        }
    };
    let (Ok(g), Ok(d)) = (sep(cx, 1), sep(cx, 2)) else {
        return bad(cx, site, "number", "separators must be strings");
    };
    if let Some(g) = &g {
        if !matches!(&**g, " " | "," | ".") {
            return bad(cx, site, "number", "grouping separator must be one of space, comma or period");
        }
    }
    if let Some(d) = &d {
        if !matches!(&**d, "," | ".") {
            return bad(cx, site, "number", "decimal separator must be comma or period");
        }
    }
    if g.is_some() && g == d {
        return bad(cx, site, "number", "grouping and decimal separators must differ");
    }
    let mut t = from.to_string();
    if let Some(g) = &g {
        t = t.replace(&**g, "");
    }
    if let Some(d) = &d {
        if &**d != "." {
            if t.contains('.') {
                return bad(cx, site, "number", "unexpected '.'");
            }
            t = t.replace(&**d, ".");
        }
    }
    let ok = !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'));
    match if ok { Decimal::parse(&t) } else { None } {
        Some(x) => cx.rt().num(x),
        None => bad(cx, site, "number", "not a number"),
    }
}

/// FEEL string representation used by `string()`.
pub fn to_feel_string(rt: &Runtime, h: H, top: bool) -> String {
    match rt.get(h) {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Num(d) => d.to_string(),
        Value::Str(s) => {
            if top {
                s.to_string()
            } else {
                let mut o = String::new();
                crate::json::write_json_string(&mut o, s);
                o
            }
        }
        Value::Date(d) => d.to_string(),
        Value::Time(t) => t.to_string(),
        Value::DateTime(t) => t.to_string(),
        Value::Dtd(d) => d.to_string(),
        Value::Ymd(d) => d.to_string(),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(|x| to_feel_string(rt, *x, false)).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Ctx(c) => {
            let parts: Vec<String> =
                c.keys.iter().zip(c.vals.iter()).map(|(k, v)| format!("{}: {}", k, to_feel_string(rt, *v, false))).collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Range(r) => {
            let ep = |x: Option<H>| x.map(|h| to_feel_string(rt, h, false)).unwrap_or_default();
            match (r.start, r.end) {
                (None, Some(e)) => format!("{} {}", if r.end_incl { "<=" } else { "<" }, to_feel_string(rt, e, false)),
                (Some(s), None) => format!("{} {}", if r.start_incl { ">=" } else { ">" }, to_feel_string(rt, s, false)),
                _ => format!(
                    "{}{}..{}{}",
                    if r.start_incl { "[" } else { "(" },
                    ep(r.start),
                    ep(r.end),
                    if r.end_incl { "]" } else { ")" }
                ),
            }
        }
        Value::Func(_) => "function".to_string(),
    }
}

fn bi_string(cx: &mut dyn Cx, a: &[H], _site: u32) -> H {
    if cx.rt().is_null(a[0]) {
        return NULL;
    }
    let s = to_feel_string(cx.rt(), a[0], true);
    cx.rt().string(s)
}

fn bi_duration(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(x) = s(cx, a[0]) else { return bad(cx, site, "duration", "from must be a string") };
    match tm::parse_duration(&x) {
        Some(tm::Duration::DayTime(d)) => cx.rt().alloc(Value::Dtd(d)),
        Some(tm::Duration::YearMonth(d)) => cx.rt().alloc(Value::Ymd(d)),
        None => bad(cx, site, "duration", "invalid duration"),
    }
}

fn to_dt(v: &Value) -> Option<tm::DateTime> {
    match v {
        Value::Date(d) => Some(tm::DateTime::from_date(d)),
        Value::DateTime(d) => Some(d.clone()),
        _ => None,
    }
}

fn bi_ymd(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let f = to_dt(cx.rt().get(a[0]));
    let t = to_dt(cx.rt().get(a[1]));
    match (f, t) {
        (Some(f), Some(t)) => match tm::years_and_months_between(&f, &t) {
            Some(d) => cx.rt().alloc(Value::Ymd(d)),
            None => bad(cx, site, "years and months duration", "out of range"),
        },
        _ => bad(cx, site, "years and months duration", "arguments must be dates or date and times"),
    }
}

fn bi_range(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(x) = s(cx, a[0]) else { return bad(cx, site, "range", "from must be a string") };
    match crate::rangelit::parse_range(cx.rt(), &x) {
        Some(h) => h,
        None => bad(cx, site, "range", "invalid range literal"),
    }
}

fn bi_not(cx: &mut dyn Cx, a: &[H], _site: u32) -> H {
    match cx.rt().get(a[0]) {
        Value::Bool(b) => {
            let b = !*b;
            cx.rt().bool(b)
        }
        _ => NULL,
    }
}

// ------------------------------------------------------------------ strings

fn bi_substring(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(st) = s(cx, a[0]) else { return bad(cx, site, "substring", "string expected") };
    let Some(start) = num(cx, a[1]) else { return bad(cx, site, "substring", "start position must be a number") };
    let chars: Vec<char> = st.chars().collect();
    let len = chars.len() as i64;
    let start = match start.trunc().to_i64() {
        Some(x) => x,
        None => return bad(cx, site, "substring", "invalid start position"),
    };
    if start == 0 || start > len || start < -len {
        return bad(cx, site, "substring", "start position out of range");
    }
    let from = if start > 0 { start - 1 } else { len + start };
    let to = if a.len() > 2 && !cx.rt().is_null(a[2]) {
        let Some(l) = num(cx, a[2]) else { return bad(cx, site, "substring", "length must be a number") };
        let l = match l.trunc().to_i64() {
            Some(x) => x,
            None => return bad(cx, site, "substring", "invalid length"),
        };
        if l < 0 {
            return bad(cx, site, "substring", "length must not be negative");
        }
        (from + l).min(len)
    } else {
        len
    };
    let r: String = chars[from as usize..to as usize].iter().collect();
    cx.rt().string(r)
}

fn bi_string_length(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match s(cx, a[0]) {
        Some(x) => {
            let n = x.chars().count() as i64;
            cx.rt().int(n)
        }
        None => bad(cx, site, "string length", "string expected"),
    }
}

fn bi_upper(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match s(cx, a[0]) {
        Some(x) => {
            let r = x.to_uppercase();
            cx.rt().string(r)
        }
        None => bad(cx, site, "upper case", "string expected"),
    }
}

fn bi_lower(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match s(cx, a[0]) {
        Some(x) => {
            let r = x.to_lowercase();
            cx.rt().string(r)
        }
        None => bad(cx, site, "lower case", "string expected"),
    }
}

fn two_strings(cx: &mut dyn Cx, a: &[H]) -> Option<(Rc<str>, Rc<str>)> {
    Some((s(cx, a[0])?, s(cx, a[1])?))
}

fn bi_substring_before(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some((x, m)) = two_strings(cx, a) else { return bad(cx, site, "substring before", "strings expected") };
    let r = match x.find(&*m) {
        Some(i) => x[..i].to_string(),
        None => String::new(),
    };
    cx.rt().string(r)
}

fn bi_substring_after(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some((x, m)) = two_strings(cx, a) else { return bad(cx, site, "substring after", "strings expected") };
    let r = match x.find(&*m) {
        Some(i) => x[i + m.len()..].to_string(),
        None => String::new(),
    };
    cx.rt().string(r)
}

fn bi_contains(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some((x, m)) = two_strings(cx, a) else { return bad(cx, site, "contains", "strings expected") };
    cx.rt().bool(x.contains(&*m))
}
fn bi_starts_with(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some((x, m)) = two_strings(cx, a) else { return bad(cx, site, "starts with", "strings expected") };
    cx.rt().bool(x.starts_with(&*m))
}
fn bi_ends_with(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some((x, m)) = two_strings(cx, a) else { return bad(cx, site, "ends with", "strings expected") };
    cx.rt().bool(x.ends_with(&*m))
}

fn regex_arg(cx: &mut dyn Cx, pat: H, flags: Option<H>) -> Option<Rc<fancy_regex::Regex>> {
    let p = s(cx, pat)?;
    let f = match flags {
        Some(h) if !cx.rt().is_null(h) => s(cx, h)?,
        _ => Rc::from(""),
    };
    let rt = cx.rt();
    if let Some((_, _, r)) = rt.regex_cache.iter().find(|(a, b, _)| **a == *p && **b == *f) {
        return Some(r.clone());
    }
    let r = Rc::new(crate::regex_xpath::compile(&p, &f)?);
    if rt.regex_cache.len() > 64 {
        rt.regex_cache.remove(0);
    }
    rt.regex_cache.push((p.to_string(), f.to_string(), r.clone()));
    Some(r)
}

fn bi_matches(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(input) = s(cx, a[0]) else { return bad(cx, site, "matches", "input must be a string") };
    let Some(re) = regex_arg(cx, a[1], a.get(2).copied()) else {
        return bad(cx, site, "matches", "invalid pattern or flags");
    };
    match re.is_match(&*input) {
        Ok(b) => cx.rt().bool(b),
        Err(_) => bad(cx, site, "matches", "regex evaluation failed"),
    }
}

fn bi_replace(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(input) = s(cx, a[0]) else { return bad(cx, site, "replace", "input must be a string") };
    let Some(rep) = s(cx, a[2]) else { return bad(cx, site, "replace", "replacement must be a string") };
    let Some(re) = regex_arg(cx, a[1], a.get(3).copied()) else {
        return bad(cx, site, "replace", "invalid pattern or flags");
    };
    if let Ok(true) = re.is_match("") {
        return bad(cx, site, "replace", "pattern matches the empty string");
    }
    let ngroups = re.captures_len() - 1;
    let mut out = String::new();
    let mut last = 0;
    for caps in re.captures_iter(&*input) {
        let Ok(caps) = caps else { return bad(cx, site, "replace", "regex evaluation failed") };
        let m = caps.get(0).unwrap();
        out.push_str(&input[last..m.start()]);
        match crate::regex_xpath::expand_replacement(&rep, &caps, ngroups) {
            Some(x) => out.push_str(&x),
            None => return bad(cx, site, "replace", "invalid replacement string"),
        }
        last = m.end();
    }
    out.push_str(&input[last..]);
    cx.rt().string(out)
}

fn bi_split(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(input) = s(cx, a[0]) else { return bad(cx, site, "split", "string expected") };
    let Some(re) = regex_arg(cx, a[1], None) else { return bad(cx, site, "split", "invalid delimiter pattern") };
    if let Ok(true) = re.is_match("") {
        return bad(cx, site, "split", "delimiter matches the empty string");
    }
    let mut parts = Vec::new();
    let mut last = 0;
    for m in re.find_iter(&*input) {
        let Ok(m) = m else { return bad(cx, site, "split", "regex evaluation failed") };
        parts.push(input[last..m.start()].to_string());
        last = m.end();
    }
    parts.push(input[last..].to_string());
    let hs: Vec<H> = parts.into_iter().map(|p| cx.rt().string(p)).collect();
    cx.rt().list(hs)
}

fn bi_string_join(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list(cx, a[0]) else { return bad(cx, site, "string join", "list expected") };
    let delim: Rc<str> = if a.len() > 1 && !cx.rt().is_null(a[1]) {
        match s(cx, a[1]) {
            Some(d) => d,
            None => return bad(cx, site, "string join", "delimiter must be a string"),
        }
    } else {
        Rc::from("")
    };
    let mut parts = Vec::new();
    for it in items {
        match cx.rt().get(it) {
            Value::Null => {}
            Value::Str(x) => parts.push(x.to_string()),
            _ => return bad(cx, site, "string join", "list elements must be strings"),
        }
    }
    let r = parts.join(&delim);
    cx.rt().string(r)
}

// ------------------------------------------------------------------ lists

fn bi_list_contains(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list(cx, a[0]) else { return bad(cx, site, "list contains", "list expected") };
    let found = items.iter().any(|x| ops::eq(cx.rt(), *x, a[1]) == Some(true));
    cx.rt().bool(found)
}

fn bi_count(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match list(cx, a[0]) {
        Some(items) => cx.rt().int(items.len() as i64),
        None => bad(cx, site, "count", "list expected"),
    }
}

fn extreme(cx: &mut dyn Cx, a: &[H], site: u32, name: &str, want: Ordering) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    if items.is_empty() {
        return NULL;
    }
    let mut best = items[0];
    for &it in &items[1..] {
        match ops::cmp(cx.rt(), it, best) {
            Some(o) if o == want => best = it,
            Some(_) => {}
            None => return bad(cx, site, name, "items are not comparable"),
        }
    }
    if items.len() == 1 && ops::cmp(cx.rt(), best, best).is_none() {
        return bad(cx, site, name, "item is not comparable");
    }
    best
}

fn bi_min(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    extreme(cx, a, site, "min", Ordering::Less)
}
fn bi_max(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    extreme(cx, a, site, "max", Ordering::Greater)
}

fn bi_sum(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    if items.is_empty() {
        return NULL;
    }
    let Some(ns) = nums(cx, &items) else { return bad(cx, site, "sum", "all items must be numbers") };
    let mut acc = Decimal::ZERO;
    for n in ns {
        match acc.add(&n) {
            Some(x) => acc = x,
            None => return bad(cx, site, "sum", "overflow"),
        }
    }
    cx.rt().num(acc)
}

fn bi_product(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    if items.is_empty() {
        return NULL;
    }
    let Some(ns) = nums(cx, &items) else { return bad(cx, site, "product", "all items must be numbers") };
    let mut acc = Decimal::ONE;
    for n in ns {
        match acc.mul(&n) {
            Some(x) => acc = x,
            None => return bad(cx, site, "product", "overflow"),
        }
    }
    cx.rt().num(acc)
}

fn mean_of(ns: &[Decimal]) -> Option<Decimal> {
    let mut acc = Decimal::ZERO;
    for n in ns {
        acc = acc.add(n)?;
    }
    acc.div(&Decimal::from_i64(ns.len() as i64))
}

fn bi_mean(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    if items.is_empty() {
        return NULL;
    }
    let Some(ns) = nums(cx, &items) else { return bad(cx, site, "mean", "all items must be numbers") };
    let m = mean_of(&ns);
    opt_num(cx, m, site, "mean")
}

fn bi_median(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    if items.is_empty() {
        return NULL;
    }
    let Some(mut ns) = nums(cx, &items) else { return bad(cx, site, "median", "all items must be numbers") };
    ns.sort();
    let n = ns.len();
    if n % 2 == 1 {
        cx.rt().num(ns[n / 2])
    } else {
        let m = ns[n / 2 - 1].add(&ns[n / 2]).and_then(|x| x.div(&Decimal::from_i64(2)));
        opt_num(cx, m, site, "median")
    }
}

fn bi_stddev(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    if items.len() < 2 {
        return NULL;
    }
    let Some(ns) = nums(cx, &items) else { return bad(cx, site, "stddev", "all items must be numbers") };
    let r = (|| {
        let m = mean_of(&ns)?;
        let mut acc = Decimal::ZERO;
        for x in &ns {
            let d = x.sub(&m)?;
            acc = acc.add(&d.mul(&d)?)?;
        }
        acc.div(&Decimal::from_i64(ns.len() as i64 - 1))?.sqrt()
    })();
    opt_num(cx, r, site, "stddev")
}

fn bi_mode(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    let Some(mut ns) = nums(cx, &items) else { return bad(cx, site, "mode", "all items must be numbers") };
    ns.sort();
    let mut best: Vec<Decimal> = Vec::new();
    let mut best_n = 0;
    let mut i = 0;
    while i < ns.len() {
        let mut j = i;
        while j < ns.len() && ns[j] == ns[i] {
            j += 1;
        }
        let c = j - i;
        if c > best_n {
            best_n = c;
            best = vec![ns[i]];
        } else if c == best_n {
            best.push(ns[i]);
        }
        i = j;
    }
    let hs: Vec<H> = best.into_iter().map(|d| cx.rt().num(d)).collect();
    cx.rt().list(hs)
}

fn bi_all(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    let mut r = T_TRUE;
    for it in items {
        let t = ops::truth(cx.rt(), it);
        r = ops::tri_and(r, t);
    }
    if r == T_NULL {
        cx.rt().diag("FEEL-E340", site, "all: non-boolean item");
    }
    ops::tri_to_h(r)
}

fn bi_any(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list_or_varargs(cx, a) else { return NULL };
    let mut r = T_FALSE;
    for it in items {
        let t = ops::truth(cx.rt(), it);
        r = ops::tri_or(r, t);
    }
    if r == T_NULL {
        cx.rt().diag("FEEL-E340", site, "any: non-boolean item");
    }
    ops::tri_to_h(r)
}

fn position(cx: &mut dyn Cx, h: H, len: usize) -> Option<usize> {
    let p = int_trunc(cx, h)?;
    let len = len as i64;
    if p > 0 && p <= len {
        Some((p - 1) as usize)
    } else if p < 0 && -p <= len {
        Some((len + p) as usize)
    } else {
        None
    }
}

fn bi_sublist(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list(cx, a[0]) else { return bad(cx, site, "sublist", "list expected") };
    let Some(from) = position(cx, a[1], items.len()) else {
        return bad(cx, site, "sublist", "start position out of range");
    };
    let to = if a.len() > 2 && !cx.rt().is_null(a[2]) {
        let Some(l) = int_trunc(cx, a[2]) else { return bad(cx, site, "sublist", "length must be an integer") };
        if l < 0 || from as i64 + l > items.len() as i64 {
            return bad(cx, site, "sublist", "length out of range");
        }
        from + l as usize
    } else {
        items.len()
    };
    cx.rt().list(items[from..to].to_vec())
}

fn bi_append(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(mut items) = list(cx, a[0]) else { return bad(cx, site, "append", "list expected") };
    items.extend_from_slice(&a[1..]);
    cx.rt().list(items)
}

fn bi_concatenate(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let mut out = Vec::new();
    for &x in a {
        match list(cx, x) {
            Some(items) => out.extend(items),
            None => return bad(cx, site, "concatenate", "all arguments must be lists"),
        }
    }
    cx.rt().list(out)
}

fn bi_insert_before(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(mut items) = list(cx, a[0]) else { return bad(cx, site, "insert before", "list expected") };
    let Some(p) = position(cx, a[1], items.len()) else {
        return bad(cx, site, "insert before", "position out of range");
    };
    items.insert(p, a[2]);
    cx.rt().list(items)
}

fn bi_remove(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(mut items) = list(cx, a[0]) else { return bad(cx, site, "remove", "list expected") };
    let Some(p) = position(cx, a[1], items.len()) else { return bad(cx, site, "remove", "position out of range") };
    items.remove(p);
    cx.rt().list(items)
}

fn bi_list_replace(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(mut items) = list(cx, a[0]) else { return bad(cx, site, "list replace", "list expected") };
    if let Value::Func(_) = cx.rt().get(a[1]) {
        for i in 0..items.len() {
            let args = cx.rt().list(vec![items[i], a[2]]);
            let r = call_value(cx, a[1], args, false, site);
            match cx.rt().as_bool(r) {
                Some(true) => items[i] = a[2],
                Some(false) => {}
                None => return bad(cx, site, "list replace", "match function must return a boolean"),
            }
            if cx.rt().aborted() {
                return NULL;
            }
        }
        return cx.rt().list(items);
    }
    let Some(p) = position(cx, a[1], items.len()) else {
        return bad(cx, site, "list replace", "position out of range");
    };
    items[p] = a[2];
    cx.rt().list(items)
}

fn bi_reverse(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(mut items) = list(cx, a[0]) else { return bad(cx, site, "reverse", "list expected") };
    items.reverse();
    cx.rt().list(items)
}

fn bi_index_of(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list(cx, a[0]) else { return bad(cx, site, "index of", "list expected") };
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if ops::eq(cx.rt(), *it, a[1]) == Some(true) {
            let h = cx.rt().int(i as i64 + 1);
            out.push(h);
        }
    }
    cx.rt().list(out)
}

fn distinct(cx: &mut dyn Cx, items: Vec<H>) -> Vec<H> {
    let mut out: Vec<H> = Vec::new();
    for it in items {
        if !out.iter().any(|o| ops::eq(cx.rt(), *o, it) == Some(true)) {
            out.push(it);
        }
    }
    out
}

fn bi_union(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let mut all = Vec::new();
    for &x in a {
        match list(cx, x) {
            Some(items) => all.extend(items),
            None => return bad(cx, site, "union", "all arguments must be lists"),
        }
    }
    let d = distinct(cx, all);
    cx.rt().list(d)
}

fn bi_distinct_values(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list(cx, a[0]) else { return bad(cx, site, "distinct values", "list expected") };
    let d = distinct(cx, items);
    cx.rt().list(d)
}

fn flatten_into(rt: &Runtime, h: H, out: &mut Vec<H>, depth: u32) {
    match rt.get(h) {
        Value::List(items) if depth < 256 => {
            for x in items {
                flatten_into(rt, *x, out, depth + 1)
            }
        }
        _ => out.push(h),
    }
}

fn bi_flatten(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    if list(cx, a[0]).is_none() {
        return bad(cx, site, "flatten", "list expected");
    }
    let mut out = Vec::new();
    flatten_into(cx.rt(), a[0], &mut out, 0);
    cx.rt().list(out)
}

// ------------------------------------------------------------------ numeric

fn scale_arg(cx: &mut dyn Cx, h: H) -> Option<i64> {
    let s = int_trunc(cx, h)?;
    if (-6111..=6176).contains(&s) {
        Some(s)
    } else {
        None
    }
}

fn round_fn(cx: &mut dyn Cx, a: &[H], site: u32, name: &str, mode: RoundingMode, default_scale: Option<i64>) -> H {
    let Some(n) = num(cx, a[0]) else { return bad(cx, site, name, "n must be a number") };
    let scale = if a.len() > 1 {
        if cx.rt().is_null(a[1]) {
            return bad(cx, site, name, "scale is null");
        }
        match scale_arg(cx, a[1]) {
            Some(s) => s,
            None => return bad(cx, site, name, "scale must be an integer in [-6111..6176]"),
        }
    } else {
        match default_scale {
            Some(s) => s,
            None => return bad(cx, site, name, "scale required"),
        }
    };
    // set the scale; if padding would exceed 34 digits the value already has
    // fewer fractional digits than requested and is returned unchanged
    let r = n.set_scale(scale, mode).or_else(|| if n.scale() as i64 <= scale { Some(n) } else { n.round_to_scale(scale, mode) });
    opt_num(cx, r, site, name)
}

fn bi_decimal(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    round_fn(cx, a, site, "decimal", RoundingMode::HalfEven, None)
}
fn bi_floor(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    round_fn(cx, a, site, "floor", RoundingMode::Floor, Some(0))
}
fn bi_ceiling(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    round_fn(cx, a, site, "ceiling", RoundingMode::Ceiling, Some(0))
}
fn bi_round_up(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    round_fn(cx, a, site, "round up", RoundingMode::Up, None)
}
fn bi_round_down(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    round_fn(cx, a, site, "round down", RoundingMode::Down, None)
}
fn bi_round_half_up(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    round_fn(cx, a, site, "round half up", RoundingMode::HalfUp, None)
}
fn bi_round_half_down(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    round_fn(cx, a, site, "round half down", RoundingMode::HalfDown, None)
}

fn bi_abs(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match cx.rt().get(a[0]).clone() {
        Value::Num(d) => cx.rt().num(d.abs()),
        Value::Dtd(d) => cx.rt().alloc(Value::Dtd(tm::DayTimeDuration { nanos: d.nanos.abs() })),
        Value::Ymd(d) => cx.rt().alloc(Value::Ymd(tm::YearMonthDuration { months: d.months.abs() })),
        _ => bad(cx, site, "abs", "number or duration expected"),
    }
}

fn bi_modulo(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let (Some(x), Some(y)) = (num(cx, a[0]), num(cx, a[1])) else {
        return bad(cx, site, "modulo", "numbers expected");
    };
    if y.is_zero() {
        return bad(cx, site, "modulo", "divisor is zero");
    }
    let r = x.modulo(&y);
    opt_num(cx, r, site, "modulo")
}

fn bi_sqrt(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(x) = num(cx, a[0]) else { return bad(cx, site, "sqrt", "number expected") };
    if x.is_negative() {
        return bad(cx, site, "sqrt", "negative argument");
    }
    let r = x.sqrt();
    opt_num(cx, r, site, "sqrt")
}

fn bi_log(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(x) = num(cx, a[0]) else { return bad(cx, site, "log", "number expected") };
    if !x.is_positive() {
        return bad(cx, site, "log", "argument must be positive");
    }
    let r = x.ln();
    opt_num(cx, r, site, "log")
}

fn bi_exp(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(x) = num(cx, a[0]) else { return bad(cx, site, "exp", "number expected") };
    let r = x.exp();
    opt_num(cx, r, site, "exp")
}

fn bi_odd(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(x) = num(cx, a[0]) else { return bad(cx, site, "odd", "number expected") };
    if !x.is_integer() {
        return bad(cx, site, "odd", "integer expected");
    }
    let r = x.rem(&Decimal::from_i64(2)).map(|r| !r.is_zero()).unwrap_or(false);
    cx.rt().bool(r)
}

fn bi_even(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(x) = num(cx, a[0]) else { return bad(cx, site, "even", "number expected") };
    if !x.is_integer() {
        return bad(cx, site, "even", "integer expected");
    }
    let r = x.rem(&Decimal::from_i64(2)).map(|r| r.is_zero()).unwrap_or(false);
    cx.rt().bool(r)
}

// ------------------------------------------------------------------ is

pub fn is_same(rt: &Runtime, a: H, b: H) -> bool {
    match (rt.get(a), rt.get(b)) {
        (Value::Null, Value::Null) => true,
        (Value::Time(x), Value::Time(y)) => tm::is_same_time(x, y),
        (Value::DateTime(x), Value::DateTime(y)) => tm::is_same_datetime(x, y),
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| is_same(rt, *p, *q))
        }
        (Value::Ctx(x), Value::Ctx(y)) => {
            x.len() == y.len()
                && x.keys.iter().zip(x.vals.iter()).all(|(k, v)| y.get(k).map(|w| is_same(rt, *v, w)).unwrap_or(false))
        }
        (x, y) => {
            if std::mem::discriminant(x) != std::mem::discriminant(y) {
                return false;
            }
            ops::eq(rt, a, b) == Some(true)
        }
    }
}

fn bi_is(cx: &mut dyn Cx, a: &[H], _site: u32) -> H {
    let r = is_same(cx.rt(), a[0], a[1]);
    cx.rt().bool(r)
}

// ------------------------------------------------------------------ range functions (Table 78)

#[derive(Clone)]
struct R {
    s: Option<H>,
    e: Option<H>,
    si: bool,
    ei: bool,
}

fn as_range(rt: &Runtime, h: H) -> Option<R> {
    match rt.get(h) {
        Value::Range(r) => Some(R { s: r.start, e: r.end, si: r.start_incl, ei: r.end_incl }),
        _ => None,
    }
}

/// Comparison helpers over possibly-undefined endpoints: an undefined start
/// is -infinity, an undefined end +infinity.
#[derive(Clone, Copy)]
enum P {
    Val(H),
    NegInf,
    PosInf,
}

struct RC<'a> {
    rt: &'a Runtime,
    err: bool,
}

impl RC<'_> {
    fn c(&mut self, a: P, b: P) -> Ordering {
        match (a, b) {
            (P::NegInf, P::NegInf) | (P::PosInf, P::PosInf) => Ordering::Equal,
            (P::NegInf, _) | (_, P::PosInf) => Ordering::Less,
            (P::PosInf, _) | (_, P::NegInf) => Ordering::Greater,
            (P::Val(x), P::Val(y)) => match ops::cmp(self.rt, x, y) {
                Some(o) => o,
                None => {
                    self.err = true;
                    Ordering::Equal
                }
            },
        }
    }
    fn lt(&mut self, a: P, b: P) -> bool {
        self.c(a, b) == Ordering::Less
    }
    fn gt(&mut self, a: P, b: P) -> bool {
        self.c(a, b) == Ordering::Greater
    }
    fn eq(&mut self, a: P, b: P) -> bool {
        self.c(a, b) == Ordering::Equal
    }
}

fn st(r: &R) -> P {
    r.s.map(P::Val).unwrap_or(P::NegInf)
}
fn en(r: &R) -> P {
    r.e.map(P::Val).unwrap_or(P::PosInf)
}

enum Arg {
    Point(H),
    Range(R),
}

fn classify(cx: &mut dyn Cx, h: H) -> Option<Arg> {
    if cx.rt().is_null(h) {
        return None;
    }
    Some(match as_range(cx.rt(), h) {
        Some(r) => Arg::Range(r),
        None => Arg::Point(h),
    })
}

fn range_fn(cx: &mut dyn Cx, a: &[H], site: u32, name: &str, f: fn(&mut RC, Arg, Arg) -> Option<bool>) -> H {
    let (Some(x), Some(y)) = (classify(cx, a[0]), classify(cx, a[1])) else {
        return bad(cx, site, name, "arguments must not be null");
    };
    let rt = cx.rt();
    let mut rc = RC { rt, err: false };
    let r = f(&mut rc, x, y);
    let err = rc.err;
    match r {
        Some(b) if !err => cx.rt().bool(b),
        _ => bad(cx, site, name, "arguments are not comparable or not of the expected kinds"),
    }
}

fn bi_before(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "before", |c, x, y| match (x, y) {
        (Arg::Point(p), Arg::Point(q)) => Some(c.lt(P::Val(p), P::Val(q))),
        (Arg::Point(p), Arg::Range(r)) => Some(c.lt(P::Val(p), st(&r)) || (c.eq(P::Val(p), st(&r)) && !r.si)),
        (Arg::Range(r), Arg::Point(p)) => Some(c.lt(en(&r), P::Val(p)) || (c.eq(en(&r), P::Val(p)) && !r.ei)),
        (Arg::Range(r1), Arg::Range(r2)) => {
            Some(c.lt(en(&r1), st(&r2)) || ((!r1.ei || !r2.si) && c.eq(en(&r1), st(&r2))))
        }
    })
}

fn bi_after(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "after", |c, x, y| match (x, y) {
        (Arg::Point(p), Arg::Point(q)) => Some(c.gt(P::Val(p), P::Val(q))),
        (Arg::Point(p), Arg::Range(r)) => Some(c.gt(P::Val(p), en(&r)) || (c.eq(P::Val(p), en(&r)) && !r.ei)),
        (Arg::Range(r), Arg::Point(p)) => Some(c.gt(st(&r), P::Val(p)) || (c.eq(st(&r), P::Val(p)) && !r.si)),
        (Arg::Range(r1), Arg::Range(r2)) => {
            Some(c.gt(st(&r1), en(&r2)) || ((!r1.si || !r2.ei) && c.eq(st(&r1), en(&r2))))
        }
    })
}

fn rr(x: Arg, y: Arg) -> Option<(R, R)> {
    match (x, y) {
        (Arg::Range(a), Arg::Range(b)) => Some((a, b)),
        _ => None,
    }
}

fn bi_meets(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "meets", |c, x, y| {
        let (r1, r2) = rr(x, y)?;
        Some(r1.ei && r2.si && c.eq(en(&r1), st(&r2)))
    })
}
fn bi_met_by(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "met by", |c, x, y| {
        let (r1, r2) = rr(x, y)?;
        Some(r1.si && r2.ei && c.eq(st(&r1), en(&r2)))
    })
}
fn bi_overlaps(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "overlaps", |c, x, y| {
        let (r1, r2) = rr(x, y)?;
        Some(
            (c.gt(en(&r1), st(&r2)) || (c.eq(en(&r1), st(&r2)) && r1.ei && r2.si))
                && (c.lt(st(&r1), en(&r2)) || (c.eq(st(&r1), en(&r2)) && r1.si && r2.ei)),
        )
    })
}
fn bi_overlaps_before(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "overlaps before", |c, x, y| {
        let (r1, r2) = rr(x, y)?;
        Some(
            (c.lt(st(&r1), st(&r2)) || (c.eq(st(&r1), st(&r2)) && r1.si && !r2.si))
                && (c.gt(en(&r1), st(&r2)) || (c.eq(en(&r1), st(&r2)) && r1.ei && r2.si))
                && (c.lt(en(&r1), en(&r2)) || (c.eq(en(&r1), en(&r2)) && (!r1.ei || r2.ei))),
        )
    })
}
fn bi_overlaps_after(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "overlaps after", |c, x, y| {
        let (r1, r2) = rr(x, y)?;
        Some(
            (c.lt(st(&r2), st(&r1)) || (c.eq(st(&r2), st(&r1)) && r2.si && !r1.si))
                && (c.gt(en(&r2), st(&r1)) || (c.eq(en(&r2), st(&r1)) && r2.ei && r1.si))
                && (c.lt(en(&r2), en(&r1)) || (c.eq(en(&r2), en(&r1)) && (!r2.ei || r1.ei))),
        )
    })
}
fn bi_finishes(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "finishes", |c, x, y| match (x, y) {
        (Arg::Point(p), Arg::Range(r)) => Some(r.ei && c.eq(en(&r), P::Val(p))),
        (Arg::Range(r1), Arg::Range(r2)) => Some(
            r1.ei == r2.ei
                && c.eq(en(&r1), en(&r2))
                && (c.gt(st(&r1), st(&r2)) || (c.eq(st(&r1), st(&r2)) && (!r1.si || r2.si))),
        ),
        _ => None,
    })
}
fn bi_finished_by(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "finished by", |c, x, y| match (x, y) {
        (Arg::Range(r), Arg::Point(p)) => Some(r.ei && c.eq(en(&r), P::Val(p))),
        (Arg::Range(r1), Arg::Range(r2)) => Some(
            r1.ei == r2.ei
                && c.eq(en(&r1), en(&r2))
                && (c.lt(st(&r1), st(&r2)) || (c.eq(st(&r1), st(&r2)) && (r1.si || !r2.si))),
        ),
        _ => None,
    })
}
fn includes_point(c: &mut RC, r: &R, p: H) -> bool {
    (c.lt(st(r), P::Val(p)) && c.gt(en(r), P::Val(p)))
        || (c.eq(st(r), P::Val(p)) && r.si)
        || (c.eq(en(r), P::Val(p)) && r.ei)
}
fn includes_range(c: &mut RC, r1: &R, r2: &R) -> bool {
    (c.lt(st(r1), st(r2)) || (c.eq(st(r1), st(r2)) && (r1.si || !r2.si)))
        && (c.gt(en(r1), en(r2)) || (c.eq(en(r1), en(r2)) && (r1.ei || !r2.ei)))
}
fn bi_includes(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "includes", |c, x, y| match (x, y) {
        (Arg::Range(r), Arg::Point(p)) => Some(includes_point(c, &r, p)),
        (Arg::Range(r1), Arg::Range(r2)) => Some(includes_range(c, &r1, &r2)),
        _ => None,
    })
}
fn bi_during(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "during", |c, x, y| match (x, y) {
        (Arg::Point(p), Arg::Range(r)) => Some(includes_point(c, &r, p)),
        (Arg::Range(r1), Arg::Range(r2)) => Some(includes_range(c, &r2, &r1)),
        _ => None,
    })
}
fn bi_starts(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "starts", |c, x, y| match (x, y) {
        (Arg::Point(p), Arg::Range(r)) => Some(r.si && c.eq(st(&r), P::Val(p))),
        (Arg::Range(r1), Arg::Range(r2)) => Some(
            c.eq(st(&r1), st(&r2))
                && r1.si == r2.si
                && (c.lt(en(&r1), en(&r2)) || (c.eq(en(&r1), en(&r2)) && (!r1.ei || r2.ei))),
        ),
        _ => None,
    })
}
fn bi_started_by(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "started by", |c, x, y| match (x, y) {
        (Arg::Range(r), Arg::Point(p)) => Some(r.si && c.eq(st(&r), P::Val(p))),
        (Arg::Range(r1), Arg::Range(r2)) => Some(
            c.eq(st(&r1), st(&r2))
                && r1.si == r2.si
                && (c.lt(en(&r2), en(&r1)) || (c.eq(en(&r2), en(&r1)) && (!r2.ei || r1.ei))),
        ),
        _ => None,
    })
}
fn bi_coincides(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    range_fn(cx, a, site, "coincides", |c, x, y| match (x, y) {
        (Arg::Point(p), Arg::Point(q)) => Some(c.eq(P::Val(p), P::Val(q))),
        (Arg::Range(r1), Arg::Range(r2)) => {
            Some(c.eq(st(&r1), st(&r2)) && r1.si == r2.si && c.eq(en(&r1), en(&r2)) && r1.ei == r2.ei)
        }
        _ => None,
    })
}

// ------------------------------------------------------------------ temporal

fn date_arg(cx: &mut dyn Cx, h: H) -> Option<tm::Date> {
    match cx.rt().get(h) {
        Value::Date(d) => Some(*d),
        Value::DateTime(dt) => Some(dt.date),
        _ => None,
    }
}

fn bi_day_of_year(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match date_arg(cx, a[0]) {
        Some(d) => cx.rt().int(d.day_of_year() as i64),
        None => bad(cx, site, "day of year", "date expected"),
    }
}
fn bi_day_of_week(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match date_arg(cx, a[0]) {
        Some(d) => cx.rt().str(tm::weekday_name(d.weekday())),
        None => bad(cx, site, "day of week", "date expected"),
    }
}
fn bi_month_of_year(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match date_arg(cx, a[0]) {
        Some(d) => cx.rt().str(tm::month_name(d.month())),
        None => bad(cx, site, "month of year", "date expected"),
    }
}
fn bi_week_of_year(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    match date_arg(cx, a[0]) {
        Some(d) => cx.rt().int(d.iso_week() as i64),
        None => bad(cx, site, "week of year", "date expected"),
    }
}

// ------------------------------------------------------------------ sort

fn bi_sort(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(items) = list(cx, a[0]) else { return bad(cx, site, "sort", "list expected") };
    let f = a.get(1).copied().filter(|h| !cx.rt().is_null(*h));
    if let Some(f) = f {
        if !matches!(cx.rt().get(f), Value::Func(_)) {
            return bad(cx, site, "sort", "precedes must be a function");
        }
    }
    // stable merge sort; `precedes(x, y)` true means x sorts before y
    let mut v = items;
    let mut err = false;
    let mut buf = v.clone();
    let n = v.len();
    let mut width = 1;
    while width < n {
        let mut i = 0;
        while i < n {
            let mid = (i + width).min(n);
            let hi = (i + 2 * width).min(n);
            let (mut l, mut r, mut k) = (i, mid, i);
            while l < mid && r < hi {
                // take from the right only if right strictly precedes left
                let take_right = match f {
                    Some(f) => {
                        let args = cx.rt().list(vec![v[r], v[l]]);
                        let res = call_value(cx, f, args, false, site);
                        match cx.rt().as_bool(res) {
                            Some(b) => b,
                            None => {
                                err = true;
                                false
                            }
                        }
                    }
                    None => match ops::cmp(cx.rt(), v[r], v[l]) {
                        Some(o) => o == Ordering::Less,
                        None => {
                            err = true;
                            false
                        }
                    },
                };
                if cx.rt().aborted() {
                    return NULL;
                }
                if take_right {
                    buf[k] = v[r];
                    r += 1;
                } else {
                    buf[k] = v[l];
                    l += 1;
                }
                k += 1;
            }
            while l < mid {
                buf[k] = v[l];
                l += 1;
                k += 1;
            }
            while r < hi {
                buf[k] = v[r];
                r += 1;
                k += 1;
            }
            i += 2 * width;
        }
        std::mem::swap(&mut v, &mut buf);
        width *= 2;
    }
    if err {
        return bad(cx, site, "sort", "precedes did not return a boolean / items not comparable");
    }
    cx.rt().list(v)
}

// ------------------------------------------------------------------ context

fn bi_get_value(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(k) = s(cx, a[1]) else { return bad(cx, site, "get value", "key must be a string") };
    match cx.rt().get(a[0]) {
        Value::Ctx(c) => c.get(&k).unwrap_or(NULL),
        _ => {
            // properties of temporal values / ranges are also reachable
            let r = ops::select(cx.rt(), a[0], &k);
            if r == ABSENT {
                bad(cx, site, "get value", "context expected")
            } else {
                r
            }
        }
    }
}

fn bi_get_entries(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(c) = cx.rt().as_ctx(a[0]).cloned() else { return bad(cx, site, "get entries", "context expected") };
    let kk: Rc<str> = Rc::from("key");
    let vk: Rc<str> = Rc::from("value");
    let mut out = Vec::with_capacity(c.len());
    for (k, v) in c.keys.iter().zip(c.vals.iter()) {
        let kh = cx.rt().str(k);
        let e = Ctx { keys: vec![kk.clone(), vk.clone()], vals: vec![kh, *v] };
        out.push(cx.rt().ctx(e));
    }
    cx.rt().list(out)
}

fn bi_context(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let items = match cx.rt().get(a[0]) {
        Value::List(v) => v.clone(),
        Value::Ctx(_) => vec![a[0]],
        _ => return bad(cx, site, "context", "list of entries expected"),
    };
    let mut c = Ctx::default();
    for it in items {
        let Some(e) = cx.rt().as_ctx(it).cloned() else { return bad(cx, site, "context", "entries must be contexts") };
        let (Some(k), Some(v)) = (e.get("key"), e.get("value")) else {
            return bad(cx, site, "context", "entries must have 'key' and 'value'");
        };
        let Some(ks) = s(cx, k) else { return bad(cx, site, "context", "keys must be strings") };
        if c.get(&ks).is_some() {
            return bad(cx, site, "context", "duplicate key");
        }
        c.put(ks, v);
    }
    cx.rt().ctx(c)
}

fn context_put_path(cx: &mut dyn Cx, ctx: H, keys: &[Rc<str>], v: H) -> Option<H> {
    let mut c = cx.rt().as_ctx(ctx)?.clone();
    let (k, rest) = keys.split_first()?;
    let nv = if rest.is_empty() {
        v
    } else {
        let inner = c.get(k)?;
        context_put_path(cx, inner, rest, v)?
    };
    c.put(k.clone(), nv);
    Some(cx.rt().ctx(c))
}

fn bi_context_put(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let keys: Vec<Rc<str>> = match cx.rt().get(a[1]).clone() {
        Value::Str(k) => vec![k],
        Value::List(items) => {
            let mut ks = Vec::new();
            for it in items {
                match s(cx, it) {
                    Some(k) => ks.push(k),
                    None => return bad(cx, site, "context put", "keys must be strings"),
                }
            }
            ks
        }
        _ => return bad(cx, site, "context put", "key must be a string or list of strings"),
    };
    if keys.is_empty() {
        return bad(cx, site, "context put", "keys must not be empty");
    }
    match context_put_path(cx, a[0], &keys, a[2]) {
        Some(h) => h,
        None => bad(cx, site, "context put", "context expected along the key path"),
    }
}

fn bi_context_merge(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let items = match cx.rt().get(a[0]) {
        Value::List(v) => v.clone(),
        Value::Ctx(_) => vec![a[0]],
        _ => return bad(cx, site, "context merge", "list of contexts expected"),
    };
    let mut c = Ctx::default();
    for it in items {
        let Some(e) = cx.rt().as_ctx(it).cloned() else { return bad(cx, site, "context merge", "contexts expected") };
        for (k, v) in e.keys.iter().zip(e.vals.iter()) {
            c.put(k.clone(), *v);
        }
    }
    cx.rt().ctx(c)
}

fn bi_has_key(cx: &mut dyn Cx, a: &[H], site: u32) -> H {
    let Some(k) = s(cx, a[1]) else { return bad(cx, site, "has key", "key must be a string") };
    match cx.rt().as_ctx(a[0]) {
        Some(c) => {
            let b = c.get(&k).is_some();
            cx.rt().bool(b)
        }
        None => bad(cx, site, "has key", "context expected"),
    }
}

// ------------------------------------------------------------------ clock capability

fn bi_now(cx: &mut dyn Cx, _a: &[H], site: u32) -> H {
    match cx.rt().clock.clone() {
        Some(c) => cx.rt().alloc(Value::DateTime(c.now)),
        None => bad(cx, site, "now", "no clock capability was supplied by the host"),
    }
}

fn bi_today(cx: &mut dyn Cx, _a: &[H], site: u32) -> H {
    match cx.rt().clock.clone() {
        Some(c) => cx.rt().alloc(Value::Date(c.now.date)),
        None => bad(cx, site, "today", "no clock capability was supplied by the host"),
    }
}

#[allow(dead_code)]
fn unused(_: RtType) -> i32 {
    tri_of(None)
}
