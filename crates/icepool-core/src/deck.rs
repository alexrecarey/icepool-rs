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
use crate::deal::Deal;
use crate::outcome::{ErrInto, Outcome, OutcomeOrder};
use crate::Quantity;

/// A deck of cards. Immutable and cheap to clone (the cards are shared). Equality and hashing are structural, as
/// `hash_key` `(Deck, tuple(items))`.
///
/// ```
/// use icepool_core::Deck;
/// // Python's `Deck([1, 2, 2, 3])`: duplicates add up, nothing is rescaled.
/// let deck = Deck::from_iter([1, 2, 2, 3]);
/// assert_eq!(deck.repr().unwrap(), "Deck({1: 1, 2: 2, 3: 1})");
/// assert_eq!(deck.size().to_string(), "4");
/// ```
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
    ///
    /// ```
    /// use icepool_core::value::DieArg;
    /// use icepool_core::{d, Deck, Quantity};
    /// // Python's `Deck([1, d(2)], times=[1, 2])`: the die's cards are duplicated by the times, and add to the 1.
    /// let args = [DieArg::Outcome(1), DieArg::Die(d(2))];
    /// let deck = Deck::new(&args, &[Quantity::ONE, Quantity::from(2u8)]).unwrap();
    /// assert_eq!(deck, Deck::from_weights([(1, 3u32), (2, 2)]));
    /// ```
    pub fn new(outcomes: &[DieArg<O>], times: &[Quantity]) -> Result<Self> {
        Ok(Self::from_counts(expand_args_for_deck(outcomes, times)?))
    }

    /// `Deck._new_raw`: wraps already-processed data.
    ///
    /// ```
    /// use icepool_core::value::Counts;
    /// use icepool_core::{Deck, Quantity};
    /// let counts = Counts::new(vec![(2, Quantity::ONE), (1, Quantity::from(3u8))]).unwrap();
    /// assert_eq!(Deck::from_counts(counts), Deck::from_iter([1, 1, 1, 2]));
    /// ```
    pub fn from_counts(data: Counts<O>) -> Self {
        Deck {
            cards: Die::from_counts(data),
        }
    }

    /// `Deck(dict)` for items whose outcomes are distinct and whose quantities are ints: zero quantities drop out
    /// (`merge_duplicates`), then `Counts` sorts what is left.
    ///
    /// ```
    /// use icepool_core::{Deck, Quantity};
    /// // Python's `Deck({1: 2, 2: 1, 3: 0})`.
    /// let q = Quantity::from;
    /// let deck = Deck::from_dict(vec![(1, q(2u8)), (2, q(1u8)), (3, q(0u8))]).unwrap();
    /// assert_eq!((deck.len(), deck.size()), (2, q(3u8)));
    /// ```
    pub fn from_dict(items: Vec<(O, Quantity)>) -> std::result::Result<Self, O::Incomparable> {
        let items = items.into_iter().filter(|(_, q)| !q.is_zero()).collect();
        Ok(Self::from_counts(Counts::new(items)?))
    }

    /// `Population.split` of a deck given the selection (one flag per outcome, the caller's `_select_outcomes`):
    /// the selected cards and the rest, each as `Deck(dict)` (zero quantities drop out).
    ///
    /// ```
    /// use icepool_core::{Deck, Quantity};
    /// let deck = Deck::from_dict((1..=4).map(|o| (o, Quantity::ONE)).collect()).unwrap();
    /// let (low, high) = deck.split(&[true, true, false, false]).unwrap();
    /// assert_eq!((low.len(), high.outcomes().next()), (2, Some(&3)));
    /// ```
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
    ///
    /// ```
    /// use icepool_core::{Deck, Die, Quantity};
    /// let deck = Deck::from_dict(vec![(1, Quantity::from(2u8)), (2, Quantity::ONE)]).unwrap();
    /// assert_eq!(*deck.population(), Die::from_iter([1, 1, 2]));
    /// ```
    pub fn population(&self) -> &Die<O> {
        &self.cards
    }

    /// Every `(outcome, quantity)` in sorted order.
    ///
    /// ```
    /// use icepool_core::{Deck, Quantity};
    /// let deck = Deck::from_dict(vec![(2, Quantity::ONE), (1, Quantity::ONE)]).unwrap();
    /// assert_eq!(deck.items(), [(1, Quantity::ONE), (2, Quantity::ONE)]); // sorted
    /// ```
    pub fn items(&self) -> &[(O, Quantity)] {
        self.cards.items()
    }

    /// The distinct cards in ascending order.
    ///
    /// ```
    /// use icepool_core::Deck;
    /// let deck = Deck::from_iter([3, 1, 3, 2]);
    /// assert_eq!(deck.outcomes().copied().collect::<Vec<_>>(), [1, 2, 3]);
    /// ```
    pub fn outcomes(&self) -> impl ExactSizeIterator<Item = &O> {
        self.cards.outcomes()
    }

    /// The number of distinct outcomes (`len(deck)`).
    ///
    /// ```
    /// use icepool_core::Deck;
    /// // Three distinct cards among four.
    /// assert_eq!(Deck::from_iter([1, 2, 2, 3]).len(), 3);
    /// ```
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Whether the deck has no cards at all.
    ///
    /// ```
    /// use icepool_core::Deck;
    /// assert!(Deck::<i64>::from_iter([]).is_empty());
    /// assert!(!Deck::from_iter([1]).is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// The number of cards (`size = Population.denominator`).
    ///
    /// ```
    /// use icepool_core::{Deck, Quantity};
    /// let deck = Deck::from_dict(vec![(1, Quantity::from(4u8)), (2, Quantity::ONE)]).unwrap();
    /// assert_eq!(deck.size(), Quantity::from(5u8));
    /// ```
    pub fn size(&self) -> Quantity {
        self.cards.denominator()
    }

    /// `Deck._pop_min`: a deck with the min outcome removed, and its quantity (`IndexError` for an empty deck).
    ///
    /// ```
    /// use icepool_core::{Deck, Quantity};
    /// // Python's `Deck([1, 2, 2, 3])._pop_min()` is `(Deck({2: 2, 3: 1}), 1)`.
    /// let (rest, quantity) = Deck::from_iter([1, 2, 2, 3]).pop_min().unwrap();
    /// assert_eq!((rest, quantity), (Deck::from_iter([2, 2, 3]), Quantity::ONE));
    /// assert!(Deck::<i64>::from_iter([]).pop_min().is_err());
    /// ```
    pub fn pop_min(&self) -> Result<(Self, Quantity)> {
        let (cards, q) = self.cards.pop_min()?;
        Ok((Deck { cards }, q))
    }

    /// `Deck._pop_max`: a deck with the max outcome removed, and its quantity.
    ///
    /// ```
    /// use icepool_core::{Deck, Quantity};
    /// // Python's `Deck([1, 2, 2, 3])._pop_max()` is `(Deck({1: 1, 2: 2}), 1)`.
    /// let (rest, quantity) = Deck::from_iter([1, 2, 2, 3]).pop_max().unwrap();
    /// assert_eq!((rest, quantity), (Deck::from_iter([1, 2, 2]), Quantity::ONE));
    /// ```
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
    ///
    /// ```
    /// use icepool_core::value::{Counter, CounterOp};
    /// use icepool_core::Deck;
    /// // Python's `Deck([1, 2, 2, 3]) - [2, 3, 4]` is `Deck([1, 2])`; `&` keeps the cards both have.
    /// let deck = Deck::from_iter([1, 2, 2, 3]);
    /// let other = Counter::from_elements([2, 3, 4]);
    /// assert_eq!(deck.counter_op(CounterOp::Sub, &other).unwrap(), Deck::from_iter([1, 2]));
    /// assert_eq!(deck.counter_op(CounterOp::And, &other).unwrap(), Deck::from_iter([2, 3]));
    /// ```
    pub fn counter_op(&self, op: CounterOp, other: &Counter<O>) -> Result<Self> {
        Self::from_dict(self.counter_items(op, other)).err_into()
    }

    /// The items of `+data` in [`Deck::counter_op`].
    ///
    /// ```
    /// use icepool_core::value::{Counter, CounterOp};
    /// use icepool_core::{Deck, Quantity};
    /// // `+data` of `Deck([1, 2]) ^ Counter([2, 2, 3])`: how much the counts of each card differ.
    /// let deck = Deck::from_iter([1, 2]);
    /// let items = deck.counter_items(CounterOp::Xor, &Counter::from_elements([2, 2, 3]));
    /// assert_eq!(items, [(1, Quantity::ONE), (2, Quantity::ONE), (3, Quantity::ONE)]);
    /// ```
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
    ///
    /// ```
    /// use icepool_core::Deck;
    /// // Python's `Deck([1, 1, 2]).zero()` is `Deck({0: 3})`.
    /// assert_eq!(Deck::from_iter([1, 1, 2]).zero().unwrap(), Deck::from_iter([0, 0, 0]));
    /// ```
    pub fn zero(&self) -> Result<Self> {
        let zeros = self.cards.unary_operator(O::try_zero)?;
        let result = Self::from_dict(zeros.items().to_vec()).err_into()?;
        if result.len() != 1 {
            return crate::error::invalid_argument("zero() did not resolve to a single outcome.");
        }
        Ok(result)
    }
}

