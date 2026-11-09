//! Table 57 arithmetic beyond the TCK: every operand combination, leap
//! years and centuries, end-of-month clamping, negative and 5+ digit years,
//! year range boundaries and overflow.

mod common;
use common::*;
use feel_temporal::*;

fn ev(lhs: &str, op: char, rhs: &str) -> String {
    let (a, b) = (at(lhs), at(rhs));
    show(&if op == '+' { add(&a, &b) } else { sub(&a, &b) })
}

#[test]
fn table_57_all_combinations() {
    for (lhs, op, rhs, out) in [
        // date and time - date and time
        (
            "2012-12-24T23:59:00",
            '-',
            "2012-12-22T03:45:00",
            "P2DT20H14M",
        ),
        (
            "2012-12-24T23:59:00Z",
            '-',
            "2012-12-24T23:59:00+01:00",
            "PT1H",
        ),
        ("2012-12-24T23:59:00", '-', "2012-12-24T23:59:00Z", "null"),
        // date and time +- years and months duration (and commutative +)
        ("2012-12-24T23:59:00", '+', "P1Y2M", "2014-02-24T23:59:00"),
        ("P1Y2M", '+', "2012-12-24T23:59:00", "2014-02-24T23:59:00"),
        ("2012-12-24T23:59:00", '-', "P1Y2M", "2011-10-24T23:59:00"),
        ("P1Y2M", '-', "2012-12-24T23:59:00", "null"),
        // date and time +- days and time duration
        ("2012-12-24T23:59:00", '+', "PT1M", "2012-12-25T00:00:00"),
        ("PT1M", '+', "2012-12-24T23:59:00", "2012-12-25T00:00:00"),
        ("2012-12-25T00:00:00", '-', "PT1M", "2012-12-24T23:59:00"),
        ("PT1M", '-', "2012-12-24T23:59:00", "null"),
        (
            "2012-12-24T23:59:00.5Z",
            '+',
            "PT0.5S",
            "2012-12-24T23:59:01Z",
        ),
        // time - time, time +- days and time duration
        ("23:59:00", '-', "00:01:00", "PT23H58M"),
        ("23:59:00Z", '+', "PT2M", "00:01:00Z"),
        ("PT2M", '+', "23:59:00Z", "00:01:00Z"),
        ("00:01:00", '-', "PT2M", "23:59:00"),
        ("00:00:00", '-', "P1DT1S", "23:59:59"),
        ("10:00:00", '+', "P365DT1H", "11:00:00"),
        ("10:00:00", '-', "-PT25H", "11:00:00"),
        ("10:00:00", '+', "P1Y", "null"),
        ("PT2M", '-', "23:59:00Z", "null"),
        // duration +- duration
        ("P1Y", '+', "P1M", "P1Y1M"),
        ("P1D", '-', "PT25H", "-PT1H"),
        ("P1Y", '+', "P1D", "null"),
        ("P1Y", '-', "P1D", "null"),
        // date +- years and months duration
        ("2012-12-25", '+', "P1M", "2013-01-25"),
        ("P1M", '+', "2012-12-25", "2013-01-25"),
        ("2012-12-25", '-', "P12M", "2011-12-25"),
        ("P1M", '-', "2012-12-25", "null"),
        // date +- days and time duration
        ("2012-12-24", '+', "PT23H59M59.999999999S", "2012-12-24"),
        ("2012-12-24", '+', "PT24H", "2012-12-25"),
        ("2012-12-24", '-', "PT0.000000001S", "2012-12-23"),
        ("2012-12-24", '-', "-PT23H", "2012-12-24"),
        ("PT24H", '-', "2012-12-24", "null"),
        // date - date
        ("2012-12-25", '-', "2012-12-24", "P1D"),
        ("2012-12-24", '-', "2012-12-25", "-P1D"),
        ("2012-03-01", '-', "2012-02-28", "P2D"),
        ("2013-03-01", '-', "2013-02-28", "P1D"),
        ("2001-01-01", '-', "2000-01-01", "P366D"),
        ("1901-01-01", '-', "1900-01-01", "P365D"),
        ("0001-01-01", '-', "-0001-01-01", "P731D"),
        // date - date and time (date = UTC midnight)
        ("2012-12-25", '-', "2012-12-24T12:00:00Z", "PT12H"),
        (
            "2012-12-25T12:00:00@Europe/Paris",
            '-',
            "2012-12-25",
            "PT11H",
        ),
        ("2012-12-25", '-', "2012-12-24T12:00:00", "null"),
        // undefined combinations
        ("2012-12-25", '+', "2012-12-25", "null"),
        ("2012-12-25T00:00:00", '+', "2012-12-25T00:00:00", "null"),
        ("10:00:00", '+', "10:00:00", "null"),
        ("2012-12-25", '-', "10:00:00", "null"),
        ("10:00:00", '-', "2012-12-25T00:00:00", "null"),
    ] {
        assert_eq!(ev(lhs, op, rhs), out, "{lhs} {op} {rhs}");
    }
}

