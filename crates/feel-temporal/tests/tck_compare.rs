//! Equality / ordering (TCK 0068, 0071, 0072, 0017) and identity
//! (`is()`, TCK 0103), plus mixed zoned / unzoned comparisons.

mod common;
use common::*;
use core::cmp::Ordering::*;
use feel_temporal::*;

fn eqt(a: &str, b: &str) -> Option<bool> {
    eq_time(&t(a), &t(b))
}
fn eqdt(a: &str, b: &str) -> Option<bool> {
    eq_datetime(&dt(a), &dt(b))
}

#[test]
fn tck_0068_date_equality() {
    assert_eq!(d("2018-12-08"), d("2018-12-08"));
    assert_ne!(d("2018-12-07"), d("2018-12-08"));
    assert_eq!(cmp_date(&d("2018-12-07"), &d("2018-12-08")), Less);
}

#[test]
fn tck_0068_time_equality() {
    assert_eq!(eqt("10:30:00", "10:30:00"), Some(true));
    assert_eq!(eqt("11:30:00", "10:30:00"), Some(false));
    assert_eq!(eqt("10:31:00", "10:30:00"), Some(false));
    assert_eq!(eqt("10:30:01", "10:30:00"), Some(false));
    assert_eq!(eqt("10:30:00.0000", "10:30:00"), Some(true));
    assert_eq!(eqt("10:30:00.0001", "10:30:00.0001"), Some(true));
    // TCK time_005: equality is at whole-second precision
    assert_eq!(eqt("10:30:00.0001", "10:30:00.0002"), Some(true));
    assert_eq!(
        eq_time_exact(&t("10:30:00.0001"), &t("10:30:00.0002")),
        Some(false)
    );
    assert_eq!(
        eqt("10:30:00@Europe/Paris", "10:30:00@Europe/Paris"),
        Some(true)
    );
    assert_eq!(eqt("10:30:00Z", "10:30:00+00:00"), Some(true));
    // spec Table 72 example: time("23:59:00z") + duration("PT2M") = time("00:01:00@Etc/UTC")
    let sum = time_add_dtd(&t("23:59:00z"), &dtd("PT2M")).unwrap();
    assert_eq!(eq_time(&sum, &t("00:01:00@Etc/UTC")), Some(true));
    // spec Table 72 example: time("23:59:00z") = time(23, 59, 0, duration("PT0H"))
    let built = Time::new(23, 59, 0, Zone::from_offset_duration(&dtd("PT0H"))).unwrap();
    assert_eq!(eq_time(&t("23:59:00z"), &built), Some(true));
    assert!(is_same_time(&t("23:59:00z"), &built));
}

#[test]
fn tck_0068_datetime_equality() {
    assert_eq!(
        eqdt("2018-12-08T10:30:00", "2018-12-08T10:30:00"),
        Some(true)
    );
    assert_eq!(eqdt("2018-12-08", "2018-12-08T00:00:00"), Some(true));
    assert_eq!(eqdt("2018-12-08", "2018-12-08T00:00:00.0000"), Some(true));
    assert_eq!(
        eqdt("2018-12-08T00:00:00.0001", "2018-12-08T00:00:00.0001"),
        Some(true)
    );
    // TCK datetime_003_a: whole-second precision
    assert_eq!(
        eqdt("2018-12-08T00:00:00.0001", "2018-12-08T00:00:00.0000"),
        Some(true)
    );
    assert_eq!(
        eq_datetime_exact(
            &dt("2018-12-08T00:00:00.0001"),
            &dt("2018-12-08T00:00:00.0000")
        ),
        Some(false)
    );
    assert_eq!(
        eqdt("2018-12-08T00:00:00", "2018-12-07T00:00:00"),
        Some(false)
    );
    assert_eq!(
        eqdt(
            "2018-12-08T00:00:00@Europe/Paris",
            "2018-12-08T00:00:00@Europe/Paris"
        ),
        Some(true)
    );
    assert_eq!(
        eqdt(
            "2018-12-08T00:00:00@Europe/Paris",
            "2018-12-08T00:00:00@Asia/Dhaka"
        ),
        Some(false)
    );
    assert_eq!(
        eqdt(
            "2018-10-08T00:00:00+02:00",
            "2018-10-08T00:00:00@Europe/Paris"
        ),
        Some(true)
    );
    assert_eq!(
        eqdt(
            "2018-02-08T00:00:00+01:00",
            "2018-02-08T00:00:00@Europe/Paris"
        ),
        Some(true)
    );
    assert_eq!(
        eqdt(
            "2002-04-02T12:00:00@Australia/Melbourne",
            "2002-04-02T12:00:00@Australia/Sydney"
        ),
        Some(true)
    );
    assert_eq!(
        eqdt("2018-12-08T00:00:00+00:00", "2018-12-08T00:00:00@Etc/UTC"),
        Some(true)
    );
    assert_eq!(
        eqdt("2002-04-02T12:00:00-01:00", "2002-04-02T17:00:00+04:00"),
        Some(true)
    );
    assert_eq!(
        eqdt("2002-04-02T23:00:00-04:00", "2002-04-03T02:00:00-01:00"),
        Some(true)
    );
}

