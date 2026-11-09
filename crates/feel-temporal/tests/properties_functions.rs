//! Properties (TCK 0074, Tables 65/66), temporal built-ins (TCK 0095–0098,
//! Table 79), `years and months duration()` (TCK 1121) and duration helpers.

mod common;
use common::*;
use feel_temporal::*;

const S: i64 = NANOS_PER_SEC;

#[test]
fn tck_0074_date_and_datetime_properties() {
    let x = d("2018-12-10");
    assert_eq!(
        (x.year(), x.month(), x.day(), x.weekday()),
        (2018, 12, 10, 1)
    );
    let y = dt("2018-12-10T10:30:01");
    assert_eq!(
        (y.year(), y.month(), y.day(), y.weekday()),
        (2018, 12, 10, 1)
    );
    assert_eq!((y.hour(), y.minute(), y.second_nanos()), (10, 30, S));
    let z = dt("2018-12-10");
    assert_eq!((z.hour(), z.minute(), z.second_nanos()), (0, 0, 0));
    assert_eq!(
        dt("2018-12-10T10:30:00+05:00").time_offset(),
        Some(dtd("PT5H"))
    );
    assert_eq!(dt("2018-12-10T10:30:00").time_offset(), None);
    assert_eq!(
        dt("2018-12-10T10:30:00@Etc/UTC").timezone(),
        Some("Etc/UTC")
    );
    assert_eq!(dt("2018-12-10T10:30:00").timezone(), None);
    assert_eq!(dt("2018-12-10T10:30:00+05:00").timezone(), None);
    // named zones: the offset in force at that local date-time
    assert_eq!(
        dt("2018-12-10T10:30:00@Etc/UTC").time_offset(),
        Some(dtd("PT0S"))
    );
    assert_eq!(
        dt("2018-12-10T10:30:00@Europe/Paris").time_offset(),
        Some(dtd("PT1H"))
    );
    assert_eq!(
        dt("2018-07-10T10:30:00@Europe/Paris").time_offset(),
        Some(dtd("PT2H"))
    );
    assert_eq!(
        dt("2018-07-10T10:30:00@America/New_York").time_offset(),
        Some(dtd("-PT4H"))
    );
    assert_eq!(dt("2018-12-10T10:30:00.25").second_nanos(), S / 4);
    assert_eq!(
        dt("2018-12-10T10:30:59.999999999").second_nanos(),
        60 * S - 1
    );
    // negative / large years
    let n = dt("-0044-03-15T12:00:00");
    assert_eq!((n.year(), n.month(), n.day()), (-44, 3, 15));
}

#[test]
fn tck_0074_time_properties() {
    let x = t("10:30:01");
    assert_eq!((x.hour(), x.minute(), x.second_nanos()), (10, 30, S));
    assert_eq!(t("10:30:00+05:00").time_offset(), Some(dtd("PT5H")));
    assert_eq!(
        t("10:30:00-05:30").offset_seconds(),
        Some(-(5 * 3600 + 1800))
    );
    assert_eq!(t("10:30:00").time_offset(), None);
    assert_eq!(t("10:30:00").timezone(), None);
    assert_eq!(t("10:30:00@Etc/UTC").timezone(), Some("Etc/UTC"));
    // named zone on a bare time: standard offset
    assert_eq!(t("10:30:00@Europe/Paris").time_offset(), Some(dtd("PT1H")));
    assert_eq!(
        t("10:30:00@Australia/Melbourne").offset_seconds(),
        Some(36_000)
    );
    assert_eq!(t("23:59:59.5").second_nanos(), 59 * S + S / 2);
    assert_eq!(t("23:59:59.5").hour(), 23);
}

