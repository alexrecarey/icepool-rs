//! `PyDie`: `Die<Value>`'s Python-only methods. `Die.cmp`, `mean_time_to_sum` and `sign` (population/die.py)
//! compare outcomes with Python's operators; the marginals call Python's `[]` and `getattr` on them; the quantity
//! management of population/base.py (`multiply_quantities` to `remove`, `to_one_hot`) takes quantities that may be
//! any Python number. The generic `Die` lives in `crate::die`.

use rustc_hash::FxHashMap;

use crate::die::Die;
use crate::error::{invalid_argument, Error, Result};
use crate::outcome::OutcomeArith;
use crate::population::{check_public_attr, int};
use crate::value::builtins::{getattr, getitem, py_len, Index};
use crate::value::{BinOp, UnOp, Value};
use crate::Quantity;

/// A population's `dict` for `Die(...)`: `(outcome, quantity)` pairs in insertion order.
///
/// ```
/// use icepool_core::value::{Data, PyDie};
/// use icepool_core::{try_d, Die, Value};
///
/// // `d3.append(4, 2)`: the new outcome goes last.
/// let d3: Die<Value> = try_d(3).unwrap();
/// let data: Data = d3.append(&Value::from(4), &Value::from(2)).unwrap();
/// let items: Vec<String> = data.iter().map(|(o, q)| format!("{o}: {q}")).collect();
/// assert_eq!(items, ["1: 1", "2: 1", "3: 1", "4: 2"]);
/// ```
pub type Data = Vec<(Value, Value)>;

/// `data[outcome] = quantity` in a `dict`'s items: an existing key keeps its place (and its spelling), a new one
/// goes last.
fn set_item(data: &mut Data, position: Option<usize>, outcome: &Value, quantity: Value) {
    match position {
        Some(i) => data[i].1 = quantity,
        None => data.push((outcome.clone(), quantity)),
    }
}

/// `Die<Value>`'s Python-only methods: those that compare or compute with Python's operators (`Die.cmp`,
/// `mean_time_to_sum`, `sign`), the marginals, and population/base.py's quantity management, whose quantities may be
/// any Python number. Typed dice have their own versions where one makes sense (`Die::mean_time_to_sum`).
///
/// ```
/// use icepool_core::value::builtins::fraction;
/// use icepool_core::value::PyDie;
/// use icepool_core::{try_d, Die, Value};
///
/// // `d6.cmp(d6)`: -1, 0 and 1 with quantities 15, 6, 15.
/// let d6: Die<Value> = try_d(6).unwrap();
/// let cmp = d6.cmp_die(&d6).unwrap();
/// let items: Vec<String> = cmp.items().iter().map(|(o, q)| format!("{o}: {q}")).collect();
/// assert_eq!(items, ["-1: 15", "0: 6", "1: 15"]);
/// // `d4.mean_time_to_sum(2)` is `Fraction(5, 4)`; the cache starts as `[Fraction(0)]`.
/// let d4: Die<Value> = try_d(4).unwrap();
/// let mut cache = vec![Value::Fraction(fraction(&Value::from(0)).unwrap())];
/// assert_eq!(d4.mean_time_to_sum(&mut cache, &Value::from(2)).unwrap().to_string(), "Fraction(5, 4)");
/// ```
pub trait PyDie {
    /// A `Die` with outcomes 1, -1, and 0 (`Die.cmp`).
    ///
    /// The quantities are equal to the positive outcome of `self > other`, `self < other`, and the remainder
    /// respectively. As in icepool, the three comparison dice are made in the order `<`, `==`, `>` (so the first
    /// that raises decides the error), and each contributes the quantity of its `True` outcome if it has one
    /// (`if True in lt: data[-1] = lt[True]`, a dict lookup, so an outcome equal to `True` such as `1` counts).
    fn cmp_die(&self, other: &Die<Value>) -> Result<Die<Value>>;

