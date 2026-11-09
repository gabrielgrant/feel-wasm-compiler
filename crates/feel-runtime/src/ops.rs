//! FEEL operators (DMN 1.5 Tables 50-62, 64-68) on arena values.
//!
//! Three-valued results are returned as "tri" codes: 0 = false, 1 = true,
//! 2 = any other value (i.e. FEEL `null` in boolean position).

use crate::value::*;
use feel_decimal::{Decimal, RoundingMode};
use feel_temporal as tm;
use std::cmp::Ordering;
use std::rc::Rc;

pub const T_FALSE: i32 = 0;
pub const T_TRUE: i32 = 1;
pub const T_NULL: i32 = 2;

#[inline]
pub fn tri_of(b: Option<bool>) -> i32 {
    match b {
        Some(true) => T_TRUE,
        Some(false) => T_FALSE,
        None => T_NULL,
    }
}

#[inline]
pub fn tri_to_h(t: i32) -> H {
    match t {
        T_TRUE => TRUE,
        T_FALSE => FALSE,
        _ => NULL,
    }
}

#[inline]
pub fn truth(rt: &Runtime, h: H) -> i32 {
    match h {
        TRUE => T_TRUE,
        FALSE => T_FALSE,
        _ => match rt.get(h) {
            Value::Bool(true) => T_TRUE,
            Value::Bool(false) => T_FALSE,
            _ => T_NULL,
        },
    }
}

pub fn tri_and(a: i32, b: i32) -> i32 {
    match (a, b) {
        (T_FALSE, _) | (_, T_FALSE) => T_FALSE,
        (T_TRUE, T_TRUE) => T_TRUE,
        _ => T_NULL,
    }
}
pub fn tri_or(a: i32, b: i32) -> i32 {
    match (a, b) {
        (T_TRUE, _) | (_, T_TRUE) => T_TRUE,
        (T_FALSE, T_FALSE) => T_FALSE,
        _ => T_NULL,
    }
}
pub fn tri_not(a: i32) -> i32 {
    match a {
        T_TRUE => T_FALSE,
        T_FALSE => T_TRUE,
        _ => T_NULL,
    }
}

// -------------------------------------------------------------------- helpers

fn dur_nanos_dec(d: &tm::DayTimeDuration) -> Option<Decimal> {
    Decimal::from_i128(d.nanos)
}

fn dec_to_nanos(d: &Decimal) -> Option<i128> {
    d.set_scale(0, RoundingMode::HalfEven)
        .and_then(|x| x.to_i128())
}

/// Implicit conversion date → date and time (10.3.2.3.5): UTC midnight.
pub fn date_to_datetime(d: &tm::Date) -> tm::DateTime {
    tm::DateTime::from_date_utc(d)
}

fn op_err(rt: &mut Runtime, site: u32, op: &str, a: H, b: H) -> H {
    let (ka, kb) = (rt.kind_name(a), rt.kind_name(b));
    if ka == "null" || kb == "null" {
        rt.diag(
            "FEEL-E301",
            site,
            format!("'{op}' with a null operand ({ka}, {kb}) yields null"),
        );
    } else {
        rt.diag(
            "FEEL-E302",
            site,
            format!("'{op}' is not defined for {ka} and {kb}"),
        );
    }
    NULL
}

fn overflow(rt: &mut Runtime, site: u32, what: &str) -> H {
    rt.diag(
        "FEEL-E303",
        site,
        format!("{what}: result outside the FEEL number range"),
    );
    NULL
}

// -------------------------------------------------------------------- arithmetic

