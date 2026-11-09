//! Named zones: offset resolution, DST gaps / overlaps (Europe/Paris,
//! America/New_York, Australia/Melbourne), instant arithmetic and far
//! past / future projection.

mod common;
use common::*;
use core::cmp::Ordering::*;
use feel_temporal::*;

const H: i32 = 3600;

fn off(s: &str) -> i32 {
    dt(s).offset_seconds().unwrap()
}

fn epoch_secs(s: &str) -> i128 {
    dt(s).epoch_nanos().unwrap() / i128::from(NANOS_PER_SEC)
}

#[test]
fn paris_offsets_around_transitions() {
    // spring forward 2021-03-28 02:00 -> 03:00
    assert_eq!(off("2021-03-28T01:59:59@Europe/Paris"), H);
    assert_eq!(off("2021-03-28T03:00:00@Europe/Paris"), 2 * H);
    // gap: offset before the transition (02:30 local = 01:30Z = 03:30+02:00)
    assert_eq!(off("2021-03-28T02:00:00@Europe/Paris"), H);
    assert_eq!(off("2021-03-28T02:30:00@Europe/Paris"), H);
    assert_eq!(
        dt("2021-03-28T02:30:00@Europe/Paris").epoch_nanos(),
        dt("2021-03-28T01:30:00Z").epoch_nanos()
    );
    assert_eq!(
        eq_datetime(
            &dt("2021-03-28T02:30:00@Europe/Paris"),
            &dt("2021-03-28T03:30:00+02:00")
        ),
        Some(true)
    );
    // fall back 2021-10-31 03:00 -> 02:00: overlap uses the earlier instant (+02:00)
    assert_eq!(off("2021-10-31T01:59:59@Europe/Paris"), 2 * H);
    assert_eq!(off("2021-10-31T02:00:00@Europe/Paris"), 2 * H);
    assert_eq!(off("2021-10-31T02:30:00@Europe/Paris"), 2 * H);
    assert_eq!(off("2021-10-31T03:00:00@Europe/Paris"), H);
    assert_eq!(
        epoch_secs("2021-10-31T02:30:00@Europe/Paris"),
        epoch_secs("2021-10-31T00:30:00Z")
    );
}

#[test]
fn new_york_offsets_around_transitions() {
    // spring forward 2021-03-14 02:00 -> 03:00 (EST -5 -> EDT -4)
    assert_eq!(off("2021-03-14T01:59:59@America/New_York"), -5 * H);
    assert_eq!(off("2021-03-14T02:30:00@America/New_York"), -5 * H);
    assert_eq!(off("2021-03-14T03:00:00@America/New_York"), -4 * H);
    // fall back 2021-11-07 02:00 -> 01:00: 01:30 is ambiguous -> EDT (earlier)
    assert_eq!(off("2021-11-07T01:30:00@America/New_York"), -4 * H);
    assert_eq!(off("2021-11-07T02:00:00@America/New_York"), -5 * H);
    assert_eq!(off("2021-07-04T12:00:00@America/New_York"), -4 * H);
    assert_eq!(off("2021-01-04T12:00:00@America/New_York"), -5 * H);
    // historic: no DST in 1900 (LMT until 1883, then EST)
    assert_eq!(off("1900-07-01T12:00:00@America/New_York"), -5 * H);
    assert_eq!(
        off("1850-07-01T12:00:00@America/New_York"),
        -(4 * H + 56 * 60 + 2)
    );
}

#[test]
fn southern_hemisphere() {
    assert_eq!(off("2021-01-15T12:00:00@Australia/Melbourne"), 11 * H);
    assert_eq!(off("2021-07-15T12:00:00@Australia/Melbourne"), 10 * H);
    assert_eq!(off("2021-04-02T23:00:00@Australia/Melbourne"), 11 * H);
    assert_eq!(off("2021-10-02T23:00:00@Australia/Melbourne"), 10 * H);
}

