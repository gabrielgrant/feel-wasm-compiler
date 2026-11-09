//! Proleptic Gregorian calendar arithmetic on `i64` day counts.
//!
//! The conversions follow Howard Hinnant's `days_from_civil` /
//! `civil_from_days` algorithms ("chrono-Compatible Low-Level Date
//! Algorithms"), using floor division so that they are exact for every year
//! representable in an `i64` day count, in particular for the whole FEEL year
//! range `[-999_999_999, 999_999_999]`. Year numbering is astronomical
//! (ISO 8601 / XSD 1.1): year `0` exists and is the year before `1`.

/// Number of days in a 400-year Gregorian cycle.
pub(crate) const DAYS_PER_400_YEARS: i64 = 146_097;

/// `true` if `year` is a Gregorian leap year (astronomical numbering).
pub(crate) fn is_leap_year(year: i64) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}

/// Number of days in `month` (1..=12) of `year`.
pub(crate) fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days since 1970-01-01 of the civil date `year-month-day`.
///
/// `month` must be in `1..=12` and `day` in `1..=31`; no further validation
/// is done. Exact (no overflow) for `|year| < 2^54`.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let mp = i64::from((month + 9) % 12); // March = 0
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * DAYS_PER_400_YEARS + doe - 719_468
}

/// Civil date `(year, month, day)` of the day `days` since 1970-01-01.
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(DAYS_PER_400_YEARS);
    let doe = z - era * DAYS_PER_400_YEARS; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// ISO 8601 day of week of the day `days` since 1970-01-01: 1 = Monday .. 7 = Sunday.
pub(crate) fn weekday_from_days(days: i64) -> u32 {
    // 1970-01-01 was a Thursday (4).
    ((days + 3).rem_euclid(7) + 1) as u32
}

/// Number of ISO 8601 weeks (52 or 53) in the ISO week-numbering year `year`.
pub(crate) fn iso_weeks_in_year(year: i64) -> u32 {
    // A year has 53 ISO weeks iff it starts on a Thursday, or it is a leap year
    // starting on a Wednesday.
    let jan1 = weekday_from_days(days_from_civil(year, 1, 1));
    if jan1 == 4 || (jan1 == 3 && is_leap_year(year)) {
        53
    } else {
        52
    }
}

/// ISO 8601 week number (1..=53) of the given civil date.
pub(crate) fn iso_week(year: i64, month: u32, day: u32) -> u32 {
    let days = days_from_civil(year, month, day);
    let doy = days - days_from_civil(year, 1, 1) + 1; // 1-based
    let wd = i64::from(weekday_from_days(days));
    let w = (doy - wd + 10).div_euclid(7);
    if w < 1 {
        iso_weeks_in_year(year - 1)
    } else if w > i64::from(iso_weeks_in_year(year)) {
        1
    } else {
        w as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(weekday_from_days(0), 4);
    }

    #[test]
    fn known_dates() {
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(0, 1, 1), -719_528);
        assert_eq!(civil_from_days(-719_528), (0, 1, 1));
        assert_eq!(civil_from_days(-719_529), (-1, 12, 31));
    }

    #[test]
    fn leap_years() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2004));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2100));
        assert!(is_leap_year(0));
        assert!(is_leap_year(-4));
        assert!(!is_leap_year(-1));
        assert!(!is_leap_year(-100));
        assert!(is_leap_year(-400));
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(-400, 2), 29);
    }

    #[test]
    fn round_trip_dense_window() {
        // Every day across several 400-year cycles around year 0 and 1970.
        for days in -900_000..900_000i64 {
            let (y, m, d) = civil_from_days(days);
            assert!((1..=12).contains(&m));
            assert!(d >= 1 && d <= days_in_month(y, m));
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
    }

    #[test]
    fn round_trip_extremes() {
        for &(y, m, d) in &[
            (-999_999_999i64, 1u32, 1u32),
            (-999_999_999, 12, 31),
            (999_999_999, 12, 31),
            (999_999_999, 1, 1),
            (-4, 2, 29),
            (123_456_789, 2, 28),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d));
        }
        // consecutive days stay consecutive at the extremes
        let a = days_from_civil(999_999_999, 12, 31);
        assert_eq!(civil_from_days(a - 1), (999_999_999, 12, 30));
        let b = days_from_civil(-999_999_999, 1, 1);
        assert_eq!(civil_from_days(b + 1), (-999_999_999, 1, 2));
        assert_eq!(civil_from_days(b - 1), (-1_000_000_000, 12, 31));
    }

    #[test]
    fn iso_weeks() {
        assert_eq!(iso_week(2003, 12, 29), 1);
        assert_eq!(iso_week(2003, 12, 31), 1);
        assert_eq!(iso_week(2004, 1, 1), 1);
        assert_eq!(iso_week(2004, 1, 4), 1);
        assert_eq!(iso_week(2005, 1, 1), 53);
        assert_eq!(iso_week(2005, 1, 2), 53);
        assert_eq!(iso_week(2005, 1, 3), 1);
        assert_eq!(iso_week(2005, 1, 9), 1);
        assert_eq!(iso_week(2010, 1, 1), 53);
        assert_eq!(iso_week(2019, 9, 17), 38);
        assert_eq!(iso_week(1970, 1, 1), 1);
        assert_eq!(iso_week(2020, 12, 31), 53);
        assert_eq!(iso_week(2021, 1, 3), 53);
        assert_eq!(iso_week(2021, 1, 4), 1);
        assert_eq!(iso_week(2008, 12, 29), 1);
        assert_eq!(iso_weeks_in_year(2004), 53);
        assert_eq!(iso_weeks_in_year(2005), 52);
        assert_eq!(iso_weeks_in_year(2015), 53);
        assert_eq!(iso_weeks_in_year(2020), 53);
    }
}
