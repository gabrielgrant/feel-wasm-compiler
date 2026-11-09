//! The versioned runtime ABI (`feel_rt_v1`) used by generated modules.
//!
//! Every function takes and returns `i32` values only (value handles, tri-state
//! booleans, counts, constant indices, source-site ids). The list below is the
//! single source of truth: the code generator declares imports from it, the
//! native host binds it to wasmi host functions, and `feel-runtime-wasm`
//! exports it from `runtime.wasm`. See `docs/ABI.md`.
//!
//! Conventions:
//! * handles: `0` null, `1` true, `2` false, `3..` constants, then evaluation
//!   values; `-1` (`ABSENT`) marks "no value" in lookups that distinguish a
//!   missing key from a null value.
//! * tri-state booleans: `0` false, `1` true, `2` null/other.
//! * `site`: index into the module's source-site table (spans) for diagnostics.
//! * failure: when a resource limit is exceeded or the ABI is misused, the
//!   runtime records an [`Abort`] and the binding layer traps immediately; the
//!   host reports the abort instead of a FEEL value.

use crate::builtins::{self, Cx};
use crate::ops;
use crate::value::*;

/// ABI version encoded in the import module name (`feel_rt_v1`) and in module
/// metadata. Bump on any incompatible change.
pub const ABI_VERSION: u32 = 1;
pub const ABI_MODULE: &str = "feel_rt_v1";
/// Import module for callbacks *from* the runtime into the host.
pub const HOST_MODULE: &str = "feel_host_v1";

/// Invokes `$m!` with the ABI function list: `name(params...) -> results;`
/// where results is `0` or `1`.
#[macro_export]
macro_rules! feel_abi_list {
    ($m:ident) => {
        $m! {
            slot(i) -> 1;
            truth(h) -> 1;
            bool_of(t) -> 1;
            add(a, b, site) -> 1;
            sub(a, b, site) -> 1;
            mul(a, b, site) -> 1;
            div(a, b, site) -> 1;
            pow(a, b, site) -> 1;
            neg(a, site) -> 1;
            eq(a, b, site) -> 1;
            ne(a, b, site) -> 1;
            lt(a, b, site) -> 1;
            le(a, b, site) -> 1;
            gt(a, b, site) -> 1;
            ge(a, b, site) -> 1;
            in_value(c, v, uses_q, site) -> 1;
            range(s, e, flags, site) -> 1;
            list_new(cap) -> 1;
            list_push(l, v) -> 0;
            list_len(l) -> 1;
            list_get(l, i) -> 1;
            list_snapshot(l) -> 1;
            ctx_new(cap) -> 1;
            ctx_put(c, k, v) -> 0;
            path(v, k, site) -> 1;
            select_or_null(v, k) -> 1;
            item_field(item, k) -> 1;
            index(l, n, site) -> 1;
            as_iter(v, site) -> 1;
            iter_range(a, b, site) -> 1;
            filter_list(v) -> 1;
            is_number(h) -> 1;
            make_closure(idx, env, sig) -> 1;
            fn_kind(f) -> 1;
            fn_index(f) -> 1;
            fn_env(f) -> 1;
            bind_args(f, args, named, site) -> 1;
            call_value(f, args, named, site) -> 1;
            call_builtin(id, args, site) -> 1;
            call_builtin_named(id, ctx, site) -> 1;
            builtin_fn(id) -> 1;
            host_fn(name, sig) -> 1;
            external_fn(mapping, sig) -> 1;
            instance_of(v, ty) -> 1;
            coerce(v, ty, site) -> 1;
            not_bool(h, site) -> 1;
            cond_diag(h, site) -> 0;
            fuel_exhausted() -> 0;
            depth_exceeded() -> 0;
        }
    };
}

macro_rules! count_list {
    ($($name:ident($($p:ident),*) -> $r:tt;)*) => {
        /// `(name, parameter count, result count)` for every ABI function.
        pub const ABI_FUNCS: &[(&str, usize, usize)] = &[
            $( (stringify!($name), count_list!(@n $($p)*), $r), )*
        ];
    };
    (@n) => { 0 };
    (@n $x:ident $($xs:ident)*) => { 1 + count_list!(@n $($xs)*) };
}
feel_abi_list!(count_list);

/// Field name of an ABI function in the `feel_rt_v1` import module.
pub fn import_name(name: &str) -> String {
    format!("f_{name}")
}

pub fn abi_index(name: &str) -> Option<usize> {
    ABI_FUNCS.iter().position(|f| f.0 == name)
}

#[inline]
fn h(x: i32) -> H {
    x as u32
}
#[inline]
fn r(x: H) -> i32 {
    x as i32
}