#[test]
fn durations_compare_by_value() {
    assert_eq!(dtd("P1D"), dtd("PT24H"));
    assert_ne!(dtd("P1D"), dtd("P2D"));
    assert_ne!(dtd("P1D"), dtd("-P1D"));
    assert_eq!(dtd("P0D"), dtd("-P0D"));
    assert_eq!(ymd("P1Y"), ymd("P12M"));
    assert_ne!(ymd("P1Y"), ymd("-P1Y"));
    assert_eq!(ymd("P0Y"), ymd("-P0Y"));
    assert!(dtd("P1D") < dtd("P2D") && dtd("-P1D") < dtd("PT0S"));
    assert!(ymd("P2Y") < ymd("P25M") && ymd("-P1Y") < ymd("P0M"));
    // P0Y and P0D are different kinds (`is` false, `=` undefined at the caller)
    assert_ne!(parse_duration("P0Y"), parse_duration("P0D"));
}

#[test]
fn tck_0071_between_and_0072_in() {
    let between = |x: &str, a: &str, b: &str| {
        cmp_time(&t(x), &t(a)).unwrap() != Less && cmp_time(&t(x), &t(b)).unwrap() != Greater
    };
    assert!(!between("10:31:00", "10:32:00", "10:34:00"));
    assert!(between("10:32:00", "10:32:00", "10:34:00"));
    assert!(between("10:33:00", "10:32:00", "10:34:00"));
    assert!(between("10:34:00", "10:32:00", "10:34:00"));
    assert!(!between("10:35:00", "10:32:00", "10:34:00"));
    let c = |a: &str, b: &str| cmp_datetime(&dt(a), &dt(b)).unwrap();
    assert_eq!(c("2018-12-01T10:30:00", "2018-12-02T10:30:00"), Less);
    assert_eq!(c("2018-12-02T10:30:00", "2018-12-02T10:30:00"), Equal);
    assert_eq!(c("2018-12-05T10:30:00", "2018-12-04T10:30:00"), Greater);
    assert_eq!(cmp_date(&d("2016-11-01"), &d("2016-10-01")), Greater);
    assert_eq!(cmp_date(&d("-2016-11-01"), &d("2016-10-01")), Less);
    assert_eq!(cmp_date(&d("10000-01-01"), &d("9999-12-31")), Greater);
}

