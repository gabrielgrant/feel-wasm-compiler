//! Time zones: fixed UTC offsets and IANA zone ids resolved with `chrono-tz`.

use core::fmt;

use chrono::{LocalResult, NaiveDateTime, Offset, TimeZone};
use chrono_tz::{GapInfo, OffsetComponents, Tz};

use crate::civil::{
    civil_from_days, days_from_civil, is_leap_year, weekday_from_days, DAYS_PER_400_YEARS,
};
use crate::duration::DayTimeDuration;
use crate::NANOS_PER_SEC;

/// Largest accepted absolute value of a fixed offset, in seconds (14 hours, as
/// in XML Schema `xs:dateTime` / `xs:time` and DMN Table 66 "time offset").
pub const MAX_OFFSET_SECONDS: i32 = 14 * 3600;

/// The time zone of a [`Time`](crate::Time) or [`DateTime`](crate::DateTime).
///
/// * `Offset(s)` – a fixed offset of `s` seconds east of UTC (`|s| <=`
///   [`MAX_OFFSET_SECONDS`]). `Z`, `+00:00` and `-00:00` all map to
///   `Offset(0)`, which is displayed as `Z`.
/// * `Named(id)` – an IANA time zone id such as `"Europe/Paris"` or
///   `"Etc/UTC"`, as accepted (case-sensitively) by `chrono-tz`. The id is kept
///   exactly as written; `Named("Etc/UTC")` and `Offset(0)` are different
///   values (they are `=` but not `is()`-identical).
///
/// Values built through [`Zone::offset`], [`Zone::named`] and the parsers are
/// always valid. The variants are public, so a hand-built `Zone` may be
/// invalid; [`Zone::is_valid`] checks it and constructors such as
/// [`Time::new`](crate::Time::new) reject invalid zones.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    /// Fixed offset in seconds east of UTC.
    Offset(i32),
    /// IANA time zone id, e.g. `"Europe/Paris"`.
    Named(String),
}

/// Reference instant (2025-01-01T00:00:00Z, the year of the bundled tzdb
/// 2025b) used to pick the *standard* offset of an IANA zone attached to a
/// bare time (a `time` value has no date, so the DST state is unknown).
pub(crate) const BARE_TIME_REFERENCE_EPOCH_SECONDS: i64 = 1_735_689_600;

/// `chrono-tz` 0.10.4 compiles explicit transitions up to the end of 2099 and
/// then keeps the last offset forever (no DST after 2099). To project the
/// zone's current rules indefinitely (as `java.time` does), an instant in year
/// `Y >= 2100` is mapped onto the year of the same calendar type (leap-ness
/// and weekday of 1 January) in the 28-year window 2072..=2099, which contains
/// every calendar type and no skipped leap year.
const PROJECTION_FIRST_YEAR: i64 = 2072;
const PROJECTION_END_YEAR: i64 = 2100; // exclusive; first year without tz data
const PROJECTION_START_SECONDS: i64 = 4_102_444_800; // 2100-01-01T00:00:00

/// Instants before ~100,000 BCE are shifted by whole 400-year Gregorian cycles
/// (which preserve the calendar) into chrono's range (its `NaiveDate` covers
/// about ±262,000 years). The tz database has a constant offset (LMT) before
/// its first transition, so the shift does not change the answer.
const SAFE_MIN_SECONDS: i64 = -100_000 * 365 * 86_400;
const CYCLE_SECONDS: i64 = DAYS_PER_400_YEARS * 86_400;

