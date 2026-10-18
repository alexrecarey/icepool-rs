//! The free functions of function.py (`z`, `icepool.dN` / `zN`, `coin`, `stochastic_round`, `one_hot`,
//! `pointwise_max/min`, `min/max_outcome`, `consecutive`, `sorted_union`, `harmonize_denominators`, `from_rv`),
//! map_tools/function.py `reduce` / `accumulate`, and wallenius.py. Nodes in harness/grammar/function.py.

use std::sync::Arc;

use dashu_int::IBig;
use rustc_hash::FxHashMap;

use super::die_methods::from_cumulative;
use super::expression::{add_inner, as_expression, MExpr};
use super::map::{die_of, the_die};
use super::ops::{binary, d, py_iter, to_die, unhashable_list, weights_die, wrong_type};
use super::statistics::new_or_self;
use super::{raise, PErr, PValue};
use icepool_core::function;
use icepool_core::value::PyDie;
use icepool_core::value::builtins::{fraction, limit_denominator, py_min_max};
use icepool_core::value::function as value_function;
use icepool_core::value::{BinOp, UnOp};
use icepool_core::Error;
use icepool_core::{Die, Quantity, Value};

fn value(x: PValue) -> Result<Value, PErr> {
    match x {
        PValue::Value(v) => Ok(v),
        _ => Err(PErr::Unimplemented(
            "a die or other object where a plain value is expected".into(),
        )),
    }
}

fn values(xs: Vec<PValue>) -> Result<Vec<Value>, PErr> {
    xs.into_iter().map(value).collect()
}

fn flag(x: &PValue) -> Result<bool, PErr> {
    super::catalogue::truthy(x)
}

/// `icepool.z(sides)`: `sides` must be an `int` (a `bool` is one).
pub fn z(sides: PValue) -> Result<PValue, PErr> {
    let sides = match sides {
        PValue::Value(Value::Int(i)) => i,
        PValue::Value(Value::Bool(b)) => IBig::from(b as u8),
        _ => return wrong_type("sides must be an int."),
    };
    let sides =
        i64::try_from(sides).map_err(|_| PErr::Raise(Error::Overflow("sides too large".into())))?;
    Ok(PValue::Die(function::try_z(sides)?))
}

