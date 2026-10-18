//! `WasmDie`: a die of the core, under the TypeScript `Die` (js/src/die.ts), which does Python's argument
//! juggling (population/die.py, population/base.py) and calls these for the work.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};

use dashu_int::IBig;
use icepool_core::format::{common_outcome_length, format as format_population, Formatted};
use icepool_core::population::{Comparison, Data};
use icepool_core::value::builtins::Index;
use icepool_core::value::pystr::py_repr;
use icepool_core::value::{matmul_dice, BinOp, Num, UnOp};
use icepool_core::expand::tupleize_dice;
use icepool_core::memory;
use icepool_core::random::Random;
use icepool_core::{function, DieArg, Die, Error, Quantity, Value};
use wasm_bindgen::prelude::*;

use super::convert::{int_from_js, quantity_to_js, value_from_js, value_to_js, values_to_js};
use super::error::{JsResult, OrThrow};

/// A die of the core. JS frees it with `free()` (the TypeScript `Die` does so on `dispose`, or when it is
/// garbage-collected).
#[wasm_bindgen]
pub struct WasmDie {
    pub(crate) die: Die<Value>,
    /// Python's `Die._mean_time_to_sum_cache`: the mean times computed so far, for targets 0, 1, ...
    mean_time_to_sum_cache: RefCell<Vec<Value>>,
}

thread_local! {
    /// How many `WasmDie`s are alive (for the JS tests' leak checks).
    static LIVE: Cell<usize> = const { Cell::new(0) };
}

impl WasmDie {
    pub(crate) fn new(die: Die<Value>) -> WasmDie {
        LIVE.with(|n| n.set(n.get() + 1));
        WasmDie { die, mean_time_to_sum_cache: RefCell::default() }
    }
}

impl Drop for WasmDie {
    fn drop(&mut self) {
        LIVE.with(|n| n.set(n.get() - 1));
    }
}

/// How many dice JS holds (for leak checks).
#[wasm_bindgen(js_name = liveDice)]
pub fn live_dice() -> usize {
    LIVE.with(Cell::get)
}

/// `Die(outcomes, times)` once the TypeScript `Die` has itemized its arguments (creation_args.py `itemize`): each
/// element of `outcomes` with its weight (a time as `itemize` leaves it: a non-negative number, which must be an int
/// unless it is zero). `build` mixes them (`expand_args_for_die`).
#[wasm_bindgen]
#[derive(Default)]
pub struct DieBuilder {
    args: Vec<DieArg<Value>>,
    weights: Vec<Quantity>,
}

#[wasm_bindgen]
impl DieBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> DieBuilder {
        DieBuilder::default()
    }

    /// An outcome.
    pub fn outcome(&mut self, outcome: JsValue, weight: JsValue) -> JsResult<()> {
        self.push(DieArg::Outcome(value(&outcome)?), &weight)
    }

    /// A die, mixed in with its outcomes sharing the weight.
    pub fn die(&mut self, die: &WasmDie, weight: JsValue) -> JsResult<()> {
        self.push(DieArg::Die(die.die.clone()), &weight)
    }

    /// `Reroll` (or `Restart`): drops out.
    pub fn reroll(&mut self, weight: JsValue) -> JsResult<()> {
        self.push(DieArg::Reroll, &weight)
    }

    pub fn build(&mut self) -> JsResult<WasmDie> {
        let (args, weights) = (std::mem::take(&mut self.args), std::mem::take(&mut self.weights));
        Ok(WasmDie::new(Die::new(args, weights).or_throw()?))
    }
}

impl DieBuilder {
    fn push(&mut self, arg: DieArg<Value>, weight: &JsValue) -> JsResult<()> {
        let weight = die_weight(&arg, &value(weight)?).or_throw()?;
        self.args.push(arg);
        self.weights.push(weight);
        Ok(())
    }
}

