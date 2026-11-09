//! Powers, square root and transcendental functions.
//!
//! `exp`, `ln`, `log10` and non-integer powers are ports of the
//! correctly-rounded algorithms of CPython's `_pydecimal` module (`_dexp`,
//! `_dlog`, `_dlog10`, `_dpower`, `_power_exact`): an integer fixed-point
//! approximation with a rigorous error bound of one unit in the last place
//! is computed with a few guard digits, and the precision is increased
//! until the approximation can be rounded unambiguously (Ziv's strategy).

use crate::bigint::BigInt;
use crate::u256::{mul_pow10, U256};
use crate::{
    digits_u128, finish, round34_u128, strip_zeros, Decimal, COEFF_LIMIT, MAX_DIGITS, POW10,
};

/// Target precision.
const P: i64 = MAX_DIGITS as i64;

/// Truncated digits of ln(10) = 2.302585...; `log10_digits(p)` needs the
/// first `p + 1` of them. Longer requests are computed on the fly.
const LN10_DIGITS: &str = "23025850929940456840179914546843642076011014886287729760333279009675726096773524802359972050895982983419677840422862486334095254650828067566662873690987816894829072083255546808437998948262331985283935053089653777326288461633662222876982198867465436674744042432743651550489343149393914796194044002221051017141748003688084012647080685567743216228355220114804663715659121373450747856947683463616792101806445070648000277502684916746550586856935";

/// Upper bound on the extra precision tried by the Ziv loops before giving up
/// and rounding the best approximation (never reached in practice: the
/// results are irrational, so a few iterations always suffice).
const MAX_EXTRA: i64 = 400;

// ---------------------------------------------------------------------------
// Rounding of big results
// ---------------------------------------------------------------------------

/// Rounds `|mag| * 10^-scale` `HALF_EVEN` to 34 digits and applies the range
/// rules. `mag` must be non-negative.
fn round_big(neg: bool, mag: &BigInt, scale: i64) -> Option<Decimal> {
    if let Some(v) = mag.abs_u128() {
        let (c, s) = round34_u128(v, scale);
        return finish(neg, c, s);
    }
    let k = mag.dec_len() as i64 - P;
    let p = BigInt::pow10(k as u64);
    let (q, r) = mag.divrem_abs(&p);
    let mut q = q.abs_u128().expect("34-digit quotient");
    let twice = r.shl(1);
    let up = match twice.cmp(&p) {
        core::cmp::Ordering::Greater => true,
        core::cmp::Ordering::Equal => q & 1 == 1,
        core::cmp::Ordering::Less => false,
    };
    let mut k = k;
    if up {
        q += 1;
        if q == COEFF_LIMIT {
            q = POW10[33];
            k += 1;
        }
    }
    finish(neg, q, scale - k)
}

/// The Ziv termination test of `_pydecimal`: an approximation `coeff` (with
/// error < 1) to a result rounded to `P` digits is safe to round iff it is
/// not a multiple of `5 * 10^(len(coeff) - P - 1)`.
fn needs_more_precision(coeff: &BigInt) -> bool {
    let len = coeff.dec_len() as i64;
    if len < P + 1 {
        return true;
    }
    let m = BigInt::pow10((len - P - 1) as u64).mul_u64(5);
    coeff.is_multiple_of(&m)
}

// ---------------------------------------------------------------------------
// Ports of the `_pydecimal` helpers
// ---------------------------------------------------------------------------

fn big(v: i64) -> BigInt {
    BigInt::from_i64(v)
}

/// Closest integer to `a / b` (`b > 0`), ties to even.
fn div_nearest(a: &BigInt, b: &BigInt) -> BigInt {
    let (q, r) = a.divmod_floor(b);
    let lhs = r.shl(1).add_i64(q.is_odd() as i64);
    if lhs > *b {
        q.add_i64(1)
    } else {
        q
    }
}

