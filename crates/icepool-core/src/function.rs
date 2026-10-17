//! Port of icepool `function.py` (free functions).

use dashu_base::{Gcd, UnsignedAbs};
use dashu_int::IBig;

use crate::counts::Counts;
use crate::die::Die;
use crate::error::{value_error, Error, Result};
use crate::memory::try_collect;
use crate::outcome::{ErrInto, OutcomeOrder};
use crate::value::{BinOp, Vector};
use crate::{Quantity, Value};

/// A standard die, uniformly distributed from `1` to `sides` inclusive.
///
/// Don't confuse this with `Die::new`: `Die([6])` always rolls the integer 6, `d(6)` is a d6. (Python's
/// `TypeError` for a non-`int` `sides` is the facade's job.)
pub fn d<O: OutcomeOrder + From<i64>>(sides: i64) -> Result<Die<O>> {
    if sides < 1 {
        return value_error("sides must be at least 1.");
    }
    // `range(1, sides + 1)` as a list: Python's `MemoryError` where no machine holds it (`memory`).
    let items = try_collect(Some(sides), (1..=sides).map(|i| (O::from(i), Quantity::ONE)), "d(sides)")?;
    Ok(Die::from_counts(Counts::from_sorted(items)))
}

/// Merge sets into a sorted sequence (`sorted(set.union(*(set(arg) for arg in args)))`).
///
/// An outcome keeps the spelling of the first argument that has it (`set.union` adds later equal elements to the
/// copy of the first set as no-ops). Python sorts the set in hash order, the port in first-seen order: the same
/// result whenever the outcomes are totally ordered (see NOTES.md for the NaN / partially comparable gap).
pub fn sorted_union<'a, O, I>(args: impl IntoIterator<Item = I>) -> Result<Vec<O>>
where
    O: OutcomeOrder + 'a,
    I: IntoIterator<Item = &'a O>,
{
    let mut seen = rustc_hash::FxHashSet::default();
    let mut union = Vec::new();
    for arg in args {
        for outcome in arg {
            if seen.insert(outcome.clone()) {
                union.push(outcome.clone());
            }
        }
    }
    crate::outcome::sorted(union)
}

/// A die uniformly distributed from `0` to `sides - 1` inclusive (`z`). Equal to `d(sides) - 1`.
pub fn z<O: OutcomeOrder + From<i64>>(sides: i64) -> Result<Die<O>> {
    if sides < 1 {
        return value_error("sides must be at least 1.");
    }
    let items = try_collect(Some(sides), (0..sides).map(|i| (O::from(i), Quantity::ONE)), "z(sides)")?;
    Ok(Die::from_counts(Counts::from_sorted(items)))
}

/// `pointwise_max` (`max` true) / `pointwise_min` once the dice are harmonized (`harmonize_denominators`).
///
/// pointwise_max selects the highest chance of rolling >= each outcome among the arguments: for each outcome, the
/// chance of the result rolling >= that outcome is the same as the highest chance of rolling >= that outcome
/// among the arguments (equivalently, any quantile in the result is the highest of that quantile among the
/// arguments). So its cumulative quantity `<=` each outcome is the *least* of the dice's; pointwise_min takes the
/// greatest. Each die's cumulative quantities never decrease, so neither do their min / max.
pub fn pointwise<O: OutcomeOrder>(dice: &[Die<O>], max: bool) -> Result<Die<O>> {
    let outcomes = sorted_union(dice.iter().map(Die::outcomes))?;
    let mut cumulative = Vec::with_capacity(outcomes.len());
    for outcome in &outcomes {
        let mut best: Option<Quantity> = None;
        for die in dice {
            let q = die.try_quantity_le(outcome).err_into()?;
            // `min(...)` / `max(...)` of ints: which of equal ones is kept does not matter.
            best = Some(match best {
                Some(b) if (max && q < b) || (!max && q > b) => q,
                Some(b) => b,
                None => q,
            });
        }
        cumulative.push(best.expect("an outcome comes from some die"));
    }
    crate::die::from_cumulative(outcomes, cumulative, false).err_into()
}

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
        Error::Type(format!(
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

/// Python's `min(iterable)` (`max` false) or `max(iterable)`: the first of the least (greatest) elements, comparing
/// each `item < best` (`item > best`); an empty iterable raises ValueError.
pub fn py_min_max(items: Vec<Value>, max: bool) -> Result<Value> {
    let mut items = items.into_iter();
    let Some(mut best) = items.next() else {
        let f = if max { "max" } else { "min" };
        return value_error(format!("{f}() iterable argument is empty"));
    };
    for item in items {
        best = if max {
            Value::py_max(best, item)?
        } else {
            Value::py_min(best, item)?
        };
    }
    Ok(best)
}

/// Longest `range` the port builds (Python would build any, slowly).
const MAX_RANGE: usize = 1 << 20;

/// `consecutive(*args)`: a minimal sequence of consecutive ints covering the argument sets.
///
/// Python chains the arguments twice, once for `min(..., default=None)` and once for `max`: `first` and `second`
/// are what each chain yields (an iterator argument is exhausted by the first, so the second `max` may raise).
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
            return Err(Error::Unsupported("consecutive() over a huge range".into()));
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
pub fn one_hot(sides: &Value) -> Result<Die<Value>> {
    let sides = index_arg(sides)?;
    if sides > IBig::from(4096) {
        return Err(Error::Unsupported("one_hot() with this many sides".into()));
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
    fn huge_standard_dice_are_memory_errors() {
        // Fixtures only see that both sides raise: Python's type here is MemoryError, not an abort or a panic.
        for sides in [i64::MAX, 1 << 40] {
            assert!(matches!(d::<Value>(sides), Err(Error::Memory(_))), "{sides}");
            assert!(matches!(z::<Value>(sides), Err(Error::Memory(_))), "{sides}");
        }
        assert_eq!(d::<Value>(3).unwrap().len(), 3);
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
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            consecutive_of(&[0, 1 << 40]),
            Err(Error::Unsupported(_))
        ));
        // Longer than `sys.maxsize`: `len()` of the range overflows.
        assert!(matches!(
            consecutive_of(&[0, i64::MAX - 1]),
            Err(Error::Unsupported(_))
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
            Err(Error::Unsupported(_))
        ));
        assert_eq!(one_hot(&Value::from(3)).unwrap().len(), 3);
    }
}