/// One weight given to `Die` (a time of `itemize`, already checked to be a non-negative number), following what
/// Python does with it in `merge_weights_lcm`: an `int` (or `bool`) is the quantity; another number (`float`,
/// `Fraction`) only survives if it can never reach `math.gcd`: when it is zero, or when the argument is an empty
/// die (denominator 0). Either way it then contributes nothing, exactly like a 0.
fn die_weight(arg: &DieArg<Value>, t: &Value) -> icepool_core::Result<Quantity> {
    let negative = || Error::InvalidArgument("times cannot have negative values.".into());
    match t {
        Value::Int(i) => Quantity::try_from(i.clone()).map_err(|_| negative()),
        Value::Bool(b) => Ok(Quantity::from(*b as u8)),
        _ => {
            let zero = Num::Int(Cow::Owned(IBig::ZERO));
            if t.as_num().is_some_and(|n| n < zero) {
                return Err(negative());
            }
            let empty_die = matches!(arg, DieArg::Die(d) if d.denominator().is_zero());
            if t.as_num().is_some_and(|n| n == zero) || empty_die {
                Ok(Quantity::ZERO)
            } else {
                Err(Error::WrongType(format!(
                    "'{}' object cannot be interpreted as an integer",
                    t.type_name()
                )))
            }
        }
    }
}

/// Python's binary operators on dice (`Die.binary_operator` with `operator.add`, ...).
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    TrueDiv,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    And,
    Or,
    Xor,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// Python's unary operators on dice (`Die.unary_operator` with `operator.neg`, `math.floor`, ...).
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum UnaryOp {
    Neg,
    Pos,
    Invert,
    Abs,
    Trunc,
    Floor,
    Ceil,
}

impl From<UnaryOp> for UnOp {
    fn from(op: UnaryOp) -> UnOp {
        match op {
            UnaryOp::Neg => UnOp::Neg,
            UnaryOp::Pos => UnOp::Pos,
            UnaryOp::Invert => UnOp::Invert,
            UnaryOp::Abs => UnOp::Abs,
            UnaryOp::Trunc => UnOp::Trunc,
            UnaryOp::Floor => UnOp::Floor,
            UnaryOp::Ceil => UnOp::Ceil,
        }
    }
}

/// `x <op> y` on two outcomes.
pub(super) fn apply(op: BinaryOp, x: &Value, y: &Value) -> icepool_core::Result<Value> {
    let op = match op {
        BinaryOp::Eq => return x.eq_op(y, false),
        BinaryOp::Ne => return x.eq_op(y, true),
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::TrueDiv => BinOp::TrueDiv,
        BinaryOp::FloorDiv => BinOp::FloorDiv,
        BinaryOp::Mod => BinOp::Mod,
        BinaryOp::Pow => BinOp::Pow,
        BinaryOp::LShift => BinOp::LShift,
        BinaryOp::RShift => BinOp::RShift,
        BinaryOp::And => BinOp::And,
        BinaryOp::Or => BinOp::Or,
        BinaryOp::Xor => BinOp::Xor,
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::Le => BinOp::Le,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::Ge => BinOp::Ge,
    };
    x.binary(op, y)
}

/// A comparison given as Python spells it (`'<='`, ...); `None` (JS `null`) is `'=='`.
fn comparison(c: Option<String>) -> icepool_core::Result<Comparison> {
    match c {
        None => Ok(Comparison::Eq),
        Some(c) => Comparison::parse(&Value::str(&c)),
    }
}

/// A sequence of `[outcome, quantity]` pairs, as JS arrays.
fn pairs(items: &[(Value, Quantity)]) -> JsResult<Vec<JsValue>> {
    items
        .iter()
        .map(|(o, q)| Ok(JsValue::from(vec![value_to_js(o).or_throw()?, quantity_to_js(q)])))
        .collect()
}

fn value(x: &JsValue) -> JsResult<Value> {
    value_from_js(x).or_throw()
}

fn js(v: icepool_core::Result<Value>) -> JsResult<JsValue> {
    value_to_js(&v.or_throw()?).or_throw()
}

/// `Die(data)` for the `(outcome, quantity)` pairs a quantity method computed (population/base.py `_new_type`):
/// the quantities are Python numbers, which `Die` takes only as non-negative ints (or zeros).
fn from_data(data: Data) -> icepool_core::Result<Die<Value>> {
    let zero = Num::Int(Cow::Owned(IBig::ZERO));
    let mut args = Vec::with_capacity(data.len());
    let mut quantities = Vec::with_capacity(data.len());
    for (outcome, q) in data {
        // `itemize`: `any(x < 0 for x in times)` raises for negatives and for anything not a number.
        match q.as_num() {
            Some(n) if n < zero => {
                return Err(Error::InvalidArgument("times cannot have negative values.".into()))
            }
            Some(_) => {}
            None => {
                return Err(Error::Incomparable { op: "<", left: q.type_name(), right: "int" })
            }
        }
        // `merge_weights_lcm`: an int is the weight; another number only survives as a zero.
        let q = match q {
            Value::Int(i) => Quantity::try_from(i).expect("checked non-negative"),
            Value::Bool(b) => Quantity::from(b as u8),
            q if q.as_num() == Some(zero.clone()) => Quantity::ZERO,
            q => {
                return Err(Error::WrongType(format!(
                    "'{}' object cannot be interpreted as an integer",
                    q.type_name()
                )))
            }
        };
        args.push(DieArg::Outcome(outcome));
        quantities.push(q);
    }
    Die::new(args, quantities)
}

