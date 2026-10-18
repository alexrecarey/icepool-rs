//! `WasmGenerator`: a keep generator of the core (generator/pool.py `Pool`, generator/deal.py `Deal`,
//! generator/compound_keep.py `CompoundKeepGenerator`), under the TypeScript `Pool` / `Deal` / `KeepGenerator`
//! (js/src/multiset.ts, deck.ts), which does
//! Python's argument juggling (pool.py, keep.py, multiset_expression.py) and calls these for the work: making the
//! pool from dice already converted, and the keep tuple arithmetic (`keep`, `multiply_counts`, merging pools).
//!
//! Multiset expressions over these stay TypeScript objects; `evaluate.rs` turns them into the core's tree when one
//! is evaluated.

use icepool_core::evaluator::CompoundKeep;
use icepool_core::evaluator::lexi::{compute_lexi_tuple, compute_lexi_tuple_with_zero_right_first};
use icepool_core::evaluator::MultisetSource;
use icepool_core::evaluator::{highest_slice, lowest_slice, middle_slice, KeepIndex, Slice, Tie};
use icepool_core::value::check_len;
use icepool_core::value::pystr::py_repr;
use icepool_core::value::recursion::level;
use icepool_core::{Deal, Die, Error, Int, Pool, Value};
use wasm_bindgen::prelude::*;

use super::convert::{int_from_js, int_to_js, quantity_to_js, value_from_js, values_to_js};
use super::deck::{deal_str, WasmDeck};
use super::die::{die_repr, WasmDie};
use super::error::{JsResult, OrThrow};

/// `Pool(dice, times)` once the TypeScript has itemized its arguments and converted each element to a die
/// (`implicit_convert_to_die`): each die with its count, in order. `build` makes the pool.
#[wasm_bindgen]
#[derive(Default)]
pub struct PoolBuilder {
    dice: Vec<(Die<Value>, usize)>,
}

#[wasm_bindgen]
impl PoolBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> PoolBuilder {
        PoolBuilder::default()
    }

    /// A die put in `count` times (a time of `itemize`, already checked to be a non-negative number).
    pub fn add(&mut self, die: &WasmDie, count: JsValue) -> JsResult<()> {
        self.dice.push((die.die.clone(), die_count(&count).or_throw()?));
        Ok(())
    }

    pub fn build(&mut self) -> JsResult<WasmGenerator> {
        let pool = Pool::new(std::mem::take(&mut self.dice)).or_throw()?;
        Ok(WasmGenerator { g: Generator::Pool(pool) })
    }
}

/// A count of dice in the pool. Python would build a huge keep tuple (`(1, ) * sum(times)`); the port refuses
/// beyond `usize`.
fn die_count(t: &JsValue) -> icepool_core::Result<usize> {
    match value_from_js(t)? {
        Value::Int(i) => usize::try_from(&i).map_err(|_| Error::Overflow("too many dice".into())),
        Value::Bool(b) => Ok(b as usize),
        // `(1, ) * sum(times)` with a float or Fraction in the sum.
        _ => Err(Error::WrongType("can't multiply sequence by non-int".into())),
    }
}

/// A keep generator of the core: a pool, a deal, or the additive union of keep generators that is still one.
#[derive(Clone)]
pub(crate) enum Generator {
    Pool(Pool<Value>),
    Deal(Deal<Value>),
    Compound(CompoundKeep<Value>),
}

impl Generator {
    /// `_make_source`.
    pub(crate) fn source(&self) -> icepool_core::Result<MultisetSource<Value>> {
        Ok(match self {
            Generator::Pool(p) => MultisetSource::Pool(p.source()),
            Generator::Deal(d) => MultisetSource::Deal(d.source()),
            Generator::Compound(c) => MultisetSource::CompoundKeep(c.source()?),
        })
    }