#[test]
fn tck_0074_duration_properties() {
    assert_eq!(ymd("P1Y2M").years(), 1);
    assert_eq!(ymd("P2M").years(), 0);
    assert_eq!(ymd("P2M").months(), 2);
    assert_eq!(ymd("P1Y").months(), 0);
    assert_eq!(ymd("P26M").years(), 2);
    assert_eq!(ymd("P26M").months(), 2);
    assert_eq!(dtd("P1D").days(), 1);
    assert_eq!(dtd("PT2H").days(), 0);
    assert_eq!(dtd("PT2H").hours(), 2);
    assert_eq!(dtd("P1D").hours(), 0);
    assert_eq!(dtd("PT2M").minutes(), 2);
    assert_eq!(dtd("P1D").minutes(), 0);
    assert_eq!(dtd("PT2S").seconds_nanos(), 2 * S);
    assert_eq!(dtd("P1D").seconds_nanos(), 0);
    let x = dtd("P0DT25H61M1.5S");
    assert_eq!(
        (x.days(), x.hours(), x.minutes(), x.seconds_nanos()),
        (1, 2, 1, S + S / 2)
    );
    assert_eq!(dtd("P13DT2H14S").seconds_nanos(), 14 * S);
}

#[test]
fn duration_property_signs() {
    // components carry the sign of the duration (java.time / TCK runners)
    let x = dtd("-P1DT2H3M4.5S");
    assert_eq!((x.days(), x.hours(), x.minutes()), (-1, -2, -3));
    assert_eq!(x.seconds_nanos(), -(4 * S + S / 2));
    let total = x.days() * 86_400 * i128::from(S)
        + i128::from(x.hours()) * 3600 * i128::from(S)
        + i128::from(x.minutes()) * 60 * i128::from(S)
        + i128::from(x.seconds_nanos());
    assert_eq!(total, x.nanos);
    let y = ymd("-P1Y2M");
    assert_eq!((y.years(), y.months()), (-1, -2));
    assert_eq!(ymd("-P11M").years(), 0);
    assert_eq!(ymd("-P11M").months(), -11);
    assert_eq!(dtd("-PT30M").hours(), 0);
    assert_eq!(dtd("-PT30M").minutes(), -30);
}

#[test]
fn tck_0095_day_of_year() {
    assert_eq!(d("1970-01-01").day_of_year(), 1);
    assert_eq!(d("2019-12-31").day_of_year(), 365);
    assert_eq!(d("2020-12-31").day_of_year(), 366);
    assert_eq!(d("2019-09-17").day_of_year(), 260);
    assert_eq!(dt("2019-09-17T10:10:10").date().day_of_year(), 260);
    assert_eq!(d("2020-03-01").day_of_year(), 61);
    assert_eq!(d("1900-03-01").day_of_year(), 60);
    assert_eq!(d("-0004-12-31").day_of_year(), 366);
}

#[test]
fn tck_0096_day_of_week() {
    let start = d("2021-01-11");
    let names: Vec<&str> = (0..7)
        .map(|i| weekday_name(start.add_days(i).unwrap().weekday()))
        .collect();
    assert_eq!(
        names,
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday"
        ]
    );
    assert_eq!(weekday_name(d("2019-09-17").weekday()), "Tuesday");
    assert_eq!(weekday_name(dt("2019-09-17T10:10:10").weekday()), "Tuesday");
    assert_eq!(weekday_name(0), "");
    assert_eq!(weekday_name(8), "");
    assert_eq!(d("1970-01-01").weekday(), 4);
    assert_eq!(d("2000-01-01").weekday(), 6);
    assert_eq!(d("0001-01-01").weekday(), 1);
    assert_eq!(
        d("-999999999-01-01").add_days(7).unwrap().weekday(),
        d("-999999999-01-01").weekday()
    );
}