#[test]
fn instant_arithmetic_across_dst() {
    let add = |s: &str, d: &str| datetime_add_dtd(&dt(s), &dtd(d)).unwrap().to_string();
    let sub = |s: &str, d: &str| datetime_sub_dtd(&dt(s), &dtd(d)).unwrap().to_string();
    assert_eq!(
        add("2021-03-28T01:30:00@Europe/Paris", "PT1H"),
        "2021-03-28T03:30:00@Europe/Paris"
    );
    assert_eq!(
        add("2021-03-27T12:00:00@Europe/Paris", "PT24H"),
        "2021-03-28T13:00:00@Europe/Paris"
    );
    assert_eq!(
        add("2021-03-27T12:00:00@Europe/Paris", "P1D"),
        "2021-03-28T13:00:00@Europe/Paris"
    );
    assert_eq!(
        add("2021-10-30T12:00:00@Europe/Paris", "P1D"),
        "2021-10-31T11:00:00@Europe/Paris"
    );
    assert_eq!(
        sub("2021-03-28T03:30:00@Europe/Paris", "PT1H"),
        "2021-03-28T01:30:00@Europe/Paris"
    );
    // a non-existent local time is normalized by instant arithmetic
    assert_eq!(
        add("2021-03-28T02:30:00@Europe/Paris", "PT0S"),
        "2021-03-28T03:30:00@Europe/Paris"
    );
    // through the overlap: 00:30Z + 1h = 01:30Z = 02:30+01:00 (second 02:30)
    let x = datetime_add_dtd(&dt("2021-10-31T02:30:00@Europe/Paris"), &dtd("PT1H")).unwrap();
    assert_eq!(x.to_string(), "2021-10-31T02:30:00@Europe/Paris");
    // ... whose offset resolves to the earlier instant again (documented policy)
    assert_eq!(x.offset_seconds(), Some(2 * H));
    assert_eq!(
        add("2021-11-07T00:30:00@America/New_York", "PT3H"),
        "2021-11-07T02:30:00@America/New_York"
    );
    // fixed offsets: plain local arithmetic
    assert_eq!(
        add("2021-03-27T12:00:00+01:00", "P1D"),
        "2021-03-28T12:00:00+01:00"
    );
    assert_eq!(add("2021-03-27T12:00:00", "P1D"), "2021-03-28T12:00:00");
    // year and month arithmetic keeps the local time
    let ym = |s: &str, d: &str| datetime_add_ymd(&dt(s), &ymd(d)).unwrap().to_string();
    assert_eq!(
        ym("2021-02-28T02:30:00@Europe/Paris", "P1M"),
        "2021-03-28T02:30:00@Europe/Paris"
    );
    assert_eq!(
        ym("2021-01-01T10:10:10@Australia/Melbourne", "P1M"),
        "2021-02-01T10:10:10@Australia/Melbourne"
    );
}

#[test]
fn subtraction_across_dst() {
    let s = |a: &str, b: &str| datetime_sub(&dt(a), &dt(b)).unwrap().to_string();
    assert_eq!(
        s(
            "2021-03-28T12:00:00@Europe/Paris",
            "2021-03-27T12:00:00@Europe/Paris"
        ),
        "PT23H"
    );
    assert_eq!(
        s(
            "2021-10-31T12:00:00@Europe/Paris",
            "2021-10-30T12:00:00@Europe/Paris"
        ),
        "P1DT1H"
    );
    assert_eq!(
        s(
            "2021-01-02T10:10:10@Europe/Paris",
            "2021-01-01T10:10:10@Asia/Dhaka"
        ),
        "P1DT5H"
    );
    assert_eq!(
        s(
            "2021-07-02T10:10:10@Europe/Paris",
            "2021-07-02T10:10:10@America/New_York"
        ),
        "-PT6H"
    );
    assert_eq!(
        s(
            "2021-03-14T03:00:00@America/New_York",
            "2021-03-14T01:00:00@America/New_York"
        ),
        "PT1H"
    );
    // unzoned values are local: no DST
    assert_eq!(s("2021-03-28T12:00:00", "2021-03-27T12:00:00"), "P1D");
    // mixed zoned / unzoned is undefined
    assert_eq!(
        datetime_sub(
            &dt("2021-03-28T12:00:00@Europe/Paris"),
            &dt("2021-03-27T12:00:00")
        ),
        None
    );
    assert_eq!(
        datetime_sub(&dt("2021-03-28T12:00:00"), &dt("2021-03-27T12:00:00Z")),
        None
    );
}

