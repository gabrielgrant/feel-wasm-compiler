//! Temporal arithmetic (DMN 1.5 Table 57) and `years and months duration()`.

use crate::duration::{DayTimeDuration, YearMonthDuration};
use crate::zone::Zone;
use crate::{Date, DateTime, Time, NANOS_PER_DAY};

/// `date and time - date and time` (Table 57): `valuedt(a) - valuedt(b)`.
///
/// Both values must have a zone (compared as instants) or both must have no
/// zone (compared as local date-times); a mixed pair is undefined → `None`.
/// To subtract a `date`, convert it with [`DateTime::from_date_utc`] (or use
/// [`datetime_sub_date`] / [`date_sub_datetime`]).
pub fn datetime_sub(a: &DateTime, b: &DateTime) -> Option<DayTimeDuration> {
    let n = match (&a.zone, &b.zone) {
        (None, None) => a.local_epoch_nanos() - b.local_epoch_nanos(),
        (Some(_), Some(_)) => a.epoch_nanos()? - b.epoch_nanos()?,
        _ => return None,
    };
    Some(DayTimeDuration::from_nanos(n))
}

/// `date and time - date`: the date is implicitly converted to UTC midnight
/// (a *zoned* value), so `a` must have a zone (TCK 0100:
/// `@"2021-01-02T10:10:10@Europe/Paris" - @"2021-01-01" = P1DT9H10M10S`,
/// `@"2021-01-01T00:00:00" - @"2021-01-02"` = null).
pub fn datetime_sub_date(a: &DateTime, b: &Date) -> Option<DayTimeDuration> {
    datetime_sub(a, &DateTime::from_date_utc(b))
}

/// `date - date and time`: the date is implicitly converted to UTC midnight,
/// so `b` must have a zone (TCK 0100: `@"2021-01-02" -
/// @"2021-01-01T10:10:10+11:00" = P1DT49M50S`).
pub fn date_sub_datetime(a: &Date, b: &DateTime) -> Option<DayTimeDuration> {
    datetime_sub(&DateTime::from_date_utc(a), b)
}

/// `date and time + days and time duration` (Table 57):
/// `valuedt⁻¹(valuedt(a) + valuedtd(d))`, keeping the zone.
///
/// * no zone / fixed offset: plain local arithmetic;
/// * named zone: instant arithmetic – the instant is shifted and converted
///   back to local fields in the same zone, so DST transitions are honoured
///   (`2021-03-27T12:00@Europe/Paris + PT24H = 2021-03-28T13:00@Europe/Paris`).
///
/// `None` when the result leaves the year range.
pub fn datetime_add_dtd(a: &DateTime, d: &DayTimeDuration) -> Option<DateTime> {
    match &a.zone {
        Some(z @ Zone::Named(_)) => {
            let e = a.epoch_nanos()?.checked_add(d.nanos)?;
            DateTime::from_epoch_nanos(e, z.clone())
        }
        _ => DateTime::from_local_epoch_nanos(
            a.local_epoch_nanos().checked_add(d.nanos)?,
            a.zone.clone(),
        ),
    }
}

/// `date and time - days and time duration` (see [`datetime_add_dtd`]).
pub fn datetime_sub_dtd(a: &DateTime, d: &DayTimeDuration) -> Option<DateTime> {
    datetime_add_dtd(a, &d.checked_neg()?)
}

/// `date and time + years and months duration` (Table 57): the date part is
/// shifted by whole months (day clamped to the end of the month, see
/// [`Date::add_months`]); time of day and zone are kept.
pub fn datetime_add_ymd(a: &DateTime, d: &YearMonthDuration) -> Option<DateTime> {
    Some(DateTime {
        date: a.date.add_months(d.months)?,
        nanos: a.nanos,
        zone: a.zone.clone(),
    })
}

/// `date and time - years and months duration` (see [`datetime_add_ymd`]).
pub fn datetime_sub_ymd(a: &DateTime, d: &YearMonthDuration) -> Option<DateTime> {
    datetime_add_ymd(a, &d.checked_neg()?)
}

/// `date - date` (Table 57 with both dates converted to UTC midnight): the
/// whole number of days between them.
pub fn date_sub(a: &Date, b: &Date) -> DayTimeDuration {
    DayTimeDuration::from_nanos(
        (i128::from(a.days()) - i128::from(b.days())) * i128::from(NANOS_PER_DAY),
    )
}