/// Closest integer to `x / 2^shift`, ties to even.
fn rshift_nearest(x: &BigInt, shift: u64) -> BigInt {
    let q = x.shr_floor(shift);
    let r = x.sub(&q.shl(shift)); // x & (2^shift - 1), non-negative
    let b = BigInt::one().shl(shift);
    let lhs = r.shl(1).add_i64(q.is_odd() as i64);
    if lhs > b {
        q.add_i64(1)
    } else {
        q
    }
}

/// Closest integer to the square root of `n > 0`, starting from `a > 0`.
fn sqrt_nearest(n: &BigInt, a: &BigInt) -> BigInt {
    let mut a = a.clone();
    let mut b = BigInt::zero();
    let neg_n = n.negate();
    while a != b {
        b = a.clone();
        // a = (a - (-n // a)) >> 1
        let t = neg_n.divmod_floor(&a).0;
        a = a.sub(&t).shr_floor(1);
    }
    a
}

/// `ceil(10 * (p + 1) / 24)`: number of series terms for `M = 10^p`
/// (`-int(-10*len(str(M))//(3*L))` with `L = 8`).
fn series_terms(p: i64) -> i64 {
    (10 * (p + 1) + 23) / 24
}

/// Integer approximation to `M * log(x / M)` with `M = 10^p` (`_ilog`).
fn ilog(x: &BigInt, p: i64) -> BigInt {
    const L: u64 = 8;
    let m = BigInt::pow10(p as u64);
    let mut y = x.sub(&m);
    let mut r: u64 = 0;
    loop {
        let ay = y.abs();
        let cont = if r <= L {
            ay.shl(L - r) >= m
        } else {
            ay.shr_floor(r - L) >= m
        };
        if !cont {
            break;
        }
        let inner = m.mul(&m.add(&rshift_nearest(&y, r)));
        let s = sqrt_nearest(&inner, &m);
        y = div_nearest(&m.mul(&y).shl(1), &m.add(&s));
        r += 1;
    }
    let t = series_terms(p);
    let yshift = rshift_nearest(&y, r);
    let mut w = div_nearest(&m, &big(t));
    for k in (1..t).rev() {
        w = div_nearest(&m, &big(k)).sub(&div_nearest(&yshift.mul(&w), &m));
    }
    div_nearest(&w.mul(&y), &m)
}

/// `floor(10^p * ln(10))` (`_log10_digits`).
fn log10_digits(p: i64) -> BigInt {
    let p = p.max(0);
    if (p as usize) < LN10_DIGITS.len() {
        return BigInt::from_dec_str(&LN10_DIGITS[..p as usize + 1]);
    }
    let mut extra = 3;
    let digits = loop {
        let mp = p + extra + 2;
        let m = BigInt::pow10(mp as u64);
        let v = div_nearest(&ilog(&m.mul_u64(10), mp), &big(100));
        let s = v.to_dec_string();
        if !s.ends_with(&"0".repeat(extra as usize)) {
            break s;
        }
        extra += 3;
    };
    let trimmed = digits.trim_end_matches('0');
    let reliable = &trimmed[..trimmed.len() - 1];
    BigInt::from_dec_str(&reliable[..(p as usize + 1).min(reliable.len())])
}

/// Integer approximation to `10^p * ln(c * 10^e)`, error at most 1
/// (`_dlog`). `c > 0`, `c * 10^e != 1`.
fn dlog(c: &BigInt, e: i64, p: i64) -> BigInt {
    let p = p + 2;
    let l = c.dec_len() as i64;
    let f = e + l - ((e + l >= 1) as i64);
    let log_d = if p > 0 {
        let k = e + p - f;
        let c2 = if k >= 0 {
            c.mul(&BigInt::pow10(k as u64))
        } else {
            div_nearest(c, &BigInt::pow10((-k) as u64))
        };
        ilog(&c2, p)
    } else {
        BigInt::zero()
    };
    let f_log_ten = if f != 0 {
        let extra = big(f).dec_len() as i64 - 1;
        if p + extra >= 0 {
            div_nearest(
                &big(f).mul(&log10_digits(p + extra)),
                &BigInt::pow10(extra as u64),
            )
        } else {
            BigInt::zero()
        }
    } else {
        BigInt::zero()
    };
    div_nearest(&f_log_ten.add(&log_d), &big(100))
}

