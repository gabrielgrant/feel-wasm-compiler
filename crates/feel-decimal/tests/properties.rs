//! Randomized property tests: no panics (tests run with overflow checks),
//! representation invariants on every result, and algebraic identities that
//! hold exactly under the crate's rounding rules.

use std::cmp::Ordering;

use feel_decimal::{Decimal, RoundingMode, MAX_SCALE, MIN_SCALE};

/// Deterministic xorshift64* generator (no external crates).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    fn coeff(&mut self) -> u128 {
        let digits = match self.below(10) {
            0..=2 => 34,
            3 => self.range(25, 33) as u32,
            _ => self.range(1, 12) as u32,
        };
        let mut c: u128 = 0;
        for i in 0..digits {
            let d = if i == 0 {
                1 + self.below(9)
            } else {
                self.below(10)
            };
            c = c * 10 + d as u128;
        }
        match self.below(12) {
            0 => 10u128.pow(digits - 1),
            1 => 10u128.pow(digits) - 1,
            2 => 0,
            _ => c,
        }
    }

    fn scale(&mut self) -> i32 {
        (match self.below(10) {
            0..=4 => self.range(-5, 12),
            5..=6 => self.range(-40, 60),
            7 => self.range(MIN_SCALE as i64, MAX_SCALE as i64),
            8 => self.range(MAX_SCALE as i64 - 70, MAX_SCALE as i64),
            _ => self.range(MIN_SCALE as i64, MIN_SCALE as i64 + 70),
        }) as i32
    }

    fn dec(&mut self) -> Decimal {
        let neg = self.below(2) == 0;
        Decimal::from_parts(neg, self.coeff(), self.scale()).expect("in range")
    }

    fn moderate(&mut self) -> Decimal {
        let c = self.coeff();
        let digits = c.to_string().len() as i64;
        let scale = self.range(digits - 6, digits + 6) as i32;
        Decimal::from_parts(self.below(2) == 0, c, scale).expect("in range")
    }
}

fn check(d: &Option<Decimal>) {
    if let Some(v) = d {
        let (neg, c, s) = v.to_parts();
        assert!(c < 10u128.pow(34), "coefficient too large: {v:?}");
        assert!(
            (MIN_SCALE..=MAX_SCALE).contains(&s),
            "scale out of range: {v:?}"
        );
        assert!(!(neg && c == 0), "negative zero: {v:?}");
        // Round trip through both string forms.
        assert_eq!(
            Decimal::parse(&v.to_scientific_string())
                .unwrap()
                .to_parts(),
            v.to_parts()
        );
        let plain = Decimal::parse(&v.to_string()).unwrap();
        assert_eq!(plain, *v);
    }
}

const MODES: [RoundingMode; 7] = [
    RoundingMode::HalfEven,
    RoundingMode::HalfUp,
    RoundingMode::HalfDown,
    RoundingMode::Up,
    RoundingMode::Down,
    RoundingMode::Floor,
    RoundingMode::Ceiling,
];

#[test]
fn arithmetic_invariants() {
    let mut rng = Rng(0x0DD_BA11_CAFE_F00D);
    for i in 0..30_000 {
        let (a, b) = if i % 3 == 0 {
            (rng.moderate(), rng.moderate())
        } else {
            (rng.dec(), rng.dec())
        };
        let sum = a.add(&b);
        let diff = a.sub(&b);
        let prod = a.mul(&b);
        let quot = a.div(&b);
        for r in [&sum, &diff, &prod, &quot] {
            check(r);
        }
        // Commutativity and sign symmetry are exact under correct rounding.
        assert_eq!(sum.map(|x| x.to_parts()), b.add(&a).map(|x| x.to_parts()));
        assert_eq!(prod.map(|x| x.to_parts()), b.mul(&a).map(|x| x.to_parts()));
        assert_eq!(
            diff.map(|x| x.to_parts()),
            b.sub(&a).map(|x| x.neg().to_parts()),
            "{a:?} - {b:?}"
        );
        assert_eq!(
            sum.map(|x| x.to_parts()),
            a.sub(&b.neg()).map(|x| x.to_parts())
        );
        assert_eq!(
            quot.map(|x| x.to_parts()),
            a.neg().div(&b.neg()).map(|x| x.to_parts())
        );
        // Subtraction sign agrees with comparison when the result exists.
        if let Some(dv) = diff {
            match a.cmp(&b) {
                Ordering::Less => assert!(dv.is_negative()),
                Ordering::Equal => assert!(dv.is_zero()),
                Ordering::Greater => assert!(dv.is_positive()),
            }
        }
        // Remainder / modulo invariants.
        if !b.is_zero() {
            let r = a.rem(&b).expect("remainder exists");
            check(&Some(r));
            assert!(r.abs() < b.abs());
            assert!(r.is_zero() || r.is_negative() == a.is_negative());
            let m = a.modulo(&b).expect("modulo exists");
            check(&Some(m));
            assert!(m.is_zero() || m.is_negative() == b.is_negative() || m.abs() == b.abs());
        } else {
            assert!(quot.is_none() && a.rem(&b).is_none() && a.modulo(&b).is_none());
        }
        // Scale manipulation.
        let target = a.scale() as i64 + rng.range(-40, 40);
        for mode in MODES {
            let r = a.set_scale(target, mode);
            check(&r);
            if let Some(v) = r {
                assert_eq!(v.scale() as i64, target);
            }
            check(&a.round_to_scale(target, mode));
        }
        let n = a.normalize();
        assert_eq!(n, a);
        check(&Some(n));
        for v in [a.floor(), a.ceil(), a.trunc()] {
            check(&Some(v));
            assert!(v.is_integer());
        }
        assert!(a.floor() <= a && a <= a.ceil());
    }
}

#[test]
fn transcendental_no_panic() {
    let mut rng = Rng(0xFEE1_DEAD_BEEF_0001);
    for i in 0..1500 {
        let a = if i % 2 == 0 {
            rng.moderate()
        } else {
            rng.dec()
        };
        for r in [a.sqrt(), a.exp(), a.ln(), a.log10()] {
            check(&r);
        }
        let n = rng.range(-60, 60);
        check(&a.pow_i64(n));
        let y = Decimal::from_parts(
            rng.below(2) == 0,
            rng.coeff() % 1_000_000,
            rng.range(0, 6) as i32,
        )
        .unwrap();
        check(&a.abs().pow(&y));
        let y = rng.dec();
        check(&a.abs().pow(&y));
    }
}

#[test]
fn sqrt_squares_back() {
    let mut rng = Rng(42);
    for _ in 0..3000 {
        let a = rng.moderate().abs();
        if let Some(r) = a.sqrt() {
            // r is correctly rounded to 34 digits, so r^2 is within a
            // relative 1e-32 of a.
            let sq = r.mul(&r).unwrap();
            let tol = a.mul(&Decimal::parse("1E-32").unwrap()).unwrap();
            assert!(sq.sub(&a).unwrap().abs() <= tol, "sqrt({a}) = {r}");
        }
    }
}
