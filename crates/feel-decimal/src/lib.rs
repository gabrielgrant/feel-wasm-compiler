//! # feel-decimal
//!
//! Self-contained decimal arithmetic implementing the number semantics of
//! FEEL (DMN 1.5, clause 10.3.2.3.1 and Tables 57, 59, 60 and 76).
//!
//! ## Model
//!
//! A FEEL number is a pair `(p, s)` of a signed integer coefficient `p` with
//! at most [`MAX_DIGITS`] (34) decimal digits and a scale `s` in
//! [`MIN_SCALE`]`..=`[`MAX_SCALE`] (`-6111..=6176`); its value is
//! `p / 10^s`. This is the IEEE 754-2008 decimal128 value set, and the
//! arithmetic is that of Java's `BigDecimal` with `MathContext.DECIMAL128`
//! (34 digits, rounding `HALF_EVEN`). There is no NaN, no infinity and no
//! negative zero: every operation whose result does not exist returns
//! `None`, which the FEEL runtime maps to `null`.
//!
//! Different pairs may denote the same value (`1`, `1.0`, `1.00`); the scale
//! is preserved by arithmetic exactly as in `BigDecimal`, and is visible in
//! [`Display`](core::fmt::Display) output (`1.10 + 1 = 2.10`), but equality,
//! ordering and hashing only look at the numeric value (`1 = 1.000`).
//!
//! ## Arithmetic
//!
//! * `+`, `-`, `*` ([`Decimal::add`], [`Decimal::sub`], [`Decimal::mul`]):
//!   the exact result has scale `max(s1, s2)` (addition, subtraction) or
//!   `s1 + s2` (multiplication). If its coefficient has more than 34 digits
//!   it is rounded `HALF_EVEN` to 34 digits (reducing the scale).
//!   E.g. `.10 * 30.00 = 3.0000`, `.25 + .2 = 0.45`.
//! * `/` ([`Decimal::div`]): like `BigDecimal.divide(d, DECIMAL128)` (and
//!   Python's `decimal`). If the quotient is exactly representable in 34
//!   digits, it is returned with the scale closest to the preferred scale
//!   `s1 - s2` (`10/4 = 2.5`, `6.0/2 = 3.0`, `1/2 = 0.5`); otherwise it is
//!   rounded `HALF_EVEN` to 34 significant digits (`1/3 = 0.3333…3`, 34
//!   threes). Division by zero is `None`.
//! * [`Decimal::rem`] is the exact truncated remainder (sign of the
//!   dividend, scale `max(s1, s2)`); [`Decimal::modulo`] is FEEL `modulo`
//!   (sign of the divisor).
//! * [`Decimal::pow_i64`] / [`Decimal::pow`], [`Decimal::sqrt`],
//!   [`Decimal::exp`], [`Decimal::ln`] and [`Decimal::log10`] are correctly
//!   rounded to 34 digits (`HALF_EVEN`) using Ziv-style iteration on a
//!   private big-integer implementation; exact results are returned exactly.
//!
//! ## Exceptional behaviour (range rules)
//!
//! Every operation first computes its result rounded to at most 34 digits
//! (as if the scale range were unbounded) and then applies these rules,
//! which implement "requires a scale outside the range of valid scales, the
//! result is null" from the specification:
//!
//! 1. A zero result is always representable: its scale is clamped into
//!    `MIN_SCALE..=MAX_SCALE` (e.g. `0E-7000` becomes `0E-6176`).
//! 2. If the scale is greater than [`MAX_SCALE`] (a very small magnitude),
//!    trailing zeros of the coefficient are stripped (exactly) to lower the
//!    scale toward `MAX_SCALE`. If the scale is still too large the value is
//!    not representable without further rounding: the result is `None`
//!    (underflow). Example: `1E-6176 / 10` is `None`, `10E-6177` is `1E-6176`.
//! 3. If the scale is less than [`MIN_SCALE`] (a very large magnitude), the
//!    coefficient is padded with zeros (scale raised) while it still has at
//!    most 34 digits. If the scale is still too small the result is `None`
//!    (overflow). Example: `1E+6144` is stored as `1000…0E+6111` (34
//!    digits); `1E+6145` is `None`.
//!
//! These rules are the decimal128 "clamped" encoding of IEEE 754 with the
//! difference that results which would be *subnormal and inexact* (would need
//! rounding to fewer than 34 digits) are `None` instead of being rounded
//! again. A consequence is that an inexact result with magnitude below
//! `1E-6143` (34 digits would need a scale above 6176) is `None` unless its
//! 34-digit rounding happens to end in enough zeros; e.g. `exp(-14200)` is
//! `None`. Division by zero, `ln`/`log10` of non-positive numbers, `sqrt` of
//! negative numbers, `0 ** n` for negative `n` and `x ** y` for negative `x`
//! and non-integer `y` are also `None`.
//!
//! ## Conventions where Java and Python differ
//!
//! The reference implementation used for testing is Python's `decimal`
//! module (with an unbounded exponent range plus the rules above); the
//! results of `+ - * /`, `set_scale`, comparisons and parsing are identical
//! to both Python and Java. Where conventions differ:
//!
//! * [`Decimal::sqrt`] of an exact square uses Python's preferred scale
//!   `ceil(s / 2)` (`sqrt(1.0) = 1.0`; Java's `BigDecimal.sqrt` would give `1`).
//! * [`Decimal::pow_i64`] is correctly rounded (Java's `pow(n, mc)` may be off
//!   by one ulp for inexact results); exact results have the scale of
//!   repeated multiplication (`1.10^2 = 1.2100`), and `0.0^2 = 0.00` as in
//!   Java (Python gives `0`).
//! * [`Decimal::pow`] with a non-integer exponent returns exact results with
//!   only significant digits (`4 ** 0.5 = 2`; Python pads to 34 digits).
//!   Integral exponents beyond `±999_999_999` (outside FEEL's input domain
//!   for `**`) are evaluated via `exp(y * ln x)` rather than rejected; callers
//!   that want the strict domain should check it before calling.
//! * [`Decimal::rem`] is always exact, even when the integer quotient has
//!   more than 34 digits (Python's `remainder` raises `DivisionImpossible`).
//!
//! ## Parsing and formatting
//!
//! [`Decimal::parse`] accepts `[+-]? (digits [. digits?] | . digits)
//! ([eE] [+-]? digits)?` and keeps the exact digits (`"1.10"` has scale 2,
//! `"1e3"` has scale -3); longer inputs are rounded `HALF_EVEN` to 34 digits.
//! [`Display`](core::fmt::Display) is `BigDecimal.toPlainString()` (never an
//! exponent); [`Decimal::to_scientific_string`] is `BigDecimal.toString()`.
//!
//! The crate has no dependencies, uses no `unsafe`, and builds for
//! `wasm32-unknown-unknown`. `+ - * /`, comparisons and parsing do not
//! allocate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bigint;
mod format;
mod math;
mod u256;

