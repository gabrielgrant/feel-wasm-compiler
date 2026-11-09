#!/usr/bin/env python3
"""Generate tests/vectors.txt for the feel-decimal crate.

Reference results come from Python's `decimal` module:

* `+ - * /`, remainder, quantize, comparison and parsing use the C `decimal`
  module with prec=34, ROUND_HALF_EVEN and an effectively unbounded exponent
  range;
* powers, sqrt, exp, ln and log10 use the pure-Python `_pydecimal` module,
  whose algorithms are correctly rounded in every case.

Every result (computed as if the exponent range were unbounded) is then
passed through `finish`, which implements the crate's documented range rules
(Python exponent = -scale):

* zero: clamp the scale into [MIN_SCALE, MAX_SCALE];
* scale > MAX_SCALE: strip trailing zeros; still too large -> null;
* scale < MIN_SCALE: pad zeros while the coefficient has <= 34 digits;
  still too small -> null.

Output format: one vector per line, space separated:
    <op> <operand>... <expected>
Operands and results use Python's `str()` form (= Java's BigDecimal.toString(),
= Decimal::to_scientific_string), so the scale is checked exactly. `null`
means the operation must return None. The `plain` op checks Display
(toPlainString) and `parse` checks parsing of a raw literal.

Usage: python3 gen_vectors.py > vectors.txt
"""

import random
import sys

import decimal as C
import _pydecimal as P

MIN_SCALE, MAX_SCALE = -6111, 6176
HUGE = 10**9

CCTX = C.Context(prec=34, rounding=C.ROUND_HALF_EVEN, Emax=HUGE, Emin=-HUGE, traps=[])
PCTX = P.Context(prec=34, rounding=P.ROUND_HALF_EVEN, Emax=HUGE, Emin=-HUGE, traps=[])
BIG = C.Context(prec=20000, rounding=C.ROUND_HALF_EVEN, Emax=HUGE, Emin=-HUGE, traps=[])

MODES = {
    "HalfEven": C.ROUND_HALF_EVEN,
    "HalfUp": C.ROUND_HALF_UP,
    "HalfDown": C.ROUND_HALF_DOWN,
    "Up": C.ROUND_UP,
    "Down": C.ROUND_DOWN,
    "Floor": C.ROUND_FLOOR,
    "Ceiling": C.ROUND_CEILING,
}

rng = random.Random(20261109)
out = []


def mk(sign, coeff, scale):
    """C.Decimal from sign (0/1), integer coefficient and scale."""
    return C.Decimal((sign, tuple(int(ch) for ch in str(coeff)), -scale))


def parts(d):
    sign, digits, exp = d.as_tuple()
    return sign, int("".join(map(str, digits))), -exp


def finish(d):
    """Apply the crate's range rules; returns a C.Decimal or None."""
    if d is None:
        return None
    d = C.Decimal(str(d))
    if not d.is_finite():
        return None
    sign, coeff, scale = parts(d)
    assert len(str(coeff)) <= 34, d
    if coeff == 0:
        return mk(0, 0, min(max(scale, MIN_SCALE), MAX_SCALE))
    if scale > MAX_SCALE:
        need = scale - MAX_SCALE
        if need > 33 or coeff % 10**need:
            return None
        coeff //= 10**need
        scale = MAX_SCALE
    elif scale < MIN_SCALE:
        need = MIN_SCALE - scale
        if need > 33 or len(str(coeff)) + need > 34:
            return None
        coeff *= 10**need
        scale = MIN_SCALE
    return mk(sign, coeff, scale)


def pc(method, *args):
    """Call a PCTX method; a result that underflowed even the huge exponent
    range of the oracle context is a nonzero value far below MAX_SCALE, i.e.
    null (Python would round it to zero)."""
    PCTX.clear_flags()
    r = getattr(PCTX, method)(*args)
    if PCTX.flags[P.Underflow]:
        return None
    return r


def s(d):
    return "null" if d is None else str(d)