#[test]
fn zoned_vs_unzoned_comparisons() {
    // "If one input has a null timezone offset, that input uses the timezone
    // offset of the other input": local fields are compared.
    assert_eq!(
        eqdt("2018-12-08T10:00:00", "2018-12-08T10:00:00+05:00"),
        Some(true)
    );
    assert_eq!(
        eqdt("2018-12-08T10:00:00", "2018-12-08T10:00:00@Asia/Dhaka"),
        Some(true)
    );
    assert_eq!(
        cmp_datetime(&dt("2018-12-08T09:00:00"), &dt("2018-12-08T10:00:00Z")),
        Some(Less)
    );
    assert_eq!(
        cmp_datetime(&dt("2018-12-08T11:00:00+05:00"), &dt("2018-12-08T10:00:00")),
        Some(Greater)
    );
    assert_eq!(eqt("10:00:00", "10:00:00+05:00"), Some(true));
    assert_eq!(
        cmp_time(&t("09:00:00"), &t("10:00:00@Europe/Paris")),
        Some(Less)
    );
    // both zoned: instants / UTC-normalized times compare
    assert_eq!(eqt("10:00:00+02:00", "08:00:00Z"), Some(true));
    assert_eq!(eqt("10:30:00+02:00", "10:30:00+01:00"), Some(false));
    assert_eq!(
        cmp_time(&t("10:30:00+02:00"), &t("10:30:00+01:00")),
        Some(Less)
    );
    // valuet is not reduced modulo 24h (XSD): 01:00+02:00 is 23:00Z the day before
    assert_eq!(cmp_time(&t("01:00:00+02:00"), &t("22:00:00Z")), Some(Less));
    // named zones on bare times use their standard offset
    assert_eq!(eqt("10:30:00@Europe/Paris", "09:30:00Z"), Some(true));
    assert_eq!(eqt("10:30:00@Europe/Paris", "10:30:00+01:00"), Some(true));
    assert_eq!(
        eqt("10:30:00@Europe/Paris", "10:30:00@Europe/Berlin"),
        Some(true)
    );
    assert_eq!(
        eqt("10:30:00@Europe/Paris", "10:30:00@Asia/Dhaka"),
        Some(false)
    );
    assert_eq!(eqt("10:30:00@Etc/UTC", "10:30:00Z"), Some(true));
    assert_eq!(eqt("10:30:00@Europe/Paris", "10:30:00"), Some(true));
    assert_eq!(
        cmp_time(&t("10:30:00@Asia/Dhaka"), &t("10:30:00@Europe/Paris")),
        Some(Less)
    );
}

#[test]
fn millisecond_comparison_is_consistent() {
    // sub-millisecond differences compare equal (TCK 0068 time_005)
    let (a, b) = (t("10:30:00.0001"), t("10:30:00.0009"));
    assert_eq!(cmp_time(&a, &b), Some(Equal));
    assert_eq!(cmp_time_exact(&a, &b), Some(Less));
    // ... but milliseconds and above are significant
    assert_eq!(cmp_time(&t("10:30:00.1"), &t("10:30:00.2")), Some(Less));
    assert_eq!(cmp_time(&t("10:30:00.9"), &t("10:30:01")), Some(Less));
    let (a, b) = (dt("2018-12-08T23:59:59.999"), dt("2018-12-09T00:00:00"));
    assert_eq!(cmp_datetime(&a, &b), Some(Less));
    // floor, not truncation toward zero, before the epoch
    let (a, b) = (
        dt("1969-12-31T23:59:59.5005Z"),
        dt("1969-12-31T23:59:59.5Z"),
    );
    assert_eq!(cmp_datetime(&a, &b), Some(Equal));
    assert_eq!(cmp_datetime(&a, &dt("1970-01-01T00:00:00Z")), Some(Less));
}

