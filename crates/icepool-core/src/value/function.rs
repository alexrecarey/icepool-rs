//! The free functions of icepool `function.py` and `math.py` that only `Value` outcomes have: they take Python
//! objects as arguments (`operator.index`, Python's `min`/`max`, `Vector` outcomes). The generic ones (`d`, `z`,
//! `pointwise`, `sorted_union`) live in `crate::function`.

use dashu_base::{Gcd, UnsignedAbs};
use dashu_int::IBig;

use crate::error::{Error, Result};
use crate::value::builtins::py_min_max;
use crate::value::{BinOp, Vector};
use crate::{Die, Quantity, Value};

/// `math.gcd`'s / `range`'s view of an integer argument (`operator.index`): an `int` (a `bool` is one) or a host
/// object with `__index__`.
fn index_arg(x: &Value) -> Result<IBig> {
    let i = match x {
        Value::Int(i) => Some(i.clone()),
        Value::Bool(b) => Some(IBig::from(*b as u8)),
        Value::Host(h) => h.as_index()?,
        _ => None,
    };
    i.ok_or_else(|| {
        Error::WrongType(format!(
            "'{}' object cannot be interpreted as an integer",
            x.type_name()
        ))
    })
}

/// math.py `weighted_lcm(denominators, weights)`: a minimal scale factor for each denominator so that they are in
/// the ratio given by the weights. If a denominator or its corresponding weight is 0, the corresponding result is
/// 0. The weights are any Python objects (`harmonize_denominators` takes them from the caller): they are compared
/// with `0`, given to `math.gcd` and multiplied as Python would, and `zip` stops at the shorter sequence.
///
/// Python zips the weights with the denominators twice, once for the lcm and once for the results: `weights` is
/// what the first `zip` takes, `weights_again` what the second does (the same for a sequence; for an iterator the
/// second pass gets what the first left, often nothing).
///
/// ```
/// use icepool_core::value::weighted_lcm;
/// use icepool_core::{Quantity, Value};
/// // A d4 and a d6, equally weighted: scale them to 12.
/// let denominators = [Quantity::from(4u8), Quantity::from(6u8)];
/// let weights = [Value::from(1), Value::from(1)];
/// let factors = weighted_lcm(&denominators, &weights, &weights).unwrap();
/// assert_eq!(factors, [Value::from(3), Value::from(2)]);
/// ```
pub fn weighted_lcm(
    denominators: &[Quantity],
    weights: &[Value],
    weights_again: &[Value],
) -> Result<Vec<Value>> {
    let zero = Value::from(0);
    // math.lcm() of nothing is 1.
    let mut denominator_lcm = Quantity::ONE;
    for (d, w) in denominators.iter().zip(weights) {
        if !d.is_zero() && w.binary(BinOp::Gt, &zero)?.truthy()? {
            let w = index_arg(w)?.unsigned_abs();
            let x = d / d.gcd(&w);
            denominator_lcm = crate::math::lcm(&denominator_lcm, &x);
        }
    }
    let denominator_lcm = Value::Int(denominator_lcm.into());
    denominators
        .iter()
        .zip(weights_again)
        .map(|(d, w)| {
            if d.is_zero() {
                Ok(zero.clone())
            } else {
                denominator_lcm
                    .binary(BinOp::Mul, w)?
                    .binary(BinOp::FloorDiv, &Value::Int(d.clone().into()))
            }
        })
        .collect()
}

/// The data of `coin(n, d)` once `n` and `d` are known (a non-`int` numerator has already become a `Fraction`'s
/// numerator and denominator): `{False: min(d - n, d)}` if `n < d`, then `True: min(n, d)` if `n > 0`, as Python
/// `Value`s for `Die(data)` to check (a non-`int` `d` makes non-`int` quantities, which it refuses).
///
/// ```
/// use icepool_core::value::coin_data;
/// use icepool_core::Value;
/// let data = coin_data(&Value::from(1), &Value::from(3)).unwrap();
/// assert_eq!(data, [(Value::Bool(false), Value::from(2)), (Value::Bool(true), Value::from(1))]);
/// ```
pub fn coin_data(n: &Value, d: &Value) -> Result<Vec<(Value, Value)>> {
    let mut data = Vec::new();
    if n.binary(BinOp::Lt, d)?.truthy()? {
        data.push((
            Value::Bool(false),
            Value::py_min(d.binary(BinOp::Sub, n)?, d.clone())?,
        ));
    }
    if n.binary(BinOp::Gt, &Value::from(0))?.truthy()? {
        data.push((Value::Bool(true), Value::py_min(n.clone(), d.clone())?));
    }
    Ok(data)
}

/// Longest `range` the port builds (Python would build any, slowly).
const MAX_RANGE: usize = 1 << 20;