pub fn add(rt: &mut Runtime, a: H, b: H, site: u32) -> H {
    use Value::*;
    let r = match (rt.get(a), rt.get(b)) {
        (Num(x), Num(y)) => match x.add(y) {
            Some(d) => return rt.num(d),
            None => return overflow(rt, site, "addition"),
        },
        (Str(x), Str(y)) => {
            let mut s = String::with_capacity(x.len() + y.len());
            s.push_str(x);
            s.push_str(y);
            return rt.string(s);
        }
        (Ymd(x), Ymd(y)) => x
            .months
            .checked_add(y.months)
            .map(|m| Ymd(tm::YearMonthDuration { months: m })),
        (Dtd(x), Dtd(y)) => x
            .nanos
            .checked_add(y.nanos)
            .map(|n| Dtd(tm::DayTimeDuration { nanos: n })),
        (DateTime(x), Ymd(y)) | (Ymd(y), DateTime(x)) => tm::datetime_add_ymd(x, y).map(DateTime),
        (DateTime(x), Dtd(y)) | (Dtd(y), DateTime(x)) => tm::datetime_add_dtd(x, y).map(DateTime),
        (Time(x), Dtd(y)) | (Dtd(y), Time(x)) => tm::time_add_dtd(x, y).map(Time),
        (Date(x), Ymd(y)) | (Ymd(y), Date(x)) => tm::date_add_ymd(x, y).map(Date),
        (Date(x), Dtd(y)) | (Dtd(y), Date(x)) => tm::date_add_dtd(x, y).map(Date),
        _ => return op_err(rt, site, "+", a, b),
    };
    match r {
        Some(v) => rt.alloc(v),
        None => {
            rt.diag("FEEL-E304", site, "temporal addition out of range");
            NULL
        }
    }
}

pub fn sub(rt: &mut Runtime, a: H, b: H, site: u32) -> H {
    use Value::*;
    let r = match (rt.get(a), rt.get(b)) {
        (Num(x), Num(y)) => match x.sub(y) {
            Some(d) => return rt.num(d),
            None => return overflow(rt, site, "subtraction"),
        },
        (Ymd(x), Ymd(y)) => x
            .months
            .checked_sub(y.months)
            .map(|m| Ymd(tm::YearMonthDuration { months: m })),
        (Dtd(x), Dtd(y)) => x
            .nanos
            .checked_sub(y.nanos)
            .map(|n| Dtd(tm::DayTimeDuration { nanos: n })),
        (DateTime(x), DateTime(y)) => tm::datetime_sub(x, y).map(Dtd),
        (Date(x), Date(y)) => Some(Dtd(tm::date_sub(x, y))),
        (Date(x), DateTime(y)) => tm::date_sub_datetime(x, y).map(Dtd),
        (DateTime(x), Date(y)) => tm::datetime_sub_date(x, y).map(Dtd),
        (Time(x), Time(y)) => tm::time_sub(x, y).map(Dtd),
        (DateTime(x), Ymd(y)) => {
            tm::datetime_add_ymd(x, &tm::YearMonthDuration { months: -y.months }).map(DateTime)
        }
        (DateTime(x), Dtd(y)) => {
            tm::datetime_add_dtd(x, &tm::DayTimeDuration { nanos: -y.nanos }).map(DateTime)
        }
        (Time(x), Dtd(y)) => {
            tm::time_add_dtd(x, &tm::DayTimeDuration { nanos: -y.nanos }).map(Time)
        }
        (Date(x), Ymd(y)) => {
            tm::date_add_ymd(x, &tm::YearMonthDuration { months: -y.months }).map(Date)
        }
        (Date(x), Dtd(y)) => {
            tm::date_add_dtd(x, &tm::DayTimeDuration { nanos: -y.nanos }).map(Date)
        }
        _ => return op_err(rt, site, "-", a, b),
    };
    match r {
        Some(v) => rt.alloc(v),
        None => {
            rt.diag(
                "FEEL-E304",
                site,
                "temporal subtraction undefined or out of range",
            );
            NULL
        }
    }
}

fn ymd_scale(m: i64, d: &Decimal) -> Option<i64> {
    Decimal::from_i64(m)
        .mul(d)
        .map(|x| x.trunc())
        .and_then(|x| x.to_i64())
}

fn dtd_scale(n: i128, d: &Decimal) -> Option<i128> {
    Decimal::from_i128(n)
        .and_then(|x| x.mul(d))
        .and_then(|x| dec_to_nanos(&x))
}

pub fn mul(rt: &mut Runtime, a: H, b: H, site: u32) -> H {
    use Value::*;
    let r = match (rt.get(a), rt.get(b)) {
        (Num(x), Num(y)) => match x.mul(y) {
            Some(d) => return rt.num(d),
            None => return overflow(rt, site, "multiplication"),
        },
        (Ymd(x), Num(y)) | (Num(y), Ymd(x)) => {
            ymd_scale(x.months, y).map(|m| Ymd(tm::YearMonthDuration { months: m }))
        }
        (Dtd(x), Num(y)) | (Num(y), Dtd(x)) => {
            dtd_scale(x.nanos, y).map(|n| Dtd(tm::DayTimeDuration { nanos: n }))
        }
        _ => return op_err(rt, site, "*", a, b),
    };
    match r {
        Some(v) => rt.alloc(v),
        None => {
            rt.diag("FEEL-E304", site, "duration multiplication out of range");
            NULL
        }
    }
}

