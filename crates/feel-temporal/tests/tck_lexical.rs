//! Lexical forms and conversion functions: TCK 1115 (date), 1116 (time),
//! 1117 (date and time), 1120 (duration), 0093 (@-literals), 0007 and the
//! (non-compliant folder) 0079 `string()` forms, plus round trips.

mod common;
use common::*;
use feel_temporal::*;

// ---------------------------------------------------------------- 1115 date

#[test]
fn tck_1115_date_strings_valid() {
    for (s, out) in [
        ("2017-12-31", "2017-12-31"),
        ("2017-01-01", "2017-01-01"),
        ("-2017-12-31", "-2017-12-31"),
        ("-2017-01-01", "-2017-01-01"),
        ("999999999-12-31", "999999999-12-31"),
        ("-999999999-12-31", "-999999999-12-31"),
        ("2012-12-25", "2012-12-25"),
        ("0000-01-01", "0000-01-01"),
        ("-0001-01-01", "-0001-01-01"),
        ("10000-01-01", "10000-01-01"),
        ("2020-02-29", "2020-02-29"),
        ("2000-02-29", "2000-02-29"),
    ] {
        assert_eq!(d(s).to_string(), out, "{s}");
    }
}

#[test]
fn tck_1115_date_strings_invalid() {
    for s in [
        "2012-12-25T",
        "",
        "2012/12/25",
        "0000-12-25T",
        "9999999999-12-25",
        "2017-13-10",
        "2017-12-32",
        "998-12-31",
        "01211-12-31",
        "2012T-12-2511:00:00Z",
        "+2012-12-02",
        // not a date string (a date and time string)
        "2012-12-25T12:00:00",
        // other malformed inputs
        "2017-00-10",
        "2017-1-10",
        "2017-01-1",
        "2019-02-29",
        "1900-02-29",
        "-0000-01-01",
        " 2012-12-25",
        "2012-12-25 ",
        "2012-12-25Z",
        "1000000000-01-01",
        "-1000000000-01-01",
        "2012-12-25+01:00",
        "２０１２-12-25",
    ] {
        assert_eq!(Date::parse(s), None, "{s:?}");
    }
}

#[test]
fn tck_1115_date_from_components() {
    assert_eq!(
        Date::from_ymd(2017, 12, 31).unwrap().to_string(),
        "2017-12-31"
    );
    assert_eq!(
        Date::from_ymd(2017, 1, 1).unwrap().to_string(),
        "2017-01-01"
    );
    assert_eq!(
        Date::from_ymd(-2017, 12, 31).unwrap().to_string(),
        "-2017-12-31"
    );
    assert_eq!(
        Date::from_ymd(-2017, 1, 1).unwrap().to_string(),
        "-2017-01-01"
    );
    assert_eq!(
        Date::from_ymd(999_999_999, 12, 31).unwrap().to_string(),
        "999999999-12-31"
    );
    assert_eq!(
        Date::from_ymd(-999_999_999, 12, 31).unwrap().to_string(),
        "-999999999-12-31"
    );
    assert_eq!(Date::from_ymd(2017, 8, 30).unwrap(), d("2017-08-30"));
    assert_eq!(Date::from_ymd(2012, 12, 25).unwrap(), d("2012-12-25"));
    for (y, m, dd) in [
        (2017, 13, 31),
        (2017, 12, 32),
        (2017, -8, 2),
        (2017, 8, -2),
        (-1_000_999_999, 12, 1),
        (1_000_999_999, 12, 32),
        (1_000_000_000, 1, 1),
        (2017, 0, 1),
        (2017, 1, 0),
        (2019, 2, 29),
        (2017, 4, 31),
    ] {
        assert_eq!(Date::from_ymd(y, m, dd), None, "{y} {m} {dd}");
    }
}

