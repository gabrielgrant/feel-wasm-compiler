# feel-temporal

FEEL (DMN 1.5) temporal values for the FEEL → WebAssembly compiler: `date`,
`time`, `date and time`, `days and time duration` and `years and months
duration`, with their lexical forms, Table 57 arithmetic, Table 52–54
comparison, `is()` identity, Table 65/66 properties and the Table 79 temporal
built-ins.

The crate does not depend on a decimal library. Seconds are integer
**nanoseconds** (`i64` for times of day, `i128` for durations and instants).
The caller converts to and from FEEL numbers. Calendar arithmetic uses `i64`
day counts (Hinnant's civil algorithms, proleptic Gregorian, astronomical
year numbering) and is exact over the whole FEEL year range
`[-999,999,999 .. 999,999,999]`. A result that leaves the range is `None`
(FEEL null). `chrono` and `chrono-tz` are only used to look up IANA zone
offsets (tzdb version: `tzdb_version()`, which returns `"2025b"` for
chrono-tz 0.10.4). The crate builds for `wasm32-unknown-unknown`.

## API overview

| Item | Notes |
| --- | --- |
| `Date` | days since 1970-01-01; `from_ymd`, `from_days`, `parse`, `year/month/day/weekday/day_of_year/iso_week`, `add_days`, `add_months` (end-of-month clamping), `add_months_strict` |
| `Time { nanos, zone }` | `new(h, m, second_nanos, zone)`, `from_nanos`, `parse`, `hour/minute/second_nanos`, `offset_seconds`, `time_offset`, `timezone`, `value_nanos` (valuet) |
| `DateTime { date, nanos, zone }` | `new`, `parse`, `from_date_time`, `from_date` (unzoned midnight), `from_date_utc` (implicit conversion), `date()`, `time()`, field accessors, `offset_seconds`, `time_offset`, `timezone`, `epoch_nanos` (valuedt), `local_epoch_nanos`, `from_local_epoch_nanos`, `from_epoch_nanos` |
| `Zone` | `Offset(secs)` / `Named(iana id)`; `utc`, `offset`, `named`, `from_offset_duration`, `iana_name`, `standard_offset`, `offset_for_local_nanos`, `offset_for_epoch_nanos`. `Display` prints the suffix (`Z`, `+02:00`, `@Europe/Paris`) |
| `DayTimeDuration { nanos }`, `YearMonthDuration { months }`, `Duration` | `parse_duration`, canonical `Display`, `days/hours/minutes/seconds_nanos`, `years/months`, `checked_add/sub/neg/abs`, `mul_div(num, den)` (Table 59 helper, truncates toward zero) |
| Arithmetic | `datetime_sub`, `datetime_sub_date`, `date_sub_datetime`, `datetime_add_dtd/sub_dtd`, `datetime_add_ymd/sub_ymd`, `date_sub`, `date_add_dtd/sub_dtd`, `date_add_ymd/sub_ymd`, `time_sub`, `time_add_dtd/sub_dtd`, `years_and_months_between(_dates)` |
| Comparison | `cmp_date`, `cmp_datetime`/`eq_datetime`, `cmp_time`/`eq_time` (millisecond precision, TCK), `*_exact` variants (nanoseconds), `is_same_date/time/datetime` (`is()`) |
| Misc | `weekday_name`, `month_name`, `tzdb_version`, `NANOS_PER_SEC`, `NANOS_PER_DAY`, `MIN_YEAR`, `MAX_YEAR`, `MAX_OFFSET_SECONDS` |

The derived `==` and `Hash` on `Time` and `DateTime` are **identity**
(`is()`). FEEL `=` and `<` are the `eq_*` and `cmp_*` functions.

## Semantic choices

These choices were checked against DMN TCK revision `5cd912f2`. The tests in
`tests/` reproduce the relevant TCK cases.

* **Date strings**: `-?YYYY-MM-DD`. The year has at least 4 digits and no
  leading zero beyond 4 digits (`998-…` and `01211-…` are rejected), no `+`
  sign, and must be in range. Year `0000` is accepted (XSD 1.1 / ISO 8601:
  `-0001 + P1Y = 0000`) but `-0000` is rejected. A date string with a time
  part or a zone is not a date string.