#[wasm_bindgen]
impl WasmDie {
    // --- Construction -------------------------------------------------------------------------------------------

    /// `icepool.d(sides)`.
    pub fn d(sides: JsValue) -> JsResult<WasmDie> {
        let sides = int_from_js(&sides, "sides").or_throw()?;
        // Beyond i64 Python would run out of memory long before finishing.
        let sides = i64::try_from(sides).map_err(|_| Error::Overflow("sides too large".into())).or_throw()?;
        Ok(WasmDie::new(function::try_d(sides).or_throw()?))
    }

    /// `icepool.z(sides)`.
    pub fn z(sides: JsValue) -> JsResult<WasmDie> {
        let sides = int_from_js(&sides, "sides").or_throw()?;
        let sides = i64::try_from(sides).map_err(|_| Error::Overflow("sides too large".into())).or_throw()?;
        Ok(WasmDie::new(function::try_z(sides).or_throw()?))
    }

    /// `Die([outcome])`.
    pub fn single(outcome: JsValue) -> JsResult<WasmDie> {
        let outcome = value(&outcome)?;
        Ok(WasmDie::new(Die::new(vec![DieArg::Outcome(outcome)], vec![Quantity::ONE]).or_throw()?))
    }

    /// The die with no outcomes (`Die([])`, `Die([Reroll])`).
    pub fn empty() -> JsResult<WasmDie> {
        Ok(WasmDie::new(Die::new(vec![], vec![]).or_throw()?))
    }

    // --- The mapping protocol -----------------------------------------------------------------------------------

    /// `[outcome, quantity]` pairs in sorted order, zero quantities included.
    pub fn items(&self) -> JsResult<Vec<JsValue>> {
        pairs(self.die.items())
    }

    /// The outcomes in sorted order.
    pub fn outcomes(&self) -> JsResult<Vec<JsValue>> {
        self.die.outcomes().map(|o| value_to_js(o).or_throw()).collect()
    }

    /// The number of outcomes.
    #[wasm_bindgen(getter)]
    pub fn size(&self) -> usize {
        self.die.len()
    }

    /// The quantity of an outcome (a `KeyError` if it is not one). Python looks it up in a dict, which hashes
    /// the key first.
    pub fn get(&self, outcome: JsValue) -> JsResult<JsValue> {
        let key = value(&outcome)?;
        key.check_hashable().or_throw()?;
        match self.die.position(&key) {
            Some(i) => Ok(quantity_to_js(&self.die.items()[i].1)),
            None => Err(Error::MissingKey(format!("{key}"))).or_throw(),
        }
    }

    /// `outcome in die`.
    pub fn has(&self, outcome: JsValue) -> JsResult<bool> {
        let key = value(&outcome)?;
        key.check_hashable().or_throw()?;
        Ok(self.die.position(&key).is_some())
    }

    // --- Operators ----------------------------------------------------------------------------------------------

    /// `self <op> other` outcome by outcome (`Die.binary_operator`).
    pub fn binary(&self, op: BinaryOp, other: &WasmDie) -> JsResult<WasmDie> {
        let die = self.die.binary_operator(&other.die, |x, y| apply(op, x, y)).or_throw()?;
        Ok(WasmDie::new(die))
    }

    /// `<op> self` outcome by outcome (`Die.unary_operator`).
    pub fn unary(&self, op: UnaryOp) -> JsResult<WasmDie> {
        let op = UnOp::from(op);
        Ok(WasmDie::new(self.die.unary_operator(|x| x.unary(op)).or_throw()?))
    }

