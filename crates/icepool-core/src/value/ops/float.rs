//! `float` arithmetic as CPython's `Objects/floatobject.c` does it: `//`, `%`, `**` and `round`.

use super::integer::round_half_even;
use super::{zero_division, BinOp};
use crate::error::{Error, Result};
use crate::value::Value;
use dashu_base::UnsignedAbs;
use dashu_int::IBig;
use dashu_ratio::RBig;

fn is_odd_integer(x: f64) -> bool {
    (x.abs() % 2.0) == 1.0
}

/// `round(x, ndigits)` for a float (CPython `float___round___impl` and `double_round`): the exact binary value is
/// rounded to `ndigits` decimals, ties to even (dtoa mode 3), and the decimal string is read back as a float.
pub(super) fn round_float(x: f64, ndigits: &IBig) -> Result<f64> {
    // NDIGITS_MAX and NDIGITS_MIN: beyond them the result is `x`, or a zero, outright. (ndigits saturates.)
    let nd = i64::try_from(ndigits.clone()).unwrap_or(if *ndigits > IBig::ZERO {
        i64::MAX
    } else {
        i64::MIN
    });
    // Nans and infinities round to themselves.
    if !x.is_finite() || nd > 323 {
        return Ok(x);
    }
    if nd < -308 {
        return Ok(0.0 * x);
    }
    let exact = RBig::try_from(x).expect("finite");
    let pow10 = IBig::from(10u8).pow(nd.unsigned_abs() as usize);
    let (num, den) = (exact.numerator(), IBig::from(exact.denominator().clone()));
    let digits = if nd >= 0 {
        round_half_even(&(num * &pow10), &den)
    } else {
        round_half_even(num, &(den * &pow10))
    };
    let sign = if x.is_sign_negative() { "-" } else { "" };
    let rounded: f64 = format!("{sign}{}e{}", digits.unsigned_abs(), -nd)
        .parse()
        .expect("a valid float literal");
    if rounded.is_infinite() {
        return Err(Error::Overflow(
            "rounded value too large to represent".into(),
        ));
    }
    Ok(rounded)
}

/// Float arithmetic, as CPython's `float_add` ... `float_pow`.
pub(super) fn float_op(op: BinOp, a: f64, b: f64) -> Result<Value> {
    use BinOp::*;
    Ok(Value::Float(match op {
        Add => a + b,
        Sub => a - b,
        Mul => a * b,
        TrueDiv if b == 0.0 => return zero_division("float division by zero"),
        TrueDiv => a / b,
        FloorDiv | Mod if b == 0.0 => return zero_division("float floor division by zero"),
        FloorDiv => float_divmod(a, b).0,
        Mod => float_divmod(a, b).1,
        Pow => return float_pow(a, b),
        _ => unreachable!("bitwise operators and comparisons never reach float_op"),
    }))
}

/// CPython's `_float_div_mod`: `fmod`, with the remainder moved to the divisor's sign and the quotient snapped to
/// the nearest integer.
pub(super) fn float_divmod(vx: f64, wx: f64) -> (f64, f64) {
    let mut m = vx % wx;
    // fmod is exact, so vx - m is an exact multiple of wx (up to rounding of the division).
    let mut div = (vx - m) / wx;
    if m != 0.0 {
        // Ensure the remainder has the same sign as the denominator.
        if (wx < 0.0) != (m < 0.0) {
            m += wx;
            div -= 1.0;
        }
    } else {
        // The remainder is zero, and in the presence of signed zeroes fmod returns different results across
        // platforms; ensure it has the same sign as the denominator.
        m = 0.0f64.copysign(wx);
    }
    let floordiv = if div != 0.0 {
        // Snap the quotient to the nearest integral value.
        let mut f = div.floor();
        if div - f > 0.5 {
            f += 1.0;
        }
        f
    } else {
        // div is zero: get the correct sign.
        0.0f64.copysign(vx / wx)
    };
    (floordiv, m)
}

/// CPython's `float_pow`, which sorts out the special cases itself instead of relying on the platform `pow()`.
pub(super) fn float_pow(iv: f64, iw: f64) -> Result<Value> {
    let float = |x: f64| Ok(Value::Float(x));
    // v**0 is 1, even 0**0.
    if iw == 0.0 {
        return float(1.0);
    }
    // nan**w = nan, unless w == 0.
    if iv.is_nan() {
        return float(iv);
    }
    // v**nan = nan, unless v == 1; 1**nan = 1.
    if iw.is_nan() {
        return float(if iv == 1.0 { 1.0 } else { iw });
    }
    if iw.is_infinite() {
        // v**inf is: 0.0 if abs(v) < 1; 1.0 if abs(v) == 1; inf if abs(v) > 1 (including case where v infinite).
        // v**-inf is: inf if abs(v) < 1; 1.0 if abs(v) == 1; 0.0 if abs(v) > 1 (including case where v infinite).
        let av = iv.abs();
        return float(if av == 1.0 {
            1.0
        } else if (iw > 0.0) == (av > 1.0) {
            iw.abs()
        } else {
            0.0
        });
    }
    if iv.is_infinite() {
        // (+-inf)**w is: inf for w positive, 0 for w negative; in both cases, we need to add the appropriate sign
        // if w is an odd integer.
        let odd = is_odd_integer(iw);
        return float(if iw > 0.0 {
            if odd {
                iv
            } else {
                iv.abs()
            }
        } else if odd {
            0.0f64.copysign(iv)
        } else {
            0.0
        });
    }
    if iv == 0.0 {
        // 0**w is: 0 for w positive, 1 for w zero (already dealt with above), and an error if w is negative.
        if iw < 0.0 {
            return zero_division("zero to a negative power");
        }
        // Use correct sign if iw is odd.
        return float(if is_odd_integer(iw) { iv } else { 0.0 });
    }
    let mut base = iv;
    let mut negate = false;
    if iv < 0.0 {
        // Negative numbers raised to fractional powers become complex.
        if iw != iw.floor() {
            return crate::value::complex::real_pow(iv, iw);
        }
        // iw is an exact integer, albeit perhaps a very large one. Replace iv by its absolute value and remember
        // to negate the pow result if iw is odd.
        base = -iv;
        negate = is_odd_integer(iw);
    }
    // 1**w is 1, even 1**inf and 1**nan; (-1) ** large_integer also ends up here.
    if base == 1.0 {
        return float(if negate { -1.0 } else { 1.0 });
    }
    // Now iv and iw are finite, iw is nonzero, and iv is positive and not equal to 1.0. We finally allow the
    // platform pow to step in and do the rest. Overflow raises; underflow to 0 is fine.
    let ix = base.powf(iw);
    if ix.is_infinite() {
        return Err(Error::Overflow("Numerical result out of range".into()));
    }
    float(if negate { -ix } else { ix })
}
