//! Unit tests for the public API: FEEL specification examples, rounding
//! modes, parsing, formatting, ordering/hashing and range boundaries.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use feel_decimal::{Decimal, RoundingMode, MAX_SCALE, MIN_SCALE};

fn d(s: &str) -> Decimal {
    Decimal::parse(s).unwrap_or_else(|| panic!("parse failed: {s}"))
}

fn plain(x: Option<Decimal>) -> String {
    match x {
        Some(v) => v.to_string(),
        None => "null".to_string(),
    }
}

fn hash_of(x: &Decimal) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// FEEL specification examples (Table 40, Table 76, ...)
// ---------------------------------------------------------------------------

#[test]
fn spec_table_40_examples() {
    let one = Decimal::ONE;
    // decimal(1, 2) = 1.00
    assert_eq!(plain(one.set_scale(2, RoundingMode::HalfEven)), "1.00");
    // .25 + .2 = 0.45
    assert_eq!(plain(d(".25").add(&d(".2"))), "0.45");
    // .10 * 30.00 = 3.0000
    assert_eq!(plain(d(".10").mul(&d("30.00"))), "3.0000");
    // 1 + 3/2*2 - 2**3 = -4.0
    let t = d("3").div(&d("2")).unwrap().mul(&d("2")).unwrap();
    let p = d("2").pow(&d("3")).unwrap();
    assert_eq!(plain(one.add(&t).unwrap().sub(&p)), "-4.0");
    // 1/3 = 0.3333333333333333333333333333333333
    assert_eq!(
        plain(d("1").div(&d("3"))),
        "0.3333333333333333333333333333333333"
    );
    // decimal(1/3, 2) = 0.33
    assert_eq!(
        plain(
            d("1")
                .div(&d("3"))
                .unwrap()
                .set_scale(2, RoundingMode::HalfEven)
        ),
        "0.33"
    );
    // 1 = 1.000
    assert_eq!(d("1"), d("1.000"));
    // 1.01/2 = 0.505
    assert_eq!(plain(d("1.01").div(&d("2"))), "0.505");
    // decimal(0.505, 2) = 0.50, decimal(0.515, 2) = 0.52
    assert_eq!(
        plain(d("0.505").set_scale(2, RoundingMode::HalfEven)),
        "0.50"
    );
    assert_eq!(
        plain(d("0.515").set_scale(2, RoundingMode::HalfEven)),
        "0.52"
    );
    // 1.0*10**3 = 1000.0
    assert_eq!(
        plain(d("1.0").mul(&d("10").pow(&d("3")).unwrap())),
        "1000.0"
    );
    // 10/4 = 2.5, 6.0/2 = 3.0, 1/2 = 0.5
    assert_eq!(plain(d("10").div(&d("4"))), "2.5");
    assert_eq!(plain(d("6.0").div(&d("2"))), "3.0");
    assert_eq!(plain(d("1").div(&d("2"))), "0.5");
    // 1.2e3 literal
    assert_eq!(d("1.2e3"), d("1200"));
    assert_eq!(d("1.2e3").scale(), -2);
    assert_eq!(d("1.2e3").to_string(), "1200");
}

#[test]
fn spec_rounding_function_examples() {
    use RoundingMode::*;
    let cases: &[(&str, i64, RoundingMode, &str)] = &[
        ("1.5", 0, Floor, "1"),
        ("-1.56", 1, Floor, "-1.6"),
        ("1.5", 0, Ceiling, "2"),
        ("-1.56", 1, Ceiling, "-1.5"),
        ("5.5", 0, Up, "6"),
        ("-5.5", 0, Up, "-6"),
        ("1.121", 2, Up, "1.13"),
        ("-1.126", 2, Up, "-1.13"),
        ("5.5", 0, Down, "5"),
        ("-5.5", 0, Down, "-5"),
        ("1.121", 2, Down, "1.12"),
        ("-1.126", 2, Down, "-1.12"),
        ("5.5", 0, HalfUp, "6"),
        ("-5.5", 0, HalfUp, "-6"),
        ("1.121", 2, HalfUp, "1.12"),
        ("-1.126", 2, HalfUp, "-1.13"),
        ("5.5", 0, HalfDown, "5"),
        ("-5.5", 0, HalfDown, "-5"),
        ("1.121", 2, HalfDown, "1.12"),
        ("-1.126", 2, HalfDown, "-1.13"),
        ("1.5", 0, HalfEven, "2"),
        ("2.5", 0, HalfEven, "2"),
        ("-2.5", 0, HalfEven, "-2"),
        ("-3.5", 0, HalfEven, "-4"),
    ];
    for &(n, s, mode, want) in cases {
        assert_eq!(plain(d(n).set_scale(s, mode)), want, "{n} {s} {mode:?}");
    }
    // decimal(1/3, 2) = .33
    let third = d("1").div(&d("3")).unwrap();
    assert_eq!(plain(third.set_scale(2, HalfEven)), "0.33");
    assert_eq!(d("1.5").floor().to_string(), "1");
    assert_eq!(d("1.5").ceil().to_string(), "2");
    assert_eq!(d("-1.5").floor().to_string(), "-2");
    assert_eq!(d("-1.5").ceil().to_string(), "-1");
    assert_eq!(d("-1.5").trunc().to_string(), "-1");
    assert_eq!(d("1E+3").floor().to_string(), "1000");
    assert_eq!(d("1E+3").floor().scale(), 0);
    // Integral values with too many digits for scale 0 keep their scale.
    assert_eq!(d("1E+40").floor(), d("1E+40"));
    assert_eq!(d("-0.4").ceil().to_string(), "0");
}