pub fn div(rt: &mut Runtime, a: H, b: H, site: u32) -> H {
    use Value::*;
    let r = match (rt.get(a), rt.get(b)) {
        (Num(x), Num(y)) => {
            if y.is_zero() {
                rt.diag("FEEL-E305", site, "division by zero");
                return NULL;
            }
            match x.div(y) {
                Some(d) => return rt.num(d),
                None => return overflow(rt, site, "division"),
            }
        }
        (Ymd(x), Num(y)) => {
            if y.is_zero() {
                rt.diag("FEEL-E305", site, "division by zero");
                return NULL;
            }
            Decimal::from_i64(x.months)
                .div(y)
                .map(|q| q.trunc())
                .and_then(|q| q.to_i64())
                .map(|m| Ymd(tm::YearMonthDuration { months: m }))
        }
        (Ymd(x), Ymd(y)) => {
            if y.months == 0 {
                rt.diag("FEEL-E305", site, "division by a zero duration");
                return NULL;
            }
            let q = Decimal::from_i64(x.months).div(&Decimal::from_i64(y.months));
            return rt.num_opt(q);
        }
        (Dtd(x), Num(y)) => {
            if y.is_zero() {
                rt.diag("FEEL-E305", site, "division by zero");
                return NULL;
            }
            dur_nanos_dec(x)
                .and_then(|n| n.div(y))
                .and_then(|q| dec_to_nanos(&q))
                .map(|n| Dtd(tm::DayTimeDuration { nanos: n }))
        }
        (Dtd(x), Dtd(y)) => {
            if y.nanos == 0 {
                rt.diag("FEEL-E305", site, "division by a zero duration");
                return NULL;
            }
            let q = dur_nanos_dec(x).and_then(|a| dur_nanos_dec(y).and_then(|b| a.div(&b)));
            return rt.num_opt(q);
        }
        _ => return op_err(rt, site, "/", a, b),
    };
    match r {
        Some(v) => rt.alloc(v),
        None => {
            rt.diag("FEEL-E304", site, "duration division out of range");
            NULL
        }
    }
}

pub fn pow(rt: &mut Runtime, a: H, b: H, site: u32) -> H {
    match (rt.get(a), rt.get(b)) {
        (Value::Num(x), Value::Num(y)) => {
            let lim = Decimal::from_i64(999_999_999);
            if y.abs() > lim {
                rt.diag(
                    "FEEL-E306",
                    site,
                    "exponent outside [-999,999,999..999,999,999]",
                );
                return NULL;
            }
            let (x, y) = (*x, *y);
            match x.pow(&y) {
                Some(d) => rt.num(d),
                None => {
                    rt.diag(
                        "FEEL-E303",
                        site,
                        "exponentiation result is not a FEEL number",
                    );
                    NULL
                }
            }
        }
        _ => op_err(rt, site, "**", a, b),
    }
}

pub fn neg(rt: &mut Runtime, a: H, site: u32) -> H {
    match rt.get(a) {
        Value::Num(x) => {
            let n = x.neg();
            rt.num(n)
        }
        Value::Dtd(d) => {
            let n = -d.nanos;
            rt.alloc(Value::Dtd(tm::DayTimeDuration { nanos: n }))
        }
        Value::Ymd(d) => {
            let m = -d.months;
            rt.alloc(Value::Ymd(tm::YearMonthDuration { months: m }))
        }
        Value::Null => {
            rt.diag("FEEL-E301", site, "negation of null yields null");
            NULL
        }
        _ => {
            let k = rt.kind_name(a);
            rt.diag(
                "FEEL-E302",
                site,
                format!("negation is not defined for {k}"),
            );
            NULL
        }
    }
}

// -------------------------------------------------------------------- equality / ordering