/// The typed API (docs/design/library-architecture.md section 4): an [`Outcome`]'s comparisons cannot fail, so
/// neither can these.
impl<O: Outcome> Deck<O> {
    /// A deck from `(card, quantity)` pairs (Python's `Deck({card: quantity, ...})`): the quantity is how many
    /// copies of the card the deck holds. Equal cards add up; cards with zero quantity are dropped.
    ///
    /// ```
    /// use icepool_core::Deck;
    /// // Four suits of 13 ranks.
    /// let deck = Deck::from_weights((1..=13).map(|rank| (rank, 4u32)));
    /// assert_eq!((deck.len(), deck.size().to_string()), (13, "52".to_string()));
    /// assert_eq!(Deck::from_weights([(1, 2u32), (2, 0), (1, 1)]), Deck::from_iter([1, 1, 1]));
    /// ```
    pub fn from_weights<Q: Into<Quantity>>(items: impl IntoIterator<Item = (O, Q)>) -> Self {
        Deck {
            cards: Die::from_weights(items),
        }
    }

    /// Deals `hand_size` cards from this deck (Python's `deck.deal(hand_size)`): a [`Deal`], which keeps and
    /// evaluates as a pool does, drawing without replacement.
    ///
    /// # Panics
    ///
    /// If `hand_size` exceeds the size of the deck (Python raises `ValueError: The number of cards dealt cannot
    /// exceed the size of the deck.`). [`Deal::new`] is the fallible form.
    ///
    /// ```
    /// use icepool_core::evaluator::poker::LargestCountEvaluator;
    /// use icepool_core::{Deck, Int};
    /// // Two cards from a deck of two 1s and two 2s: a pair 2 times in 6.
    /// let deal = Deck::from_iter([1, 1, 2, 2]).deal(2);
    /// assert_eq!(deal.denominator().to_string(), "6");
    /// let pairs = deal.evaluate(&LargestCountEvaluator).unwrap();
    /// assert_eq!(pairs.quantity_ge(&Int::from(2)).to_string(), "2");
    /// ```
    pub fn deal(&self, hand_size: usize) -> Deal<O> {
        Deal::new(self.clone(), &IBig::from(hand_size)).unwrap_or_else(|e| panic!("{e}"))
    }
}