use core::cmp::Ordering;
use core::hash::{Hash, Hasher};

use u256::{mul_mod, mul_pow10, pow10_mod, U256};

/// Maximum number of significant decimal digits of a coefficient.
pub const MAX_DIGITS: u32 = 34;
/// Smallest valid scale (largest exponent): values up to about `10^6145`.
pub const MIN_SCALE: i32 = -6111;
/// Largest valid scale (smallest exponent): the smallest positive value is
/// `1E-6176`.
pub const MAX_SCALE: i32 = 6176;

/// Powers of ten that fit in a `u128` (10^0 ..= 10^38).
pub(crate) const POW10: [u128; 39] = {
    let mut t = [1u128; 39];
    let mut i = 1;
    while i < 39 {
        t[i] = t[i - 1] * 10;
        i += 1;
    }
    t
};

/// Exclusive upper bound of a coefficient: 10^34.
pub(crate) const COEFF_LIMIT: u128 = POW10[34];

/// Rounding modes, with the semantics of `java.math.RoundingMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundingMode {
    /// Round to nearest; ties go to the even neighbour (banker's rounding).
    HalfEven,
    /// Round to nearest; ties go away from zero.
    HalfUp,
    /// Round to nearest; ties go toward zero.
    HalfDown,
    /// Round away from zero.
    Up,
    /// Round toward zero (truncate).
    Down,
    /// Round toward negative infinity.
    Floor,
    /// Round toward positive infinity.
    Ceiling,
}

/// A FEEL number: a coefficient of at most 34 decimal digits and a scale in
/// `MIN_SCALE..=MAX_SCALE`. See the [crate documentation](crate) for the
/// semantics.
///
/// Invariants: `coeff < 10^34`, `MIN_SCALE <= scale <= MAX_SCALE`, and zero
/// is never negative.
#[derive(Clone, Copy, Debug)]
pub struct Decimal {
    coeff: u128,
    scale: i32,
    neg: bool,
}

// ---------------------------------------------------------------------------
// Internal helpers: digit counting, rounding and the range rules.
// ---------------------------------------------------------------------------

/// Number of decimal digits of `x` (1 for zero).
#[inline]
pub(crate) fn digits_u128(x: u128) -> u32 {
    if x == 0 {
        return 1;
    }
    let bits = 128 - x.leading_zeros();
    // floor(bits * log10(2)) is either digits-1 or digits.
    let est = (bits * 1233) >> 12;
    if x >= POW10[est as usize] {
        est + 1
    } else {
        est
    }
}

/// How the discarded part of a rounded value compares to half a unit in the
/// last kept place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Discard {
    /// Nothing discarded (exact).
    Zero,
    /// Nonzero, less than half.
    Below,
    /// Exactly half.
    Half,
    /// More than half.
    Above,
}

impl Discard {
    #[inline]
    fn from_rem(r: u128, half: u128) -> Discard {
        match r.cmp(&half) {
            _ if r == 0 => Discard::Zero,
            Ordering::Less => Discard::Below,
            Ordering::Equal => Discard::Half,
            Ordering::Greater => Discard::Above,
        }
    }

