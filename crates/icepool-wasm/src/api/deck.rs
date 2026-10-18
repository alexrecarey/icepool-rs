//! `WasmDeck` and `WasmMultiDeal`: a deck and a multi-deal of the core, under the TypeScript `Deck` and `MultiDeal`
//! (js/src/deck.ts), which do Python's argument juggling (population/deck.py, generator/deal.py,
//! generator/multi_deal.py) and call these for the work. A `Deal` is a keep generator (`multiset.rs`).
//!
//! The `Population` methods a deck shares with a die (statistics, the mapping protocol, formatting) run on the
//! deck's cards as a die (`population`), as in the Python facade (crates/icepool-py/src/deck.rs).

use std::borrow::Cow;

use dashu_int::IBig;
use icepool_core::creation_args::DieArg;
use icepool_core::format::{format as format_population, Formatted};
use icepool_core::population::Data;
use icepool_core::value::pystr::py_repr;
use icepool_core::value::{Counter, CounterOp, Num};
use icepool_core::{Deal, Deck, Die, Error, MultiDeal, Quantity, Value};
use wasm_bindgen::prelude::*;

use super::convert::{int_from_js, quantity_to_js, value_from_js};
use super::die::WasmDie;
use super::error::{JsResult, OrThrow};
use super::multiset::{Generator, WasmGenerator};

/// A deck of the core.
#[wasm_bindgen]
pub struct WasmDeck {
    pub(crate) deck: Deck<Value>,
}

/// `Deck(outcomes, times)` once the TypeScript has itemized its arguments (creation_args.py `itemize`) and expanded
/// each (`expand_arg`: a die or deck stays a population, a tuple holding dice is their Cartesian product): each
/// argument with its time, a non-negative number. `build` merges them (`expand_args_for_deck`, `merge_duplicates`).
#[wasm_bindgen]
#[derive(Default)]
pub struct DeckBuilder {
    args: Vec<(DieArg<Value>, Value)>,
    /// Whether some argument was a tuple holding `Reroll` with a nonzero time (see `rerollTuple`).
    reroll_tuple: bool,
}

#[wasm_bindgen]
impl DeckBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> DeckBuilder {
        DeckBuilder::default()
    }

    /// An outcome: one card, times `time`.
    pub fn outcome(&mut self, outcome: JsValue, time: JsValue) -> JsResult<()> {
        self.push(DieArg::Outcome(value_from_js(&outcome).or_throw()?), &time)
    }

    /// A die: its outcomes, each duplicated by its quantity times `time`.
    pub fn die(&mut self, die: &WasmDie, time: JsValue) -> JsResult<()> {
        self.push(DieArg::Die(die.die.clone()), &time)
    }

    /// A deck, flattened into the result (`time` duplicates the whole deck).
    pub fn deck(&mut self, deck: &WasmDeck, time: JsValue) -> JsResult<()> {
        self.push(DieArg::Die(deck.deck.population().clone()), &time)
    }

    /// `Reroll` (or `Restart`): `expand_arg` makes it `{}`, no cards.
    pub fn reroll(&mut self, time: JsValue) -> JsResult<()> {
        self.push(DieArg::Reroll, &time)
    }

    /// A tuple holding `Reroll`: `cartesian_product` makes it `Reroll` itself, which `expand_arg` passes on as it
    /// is (it is not in `REROLL_TYPES` by then: the check came first), so `merge_duplicates` asks it for `items()`
    /// unless its time is zero.
    #[wasm_bindgen(js_name = rerollTuple)]
    pub fn reroll_tuple(&mut self, time: JsValue) -> JsResult<()> {
        let time = value_from_js(&time).or_throw()?;
        self.reroll_tuple |= !is_zero(&time);
        self.args.push((DieArg::Reroll, time));
        Ok(())
    }

    pub fn build(&mut self) -> JsResult<WasmDeck> {
        // `merge_duplicates` raises at the `Reroll` as it meets it; a non-int duplicate only raises afterwards,
        // when `Counts` checks the merged quantities.
        if std::mem::take(&mut self.reroll_tuple) {
            return Err(Error::MissingAttribute("'RerollType' object has no attribute 'items'".into())).or_throw();
        }
        let (args, times): (Vec<_>, Vec<_>) = std::mem::take(&mut self.args).into_iter().unzip();
        let duplicates =
            args.iter().zip(&times).map(|(arg, t)| duplicate(arg, t)).collect::<icepool_core::Result<Vec<_>>>();
        Ok(WasmDeck { deck: Deck::new(&args, &duplicates.or_throw()?).or_throw()? })
    }
}

