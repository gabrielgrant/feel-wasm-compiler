//! # feel-temporal
//!
//! FEEL (DMN 1.5) temporal values – `date`, `time`, `date and time`,
//! `days and time duration`, `years and months duration` – with their
//! lexical forms, arithmetic (Table 57), comparison (Tables 52–54), identity
//! (`is()`, Table 77), properties (Tables 65/66) and the temporal built-ins
//! of Table 79.
//!
//! The crate is numeric-type agnostic: seconds are integer **nanoseconds**
//! (`i64` for times of day, `i128` for durations and instants) and the caller
//! converts to / from FEEL decimals. Calendar arithmetic is done on `i64` day
//! counts (proleptic Gregorian, astronomical year numbering) and is exact over
//! the whole FEEL year range `[MIN_YEAR, MAX_YEAR]`; results leaving the range
//! are `None` (FEEL null). `chrono` / `chrono-tz` are used only to look up
//! IANA zone offsets.
//!
//! ## Semantic choices (see also `README.md`)
//!
//! * **Lexical forms.** Dates: `-?YYYY-MM-DD` (≥4 year digits, no leading
//!   zero beyond 4 digits, no `+`, `0000` allowed, `-0000` rejected). Times:
//!   `hh:mm:ss(.f+)?` + optional `Z`/`z`, `±hh:mm` (`±hh:mm:ss` accepted),
//!   or `@IANA/Id`; `24:00:00` is the XSD end-of-day form (TCK 0100 uses
//!   `@"2021-01-01T24:00:00"`); no leap seconds. Fixed offsets are limited to
//!   ±14:00 (TCK 1116/1117 reject `+19:00`). Date-times: date string,
//!   optionally `T` + time string; a bare date string is local midnight
//!   without zone (TCK 1117 `007`).
//! * **Fractional seconds**: nanosecond resolution; extra fraction digits are
//!   *truncated*, not rejected (XSD allows arbitrary precision).
//! * **Durations**: `parse_duration` accepts the `xs:dayTimeDuration` and
//!   `xs:yearMonthDuration` lexical spaces only; mixed forms (`P1Y2D`) and
//!   weeks are `None`. Values are normalized and printed canonically
//!   (`PT0S` / `P0M` for zero). Component properties carry the sign.
//! * **Date → date-time implicit conversion** is UTC midnight
//!   ([`DateTime::from_date_utc`], zone `Z`), so `date - unzoned date-time`
//!   is null while `date - zoned date-time` is defined (TCK 0100).
//!   `date and time("2012-12-24")` is unzoned midnight ([`DateTime::from_date`]).
//! * **Named zones on date-times** are resolved with chrono-tz (tzdb
//!   [`tzdb_version`]) at the local date-time. DST gap → offset before the
//!   transition; overlap → earlier instant (both as `java.time`). Adding a
//!   days-and-time duration to a named-zone date-time is instant arithmetic.
//!   chrono-tz only has explicit transitions until 2099; later years reuse the
//!   rules of the 2072–2099 year with the same calendar, and years before
//!   ~100,000 BCE are shifted by 400-year cycles (constant LMT offset).
//! * **Named zones on bare times** (no date) use the zone's *standard*
//!   (non-DST) offset as of 2025-01-01 for comparison, subtraction and the
//!   `time offset` property; two times in the same zone compare by local time.
//! * **Comparison / equality** of times and date-times is at **whole-second**
//!   precision, as required by the TCK (0068 `time_005`, `datetime_003_a`);
//!   `_exact` variants compare nanoseconds. A zoned value compared with an
//!   unzoned one uses the zoned one's offset (i.e. local fields compare).
//! * **Month arithmetic** clamps the day to the end of the target month
//!   (`2017-01-31 + P1M = 2017-02-28`, XSD Appendix E / XPath / `java.time`)
//!   instead of the literal Table 57 null; [`Date::add_months_strict`] gives
//!   the literal behaviour.
//! * **`years and months duration(from, to)`** counts whole months between the
//!   local dates (time and zone ignored), truncated toward zero
//!   (`java.time.Period.between`, matching TCK 1121).
//! * **Identity** (`is()`): the derived `PartialEq` of [`Time`] and
//!   [`DateTime`] – same local fields and same zone representation.
//!
//! ## Example
//!
//! ```
//! use feel_temporal::*;
//!
//! // date and time("2012-12-24T23:59:00") + duration("PT1M")
//! let a = DateTime::parse("2012-12-24T23:59:00").unwrap();
//! let one_minute = DayTimeDuration::parse("PT1M").unwrap();
//! assert_eq!(datetime_add_dtd(&a, &one_minute).unwrap().to_string(), "2012-12-25T00:00:00");
//!
//! // date and time("2012-12-24T23:59:00") - date and time("2012-12-22T03:45:00")
//! let b = DateTime::parse("2012-12-22T03:45:00").unwrap();
//! assert_eq!(datetime_sub(&a, &b).unwrap().to_string(), "P2DT20H14M");
//!
//! // a zoned and an unzoned value cannot be subtracted
//! let z = DateTime::parse("2012-12-22T03:45:00@Europe/Paris").unwrap();
//! assert_eq!(datetime_sub(&a, &z), None);
//!
//! // years and months duration(date("2011-12-22"), date("2013-08-24"))
//! let from = DateTime::from_date(&Date::parse("2011-12-22").unwrap());
//! let to = DateTime::from_date(&Date::parse("2013-08-24").unwrap());
//! assert_eq!(years_and_months_between(&from, &to).unwrap().to_string(), "P1Y8M");
//!
//! // time("23:59:00z") + duration("PT2M") = time("00:01:00@Etc/UTC")
//! let t = time_add_dtd(&Time::parse("23:59:00z").unwrap(), &DayTimeDuration::parse("PT2M").unwrap())
//!     .unwrap();
//! assert_eq!(eq_time(&t, &Time::parse("00:01:00@Etc/UTC").unwrap()), Some(true));
//! assert!(!is_same_time(&t, &Time::parse("00:01:00@Etc/UTC").unwrap()));
//!
//! // week of year(date(2005, 1, 1)), day of week(date(2019, 9, 17))
//! assert_eq!(Date::from_ymd(2005, 1, 1).unwrap().iso_week(), 53);
//! assert_eq!(weekday_name(Date::from_ymd(2019, 9, 17).unwrap().weekday()), "Tuesday");
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod arith;
mod civil;
mod compare;
mod date;
mod datetime;
mod duration;
mod parse;
mod time;
mod zone;

