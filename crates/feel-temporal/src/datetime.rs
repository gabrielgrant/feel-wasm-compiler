//! The FEEL `date and time` type.

use core::fmt;
use core::str::FromStr;

use crate::date::{fmt_year, ParseTemporalError};
use crate::duration::DayTimeDuration;
use crate::parse::{parse_date_fields, parse_time_fields, Cursor};
use crate::time::fmt_time_of_day;
use crate::zone::Zone;
use crate::{Date, Time, NANOS_PER_DAY, NANOS_PER_SEC};

/// A FEEL `date and time`: a date, a local time of day and an optional zone.
///
/// The local fields are stored as written; for a named zone the UTC offset is
/// resolved on demand ([`DateTime::offset_seconds`]). The derived `==` /
/// `Hash` are *identity* (FEEL `is()`): same local fields and same zone
/// representation. FEEL `=` / `<` are [`eq_datetime`](crate::eq_datetime) /
/// [`cmp_datetime`](crate::cmp_datetime).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DateTime {
    /// The local date.
    pub date: Date,
    /// Local time of day in nanoseconds, `[0, 86_400e9)`.
    pub nanos: i64,
    /// `None` for a local date-time without offset.
    pub zone: Option<Zone>,
}

impl DateTime {
    /// Builds a date-time, validating the time of day and the zone.
    pub fn new(date: Date, nanos: i64, zone: Option<Zone>) -> Option<DateTime> {
        if !(0..NANOS_PER_DAY).contains(&nanos) || zone.as_ref().is_some_and(|z| !z.is_valid()) {
            return None;
        }
        Some(DateTime { date, nanos, zone })
    }

    /// Parses a FEEL *date time string*: a date string (see [`Date::parse`]),
    /// optionally followed by `T` and a time string (see [`Time::parse`]).
    ///
    /// A bare date string gives local midnight without zone
    /// (`"2012-12-24"` → `2012-12-24T00:00:00`). `T24:00:00` denotes midnight
    /// at the end of the day (`2021-01-01T24:00:00` = `2021-01-02T00:00:00`).
    /// A dangling `T`, a zone on a bare date, or offset+IANA combinations are
    /// rejected.
    pub fn parse(s: &str) -> Option<DateTime> {
        let mut c = Cursor::new(s);
        let date = parse_date_fields(&mut c)?;
        if c.is_eof() {
            return Some(DateTime {
                date,
                nanos: 0,
                zone: None,
            });
        }
        c.expect(b'T')?;
        let tf = parse_time_fields(&mut c)?;
        let date = if tf.end_of_day {
            date.add_days(1)?
        } else {
            date
        };
        Some(DateTime {
            date,
            nanos: tf.nanos,
            zone: tf.zone,
        })
    }

    /// FEEL `date and time(date, time)`: the date combined with the time's
    /// local time of day and zone.
    pub fn from_date_time(d: &Date, t: &Time) -> DateTime {
        DateTime {
            date: *d,
            nanos: t.nanos,
            zone: t.zone.clone(),
        }
    }

    /// Local midnight *without* zone (`date and time("2012-12-24")` form).
    pub fn from_date(d: &Date) -> DateTime {
        DateTime {
            date: *d,
            nanos: 0,
            zone: None,
        }
    }

    /// UTC midnight (`Zone::Offset(0)`): the implicit date → date-and-time
    /// conversion of 10.3.2.3.5 / 10.3.2.9.4, used when a date is mixed with a
    /// date-time (Table 57 subtraction) or passed where a date-time is
    /// expected (TCK 1157: `date(2000,1,2)` → `2000-01-02T00:00:00Z`; TCK 0100:
    /// `@"2021-01-02T00:00:00Z" - @"2021-01-02" = PT0S` but
    /// `@"2021-01-01T00:00:00" - @"2021-01-02"` = null).
    pub fn from_date_utc(d: &Date) -> DateTime {
        DateTime {
            date: *d,
            nanos: 0,
            zone: Some(Zone::Offset(0)),
        }
    }

    /// The date part (FEEL `date(date and time)`).
    pub fn date(&self) -> Date {
        self.date
    }

    /// The time part with the same zone (FEEL `time(date and time)`).
    pub fn time(&self) -> Time {
        Time {
            nanos: self.nanos,
            zone: self.zone.clone(),
        }
    }

    /// Year (FEEL `year` property).
    pub fn year(&self) -> i64 {
        self.date.year()
    }

    /// Month 1..=12 (FEEL `month` property).
    pub fn month(&self) -> u32 {
        self.date.month()
    }

    /// Day 1..=31 (FEEL `day` property).
    pub fn day(&self) -> u32 {
        self.date.day()
    }

    /// ISO weekday 1 = Monday .. 7 = Sunday (FEEL `weekday` property).
    pub fn weekday(&self) -> u32 {
        self.date.weekday()
    }