impl DeckBuilder {
    fn push(&mut self, arg: DieArg<Value>, time: &JsValue) -> JsResult<()> {
        self.args.push((arg, value_from_js(time).or_throw()?));
        Ok(())
    }
}

/// Python's `t == 0` for a number.
fn is_zero(t: &Value) -> bool {
    t.as_num().is_some_and(|n| n == Num::Int(Cow::Owned(IBig::ZERO)))
}

/// One time as `merge_duplicates` uses it (`itemize` has checked it is a non-negative number). An int is the
/// duplicate count. Any other number is skipped if it is zero (`subdup == 0`); otherwise it multiplies the
/// argument's duplicates into a non-int, which `Counts` rejects (`ValueError`), unless the argument contributes no
/// cards at all.
fn duplicate(arg: &DieArg<Value>, t: &Value) -> icepool_core::Result<Quantity> {
    match t {
        Value::Int(i) => {
            Quantity::try_from(i.clone()).map_err(|_| Error::InvalidArgument("times cannot have negative values.".into()))
        }
        Value::Bool(b) => Ok(Quantity::from(*b as u8)),
        t if is_zero(t) => Ok(Quantity::ZERO),
        t => {
            let contributes = match arg {
                DieArg::Outcome(_) => true,
                DieArg::Die(d) => d.items().iter().any(|(_, q)| !q.is_zero()),
                DieArg::Reroll => false,
            };
            if contributes {
                Err(Error::InvalidArgument(format!("Values must be ints, got {}", t.type_name())))
            } else {
                Ok(Quantity::ZERO)
            }
        }
    }
}

/// `Deck(data)` for the `(outcome, quantity)` pairs a `Population` method computed (`self._new_type(data)`):
/// `itemize` refuses negative quantities (and anything not a number), then each is a duplicate count.
fn deck_from_data(data: Data) -> icepool_core::Result<Deck<Value>> {
    let zero = Num::Int(Cow::Owned(IBig::ZERO));
    let mut args = Vec::with_capacity(data.len());
    let mut duplicates = Vec::with_capacity(data.len());
    for (outcome, q) in data {
        // `itemize`: `any(x < 0 for x in times)`.
        match q.as_num() {
            Some(n) if n < zero => return Err(Error::InvalidArgument("times cannot have negative values.".into())),
            Some(_) => {}
            None => return Err(Error::Incomparable { op: "<", left: q.type_name(), right: "int" }),
        }
        let arg = DieArg::Outcome(outcome);
        duplicates.push(duplicate(&arg, &q)?);
        args.push(arg);
    }
    Deck::new(&args, &duplicates)
}

/// A deck operator by its Python name: `add` (also `__radd__`), `sub`, `rsub`, `and`, `or`, `xor`.
fn counter_op(op: &str) -> JsResult<CounterOp> {
    Ok(match op {
        "add" => CounterOp::Add,
        "sub" => CounterOp::Sub,
        "rsub" => CounterOp::RSub,
        "and" => CounterOp::And,
        "or" => CounterOp::Or,
        "xor" => CounterOp::Xor,
        _ => return Err(Error::InvalidArgument(format!("no deck operator {op}"))).or_throw(),
    })
}

/// A hashable outcome for a `Counter` key.
fn key(x: &JsValue) -> JsResult<Value> {
    let k = value_from_js(x).or_throw()?;
    k.check_hashable().or_throw()?;
    Ok(k)
}