/// Maps seconds since 1970-01-01T00:00:00 (local or UTC) onto an equivalent
/// instant covered by chrono-tz's compiled data.
fn map_into_tz_data_range(secs: i64) -> i64 {
    if secs < SAFE_MIN_SECONDS {
        let k = (SAFE_MIN_SECONDS - secs + CYCLE_SECONDS - 1) / CYCLE_SECONDS;
        secs + k * CYCLE_SECONDS
    } else if secs >= PROJECTION_START_SECONDS {
        let days = secs.div_euclid(86_400);
        let (year, _, _) = civil_from_days(days);
        let jan1 = days_from_civil(year, 1, 1);
        let (leap, wd) = (is_leap_year(year), weekday_from_days(jan1));
        (PROJECTION_FIRST_YEAR..PROJECTION_END_YEAR)
            .map(|y| (y, days_from_civil(y, 1, 1)))
            .find(|&(y, d)| is_leap_year(y) == leap && weekday_from_days(d) == wd)
            .map_or(secs, |(_, d)| secs + (d - jan1) * 86_400)
    } else {
        secs
    }
}

fn naive_from_secs(secs: i64) -> Option<NaiveDateTime> {
    chrono::DateTime::from_timestamp(map_into_tz_data_range(secs), 0).map(|d| d.naive_utc())
}

fn floor_secs(nanos: i128) -> i64 {
    // In-range date-times need |secs| < 3.2e16; clamping keeps arbitrary
    // caller input from overflowing the range mapping below.
    const LIMIT: i128 = 100_000_000_000_000_000;
    nanos
        .div_euclid(i128::from(NANOS_PER_SEC))
        .clamp(-LIMIT, LIMIT) as i64
}

/// Offset (seconds east of UTC) of `tz` for the given *local* wall-clock time,
/// expressed as seconds since 1970-01-01T00:00:00 local.
///
/// Policy for non-existent / ambiguous local times (same as `java.time`
/// `ZonedDateTime.of`):
/// * gap (e.g. 02:30 on a spring-forward day): the offset in force *before*
///   the transition is used (so 02:30@Europe/Paris on 2021-03-28 denotes the
///   instant 01:30Z, i.e. 03:30+02:00);
/// * overlap (e.g. 02:30 on a fall-back day): the *earlier* instant is used,
///   i.e. the offset in force before the transition (summer time).
pub(crate) fn tz_offset_for_local_secs(tz: &Tz, local_secs: i64) -> Option<i32> {
    let naive = naive_from_secs(local_secs)?;
    match tz.offset_from_local_datetime(&naive) {
        LocalResult::Single(o) => Some(o.fix().local_minus_utc()),
        LocalResult::Ambiguous(a, b) => {
            let (a, b) = (a.fix().local_minus_utc(), b.fix().local_minus_utc());
            // The larger offset yields the earlier instant.
            Some(a.max(b))
        }
        LocalResult::None => match GapInfo::new(&naive, tz).and_then(|g| g.begin) {
            Some((_, before)) => Some(before.fix().local_minus_utc()),
            // Should not happen; fall back to treating the local time as UTC
            // to find the surrounding offset.
            None => Some(tz.offset_from_utc_datetime(&naive).fix().local_minus_utc()),
        },
    }
}

/// Offset (seconds east of UTC) of `tz` at the UTC instant `utc_secs`.
pub(crate) fn tz_offset_for_utc_secs(tz: &Tz, utc_secs: i64) -> Option<i32> {
    let naive = naive_from_secs(utc_secs)?;
    Some(tz.offset_from_utc_datetime(&naive).fix().local_minus_utc())
}

impl Zone {
    /// UTC as a fixed offset (`Offset(0)`, displayed as `Z`).
    pub fn utc() -> Zone {
        Zone::Offset(0)
    }

    /// A fixed offset, validated against `|seconds| <= MAX_OFFSET_SECONDS`.
    pub fn offset(seconds: i32) -> Option<Zone> {
        if seconds.unsigned_abs() <= MAX_OFFSET_SECONDS as u32 {
            Some(Zone::Offset(seconds))
        } else {
            None
        }
    }

    /// A named IANA zone, validated with `chrono-tz` (case-sensitive, aliases
    /// from the `backward` file such as `"UTC"` or `"US/Eastern"` included).
    pub fn named(id: &str) -> Option<Zone> {
        id.parse::<Tz>().ok().map(|_| Zone::Named(id.to_owned()))
    }