// ---------------------------------------------------------------- implementations
// Each takes the callback context first; results are i32 (0 for "void").

pub fn slot(cx: &mut dyn Cx, i: i32) -> i32 {
    let rt = cx.rt();
    match rt.slots.get(i as usize) {
        Some(v) => r(*v),
        None => {
            rt.boundary("FEEL-B502", format!("input slot {i} was not bound"));
            0
        }
    }
}
pub fn truth(cx: &mut dyn Cx, x: i32) -> i32 {
    ops::truth(cx.rt(), h(x))
}
pub fn bool_of(_cx: &mut dyn Cx, t: i32) -> i32 {
    r(ops::tri_to_h(t))
}
pub fn add(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    r(ops::add(cx.rt(), h(a), h(b), site as u32))
}
pub fn sub(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    r(ops::sub(cx.rt(), h(a), h(b), site as u32))
}
pub fn mul(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    r(ops::mul(cx.rt(), h(a), h(b), site as u32))
}
pub fn div(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    r(ops::div(cx.rt(), h(a), h(b), site as u32))
}
pub fn pow(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    r(ops::pow(cx.rt(), h(a), h(b), site as u32))
}
pub fn neg(cx: &mut dyn Cx, a: i32, site: i32) -> i32 {
    r(ops::neg(cx.rt(), h(a), site as u32))
}
pub fn eq(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    ops::eq_tri(cx.rt(), h(a), h(b), site as u32)
}
pub fn ne(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    ops::tri_not(ops::eq_tri(cx.rt(), h(a), h(b), site as u32))
}
pub fn lt(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    ops::lt(cx.rt(), h(a), h(b), site as u32)
}
pub fn le(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    ops::le(cx.rt(), h(a), h(b), site as u32)
}
pub fn gt(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    ops::gt(cx.rt(), h(a), h(b), site as u32)
}
pub fn ge(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    ops::ge(cx.rt(), h(a), h(b), site as u32)
}
pub fn in_value(cx: &mut dyn Cx, c: i32, v: i32, uses_q: i32, site: i32) -> i32 {
    let rt = cx.rt();
    let t = ops::in_value(rt, h(c), h(v), uses_q != 0, site as u32);
    if t == ops::T_NULL && rt.bfeel() {
        return ops::T_FALSE;
    }
    t
}
pub fn range(cx: &mut dyn Cx, s: i32, e: i32, flags: i32, site: i32) -> i32 {
    r(ops::make_range(cx.rt(), h(s), h(e), flags, site as u32))
}
pub fn list_new(cx: &mut dyn Cx, cap: i32) -> i32 {
    let v = Vec::with_capacity((cap.max(0) as usize).min(1024));
    let rt = cx.rt();
    if rt.aborted() {
        return 0;
    }
    r(rt.alloc(Value::List(v)))
}
pub fn list_push(cx: &mut dyn Cx, l: i32, v: i32) -> i32 {
    cx.rt().list_push(h(l), h(v));
    0
}
pub fn list_len(cx: &mut dyn Cx, l: i32) -> i32 {
    match cx.rt().as_list(h(l)) {
        Some(v) => v.len() as i32,
        None => -1,
    }
}
pub fn list_get(cx: &mut dyn Cx, l: i32, i: i32) -> i32 {
    match cx.rt().as_list(h(l)) {
        Some(v) => v.get(i as usize).map(|x| r(*x)).unwrap_or(0),
        None => 0,
    }
}
pub fn list_snapshot(cx: &mut dyn Cx, l: i32) -> i32 {
    r(ops::list_snapshot(cx.rt(), h(l)))
}
pub fn ctx_new(cx: &mut dyn Cx, cap: i32) -> i32 {
    let c = Ctx {
        keys: Vec::with_capacity((cap.max(0) as usize).min(256)),
        vals: Vec::with_capacity((cap.max(0) as usize).min(256)),
    };
    let rt = cx.rt();
    if rt.aborted() {
        return 0;
    }
    r(rt.alloc(Value::Ctx(c)))
}
pub fn ctx_put(cx: &mut dyn Cx, c: i32, k: i32, v: i32) -> i32 {
    cx.rt().ctx_put(h(c), h(k), h(v));
    0
}
pub fn path(cx: &mut dyn Cx, v: i32, k: i32, site: i32) -> i32 {
    r(ops::path(cx.rt(), h(v), h(k), site as u32))
}
pub fn select_or_null(cx: &mut dyn Cx, v: i32, k: i32) -> i32 {
    r(ops::select_or_null(cx.rt(), h(v), h(k)))
}
pub fn item_field(cx: &mut dyn Cx, item: i32, k: i32) -> i32 {
    r(ops::item_field(cx.rt(), h(item), h(k)))
}
pub fn index(cx: &mut dyn Cx, l: i32, n: i32, site: i32) -> i32 {
    r(ops::index(cx.rt(), h(l), h(n), site as u32))
}
pub fn as_iter(cx: &mut dyn Cx, v: i32, site: i32) -> i32 {
    r(ops::as_iter(cx.rt(), h(v), site as u32))
}
pub fn iter_range(cx: &mut dyn Cx, a: i32, b: i32, site: i32) -> i32 {
    r(ops::iter_range(cx.rt(), h(a), h(b), site as u32))
}
pub fn filter_list(cx: &mut dyn Cx, v: i32) -> i32 {
    r(ops::as_filter_list(cx.rt(), h(v)))
}
pub fn is_number(cx: &mut dyn Cx, x: i32) -> i32 {
    matches!(cx.rt().get(h(x)), Value::Num(_)) as i32
}
pub fn make_closure(cx: &mut dyn Cx, idx: i32, env: i32, sig: i32) -> i32 {
    r(cx.rt().alloc(Value::Func(Func::User {
        index: idx as u32,
        env: h(env),
        sig: h(sig),
    })))
}
pub fn fn_kind(cx: &mut dyn Cx, f: i32) -> i32 {
    match cx.rt().get(h(f)) {
        Value::Func(Func::Builtin(_)) => 1,
        Value::Func(Func::User { .. }) => 2,
        Value::Func(_) => 3,
        _ => 0,
    }
}
pub fn fn_index(cx: &mut dyn Cx, f: i32) -> i32 {
    match cx.rt().get(h(f)) {
        Value::Func(Func::User { index, .. }) => *index as i32,
        _ => -1,
    }
}
pub fn fn_env(cx: &mut dyn Cx, f: i32) -> i32 {
    match cx.rt().get(h(f)) {
        Value::Func(Func::User { env, .. }) => r(*env),
        _ => 0,
    }
}
pub fn bind_args(cx: &mut dyn Cx, f: i32, args: i32, named: i32, site: i32) -> i32 {
    r(builtins::bind_args(
        cx.rt(),
        h(f),
        h(args),
        named != 0,
        site as u32,
    ))
}
pub fn call_value(cx: &mut dyn Cx, f: i32, args: i32, named: i32, site: i32) -> i32 {
    r(builtins::call_value(
        cx,
        h(f),
        h(args),
        named != 0,
        site as u32,
    ))
}
pub fn call_builtin(cx: &mut dyn Cx, id: i32, args: i32, site: i32) -> i32 {
    let a = cx.rt().list_items(h(args)).unwrap_or_default();
    r(builtins::call(cx, id as u16, &a, site as u32))
}
pub fn call_builtin_named(cx: &mut dyn Cx, id: i32, c: i32, site: i32) -> i32 {
    r(builtins::call_named(cx, id as u16, h(c), site as u32))
}
pub fn builtin_fn(cx: &mut dyn Cx, id: i32) -> i32 {
    r(cx.rt().alloc(Value::Func(Func::Builtin(id as u16))))
}
pub fn host_fn(cx: &mut dyn Cx, name: i32, sig: i32) -> i32 {
    let rt = cx.rt();
    let n = rt.as_str(h(name)).unwrap_or_else(|| std::rc::Rc::from(""));
    r(rt.alloc(Value::Func(Func::Host {
        name: n,
        sig: h(sig),
    })))
}
pub fn external_fn(cx: &mut dyn Cx, mapping: i32, sig: i32) -> i32 {
    r(cx.rt().alloc(Value::Func(Func::External {
        mapping: h(mapping),
        sig: h(sig),
    })))
}
pub fn instance_of(cx: &mut dyn Cx, v: i32, ty: i32) -> i32 {
    let rt = cx.rt();
    let t = rt.type_of_const(h(ty));
    let r = crate::types::instance_of(rt, h(v), &t);
    if r.is_none() && rt.bfeel() {
        return ops::T_FALSE;
    }
    ops::tri_of(r)
}
pub fn coerce(cx: &mut dyn Cx, v: i32, ty: i32, site: i32) -> i32 {
    let rt = cx.rt();
    let t = rt.type_of_const(h(ty));
    r(crate::types::coerce(rt, h(v), &t, site as u32))
}
/// `not(x)` on a value handle, returning a tri-state.
pub fn not_bool(cx: &mut dyn Cx, x: i32, site: i32) -> i32 {
    let rt = cx.rt();
    if rt.bfeel() && !matches!(rt.get(h(x)), Value::Bool(_)) {
        return ops::T_FALSE;
    }
    let t = ops::truth(rt, h(x));
    if t == ops::T_NULL && !rt.is_null(h(x)) {
        let k = rt.kind_name(h(x));
        rt.diag(
            "FEEL-E341",
            site as u32,
            format!("not() of a {k} yields null"),
        );
    }
    ops::tri_not(t)
}
/// Diagnostic for a non-boolean condition (if/filter/quantifier).
pub fn cond_diag(cx: &mut dyn Cx, x: i32, site: i32) -> i32 {
    let rt = cx.rt();
    let k = rt.kind_name(h(x));
    rt.diag(
        "FEEL-E342",
        site as u32,
        format!("condition is {k}, not a boolean; treated as false"),
    );
    0
}
pub fn fuel_exhausted(cx: &mut dyn Cx) -> i32 {
    let rt = cx.rt();
    let n = rt.limits.max_steps;
    rt.limit("FEEL-L404", format!("step limit exceeded ({n} steps)"));
    0
}
pub fn depth_exceeded(cx: &mut dyn Cx) -> i32 {
    let rt = cx.rt();
    let n = rt.limits.max_depth;
    rt.limit("FEEL-L405", format!("call depth limit exceeded ({n})"));
    0
}