/// Integer approximation to `10^p * log10(c * 10^e)`, error at most 1
/// (`_dlog10`). `c > 0`, `c * 10^e != 1`.
fn dlog10(c: &BigInt, e: i64, p: i64) -> BigInt {
    let p = p + 2;
    let l = c.dec_len() as i64;
    let f = e + l - ((e + l >= 1) as i64);
    let (log_d, log_tenpower) = if p > 0 {
        let m = BigInt::pow10(p as u64);
        let k = e + p - f;
        let c2 = if k >= 0 {
            c.mul(&BigInt::pow10(k as u64))
        } else {
            div_nearest(c, &BigInt::pow10((-k) as u64))
        };
        let log_d = ilog(&c2, p);
        let log_10 = log10_digits(p);
        let log_d = div_nearest(&log_d.mul(&m), &log_10);
        (log_d, big(f).mul(&m))
    } else {
        (
            BigInt::zero(),
            div_nearest(&big(f), &BigInt::pow10((-p) as u64)),
        )
    };
    div_nearest(&log_tenpower.add(&log_d), &big(100))
}

/// Integer approximation to `M * exp(x / M)` with `M = 10^p`, `x >= 0`
/// small (`_iexp`).
fn iexp(x: &BigInt, p: i64) -> BigInt {
    const L: u64 = 8;
    let m = BigInt::pow10(p as u64);
    let r = x.shl(L).divmod_floor(&m).0.bit_len();
    let t = series_terms(p);
    let mut y = div_nearest(x, &big(t));
    let mshift = m.shl(r);
    for i in (1..t).rev() {
        y = div_nearest(&x.mul(&mshift.add(&y)), &mshift.mul_u64(i as u64));
    }
    for k in (0..r).rev() {
        let mshift = m.shl(k + 2);
        y = div_nearest(&y.mul(&y.add(&mshift)), &mshift);
    }
    m.add(&y)
}

/// Approximation `(d, f)` to `exp(c * 10^e)` with `p` digits: `10^(p-1) <= d
/// <= 10^p` and `(d-1)*10^f < exp(c*10^e) < (d+1)*10^f` (`_dexp`).
fn dexp(c: &BigInt, e: i64, p: i64) -> (BigInt, i64) {
    let p = p + 2;
    let extra = (e + c.dec_len() as i64 - 1).max(0);
    let q = p + extra;
    let shift = e + q;
    let cshift = if shift >= 0 {
        c.mul(&BigInt::pow10(shift as u64))
    } else {
        c.divmod_floor(&BigInt::pow10((-shift) as u64)).0
    };
    let (quot, rem) = cshift.divmod_floor(&log10_digits(q));
    let rem = div_nearest(&rem, &BigInt::pow10(extra as u64));
    let quot = quot.to_i64().expect("exponent of exp() result fits in i64");
    (div_nearest(&iexp(&rem, p), &big(1000)), quot - p + 3)
}

/// Approximation `(c, e)` to `x^y` with `x = xc * 10^xe > 0`, `x != 1`,
/// `y = yc * 10^ye != 0`, error in `c` below 1 (`_dpower`).
fn dpower(xc: &BigInt, xe: i64, yc: &BigInt, ye: i64, p: i64) -> (BigInt, i64) {
    let b = yc.dec_len() as i64 + ye;
    let lxc = dlog(xc, xe, p + b + 1);
    let shift = ye - b;
    let pc = if shift >= 0 {
        lxc.mul(yc).mul(&BigInt::pow10(shift as u64))
    } else {
        div_nearest(&lxc.mul(yc), &BigInt::pow10((-shift) as u64))
    };
    if pc.is_zero() {
        // x^y is within 10^-p of 1; prefer a result that is not exactly 1.
        if (xc.dec_len() as i64 + xe >= 1) == !yc.is_neg() {
            (BigInt::pow10((p - 1) as u64).add_i64(1), 1 - p)
        } else {
            (BigInt::pow10(p as u64).add_i64(-1), -p)
        }
    } else {
        let (coeff, exp) = dexp(&pc, -(p + 1), p + 1);
        (div_nearest(&coeff, &big(10)), exp + 1)
    }
}