#[test]
fn comparisons_with_named_zones() {
    let c = |a: &str, b: &str| cmp_datetime(&dt(a), &dt(b)).unwrap();
    assert_eq!(
        c("2021-07-01T12:00:00@Europe/Paris", "2021-07-01T10:00:00Z"),
        Equal
    );
    assert_eq!(
        c("2021-01-01T12:00:00@Europe/Paris", "2021-01-01T11:00:00Z"),
        Equal
    );
    assert_eq!(
        c(
            "2021-07-01T12:00:00@Europe/Paris",
            "2021-07-01T12:00:00@America/New_York"
        ),
        Less
    );
    assert_eq!(
        c("2021-10-31T02:30:00@Europe/Paris", "2021-10-31T01:30:00Z"),
        Less
    );
}

#[test]
fn reconstruct_from_epoch() {
    let x = dt("2021-07-01T12:34:56.789@Europe/Paris");
    let e = x.epoch_nanos().unwrap();
    assert_eq!(
        DateTime::from_epoch_nanos(e, Zone::named("Europe/Paris").unwrap()).unwrap(),
        x
    );
    assert_eq!(
        DateTime::from_epoch_nanos(e, Zone::utc())
            .unwrap()
            .to_string(),
        "2021-07-01T10:34:56.789Z"
    );
    assert_eq!(
        DateTime::from_epoch_nanos(e, Zone::named("Asia/Kolkata").unwrap())
            .unwrap()
            .to_string(),
        "2021-07-01T16:04:56.789@Asia/Kolkata"
    );
    assert_eq!(DateTime::from_epoch_nanos(i128::MAX, Zone::utc()), None);
    assert_eq!(dt("2021-07-01T12:00:00").epoch_nanos(), None);
    assert_eq!(dt("1970-01-01T00:00:00Z").epoch_nanos(), Some(0));
    assert_eq!(dt("1970-01-01T01:00:00+01:00").epoch_nanos(), Some(0));
    assert_eq!(dt("1970-01-01T00:00:00").local_epoch_nanos(), 0);
}

#[test]
fn far_past_and_future() {
    // before the first transition: local mean time (Paris LMT +00:09:21)
    assert_eq!(off("1800-07-01T12:00:00@Europe/Paris"), 561);
    assert_eq!(off("-999999999-07-01T12:00:00@Europe/Paris"), 561);
    // after chrono-tz's explicit data (2099) the current rules are projected
    assert_eq!(off("2099-07-01T12:00:00@Europe/Paris"), 2 * H);
    assert_eq!(off("2100-07-01T12:00:00@Europe/Paris"), 2 * H);
    assert_eq!(off("2100-01-01T12:00:00@Europe/Paris"), H);
    assert_eq!(off("3000-07-01T12:00:00@Europe/Paris"), 2 * H);
    assert_eq!(off("999999999-07-01T12:00:00@Europe/Paris"), 2 * H);
    assert_eq!(off("999999999-12-31T23:59:59.999999999@Europe/Paris"), H);
    assert_eq!(off("2100-01-15T12:00:00@Australia/Melbourne"), 11 * H);
    assert_eq!(off("2100-07-15T12:00:00@Australia/Melbourne"), 10 * H);
    // still consistent with instant arithmetic
    let x = dt("2150-03-30T12:00:00@Europe/Paris");
    let y = datetime_add_dtd(&x, &dtd("P1D")).unwrap();
    assert_eq!(datetime_sub(&y, &x), Some(dtd("P1D")));
    // the extreme instants are representable
    assert!(dt("999999999-12-31T23:59:59.999999999@Europe/Paris")
        .epoch_nanos()
        .is_some());
    assert!(dt("-999999999-01-01T00:00:00@Europe/Paris")
        .epoch_nanos()
        .is_some());
    // ... but arithmetic leaving the range is null
    assert_eq!(
        datetime_add_dtd(&dt("999999999-12-31T23:59:59@Europe/Paris"), &dtd("PT1H")),
        None
    );
}