#[test]
fn tck_1115_date_of_date_and_time() {
    for (s, out) in [
        ("2017-08-14T14:25:00", "2017-08-14"),
        ("2017-08-14T14:25:00.123456789", "2017-08-14"),
        ("2017-09-03T09:45:30@Europe/Paris", "2017-09-03"),
        ("2017-09-06T09:45:30@Asia/Dhaka", "2017-09-06"),
        ("2012-12-25T11:00:00Z", "2012-12-25"),
        ("2017-08-03T10:15:30+01:00", "2017-08-03"),
    ] {
        assert_eq!(dt(s).date().to_string(), out, "{s}");
    }
    // date(date and time(date and time("2017-08-14T14:25:00"), time("10:50:00")))
    let x = DateTime::from_date_time(&dt("2017-08-14T14:25:00").date(), &t("10:50:00"));
    assert_eq!(x.date().to_string(), "2017-08-14");
}

// ---------------------------------------------------------------- 1116 time

#[test]
fn tck_1116_time_strings_valid() {
    for (s, out) in [
        ("01:02:03", "01:02:03"),
        ("00:00:00", "00:00:00"),
        ("11:22:33.444", "11:22:33.444"),
        ("11:22:33.123456789", "11:22:33.123456789"),
        ("23:59:00Z", "23:59:00Z"),
        ("11:00:00Z", "11:00:00Z"),
        ("00:00:00Z", "00:00:00Z"),
        ("13:20:00+02:00", "13:20:00+02:00"),
        ("13:20:00-05:00", "13:20:00-05:00"),
        ("11:22:33-00:00", "11:22:33Z"),
        ("11:22:33+00:00", "11:22:33Z"),
        ("00:01:00@Etc/UTC", "00:01:00@Etc/UTC"),
        ("00:01:00@Europe/Paris", "00:01:00@Europe/Paris"),
        ("12:45:00", "12:45:00"),
        // spec examples use a lower-case z
        ("23:59:00z", "23:59:00Z"),
        // trailing zeros of the fraction are not significant
        ("10:30:00.0000", "10:30:00"),
        ("10:30:00.0001", "10:30:00.0001"),
        ("10:30:00.100", "10:30:00.1"),
        // more than 9 fraction digits are truncated to nanoseconds
        ("10:30:00.1234567891", "10:30:00.123456789"),
        // offsets up to +-14:00, with optional seconds
        ("10:30:00+14:00", "10:30:00+14:00"),
        ("10:30:00-14:00", "10:30:00-14:00"),
        ("10:30:00.0001+05:00:01", "10:30:00.0001+05:00:01"),
        // XSD end-of-day form
        ("24:00:00", "00:00:00"),
        ("24:00:00Z", "00:00:00Z"),
        (
            "10:30:11@Australia/Melbourne",
            "10:30:11@Australia/Melbourne",
        ),
    ] {
        assert_eq!(t(s).to_string(), out, "{s}");
    }
}

#[test]
fn tck_1116_time_strings_invalid() {
    for s in [
        "",
        "23:59:60",
        "24:00:01",
        "24:01:00",
        "24:00:00.5",
        "25:00:00",
        "00:60:00",
        "00:00:61",
        "7:00:00",
        "07:1:00",
        "07:01:2",
        "13:20:00@xyz/abc",
        "13:20:00+19:00",
        "13:20:00-19:00",
        "13:20:00+14:01",
        "13:20:00+5:00",
        "13:20:00+5",
        "13:20:00+05:0",
        "13:20:00+02:00@Europe/Paris",
        "7:20",
        "07:2",
        "11:30:00T",
        "T11:30:00",
        "2012T-12-2511:00:00Z",
        "10:30",
        "10:30:00.",
        "10:30:00Z@Europe/Paris",
        "10:30:00@",
        "10:30:00@europe/paris",
        "10:30:00 Z",
        "10:30:00+05:60",
    ] {
        assert_eq!(Time::parse(s), None, "{s:?}");
    }
}