#[test]
fn tck_0097_month_of_year() {
    let start = d("2021-01-01");
    let names: Vec<&str> = (0..12)
        .map(|i| {
            month_name(
                date_add_ymd(&start, &YearMonthDuration::from_months(i))
                    .unwrap()
                    .month(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December"
        ]
    );
    assert_eq!(month_name(d("2019-09-17").month()), "September");
    assert_eq!(month_name(0), "");
    assert_eq!(month_name(13), "");
}

#[test]
fn tck_0098_week_of_year() {
    for (s, w) in [
        ("2019-09-17", 38),
        ("2003-12-29", 1),
        ("2003-12-31", 1),
        ("2004-01-01", 1),
        ("2004-01-04", 1),
        ("2010-01-01", 53),
        ("1970-01-01", 1),
        ("2005-01-01", 53),
        ("2005-01-03", 1),
        ("2005-01-09", 1),
        ("2008-12-29", 1),
        ("2009-12-31", 53),
        ("2015-12-31", 53),
        ("2016-01-03", 53),
        ("2016-01-04", 1),
        ("2021-01-03", 53),
        ("2024-12-30", 1),
    ] {
        assert_eq!(d(s).iso_week(), w, "{s}");
    }
    assert_eq!(dt("2019-09-17T10:10:10").date().iso_week(), 38);
    assert_eq!(Date::from_ymd(2005, 1, 1).unwrap().iso_week(), 53);
}

#[test]
fn tck_1121_years_and_months_duration() {
    let ym = |a: &str, b: &str| years_and_months_between(&dt(a), &dt(b)).unwrap();
    let ymd_ = |a: &str, b: &str| years_and_months_between_dates(&d(a), &d(b));
    assert_eq!(ymd_("2011-12-22", "2013-08-24").to_string(), "P1Y8M");
    assert_eq!(ymd_("2013-08-24", "2011-12-22").to_string(), "-P1Y8M");
    assert_eq!(ymd_("2015-01-21", "2016-01-21").to_string(), "P1Y");
    assert_eq!(ymd_("2016-01-21", "2015-01-21").to_string(), "-P1Y");
    assert_eq!(ymd_("2016-01-01", "2016-01-01").to_string(), "P0M");
    for (a, b, out) in [
        ("2017-12-31T13:00:00", "2017-12-31T12:00:00", "P0M"),
        ("2016-09-30T23:25:00", "2017-12-28T12:12:12", "P1Y2M"),
        ("2010-05-30T03:55:58", "2017-12-15T00:59:59", "P7Y6M"),
        ("2014-12-31T23:59:59", "2019-10-01T12:32:59", "P4Y9M"),
        ("-2016-01-30T09:05:00", "-2017-02-28T02:02:02", "-P11M"),
        ("2014-12-31T23:59:59", "-2019-10-01T12:32:59", "-P4033Y2M"),
        (
            "2017-09-05T10:20:00-01:00",
            "-2019-10-01T12:32:59+02:00",
            "-P4035Y11M",
        ),
        ("2017-09-05T10:20:00+05:00", "2019-10-01T12:32:59", "P2Y"),
        (
            "2016-08-25T15:20:59+02:00",
            "2017-08-10T10:20:00@Europe/Paris",
            "P11M",
        ),
        (
            "2011-12-31T10:15:30@Etc/UTC",
            "2017-08-10T10:20:00@Europe/Paris",
            "P5Y7M",
        ),
        ("2017-09-05T10:20:00@Etc/UTC", "2018-10-01T23:59:59", "P1Y"),
        (
            "2011-08-25T15:59:59@Europe/Paris",
            "2015-08-25T15:20:59+02:00",
            "P4Y",
        ),
        (
            "2015-12-31T23:59:59.9999999",
            "2018-10-01T12:32:59.111111",
            "P2Y9M",
        ),
        (
            "2016-09-05T22:20:55.123456+05:00",
            "2019-10-01T12:32:59.32415645",
            "P3Y",
        ),
        ("2016-12-31T00:00:01", "2017-12-31T23:59:59", "P1Y"),
        ("2014-12-31T23:59:59", "2016-12-31T00:00:01", "P2Y"),
    ] {
        assert_eq!(ym(a, b).to_string(), out, "{a} -> {b}");
    }
    // date arguments are converted by the caller
    let f = |x: &str| DateTime::from_date(&d(x));
    assert_eq!(
        years_and_months_between(&f("2013-08-24"), &dt("2017-12-15T00:59:59"))
            .unwrap()
            .to_string(),
        "P4Y3M"
    );
    assert_eq!(
        years_and_months_between(&dt("2017-02-28T23:59:59"), &f("2019-07-23"))
            .unwrap()
            .to_string(),
        "P2Y4M"
    );
    // whole months only, truncated toward zero
    assert_eq!(ymd_("2015-01-31", "2015-02-28").to_string(), "P0M");
    assert_eq!(ymd_("2015-01-31", "2015-03-01").to_string(), "P1M");
    assert_eq!(ymd_("2016-01-01", "2015-11-15").to_string(), "-P1M");
    assert_eq!(
        ymd_("-999999999-01-01", "999999999-12-31").as_months(),
        1_999_999_998 * 12 + 11
    );
}

#[test]
fn duration_arithmetic_helpers() {
    // Table 59 helpers (TCK 0100 multiply / divide cases)
    assert_eq!(dtd("P1D").mul_div(10, 1).unwrap(), dtd("P10D"));
    assert_eq!(dtd("-P1D").mul_div(-10, 1).unwrap(), dtd("P10D"));
    assert_eq!(dtd("PT1H").mul_div(26, 1).unwrap().to_string(), "P1DT2H");
    assert_eq!(
        dtd("P4DT1H").mul_div(15, 10).unwrap().to_string(),
        "P6DT1H30M"
    );
    assert_eq!(
        dtd("PT23H").mul_div(25, 10).unwrap().to_string(),
        "P2DT9H30M"
    );
    assert_eq!(
        dtd("P10DT23H").mul_div(10, 25).unwrap().to_string(),
        "P4DT9H12M"
    );
    assert_eq!(dtd("P10D").mul_div(1, 10).unwrap().to_string(), "P1D");
    assert_eq!(dtd("P1D").mul_div(0, 1).unwrap().to_string(), "PT0S");
    assert_eq!(dtd("P1D").mul_div(1, 0), None);
    assert_eq!(
        dtd("PT1S").mul_div(1, 3).unwrap().to_string(),
        "PT0.333333333S"
    );
    assert_eq!(ymd("P1M").mul_div(26, 1).unwrap().to_string(), "P2Y2M");
    assert_eq!(ymd("P2M").mul_div(15, 10).unwrap().to_string(), "P3M");
    assert_eq!(ymd("P2M").mul_div(-15, 10).unwrap().to_string(), "-P3M");
    assert_eq!(
        ymd("P1Y11M").mul_div(-25, 10).unwrap().to_string(),
        "-P4Y9M"
    );
    assert_eq!(ymd("P10Y11M").mul_div(10, 25).unwrap().to_string(), "P4Y4M");
    assert_eq!(
        ymd("P10Y11M").mul_div(-10, 25).unwrap().to_string(),
        "-P4Y4M"
    );
    assert_eq!(ymd("P10Y").mul_div(1, 0), None);
    assert_eq!(ymd("P1M").mul_div(0, 1).unwrap().to_string(), "P0M");
    // negation (TCK 0099) and abs (TCK 0050)
    assert_eq!(dtd("P1D").checked_neg().unwrap().to_string(), "-P1D");
    assert_eq!(dtd("-P1D").checked_neg().unwrap().to_string(), "P1D");
    assert_eq!(ymd("P1Y").checked_neg().unwrap().to_string(), "-P1Y");
    assert_eq!(dtd("-P1D").checked_abs().unwrap(), dtd("P1D"));
    assert_eq!(ymd("-P1Y").checked_abs().unwrap(), ymd("P1Y"));
    // overflow
    assert_eq!(
        DayTimeDuration::from_nanos(i128::MAX).checked_add(&dtd("PT1S")),
        None
    );
    assert_eq!(YearMonthDuration::from_months(i64::MIN).checked_neg(), None);
    // ratio of durations is done by the caller from the raw values
    assert_eq!(ymd("P10Y").as_months() / ymd("P5Y").as_months(), 2);
    assert_eq!(dtd("P10D").as_nanos() / dtd("P5D").as_nanos(), 2);
    assert_eq!(DayTimeDuration::from_seconds(90_000).to_string(), "P1DT1H");
    assert!(
        DayTimeDuration::ZERO.is_zero() && !dtd("-PT1S").is_zero() && dtd("-PT1S").is_negative()
    );
    assert_eq!(Duration::from(dtd("P1D")).to_string(), "P1D");
    assert_eq!(Duration::from(ymd("P13M")).to_string(), "P1Y1M");
}

#[test]
fn names_and_tzdb() {
    assert_eq!(weekday_name(1), "Monday");
    assert_eq!(weekday_name(7), "Sunday");
    assert_eq!(month_name(1), "January");
    assert_eq!(month_name(12), "December");
    assert_eq!(tzdb_version(), "2025b");
}