/// Number of decimal digits of `|v|`.
fn len_i128(v: i128) -> i64 {
    digits_u128(v.unsigned_abs()) as i64
}

/// Lexicographic comparison of the decimal strings of `a` and `b`
/// (`str(a) < str(b)` in Python).
fn lex_less(a: u128, b: u128) -> bool {
    a.to_string() < b.to_string()
}

/// Lower bound `r` with `|ln(x)| >= 10^r` for `x = c * 10^e > 0`, `x != 1`
/// (`_ln_exp_bound`).
fn ln_exp_bound(c: u128, e: i64) -> i64 {
    let adj = e + digits_u128(c) as i64 - 1;
    if adj >= 1 {
        return len_i128((adj * 23 / 10) as i128) - 1;
    }
    if adj <= -2 {
        return len_i128(((-1 - adj) * 23 / 10) as i128) - 1;
    }
    if adj == 0 {
        // 1 < x < 10, e <= 0
        let num = c - POW10[(-e) as usize];
        let den = c;
        return len_i128(num as i128) - len_i128(den as i128) - lex_less(num, den) as i64;
    }
    // adj == -1: 0.1 <= x < 1
    let num = POW10[(-e) as usize] - c;
    e + len_i128(num as i128) - 1
}

/// Lower bound `r` with `|log10(x)| >= 10^r` (`_log10_exp_bound`).
fn log10_exp_bound(c: u128, e: i64) -> i64 {
    let adj = e + digits_u128(c) as i64 - 1;
    if adj >= 1 {
        return len_i128(adj as i128) - 1;
    }
    if adj <= -2 {
        return len_i128((-1 - adj) as i128) - 1;
    }
    if adj == 0 {
        let num = c - POW10[(-e) as usize];
        let den = 231 * c;
        return len_i128(num as i128) - len_i128(den as i128) - lex_less(num, den) as i64 + 2;
    }
    let num = POW10[(-e) as usize] - c;
    len_i128(num as i128) + e - (num.to_string().as_str() < "231") as i64 - 1
}

// ---------------------------------------------------------------------------
// exp, ln, log10
// ---------------------------------------------------------------------------

/// `1.000000000000000000000000000000000` (34 digits).
fn one_34() -> Decimal {
    Decimal {
        coeff: POW10[33],
        scale: 33,
        neg: false,
    }
}

pub(crate) fn exp(x: &Decimal) -> Option<Decimal> {
    if x.coeff == 0 {
        return Some(Decimal::ONE);
    }
    // The largest representable value is just below 10^6145
    // (ln = 14149.66...); the smallest positive one is 1E-6176
    // (ln = -14220.96...).
    let adj = digits_u128(x.coeff) as i64 - 1 - x.scale as i64;
    if adj >= 5 {
        return None;
    }
    if !x.neg && *x > Decimal::from_i64(14150) {
        return None;
    }
    if x.neg && *x < Decimal::from_i64(-14221) {
        return None;
    }
    // Tiny arguments round to exactly 1 (34 digits).
    if (!x.neg && adj < -P) || (x.neg && adj < -P - 1) {
        return Some(one_34());
    }
    let c = BigInt::from_i128(x.coefficient());
    let e = -(x.scale as i64);
    let mut extra = 3;
    let (coeff, exp) = loop {
        let (coeff, exp) = dexp(&c, e, P + extra);
        if !needs_more_precision(&coeff) || extra >= MAX_EXTRA {
            break (coeff, exp);
        }
        extra += 3;
    };
    round_big(false, &coeff, -exp)
}