#[wasm_bindgen]
impl WasmDeck {
    /// The cards as a die, for the `Population` methods a deck shares with a die.
    pub fn population(&self) -> WasmDie {
        WasmDie::new(self.deck.population().clone())
    }

    /// Another handle on the same deck.
    pub fn copy(&self) -> WasmDeck {
        WasmDeck { deck: self.deck.clone() }
    }

    /// `deck == other`: their `hash_key`s `(Deck, tuple(items))`.
    pub fn equals(&self, other: &WasmDeck) -> bool {
        self.deck == other.deck
    }

    /// `deck <op> other` with `Counter(other)` of an iterable: its elements, counted.
    #[wasm_bindgen(js_name = counterElements)]
    pub fn counter_elements(&self, op: &str, elements: Vec<JsValue>) -> JsResult<WasmDeck> {
        let elements = elements.iter().map(key).collect::<JsResult<Vec<_>>>()?;
        self.counter(op, Counter::from_elements(elements))
    }

    /// `deck <op> other` with `Counter(other)` of a mapping: its keys with their int counts.
    #[wasm_bindgen(js_name = counterMapping)]
    pub fn counter_mapping(&self, op: &str, keys: Vec<JsValue>, counts: Vec<JsValue>) -> JsResult<WasmDeck> {
        let items = keys
            .iter()
            .zip(&counts)
            .map(|(k, c)| Ok((key(k)?, int_from_js(c, "a count").or_throw()?)))
            .collect::<JsResult<Vec<_>>>()?;
        self.counter(op, Counter::from_mapping(items))
    }

    /// `deck <op> other` for another deck or a die (a mapping of its quantities).
    #[wasm_bindgen(js_name = counterPopulation)]
    pub fn counter_population(&self, op: &str, other: &WasmDie) -> JsResult<WasmDeck> {
        self.counter(op, Counter::from_population(other.die.items()))
    }

    /// `deck.multiply_quantities(n)` (`deck * n`), `divide_quantities` (`//`) or `modulo_quantities` (`%`) by
    /// name, for an int `n`.
    pub fn quantities(&self, op: &str, n: JsValue) -> JsResult<WasmDeck> {
        let n = value_from_js(&n).or_throw()?;
        let cards = self.deck.population();
        let data = match op {
            "mul" => cards.multiply_quantities(&n),
            "floordiv" => cards.divide_quantities(&n),
            "mod" => cards.modulo_quantities(&n).map(Some),
            _ => return Err(Error::InvalidArgument(format!("no quantity operator {op}"))).or_throw(),
        };
        self.new_or_self(data.or_throw()?)
    }

    /// `deck.pad_to_denominator(denominator, outcome)`.
    #[wasm_bindgen(js_name = padToDenominator)]
    pub fn pad_to_denominator(&self, denominator: JsValue, outcome: JsValue) -> JsResult<WasmDeck> {
        let (denominator, outcome) = (value_from_js(&denominator).or_throw()?, value_from_js(&outcome).or_throw()?);
        let data = self.deck.population().pad_to_denominator(&denominator, &outcome).or_throw()?;
        self.new_or_self(Some(data))
    }

    /// `deck.append(outcome, quantity)`.
    pub fn append(&self, outcome: JsValue, quantity: JsValue) -> JsResult<WasmDeck> {
        let data = self.deck.population().append(&key(&outcome)?, &value_from_js(&quantity).or_throw()?).or_throw()?;
        self.new_or_self(Some(data))
    }

    /// `deck.remove(outcome, quantity)`: `null` removes all of it.
    pub fn remove(&self, outcome: JsValue, quantity: JsValue) -> JsResult<WasmDeck> {
        let quantity = match value_from_js(&quantity).or_throw()? {
            Value::None => None,
            q => Some(q),
        };
        let data = self.deck.population().remove(&key(&outcome)?, quantity.as_ref()).or_throw()?;
        self.new_or_self(Some(data))
    }