    fn keep_tuple(&self) -> &[Int] {
        match self {
            Generator::Pool(p) => p.keep_tuple(),
            Generator::Deal(d) => d.keep_tuple(),
            Generator::Compound(c) => c.keep_tuple(),
        }
    }

    fn keep_size(&self) -> Int {
        match self {
            Generator::Pool(p) => p.keep_size(),
            Generator::Deal(d) => d.keep_size(),
            Generator::Compound(c) => c.keep_size(),
        }
    }

    /// `KeepGenerator.keep` for a keep tuple without negative counts (the TypeScript checks).
    fn keep(&self, index: &KeepIndex) -> icepool_core::Result<Generator> {
        Ok(match self {
            Generator::Pool(p) => Generator::Pool(p.keep(index)?),
            Generator::Deal(d) => Generator::Deal(d.keep(index)?),
            Generator::Compound(c) => Generator::Compound(c.keep(index)?),
        })
    }
}

/// A keep generator of the core, held by the TypeScript `Pool` (and the other keep generators it makes).
#[wasm_bindgen]
pub struct WasmGenerator {
    pub(crate) g: Generator,
}

#[wasm_bindgen]
impl WasmGenerator {
    /// Another handle on the same generator, for a call that takes generators by value.
    pub fn copy(&self) -> WasmGenerator {
        WasmGenerator { g: self.g.clone() }
    }

    /// Whether this is a `Pool`.
    #[wasm_bindgen(js_name = isPool)]
    pub fn is_pool(&self) -> bool {
        matches!(self.g, Generator::Pool(_))
    }

    /// Whether this is a `Deal` (neither: a `CompoundKeepGenerator`).
    #[wasm_bindgen(js_name = isDeal)]
    pub fn is_deal(&self) -> bool {
        matches!(self.g, Generator::Deal(_))
    }

    /// `keep_tuple()`.
    #[wasm_bindgen(js_name = keepTuple)]
    pub fn keep_tuple(&self) -> Vec<JsValue> {
        self.g.keep_tuple().iter().map(|n| int_to_js(&n.into())).collect()
    }

    /// `keep_size()`.
    #[wasm_bindgen(js_name = keepSize)]
    pub fn keep_size(&self) -> JsValue {
        int_to_js(&self.g.keep_size().into())
    }

    /// `has_negative_keeps()`.
    #[wasm_bindgen(js_name = hasNegativeKeeps)]
    pub fn has_negative_keeps(&self) -> bool {
        self.g.keep_tuple().iter().any(Int::is_negative)
    }

    /// `keep(slice(start, stop, step))`: the bounds as ints (or null), `step` whether one was given (an
    /// `IndexError`).
    #[wasm_bindgen(js_name = keepSlice)]
    pub fn keep_slice(&self, start: JsValue, stop: JsValue, step: bool) -> JsResult<WasmGenerator> {
        let slice = Slice::new(bound(&start)?, bound(&stop)?);
        self.kept(&KeepIndex::Slice { slice, step })
    }

    /// `keep(sequence)`: one count per element, `null` for `...`.
    #[wasm_bindgen(js_name = keepSequence)]
    pub fn keep_sequence(&self, counts: Vec<JsValue>) -> JsResult<WasmGenerator> {
        let counts = counts
            .iter()
            .map(|c| if c.is_null() { Ok(None) } else { int_from_js(c, "a keep count").map(|c| Some(c.into())) })
            .collect::<icepool_core::Result<_>>()
            .or_throw()?;
        self.kept(&KeepIndex::Seq(counts))
    }

    /// `keep(int)` up to the generator Python then evaluates into a die (the keep evaluator, `evaluate.rs`).
    #[wasm_bindgen(js_name = keepIndex)]
    pub fn keep_index(&self, index: JsValue) -> JsResult<WasmGenerator> {
        // An index beyond 64 bits is out of range anyway.
        self.kept(&KeepIndex::Int(saturating(&index)?))
    }