#[test]
fn tck_1116_time_from_components() {
    let sec = NANOS_PER_SEC;
    let off = |s: &str| Zone::from_offset_duration(&dtd(s));
    assert_eq!(Time::new(12, 0, 0, None).unwrap().to_string(), "12:00:00");
    assert_eq!(
        Time::new(11, 59, 45 * sec, None).unwrap().to_string(),
        "11:59:45"
    );
    for (h, m, s, o, out) in [
        (11, 59, 45, "PT0H", "11:59:45Z"),
        (11, 59, 45, "PT2H", "11:59:45+02:00"),
        (11, 59, 45, "-PT2H", "11:59:45-02:00"),
        (11, 59, 0, "PT2H1M", "11:59:00+02:01"),
        (11, 59, 0, "-PT2H1M", "11:59:00-02:01"),
        (11, 59, 0, "PT2H1M0S", "11:59:00+02:01"),
        (11, 59, 0, "-PT2H1M0S", "11:59:00-02:01"),
        (11, 59, 45, "PT2H45M55S", "11:59:45+02:45:55"),
        (11, 59, 45, "-PT2H45M55S", "11:59:45-02:45:55"),
        (11, 59, 45, "-PT0H", "11:59:45Z"),
        (23, 59, 0, "PT0H", "23:59:00Z"),
    ] {
        assert_eq!(Time::new(h, m, s * sec, off(o)).unwrap().to_string(), out);
    }
    // TCK 0007: time(12, 59, 1.3, -PT1H)
    assert_eq!(
        Time::new(12, 59, 1_300_000_000, off("-PT1H"))
            .unwrap()
            .to_string(),
        "12:59:01.3-01:00"
    );
    for (h, m, s) in [
        (24, 59, 45),
        (-24, 59, 45),
        (23, 60, 45),
        (23, 59, 60),
        (-1, 0, 0),
        (0, -1, 0),
    ] {
        assert_eq!(Time::new(h, m, s * sec, None), None, "{h} {m} {s}");
    }
    assert_eq!(Time::new(0, 0, -1, None), None);
    assert!(Time::new(23, 59, 60 * sec - 1, None).is_some());
    // offsets out of range / not whole seconds
    assert_eq!(off("PT15H"), None);
    assert_eq!(off("PT0.5S"), None);
    assert_eq!(off("P1D"), None);
    assert_eq!(Time::new(1, 0, 0, Some(Zone::Offset(15 * 3600))), None);
    assert_eq!(
        Time::new(1, 0, 0, Some(Zone::Named("No/Where".into()))),
        None
    );
}

#[test]
fn tck_1116_time_of_date_and_time() {
    for (s, out) in [
        ("2017-08-10T10:20:00", "10:20:00"),
        ("2017-08-10T10:20:00+00:00", "10:20:00Z"),
        ("2017-08-10T10:20:00-00:00", "10:20:00Z"),
        ("2017-08-10T10:20:00+01:00", "10:20:00+01:00"),
        ("2017-08-10T10:20:00-01:00", "10:20:00-01:00"),
        ("2017-08-10T10:20:00Z", "10:20:00Z"),
        ("2017-08-10T10:20:00@Europe/Paris", "10:20:00@Europe/Paris"),
        ("2017-09-04T11:20:00@Asia/Dhaka", "11:20:00@Asia/Dhaka"),
        ("2012-12-24T23:59:00", "23:59:00"),
    ] {
        assert_eq!(dt(s).time().to_string(), out, "{s}");
    }
    let base = dt("2017-08-10T10:20:00").date();
    for (ts, out) in [
        ("23:59:01", "23:59:01"),
        ("23:59:01.987654321", "23:59:01.987654321"),
        ("09:15:30+02:00", "09:15:30+02:00"),
        ("09:15:30Z", "09:15:30Z"),
    ] {
        assert_eq!(
            DateTime::from_date_time(&base, &t(ts)).time().to_string(),
            out
        );
    }
    // time(date("2017-08-10")): a date converts to UTC midnight
    assert_eq!(
        DateTime::from_date_utc(&d("2017-08-10")).time().to_string(),
        "00:00:00Z"
    );
}

// ------------------------------------------------------- 1117 date and time