    /// `deck.split(which)` once the TypeScript has selected the outcomes (one flag per outcome): `[selected,
    /// rest]`.
    pub fn split(&self, mask: Vec<u8>) -> JsResult<Vec<WasmDeck>> {
        let selected: Vec<bool> = mask.iter().map(|&m| m != 0).collect();
        let (a, b) = self.deck.split(&selected).or_throw()?;
        Ok(vec![WasmDeck { deck: a }, WasmDeck { deck: b }])
    }

    /// `deck.zero()`.
    pub fn zero(&self) -> JsResult<WasmDeck> {
        Ok(WasmDeck { deck: self.deck.zero().or_throw()? })
    }

    /// `deck.sequence(deals)` for `deals >= 1` (the TypeScript checks the rest, and Python's cache): the sequences
    /// of `deals` cards dealt one by one, as a die of tuples.
    ///
    /// Python builds them step by step from `Die([()])`: each step maps every sequence so far to the die of the
    /// cards left (`Die(self - curr)`) appended to it.
    pub fn sequence(&self, deals: u32) -> JsResult<WasmDie> {
        let step = |result: &Die<Value>| -> icepool_core::Result<Die<Value>> {
            let mut transitions = Vec::with_capacity(result.len());
            for (curr, _) in result.items() {
                let Value::Tuple(cards) = curr else { unreachable!("sequences are tuples") };
                // `remaining = icepool.Die(self - curr)`.
                let remaining = self.deck.counter_op(CounterOp::Sub, &Counter::from_elements(cards.iter().cloned()))?;
                // `icepool.map(lambda curr, next: curr + (next, ), curr, remaining)`.
                let (outcomes, quantities): (Vec<_>, Vec<_>) = remaining
                    .items()
                    .iter()
                    .map(|(next, q)| {
                        let mut seq = cards.to_vec();
                        seq.push(next.clone());
                        (DieArg::Outcome(Value::tuple(seq)), q.clone())
                    })
                    .unzip();
                transitions.push(DieArg::Die(Die::new(outcomes, quantities)?));
            }
            Die::new(transitions, result.items().iter().map(|(_, q)| q.clone()).collect())
        };
        let mut result = Die::from_dict(vec![(Value::tuple(vec![]), Quantity::ONE)]).or_throw()?;
        for _ in 0..deals {
            result = step(&result).or_throw()?;
        }
        Ok(WasmDie::new(result))
    }

    /// `Deal(deck, hand_size)` for an int (the TypeScript checks the type).
    pub fn deal(&self, hand_size: JsValue) -> JsResult<WasmGenerator> {
        let hand_size: IBig = int_from_js(&hand_size, "a hand size").or_throw()?;
        Ok(WasmGenerator { g: Generator::Deal(Deal::new(self.deck.clone(), &hand_size).or_throw()?) })
    }

    /// `MultiDeal(deck, hand_groups)`: `[hand_size, group_size, ...]` flat, ints (the TypeScript checks).
    #[wasm_bindgen(js_name = multiDeal)]
    pub fn multi_deal(&self, hand_groups: Vec<JsValue>) -> JsResult<WasmMultiDeal> {
        let ints = hand_groups
            .iter()
            .map(|x| int_from_js(x, "a hand size"))
            .collect::<icepool_core::Result<Vec<IBig>>>()
            .or_throw()?;
        let groups: Vec<(IBig, IBig)> = ints.chunks(2).map(|g| (g[0].clone(), g[1].clone())).collect();
        Ok(WasmMultiDeal { md: MultiDeal::new(self.deck.clone(), &groups).or_throw()? })
    }

    /// `repr(deck)`: `Deck({outcome!r: quantity, ...})`.
    pub fn repr(&self) -> JsResult<String> {
        deck_repr(&self.deck).or_throw()
    }

    /// `format(deck, spec)` (`''` is `str(deck)`, its table).
    pub fn format(&self, spec: &str) -> JsResult<String> {
        deck_format(&self.deck, spec).or_throw()
    }
}

