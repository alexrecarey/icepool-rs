//! Port of icepool `population/deck.py`: sampling without replacement (within a single evaluation). Quantities
//! represent duplicates.
//!
//! A deck's data is a `Counts`, as a die's is, and the `Population` methods (statistics, `_pop_min` /
//! `_pop_max`, ...) mean the same on both: the deck keeps its cards in a `Die` ([`Deck::population`]) and shares
//! them. What differs is construction (`merge_duplicates`: duplicates add up, nothing is rescaled) and the multiset
//! arithmetic, which goes through Python's `Counter` ([`Counter`]).

use crate::ownership::{Owners, Traverse};
use dashu_int::IBig;
use rustc_hash::FxHashMap;

use crate::counts::Counts;
use crate::creation_args::{expand_args_for_deck, DieArg};
use crate::die::Die;
use crate::error::Result;
use crate::outcome::{ErrInto, OutcomeOrder};
use crate::Quantity;

/// A deck of cards. Immutable and cheap to clone (the cards are shared). Equality and hashing are structural, as
/// `hash_key` `(Deck, tuple(items))`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Deck<O> {
    cards: Die<O>,
}

impl<O: OutcomeOrder + Traverse> Traverse for Deck<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        self.cards.traverse(owners);
    }
}

impl<O: OutcomeOrder> Deck<O> {
    /// `Deck(outcomes, times)` after `itemize`: one duplicate count per element of `outcomes`.
    ///
    /// Outcomes with zero quantity are omitted. A die (or deck) among the outcomes is flattened into the result,
    /// each of its outcomes duplicated by its quantity times the argument's. (`Deck([deck])` returning `deck`
    /// itself is the caller's: it knows which arguments are decks.)
    pub fn new(outcomes: &[DieArg<O>], times: &[Quantity]) -> Result<Self> {
        Ok(Self::from_counts(expand_args_for_deck(outcomes, times)?))
    }

    /// `Deck._new_raw`: wraps already-processed data.
    pub fn from_counts(data: Counts<O>) -> Self {
        Deck {
            cards: Die::from_counts(data),
        }
    }

    /// `Deck(dict)` for items whose outcomes are distinct and whose quantities are ints: zero quantities drop out
    /// (`merge_duplicates`), then `Counts` sorts what is left.
    pub fn from_dict(items: Vec<(O, Quantity)>) -> std::result::Result<Self, O::Incomparable> {
        let items = items.into_iter().filter(|(_, q)| !q.is_zero()).collect();
        Ok(Self::from_counts(Counts::new(items)?))
    }

    /// `Population.split` of a deck given the selection (one flag per outcome, the caller's `_select_outcomes`):
    /// the selected cards and the rest, each as `Deck(dict)` (zero quantities drop out).
    pub fn split(&self, selected: &[bool]) -> std::result::Result<(Self, Self), O::Incomparable> {
        let (sel, rest): (Vec<_>, Vec<_>) =
            self.items().iter().zip(selected).partition(|(_, &s)| s);
        let unflag =
            |v: Vec<(&(O, Quantity), &bool)>| v.into_iter().map(|(i, _)| i.clone()).collect();
        Ok((
            Self::from_dict(unflag(sel))?,
            Self::from_dict(unflag(rest))?,
        ))
    }

    /// The cards as a population: a die with the same `Counts`, for the `Population` methods both share.
    pub fn population(&self) -> &Die<O> {
        &self.cards
    }

    /// Every `(outcome, quantity)` in sorted order.
    pub fn items(&self) -> &[(O, Quantity)] {
        self.cards.items()
    }

    /// The distinct cards in ascending order.
    pub fn outcomes(&self) -> impl ExactSizeIterator<Item = &O> {
        self.cards.outcomes()
    }

    /// The number of distinct outcomes (`len(deck)`).
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Whether the deck has no cards at all.
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// The number of cards (`size = Population.denominator`).
    pub fn size(&self) -> Quantity {
        self.cards.denominator()
    }

    /// `Deck._pop_min`: a deck with the min outcome removed, and its quantity (`IndexError` for an empty deck).
    pub fn pop_min(&self) -> Result<(Self, Quantity)> {
        let (cards, q) = self.cards.pop_min()?;
        Ok((Deck { cards }, q))
    }