#[test]
fn tck_1117_date_and_time_strings_valid() {
    for (s, out) in [
        ("2012-12-24", "2012-12-24T00:00:00"),
        ("2017-12-31T00:00:00", "2017-12-31T00:00:00"),
        ("2017-12-31T11:22:33", "2017-12-31T11:22:33"),
        ("-2017-12-31T11:22:33", "-2017-12-31T11:22:33"),
        ("99999-12-31T11:22:33", "99999-12-31T11:22:33"),
        ("-99999-12-31T11:22:33", "-99999-12-31T11:22:33"),
        ("2017-12-31T11:22:33.345", "2017-12-31T11:22:33.345"),
        (
            "2017-12-31T11:22:33.123456789",
            "2017-12-31T11:22:33.123456789",
        ),
        ("2017-12-31T11:22:33Z", "2017-12-31T11:22:33Z"),
        ("2017-12-31T11:22:33.567Z", "2017-12-31T11:22:33.567Z"),
        ("2017-12-31T11:22:33+01:00", "2017-12-31T11:22:33+01:00"),
        ("2017-12-31T11:22:33-02:00", "2017-12-31T11:22:33-02:00"),
        ("2017-12-31T11:22:33+01:35", "2017-12-31T11:22:33+01:35"),
        ("2017-12-31T11:22:33-01:35", "2017-12-31T11:22:33-01:35"),
        (
            "2017-12-31T11:22:33.456+01:35",
            "2017-12-31T11:22:33.456+01:35",
        ),
        (
            "-2017-12-31T11:22:33.456+01:35",
            "-2017-12-31T11:22:33.456+01:35",
        ),
        (
            "2011-12-31T10:15:30@Europe/Paris",
            "2011-12-31T10:15:30@Europe/Paris",
        ),
        ("2011-12-31T10:15:30@Etc/UTC", "2011-12-31T10:15:30@Etc/UTC"),
        (
            "2011-12-31T10:15:30.987@Europe/Paris",
            "2011-12-31T10:15:30.987@Europe/Paris",
        ),
        (
            "2011-12-31T10:15:30.123456789@Europe/Paris",
            "2011-12-31T10:15:30.123456789@Europe/Paris",
        ),
        (
            "999999999-12-31T23:59:59.999999999@Europe/Paris",
            "999999999-12-31T23:59:59.999999999@Europe/Paris",
        ),
        (
            "-999999999-12-31T23:59:59.999999999+02:00",
            "-999999999-12-31T23:59:59.999999999+02:00",
        ),
        ("2012-12-24T23:59:00", "2012-12-24T23:59:00"),
        ("2018-12-10T10:30:00.0001", "2018-12-10T10:30:00.0001"),
        (
            "2018-12-10T10:30:00.0001+05:00:01",
            "2018-12-10T10:30:00.0001+05:00:01",
        ),
        ("2016-12-24T23:59:00-08:00", "2016-12-24T23:59:00-08:00"),
        // XSD end-of-day
        ("2021-01-01T24:00:00", "2021-01-02T00:00:00"),
        ("2020-12-31T24:00:00Z", "2021-01-01T00:00:00Z"),
        ("2012-12-25T11:00:00z", "2012-12-25T11:00:00Z"),
    ] {
        assert_eq!(dt(s).to_string(), out, "{s}");
    }
}

#[test]
fn tck_1117_date_and_time_strings_invalid() {
    for s in [
        "",
        "11:00:00",
        "2011-12-0310:15:30",
        "2011-12-03T10:15:30+01:00@Europe/Paris",
        "9999999999-12-27T11:22:33",
        "2017-13-10T11:22:33",
        "2017-00-10T11:22:33",
        "2017-13-32T11:22:33",
        "2017-13-0T11:22:33",
        "998-12-31T11:22:33",
        "01211-12-31T11:22:33",
        "+99999-12-01T11:22:33",
        "2017-12-31T24:00:01",
        "2017-12-31T24:01:00",
        "2017-12-31T25:00:00",
        "2017-12-31T00:60:00",
        "2017-12-31T00:00:61",
        "2017-12-31T7:00:00",
        "2017-12-31T07:1:00",
        "2017-12-31T07:01:2",
        "2017-12-31T13:20:00@xyz/abc",
        "2017-12-31T13:20:00+19:00",
        "2017-12-31T13:20:00-19:00",
        "2017-12-31T13:20:00+05:0",
        "2017-12-31T13:20:00+5:00",
        "2017-12-31T13:20:00+5",
        "2017-12-31T13:20:00+02:00@Europe/Paris",
        "2017-12-31T7:20",
        "2017-12-31T07:2",
        "2012-12-25T",
        "2012-12-25t10:00:00",
        "2012-12-25 10:00:00",
        "2012-12-25Z",
        "999999999-12-31T24:00:00",
        "2012T-12-2511:00:00Z",
    ] {
        assert_eq!(DateTime::parse(s), None, "{s:?}");
    }
}