// ---------------------------------------------------------------- host-side helpers

/// Bind input slots from an input context. `names` is the module's slot-name
/// list, `types` the optional declared types (strings or null) used for
/// boundary validation. `?` binds the unary-test candidate.
pub fn bind_inputs(
    rt: &mut Runtime,
    inputs: H,
    candidate: Option<H>,
    names: &[String],
    types: &[Option<String>],
    validate: bool,
) -> Result<(), String> {
    // constant 2 holds the "quiet" flags (slots only used as implicit filter
    // names are not reported when missing)
    let quiet: Vec<bool> = rt
        .list_items(crate::value::RESERVED + 2)
        .unwrap_or_default()
        .iter()
        .map(|h| *h == crate::value::TRUE)
        .collect();
    rt.slots.clear();
    let ctx = rt.as_ctx(inputs).cloned();
    if ctx.is_none() && !rt.is_null(inputs) {
        return Err("inputs must be a context (JSON object)".to_string());
    }
    for (i, n) in names.iter().enumerate() {
        let v = if n == "?" {
            candidate.unwrap_or(NULL)
        } else {
            match ctx.as_ref().and_then(|c| c.get(n)) {
                Some(v) => v,
                None => {
                    if !quiet.get(i).copied().unwrap_or(false) {
                        rt.diag(
                            "FEEL-E350",
                            0,
                            format!("input '{n}' was not provided; it evaluates to null"),
                        );
                    }
                    NULL
                }
            }
        };
        if validate {
            if let Some(Some(t)) = types.get(i) {
                let ty = crate::types::parse_type(t);
                if crate::types::value_conforms(rt, v, &ty) == Some(false) {
                    let k = rt.kind_name(v);
                    return Err(format!(
                        "input '{n}' of type {k} does not conform to the declared type {ty}"
                    ));
                }
            }
        }
        rt.slots.push(v);
    }
    Ok(())
}

