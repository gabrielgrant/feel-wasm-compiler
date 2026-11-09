//! The FEEL `time` type.

use core::fmt;
use core::str::FromStr;

use crate::date::ParseTemporalError;
use crate::duration::{write_fraction, DayTimeDuration};
use crate::parse::{parse_time_fields, Cursor};
use crate::zone::Zone;
use crate::{NANOS_PER_DAY, NANOS_PER_SEC};

/// A FEEL `time`: a time of day with an optional zone.
///
/// `nanos` is the local time of day in nanoseconds, in `[0, 86_400e9)`
/// (leap seconds are not representable). The derived `==` / `Hash` are
/// *identity* (FEEL `is()`): same local time and same zone representation.
/// FEEL `=` / `<` are [`eq_time`](crate::eq_time) / [`cmp_time`](crate::cmp_time).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Time {
    /// Nanoseconds since local midnight, `[0, 86_400e9)`.
    pub nanos: i64,
    /// `None` for a local time without offset.
    pub zone: Option<Zone>,
}

impl Time {
    /// FEEL `time(hour, minute, second, offset?)`: `hour` in `0..=23`,
    /// `minute` in `0..=59`, `second_nanos` (seconds × 10⁹ including the
    /// fraction) in `[0, 60e9)`; the zone must be valid (use
    /// [`Zone::from_offset_duration`] for the `offset` argument).
    pub fn new(hour: i64, minute: i64, second_nanos: i64, zone: Option<Zone>) -> Option<Time> {
        if !(0..=23).contains(&hour)
            || !(0..=59).contains(&minute)
            || !(0..60 * NANOS_PER_SEC).contains(&second_nanos)
        {
            return None;
        }
        if zone.as_ref().is_some_and(|z| !z.is_valid()) {
            return None;
        }
        Some(Time {
            nanos: (hour * 3600 + minute * 60) * NANOS_PER_SEC + second_nanos,
            zone,
        })
    }

    /// From nanoseconds since midnight (`[0, 86_400e9)`) and a valid zone.
    pub fn from_nanos(nanos: i64, zone: Option<Zone>) -> Option<Time> {
        if !(0..NANOS_PER_DAY).contains(&nanos) || zone.as_ref().is_some_and(|z| !z.is_valid()) {
            return None;
        }
        Some(Time { nanos, zone })
    }

    /// Parses a FEEL *time string*: `hh:mm:ss(.f+)?` optionally followed by
    /// `Z` (or `z`, as in the spec's examples), `±hh:mm` (`±hh:mm:ss` also
    /// accepted), or `@` + IANA zone id.
    ///
    /// * exactly two digits per field; `hh` ≤ 23, `mm` ≤ 59, `ss` ≤ 59 (no leap
    ///   second `23:59:60`); `24:00:00` (zero fraction) is accepted as the XSD
    ///   end-of-day form and equals `00:00:00`;
    /// * at least one fraction digit after `.`; digits beyond nanoseconds are
    ///   truncated;
    /// * `|offset|` ≤ 14:00 (`+19:00` → `None`); `-00:00` / `+00:00` = `Z`;
    /// * an offset combined with an IANA id (`+02:00@Europe/Paris`), a `T`
    ///   prefix/suffix, unknown zone ids → `None`.
    pub fn parse(s: &str) -> Option<Time> {
        let mut c = Cursor::new(s);
        let tf = parse_time_fields(&mut c)?;
        Some(Time {
            nanos: tf.nanos,
            zone: tf.zone,
        })
    }

    /// Hour, 0..=23 (FEEL `hour` property).
    pub fn hour(&self) -> u32 {
        (self.nanos / (3600 * NANOS_PER_SEC)) as u32
    }

    /// Minute, 0..=59 (FEEL `minute` property).
    pub fn minute(&self) -> u32 {
        ((self.nanos / (60 * NANOS_PER_SEC)) % 60) as u32
    }

    /// Second including the fraction, in nanoseconds, `[0, 60e9)` (FEEL
    /// `second` property = this / 10⁹).
    pub fn second_nanos(&self) -> i64 {
        self.nanos % (60 * NANOS_PER_SEC)
    }

    /// The offset used for comparisons, subtraction and the FEEL `time offset`
    /// property: the fixed offset for `Zone::Offset`; `None` without a zone;
    /// for a `Zone::Named` zone (a bare time has no date, so DST cannot be
    /// determined) the zone's *standard* offset as of 2025-01-01 – see
    /// [`Zone::standard_offset`].
    pub fn offset_seconds(&self) -> Option<i32> {
        self.zone.as_ref()?.standard_offset()
    }

    /// FEEL `time offset` property as a duration (see
    /// [`Time::offset_seconds`]).
    pub fn time_offset(&self) -> Option<DayTimeDuration> {
        self.offset_seconds()
            .map(|s| DayTimeDuration::from_seconds(i64::from(s)))
    }

    /// FEEL `timezone` property: the IANA id of a named zone, else `None`.
    pub fn timezone(&self) -> Option<&str> {
        self.zone.as_ref()?.iana_name()
    }

    /// `valuet` in nanoseconds: the local time of day minus the offset (not
    /// reduced modulo 24h, as in XSD: `01:00+02:00` < `22:00Z`). Times without
    /// a zone return the local time of day.
    pub fn value_nanos(&self) -> i128 {
        i128::from(self.nanos)
            - i128::from(self.offset_seconds().unwrap_or(0)) * i128::from(NANOS_PER_SEC)
    }
}

pub(crate) fn fmt_time_of_day(f: &mut fmt::Formatter<'_>, nanos: i64) -> fmt::Result {
    let secs = nanos / NANOS_PER_SEC;
    let frac = (nanos % NANOS_PER_SEC) as u64;
    write!(
        f,
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )?;
    write_fraction(f, frac)
}

/// `11:00:00`, `11:00:00.123`, `11:00:00Z`, `11:00:00+02:00`,
/// `11:59:45+02:45:55`, `11:00:00@Europe/Paris`.
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_time_of_day(f, self.nanos)?;
        if let Some(z) = &self.zone {
            z.fmt(f)?;
        }
        Ok(())
    }
}

impl FromStr for Time {
    type Err = ParseTemporalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Time::parse(s).ok_or(ParseTemporalError)
    }
}
