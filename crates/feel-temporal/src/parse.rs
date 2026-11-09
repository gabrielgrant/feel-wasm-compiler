//! Lexical-form scanning shared by the date, time and date-time parsers.

use crate::zone::{Zone, MAX_OFFSET_SECONDS};
use crate::{Date, NANOS_PER_SEC};

/// A byte cursor over an ASCII-oriented lexical form.
pub(crate) struct Cursor<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(s: &'a str) -> Self {
        Cursor { s, pos: 0 }
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.pos).copied()
    }

    pub(crate) fn is_eof(&self) -> bool {
        self.pos >= self.s.len()
    }

    pub(crate) fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    pub(crate) fn expect(&mut self, c: u8) -> Option<()> {
        self.eat(c).then_some(())
    }

    /// Consumes a (possibly empty) run of ASCII digits.
    pub(crate) fn digits(&mut self) -> &'a [u8] {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        &self.s.as_bytes()[start..self.pos]
    }

    /// Exactly two ASCII digits.
    pub(crate) fn two_digits(&mut self) -> Option<u32> {
        let b = self.s.as_bytes();
        let (a, c) = (*b.get(self.pos)?, *b.get(self.pos + 1)?);
        if a.is_ascii_digit() && c.is_ascii_digit() {
            self.pos += 2;
            Some(u32::from(a - b'0') * 10 + u32::from(c - b'0'))
        } else {
            None
        }
    }

    /// Everything not consumed yet; consumes it.
    pub(crate) fn take_rest(&mut self) -> &'a str {
        // Only ASCII bytes were consumed so far, so `pos` is a char boundary.
        let r = &self.s[self.pos..];
        self.pos = self.s.len();
        r
    }
}

/// Value of a run of ASCII digits, `None` on `i128` overflow.
pub(crate) fn digits_value(d: &[u8]) -> Option<i128> {
    d.iter().try_fold(0i128, |acc, &c| {
        acc.checked_mul(10)?.checked_add(i128::from(c - b'0'))
    })
}

/// Nanoseconds denoted by the fractional digits `d` (the part after the
/// decimal point). Digits beyond the 9th (sub-nanosecond) are truncated.
pub(crate) fn fraction_nanos(d: &[u8]) -> i64 {
    let mut n = 0i64;
    for i in 0..9 {
        n = n * 10 + d.get(i).map_or(0, |&c| i64::from(c - b'0'));
    }
    n
}

/// Parses the date part `-?YYYY-MM-DD` of an `xs:date` / `xs:dateTime`
/// lexical form (no timezone).
///
/// The year has at least four digits and no leading zero when it has more
/// than four (`"998"` and `"01211"` are rejected); `"-0000"` is rejected; a
/// leading `+` is rejected; the value must be within `[MIN_YEAR, MAX_YEAR]`.
/// Year `0000` is accepted (XSD 1.1 / ISO 8601 astronomical numbering).
pub(crate) fn parse_date_fields(c: &mut Cursor<'_>) -> Option<Date> {
    let neg = c.eat(b'-');
    let yd = c.digits();
    if yd.len() < 4 || (yd.len() > 4 && yd[0] == b'0') || yd.len() > 10 {
        return None;
    }
    let y = digits_value(yd)? as i64;
    if neg && y == 0 {
        return None;
    }
    let year = if neg { -y } else { y };
    c.expect(b'-')?;
    let month = c.two_digits()?;
    c.expect(b'-')?;
    let day = c.two_digits()?;
    Date::from_ymd(year, i64::from(month), i64::from(day))
}

/// Result of scanning a time-of-day lexical form.
pub(crate) struct TimeFields {
    /// Nanoseconds since midnight, in `[0, 86_400e9)`.
    pub(crate) nanos: i64,
    /// `true` for the end-of-day form `24:00:00` (nanos is then 0).
    pub(crate) end_of_day: bool,
    pub(crate) zone: Option<Zone>,
}

/// Parses `hh:mm:ss(.f+)?` followed by an optional zone (`Z`/`z`,
/// `[+-]hh:mm`, `[+-]hh:mm:ss`, or `@IANA/Id`). The whole remaining input must
/// be consumed.
pub(crate) fn parse_time_fields(c: &mut Cursor<'_>) -> Option<TimeFields> {
    let h = c.two_digits()?;
    c.expect(b':')?;
    let m = c.two_digits()?;
    c.expect(b':')?;
    let s = c.two_digits()?;
    let mut frac = 0i64;
    if c.eat(b'.') {
        let fd = c.digits();
        if fd.is_empty() {
            return None;
        }
        frac = fraction_nanos(fd);
    }
    let end_of_day = h == 24 && m == 0 && s == 0 && frac == 0;
    if !end_of_day && (h > 23 || m > 59 || s > 59) {
        return None;
    }
    let nanos = if end_of_day {
        0
    } else {
        (i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(s)) * NANOS_PER_SEC + frac
    };
    let zone = parse_zone_suffix(c)?;
    if !c.is_eof() {
        return None;
    }
    Some(TimeFields {
        nanos,
        end_of_day,
        zone,
    })
}

/// `Some(None)` when no zone follows, `None` on a malformed zone.
fn parse_zone_suffix(c: &mut Cursor<'_>) -> Option<Option<Zone>> {
    match c.peek() {
        None => Some(None),
        Some(z @ (b'Z' | b'z')) => {
            c.eat(z);
            Some(Some(Zone::Offset(0)))
        }
        Some(sign @ (b'+' | b'-')) => {
            c.eat(sign);
            let h = c.two_digits()?;
            c.expect(b':')?;
            let m = c.two_digits()?;
            let s = if c.eat(b':') { c.two_digits()? } else { 0 };
            if m > 59 || s > 59 {
                return None;
            }
            let total = (h * 3600 + m * 60 + s) as i32;
            if total > MAX_OFFSET_SECONDS {
                return None;
            }
            Some(Some(Zone::Offset(if sign == b'-' {
                -total
            } else {
                total
            })))
        }
        Some(b'@') => {
            c.eat(b'@');
            let id = c.take_rest();
            Zone::named(id).map(Some)
        }
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions() {
        assert_eq!(fraction_nanos(b"5"), 500_000_000);
        assert_eq!(fraction_nanos(b"123456789"), 123_456_789);
        assert_eq!(fraction_nanos(b"1234567891234"), 123_456_789);
        assert_eq!(fraction_nanos(b"0001"), 100_000);
    }

    #[test]
    fn digits_overflow() {
        assert_eq!(digits_value(b"123"), Some(123));
        assert_eq!(digits_value(&[b'9'; 50]), None);
    }
}