/// `consecutive(*args)`: a minimal sequence of consecutive ints covering the argument sets.
///
/// Python chains the arguments twice, once for `min(..., default=None)` and once for `max`: `first` and `second`
/// are what each chain yields (an iterator argument is exhausted by the first, so the second `max` may raise).
///
/// ```
/// use icepool_core::value::consecutive;
/// use icepool_core::Value;
/// let all = vec![Value::from(4), Value::from(2)];
/// let ints = |r: std::ops::RangeInclusive<i64>| Value::tuple(r.map(Value::from).collect());
/// assert_eq!(consecutive(all.clone(), all).unwrap(), ints(2..=4));
/// assert_eq!(consecutive(vec![], vec![]).unwrap(), Value::tuple(vec![]));
/// ```
pub fn consecutive(first: Vec<Value>, second: Vec<Value>) -> Result<Value> {
    if first.is_empty() {
        return Ok(Value::tuple(vec![]));
    }
    let start = py_min_max(first, false)?;
    let stop = py_min_max(second, true)?.binary(BinOp::Add, &Value::from(1))?;
    // `range(start, stop + 1)`.
    let (start, stop) = (index_arg(&start)?, index_arg(&stop)?);
    if stop > start {
        let len = &stop - &start;
        // `tuple()` asks the range for its length first, and `len()` of a range longer than `sys.maxsize` (64-bit
        // Python) raises OverflowError.
        if len > IBig::from(i64::MAX) {
            return Err(Error::Overflow(
                "Python int too large to convert to C ssize_t".into(),
            ));
        }
        if len > IBig::from(MAX_RANGE) {
            return Err(Error::Unported("consecutive() over a huge range".into()));
        }
    }
    let mut out = Vec::new();
    let mut i = start;
    while i < stop {
        out.push(Value::Int(i.clone()));
        i += IBig::ONE;
    }
    Ok(Value::tuple(out))
}

/// `one_hot(sides)`: a die with `Vector` outcomes with one element set to `True` uniformly at random and the rest
/// `False`. This is an easy (if somewhat expensive) way of representing how many dice in a pool rolled each
/// number: the outcomes of `10 @ one_hot(6)` are the `(ones, twos, threes, fours, fives, sixes)` rolled in 10d6.
/// `range(sides)` takes any index (an empty range gives the empty die).
///
/// ```
/// use icepool_core::value::{one_hot, Vector};
/// use icepool_core::Value;
/// let die = one_hot(&Value::from(3)).unwrap();
/// assert_eq!(die.len(), 3);
/// let first = Value::Vector(Vector::new(vec![Value::Bool(false), Value::Bool(false), Value::Bool(true)]));
/// assert_eq!(die.min_outcome().unwrap(), &first);
/// ```
pub fn one_hot(sides: &Value) -> Result<Die<Value>> {
    let sides = index_arg(sides)?;
    if sides > IBig::from(4096) {
        return Err(Error::Unported("one_hot() with this many sides".into()));
    }
    let sides = usize::try_from(sides).unwrap_or(0);
    let data = (0..sides)
        .map(|i| {
            let outcome = (0..sides).map(|j| Value::Bool(i == j)).collect();
            (Value::Vector(Vector::new(outcome)), Quantity::ONE)
        })
        .collect();
    Die::from_dict(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ints(xs: &[i64]) -> Vec<Value> {
        xs.iter().map(|&x| Value::from(x)).collect()
    }

    fn consecutive_of(xs: &[i64]) -> Result<Value> {
        consecutive(ints(xs), ints(xs))
    }

    #[test]
    fn consecutive_builds_ranges_up_to_the_limit() {
        // Exactly `MAX_RANGE` steps is still built; one more is refused.
        let max = MAX_RANGE as i64;
        match consecutive_of(&[-1, max - 2]).unwrap() {
            Value::Tuple(t) => assert_eq!(t.len(), MAX_RANGE),
            v => panic!("not a tuple: {v}"),
        }
        assert!(matches!(
            consecutive_of(&[-1, max - 1]),
            Err(Error::Unported(_))
        ));
        assert!(matches!(
            consecutive_of(&[0, 1 << 40]),
            Err(Error::Unported(_))
        ));
        // Longer than `sys.maxsize`: `len()` of the range overflows.
        assert!(matches!(
            consecutive_of(&[0, i64::MAX - 1]),
            Err(Error::Unported(_))
        ));
        assert!(matches!(
            consecutive_of(&[-1, i64::MAX - 1]),
            Err(Error::Overflow(_))
        ));
        // Far from zero, only the length counts.
        assert_eq!(
            consecutive_of(&[1 << 40, (1 << 40) + 1]).unwrap(),
            Value::tuple(ints(&[1 << 40, (1 << 40) + 1]))
        );
    }

    #[test]
    fn one_hot_refuses_huge_sides() {
        assert!(matches!(
            one_hot(&Value::from(4097)),
            Err(Error::Unported(_))
        ));
        assert_eq!(one_hot(&Value::from(3)).unwrap().len(), 3);
    }
}