#[test]
fn tck_0103_is() {
    let is_t = |a: &str, b: &str| is_same_time(&t(a), &t(b));
    let is_dt = |a: &str, b: &str| is_same_datetime(&dt(a), &dt(b));
    assert!(is_same_date(&d("2012-12-25"), &d("2012-12-25")));
    assert!(!is_same_date(&d("2011-12-25"), &d("2012-12-25")));
    assert!(!is_same_date(&d("2012-12-25"), &d("2012-11-25")));
    assert!(!is_same_date(&d("2012-12-25"), &d("2012-12-26")));
    assert!(is_t("23:00:50", "23:00:50"));
    assert!(is_t("23:00:50+11:00", "23:00:50+11:00"));
    assert!(is_t(
        "23:00:50@Australia/Melbourne",
        "23:00:50@Australia/Melbourne"
    ));
    assert!(is_t("23:00:50Z", "23:00:50+00:00"));
    assert!(is_t("23:00:50Z", "23:00:50-00:00"));
    assert!(is_t("23:00:50z", "23:00:50+00:00"));
    assert!(!is_t("23:00:50z", "23:00:50"));
    assert!(!is_t("23:00:50", "23:00:50Z"));
    assert!(!is_t("23:00:50@Australia/Melbourne", "23:00:50+10:00"));
    assert!(!is_t("23:00:50@Etc/GMT", "23:00:50Z"));
    assert!(!is_t("22:00:50", "23:00:50"));
    assert!(!is_t("23:01:50", "23:00:50"));
    assert!(!is_t("23:00:51", "23:00:50"));
    assert!(!is_t("23:00:50+10:00", "23:00:50+11:00"));
    assert!(!is_t(
        "23:00:50@Australia/Melbourne",
        "23:00:50@Australia/Sydney"
    ));
    assert!(!is_t("20:00:50+00:00", "21:00:50+01:00"));
    // `is` is exact even where `=` is whole-second
    assert!(!is_t("10:30:00.0001", "10:30:00.0002"));
    assert!(is_dt("2012-12-25T10:10:10", "2012-12-25T10:10:10"));
    assert!(is_dt(
        "2012-12-25T10:10:10+11:00",
        "2012-12-25T10:10:10+11:00"
    ));
    assert!(is_dt(
        "2012-12-25T10:10:10@Australia/Melbourne",
        "2012-12-25T10:10:10@Australia/Melbourne"
    ));
    assert!(!is_dt(
        "2002-04-02T12:00:00-01:00",
        "2002-04-02T17:00:00+04:00"
    ));
    assert!(!is_dt(
        "2002-04-02T12:00:00-05:00",
        "2002-04-02T23:00:00+06:00"
    ));
    assert!(!is_dt(
        "2002-04-02T23:00:00-04:00",
        "2002-04-03T02:00:00-01:00"
    ));
    assert!(!is_dt("2002-04-02T23:00:00", "2001-04-02T23:00:00"));
    assert!(!is_dt("2002-04-02T23:00:00", "2002-05-02T23:00:00"));
    assert!(!is_dt("2002-04-02T23:00:00", "2002-04-03T23:00:00"));
    assert!(!is_dt("2002-04-02T23:00:00", "2002-04-02T22:00:00"));
    assert!(!is_dt("2002-04-02T23:00:00", "2002-04-02T23:01:00"));
    assert!(!is_dt("2002-04-02T23:00:00", "2002-04-02T23:00:01"));
    assert!(!is_dt(
        "2002-04-02T23:00:00+10:00",
        "2002-04-02T23:00:00+11:00"
    ));
    assert!(!is_dt(
        "2002-04-02T23:00:00@Australia/Melbourne",
        "2002-04-02T23:00:00@Australia/Sydney"
    ));
    assert!(!is_dt(
        "2021-04-02T23:00:00@Australia/Melbourne",
        "2021-04-02T23:00:00+11:00"
    ));
    assert!(!is_dt(
        "2021-10-02T23:00:00@Australia/Melbourne",
        "2021-10-02T23:00:00+10:00"
    ));
    // ... although those are `=`
    assert_eq!(
        eqdt(
            "2021-04-02T23:00:00@Australia/Melbourne",
            "2021-04-02T23:00:00+11:00"
        ),
        Some(true)
    );
    assert_eq!(
        eqdt(
            "2021-10-02T23:00:00@Australia/Melbourne",
            "2021-10-02T23:00:00+10:00"
        ),
        Some(true)
    );
    // date vs date and time is a type mismatch for the caller; the UTC
    // conversion differs from the zoned value anyway
    assert!(!is_dt("2012-12-25T00:00:00", "2012-12-25T00:00:00Z"));
    assert!(is_same_datetime(
        &DateTime::from_date_utc(&d("2012-12-25")),
        &dt("2012-12-25T00:00:00Z")
    ));
    // the end-of-day form is normalized at parse time
    assert!(is_dt("2021-01-01T24:00:00", "2021-01-02T00:00:00"));
}
