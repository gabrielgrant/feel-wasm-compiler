//! Shared helpers: a tiny dispatcher emulating FEEL `@"..."` literals and the
//! Table 57 `+` / `-` operators on top of the crate API, the way the compiler
//! runtime is expected to call it.
#![allow(dead_code)]

use feel_temporal::*;

#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Date(Date),
    Time(Time),
    DateTime(DateTime),
    Dtd(DayTimeDuration),
    Ymd(YearMonthDuration),
    Null,
}

/// FEEL `@"..."` literal.
pub fn at(s: &str) -> V {
    if s.starts_with('P') || s.starts_with("-P") {
        match parse_duration(s) {
            Some(Duration::DayTime(d)) => V::Dtd(d),
            Some(Duration::YearMonth(d)) => V::Ymd(d),
            None => V::Null,
        }
    } else if let Some(d) = Date::parse(s) {
        V::Date(d)
    } else if let Some(d) = DateTime::parse(s) {
        V::DateTime(d)
    } else if let Some(t) = Time::parse(s) {
        V::Time(t)
    } else {
        V::Null
    }
}

fn opt<T>(o: Option<T>, f: impl FnOnce(T) -> V) -> V {
    o.map_or(V::Null, f)
}

pub fn add(a: &V, b: &V) -> V {
    use V::*;
    match (a, b) {
        (DateTime(x), Dtd(d)) | (Dtd(d), DateTime(x)) => opt(datetime_add_dtd(x, d), DateTime),
        (DateTime(x), Ymd(d)) | (Ymd(d), DateTime(x)) => opt(datetime_add_ymd(x, d), DateTime),
        (Date(x), Dtd(d)) | (Dtd(d), Date(x)) => opt(date_add_dtd(x, d), Date),
        (Date(x), Ymd(d)) | (Ymd(d), Date(x)) => opt(date_add_ymd(x, d), Date),
        (Time(x), Dtd(d)) | (Dtd(d), Time(x)) => opt(time_add_dtd(x, d), Time),
        (Dtd(x), Dtd(y)) => opt(x.checked_add(y), Dtd),
        (Ymd(x), Ymd(y)) => opt(x.checked_add(y), Ymd),
        _ => Null,
    }
}

pub fn sub(a: &V, b: &V) -> V {
    use V::*;
    match (a, b) {
        (DateTime(x), DateTime(y)) => opt(datetime_sub(x, y), Dtd),
        (DateTime(x), Date(y)) => opt(datetime_sub_date(x, y), Dtd),
        (Date(x), DateTime(y)) => opt(date_sub_datetime(x, y), Dtd),
        (Date(x), Date(y)) => Dtd(date_sub(x, y)),
        (Time(x), Time(y)) => opt(time_sub(x, y), Dtd),
        (DateTime(x), Dtd(d)) => opt(datetime_sub_dtd(x, d), DateTime),
        (DateTime(x), Ymd(d)) => opt(datetime_sub_ymd(x, d), DateTime),
        (Date(x), Dtd(d)) => opt(date_sub_dtd(x, d), Date),
        (Date(x), Ymd(d)) => opt(date_sub_ymd(x, d), Date),
        (Time(x), Dtd(d)) => opt(time_sub_dtd(x, d), Time),
        (Dtd(x), Dtd(y)) => opt(x.checked_sub(y), Dtd),
        (Ymd(x), Ymd(y)) => opt(x.checked_sub(y), Ymd),
        _ => Null,
    }
}

pub fn show(v: &V) -> String {
    match v {
        V::Date(x) => x.to_string(),
        V::Time(x) => x.to_string(),
        V::DateTime(x) => x.to_string(),
        V::Dtd(x) => x.to_string(),
        V::Ymd(x) => x.to_string(),
        V::Null => "null".to_string(),
    }
}

/// Compares an evaluation result with a TCK expected value: durations by
/// value (the TCK compares `xsd:duration` semantically: `P0D` = `PT0S`), all
/// other values by their canonical string.
pub fn matches_expected(v: &V, expected: &str) -> bool {
    match v {
        V::Dtd(_) | V::Ymd(_) => {
            let e = at(expected);
            match (v, &e) {
                (V::Dtd(a), V::Dtd(b)) => a == b,
                (V::Ymd(a), V::Ymd(b)) => a == b,
                // the TCK writes zero durations of either kind as P0D / P0Y
                (V::Ymd(a), V::Dtd(b)) | (V::Dtd(b), V::Ymd(a)) => a.is_zero() && b.is_zero(),
                _ => false,
            }
        }
        _ => show(v) == expected,
    }
}

pub fn d(s: &str) -> Date {
    Date::parse(s).unwrap_or_else(|| panic!("date {s:?}"))
}
pub fn t(s: &str) -> Time {
    Time::parse(s).unwrap_or_else(|| panic!("time {s:?}"))
}
pub fn dt(s: &str) -> DateTime {
    DateTime::parse(s).unwrap_or_else(|| panic!("date and time {s:?}"))
}
pub fn dtd(s: &str) -> DayTimeDuration {
    DayTimeDuration::parse(s).unwrap_or_else(|| panic!("dtd {s:?}"))
}
pub fn ymd(s: &str) -> YearMonthDuration {
    YearMonthDuration::parse(s).unwrap_or_else(|| panic!("ymd {s:?}"))
}