pub(crate) fn ln(x: &Decimal) -> Option<Decimal> {
    if x.coeff == 0 || x.neg {
        return None;
    }
    if *x == Decimal::ONE {
        return Some(Decimal::ZERO);
    }
    let c = BigInt::from_u128(x.coeff);
    let e = -(x.scale as i64);
    let mut places = P - ln_exp_bound(x.coeff, e) + 2;
    let start = places;
    let coeff = loop {
        let coeff = dlog(&c, e, places);
        if !needs_more_precision(&coeff.abs()) || places - start >= MAX_EXTRA {
            break coeff;
        }
        places += 3;
    };
    round_big(coeff.is_neg(), &coeff.abs(), places)
}

pub(crate) fn log10(x: &Decimal) -> Option<Decimal> {
    if x.coeff == 0 || x.neg {
        return None;
    }
    let e = -(x.scale as i64);
    let (stripped, _) = strip_zeros(x.coeff, u64::MAX);
    if stripped == 1 {
        // Exact power of ten.
        let n = e + digits_u128(x.coeff) as i64 - 1;
        return Some(Decimal::from_i64(n));
    }
    let c = BigInt::from_u128(x.coeff);
    let mut places = P - log10_exp_bound(x.coeff, e) + 2;
    let start = places;
    let coeff = loop {
        let coeff = dlog10(&c, e, places);
        if !needs_more_precision(&coeff.abs()) || places - start >= MAX_EXTRA {
            break coeff;
        }
        places += 3;
    };
    round_big(coeff.is_neg(), &coeff.abs(), places)
}

// ---------------------------------------------------------------------------
// sqrt
// ---------------------------------------------------------------------------

/// `floor(sqrt(c))` for `c < 10^70`.
fn isqrt_u256(c: U256) -> u128 {
    if c.hi == 0 && c.lo == 0 {
        return 0;
    }
    // Start from a guaranteed overestimate and run Newton's iteration
    // downward (it decreases monotonically to floor(sqrt(c))).
    let est = c.to_f64().sqrt();
    let mut g: u128 = (est * (1.0 + 1e-9)) as u128 + 2;
    loop {
        let (q, _) = c.divrem_u128(g);
        debug_assert_eq!(q.hi, 0);
        let q = q.lo;
        if g <= q {
            return g;
        }
        g = (g + q) / 2;
    }
}

pub(crate) fn sqrt(x: &Decimal) -> Option<Decimal> {
    if x.neg {
        return None;
    }
    // Python exponent and ideal exponent floor(exp / 2).
    let exp = -(x.scale as i64);
    if x.coeff == 0 {
        return finish(false, 0, -(exp >> 1));
    }
    const PREC: u32 = 35;
    let len = digits_u128(x.coeff);
    let mut e = exp >> 1;
    let (c, l) = if exp & 1 != 0 {
        (x.coeff * 10, (len >> 1) + 1)
    } else {
        (x.coeff, (len + 1) >> 1)
    };
    // Rescale so that c has exactly PREC base-100 digits (shift >= 17).
    let shift = PREC - l;
    let c = mul_pow10(c, 2 * shift);
    e -= shift as i64;
    let mut n = isqrt_u256(c);
    if U256::mul_u128(n, n) == c {
        n /= POW10[shift as usize];
        e += shift as i64;
    } else if n.is_multiple_of(5) {
        n += 1;
    }
    let (n, s) = round34_u128(n, -e);
    finish(false, n, s)
}

// ---------------------------------------------------------------------------
// Powers
// ---------------------------------------------------------------------------

/// Approximate `log10(|x|)` for nonzero `x`, accurate near 1.
fn log10_estimate(x: &Decimal) -> f64 {
    let adj = digits_u128(x.coeff) as i64 - 1 - x.scale as i64;
    if adj == 0 || adj == -1 {
        // |x| in [0.1, 10): use log1p of the exactly computed |x| - 1.
        if let Some(t) = x.abs().sub(&Decimal::ONE) {
            return t.to_f64().ln_1p() / core::f64::consts::LN_10;
        }
    }
    (x.coeff as f64).log10() - x.scale as f64
}

/// Saturating conversion of an exponent to `i64` (values this large are far
/// outside the valid range, so saturation does not change any result).
fn sat(v: i128) -> i64 {
    v.clamp(i64::MIN as i128 / 4, i64::MAX as i128 / 4) as i64
}