/// FEEL `=` (Tables 52, 53): `None` = null.
pub fn eq(rt: &Runtime, a: H, b: H) -> Option<bool> {
    use Value::*;
    if a == b && !matches!(rt.get(a), Func(_)) {
        return Some(true);
    }
    match (rt.get(a), rt.get(b)) {
        (Null, Null) => Some(true),
        (Null, _) | (_, Null) => Some(false),
        (Bool(x), Bool(y)) => Some(x == y),
        (Num(x), Num(y)) => Some(x == y),
        (Str(x), Str(y)) => Some(x == y),
        (Date(x), Date(y)) => Some(x == y),
        (DateTime(x), DateTime(y)) => tm::eq_datetime(x, y),
        (Time(x), Time(y)) => tm::eq_time(x, y),
        (Dtd(x), Dtd(y)) => Some(x.nanos == y.nanos),
        (Ymd(x), Ymd(y)) => Some(x.months == y.months),
        (List(x), List(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            let mut res = Some(true);
            for (p, q) in x.iter().zip(y.iter()) {
                match eq(rt, *p, *q) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => res = None,
                }
            }
            res
        }
        (Ctx(x), Ctx(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            let mut res = Some(true);
            for (k, v) in x.keys.iter().zip(x.vals.iter()) {
                match y.get(k) {
                    None => return Some(false),
                    Some(w) => match eq(rt, *v, w) {
                        Some(true) => {}
                        Some(false) => return Some(false),
                        None => res = None,
                    },
                }
            }
            res
        }
        (Range(x), Range(y)) => {
            if x.start_incl != y.start_incl
                || x.end_incl != y.end_incl
                || x.ne != y.ne
                || x.eq != y.eq
            {
                return Some(false);
            }
            let ep = |p: Option<H>, q: Option<H>| match (p, q) {
                (None, None) => Some(true),
                (Some(p), Some(q)) => eq(rt, p, q),
                _ => Some(false),
            };
            match (ep(x.start, y.start), ep(x.end, y.end)) {
                (Some(true), Some(true)) => Some(true),
                (Some(false), _) | (_, Some(false)) => Some(false),
                _ => None,
            }
        }
        (Func(x), Func(y)) => Some(match (x, y) {
            (crate::value::Func::Builtin(p), crate::value::Func::Builtin(q)) => p == q,
            (
                crate::value::Func::User {
                    index: i, env: e, ..
                },
                crate::value::Func::User {
                    index: j, env: f, ..
                },
            ) => i == j && (e == f || eq(rt, *e, *f) == Some(true)),
            _ => a == b,
        }),
        _ => None,
    }
}

/// FEEL `<` family (Table 54): `None` = not comparable (null).
pub fn cmp(rt: &Runtime, a: H, b: H) -> Option<Ordering> {
    use Value::*;
    match (rt.get(a), rt.get(b)) {
        (Num(x), Num(y)) => Some(x.cmp(y)),
        (Str(x), Str(y)) => Some(x.as_ref().cmp(y.as_ref())),
        (Date(x), Date(y)) => Some(x.cmp(y)),
        (DateTime(x), DateTime(y)) => tm::cmp_datetime(x, y),
        (Date(x), DateTime(y)) => tm::cmp_datetime(&date_to_datetime(x), y),
        (DateTime(x), Date(y)) => tm::cmp_datetime(x, &date_to_datetime(y)),
        (Time(x), Time(y)) => tm::cmp_time(x, y),
        (Dtd(x), Dtd(y)) => Some(x.nanos.cmp(&y.nanos)),
        (Ymd(x), Ymd(y)) => Some(x.months.cmp(&y.months)),
        _ => None,
    }
}

pub fn cmp_diag(rt: &mut Runtime, a: H, b: H, op: &str, site: u32) {
    let (ka, kb) = (rt.kind_name(a), rt.kind_name(b));
    if ka == "null" || kb == "null" {
        rt.diag(
            "FEEL-E301",
            site,
            format!("'{op}' with a null operand yields null"),
        );
    } else {
        rt.diag(
            "FEEL-E307",
            site,
            format!("cannot compare {ka} with {kb} using '{op}'"),
        );
    }
}

pub fn lt(rt: &mut Runtime, a: H, b: H, site: u32) -> i32 {
    match cmp(rt, a, b) {
        Some(o) => tri_of(Some(o == Ordering::Less)),
        None => {
            cmp_diag(rt, a, b, "<", site);
            T_NULL
        }
    }
}
pub fn gt(rt: &mut Runtime, a: H, b: H, site: u32) -> i32 {
    match cmp(rt, a, b) {
        Some(o) => tri_of(Some(o == Ordering::Greater)),
        None => {
            cmp_diag(rt, a, b, ">", site);
            T_NULL
        }
    }
}
/// `e1 <= e2` is `e1 < e2 or e1 = e2`.
pub fn le(rt: &mut Runtime, a: H, b: H, site: u32) -> i32 {
    match cmp(rt, a, b) {
        Some(o) => tri_of(Some(o != Ordering::Greater)),
        None => {
            let r = tri_or(T_NULL, tri_of(eq_strict(rt, a, b)));
            if r == T_NULL {
                cmp_diag(rt, a, b, "<=", site);
            }
            r
        }
    }
}
pub fn ge(rt: &mut Runtime, a: H, b: H, site: u32) -> i32 {
    match cmp(rt, a, b) {
        Some(o) => tri_of(Some(o != Ordering::Less)),
        None => {
            let r = tri_or(T_NULL, tri_of(eq_strict(rt, a, b)));
            if r == T_NULL {
                cmp_diag(rt, a, b, ">=", site);
            }
            r
        }
    }
}

/// Equality where null operands yield null (used inside `<=`).
fn eq_strict(rt: &Runtime, a: H, b: H) -> Option<bool> {
    if rt.is_null(a) || rt.is_null(b) {
        return None;
    }
    eq(rt, a, b)
}

pub fn eq_tri(rt: &mut Runtime, a: H, b: H, site: u32) -> i32 {
    let r = eq(rt, a, b);
    if r.is_none() {
        let (ka, kb) = (rt.kind_name(a), rt.kind_name(b));
        rt.diag(
            "FEEL-E307",
            site,
            format!("cannot compare {ka} with {kb} using '='"),
        );
    }
    tri_of(r)
}

// -------------------------------------------------------------------- ranges and `in`

/// Does `r` contain `x`? (Table 55)
pub fn range_contains(rt: &mut Runtime, r: &Range, x: H, site: u32) -> i32 {
    if r.ne {
        let e = r.start.unwrap_or(NULL);
        return tri_not(eq_tri(rt, x, e, site));
    }
    let mut res = T_TRUE;
    if let Some(s) = r.start {
        let t = if r.start_incl {
            ge(rt, x, s, site)
        } else {
            gt(rt, x, s, site)
        };
        res = tri_and(res, t);
        if res == T_FALSE {
            return res;
        }
    }
    if let Some(e) = r.end {
        let t = if r.end_incl {
            le(rt, x, e, site)
        } else {
            lt(rt, x, e, site)
        };
        res = tri_and(res, t);
    }
    res
}

/// Semantics of `candidate in v` where `v` is the value of a positive unary
/// test expression (Table 55 and 10.3.2.10): ranges test containment, lists
/// test membership (elements may be ranges), a boolean that was computed
/// with `?` is the result, anything else is compared with `=`.
pub fn in_value(rt: &mut Runtime, cand: H, v: H, uses_q: bool, site: u32) -> i32 {
    if uses_q {
        return match rt.get(v) {
            Value::Bool(b) => tri_of(Some(*b)),
            Value::Range(r) => {
                let r = r.clone();
                range_contains(rt, &r, cand, site)
            }
            Value::List(_) => in_list(rt, cand, v, site),
            _ => {
                // `? ...` that did not yield a boolean: fall back to equality
                tri_of(eq(rt, cand, v))
            }
        };
    }
    match rt.get(v) {
        Value::Range(r) => {
            let r = r.clone();
            range_contains(rt, &r, cand, site)
        }
        Value::List(_) => {
            if let Value::List(_) = rt.get(cand) {
                // a list candidate: first try list equality, then membership
                if eq(rt, cand, v) == Some(true) {
                    return T_TRUE;
                }
            }
            in_list(rt, cand, v, site)
        }
        _ => tri_of(eq(rt, cand, v)),
    }
}

/// List membership (`list contains` semantics, Table 55): true if the
/// candidate equals an element (or is contained in a range element), false
/// otherwise — incomparable elements do not make the result null.
fn in_list(rt: &mut Runtime, cand: H, v: H, site: u32) -> i32 {
    let items = rt.list_items(v).unwrap_or_default();
    for it in items {
        let t = match rt.get(it) {
            Value::Range(r) => {
                let r = r.clone();
                let saved = rt.diags.len();
                let t = range_contains(rt, &r, cand, site);
                rt.diags.truncate(saved);
                t
            }
            _ => tri_of(eq(rt, cand, it)),
        };
        if t == T_TRUE {
            return T_TRUE;
        }
    }
    T_FALSE
}

pub fn make_range(rt: &mut Runtime, start: H, end: H, flags: i32, site: u32) -> H {
    // flags: bit0 start included, bit1 end included, bit2 start unbounded,
    // bit3 end unbounded, bit4 "not equal" (`!= e`)
    let start_unb = flags & 4 != 0;
    let end_unb = flags & 8 != 0;
    let s = if start_unb { None } else { Some(start) };
    let e = if end_unb { None } else { Some(end) };
    // endpoints must be of comparable, equivalent types (10.3.2.7)
    if let (Some(s), Some(e)) = (s, e) {
        let (ks, ke) = (rt.kind_name(s), rt.kind_name(e));
        if ks != ke {
            rt.diag(
                "FEEL-E308",
                site,
                format!("range endpoints must have the same type ({ks}, {ke})"),
            );
            return NULL;
        }
        match cmp(rt, s, e) {
            None => {
                rt.diag(
                    "FEEL-E308",
                    site,
                    format!("invalid range endpoints ({ks}, {ke})"),
                );
                return NULL;
            }
            Some(Ordering::Greater) => {
                rt.diag(
                    "FEEL-E308",
                    site,
                    "range start is greater than range end".to_string(),
                );
                return NULL;
            }
            _ => {}
        }
    } else {
        let p = s.or(e).unwrap_or(NULL);
        let k = rt.kind_name(p);
        if !matches!(
            k,
            "number"
                | "string"
                | "date"
                | "time"
                | "date and time"
                | "days and time duration"
                | "years and months duration"
        ) {
            rt.diag("FEEL-E308", site, format!("invalid range endpoint ({k})"));
            return NULL;
        }
    }
    rt.alloc(Value::Range(Range {
        start: s,
        end: e,
        start_incl: !start_unb && flags & 1 != 0,
        end_incl: !end_unb && flags & 2 != 0,
        ne: flags & 16 != 0,
        eq: flags & 32 != 0,
    }))
}

// -------------------------------------------------------------------- paths & properties

fn dec_secs(nanos: i128) -> Option<Decimal> {
    Decimal::new(nanos, 9).map(|d| d.normalize())
}

fn dtd_value(rt: &mut Runtime, secs: i64) -> H {
    rt.alloc(Value::Dtd(tm::DayTimeDuration {
        nanos: secs as i128 * 1_000_000_000,
    }))
}

fn zone_props(
    rt: &mut Runtime,
    zone: &Option<tm::Zone>,
    offset: Option<i32>,
    key: &str,
) -> Option<H> {
    match key {
        "time offset" => Some(match offset {
            Some(o) => dtd_value(rt, o as i64),
            None => NULL,
        }),
        "timezone" => Some(match zone {
            Some(tm::Zone::Named(n)) => {
                let n = n.clone();
                rt.str(&n)
            }
            _ => NULL,
        }),
        _ => None,
    }
}

/// Property / context entry selection `v.key` (Tables 64-67). Returns
/// `ABSENT` when the key does not exist (callers map it to null).
pub fn select(rt: &mut Runtime, v: H, key: &str) -> H {
    match rt.get(v).clone() {
        Value::Ctx(c) => c.get(key).unwrap_or(ABSENT),
        Value::Date(d) => match key {
            "year" => rt.int(d.year()),
            "month" => rt.int(d.month() as i64),
            "day" => rt.int(d.day() as i64),
            "weekday" => rt.int(d.weekday() as i64),
            _ => ABSENT,
        },
        Value::DateTime(dt) => match key {
            "year" => rt.int(dt.date.year()),
            "month" => rt.int(dt.date.month() as i64),
            "day" => rt.int(dt.date.day() as i64),
            "weekday" => rt.int(dt.date.weekday() as i64),
            "hour" => rt.int(dt.nanos / 3_600_000_000_000),
            "minute" => rt.int(dt.nanos / 60_000_000_000 % 60),
            "second" => {
                let d = dec_secs((dt.nanos % 60_000_000_000) as i128);
                rt.num_opt(d)
            }
            "time" => {
                let t = dt.time();
                rt.alloc(Value::Time(t))
            }
            "date" => rt.alloc(Value::Date(dt.date)),
            _ => {
                let off = dt.offset_seconds();
                zone_props(rt, &dt.zone, off, key).unwrap_or(ABSENT)
            }
        },
        Value::Time(t) => match key {
            "hour" => rt.int(t.nanos / 3_600_000_000_000),
            "minute" => rt.int(t.nanos / 60_000_000_000 % 60),
            "second" => {
                let d = dec_secs((t.nanos % 60_000_000_000) as i128);
                rt.num_opt(d)
            }
            _ => {
                let off = t.offset_seconds();
                zone_props(rt, &t.zone, off, key).unwrap_or(ABSENT)
            }
        },
        Value::Ymd(d) => {
            let m = d.months;
            match key {
                "years" => rt.int(m / 12),
                "months" => rt.int(m % 12),
                "days" | "hours" | "minutes" | "seconds" => NULL,
                _ => ABSENT,
            }
        }
        Value::Dtd(d) => {
            let n = d.nanos;
            let total_secs = n / 1_000_000_000;
            match key {
                "days" => rt.int((total_secs / 86400) as i64),
                "hours" => rt.int((total_secs / 3600 % 24) as i64),
                "minutes" => rt.int((total_secs / 60 % 60) as i64),
                "seconds" => {
                    let d = dec_secs(n % 60_000_000_000);
                    rt.num_opt(d)
                }
                "years" | "months" => NULL,
                _ => ABSENT,
            }
        }
        Value::Range(r) => match key {
            "start" => r.start.unwrap_or(NULL),
            "end" => r.end.unwrap_or(NULL),
            "start included" => rt.bool(r.start_incl),
            "end included" => rt.bool(r.end_incl),
            _ => ABSENT,
        },
        _ => ABSENT,
    }
}

/// Context entry / property selection without list projection or
/// diagnostics; missing → null (used by fused filter projections).
pub fn select_or_null(rt: &mut Runtime, v: H, key: H) -> H {
    let k = match rt.as_str(key) {
        Some(k) => k,
        None => return NULL,
    };
    if matches!(rt.get(v), Value::List(_)) {
        return NULL;
    }
    let r = select(rt, v, &k);
    if r == ABSENT {
        NULL
    } else {
        r
    }
}

/// `e.name` (rule 43, 10.3.2.5): selection on contexts, properties of
/// temporal values and ranges, and projection over lists.
pub fn path(rt: &mut Runtime, v: H, key: H, site: u32) -> H {
    let k = match rt.as_str(key) {
        Some(k) => k,
        None => return NULL,
    };
    path_str(rt, v, &k, site)
}

pub fn path_str(rt: &mut Runtime, v: H, k: &Rc<str>, site: u32) -> H {
    if let Value::List(items) = rt.get(v) {
        let items = items.clone();
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            let r = select(rt, it, k);
            out.push(if r == ABSENT { NULL } else { r });
        }
        return rt.list(out);
    }
    let r = select(rt, v, k);
    if r == ABSENT {
        let kind = rt.kind_name(v);
        if kind == "null" {
            rt.diag(
                "FEEL-E310",
                site,
                format!("selecting '{k}' from null yields null"),
            );
        } else if kind == "context" {
            rt.diag("FEEL-E311", site, format!("context has no entry '{k}'"));
        } else {
            rt.diag("FEEL-E312", site, format!("{kind} has no property '{k}'"));
        }
        NULL
    } else {
        r
    }
}

