//! A small signed arbitrary-precision integer, used only by the
//! transcendental functions (`exp`, `ln`, `log10`, non-integer `pow`) and by
//! integer powers whose exact value exceeds 256 bits. It is deliberately
//! simple (schoolbook multiplication, Knuth division) because operands rarely
//! exceed a few hundred digits.
//!
//! Semantics of division and shifts follow Python's integers (floor
//! division, arithmetic right shift), because the algorithms in
//! [`crate::math`] are ports of CPython's `_pydecimal` helpers.

use core::cmp::Ordering;

use crate::{digits_u128, POW10};

/// Signed big integer: sign + little-endian base-2^64 magnitude without
/// trailing zero limbs. Zero is always non-negative with an empty magnitude.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BigInt {
    neg: bool,
    mag: Vec<u64>,
}

const TEN19: u64 = 10_000_000_000_000_000_000;

fn trim(v: &mut Vec<u64>) {
    while let Some(&0) = v.last() {
        v.pop();
    }
}

fn cmp_mag(a: &[u64], b: &[u64]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u64], b: &[u64]) -> Vec<u64> {
    let (a, b) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(a.len() + 1);
    let mut carry = 0u64;
    for (i, &x) in a.iter().enumerate() {
        let y = if i < b.len() { b[i] } else { 0 };
        let (s1, c1) = x.overflowing_add(y);
        let (s2, c2) = s1.overflowing_add(carry);
        out.push(s2);
        carry = c1 as u64 + c2 as u64;
    }
    if carry != 0 {
        out.push(carry);
    }
    out
}

/// `a - b`, requires `a >= b`.
fn sub_mag(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0u64;
    for (i, &x) in a.iter().enumerate() {
        let y = if i < b.len() { b[i] } else { 0 };
        let (d1, b1) = x.overflowing_sub(y);
        let (d2, b2) = d1.overflowing_sub(borrow);
        out.push(d2);
        borrow = b1 as u64 + b2 as u64;
    }
    debug_assert_eq!(borrow, 0);
    trim(&mut out);
    out
}

fn mul_mag(a: &[u64], b: &[u64]) -> Vec<u64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u64; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry: u128 = 0;
        for (j, &y) in b.iter().enumerate() {
            let t = x as u128 * y as u128 + out[i + j] as u128 + carry;
            out[i + j] = t as u64;
            carry = t >> 64;
        }
        out[i + b.len()] = carry as u64;
    }
    trim(&mut out);
    out
}

fn mul_small_mag(a: &[u64], m: u64) -> Vec<u64> {
    if m == 0 || a.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(a.len() + 1);
    let mut carry: u128 = 0;
    for &x in a {
        let t = x as u128 * m as u128 + carry;
        out.push(t as u64);
        carry = t >> 64;
    }
    if carry != 0 {
        out.push(carry as u64);
    }
    out
}

fn divrem_small_mag(a: &[u64], d: u64) -> (Vec<u64>, u64) {
    debug_assert!(d != 0);
    let mut q = vec![0u64; a.len()];
    let mut r: u64 = 0;
    for i in (0..a.len()).rev() {
        let cur = ((r as u128) << 64) | a[i] as u128;
        q[i] = (cur / d as u128) as u64;
        r = (cur % d as u128) as u64;
    }
    trim(&mut q);
    (q, r)
}

fn shl_mag(a: &[u64], n: u64) -> Vec<u64> {
    if a.is_empty() {
        return Vec::new();
    }
    let limbs = (n / 64) as usize;
    let bits = (n % 64) as u32;
    let mut out = vec![0u64; limbs];
    out.reserve(a.len() + 1);
    if bits == 0 {
        out.extend_from_slice(a);
    } else {
        let mut carry = 0u64;
        for &x in a {
            out.push((x << bits) | carry);
            carry = x >> (64 - bits);
        }
        if carry != 0 {
            out.push(carry);
        }
    }
    out
}

/// Logical right shift of a magnitude; also reports whether nonzero bits
/// were shifted out.
fn shr_mag(a: &[u64], n: u64) -> (Vec<u64>, bool) {
    let limbs = (n / 64) as usize;
    let bits = (n % 64) as u32;
    if limbs >= a.len() {
        return (Vec::new(), !a.is_empty());
    }
    let mut lost = a[..limbs].iter().any(|&x| x != 0);
    let src = &a[limbs..];
    let mut out = Vec::with_capacity(src.len());
    if bits == 0 {
        out.extend_from_slice(src);
    } else {
        lost |= src[0] & ((1u64 << bits) - 1) != 0;
        for i in 0..src.len() {
            let hi = if i + 1 < src.len() {
                src[i + 1] << (64 - bits)
            } else {
                0
            };
            out.push((src[i] >> bits) | hi);
        }
    }
    trim(&mut out);
    (out, lost)
}