pub use arith::{
    date_add_dtd, date_add_ymd, date_sub, date_sub_datetime, date_sub_dtd, date_sub_ymd,
    datetime_add_dtd, datetime_add_ymd, datetime_sub, datetime_sub_date, datetime_sub_dtd,
    datetime_sub_ymd, time_add_dtd, time_sub, time_sub_dtd, years_and_months_between,
    years_and_months_between_dates,
};
pub use compare::{
    cmp_date, cmp_datetime, cmp_datetime_exact, cmp_time, cmp_time_exact, eq_datetime,
    eq_datetime_exact, eq_time, eq_time_exact, is_same_date, is_same_datetime, is_same_time,
};
pub use date::{Date, ParseTemporalError};
pub use datetime::DateTime;
pub use duration::{parse_duration, DayTimeDuration, Duration, YearMonthDuration};
pub use time::Time;
pub use zone::{Zone, MAX_OFFSET_SECONDS};

/// Nanoseconds per second.
pub const NANOS_PER_SEC: i64 = 1_000_000_000;
/// Nanoseconds per (86 400-second) day.
pub const NANOS_PER_DAY: i64 = 86_400 * NANOS_PER_SEC;
/// Smallest FEEL year (10.3.2.3.5).
pub const MIN_YEAR: i64 = -999_999_999;
/// Largest FEEL year (10.3.2.3.5).
pub const MAX_YEAR: i64 = 999_999_999;

const WEEKDAY_NAMES: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTH_NAMES: [&str; 12] = [
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
    "December",
];

/// English day name for an ISO weekday 1 = Monday .. 7 = Sunday (FEEL
/// `day of week()`); `""` outside `1..=7`.
pub fn weekday_name(n: u32) -> &'static str {
    n.checked_sub(1)
        .and_then(|i| WEEKDAY_NAMES.get(i as usize))
        .copied()
        .unwrap_or("")
}

/// English month name for 1 = January .. 12 = December (FEEL
/// `month of year()`); `""` outside `1..=12`.
pub fn month_name(n: u32) -> &'static str {
    n.checked_sub(1)
        .and_then(|i| MONTH_NAMES.get(i as usize))
        .copied()
        .unwrap_or("")
}

/// Version of the IANA time zone database compiled into `chrono-tz`
/// (`chrono_tz::IANA_TZDB_VERSION`, generated from the bundled `tz/` data;
/// `"2025b"` for chrono-tz 0.10.4).
pub fn tzdb_version() -> &'static str {
    chrono_tz::IANA_TZDB_VERSION
}
