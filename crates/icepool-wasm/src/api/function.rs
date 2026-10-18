//! function.py's free functions (and Python's dict and `itemize` checks) on the core, under the TypeScript
//! functions of js/src/functions.ts and js/src/die.ts, which do each one's argument juggling (`_iter_outcomes`,
//! `implicit_convert_to_die`, `Fraction(n)` and `limit_denominator`, the `multiply_quantities` calls) as the
//! Python facade does (python/icepool/function.py), and call these for the work.

use std::collections::HashMap;

use dashu_int::{IBig, UBig};
use icepool_core::expand::{tupleize_dice, vectorize_dice};
use icepool_core::format::format_probability_inverse as core_format_probability_inverse;
use icepool_core::value::builtins::{fraction, limit_denominator};
use icepool_core::value::BinOp;
use icepool_core::{function, Error, Quantity, Value};
use wasm_bindgen::prelude::*;

use super::convert::{int_to_js, value_from_js, value_to_js, values_to_js};
use super::die::WasmDie;
use super::error::{JsResult, OrThrow};
use super::host;

fn values(xs: &[JsValue]) -> JsResult<Vec<Value>> {
    xs.iter().map(|x| value_from_js(x).or_throw()).collect()
}

/// The arguments of a free function that takes dice and plain outcomes alike (`tupleize(d(6), 1)`), in order:
/// wasm-bindgen passes dice to Rust one by one, so the TypeScript adds each argument here.
#[wasm_bindgen]
#[derive(Default)]
pub struct DiceArgs {
    /// Each argument's `(outcome, quantity)` items: a die's own, `[(x, 1)]` for a plain outcome `x`.
    items: Vec<Vec<(Value, Quantity)>>,
    /// The arguments that are dice, in order (empty where some argument is not a die).
    dice: Vec<icepool_core::Die<Value>>,
    all_dice: bool,
}

#[wasm_bindgen]
impl DiceArgs {
    #[wasm_bindgen(constructor)]
    pub fn new() -> DiceArgs {
        DiceArgs { all_dice: true, ..DiceArgs::default() }
    }

    /// A die.
    pub fn die(&mut self, die: &WasmDie) {
        self.items.push(die.die.items().to_vec());
        self.dice.push(die.die.clone());
    }

    /// A plain outcome.
    pub fn outcome(&mut self, outcome: JsValue) -> JsResult<()> {
        self.items.push(vec![(value_from_js(&outcome).or_throw()?, Quantity::ONE)]);
        self.all_dice = false;
        Ok(())
    }

    /// `cartesian_product(*args, outcome_type=tuple)` once the TypeScript has found a die among them: a die of
    /// tuples.
    pub fn tupleize(&self) -> JsResult<WasmDie> {
        Ok(WasmDie::new(tupleize_dice(&self.items).or_throw()?))
    }

    /// `cartesian_product(*args, outcome_type=Vector)` (`vectorize`), as `tupleize`.
    pub fn vectorize(&self) -> JsResult<WasmDie> {
        Ok(WasmDie::new(vectorize_dice(&self.items).or_throw()?))
    }

    /// `pointwise_max` (`max`) / `pointwise_min` of dice whose denominators are already harmonized.
    pub fn pointwise(&self, max: bool) -> JsResult<WasmDie> {
        if !self.all_dice {
            return Err(Error::WrongType("pointwise takes dice".into())).or_throw();
        }
        Ok(WasmDie::new(function::pointwise(&self.dice, max).or_throw()?))
    }
}

/// The `dict` items of `coin(n, d)` (ints, once a non-int numerator has become a Fraction's numerator and
/// denominator), as `[outcome, quantity]` pairs.
#[wasm_bindgen(js_name = coinData)]
pub fn coin_data(n: JsValue, d: JsValue) -> JsResult<Vec<JsValue>> {
    let (n, d) = (value_from_js(&n).or_throw()?, value_from_js(&d).or_throw()?);
    function::coin_data(&n, &d)
        .or_throw()?
        .iter()
        .map(|(k, q)| Ok(JsValue::from(vec![value_to_js(k).or_throw()?, value_to_js(q).or_throw()?])))
        .collect()
}

/// `one_hot(sides)`.
#[wasm_bindgen(js_name = oneHot)]
pub fn one_hot(sides: JsValue) -> JsResult<WasmDie> {
    Ok(WasmDie::new(function::one_hot(&value_from_js(&sides).or_throw()?).or_throw()?))
}

/// Python's `min(items)` (`max` false) or `max(items)` (`min_outcome` / `max_outcome`).
#[wasm_bindgen(js_name = minMax)]
pub fn min_max(items: Vec<JsValue>, max: bool) -> JsResult<JsValue> {
    value_to_js(&function::py_min_max(values(&items)?, max).or_throw()?).or_throw()
}

/// `consecutive(*args)` given what each of its two chains over the arguments yields.
#[wasm_bindgen]
pub fn consecutive(first: Vec<JsValue>, second: Vec<JsValue>) -> JsResult<JsValue> {
    value_to_js(&function::consecutive(values(&first)?, values(&second)?).or_throw()?).or_throw()
}

/// Python's `sorted(values)` (wallenius.py sorts the weights pulled).
#[wasm_bindgen(js_name = pySorted)]
pub fn py_sorted(values: Vec<JsValue>) -> JsResult<JsValue> {
    let sorted = icepool_core::value::sort::sorted(self::values(&values)?).or_throw()?;
    values_to_js(&sorted).or_throw()
}

