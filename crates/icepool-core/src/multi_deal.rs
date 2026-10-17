//! Port of icepool `generator/multi_deal.py`: a deal of several hands from one `Deck`, and the `MultiDealSource`
//! the evaluator pops from. A `MultiDeal` is a `MultisetTupleGenerator`: the count it produces for each outcome is a
//! tuple with one count per hand ([`Count::Tuple`]).
//!
//! The cards within each hand are in sorted order. Furthermore, hands may be organized into groups in which the
//! hands are initially indistinguishable: popping an outcome deals its cards to the hands of a group in every way
//! up to reordering the group's hands (counts sorted descending within the group), and the group splits by the
//! counts it got.

use crate::ownership::{Owners, Traverse};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashu_int::IBig;

use crate::deck::Deck;
use crate::error::{Error, Result};
use crate::math::{comb, iter_hypergeom};
use crate::order::{Order, OrderReason};
use crate::outcome::OutcomeOrder;
use crate::pool::MAX_POOL_SIZE;
use crate::{Int, Quantity};

/// A hand group: `(hand_size, group_size)`, `group_size` hands of `hand_size` cards each.
pub type HandGroup = (usize, usize);

/// `hand_sizes()`: the number of cards dealt to each hand as a flat tuple.
fn hand_sizes(hand_groups: &[HandGroup]) -> Vec<usize> {
    hand_groups
        .iter()
        .flat_map(|&(hand_size, group_size)| std::iter::repeat_n(hand_size, group_size))
        .collect()
}

/// `total_cards_dealt()`.
fn total_cards_dealt(hand_groups: &[HandGroup]) -> usize {
    hand_groups.iter().map(|&(h, g)| h * g).sum()
}

/// `hand_count()` (`len(multi_deal)`): the number of hands.
fn hand_count(hand_groups: &[HandGroup]) -> usize {
    hand_groups.iter().map(|&(_, g)| g).sum()
}

/// `_denominator`: `comb(deck.size(), total) * prod(comb(total, h) for h in hand_sizes()[1:])`, as icepool computes
/// it.
fn denominator<O: OutcomeOrder>(deck: &Deck<O>, hand_groups: &[HandGroup]) -> Quantity {
    let total = total_cards_dealt(hand_groups);
    let d_total = comb(&deck.size(), total);
    let total = Quantity::from(total);
    hand_sizes(hand_groups)
        .iter()
        .skip(1)
        .fold(d_total, |acc, &h| acc * comb(&total, h))
}

/// `MultiDeal`: hands dealt from `deck`, described by an ordered tuple of hand groups.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MultiDeal<O> {
    deck: Deck<O>,
    hand_groups: Arc<[HandGroup]>,
}

impl<O: OutcomeOrder + Traverse> Traverse for MultiDeal<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        self.deck.traverse(owners);
    }
}

impl<O: OutcomeOrder> MultiDeal<O> {
    /// `MultiDeal(deck, hand_groups)`.
    ///
    /// For algorithmic reasons, you must pre-commit to the number of cards to deal for each hand.
    ///
    /// It is permissible to deal zero cards from an empty deck, but not all evaluators will handle this case,
    /// especially if they depend on the outcome type. Dealing zero cards from a non-empty deck does not have this
    /// issue.
    ///
    /// `hand_groups`: an ordered tuple of hand groups. Each group is designated by `(hand_size, hand_count)` with
    /// the hands of each group being arbitrarily ordered. The resulting counts are produced in a flat tuple.
    ///
    /// Python only checks the total; a negative size passes it, after which icepool's `comb` reads its cached
    /// rows with negative indices (the answer depends on the process's history): the port refuses it.
    pub fn new(deck: Deck<O>, hand_groups: &[(IBig, IBig)]) -> Result<Self> {
        let total: IBig = hand_groups.iter().map(|(h, g)| h * g).sum();
        if total > IBig::from(deck.size()) {
            return Err(Error::InvalidArgument(
                "The total number of cards dealt cannot exceed the size of the deck.".into(),
            ));
        }
        let groups = hand_groups
            .iter()
            .map(|(h, g)| Some((usize::try_from(h).ok()?, usize::try_from(g).ok()?)))
            .collect::<Option<Vec<HandGroup>>>()
            .ok_or_else(|| Error::Unported("a negative or huge MultiDeal hand size".into()))?;
        // `hand_sizes()` is a tuple with one element per hand: Python would run out of memory beyond this.
        if groups
            .iter()
            .try_fold(0usize, |n, &(_, g)| n.checked_add(g))
            .is_none_or(|n| n > MAX_POOL_SIZE)
        {
            return Err(Error::Unported("too many hands in a MultiDeal".into()));
        }
        Ok(MultiDeal {
            deck,
            hand_groups: groups.into(),
        })
    }