/// Exact `cp^n` is computed when it has at most this many digits.
const EXACT_POW_DIGITS: f64 = 300.0;

pub(crate) fn pow_i64(x: &Decimal, n: i64) -> Option<Decimal> {
    if n == 0 {
        return Some(Decimal::ONE);
    }
    let s = x.scale as i64;
    if x.coeff == 0 {
        if n < 0 {
            return None;
        }
        return finish(false, 0, s.saturating_mul(n));
    }
    if n == 1 {
        return Some(*x);
    }
    let neg = x.neg && n % 2 != 0;
    // Quick overflow / underflow detection.
    let est = log10_estimate(x) * n as f64;
    if !(-6178.0..=6146.0).contains(&est) {
        return None;
    }
    let an = n.unsigned_abs();
    let (cp, t) = strip_zeros(x.coeff, u64::MAX);
    let digits_est = an as f64 * (cp as f64).log10();
    if cp == 1 || digits_est <= EXACT_POW_DIGITS {
        // Exponent arithmetic in i128: `t * |n|` overflows i64 for bases
        // like 1.0 and |n| near 2^63.
        let (t, an_i, s) = (t as i128, an as i128, s as i128);
        let pw = if cp == 1 {
            BigInt::one()
        } else {
            BigInt::from_u128(cp).pow(an)
        };
        let dp = pw.dec_len() as i128;
        let p = P as i128;
        if n > 0 {
            // Exact coefficient c^n = pw * 10^(t*n) at scale s*n.
            let total = dp + t * an_i;
            let sn = s * an_i;
            if total <= p {
                let c = pw.abs_u128().expect("small") * POW10[(t * an_i) as usize];
                return finish(neg, c, sat(sn));
            }
            let k = total - p;
            if dp <= p {
                let c = pw.abs_u128().expect("small") * POW10[(p - dp) as usize];
                return finish(neg, c, sat(sn - k));
            }
            // Round pw itself (the dropped digits include all t*n zeros).
            return round_big(neg, &pw, sat(sn - t * an_i));
        }
        // 1 / x^|n| with division semantics (CPython's algorithm with the
        // 35/36-digit quotient and a sticky bit).
        let (q, r) = BigInt::pow10((dp + p) as u64).divrem_abs(&pw);
        let mut q = q.abs_u128().expect("35-digit quotient");
        let ideal = -s * an_i;
        let mut scale = dp + t * an_i + p - s * an_i;
        if r.is_zero() {
            if scale > ideal {
                let max = u64::try_from(scale - ideal).unwrap_or(u64::MAX);
                let (c, removed) = strip_zeros(q, max);
                q = c;
                scale -= removed as i128;
            }
        } else if q % 5 == 0 {
            q += 1;
        }
        let (c, sc) = round34_u128(q, sat(scale));
        return finish(neg, c, sc);
    }
    // Huge exact power: the result is certainly inexact (cp is not a power
    // of ten and cp^|n| has hundreds of digits), so compute
    // exp(n * ln|x|) with the correctly rounded Ziv loop.
    let xc = BigInt::from_u128(x.coeff);
    let yc = BigInt::from_i64(n);
    let mut extra = 3;
    let (coeff, exp) = loop {
        let (coeff, exp) = dpower(&xc, -s, &yc, 0, P + extra);
        if !needs_more_precision(&coeff) || extra >= MAX_EXTRA {
            break (coeff, exp);
        }
        extra += 3;
    };
    round_big(neg, &coeff, -exp)
}