/// `sorted_union(*args)` given the elements of each argument (an array per argument).
#[wasm_bindgen(js_name = sortedUnion)]
pub fn sorted_union(args: Vec<JsValue>) -> JsResult<JsValue> {
    let sets = args
        .iter()
        .map(|a| values(&host::array_from(a)))
        .collect::<JsResult<Vec<_>>>()?;
    let union = function::sorted_union(sets.iter().map(|s| s.iter())).or_throw()?;
    values_to_js(&union).or_throw()
}

/// math.py `weighted_lcm(denominators, weights)`: the scale factor for each die (`harmonize_denominators`), given
/// what each of Python's two `zip(denominators, weights)` passes takes from the weights.
#[wasm_bindgen(js_name = weightedLcm)]
pub fn weighted_lcm(
    denominators: Vec<JsValue>,
    weights: Vec<JsValue>,
    weights_again: Vec<JsValue>,
) -> JsResult<Vec<JsValue>> {
    let denominators = values(&denominators)?
        .into_iter()
        .map(|d| match d {
            Value::Int(i) => Ok(UBig::try_from(i).expect("a denominator is not negative")),
            _ => Err(Error::WrongType("a denominator must be an int".into())).or_throw(),
        })
        .collect::<JsResult<Vec<Quantity>>>()?;
    let scales =
        function::weighted_lcm(&denominators, &values(&weights)?, &values(&weights_again)?).or_throw()?;
    scales.iter().map(|v| value_to_js(v).or_throw()).collect()
}

/// `format_probability_inverse(probability, int_start)`.
#[wasm_bindgen(js_name = formatProbabilityInverse)]
pub fn format_probability_inverse(probability: JsValue, int_start: JsValue) -> JsResult<String> {
    let (p, start) = (value_from_js(&probability).or_throw()?, value_from_js(&int_start).or_throw()?);
    core_format_probability_inverse(&p, &start).or_throw()
}

/// The keys of a Python dict literal `{k0: ..., k1: ..., ...}` with plain-value keys: for each key, the index of the
/// first key equal to it (itself if none is), as Python's dict keeps the first spelling of a key (and the last
/// value). Keys must be hashable; equal keys are those Python's `==` (and hash) find equal: `1`, `1.0` and `true`
/// are one key, as are two arrays with equal elements.
#[wasm_bindgen(js_name = dictKeys)]
pub fn dict_keys(keys: Vec<JsValue>) -> JsResult<Vec<u32>> {
    let mut first: HashMap<Value, u32> = HashMap::new();
    let mut result = Vec::with_capacity(keys.len());
    for (i, key) in keys.iter().enumerate() {
        let key = value_from_js(key).or_throw()?;
        key.check_hashable().or_throw()?;
        result.push(*first.entry(key).or_insert(i as u32));
    }
    Ok(result)
}

/// The weights of creation_args.py `itemize(keys, times)` once the TypeScript has one time per key: each time,
/// multiplied by the mapping's value for that key when there is one (`base`: `v * x`), then checked as Python's
/// `any(x < 0 for x in times)` checks them (a `TypeError` for anything that is not a number, a `ValueError` for a
/// negative one).
#[wasm_bindgen(js_name = itemizeTimes)]
pub fn itemize_times(base: Option<Vec<JsValue>>, times: Vec<JsValue>) -> JsResult<Vec<JsValue>> {
    let mut times = values(&times)?;
    if let Some(base) = base {
        times = values(&base)?
            .iter()
            .zip(&times)
            .map(|(v, x)| v.binary(BinOp::Mul, x))
            .collect::<icepool_core::Result<_>>()
            .or_throw()?;
    }
    let zero = Value::from(0);
    for x in &times {
        if x.binary(BinOp::Lt, &zero).and_then(|lt| lt.truthy()).or_throw()? {
            return Err(Error::InvalidArgument("times cannot have negative values.".into())).or_throw();
        }
    }
    times.iter().map(|x| value_to_js(x).or_throw()).collect()
}

/// `Fraction(x)`, limited to `max_denominator` unless that is `null` (`Fraction.limit_denominator`), as its
/// `[numerator, denominator]` (ints): what `coin` does with a non-int numerator.
#[wasm_bindgen(js_name = fractionParts)]
pub fn fraction_parts(x: JsValue, max_denominator: JsValue) -> JsResult<Vec<JsValue>> {
    let mut f = fraction(&value_from_js(&x).or_throw()?).or_throw()?;
    match value_from_js(&max_denominator).or_throw()? {
        Value::None => {}
        Value::Int(m) => f = limit_denominator(&f, &m).or_throw()?,
        Value::Bool(b) => f = limit_denominator(&f, &IBig::from(b as u8)).or_throw()?,
        _ => return Err(Error::Unported("limit_denominator with a non-int bound".into())).or_throw(),
    }
    Ok(vec![int_to_js(f.numerator()), int_to_js(&IBig::from(f.denominator().clone()))])
}

/// `mapping.get(x)` for each of `xs`, in a dict with these distinct plain keys: the index of the key equal to each
/// (Python's `==` and hash), or -1 for none. Each `x` must be hashable, as Python's lookup requires.
#[wasm_bindgen(js_name = dictLookup)]
pub fn dict_lookup(keys: Vec<JsValue>, xs: Vec<JsValue>) -> JsResult<Vec<i32>> {
    let mut index: HashMap<Value, i32> = HashMap::new();
    for (i, key) in values(&keys)?.into_iter().enumerate() {
        key.check_hashable().or_throw()?;
        index.entry(key).or_insert(i as i32);
    }
    values(&xs)?
        .iter()
        .map(|x| {
            x.check_hashable().or_throw()?;
            Ok(index.get(x).copied().unwrap_or(-1))
        })
        .collect()
}