def emit(op, *args):
    out.append(" ".join([op] + [str(a) for a in args]))


def valid(d):
    """True if d is a representable operand (<= 34 digits, scale in range)."""
    _, coeff, scale = parts(d)
    return len(str(coeff)) <= 34 and MIN_SCALE <= scale <= MAX_SCALE


# ---------------------------------------------------------------------------
# Random operands
# ---------------------------------------------------------------------------

def rnd_coeff():
    r = rng.random()
    if r < 0.18:
        n = 34
    elif r < 0.30:
        n = rng.randint(28, 33)
    elif r < 0.40:
        n = rng.randint(13, 27)
    else:
        n = rng.randint(1, 12)
    lo = 10 ** (n - 1) if n > 1 else 0
    c = rng.randrange(lo, 10**n)
    t = rng.random()
    if t < 0.08 and n > 1:
        # trailing zeros
        z = rng.randint(1, n - 1)
        c = (c // 10**z) * 10**z
    elif t < 0.14:
        # runs of nines (carry stress)
        c = int("9" * n)
    elif t < 0.18 and n > 1:
        c = 10 ** (n - 1)
    return c


def rnd_scale():
    r = rng.random()
    if r < 0.55:
        return rng.randint(-4, 10)
    if r < 0.78:
        return rng.randint(-40, 60)
    if r < 0.88:
        return rng.randint(MIN_SCALE, MAX_SCALE)
    if r < 0.94:
        return rng.randint(MAX_SCALE - 75, MAX_SCALE)
    return rng.randint(MIN_SCALE, MIN_SCALE + 75)


def rnd_dec(nonzero=False, positive=False, scale=None):
    while True:
        c = rnd_coeff()
        if nonzero and c == 0:
            continue
        sign = 0 if positive else rng.randint(0, 1)
        if c == 0:
            sign = 0
        sc = rnd_scale() if scale is None else scale
        return mk(sign, c, sc)


def rnd_moderate(nonzero=False, positive=False):
    """Operand with a moderate magnitude (for functions with small domains)."""
    while True:
        c = rnd_coeff()
        if nonzero and c == 0:
            continue
        n = len(str(c))
        # value roughly within 1e-8 .. 1e8
        sc = rng.randint(n - 9, n + 8)
        sign = 0 if positive else rng.randint(0, 1)
        if c == 0:
            sign = 0
        return mk(sign, c, sc)


def rnd_pair_close():
    """Two operands with nearby scales (carries, cancellations, ties)."""
    a = rnd_dec()
    _, ca, sa = parts(a)
    sb = sa + rng.randint(-3, 40)
    sb = min(max(sb, MIN_SCALE), MAX_SCALE)
    b = mk(rng.randint(0, 1), rnd_coeff(), sb)
    if b == 0:
        b = mk(0, 0, sb)
    return a, b


# ---------------------------------------------------------------------------
# Operations
# ---------------------------------------------------------------------------

def binop(op, a, b):
    if op == "add":
        r = finish(CCTX.add(a, b))
    elif op == "sub":
        r = finish(CCTX.subtract(a, b))
    elif op == "mul":
        r = finish(CCTX.multiply(a, b))
    elif op == "div":
        r = None if b == 0 else finish(CCTX.divide(a, b))
    elif op == "rem":
        r = None if b == 0 else finish(remainder(a, b))
    elif op == "modulo":
        if b == 0:
            r = None
        else:
            rm = remainder(a, b)
            if rm != 0 and (rm < 0) != (b < 0):
                rm = CCTX.add(rm, b)
            r = finish(rm)
    else:
        raise ValueError(op)
    emit(op, a, b, s(r))


def remainder(a, b):
    r = BIG.remainder(a, b)
    if r == 0:
        r = abs(r)
    _, c, _ = parts(r)
    assert len(str(c)) <= 34, (a, b, r)
    return r


def pow_int_expected(a, n):
    if n == 0:
        return mk(0, 1, 0)
    _, ca, sa = parts(a)
    if ca == 0:
        if n < 0:
            return None
        return finish(mk(0, 0, sa * n))
    r = pc('power', P.Decimal(str(a)), P.Decimal(n))
    return finish(r)


def pow_real_expected(a, y):
    """Expected result of Decimal::pow(a, y)."""
    if y == y.to_integral_value() and abs(y) <= 999_999_999:
        return pow_int_expected(a, int(y))
    if a == 0:
        return mk(0, 0, 0) if y > 0 else None
    if a < 0:
        return None
    if a == 1:
        return mk(0, 1, 0)
    pa, py = P.Decimal(str(a)), P.Decimal(str(y))
    exact = pa._power_exact(py, 35)
    if exact is not None:
        sign, coeff, scale = parts(C.Decimal(str(exact)))
        if len(str(coeff)) > 34:
            return finish(CCTX.plus(C.Decimal(str(exact))))
        # normalized: minimal digits, integers at scale 0 when they fit
        while scale > 0 and coeff % 10 == 0:
            coeff //= 10
            scale -= 1
        if scale < 0 and len(str(coeff)) - scale <= 34:
            coeff *= 10 ** (-scale)
            scale = 0
        return finish(mk(0, coeff, scale))
    return finish(pc('power', pa, py))


def unary_expected(op, a):
    pa = P.Decimal(str(a))
    if op == "sqrt":
        return None if a < 0 else finish(pc('sqrt', pa))
    if op == "exp":
        return finish(pc('exp', pa))
    if op == "ln":
        return None if a <= 0 else finish(pc('ln', pa))
    if op == "log10":
        return None if a <= 0 else finish(pc('log10', pa))
    if op == "normalize":
        sign, c, sc = parts(a)
        if c == 0:
            return mk(0, 0, 0)
        while c % 10 == 0 and sc > MIN_SCALE:
            c //= 10
            sc -= 1
        return mk(sign, c, sc)
    if op in ("floor", "ceil", "trunc"):
        mode = {"floor": C.ROUND_FLOOR, "ceil": C.ROUND_CEILING, "trunc": C.ROUND_DOWN}[op]
        r = quantize(a, 0, mode)
        if r is None:
            return a  # integral value with too many digits for scale 0
        return r
    raise ValueError(op)


def quantize(a, scale, mode):
    if scale < MIN_SCALE or scale > MAX_SCALE:
        return None
    ctx = C.Context(prec=34, rounding=mode, Emax=HUGE, Emin=-HUGE, traps=[C.InvalidOperation])
    try:
        r = a.quantize(mk(0, 1, scale), context=ctx)
    except C.InvalidOperation:
        return None
    if r == 0:
        r = abs(r)
    return r


# ---------------------------------------------------------------------------
# Fixed edge cases
# ---------------------------------------------------------------------------

def D(v):
    """Operand literal in its canonical in-range representation."""
    r = finish(CCTX.plus(C.Decimal(v)))
    assert r is not None, v
    return r


N34 = "9" * 34
E33 = "1" + "0" * 33

edge_values = [
    "0", "0.00", "0E+5", "0E-6176", "0E+6111", "1", "-1", "2", "3", "7", "10", "0.1", "0.5",
    "-0.5", "1.5", "2.5", "-2.5", "1.10", "100", "1E+3", "1E-3", N34, "-" + N34, E33,
    E33[:-1] + "1", "5" + "0" * 33, "1E-6176", "-1E-6176", N34 + "E+6111", "-" + N34 + "E+6111",
    "1E+6144", "1E+6111", "1.000000000000000000000000000000001", "0.9999999999999999999999999999999999",
    "3.141592653589793238462643383279503", "123456789012345678901234567890.1234",
    "1E-6143", "9.999999999999999999999999999999999E-6143", "1E-6150", "12345E-6176",
]
edge = [D(v) for v in edge_values]
assert all(valid(v) for v in edge)

for a in edge:
    for b in edge:
        for op in ("add", "sub", "mul", "div"):
            binop(op, a, b)
        if rng.random() < 0.3:
            binop("rem", a, b)
            binop("modulo", a, b)

# Carries, ties and cancellation around the 34th/35th digit.
specials = [
    (N34, "1"), (N34, "0.5"), (N34, "0.4"), (N34, "0.6"), (N34, "-0.5"), (E33, "-0.5"), (E33, "-0.4"),
    (E33, "-0.6"), (E33 + "1", "0"), ("1" + "0" * 32 + "1", "0.5"), ("1" + "0" * 33, "0.5"),
    ("1" + "0" * 32 + "3", "0.5"), ("1", "-1"), ("-1", "1"), ("1.0", "-1"), ("-0.0", "0.00"),
    ("1", "1E-6176"), ("1", "-1E-6176"), ("1E+6111", "1E-6176"), ("1", "1E-33"), ("1", "-1E-33"),
    ("1", "5E-34"), ("1", "-5E-34"), ("1", "5E-35"), ("1", "-5E-35"), ("1", "5.000001E-34"),
    ("1", "-5.000001E-35"), ("1", "1E-40"), ("1", "-1E-40"), ("1", "-1E-70"), ("1", "1E-69"),
    ("1E+40", "1"), ("1E+40", "-1"), ("1E+69", "-1"), ("1E+68", "-1"), ("1E+67", "-0.5"),
    ("9E+70", "-9.9E+36"), ("1E+33", "-0.5"), ("1E+34", "-0.5"), ("1E+34", "-5"), ("1E+34", "-6"),
    ("1E+35", "-50"), ("123", "-123.000"), ("1E+6144", "-1E+6110"), (N34 + "E+6111", "5E+6110"),
    (N34 + "E+6111", "4.9E+6110"), ("1E-6176", "-1E-6176"),
]
for x, y in specials:
    a, b = D(x), D(y)
    for op in ("add", "sub", "mul", "div"):
        binop(op, a, b)

div_specials = [
    ("1", "3"), ("2", "3"), ("1", "7"), ("-1", "3"), ("1", "-3"), ("1", "9999"), ("2", "0.5"),
    ("1", "8"), ("1.00", "8"), ("10", "4"), ("6.0", "2"), ("1", "2"), ("1.01", "2"), ("100", "1"),
    ("1E+2", "1"), ("0.00", "5"), ("5", "0"), ("0", "0"), ("1E-6176", "10"), ("1E-6176", "2"),
    ("10E-6176", "10"), ("1E+6111", "0.1"), ("1E+6144", "0.1"), (N34, "3"), (N34, "7"), ("1", N34),
    (N34, N34), ("1E-6176", N34), ("1E+6144", "1E-6176"), ("1E-6176", "1E+6111"), ("22", "7"),
    ("355", "113"), ("1", "1024"), ("1", "1E+34"), ("3", "1E-40"), ("123456789", "1000"),
    ("2E+10", "4E+5"),
]
for x, y in div_specials:
    binop("div", D(x), D(y))

mod_specials = [
    ("12", "5"), ("-12", "5"), ("12", "-5"), ("-12", "-5"), ("10.1", "4.5"), ("-10.1", "4.5"),
    ("10.1", "-4.5"), ("-10.1", "-4.5"), ("10", "2.5"), ("1E+30", "7"), ("1", "1E-20"),
    ("1E+6000", "7"), ("1E+6000", "1E-6000"), ("1E-30", "1E+30"), ("-1E-30", "1E+30"),
    ("-1E-40", "1"), ("-1E-33", "1"), ("1E-6176", "-1E+6111"), (N34 + "E+6111", "3"),
    (N34 + "E+6111", "7E-6176"), ("5", "0"), ("0", "5"), ("0.000", "-5"), ("-7", "7"), ("7.00", "-7"),
]
for x, y in mod_specials:
    binop("rem", D(x), D(y))
    binop("modulo", D(x), D(y))

# ---------------------------------------------------------------------------
# Random binary operations
# ---------------------------------------------------------------------------

for _ in range(700):
    a, b = rnd_dec(), rnd_dec()
    for op in ("add", "sub", "mul", "div"):
        binop(op, a, b)
for _ in range(500):
    a, b = rnd_pair_close()
    for op in ("add", "sub"):
        binop(op, a, b)
for _ in range(250):
    # ties in multiplication: odd 34-digit coefficient times 0.5
    c = rng.randrange(10**33, 10**34) | 1
    a = mk(rng.randint(0, 1), c, rng.randint(-5, 20))
    binop("mul", a, D("0.5"))
    binop("mul", a, D("-5"))
    binop("div", a, D("2"))
for _ in range(300):
    a, b = rnd_dec(), rnd_dec(nonzero=True)
    binop("rem", a, b)
    binop("modulo", a, b)
for _ in range(300):
    a, b = rnd_moderate(), rnd_moderate(nonzero=True)
    binop("rem", a, b)
    binop("modulo", a, b)
for _ in range(400):
    # moderate magnitudes: the common case in practice
    a, b = rnd_moderate(), rnd_moderate()
    for op in ("add", "sub", "mul", "div"):
        binop(op, a, b)

# ---------------------------------------------------------------------------
# Comparison
# ---------------------------------------------------------------------------

for _ in range(600):
    a = rnd_dec()
    r = rng.random()
    if r < 0.3:
        # same value, different scale
        sign, c, sc = parts(a)
        k = rng.randint(0, 34 - len(str(c)))
        b = mk(sign, c * 10**k, sc + k) if MIN_SCALE <= sc + k <= MAX_SCALE else a
    elif r < 0.6:
        a, b = rnd_pair_close()
    else:
        b = rnd_dec()
    emit("cmp", a, b, int(a.compare(b)))

# ---------------------------------------------------------------------------
# Scale manipulation
# ---------------------------------------------------------------------------

for _ in range(700):
    a = rnd_dec() if rng.random() < 0.3 else rnd_moderate()
    _, c, sa = parts(a)
    r = rng.random()
    if r < 0.8:
        target = sa + rng.randint(-36, 36)
    elif r < 0.9:
        target = rng.randint(-50, 50)
    else:
        target = rng.choice([MIN_SCALE, MAX_SCALE, MIN_SCALE - 1, MAX_SCALE + 1, sa - 5000, sa + 5000])
    mode = rng.choice(list(MODES))
    emit("set_scale", a, target, mode, s(quantize(a, target, MODES[mode])))
    if sa <= target:
        rts = a
    else:
        rts = quantize(a, target, MODES[mode])
    emit("round_to_scale", a, target, mode, s(rts))

for v in ["5.5", "-5.5", "2.5", "-2.5", "1.121", "-1.126", "0.505", "0.515", "1.56", "-1.56",
          "0.5", "-0.5", "0.0001", "-0.0001", "9.5", N34 + "E-1"]:
    for mode in MODES:
        for target in (0, 1, 2):
            a = D(v)
            emit("set_scale", a, target, mode, s(quantize(a, target, MODES[mode])))

for _ in range(300):
    a = rnd_dec()
    op = rng.choice(["normalize", "floor", "ceil", "trunc"])
    emit(op, a, s(unary_expected(op, a)))
for v in edge:
    for op in ["normalize", "floor", "ceil", "trunc"]:
        emit(op, v, s(unary_expected(op, v)))

# ---------------------------------------------------------------------------
# Powers
# ---------------------------------------------------------------------------

pow_bases = ["2", "3", "7", "10", "0.5", "1.1", "1.10", "-2", "-1.5", "0.1", "1.0001", "0.9999",
             "1.000000000000000000000000000000001", "0.9999999999999999999999999999999999", "1E+10",
             "1E-10", "12345.6789", N34, "5", "0.2", "4", "0.25", "1.0", "1", "-1", "0", "0.00",
             "2.5", "1E+6111", "1E-6176", "99", "0.03125"]
pow_exps = [0, 1, 2, 3, -1, -2, -3, 5, 10, 17, -7, 33, 34, 35, 100, -100, 1000, 6144, -6176,
            999_999_999, -999_999_999, 123456, -54321]
for x in pow_bases:
    for n in pow_exps:
        a = D(x)
        emit("pow_i", a, n, s(pow_int_expected(a, n)))
for _ in range(400):
    a = rnd_moderate()
    if rng.random() < 0.5:
        n = rng.randint(-40, 40)
    else:
        n = rng.randint(-3000, 3000)
    emit("pow_i", a, n, s(pow_int_expected(a, n)))

real_pow = [
    ("4", "0.5"), ("2", "0.5"), ("0.25", "0.5"), ("4", "-0.5"), ("100", "1.5"), ("8", "2.5"),
    ("16", "0.25"), ("0.0625", "0.25"), ("1E+4", "0.5"), ("1E+6", "1.5"), ("9", "-0.5"),
    ("25", "-0.5"), ("32", "-0.2"), ("0.25", "-1.5"), ("100", "0.25"), ("1.44", "0.5"),
    ("2", "-0.5"), ("10", "0.1"), ("10", "-0.1"), ("1E+10", "0.5"), ("1E+11", "0.5"),
    ("1.0000000000000001", "1E+10"), ("2", "1E+10"), ("0.5", "1E+10"), ("-2", "0.5"),
    ("0", "0.5"), ("0", "-0.5"), ("1", "0.5"), ("1.00", "123.456"), ("2", "3"), ("2", "-3"),
    ("2", "3.0"), ("27", "0.3333333333333333333333333333333333"), ("1E+100", "0.01"),
    ("7", "0.5"), ("2", "6176.5"), ("2", "-6176.5"), ("10", "6144.4"), ("10", "-6176.4"),
    ("3", "1E-40"), ("3", "-1E-40"), ("123.456", "7.89"), ("0.001", "-2.5"), ("1E+6111", "1.0001"),
    ("4.0", "0.5"), ("4", "0.50"), ("1E+30", "1.1"), ("5", "-50.5"), ("25", "-50.5"),
    ("6.25", "1.5"), ("1.21", "0.5"), ("0.0016", "0.25"),
]
for x, y in real_pow:
    a, b = D(x), D(y)
    emit("pow", a, b, s(pow_real_expected(a, b)))
for _ in range(250):
    a = rnd_moderate(nonzero=True, positive=True)
    y = mk(rng.randint(0, 1), rng.randrange(1, 10**rng.randint(1, 12)), rng.randint(1, 8))
    emit("pow", a, y, s(pow_real_expected(a, y)))
for _ in range(60):
    # exact square / fourth roots
    r = rnd_moderate(nonzero=True, positive=True)
    _, c, sc = parts(r)
    if len(str(c)) > 8:
        continue
    k = rng.choice([2, 4])
    a = CCTX.power(r, k)
    if not valid(a):
        continue
    y = D("0.5") if k == 2 else D("0.25")
    if rng.random() < 0.3:
        y = -y
    emit("pow", a, y, s(pow_real_expected(a, y)))

# ---------------------------------------------------------------------------
# sqrt, exp, ln, log10
# ---------------------------------------------------------------------------

unary_edge = edge + [D(v) for v in [
    "16", "0.25", "1.00", "2", "4", "1E+6", "1E-6", "0.01", "1E+3", "1E-3", "6.25", "0.0004",
    "1E-6175", "1E+6143", "14149", "14150", "14149.6", "-14140", "-14145", "-14200", "-14220",
    "-14221", "1E-30", "-1E-30", "1E-34", "-1E-34", "1E-35", "-1E-35", "1E-36", "-1E-36",
    "5", "100", "-100", "1000", "0.999", "1.001", "1E+100", "-1E+100", "2.302585092994045684017991454684364",
]]
for op in ("sqrt", "exp", "ln", "log10"):
    for a in unary_edge:
        emit(op, a, s(unary_expected(op, a)))

for _ in range(300):
    a = rnd_dec(positive=True) if rng.random() < 0.5 else rnd_moderate(positive=True)
    emit("sqrt", a, s(unary_expected("sqrt", a)))
for _ in range(80):
    r = rnd_moderate(positive=True)
    _, c, sc = parts(r)
    if len(str(c)) > 17:
        continue
    a = CCTX.multiply(r, r)
    if valid(a):
        emit("sqrt", a, s(unary_expected("sqrt", a)))

for _ in range(250):
    c = rnd_coeff()
    n = len(str(c))
    sc = rng.randint(n - 3, n + 12)  # |x| below ~1000
    a = mk(rng.randint(0, 1), c, sc)
    emit("exp", a, s(unary_expected("exp", a)))
for _ in range(40):
    a = mk(rng.randint(0, 1), rng.randrange(10**33, 10**34), rng.randint(29, 31))
    emit("exp", a, s(unary_expected("exp", a)))

for _ in range(300):
    a = rnd_dec(positive=True, nonzero=True) if rng.random() < 0.5 else rnd_moderate(positive=True, nonzero=True)
    emit("ln", a, s(unary_expected("ln", a)))
    emit("log10", a, s(unary_expected("log10", a)))
for k in range(1, 34):
    for v in (mk(0, 10**k + 1, k), mk(0, 10**k - 1, k)):
        emit("ln", v, s(unary_expected("ln", v)))
        emit("log10", v, s(unary_expected("log10", v)))

# ---------------------------------------------------------------------------
# Parsing and plain formatting
# ---------------------------------------------------------------------------

def rnd_literal():
    def digits(n):
        return "".join(rng.choice("0123456789") for _ in range(n))
    r = rng.random()
    n = rng.choice([1, 2, 5, 20, 33, 34, 35, 36, 40, 80, 200])
    body = digits(n)
    if rng.random() < 0.2:
        body = "0" * rng.randint(1, 50) + body
    form = rng.random()
    if form < 0.4:
        lit = body
    elif form < 0.7:
        k = rng.randint(0, len(body))
        lit = body[:k] + "." + body[k:]
        if lit == ".":
            lit = "0."
    else:
        lit = "." + body
    if rng.random() < 0.4:
        e = rng.choice([rng.randint(-50, 50), rng.randint(-7000, 7000), rng.choice([6111, 6144, 6145, -6176, -6177, -6200])])
        lit += rng.choice("eE") + (rng.choice(["", "+"]) if e >= 0 else "") + str(e)
    if r < 0.5:
        lit = rng.choice(["", "-", "+"]) + lit
    return lit


for _ in range(500):
    lit = rnd_literal()
    emit("parse", lit, s(finish(CCTX.create_decimal(lit))))
for lit in ["0", "-0", "+0.000", "1.", ".5", "-.5", "1e3", "1E-3", "1.2e+3", "00012.3400",
            "9" * 35, "9" * 34 + "5", "1" + "0" * 33 + "5", "1" + "0" * 32 + "15", "1" + "0" * 33 + "51",
            "0." + "0" * 40 + "1", "1e6144", "1e6145", "1e-6176", "1e-6177", "10e-6177",
            "0e999999999999", "0e-999999999999", "12345678901234567890123456789012345e6110"]:
    emit("parse", lit, s(finish(CCTX.create_decimal(lit))))


def plain_expected(d):
    if d == 0 and d.as_tuple()[2] > 0:
        return "0"
    return format(d, "f")


for _ in range(300):
    c = rnd_coeff()
    a = mk(rng.randint(0, 1) if c else 0, c, rng.randint(-45, 60))
    emit("plain", a, plain_expected(a))
for v in edge:
    _, _, sc = parts(v)
    if -60 <= sc <= 80:
        emit("plain", v, plain_expected(v))

sys.stdout.write("# Generated by gen_vectors.py -- do not edit.\n")
sys.stdout.write("\n".join(out) + "\n")