pub(crate) fn pow(x: &Decimal, y: &Decimal) -> Option<Decimal> {
    if y.is_integer() {
        if let Some(n) = y.to_i64() {
            if n.unsigned_abs() <= 999_999_999 {
                return pow_i64(x, n);
            }
        }
    }
    if x.coeff == 0 {
        return if y.is_positive() {
            Some(Decimal::ZERO)
        } else {
            None
        };
    }
    if x.neg {
        return None;
    }
    if *x == Decimal::ONE {
        return Some(Decimal::ONE);
    }
    // Quick overflow / underflow detection (y * log10(x) out of range).
    let est = log10_estimate(x) * y.to_f64();
    if est.is_nan() || !(-6178.0..=6146.0).contains(&est) {
        return None;
    }
    if let Some((c, e)) = power_exact(x, y) {
        // Exact result (at most 35 digits).
        if c >= COEFF_LIMIT {
            let (c, s) = round34_u128(c, -e);
            return finish(false, c, s);
        }
        // Normalize: integers get scale 0 when that fits, fractions keep
        // only significant digits.
        let mut c = c;
        let mut scale = -e;
        if scale < 0 && digits_u128(c) as i64 - scale <= P {
            c *= POW10[(-scale) as usize];
            scale = 0;
        }
        return finish(false, c, scale);
    }
    let xc = BigInt::from_u128(x.coeff);
    let yc = BigInt::from_i128(y.coefficient());
    let (xe, ye) = (-(x.scale as i64), -(y.scale as i64));
    let mut extra = 3;
    let (coeff, exp) = loop {
        let (coeff, exp) = dpower(&xc, xe, &yc, ye, P + extra);
        if !needs_more_precision(&coeff) || extra >= MAX_EXTRA {
            break (coeff, exp);
        }
        extra += 3;
    };
    round_big(false, &coeff, -exp)
}

/// `n * 10^e` if that is an integer (`_decimal_lshift_exact`), with
/// overflow mapped to `None`.
fn decimal_lshift_exact(n: i128, e: i64) -> Option<i128> {
    if n == 0 {
        return Some(0);
    }
    if e >= 0 {
        if e > 38 {
            return None;
        }
        return n.checked_mul(POW10[e as usize] as i128);
    }
    let (_, tz) = strip_zeros(n.unsigned_abs(), u64::MAX);
    if (tz as i64) < -e {
        None
    } else {
        Some(n / POW10[(-e) as usize] as i128)
    }
}

/// `100 * log10(c)` lower bound (`_log10_lb`).
fn log10_lb(c: u128) -> i64 {
    let s = c.to_string();
    let corr = match s.as_bytes()[0] {
        b'1' => 100,
        b'2' => 70,
        b'3' => 53,
        b'4' => 40,
        b'5' => 31,
        b'6' => 23,
        b'7' => 16,
        b'8' => 10,
        _ => 5,
    };
    100 * s.len() as i64 - corr
}

fn bit_len(v: u128) -> u32 {
    128 - v.leading_zeros()
}