/// `date + days and time duration` (Table 57):
/// `date(valuedt⁻¹(valuedt(a) + valuedtd(d)))` with `a` at UTC midnight, i.e.
/// the day containing `midnight + d` (floor): `2021-01-02 - PT1H = 2021-01-01`,
/// `2021-01-02 + PT36H = 2021-01-03`, `2021-01-02 - -PT1H = 2021-01-02`.
pub fn date_add_dtd(a: &Date, d: &DayTimeDuration) -> Option<Date> {
    let npd = i128::from(NANOS_PER_DAY);
    let total = (i128::from(a.days()) * npd).checked_add(d.nanos)?;
    Date::from_days(i64::try_from(total.div_euclid(npd)).ok()?)
}

/// `date - days and time duration` (see [`date_add_dtd`]).
pub fn date_sub_dtd(a: &Date, d: &DayTimeDuration) -> Option<Date> {
    date_add_dtd(a, &d.checked_neg()?)
}

/// `date + years and months duration` (Table 57, end-of-month clamped – see
/// [`Date::add_months`]).
pub fn date_add_ymd(a: &Date, d: &YearMonthDuration) -> Option<Date> {
    a.add_months(d.months)
}

/// `date - years and months duration` (see [`date_add_ymd`]).
pub fn date_sub_ymd(a: &Date, d: &YearMonthDuration) -> Option<Date> {
    a.add_months(d.months.checked_neg()?)
}

/// `time - time` (Table 57): `valuet(a) - valuet(b)`.
///
/// When both times have a zone their UTC-normalized values are subtracted
/// (named zones use their standard offset, see [`Time::offset_seconds`]; two
/// times in the *same* zone therefore subtract their local times). When at
/// least one has no zone, it takes the offset of the other (as for
/// comparisons), i.e. the local times are subtracted. The result is not
/// reduced modulo 24h. Always `Some`.
pub fn time_sub(a: &Time, b: &Time) -> Option<DayTimeDuration> {
    let n = if a.zone.is_some() && b.zone.is_some() {
        a.value_nanos() - b.value_nanos()
    } else {
        i128::from(a.nanos) - i128::from(b.nanos)
    };
    Some(DayTimeDuration::from_nanos(n))
}

/// `time + days and time duration` (Table 57): `valuet⁻¹(valuet(a) + d)`,
/// i.e. the local time shifted modulo 24 hours, zone kept
/// (`10:15:00 + P1D = 10:15:00`, `23:59:00Z + PT2M = 00:01:00Z`). Always `Some`.
pub fn time_add_dtd(a: &Time, d: &DayTimeDuration) -> Option<Time> {
    let npd = i128::from(NANOS_PER_DAY);
    let nanos = (i128::from(a.nanos) + d.nanos.rem_euclid(npd)).rem_euclid(npd) as i64;
    Some(Time {
        nanos,
        zone: a.zone.clone(),
    })
}

/// `time - days and time duration` (see [`time_add_dtd`]).
pub fn time_sub_dtd(a: &Time, d: &DayTimeDuration) -> Option<Time> {
    let npd = i128::from(NANOS_PER_DAY);
    let nanos = (i128::from(a.nanos) - d.nanos.rem_euclid(npd)).rem_euclid(npd) as i64;
    Some(Time {
        nanos,
        zone: a.zone.clone(),
    })
}

/// FEEL `years and months duration(from, to)` on two date-times (the caller
/// converts `date` arguments, e.g. with [`DateTime::from_date`]).
///
/// The number of whole months from the *local date* of `from` to the local
/// date of `to`; time of day and zones are ignored, and an incomplete month
/// is dropped toward zero (`java.time.Period.between`, which the TCK 1121
/// expectations follow: `2014-12-31T23:59:59 → 2016-12-31T00:00:01 = P2Y`,
/// `2016-09-30 → 2017-12-28 = P1Y2M`, `2013-08-24 → 2011-12-22 = -P1Y8M`).
/// Always `Some`.
pub fn years_and_months_between(from: &DateTime, to: &DateTime) -> Option<YearMonthDuration> {
    Some(years_and_months_between_dates(&from.date, &to.date))
}

/// [`years_and_months_between`] on dates.
pub fn years_and_months_between_dates(from: &Date, to: &Date) -> YearMonthDuration {
    let (fy, fm, fd) = from.ymd();
    let (ty, tm, td) = to.ymd();
    let mut total = (ty * 12 + i64::from(tm)) - (fy * 12 + i64::from(fm));
    let days = i64::from(td) - i64::from(fd);
    if total > 0 && days < 0 {
        total -= 1;
    } else if total < 0 && days > 0 {
        total += 1;
    }
    YearMonthDuration::from_months(total)
}