    /// `multiply_counts(constant)` (also `generator * n` and `-generator`).
    #[wasm_bindgen(js_name = multiplyCounts)]
    pub fn multiply_counts(&self, constant: JsValue) -> JsResult<WasmGenerator> {
        let n = Int::from(int_from_js(&constant, "a count multiplier").or_throw()?);
        let g = match &self.g {
            Generator::Pool(p) => Generator::Pool(p.multiply_counts(&n)),
            Generator::Deal(d) => Generator::Deal(d.multiply_counts(&n)),
            Generator::Compound(c) => Generator::Compound(c.multiply_counts(&n)),
        };
        Ok(WasmGenerator { g })
    }

    /// `Pool.additive_union` of pools: one pool if every sorted position counts the same, else `undefined` (the
    /// TypeScript goes on to `KeepGenerator.additive_union`).
    #[wasm_bindgen(js_name = mergePools)]
    pub fn merge_pools(generators: Vec<WasmGenerator>) -> JsResult<Option<WasmGenerator>> {
        let pools = generators
            .into_iter()
            .map(|g| match g.g {
                Generator::Pool(p) => Ok(p),
                _ => Err(Error::Unported("merging a generator that is not a pool".into())),
            })
            .collect::<icepool_core::Result<Vec<_>>>()
            .or_throw()?;
        let merged = Pool::additive_union(&pools).or_throw()?;
        Ok(merged.map(|p| WasmGenerator { g: Generator::Pool(p) }))
    }

    /// `CompoundKeepGenerator(generators, (1, ) * sum(generator.keep_size() for generator in generators))`: the
    /// additive union of keep generators without negative keeps.
    pub fn compound(generators: Vec<WasmGenerator>) -> JsResult<WasmGenerator> {
        let size: Int = generators.iter().map(|g| g.g.keep_size()).sum();
        let len = icepool_core::evaluator::list_len(&size).or_throw()?;
        let sources = generators.iter().map(|g| g.g.source()).collect::<icepool_core::Result<_>>().or_throw()?;
        Ok(WasmGenerator { g: Generator::Compound(CompoundKeep::new(sources, vec![Int::ONE; len])) })
    }

    /// Whether the two have equal `hash_key`s (a `MultisetMixture` counts equal inners as one).
    #[wasm_bindgen(js_name = sameKey)]
    pub fn same_key(&self, other: &WasmGenerator) -> bool {
        match (&self.g, &other.g) {
            (Generator::Pool(a), Generator::Pool(b)) => a.dice() == b.dice() && a.keep_tuple() == b.keep_tuple(),
            // `(Deal, deck, keep_tuple)`.
            (Generator::Deal(a), Generator::Deal(b)) => a == b,
            (Generator::Compound(a), Generator::Compound(b)) => a == b,
            _ => false,
        }
    }

    /// `str(generator)`: `Pool.__str__`, `Deal.__str__` or `CompoundKeepGenerator.__str__`.
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string(&self) -> JsResult<String> {
        match &self.g {
            Generator::Deal(d) => deal_str(d),
            g => g.source().and_then(|s| source_str(&s)),
        }
        .or_throw()
    }

    // --- Deal only ---------------------------------------------------------------------------------------------

    /// `deck()`.
    pub fn deck(&self) -> JsResult<WasmDeck> {
        Ok(WasmDeck { deck: self.deal()?.deck().clone() })
    }

    /// `hand_size()`.
    #[wasm_bindgen(js_name = handSize)]
    pub fn hand_size(&self) -> JsResult<usize> {
        Ok(self.deal()?.hand_size())
    }

    /// `[repr(deal), str(deal)]`.
    #[wasm_bindgen(js_name = dealStrs)]
    pub fn deal_strs(&self) -> JsResult<Vec<String>> {
        let d = self.deal()?;
        let repr = format!("Deal({}, hand_size={})", super::deck::deck_repr(d.deck()).or_throw()?, d.hand_size());
        Ok(vec![repr, deal_str(d).or_throw()?])
    }