/// Encode diagnostics as JSON `[{"code":..,"site":..,"message":..}]`.
pub fn diags_json(rt: &Runtime) -> String {
    let mut s = String::from("[");
    for (i, d) in rt.diags.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"code\":");
        crate::json::write_json_string(&mut s, d.code);
        s.push_str(&format!(",\"site\":{},\"message\":", d.site));
        crate::json::write_json_string(&mut s, &d.message);
        s.push('}');
    }
    s.push(']');
    s
}

pub fn abort_json(rt: &Runtime) -> String {
    match &rt.abort {
        None => "null".to_string(),
        Some(a) => {
            let kind = match a {
                Abort::Limit { .. } => "limit",
                Abort::Boundary { .. } => "boundary",
            };
            let mut s = format!("{{\"kind\":\"{kind}\",\"code\":");
            crate::json::write_json_string(&mut s, a.code());
            s.push_str(",\"message\":");
            crate::json::write_json_string(&mut s, a.message());
            s.push('}');
            s
        }
    }
}

/// Parse a clock instant (`date and time` lexical form).
pub fn set_clock(rt: &mut Runtime, s: &str) -> bool {
    match feel_temporal::DateTime::parse(s) {
        Some(dt) => {
            rt.clock = Some(Clock { now: dt });
            true
        }
        None => false,
    }
}