/// One copy of each card: equal cards add up (Python's `Deck([...])` of plain outcomes).
///
/// ```
/// use icepool_core::Deck;
/// let deck = Deck::from_iter(["ace", "king", "ace"]);
/// assert_eq!(deck.repr().unwrap(), "Deck({'ace': 2, 'king': 1})");
/// ```
impl<O: Outcome> FromIterator<O> for Deck<O> {
    fn from_iter<I: IntoIterator<Item = O>>(cards: I) -> Self {
        Self::from_weights(cards.into_iter().map(|card| (card, Quantity::ONE)))
    }
}

/// The binary operators of `Deck` (each also has a method form, e.g. `additive_union`, that reduces over `+`).
///
/// ```
/// use icepool_core::value::{Counter, CounterOp};
/// use icepool_core::{Deck, Quantity};
///
/// // `Deck([1, 2, 3]) - Counter([3, 4])` is `Deck([1, 2])`; `|` keeps the larger count of each card.
/// let deck = Deck::from_dict(vec![(1, Quantity::ONE), (2, Quantity::ONE), (3, Quantity::ONE)]).unwrap();
/// let other = Counter::from_elements([3, 4]);
/// let outcomes = |op| deck.counter_items(op, &other).into_iter().map(|(o, _)| o).collect::<Vec<i64>>();
/// assert_eq!(outcomes(CounterOp::Sub), [1, 2]);
/// assert_eq!(outcomes(CounterOp::Or), [1, 2, 3, 4]);
/// ```
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
///
/// ```
/// use dashu_int::IBig;
/// use icepool_core::value::Counter;
/// use icepool_core::{Quantity, Value};
///
/// // `Counter([True, 1, 2])` is `Counter({True: 2, 2: 1})`: `1` counts as the `True` already there.
/// let mut counter = Counter::from_elements([Value::Bool(true), Value::from(1), Value::from(2)]);
/// assert_eq!(counter.items()[0].0.to_string(), "True");
/// assert_eq!(counter.get(&Value::from(1)), IBig::from(2));
/// assert_eq!(counter.get(&Value::from(5)), IBig::ZERO);
/// // `+counter` drops the counts that are not positive.
/// counter.set(Value::from(2), IBig::from(-1));
/// assert_eq!(counter.positive(), [(Value::Bool(true), Quantity::from(2u8))]);
/// ```
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
    ///
    /// ```
    /// use dashu_int::IBig;
    /// use icepool_core::value::Counter;
    /// // Python's `Counter([2, 1, 2])` is `Counter({2: 2, 1: 1})`: in order of first appearance.
    /// let counter = Counter::from_elements([2, 1, 2]);
    /// assert_eq!(counter.items(), [(2, IBig::from(2)), (1, IBig::ONE)]);
    /// ```
    pub fn from_elements(elements: impl IntoIterator<Item = O>) -> Self {
        let mut counter = Counter::default();
        for e in elements {
            let n = counter.get(&e) + IBig::ONE;
            counter.set(e, n);
        }
        counter
    }

    /// `Counter(mapping)`: `dict.update` with the mapping's items (a later equal key overwrites the count).
    ///
    /// ```
    /// use dashu_int::IBig;
    /// use icepool_core::value::Counter;
    /// // A later equal key overwrites the count but keeps its place.
    /// let counter = Counter::from_mapping([(1, IBig::from(5)), (2, IBig::ONE), (1, IBig::from(-1))]);
    /// assert_eq!(counter.items(), [(1, IBig::from(-1)), (2, IBig::ONE)]);
    /// ```
    pub fn from_mapping(items: impl IntoIterator<Item = (O, IBig)>) -> Self {
        let mut counter = Counter::default();
        for (o, c) in items {
            counter.set(o, c);
        }
        counter
    }

    /// `Counter(population)`: its outcomes and quantities.
    ///
    /// ```
    /// use dashu_int::IBig;
    /// use icepool_core::value::Counter;
    /// use icepool_core::Deck;
    /// let counter = Counter::from_population(Deck::from_iter([1, 1, 2]).items());
    /// assert_eq!(counter.items(), [(1, IBig::from(2)), (2, IBig::ONE)]);
    /// ```
    pub fn from_population(items: &[(O, Quantity)]) -> Self {
        Self::from_mapping(items.iter().map(|(o, q)| (o.clone(), q.clone().into())))
    }

    /// Every `(outcome, count)` in insertion order.
    ///
    /// ```
    /// use dashu_int::IBig;
    /// use icepool_core::value::Counter;
    /// assert_eq!(Counter::from_elements(["b", "a"]).items(), [("b", IBig::ONE), ("a", IBig::ONE)]);
    /// ```
    pub fn items(&self) -> &[(O, IBig)] {
        &self.items
    }

    /// `counter[key]`: 0 if missing.
    ///
    /// ```
    /// use dashu_int::IBig;
    /// use icepool_core::value::Counter;
    /// let counter = Counter::from_elements([1, 1]);
    /// assert_eq!((counter.get(&1), counter.get(&2)), (IBig::from(2), IBig::ZERO));
    /// // Asking for a missing key does not insert it.
    /// assert_eq!(counter.items().len(), 1);
    /// ```
    pub fn get(&self, key: &O) -> IBig {
        self.index
            .get(key)
            .map_or(IBig::ZERO, |&i| self.items[i].1.clone())
    }

    /// `counter[key] = count`.
    ///
    /// ```
    /// use dashu_int::IBig;
    /// use icepool_core::value::Counter;
    /// let mut counter = Counter::from_elements([1, 2]);
    /// counter.set(1, IBig::from(7));
    /// counter.set(3, IBig::ZERO);
    /// assert_eq!(counter.items(), [(1, IBig::from(7)), (2, IBig::ONE), (3, IBig::ZERO)]);
    /// ```
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
    ///
    /// ```
    /// use dashu_int::IBig;
    /// use icepool_core::value::Counter;
    /// use icepool_core::Quantity;
    /// let counter = Counter::from_mapping([(1, IBig::from(-1)), (2, IBig::ZERO), (3, IBig::from(2))]);
    /// assert_eq!(counter.positive(), [(3, Quantity::from(2u8))]);
    /// ```
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