    // --- Pool only ---------------------------------------------------------------------------------------------

    /// `raw_size()`: the number of dice before the keep tuple is applied.
    #[wasm_bindgen(js_name = rawSize)]
    pub fn raw_size(&self) -> JsResult<usize> {
        Ok(self.pool()?.raw_size())
    }

    /// `denominator()` (also a deal's).
    pub fn denominator(&self) -> JsResult<JsValue> {
        if let Generator::Deal(d) = &self.g {
            return Ok(quantity_to_js(&d.denominator()));
        }
        Ok(quantity_to_js(&self.pool()?.denominator().or_throw()?))
    }

    /// `outcomes()`: the union of the dice's outcomes, ascending (dice put in zero times included).
    pub fn outcomes(&self) -> JsResult<JsValue> {
        values_to_js(self.pool()?.outcomes()).or_throw()
    }

    /// `_dice`: the `(die, count)` pairs (in `hash_key` order), as `[die, count, die, count, ...]`.
    pub fn dice(&self) -> JsResult<Vec<JsValue>> {
        Ok(self
            .pool()?
            .dice()
            .iter()
            .flat_map(|(die, count)| [JsValue::from(WasmDie::new(die.clone())), JsValue::from(*count)])
            .collect())
    }
}

impl WasmGenerator {
    fn kept(&self, index: &KeepIndex) -> JsResult<WasmGenerator> {
        Ok(WasmGenerator { g: self.g.keep(index).or_throw()? })
    }

    fn pool(&self) -> JsResult<&Pool<Value>> {
        match &self.g {
            Generator::Pool(p) => Ok(p),
            _ => Err(Error::MissingAttribute("this keep generator is not a Pool".into())).or_throw(),
        }
    }

    fn deal(&self) -> JsResult<&Deal<Value>> {
        match &self.g {
            Generator::Deal(d) => Ok(d),
            _ => Err(Error::MissingAttribute("this keep generator is not a Deal".into())).or_throw(),
        }
    }
}

/// `str()` of a keep generator from its source, which keeps its dice and keep tuple: `Pool.__str__` (a header, then
/// `  {repr(die)} : {count},` per die, in `hash_key` order) or `CompoundKeepGenerator.__str__` (its inners').
fn source_str(source: &MultisetSource<Value>) -> icepool_core::Result<String> {
    let _level = level()?;
    let keep_tuple_repr = |keep_tuple: &[Int]| py_repr(&Value::tuple(keep_tuple.iter().map(Value::from).collect()));
    match source {
        MultisetSource::Pool(p) => {
            let raw_size: usize = p.dice().iter().map(|(_, count)| count).sum();
            let mut text = format!("Pool of {raw_size} dice with keep_tuple={}\n", keep_tuple_repr(p.keep_tuple())?);
            for (die, count) in p.dice() {
                text += &format!("  {} : {count},\n", die_repr(die, "Die")?);
            }
            Ok(text)
        }
        // `Deal.__str__` of an inner deal.
        MultisetSource::Deal(d) => Ok(format!(
            "Deal of hand_size={} from deck:\n{}",
            d.keep_tuple().len(),
            super::deck::deck_text(d.deck())?
        )),
        MultisetSource::CompoundKeep(c) => {
            let inners = c.inner_sources().iter().map(source_str).collect::<icepool_core::Result<Vec<_>>>()?;
            Ok(format!("CompoundKeep([{}], keep_tuple={})", inners.join(", "), keep_tuple_repr(c.keep_tuple())?))
        }
        _ => Err(Error::Unported("str() of this generator".into())),
    }
}

/// An int as an `i64`, saturating: slice bounds and indexes beyond 64 bits only ever get clamped to a size.
fn saturating(x: &JsValue) -> JsResult<i64> {
    let i = int_from_js(x, "an index").or_throw()?;
    Ok(i64::try_from(&i).unwrap_or(if i < dashu_int::IBig::ZERO { i64::MIN } else { i64::MAX }))
}

