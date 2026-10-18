//! `Die<Value>`'s Python-only methods (population/die.py): `Die.cmp`, `mean_time_to_sum` and `sign`, which compare
//! outcomes with Python's operators. The generic `Die` lives in `crate::die`.

use crate::die::Die;
use crate::error::{invalid_argument, Error, Result};
use crate::outcome::OutcomeArith;

impl Die<crate::Value> {
    /// A `Die` with outcomes 1, -1, and 0 (`Die.cmp`).
    ///
    /// The quantities are equal to the positive outcome of `self > other`, `self < other`, and the remainder
    /// respectively. As in icepool, the three comparison dice are made in the order `<`, `==`, `>` (so the first
    /// that raises decides the error), and each contributes the quantity of its `True` outcome if it has one
    /// (`if True in lt: data[-1] = lt[True]`, a dict lookup, so an outcome equal to `True` such as `1` counts).
    pub fn cmp_die(&self, other: &Self) -> Result<Self> {
        use crate::value::BinOp;
        use crate::Value;
        let truth = Value::Bool(true);
        let mut data = Vec::new();
        // `None` is `==`, which has no `BinOp`.
        for (sign, op) in [(-1, Some(BinOp::Lt)), (0, None), (1, Some(BinOp::Gt))] {
            let compared = self.binary_operator(other, |a, b| match op {
                Some(op) => a.binary(op, b),
                None => a.eq_op(b, false),
            })?;
            if let Some(i) = compared.position(&truth) {
                data.push((Value::from(sign), compared.items()[i].1.clone()));
            }
        }
        Die::from_dict(data)
    }

    /// `Die.mean_time_to_sum(target)`: the mean number of rolls until the cumulative sum is greater or equal to
    /// the target (`ValueError` if this die has negative outcomes, `ZeroDivisionError` if it only rolls 0).
    ///
    /// `cache` is the die's `_mean_time_to_sum_cache` (a `cached_property` starting as `[Fraction(0)]`), the
    /// mean time for each target so far, which this extends up to `target`. Each new entry is
    /// `time_per_effect + self.reroll([0], depth='inf').map(lambda x: self.mean_time_to_sum(i - x)).mean()`:
    /// the rolls until a nonzero outcome, then the time left after it. Python's recursive call finds every
    /// smaller target in the cache; outcomes that are not ints get there as floats or Fractions, which index a
    /// list with `TypeError`, as here.
    pub fn mean_time_to_sum(
        &self,
        cache: &mut Vec<crate::Value>,
        target: &crate::Value,
    ) -> Result<crate::Value> {
        use crate::value::BinOp;
        use crate::Value;
        let zero = Value::from(0);
        // `target = max(target, 0)`: keeps `target` unless `0 > target`.
        let target = if zero.binary(BinOp::Gt, target)?.truthy()? {
            zero.clone()
        } else {
            target.clone()
        };
        let cached = |cache: &[Value], target: &Value| -> Result<Option<Value>> {
            // `if target < len(cache): return cache[target]`.
            if !target
                .binary(BinOp::Lt, &Value::from(cache.len() as i64))?
                .truthy()?
            {
                return Ok(None);
            }
            match list_index(target) {
                Some(i) => Ok(Some(cache[i].clone())),
                None => Err(Error::WrongType(format!(
                    "list indices must be integers or slices, not {}",
                    target.type_name()
                ))),
            }
        };
        if let Some(result) = cached(cache, &target)? {
            return Ok(result);
        }
        if self.min_outcome()?.py_lt(&zero)? {
            return invalid_argument("mean_time_to_sum does not handle negative outcomes.");
        }
        let denominator = self.denominator();
        let effect = &denominator - &self.try_quantity(crate::population::Comparison::Eq, &zero)?;
        if effect.is_zero() {
            return Err(Error::DivisionByZero(format!("Fraction({denominator}, 0)")));
        }
        let time_per_effect = dashu_ratio::RBig::from_parts(denominator.into(), effect);
        // `range(len(cache), target + 1)`.
        let stop = target.binary(BinOp::Add, &Value::from(1))?;
        let stop = match (&stop, list_index(&stop)) {
            (_, Some(stop)) => stop,
            // Python would loop over every target up to this one, for hours.
            (Value::Int(_), None) => {
                return Err(Error::Unported("mean_time_to_sum of a huge target".into()))
            }
            _ => {
                return Err(Error::WrongType(format!(
                    "'{}' object cannot be interpreted as an integer",
                    stop.type_name()
                )))
            }
        };
        // `self.reroll([0], depth='inf')`: the outcomes `in [0]` (`==`, as a list compares) are rerolled.
        let rerolled = self.reroll(
            &self
                .outcomes()
                .map(|o| o.eq_op(&zero, false)?.truthy())
                .collect::<Result<Vec<_>>>()?,
            None,
        )?;
        let mut result = None;
        for i in cache.len()..stop {
            crate::interrupt::poll()?;
            let i_value = Value::from(i as i64);
            // `.map(lambda x: self.mean_time_to_sum(i - x)).mean()`: the mapped Fractions weighted by quantity.
            let mut total = dashu_ratio::RBig::ZERO;
            for (x, quantity) in rerolled.items() {
                let next = i_value.binary(BinOp::Sub, x)?;
                let next = if zero.binary(BinOp::Gt, &next)?.truthy()? {
                    zero.clone()
                } else {
                    next
                };
                // Every `i - x` is below `i`, so cached; one that is not an int raises in the lookup or (NaN, never
                // `<` anything) in `range()` of the recursive call.
                let time = match cached(cache, &next)? {
                    Some(Value::Fraction(f)) => f,
                    _ => {
                        return Err(Error::WrongType(format!(
                            "'{}' object cannot be interpreted as an integer",
                            next.type_name()
                        )))
                    }
                };
                total += time * dashu_ratio::RBig::from(dashu_int::IBig::from(quantity.clone()));
            }
            let mean =
                total / dashu_ratio::RBig::from(dashu_int::IBig::from(rerolled.denominator()));
            let value = Value::Fraction(time_per_effect.clone() + mean);
            cache.push(value.clone());
            result = Some(value);
        }
        // `target >= len(cache)` here, so the loop ran at least once.
        result.ok_or_else(|| Error::InvalidArgument("mean_time_to_sum: empty range".into()))
    }

    /// Outcomes become 1 if greater than `zero()`, -1 if less than `zero()`, and 0 otherwise (`Die.sign`).
    ///
    /// Note that for `float`s, +0.0, -0.0, and nan all become 0. Each outcome is compared with its own zero,
    /// `x * 0` (`Die._sign`), so a tuple compares with `()` and a str with `''`.
    pub fn sign(&self) -> Result<Self> {
        self.unary_operator(|x| {
            let z = x.try_zero()?;
            Ok(crate::Value::from(if x.py_gt(&z)? {
                1
            } else if x.py_lt(&z)? {
                -1
            } else {
                0
            }))
        })
    }
}

/// `x` as a Python list index, if it is one (an `int` or a `bool`) and fits the port's lists: `None` for anything
/// else, which Python rejects with `TypeError` (negative indices never reach here).
fn list_index(x: &crate::Value) -> Option<usize> {
    match x {
        crate::Value::Bool(b) => Some(*b as usize),
        crate::Value::Int(i) => usize::try_from(i).ok(),
        _ => None,
    }
}