    /// `round(die, ndigits)`: each outcome rounded (so a bad `ndigits` only raises if there is an outcome).
    pub fn round(&self, ndigits: JsValue) -> JsResult<WasmDie> {
        let ndigits = value(&ndigits)?;
        Ok(WasmDie::new(self.die.unary_operator(|x| x.round(&ndigits)).or_throw()?))
    }

    /// `self @ die`: roll this die, then roll `die` that many times and sum.
    pub fn matmul(&self, die: &WasmDie) -> JsResult<WasmDie> {
        Ok(WasmDie::new(matmul_dice(&self.die, &die.die).or_throw()?))
    }

    /// `die.cmp(other)`: 1, -1 or 0 per pair of outcomes.
    #[wasm_bindgen(js_name = cmp)]
    pub fn cmp_die(&self, other: &WasmDie) -> JsResult<WasmDie> {
        Ok(WasmDie::new(self.die.cmp_die(&other.die).or_throw()?))
    }

    /// `die.sign()`.
    pub fn sign(&self) -> JsResult<WasmDie> {
        Ok(WasmDie::new(self.die.sign().or_throw()?))
    }

    /// `die.equals(other, simplify=simplify)` for another die.
    pub fn equals(&self, other: &WasmDie, simplify: bool) -> JsResult<bool> {
        self.die.equals(&other.die, simplify).or_throw()
    }

    // --- Selections: the TypeScript `Die` computes which outcomes a `which` selects (calling JS functions, as
    // population/base.py `_select_outcomes` calls Python ones) and passes one flag per outcome (`mask`, 0 or 1) ----

    /// `outcome in collection` for each outcome (`_select_outcomes` given a collection): Python's `==` between the
    /// collection's elements and the outcome, element by element.
    pub fn memberships(&self, collection: Vec<JsValue>) -> JsResult<Vec<u8>> {
        let items = collection.iter().map(value).collect::<JsResult<Vec<_>>>()?;
        self.die
            .outcomes()
            .map(|o| {
                for item in &items {
                    if item.eq_op(o, false)?.truthy()? {
                        return Ok(1);
                    }
                }
                Ok(0)
            })
            .collect::<icepool_core::Result<_>>()
            .or_throw()
    }

    /// `die.reroll(...)` once the selection is known: the selected outcomes are rerolled up to `depth` times
    /// (`undefined`: until they stop coming up).
    pub fn reroll(&self, mask: Vec<u8>, depth: Option<u32>) -> JsResult<WasmDie> {
        Ok(WasmDie::new(self.die.reroll(&flags(&mask), depth).or_throw()?))
    }

    /// `die.split(...)` once the selection is known: `[selected, rest]`.
    pub fn split(&self, mask: Vec<u8>) -> JsResult<Vec<WasmDie>> {
        let (selected, rest) = self.die.split(&flags(&mask)).or_throw()?;
        Ok(vec![WasmDie::new(selected), WasmDie::new(rest)])
    }

    /// `die.quantity_where(...)` once the selection is known.
    #[wasm_bindgen(js_name = quantityWhere)]
    pub fn quantity_where(&self, mask: Vec<u8>) -> JsValue {
        quantity_to_js(&self.die.quantity_where(&flags(&mask)))
    }

    /// `die.probability_where(..., percent=percent)` once the selection is known.
    #[wasm_bindgen(js_name = probabilityWhere)]
    pub fn probability_where(&self, mask: Vec<u8>, percent: bool) -> JsResult<JsValue> {
        js(self.die.probability_where(&flags(&mask), percent))
    }

    /// `die.group_by(...)` given each outcome's key (plain values): `[key, die]` pairs for each distinct key (by
    /// Python's `==`), in order of first appearance.
    #[wasm_bindgen(js_name = groupBy)]
    pub fn group_by(&self, keys: Vec<JsValue>) -> JsResult<Vec<JsValue>> {
        let keys = keys.iter().map(value).collect::<JsResult<Vec<_>>>()?;
        self.die
            .group_by(keys)
            .or_throw()?
            .into_iter()
            .map(|(k, d)| Ok(JsValue::from(vec![value_to_js(&k).or_throw()?, WasmDie::new(d).into()])))
            .collect()
    }

    // --- Quantities and outcomes --------------------------------------------------------------------------------

    /// `die.simplify()`.
    pub fn simplify(&self) -> JsResult<WasmDie> {
        Ok(WasmDie::new(self.die.simplify().or_throw()?))
    }