/// Lookup used for implicit names inside filters: entries of the current
/// item (if it is a context). Returns `ABSENT` if not found.
pub fn item_field(rt: &Runtime, item: H, key: H) -> H {
    if item == ABSENT {
        return ABSENT;
    }
    let k = match rt.get(key) {
        Value::Str(s) => s,
        _ => return ABSENT,
    };
    match rt.get(item) {
        Value::Ctx(c) => c.get(k).unwrap_or(ABSENT),
        _ => ABSENT,
    }
}

// -------------------------------------------------------------------- lists

/// FEEL list indexing `l[n]` (Table 68), 1-based, negative from the end.
pub fn index(rt: &mut Runtime, l: H, n: H, site: u32) -> H {
    let idx = match rt.as_num(n) {
        Some(d) => d,
        None => return NULL,
    };
    let i = match idx.to_i64() {
        Some(i) => i,
        None => {
            rt.diag("FEEL-E313", site, "list index must be an integer");
            return NULL;
        }
    };
    match rt.get(l) {
        Value::List(items) => {
            let len = items.len() as i64;
            let p = if i > 0 {
                i - 1
            } else if i < 0 {
                len + i
            } else {
                -1
            };
            if p < 0 || p >= len {
                rt.diag(
                    "FEEL-E314",
                    site,
                    format!("list index {i} out of bounds (length {len})"),
                );
                NULL
            } else {
                items[p as usize]
            }
        }
        Value::Null => NULL,
        _ => {
            if i == 1 {
                l
            } else {
                NULL
            }
        }
    }
}

