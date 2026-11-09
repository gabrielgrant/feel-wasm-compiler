//! Parsing (`Decimal::parse`) and string conversion (`Display`, i.e.
//! `BigDecimal.toPlainString()`, and `to_scientific_string`, i.e.
//! `BigDecimal.toString()`).

use core::fmt;

use crate::{
    finish, round_up_needed, Decimal, Discard, RoundingMode, COEFF_LIMIT, MAX_DIGITS, POW10,
};

/// Saturation bound for parsed exponents: any exponent this large is out of
/// range regardless of the number of digits (lengths are far smaller).
const EXP_LIMIT: i64 = 1_000_000_000_000_000;

pub(crate) fn parse(s: &str) -> Option<Decimal> {
    let b = s.as_bytes();
    let n = b.len();
    let mut i = 0;
    let neg = match b.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };

    // Significant digits are accumulated into `coeff` (at most 34); the
    // 35th becomes the rounding digit and anything after it is sticky.
    let mut coeff: u128 = 0;
    let mut kept: u32 = 0;
    let mut round_digit: u8 = 0;
    let mut sticky = false;
    let mut dropped: i64 = 0;
    let mut frac_digits: i64 = 0;
    let mut any_digit = false;

    let mut push = |d: u8| {
        if kept == 0 && d == 0 {
            return; // leading zero, insignificant
        }
        if kept < MAX_DIGITS {
            coeff = coeff * 10 + d as u128;
            kept += 1;
        } else {
            if dropped == 0 {
                round_digit = d;
            } else if d != 0 {
                sticky = true;
            }
            dropped += 1;
        }
    };

    while i < n && b[i].is_ascii_digit() {
        push(b[i] - b'0');
        any_digit = true;
        i += 1;
    }
    if i < n && b[i] == b'.' {
        i += 1;
        while i < n && b[i].is_ascii_digit() {
            push(b[i] - b'0');
            frac_digits += 1;
            any_digit = true;
            i += 1;
        }
    }
    if !any_digit {
        return None;
    }
    let mut exp: i64 = 0;
    if i < n && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        let eneg = match b.get(i) {
            Some(b'-') => {
                i += 1;
                true
            }
            Some(b'+') => {
                i += 1;
                false
            }
            _ => false,
        };
        let start = i;
        while i < n && b[i].is_ascii_digit() {
            if exp < EXP_LIMIT {
                exp = exp * 10 + (b[i] - b'0') as i64;
            }
            i += 1;
        }
        if i == start {
            return None;
        }
        if eneg {
            exp = -exp;
        }
    }
    if i != n {
        return None;
    }

    let mut scale = frac_digits - exp - dropped;
    if dropped > 0 {
        let d = match round_digit {
            0 if !sticky => Discard::Zero,
            0..=4 => Discard::Below,
            5 if !sticky => Discard::Half,
            _ => Discard::Above,
        };
        if round_up_needed(coeff & 1 == 1, d, neg, RoundingMode::HalfEven) {
            coeff += 1;
            if coeff == COEFF_LIMIT {
                coeff = POW10[33];
                scale -= 1;
            }
        }
    }
    finish(neg, coeff, scale)
}

/// ASCII digits of `c` into `buf`, returning the used tail slice.
fn coeff_digits(c: u128, buf: &mut [u8; 40]) -> &[u8] {
    let mut i = buf.len();
    let mut c = c;
    loop {
        i -= 1;
        buf[i] = b'0' + (c % 10) as u8;
        c /= 10;
        if c == 0 {
            break;
        }
    }
    &buf[i..]
}

const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn write_zeros<W: fmt::Write>(w: &mut W, mut n: usize) -> fmt::Result {
    while n > 0 {
        let k = n.min(ZEROS.len());
        w.write_str(&ZEROS[..k])?;
        n -= k;
    }
    Ok(())
}

fn as_str(d: &[u8]) -> &str {
    // Only ASCII digits are ever stored.
    core::str::from_utf8(d).unwrap_or("")
}

/// Writes the plain (no exponent) form, without sign.
fn write_plain_abs<W: fmt::Write>(w: &mut W, d: &Decimal) -> fmt::Result {
    let mut buf = [0u8; 40];
    let digits = coeff_digits(d.coeff, &mut buf);
    let scale = d.scale;
    if scale <= 0 {
        if d.coeff == 0 {
            return w.write_str("0");
        }
        w.write_str(as_str(digits))?;
        return write_zeros(w, (-(scale as i64)) as usize);
    }
    let scale = scale as usize;
    if digits.len() > scale {
        let split = digits.len() - scale;
        w.write_str(as_str(&digits[..split]))?;
        w.write_str(".")?;
        w.write_str(as_str(&digits[split..]))
    } else {
        w.write_str("0.")?;
        write_zeros(w, scale - digits.len())?;
        w.write_str(as_str(digits))
    }
}

impl fmt::Display for Decimal {
    /// `BigDecimal.toPlainString()`: never uses an exponent and keeps the
    /// scale (`1.10`, `1000` for `1E+3`, `-0.0050`, `0.00`).
    ///
    /// Width, fill, alignment and the `+` flag are honoured like for
    /// integers.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.width().is_none() && !f.sign_plus() {
            if self.neg {
                f.write_str("-")?;
            }
            return write_plain_abs(f, self);
        }
        let mut s = String::new();
        write_plain_abs(&mut s, self)?;
        f.pad_integral(!self.neg, "", &s)
    }
}

pub(crate) fn to_scientific_string(d: &Decimal) -> String {
    let mut buf = [0u8; 40];
    let digits = coeff_digits(d.coeff, &mut buf);
    let scale = d.scale as i64;
    let adjusted = -scale + (digits.len() as i64 - 1);
    let mut out = String::new();
    if d.neg {
        out.push('-');
    }
    if scale == 0 {
        out.push_str(as_str(digits));
    } else if scale > 0 && adjusted >= -6 {
        // Plain notation (scale > 0 here).
        let _ = write_plain_abs(&mut out, d);
    } else {
        out.push_str(as_str(&digits[..1]));
        if digits.len() > 1 {
            out.push('.');
            out.push_str(as_str(&digits[1..]));
        }
        if adjusted != 0 {
            out.push('E');
            if adjusted > 0 {
                out.push('+');
            }
            out.push_str(&adjusted.to_string());
        }
    }
    out
}
