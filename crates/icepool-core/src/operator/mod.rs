//! Port of icepool `operator/*.py`: the internal nodes of a multiset expression. Each takes the `int` counts of
//! its children for one outcome and produces a single `int` count (`MultisetOperator`).
//!
//! In Python every operator becomes a `BodyDungeonlet` (its `_next_state`) and a `BodyQuestlet` (its
//! `_initial_state`) when an expression is prepared for evaluation; here an [`Operator`] is both, and its local
//! state (the "statelet") is a [`Statelet`]. How the nodes of an expression are wired together and driven is in
//! `crate::expression`.
//!
//! Counts are `int`s in Python and `i64` here. Python's counts could become other numbers through a user
//! `map_counts` function or a non-`int` constant; the callers refuse those (`Unsupported`) before they get here.
//! Arithmetic past `i64` is `Unsupported` too.

pub mod adjust_counts;
pub mod binary;
pub mod debug;
pub mod filter_outcomes;
pub mod keep;
pub mod pair;
pub mod versus;

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::order::Order;
use crate::pool::Count;

pub use adjust_counts::{CountFunction, KeepCountsComparison};
pub use binary::BinaryOperator;
pub use filter_outcomes::{OutcomeFilter, OutcomePredicate};
pub use keep::MultisetKeepIndex;

/// One operator node of a multiset expression, with its parameters (the `_expression_key` of each class).
#[derive(Clone)]
pub enum Operator<O> {
    /// `MultisetIntersection`, `MultisetDifferenceDropNegative`, ... (operator/binary.py): the counts of all
    /// children `reduce`d by one merge function.
    Binary(BinaryOperator),
    /// `MultisetMapCounts`: `function(outcome, *counts)`.
    MapCounts(Arc<dyn CountFunction<O> + Send + Sync>),
    /// `MultisetMultiplyCounts`: `count * constant`.
    MultiplyCounts(i64),
    /// `MultisetFloordivCounts`: `count // constant`.
    FloordivCounts(i64),
    /// `MultisetModuloCounts`: `count % constant`.
    ModuloCounts(i64),
    /// `MultisetUnique`: `min(count, constant)`.
    Unique(i64),
    /// `MultisetKeepCounts`: the count if it fits the comparison with the constant, else 0.
    KeepCounts(KeepCountsComparison, i64),
    /// `MultisetFilterOutcomes`: the count if the outcome passes the filter (`!= invert`), else 0.
    FilterOutcomes {
        filter: OutcomeFilter<O>,
        invert: bool,
    },
    /// `MultisetFilterOutcomesBinary`: the first child's count if the second child's is positive (`!= invert`).
    FilterOutcomesBinary { invert: bool },
    /// `MultisetKeep`: keep some of the lowest or highest elements.
    Keep(MultisetKeepIndex),
    /// `MultisetSortPair`. `sort_order` is already `Descending` if `Any` was given (as the constructor does).
    SortPair {
        comparison: Arc<str>,
        sort_order: i64,
        extra: Arc<str>,
    },
    /// `MultisetSortPairWhile`.
    SortPairWhile {
        keep: bool,
        comparison: Arc<str>,
        sort_order: i64,
        extra: Arc<str>,
    },
    /// `MultisetMaxPairLate`.
    MaxPairLate {
        order: i64,
        pair_equal: bool,
        keep: bool,
    },
    /// `MultisetMaxPairEarly`.
    MaxPairEarly {
        order: i64,
        pair_equal: bool,
        keep: bool,
    },
    /// `MultisetVersus`.
    Versus {
        lexi_tuple: (i64, i64, i64),
        order: i64,
    },
    /// `MultisetForceOrder` (operator/debug.py).
    ForceOrder(i64),
}

/// The local state of an operator node (`None` for the stateless ones), part of the evaluation's room key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Statelet {
    None,
    /// `(keep_tuple, keep_more)`: a keep tuple that pops from the left regardless of order, and whether items are
    /// kept after running out of it.
    Keep {
        keep_tuple: Arc<[i64]>,
        keep_more: bool,
    },
    /// `(lexi_tuple, countdown)`.
    Versus {
        lexi_tuple: (i64, i64, i64),
        countdown: i64,
    },
    /// `(lexi_tuple, forward_order, left_lead, countdown)`.
    SortPair {
        lexi_tuple: [i64; 4],
        forward_order: bool,
        left_lead: i64,
        countdown: Option<i64>,
    },
    /// `(lexi_tuple, left_lead, countdown)`; `lexi_tuple` and `left_lead` become `None` once the "while" ends.
    SortPairWhile {
        lexi_tuple: Option<[i64; 4]>,
        left_lead: Option<i64>,
        countdown: Option<i64>,
    },
    /// `prev_pairable`.
    MaxPairLate(i64),
    /// `(right_unpaired, right_remaining)`.
    MaxPairEarly(i64, i64),
}

/// A count the operators can compute with: Python's operators would get a tuple only from a multiset tuple
/// expression, which has no operators (a hand of a `MultiDeal` is another task).
pub(crate) fn int(c: &Count) -> Result<i64> {
    match c {
        Count::Int(c) => Ok(*c),
        Count::Tuple(_) => Err(Error::Unsupported(
            "a multiset operator over tuple counts".into(),
        )),
    }
}

pub(crate) fn overflow() -> Error {
    Error::Unsupported("multiset counts beyond 64 bits".into())
}

pub(crate) fn add(a: i64, b: i64) -> Result<i64> {
    a.checked_add(b).ok_or_else(overflow)
}

pub(crate) fn sub(a: i64, b: i64) -> Result<i64> {
    a.checked_sub(b).ok_or_else(overflow)
}