/// Knuth algorithm D: `(a / b, a % b)` for magnitudes, `b` nonzero.
fn divrem_mag(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    assert!(!b.is_empty(), "division by zero");
    if cmp_mag(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let (q, r) = divrem_small_mag(a, b[0]);
        let r = if r == 0 { Vec::new() } else { vec![r] };
        return (q, r);
    }
    const BASE: u128 = 1 << 64;
    let s = b[b.len() - 1].leading_zeros() as u64;
    let vn = shl_mag(b, s);
    debug_assert_eq!(vn.len(), b.len());
    let mut un = shl_mag(a, s);
    un.resize(a.len() + 1, 0);
    let n = vn.len();
    let m = a.len() - n;
    let mut q = vec![0u64; m + 1];
    let vtop = vn[n - 1] as u128;
    let vsec = vn[n - 2] as u128;
    for j in (0..=m).rev() {
        let num = ((un[j + n] as u128) << 64) | un[j + n - 1] as u128;
        let mut qhat = num / vtop;
        let mut rhat = num % vtop;
        while qhat >= BASE || qhat * vsec > ((rhat << 64) | un[j + n - 2] as u128) {
            qhat -= 1;
            rhat += vtop;
            if rhat >= BASE {
                break;
            }
        }
        // Multiply and subtract.
        let mut borrow: u64 = 0;
        let mut carry: u128 = 0;
        for i in 0..n {
            let p = qhat * vn[i] as u128 + carry;
            carry = p >> 64;
            let (d1, b1) = un[i + j].overflowing_sub(p as u64);
            let (d2, b2) = d1.overflowing_sub(borrow);
            un[i + j] = d2;
            borrow = b1 as u64 + b2 as u64;
        }
        let top = un[j + n] as i128 - carry as i128 - borrow as i128;
        un[j + n] = top as u64;
        if top < 0 {
            // Estimate was one too large: add back.
            qhat -= 1;
            let mut c: u128 = 0;
            for i in 0..n {
                let t = un[i + j] as u128 + vn[i] as u128 + c;
                un[i + j] = t as u64;
                c = t >> 64;
            }
            un[j + n] = un[j + n].wrapping_add(c as u64);
        }
        q[j] = qhat as u64;
    }
    trim(&mut q);
    un.truncate(n);
    trim(&mut un);
    let (r, _) = shr_mag(&un, s);
    (q, r)
}

impl BigInt {
    fn from_parts(neg: bool, mut mag: Vec<u64>) -> BigInt {
        trim(&mut mag);
        let neg = neg && !mag.is_empty();
        BigInt { neg, mag }
    }

    pub fn zero() -> BigInt {
        BigInt {
            neg: false,
            mag: Vec::new(),
        }
    }

    pub fn one() -> BigInt {
        BigInt::from_u128(1)
    }

    pub fn from_u128(v: u128) -> BigInt {
        BigInt::from_parts(false, vec![v as u64, (v >> 64) as u64])
    }

    pub fn from_i128(v: i128) -> BigInt {
        let m = v.unsigned_abs();
        BigInt::from_parts(v < 0, vec![m as u64, (m >> 64) as u64])
    }

    pub fn from_i64(v: i64) -> BigInt {
        BigInt::from_i128(v as i128)
    }

    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    pub fn is_neg(&self) -> bool {
        self.neg
    }

    pub fn is_odd(&self) -> bool {
        self.mag.first().is_some_and(|&x| x & 1 == 1)
    }

    pub fn abs(&self) -> BigInt {
        BigInt {
            neg: false,
            mag: self.mag.clone(),
        }
    }

    pub fn negate(&self) -> BigInt {
        BigInt::from_parts(!self.neg, self.mag.clone())
    }

    /// The magnitude as a `u128`, if it fits.
    pub fn abs_u128(&self) -> Option<u128> {
        match self.mag.len() {
            0 => Some(0),
            1 => Some(self.mag[0] as u128),
            2 => Some(((self.mag[1] as u128) << 64) | self.mag[0] as u128),
            _ => None,
        }
    }

    pub fn to_i64(&self) -> Option<i64> {
        let m = self.abs_u128()?;
        if self.neg {
            if m <= 1u128 << 63 {
                Some((m as i128).wrapping_neg() as i64)
            } else {
                None
            }
        } else {
            i64::try_from(m).ok()
        }
    }

