//! The FEEL `date` type.

use core::fmt;
use core::str::FromStr;

use crate::civil::{civil_from_days, days_from_civil, days_in_month, iso_week, weekday_from_days};
use crate::parse::{parse_date_fields, Cursor};
use crate::{MAX_YEAR, MIN_YEAR};

/// Day counts (since 1970-01-01) of `-999999999-01-01` and `999999999-12-31`.
const MIN_DAYS: i64 = -365_243_219_162; // days_from_civil(MIN_YEAR, 1, 1)
const MAX_DAYS: i64 = 365_241_780_471; // days_from_civil(MAX_YEAR, 12, 31)

/// A FEEL `date`: a proleptic Gregorian calendar date without time zone,
/// stored as the number of days since 1970-01-01.
///
/// The year is in `[MIN_YEAR, MAX_YEAR]` = `[-999_999_999, 999_999_999]`
/// with astronomical numbering (year `0` exists). Ordering / equality are
/// chronological (Table 54 "date": year, then month, then day).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    days: i64,
}

impl Date {
    /// The earliest representable date, `-999999999-01-01`.
    pub const MIN: Date = Date { days: MIN_DAYS };
    /// The latest representable date, `999999999-12-31`.
    pub const MAX: Date = Date { days: MAX_DAYS };

    /// FEEL `date(year, month, day)`: `None` unless the year is within
    /// `[MIN_YEAR, MAX_YEAR]`, the month in `1..=12` and the day valid for
    /// that month (leap years included).
    pub fn from_ymd(year: i64, month: i64, day: i64) -> Option<Date> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) || !(1..=12).contains(&month) {
            return None;
        }
        let month = month as u32;
        if day < 1 || day > i64::from(days_in_month(year, month)) {
            return None;
        }
        Some(Date {
            days: days_from_civil(year, month, day as u32),
        })
    }

    /// The date `days` days after 1970-01-01 (`None` outside the year range).
    pub fn from_days(days: i64) -> Option<Date> {
        (MIN_DAYS..=MAX_DAYS)
            .contains(&days)
            .then_some(Date { days })
    }

    /// Days since 1970-01-01 (negative before).
    pub fn days(&self) -> i64 {
        self.days
    }

    /// `(year, month, day)`.
    pub fn ymd(&self) -> (i64, u32, u32) {
        civil_from_days(self.days)
    }

    /// The year (FEEL `year` property).
    pub fn year(&self) -> i64 {
        self.ymd().0
    }

    /// The month, 1..=12 (FEEL `month` property).
    pub fn month(&self) -> u32 {
        self.ymd().1
    }

    /// The day of the month, 1..=31 (FEEL `day` property).
    pub fn day(&self) -> u32 {
        self.ymd().2
    }

    /// ISO day of week, 1 = Monday .. 7 = Sunday (FEEL `weekday` property).
    pub fn weekday(&self) -> u32 {
        weekday_from_days(self.days)
    }

    /// Day of the year, 1..=366 (FEEL `day of year()`).
    pub fn day_of_year(&self) -> u32 {
        let y = self.year();
        (self.days - days_from_civil(y, 1, 1) + 1) as u32
    }

    /// ISO 8601 week number, 1..=53 (FEEL `week of year()`).
    pub fn iso_week(&self) -> u32 {
        let (y, m, d) = self.ymd();
        iso_week(y, m, d)
    }

    /// Parses a FEEL *date string*: the `xs:date` lexical form without a
    /// timezone, `-?YYYY-MM-DD`.
    ///
    /// The year has at least 4 digits, no leading zero when longer than 4
    /// digits, no `+` sign, and must be in range; `-0000` is rejected but
    /// `0000` (= 1 BCE) is accepted. Month and day must be exactly two digits
    /// and valid. Anything else (a time part, a timezone, whitespace) is
    /// rejected.
    pub fn parse(s: &str) -> Option<Date> {
        let mut c = Cursor::new(s);
        let d = parse_date_fields(&mut c)?;
        c.is_eof().then_some(d)
    }

    /// Adds a number of days (`None` when leaving the year range).
    pub fn add_days(&self, days: i64) -> Option<Date> {
        Date::from_days(self.days.checked_add(days)?)
    }

    /// Adds a (possibly negative) number of months, per Table 57
    /// "date ± years and months duration".
    ///
    /// When the day does not exist in the target month it is clamped to the
    /// last day of that month (`2017-01-31 + P1M = 2017-02-28`,
    /// `2016-02-29 + P1Y = 2017-02-28`), following XML Schema Part 2
    /// Appendix E, XPath `op:add-yearMonthDuration-to-date` and `java.time`.
    /// The literal Table 57 formula (`date(y, m, e1.day)`) would give null
    /// instead; see [`Date::add_months_strict`].
    pub fn add_months(&self, months: i64) -> Option<Date> {
        let (y, m, d) = self.ymd();
        let (ny, nm) = shift_month(y, m, months)?;
        let nd = d.min(days_in_month(ny, nm));
        Date::from_ymd(ny, i64::from(nm), i64::from(nd))
    }

    /// Like [`Date::add_months`] but returns `None` when the day of month does
    /// not exist in the target month (the literal Table 57 formula).
    pub fn add_months_strict(&self, months: i64) -> Option<Date> {
        let (y, m, d) = self.ymd();
        let (ny, nm) = shift_month(y, m, months)?;
        Date::from_ymd(ny, i64::from(nm), i64::from(d))
    }
}

/// `(year, month)` shifted by `months`, `None` on overflow.
fn shift_month(year: i64, month: u32, months: i64) -> Option<(i64, u32)> {
    let total = year
        .checked_mul(12)?
        .checked_add(i64::from(month) - 1)?
        .checked_add(months)?;
    Some((total.div_euclid(12), (total.rem_euclid(12) + 1) as u32))
}

/// Writes a year as in `xs:date`: at least four digits, `-` for negative
/// years (`2012`, `0000`, `-0001`, `10000`, `-999999999`).
pub(crate) fn fmt_year(f: &mut fmt::Formatter<'_>, year: i64) -> fmt::Result {
    if year < 0 {
        write!(f, "-{:04}", year.unsigned_abs())
    } else {
        write!(f, "{year:04}")
    }
}

/// `2012-12-25`, `-0001-01-01`, `10000-01-01`.
impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        fmt_year(f, y)?;
        write!(f, "-{m:02}-{d:02}")
    }
}

/// Error returned by the `FromStr` implementations of this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseTemporalError;

impl fmt::Display for ParseTemporalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid FEEL temporal lexical form")
    }
}

impl std::error::Error for ParseTemporalError {}

impl FromStr for Date {
    type Err = ParseTemporalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Date::parse(s).ok_or(ParseTemporalError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_bounds_constants() {
        assert_eq!(MIN_DAYS, days_from_civil(MIN_YEAR, 1, 1));
        assert_eq!(MAX_DAYS, days_from_civil(MAX_YEAR, 12, 31));
    }
}