    /// `die.zero()`.
    pub fn zero(&self) -> JsResult<WasmDie> {
        Ok(WasmDie::new(self.die.zero().or_throw()?))
    }

    /// `die.zero_outcome()`: `self.zero().outcomes()[0]`.
    #[wasm_bindgen(js_name = zeroOutcome)]
    pub fn zero_outcome(&self) -> JsResult<JsValue> {
        let zero = self.die.zero().or_throw()?;
        value_to_js(&zero.items()[0].0).or_throw()
    }

    /// `die.truncate(min, max)` (`null` for no bound).
    pub fn truncate(&self, min: JsValue, max: JsValue) -> JsResult<WasmDie> {
        let (min, max) = (bound(&min)?, bound(&max)?);
        Ok(WasmDie::new(self.die.try_truncate(min.as_ref(), max.as_ref()).or_throw()?))
    }

    /// `die.clip(min, max)` (`null` for no bound).
    pub fn clip(&self, min: JsValue, max: JsValue) -> JsResult<WasmDie> {
        let (min, max) = (bound(&min)?, bound(&max)?);
        Ok(WasmDie::new(self.die.try_clip(min.as_ref(), max.as_ref()).or_throw()?))
    }

    /// `die.append(outcome, quantity)`: that much more of the outcome (a negative quantity removes, down to zero).
    pub fn append(&self, outcome: JsValue, quantity: JsValue) -> JsResult<WasmDie> {
        let outcome = counter_key(&outcome)?;
        let data = self.die.append(&outcome, &value(&quantity)?).or_throw()?;
        self.new_or_self(Some(data))
    }

    /// `die.remove(outcome, quantity)`: that much less of the outcome; `null` removes all of it.
    pub fn remove(&self, outcome: JsValue, quantity: JsValue) -> JsResult<WasmDie> {
        let outcome = counter_key(&outcome)?;
        let quantity = match value(&quantity)? {
            Value::None => None,
            q => Some(q),
        };
        let data = self.die.remove(&outcome, quantity.as_ref()).or_throw()?;
        self.new_or_self(Some(data))
    }

    /// `die.multiply_quantities(scale)`.
    #[wasm_bindgen(js_name = multiplyQuantities)]
    pub fn multiply_quantities(&self, scale: JsValue) -> JsResult<WasmDie> {
        let data = self.die.multiply_quantities(&value(&scale)?).or_throw()?;
        self.new_or_self(data)
    }

    /// `die.divide_quantities(divisor)`.
    #[wasm_bindgen(js_name = divideQuantities)]
    pub fn divide_quantities(&self, divisor: JsValue) -> JsResult<WasmDie> {
        let data = self.die.divide_quantities(&value(&divisor)?).or_throw()?;
        self.new_or_self(data)
    }

    /// `die.modulo_quantities(divisor)`.
    #[wasm_bindgen(js_name = moduloQuantities)]
    pub fn modulo_quantities(&self, divisor: JsValue) -> JsResult<WasmDie> {
        let data = self.die.modulo_quantities(&value(&divisor)?).or_throw()?;
        self.new_or_self(Some(data))
    }

    /// `die.pad_to_denominator(denominator, outcome)`.
    #[wasm_bindgen(js_name = padToDenominator)]
    pub fn pad_to_denominator(&self, denominator: JsValue, outcome: JsValue) -> JsResult<WasmDie> {
        let data = self
            .die
            .pad_to_denominator(&value(&denominator)?, &value(&outcome)?)
            .or_throw()?;
        self.new_or_self(Some(data))
    }

    /// `die.multiply_to_denominator(denominator)`.
    #[wasm_bindgen(js_name = multiplyToDenominator)]
    pub fn multiply_to_denominator(&self, denominator: JsValue) -> JsResult<WasmDie> {
        let data = self.die.multiply_to_denominator(&value(&denominator)?).or_throw()?;
        self.new_or_self(data)
    }

    // --- Statistics ---------------------------------------------------------------------------------------------

    pub fn denominator(&self) -> JsValue {
        quantity_to_js(&self.die.denominator())
    }

    /// `tuple(die.quantities(comparison))`.
    pub fn quantities(&self, comparison_: Option<String>) -> JsResult<Vec<JsValue>> {
        let c = comparison(comparison_).or_throw()?;
        Ok(self.die.quantities_by(c).iter().map(quantity_to_js).collect())
    }

