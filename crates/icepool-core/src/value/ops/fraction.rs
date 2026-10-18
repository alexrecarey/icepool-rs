//! `Fraction` arithmetic as `fractions.py` does it.

use super::float::float_pow;
use super::integer::{floor_divmod, int_pow};
use super::{zero_division, BinOp, N};
use crate::error::Result;
use crate::value::Value;
use dashu_base::UnsignedAbs;
use dashu_int::IBig;
use dashu_ratio::RBig;

/// Fraction arithmetic (`fractions.py`), for an `int` or `Fraction` with a `Fraction`.
pub(super) fn frac_op(op: BinOp, a: N, b: N) -> Result<Value> {
    use BinOp::*;
    if op == Pow {
        return frac_pow(a, b);
    }
    let (x, y) = (a.to_ratio(), b.to_ratio());
    // Numerators and denominators as `fractions.py` names them.
    let (na, da) = (x.numerator(), IBig::from(x.denominator().clone()));
    let (nb, db) = (y.numerator(), IBig::from(y.denominator().clone()));
    Ok(match op {
        Add => Value::Fraction(x + y),
        Sub => Value::Fraction(x - y),
        Mul => Value::Fraction(x * y),
        TrueDiv if *nb == IBig::ZERO => return zero_division("Fraction(n, 0)"),
        TrueDiv => Value::Fraction(x / y),
        FloorDiv | Mod if *nb == IBig::ZERO => {
            return zero_division("integer division or modulo by zero")
        }
        // `(a.numerator * b.denominator) // (a.denominator * b.numerator)`: an int.
        FloorDiv => Value::Int(floor_divmod(&(na * &db), &(&da * nb)).0),
        // `Fraction((a.numerator * db) % (b.numerator * da), da * db)`.
        Mod => Value::Fraction(RBig::from_parts_signed(
            floor_divmod(&(na * &db), &(nb * &da)).1,
            da * db,
        )),
        _ => unreachable!("bitwise operators and comparisons never reach frac_op"),
    })
}

/// `**` where an operand is a `Fraction` (and none is a float): `Fraction.__pow__` and `__rpow__`.
fn frac_pow(a: N, b: N) -> Result<Value> {
    // `int ** Fraction`: "If a is an int, keep it that way if possible."
    if let (N::Int(a), N::Frac(b)) = (&a, &b) {
        if b.denominator().is_one() && *b.numerator() >= IBig::ZERO {
            return Ok(Value::Int(int_pow(a, b.numerator())?));
        }
    }
    let (x, y) = (a.to_ratio(), b.to_ratio());
    if !y.denominator().is_one() {
        // A fractional power will generally produce an irrational number.
        return float_pow(N::Frac(x).to_f64()?, N::Frac(y).to_f64()?);
    }
    let power = y.numerator();
    let (n, d) = (x.numerator(), IBig::from(x.denominator().clone()));
    let k = IBig::from(power.unsigned_abs());
    Ok(Value::Fraction(if *power >= IBig::ZERO {
        RBig::from_parts_signed(int_pow(n, &k)?, int_pow(&d, &k)?)
    } else if *n == IBig::ZERO {
        return zero_division("Fraction(n, 0)");
    } else {
        // The sign moves to the numerator.
        RBig::from_parts_signed(int_pow(&d, &k)?, int_pow(n, &k)?)
    }))
}