* **Time strings**: `hh:mm:ss(.f+)?` followed by an optional `Z`/`z`,
  `±hh:mm` (`±hh:mm:ss` is also accepted), or `@IANA/Id`. Each field has
  exactly two digits. There are no leap seconds (`23:59:60` is null).
  `24:00:00` is the XSD end-of-day form (`T24:00:00` is the next day's
  midnight). Fixed offsets must be within ±14:00. An offset combined with an
  IANA id is rejected. IANA ids are validated case-sensitively by chrono-tz,
  and `backward` aliases are accepted.
* **Date-time strings**: a date string, optionally followed by `T` and a
  time string. A bare date string gives local midnight **without** zone
  (`date and time("2012-12-24")` = `2012-12-24T00:00:00`, TCK 1117).
* **Fractional seconds**: nanosecond resolution. Fraction digits past the
  9th are **truncated** rather than rejected, because XSD allows any
  precision. Display drops trailing zeros (`11:22:33.444`).
* **Duration strings**: only the `xs:dayTimeDuration` and
  `xs:yearMonthDuration` lexical spaces are accepted. Mixed forms such as
  `P1Y2D` or `P1MT1H`, and weeks (`P1W`), give **null**, per the "duration
  string" domain of Table 72. `PT0.S` and `PT.5S` are accepted (XSD 1.1).
  Values are normalized. Zero prints as `PT0S` or `P0M` (the XPath canonical
  form, and TCK 1120).
* **Duration properties** carry the sign of the duration (`-P1DT2H`: days
  -1, hours -2), as `java.time` does. Then
  `days·86400 + hours·3600 + minutes·60 + seconds` equals the value.
* **Date → date-time implicit conversion** is **UTC midnight** (`Z`, from
  `DateTime::from_date_utc`). As a result, `@"2021-01-02T00:00:00Z" -
  @"2021-01-02" = PT0S` but `@"2021-01-01T00:00:00" - @"2021-01-02"` is null
  (TCK 0100). `date(2000,1,2)` bound to a `date and time` gives
  `2000-01-02T00:00:00Z` (TCK 1157). `time(date)` is `00:00:00Z`.
  `date - date` is a whole number of days.
* **Named zones on date-times** are resolved at the local date-time:
  * In a **DST gap**, the offset before the transition is used
    (`2021-03-28T02:30@Europe/Paris` is the instant `01:30Z`).
  * In a **DST overlap**, the **earlier instant** is used (the
    pre-transition offset). This matches `java.time.ZonedDateTime.of`.
  * Adding a days-and-time duration to a named-zone date-time is **instant
    arithmetic** (`2021-03-27T12:00@Europe/Paris + PT24H =
    2021-03-28T13:00@Europe/Paris`). Adding a years-and-months duration
    shifts the local date.
  * chrono-tz 0.10.4 only has explicit transitions up to 2099. For year
    2100 and later, the offset is computed in the 2072–2099 year that has
    the same calendar (leap year and weekday of 1 January), so today's
    rules continue indefinitely, as in `java.time`.
  * Years before about 100,000 BCE are shifted by whole 400-year cycles.
    The offset there is the zone's constant LMT.
* **Named zones on bare times** (no date, so DST is unknown) use the zone's
  **standard** offset (tzdb `base_utc_offset`) in force on 2025-01-01. This
  applies to comparison, subtraction and the `time offset` property
  (`time("10:30:00@Europe/Paris")` → `+01:00`). Two times in the same zone
  compare by local time.
* **Comparison** (`cmp_*`, `eq_*`):
  * Zoned values compare as instants (`valuedt`), or as `valuet` = local time
    minus offset, not reduced modulo 24h.
  * If one value has no zone, it uses the other's offset, so the local fields
    are compared.
  * `datetime_sub` with exactly one zoned operand is null (Table 57).
  * `time_sub` with one unzoned operand subtracts local times, like the
    comparison rule.
* **Month arithmetic** clamps to the last day of the month
  (`2017-01-31 + P1M = 2017-02-28`, `2016-02-29 + P1Y = 2017-02-28`).
* **`date ± days and time duration`** takes the day containing
  `UTC midnight ± d` (floor), so `2021-01-02 - PT1H = 2021-01-01` (TCK 0100).