    /// Refines the classification with a sticky bit from lower digits.
    #[inline]
    fn with_sticky(self, sticky: bool) -> Discard {
        if !sticky {
            return self;
        }
        match self {
            Discard::Zero => Discard::Below,
            Discard::Half => Discard::Above,
            d => d,
        }
    }
}

/// Whether a truncated magnitude must be incremented by one unit.
#[inline]
pub(crate) fn round_up_needed(q_odd: bool, d: Discard, neg: bool, mode: RoundingMode) -> bool {
    if d == Discard::Zero {
        return false;
    }
    match mode {
        RoundingMode::HalfEven => d == Discard::Above || (d == Discard::Half && q_odd),
        RoundingMode::HalfUp => d != Discard::Below,
        RoundingMode::HalfDown => d == Discard::Above,
        RoundingMode::Up => true,
        RoundingMode::Down => false,
        RoundingMode::Floor => neg,
        RoundingMode::Ceiling => !neg,
    }
}

/// `c / 10^k` (truncated) and the classification of the discarded digits.
#[inline]
pub(crate) fn div_pow10_u128(c: u128, k: u32) -> (u128, Discard) {
    if k == 0 {
        return (c, Discard::Zero);
    }
    if k > 38 {
        // 10^k / 2 > u128::MAX >= c
        return (
            0,
            if c == 0 {
                Discard::Zero
            } else {
                Discard::Below
            },
        );
    }
    let p = POW10[k as usize];
    let (q, r) = if c <= u64::MAX as u128 && k <= 19 {
        let (c, p) = (c as u64, p as u64);
        ((c / p) as u128, (c % p) as u128)
    } else {
        (c / p, c % p)
    };
    (q, Discard::from_rem(r, p / 2))
}

/// Rounds `c * 10^-scale` to at most 34 digits (`HALF_EVEN`).
#[inline]
pub(crate) fn round34_u128(c: u128, scale: i64) -> (u128, i64) {
    if c < COEFF_LIMIT {
        return (c, scale);
    }
    let k = digits_u128(c) - MAX_DIGITS;
    let (mut q, d) = div_pow10_u128(c, k);
    let mut k = k as i64;
    if round_up_needed(q & 1 == 1, d, false, RoundingMode::HalfEven) {
        q += 1;
        if q == COEFF_LIMIT {
            q = POW10[33];
            k += 1;
        }
    }
    (q, scale - k)
}

/// Rounds a 256-bit `c * 10^-scale` to at most 34 digits (`HALF_EVEN`).
pub(crate) fn round34_u256(c: U256, scale: i64) -> (u128, i64) {
    if c.hi == 0 {
        return round34_u128(c.lo, scale);
    }
    let k = c.digits() - MAX_DIGITS;
    let (mut q, d) = if k <= 38 {
        let p = POW10[k as usize];
        let (q, r) = c.divrem_u128(p);
        debug_assert_eq!(q.hi, 0);
        (q.lo, Discard::from_rem(r, p / 2))
    } else {
        let (t, r1) = c.divrem_u128(POW10[(k - 38) as usize]);
        let p = POW10[38];
        let (q, r2) = t.divrem_u128(p);
        debug_assert_eq!(q.hi, 0);
        (q.lo, Discard::from_rem(r2, p / 2).with_sticky(r1 != 0))
    };
    let mut k = k as i64;
    if round_up_needed(q & 1 == 1, d, false, RoundingMode::HalfEven) {
        q += 1;
        if q == COEFF_LIMIT {
            q = POW10[33];
            k += 1;
        }
    }
    (q, scale - k)
}

/// Applies the range rules (see the crate docs) to a coefficient that
/// already has at most 34 digits.
#[inline]
pub(crate) fn finish(neg: bool, coeff: u128, scale: i64) -> Option<Decimal> {
    debug_assert!(coeff < COEFF_LIMIT);
    if coeff == 0 {
        let s = scale.clamp(MIN_SCALE as i64, MAX_SCALE as i64) as i32;
        return Some(Decimal {
            coeff: 0,
            scale: s,
            neg: false,
        });
    }
    if scale > MAX_SCALE as i64 {
        let need = scale - MAX_SCALE as i64;
        if need > 33 {
            return None;
        }
        let p = POW10[need as usize];
        if !coeff.is_multiple_of(p) {
            return None;
        }
        return Some(Decimal {
            coeff: coeff / p,
            scale: MAX_SCALE,
            neg,
        });
    }
    if scale < MIN_SCALE as i64 {
        let need = MIN_SCALE as i64 - scale;
        if need > 33 || digits_u128(coeff) as i64 + need > MAX_DIGITS as i64 {
            return None;
        }
        return Some(Decimal {
            coeff: coeff * POW10[need as usize],
            scale: MIN_SCALE,
            neg,
        });
    }
    Some(Decimal {
        coeff,
        scale: scale as i32,
        neg,
    })
}