    /// The fixed offset given by a days-and-time duration, as in FEEL
    /// `time(hour, minute, second, offset)`: the duration must be a whole
    /// number of seconds within ±14 hours (e.g. `PT2H45M55S` → `+02:45:55`,
    /// `-PT0H` → `Z`).
    pub fn from_offset_duration(d: &DayTimeDuration) -> Option<Zone> {
        if d.nanos % i128::from(NANOS_PER_SEC) != 0 {
            return None;
        }
        let secs = i32::try_from(d.nanos / i128::from(NANOS_PER_SEC)).ok()?;
        Zone::offset(secs)
    }

    /// `true` if the offset is within range / the IANA id is known.
    pub fn is_valid(&self) -> bool {
        match self {
            Zone::Offset(s) => s.unsigned_abs() <= MAX_OFFSET_SECONDS as u32,
            Zone::Named(n) => n.parse::<Tz>().is_ok(),
        }
    }

    /// The IANA id (FEEL `timezone` property) – `None` for fixed offsets.
    pub fn iana_name(&self) -> Option<&str> {
        match self {
            Zone::Named(n) => Some(n),
            Zone::Offset(_) => None,
        }
    }

    /// The `chrono-tz` zone for a `Named` zone.
    pub fn tz(&self) -> Option<Tz> {
        match self {
            Zone::Named(n) => n.parse::<Tz>().ok(),
            Zone::Offset(_) => None,
        }
    }

    /// The fixed offset of an `Offset` zone (`None` for named zones).
    pub fn fixed_offset(&self) -> Option<i32> {
        match self {
            Zone::Offset(s) => Some(*s),
            Zone::Named(_) => None,
        }
    }

    /// Offset in force for the given local wall-clock time (nanoseconds since
    /// 1970-01-01T00:00:00 *local*). See the crate docs for the DST gap /
    /// overlap policy. `None` only for an invalid named zone.
    pub fn offset_for_local_nanos(&self, local_nanos: i128) -> Option<i32> {
        match self {
            Zone::Offset(s) => Some(*s),
            Zone::Named(_) => tz_offset_for_local_secs(&self.tz()?, floor_secs(local_nanos)),
        }
    }

    /// Offset in force at the given instant (nanoseconds since
    /// 1970-01-01T00:00:00Z). `None` only for an invalid named zone.
    pub fn offset_for_epoch_nanos(&self, epoch_nanos: i128) -> Option<i32> {
        match self {
            Zone::Offset(s) => Some(*s),
            Zone::Named(_) => tz_offset_for_utc_secs(&self.tz()?, floor_secs(epoch_nanos)),
        }
    }

    /// Offset used for a zone attached to a bare `time` (no date): the fixed
    /// offset for `Offset`, and for a named zone its *standard* offset (DST
    /// excluded, as defined by the tz database) in force on 2025-01-01T00:00Z.
    /// E.g. `Europe/Paris` → +01:00, `America/New_York` → -05:00,
    /// `Australia/Melbourne` → +10:00, `Etc/UTC` → 0.
    pub fn standard_offset(&self) -> Option<i32> {
        match self {
            Zone::Offset(s) => Some(*s),
            Zone::Named(_) => {
                let tz = self.tz()?;
                let naive = naive_from_secs(BARE_TIME_REFERENCE_EPOCH_SECONDS)?;
                let off = tz.offset_from_utc_datetime(&naive);
                i32::try_from(off.base_utc_offset().num_seconds()).ok()
            }
        }
    }
}

/// Formats a fixed offset as used in time strings: `Z`, `+02:00`, `-05:30`,
/// `+02:45:55` (seconds only when non-zero).
pub(crate) fn fmt_offset(f: &mut fmt::Formatter<'_>, secs: i32) -> fmt::Result {
    if secs == 0 {
        return f.write_str("Z");
    }
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.unsigned_abs();
    let (h, m, s) = (a / 3600, (a / 60) % 60, a % 60);
    if s == 0 {
        write!(f, "{sign}{h:02}:{m:02}")
    } else {
        write!(f, "{sign}{h:02}:{m:02}:{s:02}")
    }
}