* **`time ± days and time duration`** works modulo 24h and keeps the zone.
* **`years and months duration(from, to)`** counts the whole months between
  the **local dates**. Time of day and zones are ignored, and an incomplete
  month is dropped toward zero (`java.time.Period.between`). This is what
  TCK 1121 expects: `2014-12-31T23:59:59 → 2016-12-31T00:00:01 = P2Y` and
  `2011-08-25T15:59:59@Europe/Paris → 2015-08-25T15:20:59+02:00 = P4Y`.
* **`is()`** compares the local fields at nanosecond precision and the zone
  representation:
  * `Z`, `+00:00` and `-00:00` are identical.
  * `@Etc/UTC` is not identical to `Z`.
  * `12:00-01:00` is not identical to `17:00+04:00` (TCK 0103).

## Spec / TCK discrepancies and gaps

1. **Equality and ordering use millisecond precision.** Table 53/54 compare
   `valuet` and `valuedt`, which have fractions. TCK 0068 expects
   `time("10:30:00.0001") = time("10:30:00.0002")` and
   `date and time("…T00:00:00.0001") = date and time("…T00:00:00.0000")` to be
   **true** — the reference implementations compare at millisecond
   precision. `eq_*` and `cmp_*` floor to whole milliseconds (so `=` and `<`
   stay consistent and `time("10:30:00.1") < time("10:30:00.2")` holds); the
   `*_exact` functions compare nanoseconds; `is()` and subtraction are exact.
2. **End-of-month month arithmetic.** The literal Table 57 formula,
   `date(y, m, e1.day)`, gives null for `2017-01-31 + P1M`. The TCK has no
   case for this. This crate clamps, following XML Schema Part 2 Appendix E,
   XPath `op:add-yearMonthDuration-to-date` and `java.time`.
   `Date::add_months_strict` gives the literal behaviour.
3. **`24:00:00`** is not in ISO 8601 extended local time, but XSD allows it
   and TCK 0100 uses `@"2021-01-01T24:00:00"`. It is accepted.
   `24:00:01` and `24:01:00` are null (TCK 1116/1117).
4. **Lower-case `z`** is not valid XSD, but the spec's own examples use it
   (`time("23:59:00z")`, `is(time("23:00:50z"), …)`). It is accepted as UTC.
5. **Offsets with seconds** (`+02:45:55`) are not XSD. TCK 1116 requires
   `time(11, 59, 45, duration("PT2H45M55S"))` to print as `11:59:45+02:45:55`.
   For round-tripping, parsing accepts `±hh:mm:ss`, as in the non-compliant
   TCK 0079. Offsets are limited to ±14:00 (XSD and Table 66), not ±18:00 as
   in `java.time`; TCK only requires `±19:00` to be rejected.
6. **Zero durations**: TCK 1120 and 0079 expect `PT0S` and `P0M`. Some 0100
   expectations are written `P0D` or `P0Y`, but the TCK compares
   `xsd:duration` values semantically. The canonical form is `PT0S` / `P0M`.
7. **Named zone on a bare time**: the spec does not define its offset. The
   TCK cases comparing such times with other zones are commented out,
   pending RTF clarification (`0068 time_007…time_009_b`). This crate uses
   the standard offset (see above) instead of a clock-dependent "current"
   offset, so results are deterministic. Same-zone comparisons and
   subtraction (the only ones the TCK tests) do not depend on this choice.
8. **`years and months duration`**: the spec only gives one example. The
   `java.time.Period` semantics on local dates reproduce all of TCK 1121
   (see above).
9. **Year `0000`** is accepted (XSD 1.1). XSD 1.0 had no year 0.
10. **Date strings with a zone** (`"2012-12-25Z"`, cf. the garbled Table 64
    example `"2012-0307Z"`) are rejected, because FEEL dates have no zone.
11. **Duration property ranges**: Table 66 says hours are in `[0..23]` and so
    on, which assumes a non-negative duration. Negative durations give
    negative components (see above).
12. The Table 72 example `time("23:59:00z") + duration("PT2M") =
    time("00:01:00@Etc/UTC")` holds for `=`, because `Etc/UTC` resolves to
    +00:00. It does not hold for `is()`.