    /// Hour 0..=23 (FEEL `hour` property).
    pub fn hour(&self) -> u32 {
        (self.nanos / (3600 * NANOS_PER_SEC)) as u32
    }

    /// Minute 0..=59 (FEEL `minute` property).
    pub fn minute(&self) -> u32 {
        ((self.nanos / (60 * NANOS_PER_SEC)) % 60) as u32
    }

    /// Second including fraction, in nanoseconds, `[0, 60e9)` (FEEL `second`
    /// property = this / 10⁹).
    pub fn second_nanos(&self) -> i64 {
        self.nanos % (60 * NANOS_PER_SEC)
    }

    /// The local date-time as nanoseconds since 1970-01-01T00:00:00 *local*
    /// (i.e. the local fields read as if they were UTC).
    pub fn local_epoch_nanos(&self) -> i128 {
        i128::from(self.date.days()) * i128::from(NANOS_PER_DAY) + i128::from(self.nanos)
    }

    /// UTC offset in seconds (FEEL `time offset` property): the fixed offset,
    /// or for a named zone the offset in force at this local date-time
    /// (chrono-tz / IANA tzdb [`tzdb_version`](crate::tzdb_version)).
    ///
    /// DST gap (non-existent local time): the offset before the transition is
    /// used; DST overlap (ambiguous local time): the earlier instant, i.e. the
    /// offset before the transition (both as `java.time.ZonedDateTime.of`).
    /// From 2100 on (beyond chrono-tz's explicit transitions) the zone's rules
    /// are projected by reusing the 2072..=2099 year with the same calendar;
    /// years before ~100,000 BCE are shifted by whole 400-year cycles (the
    /// offset is the constant local mean time there).
    pub fn offset_seconds(&self) -> Option<i32> {
        self.zone
            .as_ref()?
            .offset_for_local_nanos(self.local_epoch_nanos())
    }

    /// FEEL `time offset` property as a duration (see
    /// [`DateTime::offset_seconds`]).
    pub fn time_offset(&self) -> Option<DayTimeDuration> {
        self.offset_seconds()
            .map(|s| DayTimeDuration::from_seconds(i64::from(s)))
    }

    /// FEEL `timezone` property: the IANA id of a named zone, else `None`.
    pub fn timezone(&self) -> Option<&str> {
        self.zone.as_ref()?.iana_name()
    }

    /// `valuedt` in nanoseconds since 1970-01-01T00:00:00Z; `None` without a
    /// zone (no UTC form).
    pub fn epoch_nanos(&self) -> Option<i128> {
        let off = self.offset_seconds()?;
        Some(self.local_epoch_nanos() - i128::from(off) * i128::from(NANOS_PER_SEC))
    }

    /// From local nanoseconds since 1970-01-01T00:00:00 local, keeping `zone`
    /// as is (`None` when the date leaves the year range).
    pub fn from_local_epoch_nanos(local: i128, zone: Option<Zone>) -> Option<DateTime> {
        let npd = i128::from(NANOS_PER_DAY);
        let days = i64::try_from(local.div_euclid(npd)).ok()?;
        let nanos = local.rem_euclid(npd) as i64;
        Some(DateTime {
            date: Date::from_days(days)?,
            nanos,
            zone,
        })
    }

    /// `valuedt⁻¹`: the date-time in `zone` denoting the instant `epoch`
    /// (nanoseconds since 1970-01-01T00:00:00Z). For a named zone the local
    /// fields are those of the instant in that zone. `None` when the date
    /// leaves the year range or the zone is invalid.
    pub fn from_epoch_nanos(epoch: i128, zone: Zone) -> Option<DateTime> {
        // Reject absurd instants before converting to seconds.
        let limit = (i128::from(Date::MAX.days()) + 2) * i128::from(NANOS_PER_DAY);
        if epoch.abs() > limit {
            return None;
        }
        let off = zone.offset_for_epoch_nanos(epoch)?;
        let local = epoch + i128::from(off) * i128::from(NANOS_PER_SEC);
        DateTime::from_local_epoch_nanos(local, Some(zone))
    }
}

/// `2012-12-25T11:00:00`, `2012-12-25T11:00:00.5Z`,
/// `-2017-12-31T11:22:33+01:35`, `2011-12-31T10:15:30@Europe/Paris`.
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.date.ymd();
        fmt_year(f, y)?;
        write!(f, "-{m:02}-{d:02}T")?;
        fmt_time_of_day(f, self.nanos)?;
        if let Some(z) = &self.zone {
            z.fmt(f)?;
        }
        Ok(())
    }
}

impl FromStr for DateTime {
    type Err = ParseTemporalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DateTime::parse(s).ok_or(ParseTemporalError)
    }
}