    /// `Deck._pop_max`: a deck with the max outcome removed, and its quantity.
    pub fn pop_max(&self) -> Result<(Self, Quantity)> {
        let (cards, q) = self.cards.pop_max()?;
        Ok((Deck { cards }, q))
    }

    /// `self.get(outcome, 0)`: the quantity of an outcome, 0 if the deck does not have it.
    fn get(&self, outcome: &O) -> IBig {
        self.items()
            .iter()
            .find(|(o, _)| o == outcome)
            .map_or(IBig::ZERO, |(_, q)| q.clone().into())
    }

    /// The binary operators of a deck with `Counter(other)`: `+`, `-` (and reflected), `&`, `|`, `^`.
    ///
    /// Each builds a `Counter` (`self`'s cards first, except for `other - self`), combines the counts, and
    /// returns `Deck(+data)`: only positive counts are kept. Keys merge as in a dict, so a key keeps the spelling
    /// of whichever side put it in first.
    ///
    /// `Deck(+data)` expands each key as `Deck(...)` expands an outcome; keys made of plain outcomes are outcomes
    /// as they are. (The Python facade, whose `other` may hold dice or `Reroll`, gets [`Deck::counter_items`] and
    /// calls `Deck(...)` itself.)
    pub fn counter_op(&self, op: CounterOp, other: &Counter<O>) -> Result<Self> {
        Self::from_dict(self.counter_items(op, other)).err_into()
    }

    /// The items of `+data` in [`Deck::counter_op`].
    pub fn counter_items(&self, op: CounterOp, other: &Counter<O>) -> Vec<(O, Quantity)> {
        let combine = |data: &mut Counter<O>, f: &dyn Fn(IBig, &IBig) -> IBig| {
            for (outcome, count) in other.items() {
                let current = data.get(outcome);
                data.set(outcome.clone(), f(current, count));
            }
        };
        let data = match op {
            CounterOp::Add => {
                let mut data = Counter::from_population(self.items());
                combine(&mut data, &|a, b| a + b);
                data
            }
            CounterOp::Sub => {
                let mut data = Counter::from_population(self.items());
                combine(&mut data, &|a, b| a - b);
                data
            }
            CounterOp::RSub => {
                let mut data = other.clone();
                for (outcome, count) in self.items() {
                    let current = data.get(outcome);
                    data.set(outcome.clone(), current - IBig::from(count.clone()));
                }
                data
            }
            CounterOp::And => {
                // `data[outcome] = min(self.get(outcome, 0), count)`: the keys are `other`'s.
                let mut data = Counter::default();
                for (outcome, count) in other.items() {
                    data.set(outcome.clone(), self.get(outcome).min(count.clone()));
                }
                data
            }
            CounterOp::Or => {
                let mut data = Counter::from_population(self.items());
                combine(&mut data, &|a, b| a.max(b.clone()));
                data
            }
            CounterOp::Xor => {
                let mut data = Counter::from_population(self.items());
                combine(&mut data, &|a, b| {
                    IBig::from(dashu_base::UnsignedAbs::unsigned_abs(a - b))
                });
                data
            }
        };
        data.positive()
    }
}

impl<O: crate::outcome::OutcomeArith> Deck<O> {
    /// `Population.zero` of a deck: every outcome times 0 (`_unary_operator`, merging equal results), then
    /// `Deck(data)`; `ValueError` unless they resolve to a single outcome.
    pub fn zero(&self) -> Result<Self> {
        let zeros = self.cards.unary_operator(O::try_zero)?;
        let result = Self::from_dict(zeros.items().to_vec()).err_into()?;
        if result.len() != 1 {
            return crate::error::invalid_argument("zero() did not resolve to a single outcome.");
        }
        Ok(result)
    }
}

