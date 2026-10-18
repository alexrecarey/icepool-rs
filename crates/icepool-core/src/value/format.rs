//! The Python side of population/format.py: how a `Value` prints in a table (`FormatOutcome` with Python's `str`,
//! `repr`, `len` and `[]`), and `format_probability_inverse` ("1 in N") on any Python number. The tables themselves
//! are `crate::format`'s, generic over the outcome type.

use dashu_int::IBig;

use crate::error::{Error, Result};
use crate::format::FormatOutcome;
use crate::host::LenKind;
use crate::value::builtins::{float_ceil, getitem, math_log10, Index};
use crate::value::pystr::{format_float, py_repr, py_str, FloatFormat};
use crate::value::BinOp;
use crate::Value;

/// Python's own `str()`, `repr()`, `len()` and `[]`.
impl FormatOutcome for Value {
    fn fmt_outcome(&self) -> Result<String> {
        py_str(self)
    }

    fn repr_outcome(&self) -> Result<String> {
        py_repr(self)
    }

    /// Strs, tuples and vectors have lengths, a host object says what it has, `Symbols` is a mapping.
    fn outcome_len(&self) -> Result<LenKind> {
        Ok(match self {
            Value::Tuple(t) => LenKind::Len(t.len()),
            Value::Str(s) => LenKind::Len(s.chars().count()),
            Value::Host(h) => h.len_kind()?,
            Value::Vector(v) => LenKind::Len(v.data()?.len()),
            Value::Symbols(_) => LenKind::Mapping,
            _ => LenKind::Unsized,
        })
    }

    /// `str(outcome[i])`: Python's `[]`, which raises for an outcome without items.
    fn fmt_part(&self, i: usize) -> Result<String> {
        py_str(&getitem(self, &Index::Item(Value::from(i as i64)))?)
    }
}

/// EXPERIMENTAL: Formats the inverse of a value as "1 in N".
///
/// If N = 1 / probability is between `int_start` and 1 million times `int_start` it will be formatted as an
/// integer. Otherwise it be formatted as a float with precision at least 1 part in `int_start`.
///
/// Every step is Python's operator on the given objects (`Value::binary`), so any number type behaves as there.
///
/// ```
/// use icepool_core::value::format_probability_inverse;
/// use icepool_core::Value;
/// let ten = Value::from(10);
/// assert_eq!(format_probability_inverse(&Value::Float(0.25), &ten).unwrap(), "1 in 4.0");
/// assert_eq!(format_probability_inverse(&Value::from(0), &ten).unwrap(), "n/a");
/// ```
pub fn format_probability_inverse(probability: &Value, int_start: &Value) -> Result<String> {
    let max_precision = float_ceil(math_log10(int_start)?)?;
    let cmp = |op, x: &Value, y: i64| -> Result<bool> { x.binary(op, &Value::from(y))?.truthy() };
    if cmp(BinOp::Le, probability, 0)? || cmp(BinOp::Gt, probability, 1)? {
        return Ok("n/a".into());
    }
    let product = probability.binary(BinOp::Mul, int_start)?;
    let inverse = || Value::Float(1.0).binary(BinOp::TrueDiv, probability);
    if cmp(BinOp::Le, &product, 1)? {
        let scaled = product.binary(BinOp::Mul, &Value::from(1_000_000))?;
        if cmp(BinOp::Le, &scaled, 1)? {
            return Ok(format!(
                "1 in {}",
                format_value(&inverse()?, FloatFormat::Exponent, &max_precision)?
            ));
        } else {
            let n = Value::from(1)
                .binary(BinOp::TrueDiv, probability)?
                .round(&Value::None)?;
            return Ok(format!("1 in {}", py_str(&n)?));
        }
    }

    let mut precision = IBig::ZERO;
    let mut precision_factor = Value::from(1);
    while cmp_values(BinOp::Gt, &product, &precision_factor)? && precision < max_precision {
        precision += IBig::ONE;
        precision_factor = precision_factor.binary(BinOp::Mul, &Value::from(10))?;
    }
    Ok(format!(
        "1 in {}",
        format_value(&inverse()?, FloatFormat::Fixed, &precision)?
    ))
}

fn cmp_values(op: BinOp, a: &Value, b: &Value) -> Result<bool> {
    a.binary(op, b)?.truthy()
}

/// `f'{x:<.{precision}{kind}}'` for the float `x` (no width, so `<` changes nothing). A negative precision makes
/// the spec unparsable (`'<.-2e'`), a `ValueError` in Python.
fn format_value(x: &Value, kind: FloatFormat, precision: &IBig) -> Result<String> {
    let Value::Float(x) = x else {
        return Err(Error::Unported(format!(
            "format() of a {} in format_probability_inverse",
            x.type_name()
        )));
    };
    if *precision < IBig::ZERO {
        return Err(Error::InvalidArgument(
            "Format specifier missing precision".into(),
        ));
    }
    match usize::try_from(precision) {
        Ok(p) if p <= 10_000 => Ok(format_float(*x, kind, p)),
        _ => Err(Error::Unported("a huge format precision".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse() {
        let f = |n: i64, d: u64| Value::Fraction(dashu_ratio::RBig::from_parts(n.into(), d.into()));
        let twenty = Value::from(20);
        assert_eq!(
            format_probability_inverse(&f(1, 6), &twenty).unwrap(),
            "1 in 6.0"
        );
        assert_eq!(
            format_probability_inverse(&f(1, 100), &twenty).unwrap(),
            "1 in 100"
        );
        assert_eq!(
            format_probability_inverse(&f(1, 100_000_000), &twenty).unwrap(),
            "1 in 1.00e+08"
        );
        assert_eq!(
            format_probability_inverse(&f(0, 1), &twenty).unwrap(),
            "n/a"
        );
    }
}