/// Strips up to `max` trailing zeros from `c`; returns the new coefficient
/// and the number of zeros removed.
#[inline]
pub(crate) fn strip_zeros(mut c: u128, max: u64) -> (u128, u64) {
    if c == 0 {
        return (0, 0);
    }
    let mut removed = 0u64;
    for chunk in [16u64, 8, 4, 2, 1] {
        while max - removed >= chunk && c.is_multiple_of(POW10[chunk as usize]) {
            c /= POW10[chunk as usize];
            removed += chunk;
        }
    }
    (c, removed)
}

impl Default for Decimal {
    fn default() -> Self {
        Decimal::ZERO
    }
}

impl Decimal {
    /// The number `0` (scale 0).
    pub const ZERO: Decimal = Decimal {
        coeff: 0,
        scale: 0,
        neg: false,
    };

    /// The number `1` (scale 0).
    pub const ONE: Decimal = Decimal {
        coeff: 1,
        scale: 0,
        neg: false,
    };

    // ----- construction ----------------------------------------------------

    /// Creates `coefficient * 10^-scale`.
    ///
    /// A coefficient with more than 34 digits is rounded `HALF_EVEN`; the
    /// range rules of the crate documentation then apply (`None` on
    /// overflow/underflow).
    pub fn new(coefficient: i128, scale: i32) -> Option<Decimal> {
        let (c, s) = round34_u128(coefficient.unsigned_abs(), scale as i64);
        finish(coefficient < 0, c, s)
    }

    /// Creates a value from a sign, an unsigned coefficient and a scale
    /// (`value = (-1)^neg * coeff * 10^-scale`), rounding and range-checking
    /// like [`Decimal::new`].
    pub fn from_parts(neg: bool, coeff: u128, scale: i32) -> Option<Decimal> {
        let (c, s) = round34_u128(coeff, scale as i64);
        finish(neg, c, s)
    }

    /// Decomposes the value into `(negative, coefficient, scale)`; the inverse
    /// of [`Decimal::from_parts`].
    #[must_use]
    pub fn to_parts(&self) -> (bool, u128, i32) {
        (self.neg, self.coeff, self.scale)
    }

    /// Converts an `i64` exactly (scale 0).
    pub fn from_i64(v: i64) -> Decimal {
        Decimal {
            coeff: v.unsigned_abs() as u128,
            scale: 0,
            neg: v < 0,
        }
    }

    /// Converts a `u64` exactly (scale 0).
    pub fn from_u64(v: u64) -> Decimal {
        Decimal {
            coeff: v as u128,
            scale: 0,
            neg: false,
        }
    }

    /// Converts an `i128`, rounding `HALF_EVEN` to 34 digits if necessary.
    pub fn from_i128(v: i128) -> Option<Decimal> {
        Decimal::new(v, 0)
    }

    /// Parses a FEEL number literal (see [parsing and formatting](crate#parsing-and-formatting)).
    ///
    /// Grammar: `[+-]? (digits [. digits?] | . digits) ([eE] [+-]? digits)?`,
    /// no surrounding whitespace. The digits are kept exactly (`"1.10"` has
    /// scale 2, `"1e3"` has scale -3, `"1.2e3"` has scale -2) and rounded
    /// `HALF_EVEN` to 34 significant digits if longer. Returns `None` for
    /// malformed input or values out of range.
    pub fn parse(s: &str) -> Option<Decimal> {
        format::parse(s)
    }

    // ----- accessors -------------------------------------------------------

    /// The signed unscaled value `p` (so that the value is `p / 10^scale`).
    #[must_use]
    pub fn coefficient(&self) -> i128 {
        if self.neg {
            -(self.coeff as i128)
        } else {
            self.coeff as i128
        }
    }

    /// The scale `s` (so that the value is `p / 10^s`).
    #[must_use]
    pub fn scale(&self) -> i32 {
        self.scale
    }

    /// Number of digits of the coefficient (1 for zero).
    #[must_use]
    pub fn precision(&self) -> u32 {
        digits_u128(self.coeff)
    }

