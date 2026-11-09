//! Equality, ordering (DMN 1.5 Tables 52–54) and identity (`is()`, Table 77).

use core::cmp::Ordering;

use crate::{Date, DateTime, Time, NANOS_PER_SEC};

/// Whole milliseconds (floor) of a nanosecond value: FEEL `=`/`<` on times
/// and date-times compare at millisecond precision (TCK 0068 requires
/// `time("10:30:00.0001") = time("10:30:00.0002")`, while
/// `time("10:30:00.1") < time("10:30:00.2")` must still hold).
fn whole_seconds(n: i128) -> i128 {
    n.div_euclid(i128::from(NANOS_PER_SEC / 1000))
}

/// The two values `valuedt` compares: instants when both have a zone, local
/// date-times otherwise ("if one input has a null timezone offset, that input
/// uses the timezone offset of the other input", so the offsets cancel).
fn datetime_values(a: &DateTime, b: &DateTime) -> Option<(i128, i128)> {
    if a.zone.is_some() && b.zone.is_some() {
        Some((a.epoch_nanos()?, b.epoch_nanos()?))
    } else {
        Some((a.local_epoch_nanos(), b.local_epoch_nanos()))
    }
}

/// The two values `valuet` compares: UTC-normalized times of day (not reduced
/// modulo 24h) when both have a zone, local times otherwise.
fn time_values(a: &Time, b: &Time) -> (i128, i128) {
    if a.zone.is_some() && b.zone.is_some() {
        (a.value_nanos(), b.value_nanos())
    } else {
        (i128::from(a.nanos), i128::from(b.nanos))
    }
}

/// `date` ordering (Table 54): by year, month, day.
pub fn cmp_date(a: &Date, b: &Date) -> Ordering {
    a.cmp(b)
}

/// `date and time` ordering (Table 54) at **millisecond** precision.
///
/// The TCK (0068 `datetime_003_a`: `"…T00:00:00.0001" = "…T00:00:00.0000"`
/// is `true`) compares `valuedt` truncated to whole milliseconds (as the
/// reference implementations' `toEpochSecond()` does); `=` and `<` use the
/// same truncation so that they stay consistent. Use
/// [`cmp_datetime_exact`] for nanosecond precision.
///
/// Zoned values compare as instants; if one value has no zone it takes the
/// offset of the other, i.e. both compare by local fields. Returns `None`
/// only if a named zone cannot be resolved (invalid zone id).
pub fn cmp_datetime(a: &DateTime, b: &DateTime) -> Option<Ordering> {
    let (x, y) = datetime_values(a, b)?;
    Some(whole_seconds(x).cmp(&whole_seconds(y)))
}

/// FEEL `=` on date-times (Table 53), millisecond precision; see
/// [`cmp_datetime`].
pub fn eq_datetime(a: &DateTime, b: &DateTime) -> Option<bool> {
    cmp_datetime(a, b).map(Ordering::is_eq)
}

/// [`cmp_datetime`] at nanosecond precision.
pub fn cmp_datetime_exact(a: &DateTime, b: &DateTime) -> Option<Ordering> {
    let (x, y) = datetime_values(a, b)?;
    Some(x.cmp(&y))
}

/// [`eq_datetime`] at nanosecond precision.
pub fn eq_datetime_exact(a: &DateTime, b: &DateTime) -> Option<bool> {
    cmp_datetime_exact(a, b).map(Ordering::is_eq)
}

/// `time` ordering (Table 54) at **millisecond** precision (TCK 0068
/// `time_005`: `time("10:30:00.0001") = time("10:30:00.0002")` is `true`).
///
/// When both times have a zone, `valuet` (local time minus offset, a named
/// zone using its standard offset – see [`Time::offset_seconds`]) is
/// compared; if one has no zone it takes the other's offset, i.e. local times
/// are compared. Always `Some`.
pub fn cmp_time(a: &Time, b: &Time) -> Option<Ordering> {
    let (x, y) = time_values(a, b);
    Some(whole_seconds(x).cmp(&whole_seconds(y)))
}

/// FEEL `=` on times (Table 53), millisecond precision; see [`cmp_time`].
pub fn eq_time(a: &Time, b: &Time) -> Option<bool> {
    cmp_time(a, b).map(Ordering::is_eq)
}

/// [`cmp_time`] at nanosecond precision.
pub fn cmp_time_exact(a: &Time, b: &Time) -> Option<Ordering> {
    let (x, y) = time_values(a, b);
    Some(x.cmp(&y))
}

/// [`eq_time`] at nanosecond precision.
pub fn eq_time_exact(a: &Time, b: &Time) -> Option<bool> {
    cmp_time_exact(a, b).map(Ordering::is_eq)
}

/// FEEL `is(a, b)` on dates.
pub fn is_same_date(a: &Date, b: &Date) -> bool {
    a == b
}

/// FEEL `is(a, b)` on times (Table 77): same local time (nanosecond exact)
/// and same zone representation. `Z`, `+00:00` and `-00:00` are the same;
/// `@Etc/UTC` differs from `Z`, `@Australia/Melbourne` from `+10:00`, and
/// `20:00:50+00:00` from `21:00:50+01:00`.
pub fn is_same_time(a: &Time, b: &Time) -> bool {
    a == b
}

/// FEEL `is(a, b)` on date-times (Table 77): same local fields and same zone
/// representation (`2002-04-02T12:00:00-01:00` is not
/// `2002-04-02T17:00:00+04:00` although they are `=`).
pub fn is_same_datetime(a: &DateTime, b: &DateTime) -> bool {
    a == b
}
