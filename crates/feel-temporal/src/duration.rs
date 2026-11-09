//! FEEL `days and time duration` and `years and months duration`.

use core::fmt;
use core::str::FromStr;

use crate::date::ParseTemporalError;
use crate::parse::{digits_value, fraction_nanos, Cursor};
use crate::{NANOS_PER_DAY, NANOS_PER_SEC};

const NANOS_PER_HOUR: i128 = 3_600 * NANOS_PER_SEC as i128;
const NANOS_PER_MINUTE: i128 = 60 * NANOS_PER_SEC as i128;

/// A FEEL *days and time duration* (`xs:dayTimeDuration`), stored as a signed
/// number of nanoseconds (`valuedtd` × 10⁹).
///
/// Ordering and equality are by value (`PT24H = P1D`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DayTimeDuration {
    /// Signed length in nanoseconds.
    pub nanos: i128,
}

/// A FEEL *years and months duration* (`xs:yearMonthDuration`), stored as a
/// signed number of months (`valueymd`).
///
/// Ordering and equality are by value (`P1Y = P12M`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct YearMonthDuration {
    /// Signed length in months.
    pub months: i64,
}

/// Either kind of FEEL duration, as produced by FEEL `duration(string)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Duration {
    /// `xs:dayTimeDuration`.
    DayTime(DayTimeDuration),
    /// `xs:yearMonthDuration`.
    YearMonth(YearMonthDuration),
}

impl DayTimeDuration {
    /// The zero duration (`PT0S`).
    pub const ZERO: DayTimeDuration = DayTimeDuration { nanos: 0 };

    /// From a signed nanosecond count.
    pub const fn from_nanos(nanos: i128) -> Self {
        DayTimeDuration { nanos }
    }

    /// From whole seconds.
    pub const fn from_seconds(secs: i64) -> Self {
        DayTimeDuration {
            nanos: secs as i128 * NANOS_PER_SEC as i128,
        }
    }

    /// Signed nanosecond count (`valuedtd(d)` × 10⁹).
    pub const fn as_nanos(&self) -> i128 {
        self.nanos
    }

    /// `true` for `PT0S`.
    pub const fn is_zero(&self) -> bool {
        self.nanos == 0
    }

    /// `true` for strictly negative durations.
    pub const fn is_negative(&self) -> bool {
        self.nanos < 0
    }

    /// Normalized days component (FEEL `days` property). Like every component
    /// accessor it carries the sign of the duration (`-P1DT2H` → `-1`), so
    /// that `days*86400 + hours*3600 + minutes*60 + seconds = valuedtd`.
    pub const fn days(&self) -> i128 {
        self.nanos / NANOS_PER_DAY as i128
    }

    /// Normalized hours component in `-23..=23` (FEEL `hours` property).
    pub const fn hours(&self) -> i64 {
        ((self.nanos / NANOS_PER_HOUR) % 24) as i64
    }

    /// Normalized minutes component in `-59..=59` (FEEL `minutes` property).
    pub const fn minutes(&self) -> i64 {
        ((self.nanos / NANOS_PER_MINUTE) % 60) as i64
    }

    /// Normalized seconds component including the fraction, in nanoseconds,
    /// in `(-60e9, 60e9)` (FEEL `seconds` property = this / 10⁹).
    pub const fn seconds_nanos(&self) -> i64 {
        (self.nanos % NANOS_PER_MINUTE) as i64
    }

    /// `self + other`, `None` on overflow.
    pub fn checked_add(&self, other: &DayTimeDuration) -> Option<Self> {
        self.nanos.checked_add(other.nanos).map(Self::from_nanos)
    }

    /// `self - other`, `None` on overflow.
    pub fn checked_sub(&self, other: &DayTimeDuration) -> Option<Self> {
        self.nanos.checked_sub(other.nanos).map(Self::from_nanos)
    }

    /// `-self`, `None` on overflow.
    pub fn checked_neg(&self) -> Option<Self> {
        self.nanos.checked_neg().map(Self::from_nanos)
    }

    /// `|self|`, `None` on overflow.
    pub fn checked_abs(&self) -> Option<Self> {
        self.nanos.checked_abs().map(Self::from_nanos)
    }

    /// Helper for Table 59 multiplication / division by a number given as the
    /// exact rational `num / den` (e.g. a decimal `coefficient / 10^scale`):
    /// `valuedtd⁻¹(valuedtd(self) * num / den)`, truncated toward zero to
    /// nanosecond resolution. `None` if `den == 0` or on overflow.
    pub fn mul_div(&self, num: i128, den: i128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        self.nanos
            .checked_mul(num)?
            .checked_div(den)
            .map(Self::from_nanos)
    }

    /// Parses an `xs:dayTimeDuration` lexical form only (see [`parse_duration`]).
    pub fn parse(s: &str) -> Option<Self> {
        match parse_duration(s)? {
            Duration::DayTime(d) => Some(d),
            Duration::YearMonth(_) => None,
        }
    }
}