    /// `true` if the value is zero (of any scale).
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.coeff == 0
    }

    /// `true` if the value is strictly negative.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.neg
    }

    /// `true` if the value is strictly positive.
    #[must_use]
    pub fn is_positive(&self) -> bool {
        !self.neg && self.coeff != 0
    }

    /// `-1`, `0` or `1`.
    #[must_use]
    pub fn signum(&self) -> i32 {
        if self.coeff == 0 {
            0
        } else if self.neg {
            -1
        } else {
            1
        }
    }

    /// `true` if the numeric value is an integer (e.g. `2.00`).
    #[must_use]
    pub fn is_integer(&self) -> bool {
        if self.scale <= 0 || self.coeff == 0 {
            return true;
        }
        if self.scale > 34 {
            return false;
        }
        self.coeff.is_multiple_of(POW10[self.scale as usize])
    }

    /// The negated value (zero stays zero). Scale is preserved.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn neg(&self) -> Decimal {
        Decimal {
            coeff: self.coeff,
            scale: self.scale,
            neg: !self.neg && self.coeff != 0,
        }
    }

    /// The absolute value. Scale is preserved.
    #[must_use]
    pub fn abs(&self) -> Decimal {
        Decimal {
            coeff: self.coeff,
            scale: self.scale,
            neg: false,
        }
    }

    // ----- arithmetic ------------------------------------------------------

    /// `self + o`, rounded `HALF_EVEN` to 34 digits; scale `max(s1, s2)` if
    /// exact. `None` on overflow.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn add(&self, o: &Decimal) -> Option<Decimal> {
        add_impl(self, o.coeff, o.scale, o.neg)
    }

    /// `self - o`, rounded `HALF_EVEN` to 34 digits; scale `max(s1, s2)` if
    /// exact. `None` on overflow.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn sub(&self, o: &Decimal) -> Option<Decimal> {
        add_impl(self, o.coeff, o.scale, !o.neg && o.coeff != 0)
    }

    /// `self * o`, rounded `HALF_EVEN` to 34 digits; scale `s1 + s2` if
    /// exact. `None` on overflow/underflow.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn mul(&self, o: &Decimal) -> Option<Decimal> {
        let neg = self.neg != o.neg;
        let scale = self.scale as i64 + o.scale as i64;
        if self.coeff == 0 || o.coeff == 0 {
            return finish(false, 0, scale);
        }
        if let Some(p) = self.coeff.checked_mul(o.coeff) {
            let (c, s) = round34_u128(p, scale);
            return finish(neg, c, s);
        }
        let (c, s) = round34_u256(U256::mul_u128(self.coeff, o.coeff), scale);
        finish(neg, c, s)
    }

    /// `self / o` with `BigDecimal.divide(o, DECIMAL128)` semantics: an
    /// exact quotient uses the scale closest to `s1 - s2`; an inexact one is
    /// rounded `HALF_EVEN` to 34 digits. `None` if `o` is zero or on
    /// overflow/underflow.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn div(&self, o: &Decimal) -> Option<Decimal> {
        if o.coeff == 0 {
            return None;
        }
        let ideal = self.scale as i64 - o.scale as i64;
        if self.coeff == 0 {
            return finish(false, 0, ideal);
        }
        let neg = self.neg != o.neg;
        let (a, b) = (self.coeff, o.coeff);
        // Fast path: the coefficient quotient is an exact integer, which is
        // then the exact result at exactly the preferred scale.
        if a <= u64::MAX as u128 && b <= u64::MAX as u128 {
            let (a64, b64) = (a as u64, b as u64);
            if a64 % b64 == 0 {
                return finish(neg, (a64 / b64) as u128, ideal);
            }
        } else if a % b == 0 {
            return finish(neg, a / b, ideal);
        }
        // General case (CPython's algorithm): compute a 35/36-digit
        // quotient plus a sticky bit.
        let la = digits_u128(a);
        let lb = digits_u128(b);
        let shift = lb + 35 - la; // >= 2
        let mut scale = ideal + shift as i64;
        let (mut q, exact) = if lb + 35 <= 38 {
            let n = a * POW10[shift as usize];
            (n / b, n.is_multiple_of(b))
        } else {
            let (q, r) = mul_pow10(a, shift).divrem_u128(b);
            debug_assert_eq!(q.hi, 0);
            (q.lo, r == 0)
        };
        if !exact {
            if q % 5 == 0 {
                q += 1;
            }
        } else if scale > ideal {
            let (c, removed) = strip_zeros(q, (scale - ideal) as u64);
            q = c;
            scale -= removed as i64;
        }
        let (c, s) = round34_u128(q, scale);
        finish(neg, c, s)
    }

    /// Truncated remainder `self - o * trunc(self / o)`, computed exactly
    /// (like Java's `BigDecimal.remainder`): the result has the sign of
    /// `self` and scale `max(s1, s2)`. `None` if `o` is zero.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn rem(&self, o: &Decimal) -> Option<Decimal> {
        if o.coeff == 0 {
            return None;
        }
        let (sa, sb) = (self.scale, o.scale);
        let s = sa.max(sb) as i64;
        if self.coeff == 0 {
            return finish(false, 0, s);
        }
        let a = self.coeff;
        let r = if sa >= sb {
            // A = a, B = b * 10^(sa - sb)
            let k = (sa - sb) as usize;
            let b = if k <= 38 {
                o.coeff.checked_mul(POW10[k])
            } else {
                None
            };
            match b {
                Some(b) if b <= a => a % b,
                _ => a, // |B| > |A|: quotient 0
            }
        } else {
            // A = a * 10^(sb - sa) (possibly huge), B = b
            let k = (sb - sa) as u64;
            let b = o.coeff;
            match (k <= 38)
                .then(|| a.checked_mul(POW10[k as usize]))
                .flatten()
            {
                Some(a) => a % b,
                None => mul_mod(a % b, pow10_mod(k, b), b),
            }
        };
        finish(self.neg, r, s)
    }

    /// FEEL `modulo(dividend, divisor)`: `dividend - divisor *
    /// floor(dividend / divisor)`, with the sign of the divisor. Exact
    /// whenever the result is representable in 34 digits (otherwise the
    /// final correction `r + divisor` is rounded like `+`). `None` if the
    /// divisor is zero.
    ///
    /// `modulo(10.1, 4.5) = 1.1`, `modulo(-10.1, 4.5) = 3.4`.
    #[must_use]
    pub fn modulo(&self, o: &Decimal) -> Option<Decimal> {
        let r = self.rem(o)?;
        if r.coeff != 0 && r.neg != o.neg {
            r.add(o)
        } else {
            Some(r)
        }
    }

    // ----- scale manipulation ---------------------------------------------

    /// Returns the value with exactly the given scale, rounding with `mode`
    /// if digits are dropped (FEEL `decimal`, `floor(n, s)`, `round up`,
    /// ...). `None` if `scale` is outside `MIN_SCALE..=MAX_SCALE` or the
    /// result would need more than 34 digits.
    #[must_use]
    pub fn set_scale(&self, scale: i64, mode: RoundingMode) -> Option<Decimal> {
        if scale < MIN_SCALE as i64 || scale > MAX_SCALE as i64 {
            return None;
        }
        let cur = self.scale as i64;
        if scale >= cur {
            if self.coeff == 0 {
                return Some(Decimal {
                    coeff: 0,
                    scale: scale as i32,
                    neg: false,
                });
            }
            let k = scale - cur;
            if digits_u128(self.coeff) as i64 + k > MAX_DIGITS as i64 {
                return None;
            }
            let c = self.coeff * POW10[k as usize];
            return Some(Decimal {
                coeff: c,
                scale: scale as i32,
                neg: self.neg,
            });
        }
        let k = (cur - scale) as u64;
        let (mut q, d) = div_pow10_u128(self.coeff, k.min(39) as u32);
        if round_up_needed(q & 1 == 1, d, self.neg, mode) {
            q += 1;
        }
        Some(Decimal {
            coeff: q,
            scale: scale as i32,
            neg: self.neg && q != 0,
        })
    }

    /// Like [`Decimal::set_scale`], but a value whose scale is already `<=
    /// scale` is returned unchanged (no zero padding).
    #[must_use]
    pub fn round_to_scale(&self, scale: i64, mode: RoundingMode) -> Option<Decimal> {
        if self.scale as i64 <= scale {
            Some(*self)
        } else {
            self.set_scale(scale, mode)
        }
    }

    /// Removes trailing zeros from the coefficient (zero becomes `0` with
    /// scale 0). The scale never goes below `MIN_SCALE`.
    #[must_use]
    pub fn normalize(&self) -> Decimal {
        if self.coeff == 0 {
            return Decimal::ZERO;
        }
        let max = (self.scale as i64 - MIN_SCALE as i64) as u64;
        let (c, removed) = strip_zeros(self.coeff, max);
        Decimal {
            coeff: c,
            scale: (self.scale as i64 - removed as i64) as i32,
            neg: self.neg,
        }
    }

    fn to_integral(self, mode: RoundingMode) -> Decimal {
        match self.set_scale(0, mode) {
            Some(d) => d,
            // Only possible for scale < 0 with too many digits to pad: the
            // value is already integral.
            None => self,
        }
    }

    /// Largest integer `<= self`, with scale 0 (values with a negative scale
    /// that cannot be padded to scale 0 are returned unchanged).
    #[must_use]
    pub fn floor(&self) -> Decimal {
        self.to_integral(RoundingMode::Floor)
    }

    /// Smallest integer `>= self`, with scale 0 (see [`Decimal::floor`]).
    #[must_use]
    pub fn ceil(&self) -> Decimal {
        self.to_integral(RoundingMode::Ceiling)
    }

    /// Integer part (rounding toward zero), with scale 0 (see
    /// [`Decimal::floor`]).
    #[must_use]
    pub fn trunc(&self) -> Decimal {
        self.to_integral(RoundingMode::Down)
    }

    // ----- conversions -----------------------------------------------------

    /// The value as `i128` if it is an integer that fits.
    #[must_use]
    pub fn to_i128(&self) -> Option<i128> {
        if !self.is_integer() {
            return None;
        }
        let m: u128 = if self.coeff == 0 {
            0
        } else if self.scale <= 0 {
            let k = (-(self.scale as i64)) as usize;
            if k > 38 {
                return None;
            }
            self.coeff.checked_mul(POW10[k])?
        } else {
            self.coeff / POW10[self.scale as usize]
        };
        if self.neg {
            if m <= 1u128 << 127 {
                Some((m as i128).wrapping_neg())
            } else {
                None
            }
        } else {
            i128::try_from(m).ok()
        }
    }

    /// The value as `i64` if it is an integer that fits.
    #[must_use]
    pub fn to_i64(&self) -> Option<i64> {
        self.to_i128().and_then(|v| i64::try_from(v).ok())
    }

    /// The nearest `f64` (approximate; for non-semantic uses only).
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        if self.coeff == 0 {
            return 0.0;
        }
        let v = if self.coeff < (1u128 << 53) && self.scale.abs() <= 22 {
            // Both operands are exact, so one correctly rounded operation.
            let c = self.coeff as f64;
            let p = 10f64.powi(self.scale.abs());
            if self.scale >= 0 {
                c / p
            } else {
                c * p
            }
        } else {
            format!("{}e{}", self.coeff, -(self.scale as i64))
                .parse::<f64>()
                .unwrap_or(0.0)
        };
        if self.neg {
            -v
        } else {
            v
        }
    }

    // ----- powers and transcendental functions -----------------------------

    /// `self ^ n` for an integer exponent, correctly rounded to 34 digits.
    ///
    /// Exact results are exact with the scale of repeated multiplication
    /// (`1.1^2 = 1.21`, `1.10^2 = 1.2100`); negative exponents use division
    /// semantics (`2^-1 = 0.5`, `10^-2 = 0.01`). `x^0 = 1` (including
    /// `0^0`); `0^n` is zero with scale `n * s` for `n > 0` and `None` for
    /// `n < 0`. Overflow and underflow give `None` (detected quickly even for
    /// huge `n`).
    #[must_use]
    pub fn pow_i64(&self, n: i64) -> Option<Decimal> {
        math::pow_i64(self, n)
    }

    /// `self ^ e` (FEEL `**`).
    ///
    /// If `e` is an integer with `|e| <= 999_999_999` this is
    /// [`Decimal::pow_i64`]. Otherwise: for `self > 0` the result is
    /// `exp(e * ln(self))` correctly rounded to 34 digits, except that exact
    /// results are returned exactly with trailing fractional zeros removed
    /// (`4 ** 0.5 = 2`, `0.25 ** 0.5 = 0.5`); `0 ** e` is `0` for `e > 0`;
    /// everything else (negative base, `0 ** e` with `e <= 0`, overflow,
    /// underflow) is `None`.
    #[must_use]
    pub fn pow(&self, e: &Decimal) -> Option<Decimal> {
        math::pow(self, e)
    }

    /// Square root, correctly rounded to 34 digits; `None` if negative.
    ///
    /// Exact results use the preferred scale `ceil(s / 2)` (as Python's
    /// `decimal`): `sqrt(16) = 4`, `sqrt(0.25) = 0.5`, `sqrt(1.00) = 1.0`.
    #[must_use]
    pub fn sqrt(&self) -> Option<Decimal> {
        math::sqrt(self)
    }

    /// Natural logarithm, correctly rounded to 34 digits; `None` if
    /// `self <= 0`. `ln(1) = 0`.
    #[must_use]
    pub fn ln(&self) -> Option<Decimal> {
        math::ln(self)
    }

    /// `e ^ self`, correctly rounded to 34 digits; `None` on overflow or
    /// underflow. `exp(0) = 1`.
    #[must_use]
    pub fn exp(&self) -> Option<Decimal> {
        math::exp(self)
    }

    /// Base-10 logarithm, correctly rounded to 34 digits; exact (scale 0)
    /// for powers of ten. `None` if `self <= 0`.
    #[must_use]
    pub fn log10(&self) -> Option<Decimal> {
        math::log10(self)
    }

    /// `BigDecimal.toString()`: plain notation when the scale is `>= 0` and
    /// the adjusted exponent is `>= -6`, scientific notation (`1.23E+5`,
    /// `1E-7`) otherwise.
    #[must_use]
    pub fn to_scientific_string(&self) -> String {
        format::to_scientific_string(self)
    }

    /// Compares the magnitudes of two nonzero values.
    fn cmp_abs(&self, o: &Decimal) -> Ordering {
        let adj_a = digits_u128(self.coeff) as i64 - self.scale as i64;
        let adj_b = digits_u128(o.coeff) as i64 - o.scale as i64;
        if adj_a != adj_b {
            return adj_a.cmp(&adj_b);
        }
        // Same adjusted exponent: aligning needs at most 33 extra digits on
        // the shorter coefficient, so the product stays below 10^34.
        match self.scale.cmp(&o.scale) {
            Ordering::Equal => self.coeff.cmp(&o.coeff),
            Ordering::Less => (self.coeff * POW10[(o.scale - self.scale) as usize]).cmp(&o.coeff),
            Ordering::Greater => self
                .coeff
                .cmp(&(o.coeff * POW10[(self.scale - o.scale) as usize])),
        }
    }
}