/// Exact value of `x^y` if it is representable with at most 35 digits, as
/// `(coefficient, exponent)` meaning `c * 10^e` (`_power_exact` with
/// `p = 35`, without the ideal-exponent padding: the caller normalizes).
/// `x > 0`, `x != 1`, `y` nonzero; `y` is non-integral or huge.
fn power_exact(x: &Decimal, y: &Decimal) -> Option<(u128, i64)> {
    const PX: i64 = 35;
    let (mut xc, xt) = strip_zeros(x.coeff, u64::MAX);
    let mut xe = -(x.scale as i64) + xt as i64;
    let (yc, yt) = strip_zeros(y.coeff, u64::MAX);
    let mut ye = -(y.scale as i64) + yt as i64;
    let y_neg = y.neg;

    if xc == 1 {
        // Result is 10^(xe * y), which must be an integer power.
        let mut xey = (xe as i128).checked_mul(yc as i128)?;
        while xey % 10 == 0 {
            xey /= 10;
            ye += 1;
        }
        if ye < 0 {
            return None;
        }
        let exponent = decimal_lshift_exact(xey, ye)?;
        let exponent = if y_neg { -exponent } else { exponent };
        let exponent = i64::try_from(exponent).ok()?;
        return Some((1, exponent));
    }

    if y_neg {
        let last = xc % 10;
        let (e, xc_new) = if last % 2 == 0 {
            // xc must be a power of two.
            if xc & (xc - 1) != 0 {
                return None;
            }
            let e = bit_len(xc) as i128 - 1;
            let emax = PX * 93 / 65;
            if ye >= len_i128(emax as i128) {
                return None;
            }
            let e = decimal_lshift_exact(e * yc as i128, ye)?;
            let xe2 = decimal_lshift_exact(xe as i128 * yc as i128, ye)?;
            if e > emax as i128 {
                return None;
            }
            xe = i64::try_from(xe2).ok()?;
            (e, 5u128.pow(e as u32))
        } else if last == 5 {
            // xc must be a power of five.
            let mut e = bit_len(xc) as i128 * 28 / 65;
            let p5 = 5u128.checked_pow(e as u32)?;
            if p5 % xc != 0 {
                return None;
            }
            let mut q = p5 / xc;
            while q % 5 == 0 {
                q /= 5;
                e -= 1;
            }
            let emax = PX * 10 / 3;
            if ye >= len_i128(emax as i128) {
                return None;
            }
            let e = decimal_lshift_exact(e * yc as i128, ye)?;
            let xe2 = decimal_lshift_exact(xe as i128 * yc as i128, ye)?;
            if e > emax as i128 {
                return None;
            }
            xe = i64::try_from(xe2).ok()?;
            (e, 2u128.pow(e as u32))
        } else {
            return None;
        };
        if digits_u128(xc_new) as i64 > PX {
            return None;
        }
        return Some((xc_new, -(e as i64) - xe));
    }

    // y positive: y = m / n in lowest terms.
    let xc_bits = bit_len(xc) as i128;
    let (m, n): (u128, u128) = if ye >= 0 {
        if ye > 38 {
            return None;
        }
        (yc.checked_mul(POW10[ye as usize])?, 1)
    } else {
        if xe != 0 && len_i128(yc as i128 * xe as i128) <= -ye {
            return None;
        }
        if len_i128(yc as i128 * xc_bits) <= -ye {
            return None;
        }
        // Here -ye <= 37.
        let (mut m, mut n) = (yc, POW10[(-ye) as usize]);
        while m % 2 == 0 && n % 2 == 0 {
            m /= 2;
            n /= 2;
        }
        while m % 5 == 0 && n % 5 == 0 {
            m /= 5;
            n /= 5;
        }
        (m, n)
    };

    if n > 1 {
        if xc_bits <= n as i128 {
            return None;
        }
        let n_i = n as i64;
        if xe.rem_euclid(n_i) != 0 {
            return None;
        }
        xe = xe.div_euclid(n_i);
        // n-th root of xc by Newton's method.
        let n32 = n as u32;
        let mut a: u128 = 1u128 << ((xc_bits + n as i128 - 1) / n as i128);
        let (q, r) = loop {
            let (q, r) = match a.checked_pow(n32 - 1) {
                Some(pw) => (xc / pw, xc % pw),
                None => (0, xc),
            };
            if a <= q {
                break (q, r);
            }
            a = (a * (n - 1) + q) / n;
        };
        if !(a == q && r == 0) {
            return None;
        }
        xc = a;
    }

    if xc > 1 && m as i128 > (PX * 100 / log10_lb(xc)) as i128 {
        return None;
    }
    let xc = xc.checked_pow(u32::try_from(m).ok()?)?;
    let xe = xe.checked_mul(i64::try_from(m).ok()?)?;
    if digits_u128(xc) as i64 > PX {
        return None;
    }
    Some((xc, xe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ln10_constant_is_consistent() {
        // The constant must match the series computation.
        let p = 60;
        let mp = p + 5;
        let m = BigInt::pow10(mp as u64);
        let v = div_nearest(&ilog(&m.mul_u64(10), mp), &big(100));
        let s = v.to_dec_string();
        assert_eq!(&s[..p as usize], &LN10_DIGITS[..p as usize]);
    }

    #[test]
    fn computed_log10_digits_beyond_table() {
        let p = LN10_DIGITS.len() as i64 + 5;
        let d = log10_digits(p).to_dec_string();
        assert_eq!(&d[..LN10_DIGITS.len()], LN10_DIGITS);
    }
}