    /// A multi-deal's hand groups (`hand_groups()`) over its deck again: for a host that holds the deck apart (the
    /// Python facade holds the deck's Python object, so that only one Python object holds the cards: see
    /// `ownership`).
    pub fn from_parts(deck: Deck<O>, hand_groups: Arc<[HandGroup]>) -> Self {
        MultiDeal { deck, hand_groups }
    }

    /// The `Deck` the cards are dealt from.
    pub fn deck(&self) -> &Deck<O> {
        &self.deck
    }

    /// The `(hand size, number of hands)` groups, in order.
    pub fn hand_groups(&self) -> &[HandGroup] {
        &self.hand_groups
    }

    /// The number of cards dealt to each hand as a tuple.
    pub fn hand_sizes(&self) -> Vec<usize> {
        hand_sizes(&self.hand_groups)
    }

    /// The total number of cards dealt.
    pub fn total_cards_dealt(&self) -> usize {
        total_cards_dealt(&self.hand_groups)
    }

    /// `len(multi_deal)`: the number of hands.
    pub fn len(&self) -> usize {
        hand_count(&self.hand_groups)
    }

    /// Whether the deck has no cards.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The total number of ways the hands can be dealt.
    pub fn denominator(&self) -> Quantity {
        denominator(&self.deck, &self.hand_groups)
    }

    /// `_make_source`.
    pub fn source(&self) -> MultiDealSource<O> {
        let outcomes: Arc<[O]> = self.deck.outcomes().cloned().collect();
        MultiDealSource {
            deck: self.deck.clone(),
            lo: 0,
            hi: outcomes.len(),
            outcomes,
            hand_groups: self.hand_groups.clone(),
        }
    }
}

/// One possibility after popping an outcome from a `MultiDealSource`: the rest of the source, the count of the
/// popped outcome in each hand, its weight.
pub type TuplePopped<S> = (S, Arc<[Int]>, Quantity);

/// `MultiDealSource`: the cards not popped yet and the hand groups still to fill.
///
/// Equality and hashing follow `hash_key` `(MultiDealSource, deck, hand_groups)`. As for `DealSource`, the deck
/// only ever loses its min or max outcome, so its outcomes are a window `lo..hi` of the first deck's.
#[derive(Clone, Debug)]
pub struct MultiDealSource<O> {
    deck: Deck<O>,
    outcomes: Arc<[O]>,
    lo: usize,
    hi: usize,
    hand_groups: Arc<[HandGroup]>,
}

impl<O: OutcomeOrder + Traverse> Traverse for MultiDealSource<O> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        self.deck.traverse(owners);
        owners.arc(&self.outcomes);
    }
}

impl<O: PartialEq> PartialEq for MultiDealSource<O> {
    fn eq(&self, other: &Self) -> bool {
        self.deck == other.deck && self.hand_groups == other.hand_groups
    }
}

impl<O: Eq> Eq for MultiDealSource<O> {}

impl<O: Hash> Hash for MultiDealSource<O> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deck.hash(state);
        self.hand_groups.hash(state);
    }
}

impl<O: OutcomeOrder> MultiDealSource<O> {
    /// The deck still to deal from.
    pub fn deck(&self) -> &Deck<O> {
        &self.deck
    }

    /// The `(hand size, number of hands)` groups still to deal.
    pub fn hand_groups(&self) -> &[HandGroup] {
        &self.hand_groups
    }

    /// The outcomes of the remaining deck.
    pub fn outcomes(&self) -> &[O] {
        &self.outcomes[self.lo..self.hi]
    }

    /// `size()`: the hand sizes (a tuple source's size is a tuple).
    pub fn size(&self) -> Arc<[Int]> {
        self.hand_size_counts()
    }