#[test]
fn end_of_month_clamping() {
    let am = |s: &str, m: i64| d(s).add_months(m).map(|x| x.to_string());
    assert_eq!(am("2017-01-31", 1).as_deref(), Some("2017-02-28"));
    assert_eq!(am("2016-01-31", 1).as_deref(), Some("2016-02-29"));
    assert_eq!(am("2016-02-29", 12).as_deref(), Some("2017-02-28"));
    assert_eq!(am("2016-02-29", 48).as_deref(), Some("2020-02-29"));
    assert_eq!(am("2017-03-31", -1).as_deref(), Some("2017-02-28"));
    assert_eq!(am("2017-05-31", 1).as_deref(), Some("2017-06-30"));
    assert_eq!(am("2017-12-31", 2).as_deref(), Some("2018-02-28"));
    assert_eq!(am("2000-02-29", -1200).as_deref(), Some("1900-02-28"));
    assert_eq!(am("2017-01-15", -13).as_deref(), Some("2015-12-15"));
    assert_eq!(
        ev("2017-01-31T10:00:00@Europe/Paris", '+', "P1M"),
        "2017-02-28T10:00:00@Europe/Paris"
    );
    assert_eq!(ev("2017-01-31", '+', "P1M"), "2017-02-28");
    // literal Table 57 behaviour
    assert_eq!(d("2017-01-31").add_months_strict(1), None);
    assert_eq!(d("2017-01-30").add_months_strict(12), Some(d("2018-01-30")));
}

#[test]
fn leap_years_and_centuries() {
    assert!(Date::from_ymd(2000, 2, 29).is_some());
    assert!(Date::from_ymd(2400, 2, 29).is_some());
    assert!(Date::from_ymd(1900, 2, 29).is_none());
    assert!(Date::from_ymd(2100, 2, 29).is_none());
    assert!(Date::from_ymd(2024, 2, 29).is_some());
    assert!(Date::from_ymd(2023, 2, 29).is_none());
    assert!(Date::from_ymd(0, 2, 29).is_some());
    assert!(Date::from_ymd(-4, 2, 29).is_some());
    assert!(Date::from_ymd(-100, 2, 29).is_none());
    assert!(Date::from_ymd(-400, 2, 29).is_some());
    assert_eq!(ev("2000-02-28", '+', "P1D"), "2000-02-29");
    assert_eq!(ev("1900-02-28", '+', "P1D"), "1900-03-01");
    assert_eq!(
        ev("2100-02-28T12:00:00", '+', "PT24H"),
        "2100-03-01T12:00:00"
    );
    assert_eq!(ev("2400-03-01", '-', "2400-02-01"), "P29D");
    // 400 years = 146097 days in every cycle, also for negative years
    assert_eq!(ev("2000-01-01", '-', "1600-01-01"), "P146097D");
    assert_eq!(ev("-1600-01-01", '-', "-2000-01-01"), "P146097D");
}

#[test]
fn negative_and_large_years() {
    assert_eq!(ev("-0001-12-31", '+', "P1D"), "0000-01-01");
    assert_eq!(ev("0000-01-01", '-', "P1D"), "-0001-12-31");
    assert_eq!(ev("-0001-01-01", '+', "P1Y"), "0000-01-01");
    assert_eq!(ev("0000-01-01", '-', "P1Y"), "-0001-01-01");
    assert_eq!(
        ev("9999-12-31T23:59:59", '+', "PT1S"),
        "10000-01-01T00:00:00"
    );
    assert_eq!(ev("10000-01-01", '-', "P1D"), "9999-12-31");
    assert_eq!(ev("-9999-01-01", '-', "P1D"), "-10000-12-31");
    assert_eq!(ev("123456789-06-15", '+', "P1000Y"), "123457789-06-15");
    assert_eq!(d("-10000-12-31").to_string(), "-10000-12-31");
    assert_eq!(d("-0001-01-01").year(), -1);
    assert_eq!(d("0000-06-01").year(), 0);
}

