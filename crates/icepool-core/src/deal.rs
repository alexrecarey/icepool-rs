//! Port of icepool `generator/deal.py`: an unordered deal of a single hand from a `Deck`, and the `DealSource` the
//! evaluator pops from.
//!
//! Popping an outcome from a deal is hypergeometric: if the deck holds `deck_count` cards of the popped outcome,
//! the hand gets `count` of them in `comb(deck_count, count)` ways, and the rest of the hand comes from the rest of
//! the deck.

use crate::ownership::{Owners, Traverse};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashu_int::IBig;

use crate::deck::Deck;
use crate::error::{Error, Result};
use crate::keep_tuple::{
    self, pop_max_from_keep_tuple, pop_min_from_keep_tuple, sum_counts, KeepIndex,
};
use crate::math::comb;
use crate::order::{lo_hi_skip, Order, OrderReason};
use crate::outcome::OutcomeOrder;
use crate::pool::{Popped, MAX_POOL_SIZE};
use crate::{Int, Quantity};

/// `Deal` (a `KeepGenerator`): a hand of `len(keep_tuple)` cards dealt from `deck`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Deal<O> {
    deck: Deck<O>,
    keep_tuple: Arc<[Int]>,
}

impl<O: OutcomeOrder + Traverse> Traverse for Deal<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        self.deck.traverse(owners);
    }
}

impl<O: OutcomeOrder> Deal<O> {
    /// `Deal(deck, hand_size)`.
    ///
    /// For algorithmic reasons, you must pre-commit to the number of cards to deal.
    ///
    /// It is permissible to deal zero cards from an empty deck, but not all evaluators will handle this case,
    /// especially if they depend on the outcome type. Dealing zero cards from a non-empty deck does not have this
    /// issue.
    pub fn new(deck: Deck<O>, hand_size: &IBig) -> Result<Self> {
        if *hand_size < IBig::ZERO {
            return Err(Error::Value("hand_size cannot be negative.".into()));
        }
        if *hand_size > IBig::from(deck.size()) {
            return Err(Error::Value(
                "The number of cards dealt cannot exceed the size of the deck.".into(),
            ));
        }
        // `(1, ) * hand_size`: Python would spend minutes or run out of memory beyond this.
        let n = usize::try_from(hand_size)
            .ok()
            .filter(|&n| n <= MAX_POOL_SIZE)
            .ok_or_else(|| Error::Overflow("too many cards dealt".into()))?;
        Ok(Deal {
            deck,
            keep_tuple: vec![Int::ONE; n].into(),
        })
    }

    /// A deal's keep tuple (`keep_tuple()`) over its deck again: for a host that holds the deck apart (the Python
    /// facade holds the deck's Python object, so that only one Python object holds the cards: see `ownership`).
    pub fn from_parts(deck: Deck<O>, keep_tuple: Arc<[Int]>) -> Self {
        Deal { deck, keep_tuple }
    }

    /// The `Deck` the cards are dealt from.
    pub fn deck(&self) -> &Deck<O> {
        &self.deck
    }

    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    /// The number of cards dealt.
    pub fn hand_size(&self) -> usize {
        self.keep_tuple.len()
    }

    /// The total count produced by this generator.
    pub fn keep_size(&self) -> Int {
        sum_counts(&self.keep_tuple)
    }

    pub fn has_negative_keeps(&self) -> bool {
        self.keep_tuple.iter().any(Int::is_negative)
    }

    /// `comb(deck.size(), hand_size)`.
    pub fn denominator(&self) -> Quantity {
        comb(&self.deck.size(), self.hand_size())
    }

    /// `_set_keep_tuple`: a copy with a modified keep tuple.
    pub fn with_keep_tuple(&self, keep_tuple: Vec<Int>) -> Self {
        Deal {
            deck: self.deck.clone(),
            keep_tuple: keep_tuple.into(),
        }
    }

    /// `KeepGenerator.keep` (see `Pool::keep`).
    pub fn keep(&self, index: &KeepIndex) -> Result<Self> {
        Ok(self.with_keep_tuple(keep_tuple::keep(&self.keep_tuple, index)?))
    }

    /// `KeepGenerator.multiply_counts`.
    pub fn multiply_counts(&self, constant: &Int) -> Self {
        self.with_keep_tuple(keep_tuple::multiply_counts(&self.keep_tuple, constant))
    }

    /// `_make_source`.
    pub fn source(&self) -> DealSource<O> {
        let outcomes: Arc<[O]> = self.deck.outcomes().cloned().collect();
        DealSource {
            deck: self.deck.clone(),
            lo: 0,
            hi: outcomes.len(),
            outcomes,
            keep_tuple: self.keep_tuple.clone(),
        }
    }
}

/// `DealSource`: the cards not popped yet and the counts of the hand's sorted positions not popped yet.
///
/// Equality and hashing follow `hash_key` `(DealSource, deck, keep_tuple)`. The deck only ever loses its min or
/// max outcome, so its outcomes are a window `lo..hi` of the first deck's, shared by every source popped from it.
#[derive(Clone, Debug)]
pub struct DealSource<O> {
    deck: Deck<O>,
    outcomes: Arc<[O]>,
    lo: usize,
    hi: usize,
    keep_tuple: Arc<[Int]>,
}