#[test]
fn fixed_offset_named_zones() {
    assert_eq!(off("2021-07-01T12:00:00@Etc/UTC"), 0);
    assert_eq!(off("2021-07-01T12:00:00@Etc/GMT+5"), -5 * H);
    assert_eq!(off("2021-07-01T12:00:00@UTC"), 0);
    assert!(Zone::named("Etc/GMT-14").is_some());
}

#[test]
fn zone_api_and_constructors() {
    assert_eq!(Zone::utc(), Zone::Offset(0));
    assert_eq!(Zone::utc().to_string(), "Z");
    assert_eq!(Zone::offset(-(9 * H + 1800)).unwrap().to_string(), "-09:30");
    assert_eq!(
        Zone::offset(2 * H + 45 * 60 + 55).unwrap().to_string(),
        "+02:45:55"
    );
    assert_eq!(
        Zone::named("Europe/Paris").unwrap().to_string(),
        "@Europe/Paris"
    );
    assert_eq!(
        Zone::named("Europe/Paris").unwrap().iana_name(),
        Some("Europe/Paris")
    );
    assert_eq!(Zone::utc().iana_name(), None);
    assert_eq!(Zone::utc().fixed_offset(), Some(0));
    assert_eq!(Zone::named("Etc/UTC").unwrap().fixed_offset(), None);
    assert!(Zone::named("Asia/Dhaka").unwrap().tz().is_some());
    let paris = Zone::named("Europe/Paris").unwrap();
    let local = dt("2021-07-01T12:00:00").local_epoch_nanos();
    assert_eq!(paris.offset_for_local_nanos(local), Some(2 * H));
    assert_eq!(paris.offset_for_epoch_nanos(local), Some(2 * H));
    assert_eq!(
        Zone::Named("Bad/Zone".into()).offset_for_local_nanos(local),
        None
    );

    let day = d("2021-07-01");
    assert_eq!(
        DateTime::new(day, 3_600 * NANOS_PER_SEC, Some(paris.clone()))
            .unwrap()
            .to_string(),
        "2021-07-01T01:00:00@Europe/Paris"
    );
    assert_eq!(DateTime::new(day, NANOS_PER_DAY, None), None);
    assert_eq!(DateTime::new(day, -1, None), None);
    assert_eq!(
        DateTime::new(day, 0, Some(Zone::Named("Bad/Zone".into()))),
        None
    );
    assert_eq!(
        Time::from_nanos(NANOS_PER_DAY - 1, None)
            .unwrap()
            .to_string(),
        "23:59:59.999999999"
    );
    assert_eq!(Time::from_nanos(NANOS_PER_DAY, None), None);
    assert_eq!(Time::from_nanos(0, Some(Zone::Offset(15 * H))), None);
    // a date-time without zone has no UTC form
    assert_eq!(dt("2021-07-01T12:00:00").offset_seconds(), None);
    assert_eq!(dt("2021-07-01T12:00:00").timezone(), None);
    assert_eq!(
        dt("2021-07-01T12:00:00@Europe/Paris").timezone(),
        Some("Europe/Paris")
    );
}