impl YearMonthDuration {
    /// The zero duration (`P0M`).
    pub const ZERO: YearMonthDuration = YearMonthDuration { months: 0 };

    /// From a signed month count.
    pub const fn from_months(months: i64) -> Self {
        YearMonthDuration { months }
    }

    /// Signed month count (`valueymd`).
    pub const fn as_months(&self) -> i64 {
        self.months
    }

    /// `true` for `P0M`.
    pub const fn is_zero(&self) -> bool {
        self.months == 0
    }

    /// Normalized years component (FEEL `years` property), carrying the sign
    /// of the duration (`-P1Y2M` → `-1`).
    pub const fn years(&self) -> i64 {
        self.months / 12
    }

    /// Normalized months component in `-11..=11` (FEEL `months` property),
    /// carrying the sign of the duration (`-P1Y2M` → `-2`).
    pub const fn months(&self) -> i64 {
        self.months % 12
    }

    /// `self + other`, `None` on overflow.
    pub fn checked_add(&self, other: &YearMonthDuration) -> Option<Self> {
        self.months.checked_add(other.months).map(Self::from_months)
    }

    /// `self - other`, `None` on overflow.
    pub fn checked_sub(&self, other: &YearMonthDuration) -> Option<Self> {
        self.months.checked_sub(other.months).map(Self::from_months)
    }

    /// `-self`, `None` on overflow.
    pub fn checked_neg(&self) -> Option<Self> {
        self.months.checked_neg().map(Self::from_months)
    }

    /// `|self|`, `None` on overflow.
    pub fn checked_abs(&self) -> Option<Self> {
        self.months.checked_abs().map(Self::from_months)
    }

    /// Helper for Table 59: `valueymd⁻¹(valueymd(self) * num / den)` with the
    /// month count truncated toward zero (TCK 0100: `P2M * 1.5 = P3M`,
    /// `P1Y11M * -2.5 = -P4Y9M`, `P10Y11M / 2.5 = P4Y4M`). `None` if
    /// `den == 0` or on overflow.
    pub fn mul_div(&self, num: i128, den: i128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        let m = i128::from(self.months).checked_mul(num)?.checked_div(den)?;
        i64::try_from(m).ok().map(Self::from_months)
    }

    /// Parses an `xs:yearMonthDuration` lexical form only (see [`parse_duration`]).
    pub fn parse(s: &str) -> Option<Self> {
        match parse_duration(s)? {
            Duration::YearMonth(d) => Some(d),
            Duration::DayTime(_) => None,
        }
    }
}

impl Duration {
    /// See [`parse_duration`].
    pub fn parse(s: &str) -> Option<Duration> {
        parse_duration(s)
    }
}

/// Parses a FEEL *duration string* (FEEL `duration(from)`): the lexical space
/// of `xs:dayTimeDuration` (`-?P(nD)?(T(nH)?(nM)?(n(.n*)?S)?)?`) or of
/// `xs:yearMonthDuration` (`-?P(nY)?(nM)?`).
///
/// * At least one component is required; `T` must be followed by at least one
///   time component (`"P"`, `"P0"`, `"PT"`, `"P1H"`, `"P1S"`, `"1D"` → `None`).
/// * Components are unsigned decimal integers of any length (overflow →
///   `None`); only seconds may have a fraction, written `n.`, `n.f` or `.f`
///   (XSD 1.1, so `"PT0.S"` is `PT0S`). Fraction digits beyond nanoseconds
///   are truncated.
/// * Values are normalized: `"PT25H"` = `P1DT1H`, `"P26M"` = `P2Y2M`,
///   `"-P0D"` = `PT0S`.
/// * Mixed forms that are valid `xs:duration` but neither of the two FEEL
///   subtypes (`"P1Y2D"`, `"P1MT1H"`) return `None` (FEEL null), as do week
///   forms (`"P1W"`), per the "duration string" domain of Table 72.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let mut c = Cursor::new(s);
    let neg = c.eat(b'-');
    c.expect(b'P')?;

    // Designator positions: Y=0, M(onths)=1, D=2, H=3, M(inutes)=4, S=5.
    let mut values = [0i128; 6];
    let mut frac_nanos = 0i64;
    let mut last: Option<usize> = None;
    let mut in_time = false;
    let mut any = false;
    let mut time_any = false;
    let mut has_ym = false;
    let mut has_dt = false;

    while !c.is_eof() {
        if c.eat(b'T') {
            if in_time {
                return None;
            }
            in_time = true;
            continue;
        }
        let int_digits = c.digits();
        let mut fraction: Option<&[u8]> = None;
        if c.eat(b'.') {
            fraction = Some(c.digits());
        }
        if int_digits.is_empty() && fraction.is_none_or(|f| f.is_empty()) {
            return None;
        }
        let designator = c.peek()?;
        c.eat(designator);
        let idx = match (in_time, designator) {
            (false, b'Y') => 0,
            (false, b'M') => 1,
            (false, b'D') => 2,
            (true, b'H') => 3,
            (true, b'M') => 4,
            (true, b'S') => 5,
            _ => return None,
        };
        if last.is_some_and(|l| idx <= l) {
            return None;
        }
        if fraction.is_some() && idx != 5 {
            return None;
        }
        last = Some(idx);
        any = true;
        if in_time {
            time_any = true;
        }
        if idx <= 1 {
            has_ym = true;
        } else {
            has_dt = true;
        }
        values[idx] = digits_value(int_digits)?;
        if let Some(f) = fraction {
            frac_nanos = fraction_nanos(f);
        }
    }
    if !any || (in_time && !time_any) || (has_ym && has_dt) {
        return None;
    }
    if has_ym {
        let months = values[0].checked_mul(12)?.checked_add(values[1])?;
        let months = i64::try_from(months).ok()?;
        Some(Duration::YearMonth(YearMonthDuration {
            months: if neg { -months } else { months },
        }))
    } else {
        let nanos = values[2]
            .checked_mul(NANOS_PER_DAY as i128)?
            .checked_add(values[3].checked_mul(NANOS_PER_HOUR)?)?
            .checked_add(values[4].checked_mul(NANOS_PER_MINUTE)?)?
            .checked_add(values[5].checked_mul(NANOS_PER_SEC as i128)?)?
            .checked_add(i128::from(frac_nanos))?;
        Some(Duration::DayTime(DayTimeDuration {
            nanos: if neg { -nanos } else { nanos },
        }))
    }
}