    pub fn bit_len(&self) -> u64 {
        match self.mag.last() {
            None => 0,
            Some(&top) => (self.mag.len() as u64 - 1) * 64 + (64 - top.leading_zeros() as u64),
        }
    }

    pub fn add(&self, o: &BigInt) -> BigInt {
        if self.neg == o.neg {
            return BigInt::from_parts(self.neg, add_mag(&self.mag, &o.mag));
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Less => BigInt::from_parts(o.neg, sub_mag(&o.mag, &self.mag)),
            _ => BigInt::from_parts(self.neg, sub_mag(&self.mag, &o.mag)),
        }
    }

    pub fn sub(&self, o: &BigInt) -> BigInt {
        if self.neg != o.neg {
            return BigInt::from_parts(self.neg, add_mag(&self.mag, &o.mag));
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Less => BigInt::from_parts(!self.neg, sub_mag(&o.mag, &self.mag)),
            _ => BigInt::from_parts(self.neg, sub_mag(&self.mag, &o.mag)),
        }
    }

    pub fn add_i64(&self, v: i64) -> BigInt {
        self.add(&BigInt::from_i64(v))
    }

    pub fn mul(&self, o: &BigInt) -> BigInt {
        BigInt::from_parts(self.neg != o.neg, mul_mag(&self.mag, &o.mag))
    }

    pub fn mul_u64(&self, m: u64) -> BigInt {
        BigInt::from_parts(self.neg, mul_small_mag(&self.mag, m))
    }

    pub fn shl(&self, n: u64) -> BigInt {
        BigInt::from_parts(self.neg, shl_mag(&self.mag, n))
    }

    /// Arithmetic (floor) right shift, like Python's `>>`.
    pub fn shr_floor(&self, n: u64) -> BigInt {
        let (m, lost) = shr_mag(&self.mag, n);
        if self.neg && lost {
            BigInt::from_parts(true, add_mag(&m, &[1]))
        } else {
            BigInt::from_parts(self.neg, m)
        }
    }

    /// Floor division and modulo, like Python's `divmod` (remainder has the
    /// sign of the divisor).
    pub fn divmod_floor(&self, d: &BigInt) -> (BigInt, BigInt) {
        let (q, r) = divrem_mag(&self.mag, &d.mag);
        let q = BigInt::from_parts(self.neg != d.neg, q);
        let r = BigInt::from_parts(self.neg, r);
        if !r.is_zero() && r.neg != d.neg {
            (q.add_i64(-1), r.add(d))
        } else {
            (q, r)
        }
    }

    /// Truncating division of magnitudes by a power of ten, returning the
    /// quotient magnitude and remainder magnitude (both non-negative).
    pub fn divrem_abs(&self, d: &BigInt) -> (BigInt, BigInt) {
        let (q, r) = divrem_mag(&self.mag, &d.mag);
        (BigInt::from_parts(false, q), BigInt::from_parts(false, r))
    }

    /// `true` if `self` is an exact multiple of `d` (`d` nonzero).
    pub fn is_multiple_of(&self, d: &BigInt) -> bool {
        divrem_mag(&self.mag, &d.mag).1.is_empty()
    }

    pub fn pow(&self, mut e: u64) -> BigInt {
        let mut result = BigInt::one();
        let mut base = self.clone();
        while e > 0 {
            if e & 1 == 1 {
                result = result.mul(&base);
            }
            e >>= 1;
            if e > 0 {
                base = base.mul(&base);
            }
        }
        result
    }

    /// `10^n`.
    pub fn pow10(n: u64) -> BigInt {
        if n <= 38 {
            return BigInt::from_u128(POW10[n as usize]);
        }
        let mut mag = vec![1u64];
        let mut k = n;
        while k >= 19 {
            mag = mul_small_mag(&mag, TEN19);
            k -= 19;
        }
        if k > 0 {
            mag = mul_small_mag(&mag, POW10[k as usize] as u64);
        }
        BigInt::from_parts(false, mag)
    }

    /// Number of decimal digits of `|self|` (1 for zero), i.e. Python's
    /// `len(str(abs(x)))`.
    pub fn dec_len(&self) -> u64 {
        if let Some(v) = self.abs_u128() {
            return digits_u128(v) as u64;
        }
        let mut m = self.mag.clone();
        let mut n = 0u64;
        while m.len() > 2 {
            m = divrem_small_mag(&m, TEN19).0;
            n += 19;
        }
        let v = BigInt::from_parts(false, m).abs_u128().unwrap_or(0);
        if v == 0 {
            // Cannot happen for a value with more than two limbs, but be safe.
            n
        } else {
            n + digits_u128(v) as u64
        }
    }