    fn hand_size_counts(&self) -> Arc<[Int]> {
        hand_sizes(&self.hand_groups)
            .into_iter()
            .map(Int::from)
            .collect()
    }

    /// Whether the deal can be evaluated: there are enough cards in the deck for every hand.
    pub fn is_resolvable(&self) -> bool {
        !self.outcomes().is_empty()
    }

    /// The total number of ways the remaining hands can be dealt.
    pub fn denominator(&self) -> Quantity {
        denominator(&self.deck, &self.hand_groups)
    }

    /// A multi-deal has no preferred order.
    pub fn order_preference(&self) -> (Order, OrderReason) {
        (Order::Any, OrderReason::NoPreference)
    }

    /// Pops the lowest outcome (`order > 0`) or the highest (otherwise), if it is `outcome`: for each way of
    /// dealing its cards to the hands, `(rest of the deal, count per hand, weight)`.
    ///
    /// Possibilities where every hand is full are not yielded one by one: they come last as one source whose hands
    /// are all empty, with the hand sizes as counts.
    pub fn pop(&self, order: Order, outcome: &O) -> Result<Vec<TuplePopped<Self>>> {
        let n_hands = hand_count(&self.hand_groups);
        let unchanged = || {
            Ok(vec![(
                self.clone(),
                vec![Int::ZERO; n_hands].into(),
                Quantity::ONE,
            )])
        };
        if self.outcomes().is_empty() {
            return unchanged();
        }
        let ascending = order.value() > 0;
        let pop_outcome = if ascending {
            &self.outcomes()[0]
        } else {
            &self.outcomes()[self.outcomes().len() - 1]
        };
        if pop_outcome != outcome {
            return unchanged();
        }
        let ((popped_deck, deck_count), lo, hi) = if ascending {
            (self.deck.pop_min()?, self.lo + 1, self.hi)
        } else {
            (self.deck.pop_max()?, self.lo, self.hi - 1)
        };
        let next_source = |hand_groups: Vec<HandGroup>| MultiDealSource {
            deck: popped_deck.clone(),
            outcomes: self.outcomes.clone(),
            lo,
            hi,
            hand_groups: hand_groups.into(),
        };

        let total = total_cards_dealt(&self.hand_groups);
        let size = self.deck.size();
        let with_total = &deck_count + total;
        let min_count = if with_total > size {
            usize::try_from(with_total - size).expect("at most the cards dealt")
        } else {
            0
        };
        let max_count = usize::try_from(&deck_count).map_or(total, |c| c.min(total));
        let sizes = hand_sizes(&self.hand_groups);

        let mut result = Vec::new();
        let mut skip_weight: Option<Quantity> = None;
        for count_total in min_count..=max_count {
            let weight_total = comb(&deck_count, count_total);
            // The "deck" assigns the cards of the current outcome to hands.
            for (raw_counts, weight_split) in iter_hypergeom(&sizes, count_total) {
                let mut pos = 0;
                let mut counts = raw_counts.clone();
                let mut next_hand_groups: Vec<HandGroup> = Vec::new();
                for &(hand_size, group_size) in self.hand_groups.iter() {
                    let group = &mut counts[pos..pos + group_size];
                    group.sort_unstable_by(|a, b| b.cmp(a));
                    // `itertools.groupby` over the sorted counts: one run per distinct count.
                    for run in group.chunk_by(|a, b| a == b) {
                        let (count, next_group_size) = (run[0], run.len());
                        if count == hand_size {
                            next_hand_groups.extend(std::iter::repeat_n((0, 1), next_group_size));
                        } else {
                            next_hand_groups.push((hand_size - count, next_group_size));
                        }
                    }
                    pos += group_size;
                }
                let popped_source = next_source(next_hand_groups);
                let weight = &weight_total * weight_split;
                if popped_source.hand_groups.iter().all(|&(h, _)| h == 0) {
                    *skip_weight.get_or_insert(Quantity::ZERO) +=
                        weight * popped_source.denominator();
                    continue;
                }
                let counts = counts.into_iter().map(Int::from).collect();
                result.push((popped_source, counts, weight));
            }
        }
        if let Some(skip_weight) = skip_weight {
            let skip_source = next_source(vec![(0, 1); n_hands]);
            result.push((skip_source, self.hand_size_counts(), skip_weight));
        }
        Ok(result)
    }
}