    /// `die.quantity(comparison, outcome)`.
    pub fn quantity(&self, comparison_: Option<String>, outcome: JsValue) -> JsResult<JsValue> {
        let c = comparison(comparison_).or_throw()?;
        Ok(quantity_to_js(&self.die.try_quantity(c, &value(&outcome)?).or_throw()?))
    }

    /// `die.probability(comparison, outcome, percent=percent)`.
    pub fn probability(
        &self,
        comparison_: Option<String>,
        outcome: JsValue,
        percent: bool,
    ) -> JsResult<JsValue> {
        let c = comparison(comparison_).or_throw()?;
        js(self.die.probability(c, &value(&outcome)?, percent))
    }

    /// `die.probabilities(comparison, percent=percent)`.
    pub fn probabilities(&self, comparison_: Option<String>, percent: bool) -> JsResult<JsValue> {
        let c = comparison(comparison_).or_throw()?;
        values_to_js(&self.die.probabilities(c, percent).or_throw()?).or_throw()
    }

    /// `die.nearest(comparison, outcome)`: `null` if no outcome fits.
    pub fn nearest(&self, comparison_: String, outcome: JsValue) -> JsResult<JsValue> {
        let c = Comparison::parse(&Value::str(&comparison_)).or_throw()?;
        let found = self.die.try_nearest(c, &value(&outcome)?).or_throw()?;
        value_to_js(&found.unwrap_or(Value::None)).or_throw()
    }

    pub fn mean(&self) -> JsResult<JsValue> {
        js(self.die.mean())
    }

    pub fn variance(&self) -> JsResult<JsValue> {
        js(self.die.variance())
    }

    #[wasm_bindgen(js_name = standardDeviation)]
    pub fn standard_deviation(&self) -> JsResult<JsValue> {
        js(self.die.standard_deviation())
    }

    pub fn skewness(&self) -> JsResult<JsValue> {
        js(self.die.skewness())
    }

    #[wasm_bindgen(js_name = excessKurtosis)]
    pub fn excess_kurtosis(&self) -> JsResult<JsValue> {
        js(self.die.excess_kurtosis())
    }

    #[wasm_bindgen(js_name = standardizedMoment)]
    pub fn standardized_moment(&self, k: JsValue) -> JsResult<JsValue> {
        js(self.die.standardized_moment(&value(&k)?))
    }

    pub fn entropy(&self, base: JsValue) -> JsResult<JsValue> {
        js(self.die.entropy(&value(&base)?))
    }

    /// `die.mode()`: a tuple of the outcomes with the highest quantity.
    pub fn mode(&self) -> JsResult<JsValue> {
        values_to_js(&self.die.mode()).or_throw()
    }

    #[wasm_bindgen(js_name = modalQuantity)]
    pub fn modal_quantity(&self) -> JsResult<JsValue> {
        Ok(quantity_to_js(&self.die.modal_quantity().or_throw()?))
    }

    /// `die.quantile(n, d)` (`median` is `quantile(1, 2)`).
    pub fn quantile(&self, n: JsValue, d: JsValue) -> JsResult<JsValue> {
        js(self.die.quantile(&value(&n)?, &value(&d)?))
    }

    #[wasm_bindgen(js_name = quantileLow)]
    pub fn quantile_low(&self, n: JsValue, d: JsValue) -> JsResult<JsValue> {
        js(self.die.quantile_low(&value(&n)?, &value(&d)?))
    }

    #[wasm_bindgen(js_name = quantileHigh)]
    pub fn quantile_high(&self, n: JsValue, d: JsValue) -> JsResult<JsValue> {
        js(self.die.quantile_high(&value(&n)?, &value(&d)?))
    }

    #[wasm_bindgen(js_name = minOutcome)]
    pub fn min_outcome(&self) -> JsResult<JsValue> {
        js(self.die.min_outcome().cloned())
    }

    #[wasm_bindgen(js_name = maxOutcome)]
    pub fn max_outcome(&self) -> JsResult<JsValue> {
        js(self.die.max_outcome().cloned())
    }

    #[wasm_bindgen(js_name = kolmogorovSmirnov)]
    pub fn kolmogorov_smirnov(&self, other: &WasmDie) -> JsResult<JsValue> {
        js(self.die.kolmogorov_smirnov(&other.die))
    }

