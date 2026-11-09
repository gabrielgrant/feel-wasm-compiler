//! A minimal, allocation-free 256-bit unsigned integer used for exact
//! intermediate results of `+ - * /` (products of two 34-digit coefficients,
//! aligned sums and scaled dividends all fit comfortably in 256 bits).

use crate::POW10;

const M64: u128 = u64::MAX as u128;

/// 256-bit unsigned integer, `hi * 2^128 + lo`.
///
/// The derived ordering compares `hi` first, then `lo`, which is the numeric
/// ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct U256 {
    pub hi: u128,
    pub lo: u128,
}

impl U256 {
    #[inline]
    pub const fn from_u128(v: u128) -> U256 {
        U256 { hi: 0, lo: v }
    }

    /// Full 128 x 128 -> 256-bit product.
    #[inline]
    pub fn mul_u128(a: u128, b: u128) -> U256 {
        if (a | b) >> 64 == 0 {
            return U256 { hi: 0, lo: a * b };
        }
        let (a1, a0) = (a >> 64, a & M64);
        let (b1, b0) = (b >> 64, b & M64);
        let p00 = a0 * b0;
        let p01 = a0 * b1;
        let p10 = a1 * b0;
        let p11 = a1 * b1;
        let mid = (p00 >> 64) + (p01 & M64) + (p10 & M64);
        let lo = (p00 & M64) | (mid << 64);
        let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
        U256 { hi, lo }
    }

    /// `self * 10`, const-evaluable (used to build the power table).
    const fn mul10(self) -> U256 {
        let l0 = (self.lo & M64) * 10;
        let l1 = (self.lo >> 64) * 10 + (l0 >> 64);
        let lo = (l0 & M64) | ((l1 & M64) << 64);
        let hi = self.hi * 10 + (l1 >> 64);
        U256 { hi, lo }
    }

    #[inline]
    pub fn add_u128(self, v: u128) -> U256 {
        let (lo, c) = self.lo.overflowing_add(v);
        U256 {
            hi: self.hi + c as u128,
            lo,
        }
    }

    #[inline]
    pub fn sub_u128(self, v: u128) -> U256 {
        let (lo, b) = self.lo.overflowing_sub(v);
        U256 {
            hi: self.hi - b as u128,
            lo,
        }
    }

    /// Divides by a nonzero `u128`, returning `(quotient, remainder)`.
    #[inline]
    pub fn divrem_u128(self, d: u128) -> (U256, u128) {
        debug_assert!(d != 0);
        if self.hi == 0 {
            return (U256::from_u128(self.lo / d), self.lo % d);
        }
        let qh = self.hi / d;
        let rh = self.hi % d;
        let (ql, r) = div_2x128(rh, self.lo, d);
        (U256 { hi: qh, lo: ql }, r)
    }

    /// Number of decimal digits (1 for zero).
    pub fn digits(self) -> u32 {
        if self.hi == 0 {
            return crate::digits_u128(self.lo);
        }
        // self >= 2^128 > 10^38, so at least 39 digits.
        let mut d = 39u32;
        while d < 78 && self >= POW10_256[d as usize] {
            d += 1;
        }
        d
    }

    /// Approximate conversion to `f64`.
    pub fn to_f64(self) -> f64 {
        self.hi as f64 * 340_282_366_920_938_463_463_374_607_431_768_211_456.0 + self.lo as f64
    }
}

/// Powers of ten 10^0 ..= 10^77 as 256-bit integers.
pub(crate) const POW10_256: [U256; 78] = {
    let mut t = [U256 { hi: 0, lo: 1 }; 78];
    let mut i = 1;
    while i < 78 {
        t[i] = t[i - 1].mul10();
        i += 1;
    }
    t
};

/// Divides the 256-bit value `u1 * 2^128 + u0` by `v`, where `u1 < v`
/// (so the quotient fits in 128 bits). Returns `(quotient, remainder)`.
///
/// This is the classic two-digit long division ("divlu" from Hacker's
/// Delight) with base 2^64 digits.
pub(crate) fn div_2x128(u1: u128, u0: u128, v: u128) -> (u128, u128) {
    debug_assert!(u1 < v);
    if u1 == 0 {
        return (u0 / v, u0 % v);
    }
    const B: u128 = 1 << 64;
    let s = v.leading_zeros();
    let v = v << s;
    let vn1 = v >> 64;
    let vn0 = v & M64;
    let un32 = if s == 0 {
        u1
    } else {
        (u1 << s) | (u0 >> (128 - s))
    };
    let un10 = u0 << s;
    let un1 = un10 >> 64;
    let un0 = un10 & M64;

    let mut q1 = un32 / vn1;
    let mut rhat = un32 - q1 * vn1;
    while q1 >= B || q1 * vn0 > ((rhat << 64) | un1) {
        q1 -= 1;
        rhat += vn1;
        if rhat >= B {
            break;
        }
    }
    let un21 = (un32 << 64)
        .wrapping_add(un1)
        .wrapping_sub(q1.wrapping_mul(v));

    let mut q0 = un21 / vn1;
    let mut rhat = un21 - q0 * vn1;
    while q0 >= B || q0 * vn0 > ((rhat << 64) | un0) {
        q0 -= 1;
        rhat += vn1;
        if rhat >= B {
            break;
        }
    }
    let r = (un21 << 64)
        .wrapping_add(un0)
        .wrapping_sub(q0.wrapping_mul(v))
        >> s;
    ((q1 << 64) | q0, r)
}