#[test]
fn tck_1117_date_and_time_from_date_and_time() {
    for (ds, ts, out) in [
        ("2017-01-01", "23:59:01", "2017-01-01T23:59:01"),
        ("2017-01-01", "23:59:01Z", "2017-01-01T23:59:01Z"),
        ("2017-01-01", "23:59:01+02:00", "2017-01-01T23:59:01+02:00"),
        (
            "2017-01-01",
            "23:59:01@Europe/Paris",
            "2017-01-01T23:59:01@Europe/Paris",
        ),
        (
            "2017-01-01",
            "23:59:01.123456789@Europe/Paris",
            "2017-01-01T23:59:01.123456789@Europe/Paris",
        ),
    ] {
        assert_eq!(DateTime::from_date_time(&d(ds), &t(ts)).to_string(), out);
    }
    // the date argument may be a date and time: its time and zone are ignored
    for (dts, ts, out) in [
        ("2017-08-10T10:20:00", "23:59:01", "2017-08-10T23:59:01"),
        (
            "2017-08-10T10:20:00",
            "23:59:01.987654321",
            "2017-08-10T23:59:01.987654321",
        ),
        (
            "2017-09-05T10:20:00",
            "09:15:30+02:00",
            "2017-09-05T09:15:30+02:00",
        ),
        ("2017-09-05T10:20:00", "09:15:30Z", "2017-09-05T09:15:30Z"),
        (
            "2017-08-10T10:20:00+02:00",
            "23:59:01",
            "2017-08-10T23:59:01",
        ),
        (
            "2017-09-05T10:20:00-01:00",
            "09:15:30+02:00",
            "2017-09-05T09:15:30+02:00",
        ),
        (
            "2017-09-05T10:20:00-01:00",
            "09:15:30Z",
            "2017-09-05T09:15:30Z",
        ),
        (
            "2017-09-05T10:20:00+02:00",
            "09:15:30.123456Z",
            "2017-09-05T09:15:30.123456Z",
        ),
        (
            "2017-09-05T10:20:00-01:00",
            "09:15:30.987654321@Europe/Paris",
            "2017-09-05T09:15:30.987654321@Europe/Paris",
        ),
        (
            "2017-08-10T10:20:00@Europe/Paris",
            "23:59:01",
            "2017-08-10T23:59:01",
        ),
        (
            "2017-09-05T10:20:00@Europe/Paris",
            "09:15:30Z",
            "2017-09-05T09:15:30Z",
        ),
        ("2017-01-01T00:00:00", "23:59:01", "2017-01-01T23:59:01"),
    ] {
        assert_eq!(
            DateTime::from_date_time(&dt(dts).date(), &t(ts)).to_string(),
            out
        );
    }
}

#[test]
fn date_to_date_and_time_conversions() {
    // explicit date and time(date string): local midnight, no zone
    assert_eq!(DateTime::from_date(&d("2012-12-24")), dt("2012-12-24"));
    assert_eq!(DateTime::from_date(&d("2012-12-24")).zone, None);
    // implicit conversion (TCK 1157): UTC midnight
    assert_eq!(
        DateTime::from_date_utc(&d("2000-01-02")).to_string(),
        "2000-01-02T00:00:00Z"
    );
    assert_eq!(
        DateTime::from_date_utc(&d("2000-12-01")).to_string(),
        "2000-12-01T00:00:00Z"
    );
}

// ------------------------------------------------------------ 1120 duration

