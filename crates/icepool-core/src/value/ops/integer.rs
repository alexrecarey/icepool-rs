//! `int` arithmetic as CPython's `Objects/longobject.c` does it, and the integer helpers of `round`.

use super::float::float_pow;
use super::{too_large, zero_division, BinOp, MAX_INT_BITS};
use crate::error::{Error, Result};
use crate::value::{int_to_f64, ratio_to_f64, Value};
use dashu_base::{BitTest, DivRem, UnsignedAbs};
use dashu_int::IBig;
use dashu_ratio::RBig;
use std::cmp::Ordering;

/// `10 ** abs(n)`.
pub(super) fn pow10(n: &IBig) -> Result<IBig> {
    let k =
        usize::try_from(n.unsigned_abs()).map_err(|_| Error::Overflow("too many digits".into()))?;
    if k as u64 > MAX_INT_BITS / 4 {
        return too_large();
    }
    Ok(IBig::from(10u8).pow(k))
}

/// `n / d` rounded to the nearest integer, ties to even (`d > 0`).
pub(super) fn round_half_even(n: &IBig, d: &IBig) -> IBig {
    let (q, r) = floor_divmod(n, d);
    let twice: IBig = r * IBig::from(2u8);
    match twice.cmp(d) {
        Ordering::Less => q,
        Ordering::Greater => q + IBig::ONE,
        Ordering::Equal if (&q % IBig::from(2u8)) == IBig::ZERO => q,
        Ordering::Equal => q + IBig::ONE,
    }
}

/// Python's `divmod` on ints: the quotient rounds toward minus infinity, the remainder takes the divisor's sign.
pub(crate) fn floor_divmod(a: &IBig, b: &IBig) -> (IBig, IBig) {
    let (mut q, mut r) = a.clone().div_rem(b.clone());
    if r != IBig::ZERO && ((r < IBig::ZERO) != (*b < IBig::ZERO)) {
        q -= IBig::ONE;
        r += b;
    }
    (q, r)
}

/// `int(f)` for an integral float: `OverflowError` for infinities, `ValueError` for NaN.
pub(crate) fn float_to_int(f: f64) -> Result<IBig> {
    if f.is_nan() {
        return Err(Error::InvalidArgument(
            "cannot convert float NaN to integer".into(),
        ));
    }
    if f.is_infinite() {
        return Err(Error::Overflow(
            "cannot convert float infinity to integer".into(),
        ));
    }
    Ok(RBig::try_from(f).expect("finite").trunc())
}

/// `a ** b` for ints with `b >= 0`.
pub(super) fn int_pow(a: &IBig, b: &IBig) -> Result<IBig> {
    // 0, 1 and -1 stay small whatever the exponent.
    if a.unsigned_abs() <= dashu_int::UBig::ONE {
        let odd = (b % IBig::from(2u8)) != IBig::ZERO;
        return Ok(if *a == IBig::ZERO && *b != IBig::ZERO {
            IBig::ZERO
        } else if *a < IBig::ZERO && odd {
            -IBig::ONE
        } else {
            IBig::ONE
        });
    }
    match usize::try_from(b.clone()) {
        Ok(e) if (e as u64).saturating_mul(a.unsigned_abs().bit_len() as u64) <= MAX_INT_BITS => {
            Ok(a.pow(e))
        }
        _ => too_large(),
    }
}

pub(super) fn int_op(op: BinOp, a: &IBig, b: &IBig) -> Result<Value> {
    use BinOp::*;
    let zero = IBig::ZERO;
    Ok(Value::Int(match op {
        Add => a + b,
        Sub => a - b,
        Mul => a * b,
        TrueDiv => return Ok(Value::Float(ratio_to_f64(a, b)?)),
        FloorDiv | Mod => {
            if *b == zero {
                return zero_division("integer division or modulo by zero");
            }
            let (q, r) = floor_divmod(a, b);
            if op == FloorDiv {
                q
            } else {
                r
            }
        }
        // A negative exponent makes it a float operation (`long_pow` hands it to `float_pow`).
        Pow if *b < zero => return float_pow(int_to_f64(a)?, int_to_f64(b)?),
        Pow => int_pow(a, b)?,
        LShift | RShift if *b < zero => {
            return Err(Error::InvalidArgument("negative shift count".into()))
        }
        LShift if *a == zero => zero,
        LShift => match usize::try_from(b.clone()) {
            Ok(s)
                if (s as u64).saturating_add(a.unsigned_abs().bit_len() as u64) <= MAX_INT_BITS =>
            {
                a << s
            }
            _ => return too_large(),
        },
        // Shifting right by more than the length leaves 0, or -1 for a negative number (rounds toward -inf).
        RShift => match usize::try_from(b.clone()) {
            Ok(s) if s <= a.unsigned_abs().bit_len() => a >> s,
            _ if *a < zero => -IBig::ONE,
            _ => zero,
        },
        And => a & b,
        Or => a | b,
        Xor => a ^ b,
        Lt | Le | Gt | Ge => unreachable!("comparisons are handled by compare"),
    }))
}