/// Python's `int(s)` for a str: optional whitespace, sign, then decimal digits with single underscores between
/// them (`int(' +1_0 ')` is 10). `None` where `int` raises ValueError. Non-ASCII digits and spaces (which Python
/// also accepts) are not modelled: `Err`.
fn parse_int(s: &str) -> Result<Option<IBig>, PErr> {
    if !s.is_ascii() {
        return Err(PErr::Unimplemented("int() of a non-ASCII str".into()));
    }
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    let (negative, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let bytes = digits.as_bytes();
    let valid = !bytes.is_empty()
        && bytes[0].is_ascii_digit()
        && bytes[bytes.len() - 1].is_ascii_digit()
        && bytes.iter().all(|b| b.is_ascii_digit() || *b == b'_')
        && !digits.contains("__");
    if !valid {
        return Ok(None);
    }
    let i: IBig = digits.replace('_', "").parse().expect("validated digits");
    Ok(Some(if negative { -i } else { i }))
}

/// `getattr(icepool, name)` for a name icepool does not define, which goes to function.py's module `__getattr__`:
/// `d<int>` / `z<int>` give that die; a `ValueError` (from `int()` or from a bad number of sides) is swallowed
/// into `AttributeError`. An empty name fails on `key[0]` with IndexError.
pub fn dz_attr(name: PValue) -> Result<PValue, PErr> {
    let Value::Str(name) = value(name)? else {
        return wrong_type("attribute name must be string");
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return raise(Error::IndexOutOfRange("string index out of range".into()));
    };
    let attribute_error = || raise(Error::MissingAttribute(name.to_string()));
    if first != 'd' && first != 'z' {
        return attribute_error();
    }
    let Some(sides) = parse_int(chars.as_str())? else {
        return attribute_error();
    };
    if sides < IBig::ONE {
        // `d(sides)` raises ValueError, swallowed by the `except ValueError`.
        return attribute_error();
    }
    let sides = PValue::Value(Value::Int(sides));
    if first == 'd' {
        d(sides)
    } else {
        z(sides)
    }
}

/// `icepool.coin(n, d, max_denominator=max_denominator)`: a die that rolls `True` with probability `n / d`, and
/// `False` otherwise. If `n <= 0` or `n >= d` the result will have only one outcome. A non-`int` numerator is a
/// probability (`Fraction(n)`, optionally `limit_denominator`ed), and then `d` must be `1`.
pub fn coin(n: PValue, d: PValue, max_denominator: PValue) -> Result<PValue, PErr> {
    let (mut n, mut d) = (value(n)?, value(d)?);
    if !matches!(n, Value::Int(_) | Value::Bool(_)) {
        if d.eq_op(&Value::from(1), true)?.truthy()? {
            return raise(Error::InvalidArgument(
                "If a non-int numerator is provided, a denominator must not be provided.".into(),
            ));
        }
        let mut f = fraction(&n)?;
        match value(max_denominator)? {
            Value::None => {}
            Value::Int(m) => f = limit_denominator(&f, &m)?,
            Value::Bool(b) => f = limit_denominator(&f, &IBig::from(b as u8))?,
            _ => {
                return Err(PErr::Unimplemented(
                    "limit_denominator with a non-int bound".into(),
                ))
            }
        }
        n = Value::Int(f.numerator().clone());
        d = Value::Int(IBig::from(f.denominator().clone()));
    }
    let (keys, quantities): (Vec<_>, Vec<_>) = value_function::coin_data(&n, &d)?
        .into_iter()
        .map(|(k, q)| (PValue::Value(k), PValue::Value(q)))
        .unzip();
    weights_die(keys, quantities)
}

/// `icepool.stochastic_round(x, max_denominator=max_denominator)`: rounds `x` up with probability `x - floor(x)`
/// and down otherwise, producing a die with up to two outcomes (`floor(x) + coin(x - floor(x))`).
pub fn stochastic_round(x: PValue, max_denominator: PValue) -> Result<PValue, PErr> {
    let x = value(x)?;
    let integer_part = x.unary(UnOp::Floor)?;
    let fractional_part = x.binary(BinOp::Sub, &integer_part)?;
    let coin = coin(
        PValue::Value(fractional_part),
        PValue::Value(Value::from(1)),
        max_denominator,
    )?;
    binary(BinOp::Add, PValue::Value(integer_part), coin)
}

/// `die.stochastic_round(max_denominator=m)`: `die.map(lambda x: icepool.stochastic_round(x, max_denominator=m))`.
pub fn die_stochastic_round(die: PValue, max_denominator: PValue) -> Result<PValue, PErr> {
    let f = PValue::Func(Arc::new(super::catalogue::Func::StochasticRound(
        max_denominator,
    )));
    // Only dice have the method (a `DieWithTruth` maps as a plain die).
    let die = PValue::Die(the_die(die)?);
    super::map::map(
        f,
        PValue::List(vec![die]),
        PValue::Value(Value::None),
        vec![],
    )
}

/// `icepool.one_hot(sides)`.
pub fn one_hot(sides: PValue) -> Result<PValue, PErr> {
    Ok(PValue::Die(value_function::one_hot(&value(sides)?)?))
}

/// `harmonize_denominators(dice, weights)`: the dice (single outcomes become dice) with their quantities scaled
/// so that the denominators are proportional to the weights (all equal when `weights` is `None`: `[1] *
/// len(dice)`). `zip` stops at the shorter of the dice and the weights.
pub(super) fn harmonize(dice: PValue, weights: PValue) -> Result<Vec<Die<Value>>, PErr> {
    let dice = py_iter(dice)?
        .into_iter()
        .map(to_die)
        .collect::<Result<Vec<_>, _>>()?;
    let weights = match weights {
        PValue::Value(Value::None) => vec![Value::from(1); dice.len()],
        w => values(py_iter(w)?)?,
    };
    let denominators: Vec<Quantity> = dice.iter().map(Die::denominator).collect();
    let scale_factors = value_function::weighted_lcm(&denominators, &weights, &weights)?;
    dice.into_iter()
        .zip(scale_factors)
        .map(|(die, scale)| {
            let data = die.multiply_quantities(&scale)?;
            the_die(new_or_self(die, data)?)
        })
        .collect()
}

/// `icepool.harmonize_denominators(dice)` (`weights` `None`) or `(dice, weights)`: a tuple of dice.
pub fn harmonize_denominators(dice: PValue, weights: PValue) -> Result<PValue, PErr> {
    Ok(PValue::tuple(
        harmonize(dice, weights)?
            .into_iter()
            .map(PValue::Die)
            .collect(),
    ))
}

/// `pointwise_max(*args)` / `pointwise_min(*args)` if `spread`, else `pointwise_max(args)`: with one argument it
/// is the iterable of dice (so `pointwise_max(d6)` is over the outcomes of the d6, each its own die).
pub fn pointwise(args: PValue, max: PValue, spread: PValue) -> Result<PValue, PErr> {
    let max = flag(&max)?;
    let args = if flag(&spread)? {
        let mut args = py_iter(args)?;
        match args.len() {
            0 => {
                return wrong_type("pointwise_max() missing 1 required positional argument: 'arg0'")
            }
            1 => args.pop().expect("one"),
            _ => PValue::List(args),
        }
    } else {
        args
    };
    let dice = harmonize(args, PValue::Value(Value::None))?;
    Ok(PValue::Die(function::pointwise(&dice, max)?))
}

/// `_iter_outcomes(*args)`: a single argument is iterated instead; each population yields its outcomes, anything
/// else itself.
fn iter_outcomes(args: Vec<PValue>) -> Result<Vec<Value>, PErr> {
    let args = match <[PValue; 1]>::try_from(args) {
        Ok([one]) => py_iter(one)?,
        Err(args) => args,
    };
    let mut out = Vec::new();
    for arg in args {
        match arg {
            PValue::Die(_) | PValue::DieWithTruth(..) | PValue::Deck(_) => {
                out.extend(values(py_iter(arg)?)?)
            }
            arg => out.push(value(arg)?),
        }
    }
    Ok(out)
}

/// `min_outcome(*args)` / `max_outcome(*args)` if `spread`, else with the one argument `args`: the least
/// (greatest) outcome among the populations and single outcomes, by Python's `min` / `max`.
pub fn extreme_outcome(args: PValue, max: PValue, spread: PValue) -> Result<PValue, PErr> {
    let args = if flag(&spread)? {
        py_iter(args)?
    } else {
        vec![args]
    };
    let outcomes = iter_outcomes(args)?;
    Ok(PValue::Value(py_min_max(outcomes, flag(&max)?)?))
}

/// The elements of `itertools.chain(*args)`.
fn chain(args: PValue) -> Result<Vec<Value>, PErr> {
    let mut out = Vec::new();
    for arg in py_iter(args)? {
        out.extend(values(py_iter(arg)?)?);
    }
    Ok(out)
}

/// `icepool.consecutive(*args)`: a minimal sequence of consecutive ints covering the argument sets (`()` if there
/// is nothing). The arguments are lists here, so both chains see the same elements.
pub fn consecutive(args: PValue) -> Result<PValue, PErr> {
    let all = chain(args)?;
    Ok(PValue::Value(value_function::consecutive(all.clone(), all)?))
}

/// `icepool.sorted_union(*args)`: merge sets into a sorted sequence.
pub fn sorted_union(args: PValue) -> Result<PValue, PErr> {
    let mut sets = Vec::new();
    for arg in py_iter(args)? {
        let mut set = Vec::new();
        for x in py_iter(arg)? {
            match x {
                PValue::List(_) => return unhashable_list(),
                x => {
                    let x = value(x)?;
                    x.check_hashable()?;
                    set.push(x);
                }
            }
        }
        sets.push(set);
    }
    Ok(PValue::Value(Value::tuple(function::sorted_union(
        sets.iter().map(|s| s.iter()),
    )?)))
}

/// The dice `reduce` / `accumulate` start with: `initial` converted to a die if it is not `None`, else the first
/// of `dice` (`None` if there is none).
fn reduce_start(dice: PValue, initial: PValue) -> Result<(Option<Die<Value>>, Vec<PValue>), PErr> {
    let mut dice = py_iter(dice)?.into_iter();
    let start = match initial {
        PValue::Value(Value::None) => dice.next().map(to_die).transpose()?,
        initial => Some(to_die(initial)?),
    };
    Ok((start, dice.collect()))
}

/// `map(function, result, die)`, as `reduce` and `accumulate` do for each die after the first.
fn reduce_step(f: &PValue, result: Die<Value>, die: PValue) -> Result<Die<Value>, PErr> {
    the_die(super::map::map(
        f.clone(),
        PValue::List(vec![PValue::Die(result), die]),
        PValue::Value(Value::None),
        vec![],
    )?)
}

/// `icepool.reduce(function, dice, initial=initial)`: applies a function of two arguments cumulatively to a
/// sequence of dice (analogous to `functools.reduce`). With no dice and no initial, `next()` raises StopIteration.
pub fn reduce(f: PValue, dice: PValue, initial: PValue) -> Result<PValue, PErr> {
    let (start, rest) = reduce_start(dice, initial)?;
    let Some(mut result) = start else {
        return raise(Error::InvalidArgument("StopIteration".into()));
    };
    for die in rest {
        result = reduce_step(&f, result, die)?;
    }
    Ok(PValue::Die(result))
}

/// `tuple(icepool.accumulate(function, dice, initial=initial))`: each result of `reduce` in turn (analogous to
/// `itertools.accumulate`); nothing for no dice and no initial.
pub fn accumulate(f: PValue, dice: PValue, initial: PValue) -> Result<PValue, PErr> {
    let (start, rest) = reduce_start(dice, initial)?;
    let Some(mut result) = start else {
        return Ok(PValue::tuple(vec![]));
    };
    let mut out = vec![PValue::Die(result.clone())];
    for die in rest {
        result = reduce_step(&f, result, die)?;
        out.push(PValue::Die(result.clone()));
    }
    Ok(PValue::tuple(out))
}

/// The random variables of harness/catalogue/rvs.py (`RVS[name](lo, hi)`), duck-typed like `scipy.stats` ones.
enum Rv {
    /// `DiscreteUniform(lo, hi)`: the ints `lo..hi` (`hi` excluded), as `scipy.stats.randint`. No `pdf`.
    Discrete(Value, Value),
    /// `ContinuousUniform(lo, hi)`: uniform on `[lo, hi]`. Has a `pdf`, so `from_rv` uses midpoints.
    Continuous(Value, Value),
}

impl Rv {
    fn new(name: PValue, params: PValue) -> Result<Rv, PErr> {
        let Ok([lo, hi]) = <[Value; 2]>::try_from(values(py_iter(params)?)?) else {
            return wrong_type("expected two parameters");
        };
        match &value(name)? {
            Value::Str(s) if &**s == "discrete_uniform" => Ok(Rv::Discrete(lo, hi)),
            Value::Str(s) if &**s == "continuous_uniform" => Ok(Rv::Continuous(lo, hi)),
            _ => Err(PErr::Unimplemented("this random variable".into())),
        }
    }

    /// `rv.cdf(xs)`: `[min(max(p, 0.0), 1.0) for x in xs]` with `p = (floor(x) - lo + 1) / (hi - lo)` (discrete) or
    /// `(x - lo) / (hi - lo)` (continuous).
    fn cdf(&self, xs: &[Value]) -> Result<Vec<Value>, Error> {
        let (zero, one) = (Value::Float(0.0), Value::Float(1.0));
        xs.iter()
            .map(|x| {
                let p = match self {
                    Rv::Discrete(lo, hi) => x
                        .unary(UnOp::Floor)?
                        .binary(BinOp::Sub, lo)?
                        .binary(BinOp::Add, &Value::from(1))?
                        .binary(BinOp::TrueDiv, &hi.binary(BinOp::Sub, lo)?)?,
                    Rv::Continuous(lo, hi) => x
                        .binary(BinOp::Sub, lo)?
                        .binary(BinOp::TrueDiv, &hi.binary(BinOp::Sub, lo)?)?,
                };
                Value::py_min(Value::py_max(p, zero.clone())?, one.clone())
            })
            .collect()
    }
}

/// `icepool.from_rv(RVS[rv](*params), outcomes, denominator)`: a die from a random variable's CDF, see
/// [`from_cdf`].
pub fn from_rv(
    rv: PValue,
    params: PValue,
    outcomes: PValue,
    denominator: PValue,
) -> Result<PValue, PErr> {
    let rv = Rv::new(rv, params)?;
    let cdf = |xs: PValue| -> Result<PValue, PErr> { Ok(list(rv.cdf(&values(py_iter(xs)?)?)?)) };
    from_cdf(matches!(rv, Rv::Continuous(..)), cdf, outcomes, denominator)
}

/// A Python list of values.
fn list(xs: Vec<Value>) -> PValue {
    PValue::List(xs.into_iter().map(PValue::Value).collect())
}

/// The body of `icepool.from_rv(rv, outcomes, denominator)` for any rv: `has_pdf` is `hasattr(rv, 'pdf')`, `cdf`
/// is `rv.cdf(xs, **kwargs)`, called once (with the outcomes as given, or a list of midpoints) and iterated.
/// Continuous distributions use midpoints between the outcomes; each cumulative probability is scaled by
/// `denominator` and rounded (`int(round(x * denominator))`), then `from_cumulative`.
pub(super) fn from_cdf(
    has_pdf: bool,
    cdf: impl FnOnce(PValue) -> Result<PValue, PErr>,
    outcomes: PValue,
    denominator: PValue,
) -> Result<PValue, PErr> {
    let denominator = value(denominator)?;
    let scale = |cdf: PValue| -> Result<Vec<Value>, PErr> {
        values(py_iter(cdf)?)?
            .iter()
            .map(|x| Ok(x.binary(BinOp::Mul, &denominator)?.round(&Value::None)?))
            .collect()
    };
    let quantities_le = if has_pdf {
        // Continuous distributions use midpoints.
        let outcome_values = values(py_iter(outcomes.clone())?)?;
        let midpoints = outcome_values
            .iter()
            .zip(outcome_values.iter().skip(1))
            .map(|(a, b)| {
                a.binary(BinOp::Add, b)?
                    .binary(BinOp::TrueDiv, &Value::from(2))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut q = scale(cdf(list(midpoints))?)?;
        q.push(denominator.clone());
        q
    } else {
        scale(cdf(outcomes.clone())?)?
    };
    from_cumulative(
        outcomes,
        PValue::Value(Value::tuple(quantities_le)),
        PValue::Value(Value::Bool(false)),
    )
}

/// What `Wallenius.__init__` builds: one deck per weight (a `dict` keyed by weight, in insertion order) and the
/// die of weights, each weighted by its total weight in the decks.
struct Wallenius {
    weight_decks: Vec<(Value, icepool_core::Deck<Value>)>,
    weight_die: Die<Value>,
}

/// `population.append(outcome, quantity)` (base.py): `Counter(self)`, then `data[outcome] = max(data[outcome] +
/// quantity, 0)`, back through the population's constructor.
fn append_data(
    items: &[(Value, Quantity)],
    outcome: &Value,
    quantity: &Value,
) -> Result<Vec<(Value, Value)>, PErr> {
    let mut data: Vec<(Value, Value)> = items
        .iter()
        .map(|(o, q)| (o.clone(), Value::Int(q.clone().into())))
        .collect();
    let i = match data.iter().position(|(o, _)| o == outcome) {
        Some(i) => i,
        None => {
            data.push((outcome.clone(), Value::from(0)));
            data.len() - 1
        }
    };
    data[i].1 = Value::py_max(data[i].1.binary(BinOp::Add, quantity)?, Value::from(0))?;
    Ok(data)
}

impl Wallenius {
    /// `Wallenius(data)`: `data` is either an iterable of `(outcome, weight)`, or (`mapping`) the items of a
    /// mapping from outcomes to either weights or `(weight, quantity)`.
    fn new(data: PValue, mapping: bool) -> Result<Wallenius, PErr> {
        let mut weight_decks: Vec<(Value, icepool_core::Deck<Value>)> = Vec::new();
        let pairs = if mapping {
            super::ops::dict_items(data)?
                .into_iter()
                .map(|(k, v)| match k {
                    super::ops::Key::Value(k) => Ok((k, v)),
                    super::ops::Key::Die(_) => {
                        Err(PErr::Unimplemented("a die as a Wallenius outcome".into()))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut pairs = Vec::new();
            for item in py_iter(data)? {
                let Ok([o, w]) = <[PValue; 2]>::try_from(py_iter(item)?) else {
                    return raise(Error::InvalidArgument(
                        "not enough or too many values to unpack (expected 2)".into(),
                    ));
                };
                pairs.push((value(o)?, w));
            }
            pairs
        };
        for (outcome, v) in pairs {
            // A weight is a dict key: a list is unhashable.
            let weight_value = |w: PValue| match w {
                PValue::List(_) => unhashable_list(),
                w => value(w),
            };
            let (weight, quantity) = match (v, mapping) {
                (v, false) => (weight_value(v)?, Value::from(1)),
                // `isinstance(v, int)` (a bool is one); anything else unpacks as `weight, quantity` (a list too).
                (PValue::Value(v @ (Value::Int(_) | Value::Bool(_))), true) => (v, Value::from(1)),
                (v, true) => {
                    let Ok([w, q]) = <[PValue; 2]>::try_from(py_iter(v)?) else {
                        return raise(Error::InvalidArgument(
                            "not enough or too many values to unpack (expected 2)".into(),
                        ));
                    };
                    (weight_value(w)?, value(q)?)
                }
            };
            weight.check_hashable()?;
            // `self._weight_decks.get(weight, icepool.Deck()).append(outcome, quantity)`.
            let i = weight_decks.iter().position(|(w, _)| *w == weight);
            let items = i.map_or(&[][..], |i| weight_decks[i].1.items());
            let deck = super::deck::deck_from_data(append_data(items, &outcome, &quantity)?)?;
            let PValue::Deck(deck) = deck else {
                unreachable!("Deck(data) is a deck")
            };
            match i {
                Some(i) => weight_decks[i].1 = deck,
                None => weight_decks.push((weight, deck)),
            }
        }
        // `Die({weight: weight * deck.denominator() ...})`.
        let (keys, quantities) = weight_decks
            .iter()
            .map(|(w, deck)| {
                let size = Value::Int(deck.population().denominator().into());
                Ok((
                    PValue::Value(w.clone()),
                    PValue::Value(w.binary(BinOp::Mul, &size)?),
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?
            .into_iter()
            .unzip();
        let weight_die = the_die(weights_die(keys, quantities)?)?;
        Ok(Wallenius {
            weight_decks,
            weight_die,
        })
    }

    /// `deal(hand_size)`: a mixture of the deals of each sorted tuple of weights to pull (`Counter(weights)`: how
    /// many cards of each weight), weighted by its chance; `Pool([])` for a hand of 0.
    fn deal(&self, hand_size: &IBig) -> Result<MExpr, PErr> {
        if *hand_size == IBig::ZERO {
            let pool = icepool_core::Pool::new(vec![])?;
            return Ok(as_expression(&PValue::Pool(pool)).expect("a pool"));
        }
        let hand_weights =
            wallenius_weights(&self.weight_die, hand_size, &mut FxHashMap::default())?;
        let mut data = Vec::new();
        for (weights, quantity) in hand_weights.items() {
            let Value::Tuple(weights) = weights else {
                unreachable!("_wallenius_weights makes tuples")
            };
            // `Counter(weights)`, in first-seen order.
            let mut counts: Vec<(&Value, i64)> = Vec::new();
            for w in weights.iter() {
                match counts.iter_mut().find(|(x, _)| *x == w) {
                    Some((_, c)) => *c += 1,
                    None => counts.push((w, 1)),
                }
            }
            let mut result: Option<PValue> = None;
            for (weight, count) in counts {
                let deck = &self
                    .weight_decks
                    .iter()
                    .find(|(w, _)| w == weight)
                    .ok_or_else(|| PErr::Raise(Error::MissingKey(format!("{weight}"))))?
                    .1;
                let deal = PValue::Deal(icepool_core::Deal::new(deck.clone(), &IBig::from(count))?);
                result = Some(match result {
                    None => deal,
                    Some(r) => super::expression::binary_op("+", r, deal)?,
                });
            }
            let pool = result.expect("a hand has at least one card");
            let inner = as_expression(&pool).expect("deals and their sums are expressions");
            add_inner(&mut data, inner, quantity.clone())?;
        }
        Ok(MExpr::Mixture(Arc::new(data)))
    }
}

/// How deep `_wallenius_weights` may recurse in the port: Python's recursion limit stops it after about this
/// many levels (each level is several frames of `map`), at a point the port does not model.
const MAX_WALLENIUS_DEPTH: usize = 40;

/// `_wallenius_weights(weight_die, hand_size)`: a die whose outcomes are sorted tuples of weights to pull. Each
/// weight is pulled with its chance in `weight_die`, which then loses that card's weight (`remove(weight,
/// weight)`). Python caches it by its arguments; `cache` does the same per call.
fn wallenius_weights(
    weight_die: &Die<Value>,
    hand_size: &IBig,
    cache: &mut FxHashMap<(Die<Value>, IBig), Die<Value>>,
) -> Result<Die<Value>, PErr> {
    if *hand_size == IBig::ZERO {
        return Ok(Die::from_dict(vec![(Value::tuple(vec![]), Quantity::ONE)])?);
    }
    let key = (weight_die.clone(), hand_size.clone());
    if let Some(die) = cache.get(&key) {
        return Ok(die.clone());
    }
    if cache.len() > 1 << 16 || weight_die.denominator() > Quantity::from(1u32 << 16) {
        return Err(PErr::Unimplemented("a large Wallenius".into()));
    }
    let mut objects = Vec::new();
    let mut times = Vec::new();
    for (weight, quantity) in weight_die.items() {
        // `inner(weight)`: `(_wallenius_weights(weight_die.remove(weight, weight), hand_size - 1) + (weight,))
        // .map(lambda x: tuple(sorted(x)))`.
        let neg = weight.unary(UnOp::Neg)?;
        let (keys, values): (Vec<_>, Vec<_>) = append_data(weight_die.items(), weight, &neg)?
            .into_iter()
            .map(|(o, q)| (PValue::Value(o), PValue::Value(q)))
            .unzip();
        let removed = the_die(weights_die(keys, values)?)?;
        let rest = wallenius_weights(&removed, &(hand_size - IBig::ONE), cache)?;
        let pulled = the_die(binary(
            BinOp::Add,
            PValue::Die(rest),
            PValue::Value(Value::tuple(vec![weight.clone()])),
        )?)?;
        let (sorted, quantities): (Vec<_>, Vec<_>) = pulled
            .items()
            .iter()
            .map(|(x, q)| {
                let Value::Tuple(x) = x else {
                    unreachable!("tuples")
                };
                let sorted = icepool_core::value::sort::sorted(x.to_vec())?;
                Ok((PValue::Value(Value::tuple(sorted)), q.clone()))
            })
            .collect::<Result<Vec<_>, Error>>()?
            .into_iter()
            .unzip();
        objects.push(die_of(sorted, quantities)?);
        times.push(quantity.clone());
    }
    let result = the_die(die_of(objects, times)?)?;
    cache.insert(key, result.clone());
    Ok(result)
}

/// `icepool.Wallenius(dict(data) if mapping else data).deal(hand_size)`: the multiset expression of the hand.
pub fn wallenius_deal_expr(
    data: PValue,
    mapping: PValue,
    hand_size: PValue,
) -> Result<PValue, PErr> {
    let wallenius = Wallenius::new(data, flag(&mapping)?)?;
    let hand_size = match value(hand_size)? {
        Value::Int(i) => i,
        Value::Bool(b) => IBig::from(b as u8),
        _ => return Err(PErr::Unimplemented("a non-int Wallenius hand size".into())),
    };
    // The recursion pulls one card per level until the hand is full or the cards run out (a negative hand
    // never fills).
    let cards: Quantity = wallenius
        .weight_decks
        .iter()
        .map(|(_, d)| d.population().denominator())
        .sum();
    let depth = if hand_size < IBig::ZERO {
        IBig::from(cards)
    } else {
        hand_size.clone().min(IBig::from(cards))
    };
    if depth > IBig::from(MAX_WALLENIUS_DEPTH) {
        return Err(PErr::Unimplemented("a Wallenius hand this deep".into()));
    }
    Ok(super::expression::into_pvalue(wallenius.deal(&hand_size)?))
}

/// `getattr(icepool.Wallenius(dict(data) if mapping else data).deal(hand_size), method)()`.
pub fn wallenius_deal(
    data: PValue,
    mapping: PValue,
    hand_size: PValue,
    method: PValue,
) -> Result<PValue, PErr> {
    let expr = wallenius_deal_expr(data, mapping, hand_size)?;
    let Value::Str(method) = value(method)? else {
        return wrong_type("attribute name must be string");
    };
    super::evaluate::method_evaluation(&method, expr)
}