/// `a + (-1)^b_neg * b_coeff * 10^-b_scale`.
fn add_impl(a: &Decimal, b_coeff: u128, b_scale: i32, b_neg: bool) -> Option<Decimal> {
    // `hi` is the operand with the smaller scale (its last digit is the more
    // significant one), `lo` the one with the larger scale.
    let ((hc, hs, hn), (lc, ls, ln)) = if a.scale <= b_scale {
        ((a.coeff, a.scale, a.neg), (b_coeff, b_scale, b_neg))
    } else {
        ((b_coeff, b_scale, b_neg), (a.coeff, a.scale, a.neg))
    };
    if hc == 0 {
        // Result is `lo` exactly (scale max(s1, s2) = ls).
        return Some(Decimal {
            coeff: lc,
            scale: ls,
            neg: ln && lc != 0,
        });
    }
    let d = (ls - hs) as u32;
    if d == 0 {
        let (c, neg) = if hn == ln {
            (hc + lc, hn)
        } else if hc >= lc {
            (hc - lc, hn)
        } else {
            (lc - hc, ln)
        };
        if c < COEFF_LIMIT {
            return Some(Decimal {
                coeff: c,
                scale: ls,
                neg: neg && c != 0,
            });
        }
        let (c, s) = round34_u128(c, ls as i64);
        return finish(neg, c, s);
    }
    let h_digits = digits_u128(hc);
    if lc == 0 || h_digits + d >= 70 {
        // `lo` is zero or lies entirely below the rounding digit (at least
        // 36 dropped digits), so under HALF_EVEN the result is `hi` padded
        // to 34 digits (or to scale ls if that needs fewer digits).
        if h_digits + d <= MAX_DIGITS {
            return finish(hn, hc * POW10[d as usize], ls as i64);
        }
        let pad = MAX_DIGITS - h_digits;
        return finish(hn, hc * POW10[pad as usize], hs as i64 + pad as i64);
    }
    // Exact sum of `hc * 10^d` and `lc` (fewer than 70 digits).
    if d <= 38 {
        if let Some(hv) = hc.checked_mul(POW10[d as usize]) {
            let r = if hn == ln {
                hv.checked_add(lc).map(|c| (c, hn))
            } else if hv >= lc {
                Some((hv - lc, hn))
            } else {
                Some((lc - hv, ln))
            };
            if let Some((c, neg)) = r {
                let (c, s) = round34_u128(c, ls as i64);
                return finish(neg, c, s);
            }
        }
    }
    let hv = mul_pow10(hc, d);
    // Here hv >= 2^128 > lc (or the u128 sum overflowed), so the result has
    // the sign of `hi`.
    let v = if hn == ln {
        hv.add_u128(lc)
    } else {
        hv.sub_u128(lc)
    };
    let (c, s) = round34_u256(v, ls as i64);
    finish(hn, c, s)
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    /// Numeric order (scale is ignored).
    fn cmp(&self, other: &Self) -> Ordering {
        let (sa, sb) = (self.signum(), other.signum());
        if sa != sb {
            return sa.cmp(&sb);
        }
        if sa == 0 {
            return Ordering::Equal;
        }
        let m = self.cmp_abs(other);
        if sa > 0 {
            m
        } else {
            m.reverse()
        }
    }
}