    /// `Die.mean_time_to_sum(target)`: the mean number of rolls until the cumulative sum is greater or equal to
    /// the target (`ValueError` if this die has negative outcomes, `ZeroDivisionError` if it only rolls 0).
    ///
    /// `cache` is the die's `_mean_time_to_sum_cache` (a `cached_property` starting as `[Fraction(0)]`), the
    /// mean time for each target so far, which this extends up to `target`. Each new entry is
    /// `time_per_effect + self.reroll([0], depth='inf').map(lambda x: self.mean_time_to_sum(i - x)).mean()`:
    /// the rolls until a nonzero outcome, then the time left after it. Python's recursive call finds every
    /// smaller target in the cache; outcomes that are not ints get there as floats or Fractions, which index a
    /// list with `TypeError`, as here.
    fn mean_time_to_sum(
        &self,
        cache: &mut Vec<crate::Value>,
        target: &crate::Value,
    ) -> Result<crate::Value>;

    /// Outcomes become 1 if greater than `zero()`, -1 if less than `zero()`, and 0 otherwise (`Die.sign`).
    ///
    /// Note that for `float`s, +0.0, -0.0, and nan all become 0. Each outcome is compared with its own zero,
    /// `x * 0` (`Die._sign`), so a tuple compares with `()` and a str with `''`.
    fn sign(&self) -> Result<Die<Value>>;

    /// Multiplies all quantities by an integer (`None`: `scale == 1`, Python returns the population itself).
    fn multiply_quantities(&self, scale: &Value) -> Result<Option<Data>>;

    /// Divides all quantities by an integer, rounding down; resulting zero quantities are dropped (only those at
    /// least `divisor` are kept). `None`: `divisor == 0`, Python returns the population itself.
    fn divide_quantities(&self, divisor: &Value) -> Result<Option<Data>>;

    /// Modulus of all quantities with an integer.
    fn modulo_quantities(&self, divisor: &Value) -> Result<Data>;

    /// Changes the denominator to a target number by changing the quantity of a specified outcome. If that
    /// quantity becomes zero, the outcome is removed (`del data[outcome]`: `KeyError` if it was not there);
    /// `ValueError` if it would have to be negative.
    fn pad_to_denominator(&self, denominator: &Value, outcome: &Value) -> Result<Data>;

    /// `Population.append(outcome, quantity)` up to `self._new_type(data)`: the items of `data = Counter(self)`
    /// after `data[outcome] = max(data[outcome] + quantity, 0)`, an outcome not there yet added last (as the
    /// `dict` does). `quantity` may be negative, which removes quantity (but not below zero); it is any Python
    /// number, and `Die(data)` / `Deck(data)` (the caller's) drops the zero quantities.
    fn append(&self, outcome: &Value, quantity: &Value) -> Result<Data>;

    /// `Population.remove(outcome, quantity)` up to `self._new_type(data)`. Without a quantity all of that
    /// outcome is removed (`data[outcome] = 0`); with one, it is `self.append(outcome, -quantity)`, so a negative
    /// quantity adds instead.
    fn remove(&self, outcome: &Value, quantity: Option<&Value>) -> Result<Data>;

    /// `Population.to_one_hot(outcomes)` given, for each outcome of this population, `outcomes.index(outcome)` if
    /// `outcome in outcomes` (else `None`; the caller asks its sequence, whose `in` and `index` are its own) and
    /// `len(outcomes)`: each outcome becomes a `Vector` of `len` bools, `True` at that index, and equal vectors
    /// add up their quantities (`data[Vector(value)] += quantity`). The index is a Python list index: a negative
    /// one counts from the end, and one out of range is `IndexError`. The caller builds `Die(data)` /
    /// `Deck(data)` of the result (its keys are distinct; zero quantities drop out there).
    fn one_hot_items(
        &self,
        len: usize,
        positions: &[Option<i64>],
    ) -> Result<Vec<(Value, Quantity)>>;

    /// Multiplies all quantities to reach the target denominator; `ValueError` if this cannot be achieved using
    /// an integer scaling.
    fn multiply_to_denominator(&self, denominator: &Value) -> Result<Option<Data>>;