#[test]
fn every_rounding_mode_on_ties_and_non_ties() {
    use RoundingMode::*;
    // value, then expected for [HalfEven, HalfUp, HalfDown, Up, Down, Floor, Ceiling] at scale 0
    let table: &[(&str, [&str; 7])] = &[
        ("5.5", ["6", "6", "5", "6", "5", "5", "6"]),
        ("2.5", ["2", "3", "2", "3", "2", "2", "3"]),
        ("1.6", ["2", "2", "2", "2", "1", "1", "2"]),
        ("1.1", ["1", "1", "1", "2", "1", "1", "2"]),
        ("-1.0", ["-1", "-1", "-1", "-1", "-1", "-1", "-1"]),
        ("-1.1", ["-1", "-1", "-1", "-2", "-1", "-2", "-1"]),
        ("-1.6", ["-2", "-2", "-2", "-2", "-1", "-2", "-1"]),
        ("-2.5", ["-2", "-3", "-2", "-3", "-2", "-3", "-2"]),
        ("-5.5", ["-6", "-6", "-5", "-6", "-5", "-6", "-5"]),
        ("0.5", ["0", "1", "0", "1", "0", "0", "1"]),
        ("-0.5", ["0", "-1", "0", "-1", "0", "-1", "0"]),
        (
            "2.50000000000000000000000000000001",
            ["3", "3", "3", "3", "2", "2", "3"],
        ),
        ("0.0001", ["0", "0", "0", "1", "0", "0", "1"]),
        ("-0.0001", ["0", "0", "0", "-1", "0", "-1", "0"]),
    ];
    let modes = [HalfEven, HalfUp, HalfDown, Up, Down, Floor, Ceiling];
    for (v, expected) in table {
        for (mode, want) in modes.iter().zip(expected.iter()) {
            let got = d(v).set_scale(0, *mode).unwrap();
            assert_eq!(got.to_string(), *want, "{v} {mode:?}");
            assert_eq!(got.scale(), 0);
            assert!(!(got.is_zero() && got.is_negative()));
        }
    }
    // Rounding far below the coefficient (large scale reduction).
    let tiny = d("1E-6000");
    assert_eq!(plain(tiny.set_scale(0, Up)), "1");
    assert_eq!(plain(tiny.set_scale(0, Down)), "0");
    assert_eq!(plain(tiny.neg().set_scale(0, Floor)), "-1");
    assert_eq!(plain(tiny.set_scale(2, Ceiling)), "0.01");
    // Padding.
    assert_eq!(plain(d("1.5").set_scale(5, HalfEven)), "1.50000");
    assert_eq!(plain(d("-12").set_scale(-1, HalfEven)), "-10");
    assert_eq!(d("-12").set_scale(-1, HalfEven).unwrap().scale(), -1);
    // Too many digits / scale out of range.
    assert!(d("1").set_scale(34, HalfEven).is_none());
    assert_eq!(
        plain(d("1").set_scale(33, HalfEven)),
        format!("1.{}", "0".repeat(33))
    );
    assert!(d("1").set_scale(MAX_SCALE as i64 + 1, HalfEven).is_none());
    assert!(d("1").set_scale(MIN_SCALE as i64 - 1, HalfEven).is_none());
    assert_eq!(
        plain(
            d("0")
                .set_scale(MAX_SCALE as i64, HalfEven)
                .map(|x| Decimal::from_i64(x.scale() as i64))
        ),
        "6176"
    );
    assert_eq!(
        d("123456").set_scale(MIN_SCALE as i64, HalfEven).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn round_to_scale_does_not_pad() {
    use RoundingMode::*;
    assert_eq!(
        d("1.5").round_to_scale(3, HalfEven).unwrap().to_string(),
        "1.5"
    );
    assert_eq!(
        d("1.5678").round_to_scale(3, HalfEven).unwrap().to_string(),
        "1.568"
    );
    assert_eq!(d("1E+3").round_to_scale(0, HalfEven).unwrap().scale(), -3);
    assert_eq!(
        d("125").round_to_scale(-1, HalfEven).unwrap().to_string(),
        "120"
    );
    assert_eq!(
        d("135").round_to_scale(-1, HalfEven).unwrap().to_string(),
        "140"
    );
}

#[test]
fn modulo_examples() {
    let cases = [
        ("12", "5", "2"),
        ("-12", "5", "3"),
        ("12", "-5", "-3"),
        ("-12", "-5", "-2"),
        ("10.1", "4.5", "1.1"),
        ("-10.1", "4.5", "3.4"),
        ("10.1", "-4.5", "-3.4"),
        ("-10.1", "-4.5", "-1.1"),
        ("10", "2.5", "0.0"),
        ("1E+30", "7", "1"),
        ("1", "1E-20", "0E-20"),
    ];
    for (a, b, want) in cases {
        let got = d(a).modulo(&d(b)).unwrap();
        assert_eq!(got.to_scientific_string(), want, "modulo({a}, {b})");
    }
    assert!(d("1").modulo(&d("0")).is_none());
    assert!(d("0").modulo(&d("0.0")).is_none());
    // Truncated remainder keeps the dividend's sign.
    assert_eq!(d("-10.1").rem(&d("4.5")).unwrap().to_string(), "-1.1");
    assert_eq!(d("10.1").rem(&d("-4.5")).unwrap().to_string(), "1.1");
    // Huge quotients are still exact: 10^6000 mod 7 = 10^(6000 mod 6) mod 7 = 1.
    assert_eq!(d("1E+6000").rem(&d("7")).unwrap().to_string(), "1");
    assert_eq!(d("1E+6000").rem(&d("1E-6000")).unwrap(), Decimal::ZERO);
    assert_eq!(d("1E+6000").rem(&d("1E-6000")).unwrap().scale(), 6000);
    // Remainder smaller than the divisor needs no division.
    assert_eq!(
        d("1E-30").rem(&d("1E+30")).unwrap().to_scientific_string(),
        "1E-30"
    );
}

#[test]
fn numeric_function_examples() {
    assert_eq!(plain(d("16").sqrt()), "4");
    assert_eq!(plain(d("0.25").sqrt()), "0.5");
    assert_eq!(plain(d("1.00").sqrt()), "1.0");
    assert_eq!(plain(d("2").sqrt()), "1.414213562373095048801688724209698");
    assert_eq!(plain(d("0").sqrt()), "0");
    assert!(d("-1").sqrt().is_none());
    assert_eq!(plain(d("10").ln()), "2.302585092994045684017991454684364");
    assert_eq!(plain(d("1").ln()), "0");
    assert!(d("0").ln().is_none());
    assert!(d("-1").ln().is_none());
    assert_eq!(plain(d("1").exp()), "2.718281828459045235360287471352662");
    assert_eq!(plain(d("0").exp()), "1");
    assert_eq!(plain(d("4").exp()), "54.59815003314423907811026120286088");
    assert_eq!(plain(d("5").exp()), "148.4131591025766034211155800405523");
    assert_eq!(plain(d("100").log10()), "2");
    assert_eq!(plain(d("0.001").log10()), "-3");
    assert!(d("0").log10().is_none());
    // stddev( 2, 4, 7, 5 ) = 2.081665999466132735282297706979931
    let sum_sq = ["-2.5", "-0.5", "2.5", "0.5"]
        .iter()
        .map(|x| d(x).mul(&d(x)).unwrap())
        .fold(Decimal::ZERO, |a, b| a.add(&b).unwrap());
    let var = sum_sq.div(&d("3")).unwrap();
    assert_eq!(plain(var.sqrt()), "2.081665999466132735282297706979931");
}

#[test]
fn integer_powers() {
    let cases = [
        ("2", 3, "8"),
        ("1.1", 2, "1.21"),
        ("1.10", 2, "1.2100"),
        ("10", -2, "0.01"),
        ("2", -1, "0.5"),
        ("2", 0, "1"),
        ("0", 0, "1"),
        ("-2", 3, "-8"),
        ("-2", 2, "4"),
        ("-2", -1, "-0.5"),
        ("0.0", 2, "0.00"),
        ("3", -1, "0.3333333333333333333333333333333333"),
        ("10", 3, "1000"),
        ("1.0", 3, "1.000"),
        ("2", 100, "1267650600228229401496703205376"),
        (
            "2",
            200,
            "1606938044258990275541962092341163000000000000000000000000000",
        ),
    ];
    for (x, n, want) in cases {
        assert_eq!(plain(d(x).pow_i64(n)), want, "{x}^{n}");
    }
    assert_eq!(d("2").pow_i64(200).unwrap().precision(), 34);
    assert!(d("0").pow_i64(-1).is_none());
    // Overflow / underflow detected quickly for huge exponents.
    assert!(d("2").pow_i64(999_999_999).is_none());
    assert!(d("0.5").pow_i64(999_999_999).is_none());
    assert!(d("-2").pow_i64(-999_999_999).is_none());
    assert_eq!(plain(d("1").pow_i64(999_999_999)), "1");
    assert_eq!(plain(d("-1").pow_i64(999_999_999)), "-1");
    assert_eq!(plain(d("1.0").pow_i64(-999_999_999)), "1");
    assert_eq!(
        plain(d("1.0").pow_i64(999_999_999)),
        format!("1.{}", "0".repeat(33))
    );
    // (1 + 1e-33)^1e9 ~ 1 + 1e-24: computed through exp/ln, correctly rounded.
    assert_eq!(
        plain(d("1.000000000000000000000000000000001").pow_i64(999_999_999)),
        "1.000000000000000000000000999999999"
    );
    // Extreme i64 exponents.
    assert_eq!(
        plain(d("1.0").pow_i64(i64::MAX)),
        format!("1.{}", "0".repeat(33))
    );
    assert_eq!(plain(d("1.0").pow_i64(i64::MIN)), "1");
    assert_eq!(plain(d("-1.00").pow_i64(i64::MIN)), "1");
    assert_eq!(plain(d("-1").pow_i64(i64::MAX)), "-1");
    assert!(d("2").pow_i64(i64::MIN).is_none());
    assert!(d("0.5").pow_i64(i64::MAX).is_none());
    assert!(d("0").pow_i64(i64::MIN).is_none());
    assert_eq!(d("0.0").pow_i64(i64::MAX).unwrap().scale(), MAX_SCALE);
    assert_eq!(
        plain(d("1.000000000000000000000000000000001").pow_i64(i64::MAX)),
        "1.000000000000009223372036854818342"
    );
    assert_eq!(
        plain(d("1.000000000000000000000000000000001").pow_i64(i64::MIN)),
        "0.9999999999999907766279631452667273"
    );
    // Boundaries of the range.
    assert_eq!(d("10").pow_i64(6144).unwrap().scale(), MIN_SCALE);
    assert!(d("10").pow_i64(6145).is_none());
    assert_eq!(
        d("10").pow_i64(-6176).unwrap().to_scientific_string(),
        "1E-6176"
    );
    assert!(d("10").pow_i64(-6177).is_none());
}

#[test]
fn real_powers() {
    assert_eq!(plain(d("4").pow(&d("0.5"))), "2");
    assert_eq!(plain(d("0.25").pow(&d("0.5"))), "0.5");
    assert_eq!(plain(d("4").pow(&d("-0.5"))), "0.5");
    assert_eq!(plain(d("100").pow(&d("1.5"))), "1000");
    assert_eq!(
        plain(d("8").pow(&d("2.5"))),
        "181.0193359837561662466161566988414"
    );
    assert_eq!(
        plain(d("2").pow(&d("0.5"))),
        "1.414213562373095048801688724209698"
    );
    assert_eq!(plain(d("0").pow(&d("0.5"))), "0");
    assert!(d("0").pow(&d("-0.5")).is_none());
    assert!(d("-4").pow(&d("0.5")).is_none());
    assert_eq!(plain(d("1").pow(&d("0.5"))), "1");
    assert_eq!(plain(d("2").pow(&d("3"))), "8");
    assert_eq!(plain(d("2").pow(&d("-3"))), "0.125");
    assert!(d("10").pow(&d("1E+10")).is_none());
    assert!(d("10").pow(&d("-1E+10")).is_none());
    assert_eq!(
        plain(d("1.0000000000000001").pow(&d("1E+10"))),
        "1.000001000000500000166616708283342"
    );
    assert!(d("-1").pow(&d("1E+10")).is_none());
}

// ---------------------------------------------------------------------------
// Arithmetic details
// ---------------------------------------------------------------------------

#[test]
fn addition_scales_and_rounding() {
    assert_eq!(plain(d("1.10").add(&d("1"))), "2.10");
    assert_eq!(plain(d("1").add(&d("0.000"))), "1.000");
    assert_eq!(plain(d("0.000").add(&d("0"))), "0.000");
    assert_eq!(plain(d("1.5").sub(&d("1.5"))), "0.0");
    assert!(!d("1.5").sub(&d("1.5")).unwrap().is_negative());
    // 34 nines + 1 carries into 35 digits and is rounded back.
    let nines = "9".repeat(34);
    assert_eq!(
        d(&nines).add(&d("1")).unwrap().to_scientific_string(),
        "1.000000000000000000000000000000000E+34"
    );
    // Half-even at the 35th digit.
    assert_eq!(
        d("1000000000000000000000000000000000")
            .add(&d("0.5"))
            .unwrap()
            .to_string(),
        "1000000000000000000000000000000000"
    );
    assert_eq!(
        d("1000000000000000000000000000000001")
            .add(&d("0.5"))
            .unwrap()
            .to_string(),
        "1000000000000000000000000000000002"
    );
    // Absorbed tiny operand (huge scale difference).
    assert_eq!(
        d("1").add(&d("1E-6000")).unwrap().to_string(),
        format!("1.{}", "0".repeat(33))
    );
    assert_eq!(
        d("1").sub(&d("1E-6000")).unwrap().to_string(),
        format!("1.{}", "0".repeat(33))
    );
    assert_eq!(
        d("1").sub(&d("6E-34")).unwrap().to_string(),
        "0.9999999999999999999999999999999994"
    );
    assert_eq!(
        d("1E+6111")
            .add(&d("1E-6176"))
            .unwrap()
            .to_scientific_string(),
        "1.000000000000000000000000000000000E+6111"
    );
    // Overflow.
    let max = d(&format!("{}E+6111", "9".repeat(34)));
    assert!(max.add(&d("1E+6111")).is_none());
    assert_eq!(max.sub(&max).unwrap(), Decimal::ZERO);
}

#[test]
fn multiplication_and_division_details() {
    assert_eq!(plain(d("1.5").mul(&d("-2"))), "-3.0");
    assert_eq!(plain(d("-0").mul(&d("5"))), "0");
    assert_eq!(plain(d("1E-4000").mul(&d("1E-4000"))), "null");
    assert_eq!(
        d("1E-3000")
            .mul(&d("1E-3176"))
            .unwrap()
            .to_scientific_string(),
        "1E-6176"
    );
    assert_eq!(d("1E+3000").mul(&d("1E+3144")).unwrap().scale(), MIN_SCALE);
    assert!(d("1E+3000").mul(&d("1E+3145")).is_none());
    assert_eq!(d("0E-4000").mul(&d("0E-4000")).unwrap().scale(), MAX_SCALE);
    assert_eq!(
        plain(d("2").div(&d("3"))),
        "0.6666666666666666666666666666666667"
    );
    assert_eq!(
        plain(d("-1").div(&d("3"))),
        "-0.3333333333333333333333333333333333"
    );
    assert_eq!(
        plain(d("1").div(&d("7"))),
        "0.1428571428571428571428571428571429"
    );
    assert_eq!(plain(d("1").div(&d("8"))), "0.125");
    assert_eq!(plain(d("100").div(&d("1"))), "100");
    assert_eq!(
        d("1E+2").div(&d("1")).unwrap().to_scientific_string(),
        "1E+2"
    );
    assert_eq!(plain(d("0.00").div(&d("5"))), "0.00");
    assert!(d("1").div(&d("0")).is_none());
    assert!(d("0").div(&d("0")).is_none());
    assert!(d("1E-6176").div(&d("10")).is_none());
    assert_eq!(
        d("10E-6176").div(&d("10")).unwrap().to_scientific_string(),
        "1E-6176"
    );
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_valid_forms() {
    let cases = [
        ("0", 0, 0),
        ("-0", 0, 0),
        ("+1", 1, 0),
        ("1.10", 110, 2),
        ("1.", 1, 0),
        (".5", 5, 1),
        ("-.5", -5, 1),
        ("1e3", 1, -3),
        ("1E3", 1, -3),
        ("1.2e3", 12, -2),
        ("1.2e+3", 12, -2),
        ("1.2e-3", 12, 4),
        ("00012.3400", 123400, 4),
        ("0.000", 0, 3),
        ("0e5", 0, -5),
        ("1e-6176", 1, 6176),
        ("10e-6177", 1, 6176),
        ("1e6111", 1, -6111),
    ];
    for (s, c, sc) in cases {
        let v = d(s);
        assert_eq!((v.coefficient(), v.scale()), (c, sc), "{s}");
    }
    assert!(!d("-0").is_negative());
    assert!(!d("-0.00").is_negative());
    assert_eq!(d("1e6144").coefficient(), 10i128.pow(33));
    assert_eq!(d("1e6144").scale(), MIN_SCALE);
    // Rounded to 34 digits.
    let long = format!("1.{}5", "0".repeat(33));
    assert_eq!(d(&long).to_string(), format!("1.{}", "0".repeat(33)));
    let long = format!("1.{}15", "0".repeat(32));
    assert_eq!(d(&long).to_string(), format!("1.{}2", "0".repeat(32)));
    let long = format!("1.{}51", "0".repeat(33));
    assert_eq!(d(&long).to_string(), format!("1.{}1", "0".repeat(32)));
    let nines = "9".repeat(40);
    assert_eq!(
        d(&nines).to_scientific_string(),
        "1.000000000000000000000000000000000E+40"
    );
    let long_int = format!("1{}", "0".repeat(36));
    assert_eq!(d(&long_int).scale(), -3);
    // Thousands of digits.
    let huge = format!("0.{}1{}", "0".repeat(5000), "3".repeat(5000));
    let v = d(&huge);
    assert_eq!(v.precision(), 34);
    assert_eq!(v.scale(), 5034);
    let leading = format!("{}12.5", "0".repeat(10000));
    assert_eq!(d(&leading).to_string(), "12.5");
    let zeros = format!("0.{}", "0".repeat(10000));
    assert_eq!(d(&zeros).scale(), MAX_SCALE);
    // Huge exponents.
    assert_eq!(d("0e999999999999999999999999").scale(), MIN_SCALE);
    assert_eq!(d("0e-999999999999999999999999").scale(), MAX_SCALE);
    assert_eq!(
        d(&format!("1{}e-6200", "0".repeat(24))).to_scientific_string(),
        "1E-6176"
    );
    assert!("1.5".parse::<Decimal>().is_ok());
    assert!("x".parse::<Decimal>().is_err());
}

#[test]
fn parse_invalid_forms() {
    let bad = [
        "",
        " ",
        "+",
        "-",
        ".",
        "e",
        "e5",
        "1e",
        "1e+",
        "1e-",
        ".e1",
        "1..2",
        "1.2.3",
        " 1",
        "1 ",
        "1_000",
        "1,5",
        "--1",
        "+-1",
        "0x10",
        "inf",
        "NaN",
        "1e5.5",
        "١",
        "1e999999999999999999999",
        "1e-999999999999999999999",
        "1e6145",
        "1e-6177",
        "12345678901234567890123456789012345e6111",
    ];
    for s in bad {
        assert!(Decimal::parse(s).is_none(), "should reject {s:?}");
    }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

#[test]
fn display_plain_strings() {
    let cases = [
        ("1.10", "1.10"),
        ("1e3", "1000"),
        ("-0.0050", "-0.0050"),
        ("0.00", "0.00"),
        ("0e3", "0"),
        ("-1.5e-10", "-0.00000000015"),
        ("123.456", "123.456"),
        ("1E-7", "0.0000001"),
        ("-12E+2", "-1200"),
    ];
    for (s, want) in cases {
        assert_eq!(d(s).to_string(), want, "{s}");
    }
    assert_eq!(format!("{:>8}", d("1.5")), "     1.5");
    assert_eq!(format!("{:<6}|", d("-1.5")), "-1.5  |");
    assert_eq!(format!("{:+}", d("1.5")), "+1.5");
    assert_eq!(format!("{:08}", d("-1.5")), "-00001.5");
    let tiny = d("1E-6176");
    let s = tiny.to_string();
    assert_eq!(s.len(), 6178);
    assert!(s.starts_with("0.000") && s.ends_with('1'));
    assert_eq!(d("1E+6144").to_string(), format!("1{}", "0".repeat(6144)));
}

#[test]
fn scientific_strings() {
    let cases = [
        ("1.10", "1.10"),
        ("1e3", "1E+3"),
        ("1.2e3", "1.2E+3"),
        ("123", "123"),
        ("-123", "-123"),
        ("0.000001", "0.000001"),
        ("0.0000001", "1E-7"),
        ("0.00000012", "1.2E-7"),
        ("0e3", "0E+3"),
        ("0e-10", "0E-10"),
        ("0.00", "0.00"),
        ("0", "0"),
        ("-1.5e-10", "-1.5E-10"),
        ("12E-1", "1.2"),
        ("1234.5e5", "1.2345E+8"),
    ];
    for (s, want) in cases {
        assert_eq!(d(s).to_scientific_string(), want, "{s}");
    }
}

// ---------------------------------------------------------------------------
// Equality, ordering and hashing
// ---------------------------------------------------------------------------

#[test]
fn equality_ordering_hash() {
    let equal_groups: &[&[&str]] = &[
        &["1", "1.0", "1.000", "10E-1", "0.1e1"],
        &["0", "0.00", "0e5", "-0", "0E-6176"],
        &["-2.5", "-2.50", "-25e-1"],
        &["1E+6111", "1000000000000000000000E+6090"],
    ];
    for group in equal_groups {
        let vals: Vec<Decimal> = group.iter().map(|s| d(s)).collect();
        for a in &vals {
            for b in &vals {
                assert_eq!(a, b);
                assert_eq!(a.cmp(b), std::cmp::Ordering::Equal);
                assert_eq!(hash_of(a), hash_of(b), "{a:?} {b:?}");
            }
        }
    }
    let ordered = [
        "-1E+6111",
        "-100",
        "-1.5",
        "-1.499999999999999999999999999999999",
        "-1E-6176",
        "0",
        "1E-6176",
        "0.1",
        "0.99999999999999999999999999999999",
        "1",
        "1.000000000000000000000000000000001",
        "10",
        "99",
        "100",
        "1E+6111",
    ];
    let vals: Vec<Decimal> = ordered.iter().map(|s| d(s)).collect();
    for i in 0..vals.len() {
        for j in 0..vals.len() {
            assert_eq!(
                vals[i].cmp(&vals[j]),
                i.cmp(&j),
                "{} vs {}",
                ordered[i],
                ordered[j]
            );
        }
    }
    let mut sorted = [d("3"), d("-1"), d("2.50"), d("2.5"), d("0")];
    sorted.sort();
    let s: Vec<String> = sorted.iter().map(|x| x.to_string()).collect();
    assert_eq!(s, ["-1", "0", "2.50", "2.5", "3"]);
    let set: std::collections::HashSet<Decimal> = ["1", "1.0", "2", "2.00", "0", "0.0"]
        .iter()
        .map(|s| d(s))
        .collect();
    assert_eq!(set.len(), 3);
}

// ---------------------------------------------------------------------------
// Accessors and conversions
// ---------------------------------------------------------------------------

#[test]
fn accessors_and_conversions() {
    let x = d("-12.340");
    assert_eq!(x.coefficient(), -12340);
    assert_eq!(x.scale(), 3);
    assert_eq!(x.precision(), 5);
    assert_eq!(x.signum(), -1);
    assert!(x.is_negative() && !x.is_positive() && !x.is_zero());
    assert_eq!(x.abs().to_string(), "12.340");
    assert_eq!(x.neg().to_string(), "12.340");
    assert_eq!((-x).to_string(), "12.340");
    assert_eq!(Decimal::ZERO.neg().signum(), 0);
    assert!(!Decimal::ZERO.neg().is_negative());
    assert_eq!(d("0.000").precision(), 1);
    assert!(d("2.00").is_integer());
    assert!(!d("2.01").is_integer());
    assert!(d("1E+5").is_integer());
    assert!(!d("1E-40").is_integer());
    assert_eq!(x.normalize().to_string(), "-12.34");
    assert_eq!(d("1200").normalize().to_scientific_string(), "1.2E+3");
    assert_eq!(d("0.000").normalize().scale(), 0);
    assert_eq!(d("1E+6111").normalize().scale(), MIN_SCALE);
    assert_eq!(d("1E+6144").normalize().scale(), MIN_SCALE);
    assert_eq!(d("2.00").to_i64(), Some(2));
    assert_eq!(d("2.5").to_i64(), None);
    assert_eq!(d("-9223372036854775808").to_i64(), Some(i64::MIN));
    assert_eq!(d("9223372036854775808").to_i64(), None);
    assert_eq!(d("1E+30").to_i128(), Some(10i128.pow(30)));
    assert_eq!(d("1E+39").to_i128(), None);
    // Rounded to 34 digits, which then fits.
    assert_eq!(
        d("-170141183460469231731687303715884105728").to_i128(),
        Some(-170141183460469231731687303715884100000)
    );
    assert_eq!(d("1.8E+38").to_i128(), None);
    assert_eq!(
        d("-1.7E+38").to_i128(),
        Some(-170000000000000000000000000000000000000)
    );
    assert_eq!(d("0.5").to_f64(), 0.5);
    assert_eq!(d("-1.1").to_f64(), -1.1);
    assert_eq!(d("1E-400").to_f64(), 0.0);
    assert_eq!(d("1.7976931348623157E+308").to_f64(), f64::MAX);
    assert_eq!(
        d("123456789012345678901234567890").to_f64(),
        1.2345678901234568e29
    );
    assert_eq!(
        Decimal::from_i64(i64::MIN).to_string(),
        "-9223372036854775808"
    );
    assert_eq!(
        Decimal::from_u64(u64::MAX).to_string(),
        "18446744073709551615"
    );
    assert_eq!(
        Decimal::from_i128(i128::MIN)
            .unwrap()
            .to_scientific_string(),
        "-1.701411834604692317316873037158841E+38"
    );
    assert_eq!(Decimal::new(12345, 2).unwrap().to_string(), "123.45");
    assert_eq!(Decimal::new(-5, -3).unwrap().to_string(), "-5000");
    assert!(Decimal::new(1, 6177).is_none());
    assert!(Decimal::new(1, -6145).is_none());
    assert_eq!(Decimal::new(1, -6144).unwrap().scale(), MIN_SCALE);
    assert_eq!(Decimal::new(0, i32::MAX).unwrap().scale(), MAX_SCALE);
    assert_eq!(
        Decimal::from_parts(true, 15, 1).unwrap().to_string(),
        "-1.5"
    );
    assert_eq!(
        Decimal::from_parts(true, 0, 1).unwrap().to_parts(),
        (false, 0, 1)
    );
    assert_eq!(Decimal::default(), Decimal::ZERO);
    assert_eq!(Decimal::from(7i32).to_string(), "7");
}

// ---------------------------------------------------------------------------
// Range boundaries
// ---------------------------------------------------------------------------

#[test]
fn range_boundaries() {
    let max = d(&format!("{}E+6111", "9".repeat(34)));
    assert_eq!(max.scale(), MIN_SCALE);
    assert!(max.mul(&d("10")).is_none());
    assert!(max.add(&d("0.5E+6111")).is_none());
    assert_eq!(max.add(&d("0.4E+6111")).unwrap(), max);
    let min = d("1E-6176");
    assert!(min.div(&d("2")).is_none());
    assert!(min.mul(&d("0.1")).is_none());
    assert_eq!(min.mul(&d("1.0")).unwrap(), min);
    // An exact result with a too-large scale is rescued by stripping zeros.
    assert_eq!(d("1.0E-6175").mul(&d("0.1")).unwrap(), min);
    // Inexact underflow: 34-digit result would need scale 6177+.
    assert!(d("1.234E-6170").div(&d("1E+10")).is_none());
    // Zero results clamp their scale.
    assert_eq!(min.sub(&min).unwrap().scale(), MAX_SCALE);
    assert_eq!(d("0E-6000").mul(&d("0E-6000")).unwrap().scale(), MAX_SCALE);
    assert_eq!(d("0E+6000").mul(&d("0E+6000")).unwrap().scale(), MIN_SCALE);
    assert_eq!(d("0").div(&d("1E-6176")).unwrap().scale(), MIN_SCALE);
    // exp overflow/underflow.
    assert!(d("14150").exp().is_none());
    assert!(d("-14221").exp().is_none());
    assert!(d("1E+100").exp().is_none());
    assert!(d("-1E+100").exp().is_none());
    assert!(d("14149").exp().is_some());
    // Results below 1E-6143 need a scale above 6176 for 34 digits: inexact
    // results there underflow to null.
    assert!(d("-14140").exp().is_some());
    assert!(d("-14200").exp().is_none());
}

#[test]
fn exp_ln_consistency() {
    for s in [
        "0.5", "2", "10", "123.456", "0.001", "1E-20", "-3", "1E-40", "-1E-40",
    ] {
        let x = d(s);
        let e = x.exp().unwrap();
        if x.is_positive() {
            let l = x.ln().unwrap();
            assert!(
                l.exp().unwrap().sub(&x).unwrap().abs()
                    < d("1E-30")
                        .mul(&x.abs().add(&Decimal::ONE).unwrap())
                        .unwrap()
            );
        }
        assert!(e.is_positive());
    }
    assert_eq!(
        d("1E-40").exp().unwrap().to_string(),
        format!("1.{}", "0".repeat(33))
    );
    assert_eq!(
        d("-1E-40").exp().unwrap().to_string(),
        format!("1.{}", "0".repeat(33))
    );
}