/// Convert an iteration context value to a list (for/some/every, 10.3.2.14).
/// Returns `ABSENT` for null.
pub fn as_iter(rt: &mut Runtime, v: H, site: u32) -> H {
    match rt.get(v).clone() {
        Value::List(_) => v,
        Value::Null => ABSENT,
        Value::Range(r) => match (r.start, r.end) {
            (Some(s), Some(e)) => {
                let mut a = s;
                let mut b = e;
                // expand integer / date ranges honouring inclusivity
                if !r.start_incl {
                    a = step(rt, s, 1);
                }
                if !r.end_incl {
                    b = step(rt, e, -1);
                }
                if a == NULL || b == NULL {
                    rt.diag(
                        "FEEL-E315",
                        site,
                        "only ranges of integers or dates can be iterated",
                    );
                    return ABSENT;
                }
                if cmp(rt, a, b) == Some(Ordering::Greater) {
                    return rt.list(Vec::new());
                }
                iter_range(rt, a, b, site)
            }
            _ => {
                rt.diag("FEEL-E315", site, "cannot iterate over an unbounded range");
                ABSENT
            }
        },
        _ => rt.list(vec![v]),
    }
}

fn step(rt: &mut Runtime, v: H, by: i64) -> H {
    match rt.get(v).clone() {
        Value::Num(d) if d.is_integer() => {
            let r = d.add(&Decimal::from_i64(by));
            rt.num_opt(r)
        }
        Value::Date(d) => match tm::Date::from_days(d.days() + by) {
            Some(x) => rt.alloc(Value::Date(x)),
            None => NULL,
        },
        _ => NULL,
    }
}