#[test]
fn tck_1120_duration_strings() {
    for (s, out) in [
        ("P1D", "P1D"),
        ("PT2H", "PT2H"),
        ("PT3M", "PT3M"),
        ("PT4S", "PT4S"),
        ("PT0.999S", "PT0.999S"),
        ("P1DT2H3M4.123456789S", "P1DT2H3M4.123456789S"),
        ("PT0S", "PT0S"),
        ("PT0.000S", "PT0S"),
        ("PT0.S", "PT0S"),
        ("PT0M", "PT0S"),
        ("PT0H", "PT0S"),
        ("P0D", "PT0S"),
        ("-PT1H2M", "-PT1H2M"),
        ("PT1000M", "PT16H40M"),
        ("PT1000M0.999999999S", "PT16H40M0.999999999S"),
        ("PT555M", "PT9H15M"),
        ("PT61M", "PT1H1M"),
        ("PT24H", "P1D"),
        ("PT240H", "P10D"),
        ("P2DT100M", "P2DT1H40M"),
        ("PT3600S", "PT1H"),
        ("P2DT274M", "P2DT4H34M"),
        ("P1Y2M", "P1Y2M"),
        ("P1Y", "P1Y"),
        ("P0Y", "P0M"),
        ("P0M", "P0M"),
        ("-P1Y", "-P1Y"),
        ("P26M", "P2Y2M"),
        ("P1Y27M", "P3Y3M"),
        ("P100M", "P8Y4M"),
        ("-P100M", "-P8Y4M"),
        ("P999999999M", "P83333333Y3M"),
        ("-P999999999M", "-P83333333Y3M"),
        ("P99999999Y", "P99999999Y"),
        ("-P99999999Y", "-P99999999Y"),
        // spec 10.3.2.3.7/8 normalization examples
        ("P0DT25H", "P1DT1H"),
        ("P0Y13M", "P1Y1M"),
        // other forms
        ("-P0D", "PT0S"),
        ("-PT0S", "PT0S"),
        ("-P0Y", "P0M"),
        ("PT.5S", "PT0.5S"),
        ("PT1.000000000001S", "PT1S"),
        ("P13DT2H14S", "P13DT2H14S"),
        ("PT49H", "P2DT1H"),
        ("P25M", "P2Y1M"),
        ("P1DT2H3M4.1234S", "P1DT2H3M4.1234S"),
        ("-PT0.001S", "-PT0.001S"),
        ("P1000000000D", "P1000000000D"),
    ] {
        let v = parse_duration(s).unwrap_or_else(|| panic!("{s}"));
        assert_eq!(v.to_string(), out, "{s}");
        // round trip through the canonical form
        assert_eq!(parse_duration(&v.to_string()), Some(v), "{s}");
    }
    assert!(matches!(parse_duration("P1D"), Some(Duration::DayTime(_))));
    assert!(matches!(
        parse_duration("P0Y"),
        Some(Duration::YearMonth(_))
    ));
    assert!(matches!(
        parse_duration("P0M"),
        Some(Duration::YearMonth(_))
    ));
    assert!(matches!(parse_duration("PT0M"), Some(Duration::DayTime(_))));
}

#[test]
fn tck_1120_duration_strings_invalid() {
    for s in [
        "",
        "2012T-12-2511:00:00Z",
        "P",
        "P0",
        "1Y",
        "1D",
        "P1H",
        "P1S",
        "PT",
        "P1DT",
        "-P",
        "+P1D",
        "P-1D",
        "PT1.5H",
        "P1.5D",
        "P1.5Y",
        "PT.S",
        "PT1H1H",
        "PT1M1H",
        "P1DT1D",
        "P1M1Y",
        "PT1HT1M",
        "P1W",
        // mixed year-month / day-time forms are not FEEL durations
        "P1Y2D",
        "P1Y2M3DT4H5M6S",
        "P1MT1H",
        "P1Y0D",
        "p1d",
        "P1d",
        " P1D",
        "P1D ",
        "P99999999999999999999999999999999999999999D",
        "P9999999999999999999Y",
    ] {
        assert_eq!(parse_duration(s), None, "{s:?}");
    }
    assert_eq!(DayTimeDuration::parse("P1Y"), None);
    assert_eq!(YearMonthDuration::parse("P1D"), None);
}

// --------------------------------------------------------------- 0093 at