impl<O: OutcomeOrder + Traverse> Traverse for DealSource<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        self.deck.traverse(owners);
        owners.arc(&self.outcomes);
    }
}

impl<O: PartialEq> PartialEq for DealSource<O> {
    fn eq(&self, other: &Self) -> bool {
        self.deck == other.deck && self.keep_tuple == other.keep_tuple
    }
}

impl<O: Eq> Eq for DealSource<O> {}

impl<O: Hash> Hash for DealSource<O> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deck.hash(state);
        self.keep_tuple.hash(state);
    }
}

impl<O: OutcomeOrder> DealSource<O> {
    pub fn deck(&self) -> &Deck<O> {
        &self.deck
    }

    pub fn keep_tuple(&self) -> &[Int] {
        &self.keep_tuple
    }

    /// The outcomes of the remaining deck.
    pub fn outcomes(&self) -> &[O] {
        &self.outcomes[self.lo..self.hi]
    }

    /// `KeepSource.size`: the total count, unknown (`None`) with negative counts.
    pub fn size(&self) -> Option<Int> {
        if self.keep_tuple.iter().any(Int::is_negative) {
            return None;
        }
        Some(sum_counts(&self.keep_tuple))
    }

    pub fn is_resolvable(&self) -> bool {
        !self.deck.is_empty()
    }

    /// `comb(deck.size(), len(keep_tuple))`.
    pub fn denominator(&self) -> Quantity {
        comb(&self.deck.size(), self.keep_tuple.len())
    }

    /// A deal prefers to pop from the end whose positions it skips (like a pool's `KeepSkip`).
    pub fn order_preference(&self) -> (Order, OrderReason) {
        let (lo_skip, hi_skip) = lo_hi_skip(&self.keep_tuple);
        if lo_skip > hi_skip {
            return (Order::Descending, OrderReason::KeepSkip);
        }
        if hi_skip > lo_skip {
            return (Order::Ascending, OrderReason::KeepSkip);
        }
        (Order::Any, OrderReason::NoPreference)
    }

    /// Pops the lowest outcome (`order > 0`) or the highest (otherwise), if it is `outcome`: for each number of its
    /// cards the hand can get, `(rest of the deal, count of outcome, weight)`.
    ///
    /// Possibilities where no count remains in the keep tuple are not yielded one by one: the rest of the hand is
    /// dumped in exchange for the denominator, and they come last as one deal with an empty keep tuple.
    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<Popped<Self>>> {
        let unchanged = || Ok(vec![(self.clone(), Int::ZERO, Quantity::ONE)]);
        if self.deck.is_empty() {
            return unchanged();
        }
        let ascending = order.is_ascending();
        let pop_outcome = if ascending {
            self.deck.population().min_outcome()?
        } else {
            self.deck.population().max_outcome()?
        };
        if outcome != pop_outcome {
            return unchanged();
        }
        let ((popped_deck, deck_count), lo, hi) = if ascending {
            (self.deck.pop_min()?, self.lo + 1, self.hi)
        } else {
            (self.deck.pop_max()?, self.lo, self.hi - 1)
        };
        let next_source = |deck, keep_tuple: Vec<Int>| DealSource {
            deck,
            outcomes: self.outcomes.clone(),
            lo,
            hi,
            keep_tuple: keep_tuple.into(),
        };

        // `max(0, deck_count + len(keep_tuple) - deck.size())`: the hand must take the cards the rest of the deck
        // cannot supply. At most `len(keep_tuple)`, so it fits.
        let hand = self.keep_tuple.len();
        let size = self.deck.size();
        let with_hand = &deck_count + hand;
        let min_count = if with_hand > size {
            usize::try_from(with_hand - size).expect("at most the hand size")
        } else {
            0
        };
        let max_count = usize::try_from(&deck_count).map_or(hand, |c| c.min(hand));

        let mut result = Vec::new();
        let mut skip_weight: Option<Quantity> = None;
        for count in min_count..=max_count {
            let (popped_keep_tuple, result_count) = if ascending {
                pop_min_from_keep_tuple(&self.keep_tuple, count)
            } else {
                pop_max_from_keep_tuple(&self.keep_tuple, count)
            };
            let popped_deal = next_source(popped_deck.clone(), popped_keep_tuple);
            let weight = comb(&deck_count, count);
            if popped_deal.keep_tuple.iter().all(Int::is_zero) {
                // Dump all dice in exchange for the denominator.
                *skip_weight.get_or_insert(Quantity::ZERO) += weight * popped_deal.denominator();
                continue;
            }
            result.push((popped_deal, result_count, weight));
        }
        if let Some(skip_weight) = skip_weight {
            let popped_deal = next_source(popped_deck, Vec::new());
            result.push((popped_deal, sum_counts(&self.keep_tuple), skip_weight));
        }
        Ok(result)
    }
}