/// A slice bound: an int, or null for none.
pub(super) fn bound(x: &JsValue) -> JsResult<Option<i64>> {
    if x.is_null_or_undefined() {
        return Ok(None);
    }
    saturating(x).map(Some)
}

/// lexi.py `compute_lexi_tuple(comparison, order)`, for `max_pair_keep` / `max_pair_drop`.
#[wasm_bindgen(js_name = lexiTuple)]
pub fn lexi_tuple(comparison: &str, order: i32) -> JsResult<Vec<i32>> {
    let (a, b, c) = compute_lexi_tuple(comparison, order.into()).or_throw()?;
    Ok([a, b, c].map(|x| x as i32).to_vec())
}

/// lexi.py `compute_lexi_tuple_with_zero_right_first(comparison)`, for `versus_all` / `versus_any`: the tuple,
/// then the order.
#[wasm_bindgen(js_name = lexiTupleZeroRightFirst)]
pub fn lexi_tuple_zero_right_first(comparison: &str) -> JsResult<Vec<i32>> {
    let ((a, b, c), order) = compute_lexi_tuple_with_zero_right_first(comparison).or_throw()?;
    Ok([a, b, c, order].map(|x| x as i32).to_vec())
}

/// population/keep.py `highest_slice(keep, drop)` (`highest`) or `lowest_slice(keep, drop)`: `[start, stop]`,
/// null for no bound. `keep` and `drop` are ints or null.
#[wasm_bindgen(js_name = keepDropSlice)]
pub fn keep_drop_slice(highest: bool, keep: JsValue, drop: JsValue) -> JsResult<Vec<JsValue>> {
    let (keep, drop) = (bound(&keep)?, bound(&drop)?);
    let slice = if highest { highest_slice(keep, drop) } else { lowest_slice(keep, drop) }.or_throw()?;
    Ok(slice_bounds(slice))
}

/// `KeepGenerator.middle(keep, tie=tie)` up to the slice it keeps of `keep_size` elements: `[start, stop]`.
#[wasm_bindgen(js_name = middleSlice)]
pub fn middle_slice_of(keep_size: JsValue, keep: JsValue, tie: JsValue) -> JsResult<Vec<JsValue>> {
    let keep_size = Int::from(int_from_js(&keep_size, "a keep size").or_throw()?);
    let keep = Int::from(int_from_js(&keep, "keep").or_throw()?);
    // Anything but these three is only refused once the tiebreaker is consulted.
    let tie = match tie.as_string().as_deref() {
        Some("error") => Tie::Error,
        Some("high") => Tie::High,
        Some("low") => Tie::Low,
        _ => Tie::Invalid,
    };
    Ok(slice_bounds(middle_slice(&keep_size, &keep, tie).or_throw()?))
}

/// `d_pool` of a mapping builds `itertools.chain.from_iterable([k] * v for k, v in die_sizes.items())`: the
/// length of that list once `count` more copies of a size are added to the `len` it has (a negative count adds
/// none), or Python's `MemoryError` past what memory holds (as the program runner checks it).
#[wasm_bindgen(js_name = standardPoolLength)]
pub fn standard_pool_length(len: usize, count: JsValue) -> JsResult<usize> {
    let count = int_from_js(&count, "a die count").or_throw()?;
    let count = if count < dashu_int::IBig::ZERO { 0 } else { u64::try_from(&count).unwrap_or(u64::MAX) };
    check_len::<Value, _>((len as u64).checked_add(count), "d_pool(die_sizes)").or_throw()
}

fn slice_bounds(slice: Slice) -> Vec<JsValue> {
    let bound = |x: Option<i64>| x.map_or(JsValue::NULL, |x| int_to_js(&x.into()));
    vec![bound(slice.start), bound(slice.stop)]
}