/// `(a * b) mod m` for `m > 0`.
#[inline]
pub(crate) fn mul_mod(a: u128, b: u128, m: u128) -> u128 {
    U256::mul_u128(a, b).divrem_u128(m).1
}

/// `10^k mod m` for `m > 0`.
pub(crate) fn pow10_mod(mut k: u64, m: u128) -> u128 {
    let mut result = 1 % m;
    let mut base = 10 % m;
    while k > 0 {
        if k & 1 == 1 {
            result = mul_mod(result, base, m);
        }
        base = mul_mod(base, base, m);
        k >>= 1;
    }
    result
}

/// `c * 10^k` as a 256-bit value. Requires the result to fit (debug-checked
/// only in the sense that callers guarantee `digits(c) + k <= 77`).
#[inline]
pub(crate) fn mul_pow10(c: u128, k: u32) -> U256 {
    if k <= 38 {
        U256::mul_u128(c, POW10[k as usize])
    } else {
        U256::mul_u128(c * POW10[(k - 38) as usize], POW10[38])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple xorshift generator for deterministic pseudo-random tests.
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
        fn next_u128(&mut self) -> u128 {
            ((self.next() as u128) << 64) | self.next() as u128
        }
    }

    /// Reference: binary long division of a 256-bit value.
    fn ref_divrem(n: U256, d: u128) -> (U256, u128) {
        let mut q = U256 { hi: 0, lo: 0 };
        let mut r: u128 = 0;
        for i in (0..256).rev() {
            let bit = if i >= 128 {
                (n.hi >> (i - 128)) & 1
            } else {
                (n.lo >> i) & 1
            };
            let overflow = r >> 127;
            r = (r << 1) | bit;
            if overflow == 1 || r >= d {
                r = r.wrapping_sub(d);
                if i >= 128 {
                    q.hi |= 1 << (i - 128);
                } else {
                    q.lo |= 1 << i;
                }
            }
        }
        (q, r)
    }

    #[test]
    fn mul_and_divide_roundtrip() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for i in 0..20000 {
            let a = match i % 4 {
                0 => rng.next_u128(),
                1 => rng.next_u128() >> (rng.next() % 128),
                2 => rng.next() as u128,
                _ => u128::MAX - (rng.next() as u128),
            };
            let b = match i % 3 {
                0 => rng.next_u128() >> (rng.next() % 127),
                1 => (rng.next() as u128) | 1,
                _ => u128::MAX - (rng.next() as u128 % 1000),
            }
            .max(1);
            let p = U256::mul_u128(a, b);
            let (q, r) = p.divrem_u128(b);
            assert_eq!(q, U256::from_u128(a), "a={a} b={b}");
            assert_eq!(r, 0);
            let extra = rng.next_u128() % b;
            let p2 = p.add_u128(extra);
            if p2 >= p {
                let (q2, r2) = p2.divrem_u128(b);
                assert_eq!(q2, U256::from_u128(a));
                assert_eq!(r2, extra);
                assert_eq!(p2.divrem_u128(b), ref_divrem(p2, b));
            }
        }
    }

    #[test]
    fn digits_table() {
        for (k, &p) in POW10_256.iter().enumerate() {
            assert_eq!(p.digits(), k as u32 + 1);
            if k > 0 {
                let below = if p.lo == 0 {
                    U256 {
                        hi: p.hi - 1,
                        lo: u128::MAX,
                    }
                } else {
                    p.sub_u128(1)
                };
                assert_eq!(below.digits(), k as u32);
            }
        }
    }

    #[test]
    fn pow10_mod_matches_naive() {
        for m in [
            1u128,
            2,
            3,
            7,
            97,
            1_000_000_007,
            99_999_999_999_999_999_999_999_999_999_999u128,
        ] {
            let mut naive = 1 % m;
            for k in 0..200u64 {
                assert_eq!(pow10_mod(k, m), naive, "k={k} m={m}");
                naive = mul_mod(naive, 10, m);
            }
        }
    }
}