    /// `self.hash_key <op> other.hash_key` for `<`, `<=`, `>`, `>=`: `(Die, tuple(items))` compared as Python
    /// compares tuples. The classes are the same object, so it comes down to the items: the first `(outcome,
    /// quantity)` pair that differs (by `==`) decides, by `op` on the outcomes if those differ (which may raise),
    /// else on the quantities; if one is a prefix of the other, by `op` on the lengths. Like Python's tuple
    /// comparison, it returns what the deciding comparison returned: not a bool for vector outcomes.
    fn hash_key_compare(&self, other: &Die<Value>, op: BinOp) -> Result<Value>;

    /// `marginals[index]`: the `[]` operator applied to each outcome (`_unary_operator(operator.getitem, dims)`).
    fn marginal(&self, index: &Index) -> Result<Die<Value>>;

    /// `marginals.<name>` (`_Marginals.__getattr__`): `getattr(outcome, name)` applied to each outcome. Private
    /// names are refused before any outcome is looked at (`check_public_attr`).
    fn marginal_attr(&self, name: &str) -> Result<Die<Value>>;

    /// `len(marginals)`: the minimum `len()` of all outcomes (`ValueError` for an empty die).
    fn marginals_len(&self) -> Result<usize>;
}

impl PyDie for Die<Value> {
    fn cmp_die(&self, other: &Self) -> Result<Self> {
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

    fn mean_time_to_sum(
        &self,
        cache: &mut Vec<crate::Value>,
        target: &crate::Value,
    ) -> Result<crate::Value> {
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
        let effect = &denominator - &self.try_quantity(crate::Comparison::Eq, &zero)?;
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
        let rerolled = self.reroll_where(
            &self
                .outcomes()
                .map(|o| o.eq_op(&zero, false)?.truthy())
                .collect::<Result<Vec<_>>>()?,
            crate::Depth::Infinite,
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

    fn sign(&self) -> Result<Self> {
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

    fn multiply_quantities(&self, scale: &Value) -> Result<Option<Data>> {
        if *scale == Value::from(1) {
            return Ok(None);
        }
        self.map_quantities(|q| q.binary(BinOp::Mul, scale))
            .map(Some)
    }

    fn divide_quantities(&self, divisor: &Value) -> Result<Option<Data>> {
        if *divisor == Value::from(0) {
            return Ok(None);
        }
        let mut data = Data::new();
        for (o, q) in self.items() {
            let q = int(q);
            if q.binary(BinOp::Ge, divisor)?.truthy()? {
                data.push((o.clone(), q.binary(BinOp::FloorDiv, divisor)?));
            }
        }
        Ok(Some(data))
    }

    fn modulo_quantities(&self, divisor: &Value) -> Result<Data> {
        self.map_quantities(|q| q.binary(BinOp::Mod, divisor))
    }

    fn pad_to_denominator(&self, denominator: &Value, outcome: &Value) -> Result<Data> {
        let adjustment = denominator.binary(BinOp::Sub, &Value::Int(self.den()))?;
        let mut data = self.map_quantities(Ok)?;
        let position = data.iter().position(|(o, _)| o == outcome);
        let current = position.map_or(Value::from(0), |i| data[i].1.clone());
        let new_quantity = current.binary(BinOp::Add, &adjustment)?;
        if new_quantity.binary(BinOp::Gt, &Value::from(0))?.truthy()? {
            match position {
                Some(i) => data[i].1 = new_quantity,
                None => data.push((outcome.clone(), new_quantity)),
            }
        } else if new_quantity == Value::from(0) {
            match position {
                Some(i) => {
                    data.remove(i);
                }
                None => return Err(Error::MissingKey(format!("{outcome}"))),
            }
        } else {
            return invalid_argument(format!(
                "Padding to denominator of {denominator} would require a negative quantity of \
                 {new_quantity} for {outcome}"
            ));
        }
        Ok(data)
    }

    fn append(&self, outcome: &Value, quantity: &Value) -> Result<Data> {
        let mut data = self.map_quantities(Ok)?;
        // `data[outcome]`: a `Counter` gives 0 for a missing key.
        let position = data.iter().position(|(o, _)| o == outcome);
        let current = position.map_or(Value::from(0), |i| data[i].1.clone());
        let mut new_quantity = current.binary(BinOp::Add, quantity)?;
        // `max(x, 0)` keeps `x` unless `0 > x`.
        if Value::from(0).binary(BinOp::Gt, &new_quantity)?.truthy()? {
            new_quantity = Value::from(0);
        }
        set_item(&mut data, position, outcome, new_quantity);
        Ok(data)
    }

    fn remove(&self, outcome: &Value, quantity: Option<&Value>) -> Result<Data> {
        match quantity {
            Some(q) => self.append(outcome, &q.unary(UnOp::Neg)?),
            None => {
                let mut data = self.map_quantities(Ok)?;
                let position = data.iter().position(|(o, _)| o == outcome);
                set_item(&mut data, position, outcome, Value::from(0));
                Ok(data)
            }
        }
    }

    fn one_hot_items(
        &self,
        len: usize,
        positions: &[Option<i64>],
    ) -> Result<Vec<(Value, Quantity)>> {
        let mut data: Vec<(Value, Quantity)> = Vec::new();
        let mut index: FxHashMap<Value, usize> = FxHashMap::default();
        for ((_, quantity), position) in self.items().iter().zip(positions) {
            // `[False] * len(outcomes)`: `len` may be that of a huge sequence (`range(2**40)`).
            let mut value =
                crate::memory::try_repeat(Value::Bool(false), Some(len), "[False] * len")?;
            if let Some(i) = *position {
                let i = if i < 0 { i + len as i64 } else { i };
                match usize::try_from(i).ok().filter(|&i| i < len) {
                    Some(i) => value[i] = Value::Bool(true),
                    None => {
                        return Err(Error::IndexOutOfRange(
                            "list assignment index out of range".into(),
                        ))
                    }
                }
            }
            let key = Value::Vector(crate::value::Vector::new(value));
            match index.get(&key) {
                Some(&i) => data[i].1 += quantity,
                None => {
                    index.insert(key.clone(), data.len());
                    data.push((key, quantity.clone()));
                }
            }
        }
        Ok(data)
    }

    fn multiply_to_denominator(&self, denominator: &Value) -> Result<Option<Data>> {
        let den = Value::Int(self.den());
        if denominator.binary(BinOp::Mod, &den)?.truthy()? {
            return invalid_argument(
                "Target denominator is not an integer factor of the current denominator.",
            );
        }
        self.multiply_quantities(&denominator.binary(BinOp::FloorDiv, &den)?)
    }

    fn hash_key_compare(&self, other: &Die<Value>, op: BinOp) -> Result<Value> {
        fn cmp<T: PartialOrd>(op: BinOp, a: T, b: T) -> bool {
            match op {
                BinOp::Lt => a < b,
                BinOp::Le => a <= b,
                BinOp::Gt => a > b,
                _ => a >= b,
            }
        }
        for ((oa, qa), (ob, qb)) in self.items().iter().zip(other.items()) {
            // Python's tuple comparison skips items that are `==` (or identical, which dict equality stands in
            // for): a `Symbols` with only zero counts is `== ()`, though it is no dict key equal to it.
            if oa != ob && !oa.eq_op(ob, false)?.truthy()? {
                return oa.binary(op, ob);
            }
            if qa != qb {
                return Ok(Value::Bool(cmp(op, qa, qb)));
            }
        }
        Ok(Value::Bool(cmp(op, self.len(), other.len())))
    }

    fn marginal(&self, index: &Index) -> Result<Die<Value>> {
        self.unary_operator(|o| getitem(o, index))
    }

    fn marginal_attr(&self, name: &str) -> Result<Die<Value>> {
        check_public_attr(name)?;
        self.unary_operator(|o| getattr(o, name))
    }

    fn marginals_len(&self) -> Result<usize> {
        let mut min: Option<usize> = None;
        for o in self.outcomes() {
            let n = py_len(o)?;
            min = Some(min.map_or(n, |m| m.min(n)));
        }
        min.ok_or_else(|| Error::InvalidArgument("min() iterable argument is empty".into()))
    }
}

impl Die<Value> {
    /// Each `(outcome, f(quantity))`, zero quantities included.
    fn map_quantities(&self, f: impl Fn(Value) -> Result<Value>) -> Result<Data> {
        self.items()
            .iter()
            .map(|(o, q)| Ok((o.clone(), f(int(q))?)))
            .collect()
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