pub(crate) fn mul(a: i64, b: i64) -> Result<i64> {
    a.checked_mul(b).ok_or_else(overflow)
}

impl<O: PartialEq> Operator<O> {
    /// How many children `_next_state` reads.
    fn arity(&self) -> usize {
        match self {
            Operator::FilterOutcomesBinary { .. }
            | Operator::SortPair { .. }
            | Operator::SortPairWhile { .. }
            | Operator::MaxPairLate { .. }
            | Operator::MaxPairEarly { .. }
            | Operator::Versus { .. } => 2,
            _ => 1,
        }
    }

    /// `_initial_state(order, outcomes, child_sizes, source_sizes, arg_sizes)`: the initial statelet and the size
    /// of this node (`None` if not inferrable). `UnsupportedOrder` if the order is not supported.
    pub fn initial_state(
        &self,
        order: Order,
        child_sizes: &[Option<Count>],
    ) -> Result<(Statelet, Option<Count>)> {
        let order = order.value();
        if !matches!(self, Operator::Binary(_) | Operator::MapCounts(_))
            && child_sizes.len() < self.arity()
        {
            return Err(Error::Index("list index out of range".into()));
        }
        let sizes = || -> Result<Vec<Option<i64>>> {
            child_sizes
                .iter()
                .map(|s| s.as_ref().map(int).transpose())
                .collect()
        };
        match self {
            Operator::Binary(op) => op.initial_state(&sizes()?),
            Operator::MultiplyCounts(c) => adjust_counts::multiply_initial_state(sizes()?[0], *c),
            Operator::Keep(index) => keep::initial_state(index, order, sizes()?[0]),
            Operator::SortPair {
                comparison,
                sort_order,
                extra,
            } => pair::sort_pair_initial_state(comparison, *sort_order, extra, order, &sizes()?),
            Operator::SortPairWhile {
                comparison,
                sort_order,
                extra,
                ..
            } => pair::sort_pair_while_initial_state(
                comparison,
                *sort_order,
                extra,
                order,
                &sizes()?,
            ),
            Operator::MaxPairLate { order: o, .. } => pair::max_pair_late_initial_state(*o, order),
            Operator::MaxPairEarly { order: o, .. } => {
                pair::max_pair_early_initial_state(*o, order, &sizes()?)
            }
            Operator::Versus {
                lexi_tuple,
                order: o,
            } => versus::initial_state(*lexi_tuple, *o, order, &sizes()?),
            Operator::ForceOrder(force_order) => {
                debug::initial_state(*force_order, order, child_sizes[0].clone())
            }
            // `MultisetOperator._initial_state`'s default.
            Operator::MapCounts(_)
            | Operator::FloordivCounts(_)
            | Operator::ModuloCounts(_)
            | Operator::Unique(_)
            | Operator::KeepCounts(..)
            | Operator::FilterOutcomes { .. }
            | Operator::FilterOutcomesBinary { .. } => Ok((Statelet::None, None)),
        }
    }

    /// `_next_state(state, order, outcome, child_counts, source_counts, arg_counts)`: the next statelet and the
    /// count produced by this node.
    pub fn next_state(
        &self,
        state: &Statelet,
        _order: Order,
        outcome: &O,
        child_counts: &[Count],
    ) -> Result<(Statelet, Count)> {
        let counts = child_counts.iter().map(int).collect::<Result<Vec<i64>>>()?;
        // Every operator but the binary ones has a fixed number of children (an expression with none fails to
        // prepare in Python, IndexError).
        if !matches!(self, Operator::Binary(_) | Operator::MapCounts(_))
            && counts.len() < self.arity()
        {
            return Err(Error::Index("list index out of range".into()));
        }
        let stateless = |count: Result<i64>| Ok((Statelet::None, Count::Int(count?)));
        match self {
            Operator::Binary(op) => stateless(op.next_state(&counts)),
            Operator::MapCounts(f) => stateless(f.call(outcome, &counts)),
            Operator::MultiplyCounts(c) => stateless(mul(counts[0], *c)),
            Operator::FloordivCounts(c) => stateless(adjust_counts::floordiv(counts[0], *c)),
            Operator::ModuloCounts(c) => stateless(adjust_counts::modulo(counts[0], *c)),
            Operator::Unique(c) => stateless(Ok(counts[0].min(*c))),
            Operator::KeepCounts(comparison, c) => {
                stateless(Ok(if comparison.test(counts[0], *c) {
                    counts[0]
                } else {
                    0
                }))
            }
            Operator::FilterOutcomes { filter, invert } => stateless(filter_outcomes::next_state(
                filter, *invert, outcome, counts[0],
            )),
            Operator::FilterOutcomesBinary { invert } => stateless(Ok(
                filter_outcomes::binary_next_state(*invert, counts[0], counts[1]),
            )),
            Operator::Keep(_) => keep::next_state(state, counts[0]),
            Operator::SortPair { .. } => pair::sort_pair_next_state(state, counts[0], counts[1]),
            Operator::SortPairWhile { keep, .. } => {
                pair::sort_pair_while_next_state(*keep, state, counts[0], counts[1])
            }
            Operator::MaxPairLate {
                pair_equal, keep, ..
            } => pair::max_pair_late_next_state(*pair_equal, *keep, state, counts[0], counts[1]),
            Operator::MaxPairEarly {
                pair_equal, keep, ..
            } => pair::max_pair_early_next_state(*pair_equal, *keep, state, counts[0], counts[1]),
            Operator::Versus { .. } => versus::next_state(state, counts[0], counts[1]),
            Operator::ForceOrder(_) => Ok((Statelet::None, Count::Int(counts[0]))),
        }
    }
}