#[test]
fn tck_0093_at_literals() {
    assert_eq!(at("foo"), V::Null);
    assert!(matches!(at("2019-03-31"), V::Date(_)));
    assert_eq!(show(&at("2019-03-31")), "2019-03-31");
    assert!(matches!(at("2018-12-08T10:30:11"), V::DateTime(_)));
    assert_eq!(show(&at("2018-12-08T10:30:11")), "2018-12-08T10:30:11");
    assert_eq!(
        show(&at("2018-12-08T10:30:11@Australia/Melbourne")),
        "2018-12-08T10:30:11@Australia/Melbourne"
    );
    assert_eq!(
        show(&at("2018-12-08T10:30:11+11:00")),
        "2018-12-08T10:30:11+11:00"
    );
    assert!(matches!(at("10:30:11"), V::Time(_)));
    assert_eq!(show(&at("10:30:11+11:00")), "10:30:11+11:00");
    assert_eq!(
        show(&at("10:30:11@Australia/Melbourne")),
        "10:30:11@Australia/Melbourne"
    );
    assert!(matches!(at("P10D"), V::Dtd(_)));
    assert_eq!(show(&at("P10D")), "P10D");
    assert!(matches!(at("P10Y"), V::Ymd(_)));
    assert_eq!(show(&at("P10Y")), "P10Y");
}

// ----------------------------------------------------------- round trips

#[test]
fn display_parse_round_trips() {
    for s in [
        "2012-12-25",
        "-0001-01-01",
        "0000-02-29",
        "10000-01-01",
        "-999999999-01-01",
        "999999999-12-31",
    ] {
        assert_eq!(d(s).to_string(), s);
        assert_eq!(d(s).to_string().parse::<Date>().unwrap(), d(s));
    }
    for s in [
        "11:00:00",
        "11:00:00.123",
        "11:00:00Z",
        "11:00:00+02:00",
        "11:00:00-09:30",
        "11:00:00@Europe/Paris",
        "23:59:59.999999999+14:00",
        "11:59:45+02:45:55",
    ] {
        assert_eq!(t(s).to_string(), s);
        assert_eq!(s.parse::<Time>().unwrap(), t(s));
    }
    for s in [
        "2012-12-25T11:00:00",
        "2012-12-25T11:00:00Z",
        "2012-12-25T11:00:00+02:00",
        "2012-12-25T11:00:00@Europe/Paris",
        "-0044-03-15T12:00:00.5@Europe/Rome",
        "123456789-01-01T00:00:00.000000001-14:00",
    ] {
        assert_eq!(dt(s).to_string(), s);
        assert_eq!(s.parse::<DateTime>().unwrap(), dt(s));
    }
    assert!("nope".parse::<DateTime>().is_err());
    assert!("nope".parse::<Duration>().is_err());
    assert_eq!("P1D".parse::<DayTimeDuration>().unwrap(), dtd("PT24H"));
    assert_eq!("P1Y".parse::<YearMonthDuration>().unwrap(), ymd("P12M"));
}

#[test]
fn tck_0007_date_time() {
    let date_time = dt("2016-12-24T23:59:00-08:00");
    assert_eq!(date_time.to_string(), "2016-12-24T23:59:00-08:00");
    let from_string = d("2015-12-24");
    assert_eq!(date_time.date().to_string(), "2016-12-24");
    assert_eq!(
        Date::from_ymd(1999, 11, 22).unwrap().to_string(),
        "1999-11-22"
    );
    let time = t("00:00:01-01:00");
    assert_eq!(time.to_string(), "00:00:01-01:00");
    let date_time2 = DateTime::from_date_time(&from_string, &time);
    assert_eq!(date_time2.to_string(), "2015-12-24T00:00:01-01:00");
    assert_eq!(date_time2.time().to_string(), "00:00:01-01:00");
    let dtd1 = dtd("P13DT2H14S");
    let dtd2 = datetime_sub(&date_time, &date_time2).unwrap();
    assert_eq!(dtd2.to_string(), "P367DT6H58M59S");
    assert_eq!(
        dtd1.checked_add(&dtd2).unwrap().to_string(),
        "P380DT8H59M13S"
    );
    let ym2 = years_and_months_between(&date_time2, &date_time).unwrap();
    assert_eq!(ym2, ymd("P1Y0M"));
    assert_eq!(from_string.day(), 24);
    assert_eq!(from_string.year(), 2015);
    assert_eq!(from_string.month(), 12);
    assert_eq!(date_time2.hour(), 0);
    assert_eq!(date_time2.minute(), 0);
    assert_eq!(date_time2.second_nanos(), NANOS_PER_SEC);
    assert_eq!(date_time2.time_offset(), Some(dtd("-PT1H")));
    assert_eq!(ym2.years(), 1);
    assert_eq!(dtd1.seconds_nanos(), 14 * NANOS_PER_SEC);
}