#[test]
fn year_range_boundaries() {
    let max = d("999999999-12-31");
    let min = d("-999999999-01-01");
    assert_eq!(max, Date::MAX);
    assert_eq!(min, Date::MIN);
    assert_eq!(Date::from_days(Date::MAX.days() + 1), None);
    assert_eq!(Date::from_days(Date::MIN.days() - 1), None);
    assert_eq!(max.add_days(1), None);
    assert_eq!(min.add_days(-1), None);
    assert_eq!(max.add_days(i64::MAX), None);
    assert_eq!(max.add_months(1), None);
    assert_eq!(min.add_months(-1), None);
    assert_eq!(max.add_months(i64::MAX), None);
    assert_eq!(min.add_months(i64::MIN), None);
    assert_eq!(ev("999999999-12-31", '+', "P1D"), "null");
    assert_eq!(ev("999999999-12-31", '+', "PT23H59M59S"), "999999999-12-31");
    assert_eq!(ev("-999999999-01-01", '-', "PT1S"), "null");
    assert_eq!(
        ev("999999999-12-31T23:59:59.999999999", '+', "PT0.000000001S"),
        "null"
    );
    assert_eq!(
        ev("999999999-12-31T23:59:59.999999999Z", '-', "-PT1S"),
        "null"
    );
    assert_eq!(ev("999999999-12-01T00:00:00", '+', "P1M"), "null");
    assert_eq!(ev("-999999999-01-31T00:00:00", '-', "P1M"), "null");
    assert_eq!(
        ev("999999999-12-31T00:00:00Z", '+', "P99999999999999999999D"),
        "null"
    );
    assert_eq!(ev("10:00:00", '+', "P99999999999999999999DT1H"), "11:00:00");
    // the full range difference is representable
    let span = date_sub(&max, &min);
    assert_eq!(span.days(), i128::from(max.days() - min.days()));
    assert_eq!(date_add_dtd(&min, &span), Some(max));
    assert_eq!(
        datetime_sub(
            &dt("999999999-12-31T23:59:59.999999999Z"),
            &dt("-999999999-01-01T00:00:00-14:00")
        )
        .map(|x| x.nanos > 0),
        Some(true)
    );
    assert_eq!(
        years_and_months_between_dates(&min, &max).as_months(),
        (999_999_999i64 * 2) * 12 + 11
    );
    // the offset can move an in-range local value outside the year range in UTC
    let edge = dt("-999999999-01-01T00:00:00+14:00");
    assert!(edge.epoch_nanos().is_some());
    assert_eq!(
        DateTime::from_epoch_nanos(edge.epoch_nanos().unwrap(), Zone::utc()),
        None
    );
}

#[test]
fn time_arithmetic() {
    let s = |a: &str, b: &str| time_sub(&t(a), &t(b)).unwrap().to_string();
    assert_eq!(s("10:10:10", "09:10:10"), "PT1H");
    assert_eq!(s("10:10:10", "11:10:10"), "-PT1H");
    assert_eq!(s("10:10:10+11:00", "11:10:10+11:00"), "-PT1H");
    assert_eq!(
        s(
            "10:10:10@Australia/Melbourne",
            "09:10:10@Australia/Melbourne"
        ),
        "PT1H"
    );
    // both zoned: UTC-normalized
    assert_eq!(s("10:00:00+02:00", "10:00:00Z"), "-PT2H");
    assert_eq!(s("01:00:00+02:00", "22:00:00Z"), "-PT23H");
    // one unzoned: takes the other's offset, i.e. local difference
    assert_eq!(s("10:00:00+02:00", "09:00:00"), "PT1H");
    assert_eq!(s("10:00:00", "09:00:00@Europe/Paris"), "PT1H");
    // named zones use the standard offset
    assert_eq!(s("10:00:00@Europe/Paris", "10:00:00Z"), "-PT1H");
    assert_eq!(s("10:00:00.5", "10:00:00.25"), "PT0.25S");
    let a = |x: &str, d: &str| time_add_dtd(&t(x), &dtd(d)).unwrap().to_string();
    assert_eq!(a("10:15:00", "PT1H"), "11:15:00");
    assert_eq!(a("10:15:00", "P1D"), "10:15:00");
    assert_eq!(a("10:15:00+11:00", "P1D"), "10:15:00+11:00");
    assert_eq!(
        a("10:15:00@Australia/Melbourne", "-PT1H"),
        "09:15:00@Australia/Melbourne"
    );
    assert_eq!(a("23:00:00", "PT1H"), "00:00:00");
    assert_eq!(a("00:00:00", "-PT1S"), "23:59:59");
    assert_eq!(a("00:00:00", "-PT0.000000001S"), "23:59:59.999999999");
    assert_eq!(a("12:00:00", "-P1000DT12H"), "00:00:00");
    let m = |x: &str, d: &str| time_sub_dtd(&t(x), &dtd(d)).unwrap().to_string();
    assert_eq!(m("10:10:10", "PT1H"), "09:10:10");
    assert_eq!(m("10:10:10", "P1D"), "10:10:10");
    assert_eq!(m("00:30:00Z", "PT1H"), "23:30:00Z");
    assert_eq!(m("23:30:00", "-PT1H"), "00:30:00");
}

#[test]
fn date_and_time_values_internal_consistency() {
    // a + (b - a) = b for zoned and unzoned values
    for (a, b) in [
        ("2012-12-24T23:59:00", "2016-02-29T00:00:00.000000001"),
        ("-2012-12-24T23:59:00Z", "2016-02-29T00:00:00+05:00"),
        (
            "2021-03-27T12:00:00@Europe/Paris",
            "2021-11-01T08:00:00@Europe/Paris",
        ),
    ] {
        let (a, b) = (dt(a), dt(b));
        let diff = datetime_sub(&b, &a).unwrap();
        let back = datetime_add_dtd(&a, &diff).unwrap();
        assert_eq!(eq_datetime_exact(&back, &b), Some(true), "{a} {b}");
        assert_eq!(datetime_sub_dtd(&back, &diff).unwrap(), a);
    }
    // valuedt-based subtraction of fixed offsets
    assert_eq!(
        datetime_sub(
            &dt("2002-04-02T12:00:00-01:00"),
            &dt("2002-04-02T17:00:00+04:00")
        ),
        Some(DayTimeDuration::ZERO)
    );
}