/// `a..b` iteration contexts: integers (step ±1) or dates (step ±1 day).
pub fn iter_range(rt: &mut Runtime, a: H, b: H, site: u32) -> H {
    let max = rt.limits.max_collection as i128;
    match (rt.get(a).clone(), rt.get(b).clone()) {
        (Value::Num(x), Value::Num(y)) => {
            let (Some(x), Some(y)) = (x.to_i128(), y.to_i128()) else {
                rt.diag(
                    "FEEL-E315",
                    site,
                    "numeric iteration bounds must be integers",
                );
                return ABSENT;
            };
            let n = (y - x).abs() + 1;
            if n > max {
                rt.limit(
                    "FEEL-L402",
                    format!("iteration range of {n} items exceeds the collection limit"),
                );
                return NULL;
            }
            let s: i128 = if y >= x { 1 } else { -1 };
            let mut out = Vec::with_capacity(n as usize);
            let mut i = x;
            loop {
                let h = match Decimal::from_i128(i) {
                    Some(d) => rt.num(d),
                    None => NULL,
                };
                out.push(h);
                if i == y {
                    break;
                }
                i += s;
            }
            rt.list(out)
        }
        (Value::Date(x), Value::Date(y)) => {
            let (dx, dy) = (x.days() as i128, y.days() as i128);
            let n = (dy - dx).abs() + 1;
            if n > max {
                rt.limit(
                    "FEEL-L402",
                    format!("iteration range of {n} items exceeds the collection limit"),
                );
                return NULL;
            }
            let s: i64 = if dy >= dx { 1 } else { -1 };
            let mut out = Vec::with_capacity(n as usize);
            let mut i = x.days();
            loop {
                let h = match tm::Date::from_days(i) {
                    Some(d) => rt.alloc(Value::Date(d)),
                    None => NULL,
                };
                out.push(h);
                if i == y.days() {
                    break;
                }
                i += s;
            }
            rt.list(out)
        }
        _ => {
            let (ka, kb) = (rt.kind_name(a), rt.kind_name(b));
            rt.diag(
                "FEEL-E315",
                site,
                format!("cannot iterate from {ka} to {kb}"),
            );
            ABSENT
        }
    }
}

/// Wrap a non-list as a singleton list for filtering (10.3.2.9.4).
pub fn as_filter_list(rt: &mut Runtime, v: H) -> H {
    match rt.get(v) {
        Value::List(_) => v,
        Value::Null => ABSENT,
        _ => rt.list(vec![v]),
    }
}

pub fn list_snapshot(rt: &mut Runtime, l: H) -> H {
    let items = rt.list_items(l).unwrap_or_default();
    rt.list(items)
}