    #[wasm_bindgen(js_name = cramerVonMises)]
    pub fn cramer_von_mises(&self, other: &WasmDie) -> JsResult<JsValue> {
        js(self.die.cramer_von_mises(&other.die))
    }

    /// `die.covariance(i, j)` of two elements of tuple outcomes.
    pub fn covariance(&self, i: JsValue, j: JsValue) -> JsResult<JsValue> {
        js(self.die.covariance(&Index::Item(value(&i)?), &Index::Item(value(&j)?)))
    }

    pub fn correlation(&self, i: JsValue, j: JsValue) -> JsResult<JsValue> {
        js(self.die.correlation(&Index::Item(value(&i)?), &Index::Item(value(&j)?)))
    }

    /// `die.common_outcome_length()`: a number, or `null`.
    #[wasm_bindgen(js_name = commonOutcomeLength)]
    pub fn common_outcome_length(&self) -> JsResult<Option<usize>> {
        common_outcome_length(&self.die).or_throw()
    }

    // --- Marginals ----------------------------------------------------------------------------------------------

    /// `die.marginals[index]`.
    pub fn marginal(&self, index: JsValue) -> JsResult<WasmDie> {
        Ok(WasmDie::new(self.die.marginal(&Index::Item(value(&index)?)).or_throw()?))
    }

    /// `die.marginals[start:stop:step]`.
    #[wasm_bindgen(js_name = marginalSlice)]
    pub fn marginal_slice(&self, start: JsValue, stop: JsValue, step: JsValue) -> JsResult<WasmDie> {
        let index = Index::Slice(value(&start)?, value(&stop)?, value(&step)?);
        Ok(WasmDie::new(self.die.marginal(&index).or_throw()?))
    }

    /// `len(die.marginals)`.
    #[wasm_bindgen(js_name = marginalsLength)]
    pub fn marginals_length(&self) -> JsResult<usize> {
        self.die.marginals_len().or_throw()
    }

    // --- Again, processes, sampling -------------------------------------------------------------------------------

    /// population/again.py `evaluate_agains_using_count` once the TypeScript has checked `again_count`, built
    /// `not_again_die` (the outcomes that are not `Again`), its `zero` outcome, `step_die` (one tupleized step
    /// outcome per face) and `again_end_die` (ignored when `restart`, for `again_end=Restart`).
    #[wasm_bindgen(js_name = evaluateAgainsUsingCount)]
    pub fn evaluate_agains_using_count(
        step_die: &WasmDie,
        not_again_die: &WasmDie,
        zero: JsValue,
        again_count: u32,
        again_end_die: &WasmDie,
        restart: bool,
    ) -> JsResult<WasmDie> {
        let end = (!restart).then_some(&again_end_die.die);
        // `n @ die` with an int `n` is `Die([n]) @ die` (`Die.__rmatmul__`).
        let matmul = |n: &Value, die: &Die<Value>| {
            let n = Die::new(vec![DieArg::Outcome(n.clone())], vec![Quantity::ONE])?;
            matmul_dice(&n, die)
        };
        let die = icepool_core::again::evaluate_agains_using_count(
            &step_die.die,
            &not_again_die.die,
            &value(&zero)?,
            u64::from(again_count),
            end,
            matmul,
        )
        .or_throw()?;
        Ok(WasmDie::new(die))
    }

    /// `die.sequence(rolls)` for `rolls >= 1`: the die of the tuples of `rolls` rolls in order (`MemoryError` past
    /// what memory holds).
    pub fn sequence(&self, rolls: JsValue) -> JsResult<WasmDie> {
        let rolls = int_from_js(&rolls, "rolls").or_throw()?;
        let what = "die.sequence(rolls)";
        let n = memory::check_len::<Die<Value>, _>(Some(u64::try_from(&rolls).unwrap_or(u64::MAX)), what).or_throw()?;
        let parts = memory::try_repeat(self.die.items().to_vec(), Some(n), what).or_throw()?;
        Ok(WasmDie::new(tupleize_dice(&parts).or_throw()?))
    }

    /// A new `WasmDie` of this die, for a JS callback's result the core takes over (taking it empties the JS
    /// object, so the callback hands over a copy rather than a die of the API).
    pub fn copy(&self) -> WasmDie {
        WasmDie::new(self.die.clone())
    }