/// The binary operators of `Deck` (each also has a method form, e.g. `additive_union`, that reduces over `+`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterOp {
    /// `deck + other` (and `other + deck`: `__radd__ = __add__`, `self`'s cards still first).
    Add,
    /// `deck - other`: this deck with the other cards removed (but not below zero of each card).
    Sub,
    /// `other - deck`.
    RSub,
    /// `deck & other` (and `__rand__`): the cards that both have.
    And,
    /// `deck | other` (and `__ror__`): as many of each card as the side that has more of them.
    Or,
    /// `deck ^ other`: the difference of the counts of each card (there is no `__rxor__`).
    Xor,
}

/// Python's `collections.Counter` as deck arithmetic uses it: a dict of int counts in insertion order. A missing
/// key counts 0 (without being inserted); setting an existing key keeps its first spelling (`True` before `1`).
#[derive(Clone, Debug)]
pub struct Counter<O> {
    index: FxHashMap<O, usize>,
    items: Vec<(O, IBig)>,
}

impl<O: OutcomeOrder> Default for Counter<O> {
    fn default() -> Self {
        Counter {
            index: FxHashMap::default(),
            items: Vec::new(),
        }
    }
}

impl<O: OutcomeOrder> Counter<O> {
    /// `Counter(iterable)`: each element counted once.
    pub fn from_elements(elements: impl IntoIterator<Item = O>) -> Self {
        let mut counter = Counter::default();
        for e in elements {
            let n = counter.get(&e) + IBig::ONE;
            counter.set(e, n);
        }
        counter
    }

    /// `Counter(mapping)`: `dict.update` with the mapping's items (a later equal key overwrites the count).
    pub fn from_mapping(items: impl IntoIterator<Item = (O, IBig)>) -> Self {
        let mut counter = Counter::default();
        for (o, c) in items {
            counter.set(o, c);
        }
        counter
    }

    /// `Counter(population)`: its outcomes and quantities.
    pub fn from_population(items: &[(O, Quantity)]) -> Self {
        Self::from_mapping(items.iter().map(|(o, q)| (o.clone(), q.clone().into())))
    }

    /// Every `(outcome, count)` in insertion order.
    pub fn items(&self) -> &[(O, IBig)] {
        &self.items
    }

    /// `counter[key]`: 0 if missing.
    pub fn get(&self, key: &O) -> IBig {
        self.index
            .get(key)
            .map_or(IBig::ZERO, |&i| self.items[i].1.clone())
    }

    /// `counter[key] = count`.
    pub fn set(&mut self, key: O, count: IBig) {
        match self.index.get(&key) {
            Some(&i) => self.items[i].1 = count,
            None => {
                self.index.insert(key.clone(), self.items.len());
                self.items.push((key, count));
            }
        }
    }

    /// `+counter`: the items with a positive count, in order.
    pub fn positive(self) -> Vec<(O, Quantity)> {
        self.items
            .into_iter()
            .filter_map(|(o, c)| Quantity::try_from(c).ok().map(|q| (o, q)))
            .filter(|(_, q)| !q.is_zero())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn deck(xs: &[i64]) -> Deck<Value> {
        let args: Vec<_> = xs
            .iter()
            .map(|&x| DieArg::Outcome(Value::from(x)))
            .collect();
        Deck::new(&args, &vec![Quantity::ONE; xs.len()]).unwrap()
    }

    fn counter(xs: &[i64]) -> Counter<Value> {
        Counter::from_elements(xs.iter().map(|&x| Value::from(x)))
    }

    #[test]
    fn arithmetic_like_upstream_tests() {
        let a = deck(&[1, 2, 3]);
        let b = counter(&[3, 4]);
        assert_eq!(
            a.counter_op(CounterOp::Add, &b).unwrap(),
            deck(&[1, 2, 3, 3, 4])
        );
        assert_eq!(a.counter_op(CounterOp::Sub, &b).unwrap(), deck(&[1, 2]));
        assert_eq!(a.counter_op(CounterOp::And, &b).unwrap(), deck(&[3]));
        assert_eq!(
            a.counter_op(CounterOp::Or, &b).unwrap(),
            deck(&[1, 2, 3, 4])
        );
        assert_eq!(a.counter_op(CounterOp::Xor, &b).unwrap(), deck(&[1, 2, 4]));
        assert_eq!(a.counter_op(CounterOp::RSub, &b).unwrap(), deck(&[4]));
    }
}