impl Hash for Decimal {
    /// Hashes the normalized value, consistent with `Eq` (`1.0` and `1`
    /// hash alike).
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.coeff == 0 {
            0u8.hash(state);
            return;
        }
        let (c, removed) = strip_zeros(self.coeff, u64::MAX);
        1u8.hash(state);
        self.neg.hash(state);
        c.hash(state);
        (self.scale as i64 - removed as i64).hash(state);
    }
}

impl core::ops::Neg for Decimal {
    type Output = Decimal;
    fn neg(self) -> Decimal {
        Decimal::neg(&self)
    }
}

impl core::str::FromStr for Decimal {
    type Err = ParseDecimalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::parse(s).ok_or(ParseDecimalError)
    }
}

/// Error returned by [`str::parse`] for malformed or out-of-range numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseDecimalError;

impl core::fmt::Display for ParseDecimalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("invalid or out-of-range FEEL number")
    }
}

impl std::error::Error for ParseDecimalError {}

impl From<i64> for Decimal {
    fn from(v: i64) -> Self {
        Decimal::from_i64(v)
    }
}

impl From<i32> for Decimal {
    fn from(v: i32) -> Self {
        Decimal::from_i64(v as i64)
    }
}

impl From<u64> for Decimal {
    fn from(v: u64) -> Self {
        Decimal::from_u64(v)
    }
}

impl From<u32> for Decimal {
    fn from(v: u32) -> Self {
        Decimal::from_u64(v as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_count_estimate_is_exact() {
        for bits in 1..=128u32 {
            let lo = 1u128 << (bits - 1);
            let hi = if bits == 128 {
                u128::MAX
            } else {
                (1u128 << bits) - 1
            };
            for x in [lo, hi] {
                assert_eq!(digits_u128(x) as usize, x.to_string().len(), "x={x}");
            }
        }
        for (k, &p) in POW10.iter().enumerate() {
            assert_eq!(digits_u128(p) as usize, k + 1);
            if k > 0 {
                assert_eq!(digits_u128(p - 1) as usize, k);
            }
        }
    }

    #[test]
    fn strip_zeros_limits() {
        assert_eq!(strip_zeros(1_000_000, 3), (1_000, 3));
        assert_eq!(strip_zeros(1_000_000, 100), (1, 6));
        assert_eq!(strip_zeros(POW10[33], 100), (1, 33));
        assert_eq!(strip_zeros(12, 100), (12, 0));
    }
}