/// Canonical form: `P1DT1H`, `PT0S`, `-PT1H30M`, `PT0.001S`, `P1D`,
/// `P1DT2H3M4.123456789S` (fraction without trailing zeros).
impl fmt::Display for DayTimeDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.nanos == 0 {
            return f.write_str("PT0S");
        }
        if self.nanos < 0 {
            f.write_str("-")?;
        }
        let a = self.nanos.unsigned_abs();
        let days = a / NANOS_PER_DAY as u128;
        let rem = a % NANOS_PER_DAY as u128;
        let h = rem / NANOS_PER_HOUR as u128;
        let m = (rem / NANOS_PER_MINUTE as u128) % 60;
        let s = (rem / NANOS_PER_SEC as u128) % 60;
        let frac = (rem % NANOS_PER_SEC as u128) as u64;
        f.write_str("P")?;
        if days != 0 {
            write!(f, "{days}D")?;
        }
        if rem != 0 {
            f.write_str("T")?;
            if h != 0 {
                write!(f, "{h}H")?;
            }
            if m != 0 {
                write!(f, "{m}M")?;
            }
            if s != 0 || frac != 0 {
                write!(f, "{s}")?;
                write_fraction(f, frac)?;
                f.write_str("S")?;
            }
        }
        Ok(())
    }
}

/// Writes `.ddd` for a non-zero nanosecond fraction, trailing zeros removed.
pub(crate) fn write_fraction(f: &mut fmt::Formatter<'_>, frac_nanos: u64) -> fmt::Result {
    if frac_nanos == 0 {
        return Ok(());
    }
    let mut digits = format!("{frac_nanos:09}");
    while digits.ends_with('0') {
        digits.pop();
    }
    write!(f, ".{digits}")
}

/// Canonical form: `P1Y1M`, `P1Y`, `P2M`, `-P2Y`, `P0M` for zero.
impl fmt::Display for YearMonthDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.months == 0 {
            return f.write_str("P0M");
        }
        if self.months < 0 {
            f.write_str("-")?;
        }
        let a = self.months.unsigned_abs();
        let (y, m) = (a / 12, a % 12);
        f.write_str("P")?;
        if y != 0 {
            write!(f, "{y}Y")?;
        }
        if m != 0 {
            write!(f, "{m}M")?;
        }
        Ok(())
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Duration::DayTime(d) => d.fmt(f),
            Duration::YearMonth(d) => d.fmt(f),
        }
    }
}

impl From<DayTimeDuration> for Duration {
    fn from(d: DayTimeDuration) -> Self {
        Duration::DayTime(d)
    }
}

impl From<YearMonthDuration> for Duration {
    fn from(d: YearMonthDuration) -> Self {
        Duration::YearMonth(d)
    }
}

impl FromStr for Duration {
    type Err = ParseTemporalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_duration(s).ok_or(ParseTemporalError)
    }
}

impl FromStr for DayTimeDuration {
    type Err = ParseTemporalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DayTimeDuration::parse(s).ok_or(ParseTemporalError)
    }
}

impl FromStr for YearMonthDuration {
    type Err = ParseTemporalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        YearMonthDuration::parse(s).ok_or(ParseTemporalError)
    }
}
