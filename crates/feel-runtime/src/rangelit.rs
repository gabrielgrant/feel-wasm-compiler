//! Range literal strings for the `range(from)` builtin (rule 66/67): only
//! literal endpoints are allowed (numbers, strings, `@"..."` and
//! `date(..)`/`time(..)`/`date and time(..)`/`duration(..)` with a string).

use crate::value::*;
use feel_decimal::Decimal;
use feel_temporal as tm;

pub fn parse_range(rt: &mut Runtime, s: &str) -> Option<H> {
    let t = s.trim();
    let first = t.chars().next()?;
    let last = t.chars().last()?;
    let start_incl = match first {
        '[' => true,
        '(' | ']' => false,
        _ => return None,
    };
    let end_incl = match last {
        ']' => true,
        ')' | '[' => false,
        _ => return None,
    };
    let inner = &t[1..t.len() - 1];
    let dd = find_dots(inner)?;
    let a = inner[..dd].trim();
    let b = inner[dd + 2..].trim();
    if a.is_empty() && b.is_empty() {
        return None;
    }
    let start = if a.is_empty() {
        if start_incl {
            return None;
        }
        None
    } else {
        Some(endpoint(rt, a)?)
    };
    let end = if b.is_empty() {
        if end_incl {
            return None;
        }
        None
    } else {
        Some(endpoint(rt, b)?)
    };
    if let (Some(x), Some(y)) = (start, end) {
        if rt.kind_name(x) != rt.kind_name(y) {
            return None;
        }
        match crate::ops::cmp(rt, x, y) {
            Some(std::cmp::Ordering::Greater) | None => return None,
            _ => {}
        }
    }
    Some(rt.alloc(Value::Range(Range { start, end, start_incl: start.is_some() && start_incl, end_incl: end.is_some() && end_incl, ne: false, eq: false })))
}

/// Position of the `..` separator outside string literals.
fn find_dots(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i + 1 < b.len() {
        match b[i] {
            b'"' => in_str = !in_str,
            b'\\' if in_str => i += 1,
            b'.' if !in_str && b[i + 1] == b'.' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn string_lit(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    c => out.push(c),
                }
            } else if c == '"' {
                return None;
            } else {
                out.push(c);
            }
        }
        Some(out)
    } else {
        None
    }
}

fn temporal(rt: &mut Runtime, s: &str) -> Option<H> {
    if s.contains('T') || (s.len() > 10 && s.as_bytes().get(4) == Some(&b'-') && s.contains(':')) {
        if let Some(dt) = tm::DateTime::parse(s) {
            return Some(rt.alloc(Value::DateTime(dt)));
        }
    }
    if let Some(d) = tm::Date::parse(s) {
        return Some(rt.alloc(Value::Date(d)));
    }
    if let Some(t) = tm::Time::parse(s) {
        return Some(rt.alloc(Value::Time(t)));
    }
    if let Some(d) = tm::DateTime::parse(s) {
        return Some(rt.alloc(Value::DateTime(d)));
    }
    match tm::parse_duration(s)? {
        tm::Duration::DayTime(d) => Some(rt.alloc(Value::Dtd(d))),
        tm::Duration::YearMonth(d) => Some(rt.alloc(Value::Ymd(d))),
    }
}

fn endpoint(rt: &mut Runtime, s: &str) -> Option<H> {
    let s = s.trim();
    if let Some(st) = string_lit(s) {
        return Some(rt.str(&st));
    }
    if let Some(rest) = s.strip_prefix('@') {
        let st = string_lit(rest)?;
        return temporal(rt, &st);
    }
    for f in ["date and time", "date", "time", "duration"] {
        if let Some(rest) = s.strip_prefix(f) {
            let rest = rest.trim();
            if rest.starts_with('(') && rest.ends_with(')') {
                let st = string_lit(&rest[1..rest.len() - 1])?;
                return match f {
                    "date and time" => tm::DateTime::parse(&st).map(|d| rt.alloc(Value::DateTime(d))),
                    "date" => tm::Date::parse(&st).map(|d| rt.alloc(Value::Date(d))),
                    "time" => tm::Time::parse(&st).map(|d| rt.alloc(Value::Time(d))),
                    _ => match tm::parse_duration(&st)? {
                        tm::Duration::DayTime(d) => Some(rt.alloc(Value::Dtd(d))),
                        tm::Duration::YearMonth(d) => Some(rt.alloc(Value::Ymd(d))),
                    },
                };
            }
        }
    }
    let d = Decimal::parse(s)?;
    Some(rt.num(d))
}