    /// Parses a string of ASCII decimal digits (no sign).
    pub fn from_dec_str(s: &str) -> BigInt {
        let bytes = s.as_bytes();
        let mut mag: Vec<u64> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let end = (i + 19).min(bytes.len());
            let mut chunk = 0u64;
            for &b in &bytes[i..end] {
                chunk = chunk * 10 + (b - b'0') as u64;
            }
            mag = mul_small_mag(&mag, POW10[end - i] as u64);
            mag = add_mag(&mag, &[chunk]);
            trim(&mut mag);
            i = end;
        }
        BigInt::from_parts(false, mag)
    }

    /// Decimal digits of `|self|`.
    pub fn to_dec_string(&self) -> String {
        if self.mag.is_empty() {
            return "0".to_string();
        }
        let mut chunks = Vec::new();
        let mut m = self.mag.clone();
        while !m.is_empty() {
            let (q, r) = divrem_small_mag(&m, TEN19);
            chunks.push(r);
            m = q;
        }
        let mut s = String::new();
        for (i, c) in chunks.iter().rev().enumerate() {
            if i == 0 {
                s.push_str(&c.to_string());
            } else {
                s.push_str(&format!("{c:019}"));
            }
        }
        s
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.neg, other.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &other.mag),
            (true, true) => cmp_mag(&other.mag, &self.mag),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn big(&mut self, limbs: usize) -> BigInt {
            let mut v: Vec<u64> = (0..limbs).map(|_| self.next()).collect();
            // Occasionally create long runs of ones/zeros to stress carries.
            if self.next().is_multiple_of(4) {
                for x in v.iter_mut() {
                    if self.next().is_multiple_of(2) {
                        *x = u64::MAX;
                    }
                }
            }
            let neg = self.next().is_multiple_of(3);
            BigInt::from_parts(neg, v)
        }
    }

    #[test]
    fn small_values_match_i128() {
        let vals: [i128; 9] = [
            0,
            1,
            -1,
            7,
            -7,
            1 << 70,
            -(1 << 70),
            i64::MAX as i128,
            -12345678901234567890,
        ];
        for &a in &vals {
            for &b in &vals {
                let (ba, bb) = (BigInt::from_i128(a), BigInt::from_i128(b));
                assert_eq!(ba.add(&bb), BigInt::from_i128(a + b));
                assert_eq!(ba.sub(&bb), BigInt::from_i128(a - b));
                if b != 0 {
                    let (q, r) = ba.divmod_floor(&bb);
                    // Python floor division.
                    let mut fq = a / b;
                    if a % b != 0 && ((a < 0) != (b < 0)) {
                        fq -= 1;
                    }
                    assert_eq!(q, BigInt::from_i128(fq));
                    assert_eq!(r, BigInt::from_i128(a - b * fq));
                }
                assert_eq!(ba.cmp(&bb), a.cmp(&b));
            }
            for s in 0..80u64 {
                assert_eq!(
                    BigInt::from_i128(a).shr_floor(s),
                    BigInt::from_i128(a >> s.min(127))
                );
            }
        }
    }

    #[test]
    fn division_identity_random() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        for i in 0..3000 {
            let a = rng.big(1 + (i % 9));
            let nb = 1 + (rng.next() % 6) as usize;
            let mut b = rng.big(nb);
            if b.is_zero() {
                b = BigInt::one();
            }
            let (q, r) = a.divmod_floor(&b);
            // a == q*b + r, 0 <= |r| < |b|, sign(r) == sign(b) or r == 0
            assert_eq!(q.mul(&b).add(&r), a);
            assert_eq!(cmp_mag(&r.mag, &b.mag), Ordering::Less);
            assert!(r.is_zero() || r.neg == b.neg);
        }
    }

    #[test]
    fn decimal_strings() {
        let s = "123456789012345678901234567890123456789012345678901234567890";
        let b = BigInt::from_dec_str(s);
        assert_eq!(b.to_dec_string(), s);
        assert_eq!(b.dec_len(), s.len() as u64);
        for n in 0..120u64 {
            let p = BigInt::pow10(n);
            assert_eq!(p.dec_len(), n + 1);
            assert_eq!(p.sub(&BigInt::one()).dec_len(), n.max(1));
            assert_eq!(BigInt::from_u128(10).pow(n), p);
        }
    }
}