/// Displays the zone suffix exactly as it appears in a FEEL time / date-time
/// string: `Z`, `+02:00`, `-05:00`, `+02:45:55` or `@Europe/Paris`.
impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Zone::Offset(s) => fmt_offset(f, *s),
            Zone::Named(n) => write!(f, "@{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_constant() {
        assert_eq!(
            crate::civil::days_from_civil(2025, 1, 1) * 86_400,
            BARE_TIME_REFERENCE_EPOCH_SECONDS
        );
    }

    #[test]
    fn mapping_preserves_calendar() {
        assert_eq!(
            days_from_civil(2100, 1, 1) * 86_400,
            PROJECTION_START_SECONDS
        );
        // far past: whole 400-year cycles
        for secs in [-31_556_952_000_000_000i64, SAFE_MIN_SECONDS - 1] {
            let s = map_into_tz_data_range(secs);
            assert!(s >= SAFE_MIN_SECONDS);
            assert_eq!((secs - s) % CYCLE_SECONDS, 0);
        }
        // far future: same month/day/time-of-day and weekday, year in 2072..2100
        for secs in [
            PROJECTION_START_SECONDS,
            31_556_952_000_000_000,
            7_258_118_400 + 12_345,
        ] {
            let s = map_into_tz_data_range(secs);
            let (y0, m0, d0) = civil_from_days(secs.div_euclid(86_400));
            let (y1, m1, d1) = civil_from_days(s.div_euclid(86_400));
            assert!(
                (PROJECTION_FIRST_YEAR..PROJECTION_END_YEAR).contains(&y1),
                "{y0} -> {y1}"
            );
            assert_eq!((m0, d0), (m1, d1));
            assert_eq!(secs.rem_euclid(86_400), s.rem_euclid(86_400));
            assert_eq!(
                weekday_from_days(secs.div_euclid(86_400)),
                weekday_from_days(s.div_euclid(86_400))
            );
        }
        // every calendar type has a representative
        for y in 2100..2500 {
            let s = map_into_tz_data_range(days_from_civil(y, 6, 1) * 86_400);
            assert!(s < PROJECTION_START_SECONDS, "{y}");
        }
        assert_eq!(map_into_tz_data_range(0), 0);
    }

    #[test]
    fn extreme_inputs_do_not_panic() {
        let paris = Zone::named("Europe/Paris").unwrap();
        for n in [
            i128::MIN,
            i128::MAX,
            i128::from(i64::MIN),
            i128::from(i64::MAX),
        ] {
            assert!(paris.offset_for_local_nanos(n).is_some());
            assert!(paris.offset_for_epoch_nanos(n).is_some());
        }
    }

    #[test]
    fn validation() {
        assert!(Zone::offset(14 * 3600).is_some());
        assert!(Zone::offset(-14 * 3600).is_some());
        assert!(Zone::offset(14 * 3600 + 1).is_none());
        assert!(Zone::named("Europe/Paris").is_some());
        assert!(Zone::named("Etc/UTC").is_some());
        assert!(Zone::named("xyz/abc").is_none());
        assert!(Zone::named("europe/paris").is_none());
        assert!(Zone::named("").is_none());
        assert!(!Zone::Named("Nope/Nope".into()).is_valid());
        assert!(!Zone::Offset(15 * 3600).is_valid());
    }

    #[test]
    fn standard_offsets() {
        let so = |n: &str| Zone::named(n).unwrap().standard_offset().unwrap();
        assert_eq!(so("Europe/Paris"), 3600);
        assert_eq!(so("America/New_York"), -5 * 3600);
        assert_eq!(so("Australia/Melbourne"), 10 * 3600);
        assert_eq!(so("Etc/UTC"), 0);
        assert_eq!(so("Asia/Dhaka"), 6 * 3600);
        assert_eq!(so("Asia/Kolkata"), 5 * 3600 + 1800);
    }
}