impl WasmDeck {
    fn counter(&self, op: &str, other: Counter<Value>) -> JsResult<WasmDeck> {
        Ok(WasmDeck { deck: self.deck.counter_op(counter_op(op)?, &other).or_throw()? })
    }

    /// `Deck(data)`, or this deck itself where Python returns `self` (`None` data).
    fn new_or_self(&self, data: Option<Data>) -> JsResult<WasmDeck> {
        match data {
            None => Ok(self.copy()),
            Some(data) => Ok(WasmDeck { deck: deck_from_data(data).or_throw()? }),
        }
    }
}

/// `repr(deck)`.
pub(super) fn deck_repr(deck: &Deck<Value>) -> icepool_core::Result<String> {
    let items = deck
        .items()
        .iter()
        .map(|(o, q)| Ok(format!("{}: {q}", py_repr(o)?)))
        .collect::<icepool_core::Result<Vec<_>>>()?;
    Ok(format!("Deck({{{}}})", items.join(", ")))
}

/// `format(deck, spec)`.
fn deck_format(deck: &Deck<Value>, spec: &str) -> icepool_core::Result<String> {
    match format_population(deck.population(), "Deck", spec, false)? {
        Formatted::Text(s) => Ok(s),
        Formatted::CsvTable(..) => unreachable!("only asked for with csv writer arguments"),
    }
}

/// `str(deck)`: its table.
pub(super) fn deck_text(deck: &Deck<Value>) -> icepool_core::Result<String> {
    deck_format(deck, "")
}

/// `str(deal)`.
pub(super) fn deal_str(deal: &Deal<Value>) -> icepool_core::Result<String> {
    Ok(format!("Deal of hand_size={} from deck:\n{}", deal.hand_size(), deck_text(deal.deck())?))
}

/// A multi-deal of the core: a multiset tuple generator, under the TypeScript `MultiDeal`.
#[wasm_bindgen]
pub struct WasmMultiDeal {
    pub(crate) md: MultiDeal<Value>,
}

#[wasm_bindgen]
impl WasmMultiDeal {
    /// `deck()`.
    pub fn deck(&self) -> WasmDeck {
        WasmDeck { deck: self.md.deck().clone() }
    }

    /// `hand_sizes()`.
    #[wasm_bindgen(js_name = handSizes)]
    pub fn hand_sizes(&self) -> Vec<usize> {
        self.md.hand_sizes()
    }

    /// `total_cards_dealt()`.
    #[wasm_bindgen(js_name = totalCardsDealt)]
    pub fn total_cards_dealt(&self) -> usize {
        self.md.total_cards_dealt()
    }

    /// `len(multi_deal)`: the number of hands.
    #[wasm_bindgen(getter)]
    pub fn length(&self) -> usize {
        self.md.len()
    }

    /// `denominator()`.
    pub fn denominator(&self) -> JsValue {
        quantity_to_js(&self.md.denominator())
    }

    /// Whether the two have equal `hash_key`s `(MultiDeal, deck, hand_groups)`.
    #[wasm_bindgen(js_name = sameKey)]
    pub fn same_key(&self, other: &WasmMultiDeal) -> bool {
        self.md.deck() == other.md.deck() && self.md.hand_groups() == other.md.hand_groups()
    }

    /// `[repr(multi_deal), str(multi_deal)]`.
    pub fn strs(&self) -> JsResult<Vec<String>> {
        let groups = Value::tuple(
            self.md
                .hand_groups()
                .iter()
                .map(|&(h, g)| Value::tuple(vec![Value::Int(IBig::from(h)), Value::Int(IBig::from(g))]))
                .collect(),
        );
        let groups = py_repr(&groups).or_throw()?;
        let repr = format!("MultiDeal({}, hand_groups={groups})", deck_repr(self.md.deck()).or_throw()?);
        let text = format!("MultiDeal of hand_groups={groups} from deck:\n{}", deck_format(self.md.deck(), "").or_throw()?);
        Ok(vec![repr, text])
    }
}