    /// `die.mean_time_to_sum(target)`, extending this die's cache as Python's loop does.
    #[wasm_bindgen(js_name = meanTimeToSum)]
    pub fn mean_time_to_sum(&self, target: JsValue) -> JsResult<JsValue> {
        let target = value(&target)?;
        let mut cache = self.mean_time_to_sum_cache.borrow_mut();
        // A `cached_property` starting as `[Fraction(0)]`.
        if cache.is_empty() {
            cache.push(Value::Fraction(dashu_ratio::RBig::ZERO));
        }
        js(self.die.mean_time_to_sum(&mut cache, &target))
    }

    /// `[die.sample() for _ in range(n)]` after Python's `random.seed(seed)` (an int seed).
    pub fn sample(&self, seed: JsValue, n: u32) -> JsResult<Vec<JsValue>> {
        let mut random = Random::new(&int_from_js(&seed, "seed").or_throw()?);
        let denominator = self.die.denominator();
        (0..n)
            .map(|_| {
                let r = random.randrange(&denominator)?;
                value_to_js(&self.die.sample_at(&r)?)
            })
            .collect::<icepool_core::Result<_>>()
            .or_throw()
    }

    /// `die.to_one_hot(outcomes)` once the TypeScript has looked each of this die's outcomes up in `outcomes`
    /// (Python's `in` and `index`): `positions` holds the index of each, or -1 for one not there; `len` is
    /// `len(outcomes)`.
    #[wasm_bindgen(js_name = toOneHot)]
    pub fn to_one_hot(&self, len: usize, positions: Vec<i32>) -> JsResult<WasmDie> {
        let positions: Vec<Option<i64>> =
            positions.iter().map(|&p| (p >= 0).then_some(i64::from(p))).collect();
        let items = self.die.one_hot_items(len, &positions).or_throw()?;
        Ok(WasmDie::new(Die::from_dict(items).or_throw()?))
    }

    // --- Formatting ---------------------------------------------------------------------------------------------

    /// `format(die, spec)` (`''` is `str(die)`); `type_name` is Python's `type(die).__name__` (`Die`, or
    /// `DieWithTruth` for the result of `==` / `!=`).
    pub fn format(&self, spec: &str, type_name: &str) -> JsResult<String> {
        let type_name = if type_name == "DieWithTruth" { "DieWithTruth" } else { "Die" };
        match format_population(&self.die, type_name, spec, false).or_throw()? {
            Formatted::Text(s) => Ok(s),
            Formatted::CsvTable(..) => unreachable!("only asked for with csv writer arguments"),
        }
    }

    /// `repr(die)`: `Die({outcome!r: quantity, ...})`, or `DieWithTruth(...)`.
    pub fn repr(&self, type_name: &str) -> JsResult<String> {
        die_repr(&self.die, type_name).or_throw()
    }
}

/// `repr(die)`: `Die({outcome!r: quantity, ...})` (`type_name` `Die` or `DieWithTruth`).
pub(super) fn die_repr(die: &Die<Value>, type_name: &str) -> icepool_core::Result<String> {
    let items = die
        .items()
        .iter()
        .map(|(o, q)| Ok(format!("{}: {q}", py_repr(o)?)))
        .collect::<icepool_core::Result<Vec<_>>>()?;
    Ok(format!("{type_name}({{{}}})", items.join(", ")))
}

/// An outcome used as a `Counter` key (`append`, `remove`): hashable, as Python's `Counter` requires.
fn counter_key(outcome: &JsValue) -> JsResult<Value> {
    let key = value(outcome)?;
    key.check_hashable().or_throw()?;
    Ok(key)
}

/// A selection mask from JS (one byte per outcome, nonzero for selected).
fn flags(mask: &[u8]) -> Vec<bool> {
    mask.iter().map(|&m| m != 0).collect()
}

/// A truncate / clip bound: `null` for none.
fn bound(x: &JsValue) -> JsResult<Option<Value>> {
    Ok(match value(x)? {
        Value::None => None,
        v => Some(v),
    })
}

impl WasmDie {
    /// `Die(data)`, or this die itself where Python returns `self` (`None` data).
    fn new_or_self(&self, data: Option<Data>) -> JsResult<WasmDie> {
        match data {
            None => Ok(WasmDie::new(self.die.clone())),
            Some(data) => Ok(WasmDie::new(from_data(data).or_throw()?)),
        }
    }
}
