//! Port of icepool `operator/*.py`: the internal nodes of a multiset expression. Each takes the `int` counts of
//! its children for one outcome and produces a single `int` count (`MultisetOperator`).
//!
//! In Python every operator becomes a `BodyDungeonlet` (its `_next_state`) and a `BodyQuestlet` (its
//! `_initial_state`) when an expression is prepared for evaluation; here an [`Operator`] is both, and its local
//! state (the "statelet") is a [`Statelet`]. How the nodes of an expression are wired together and driven is in
//! `crate::expression`.
//!
//! Counts are `int`s in Python and unbounded [`Int`]s here. Python's counts could become other numbers through a
//! user `map_counts` function or a non-`int` constant; the callers refuse those (`Unsupported`) before they get
//! here.
//!
//! An operator applied to a multiset *tuple* expression (a `MultiDeal`, a tuple parameter) gets tuple counts and
//! sizes. Python has no special case for them: its arithmetic just runs on tuples, which mostly raises
//! `TypeError` (`max(count, 0)`, `left - right`) but sometimes answers (`min` / `max` of two tuples, `+`
//! concatenates, `tuple * n` repeats, `count == n` is `False`, and a count passed through untouched stays a tuple).
//! The port does the same: [`int`] is the `TypeError` where Python computes with a count, and the operators below
//! copy each case where a tuple gets through.

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
use crate::Int;

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
    MultiplyCounts(Int),
    /// `MultisetFloordivCounts`: `count // constant`.
    FloordivCounts(Int),
    /// `MultisetModuloCounts`: `count % constant`.
    ModuloCounts(Int),
    /// `MultisetUnique`: `min(count, constant)`.
    Unique(Int),
    /// `MultisetKeepCounts`: the count if it fits the comparison with the constant, else 0.
    KeepCounts(KeepCountsComparison, Int),
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
    /// An operator the host implements: in the Python facade, a `MultisetOperator` the core has no variant for
    /// (a user's subclass, or a subclass of a built-in one, which may override anything). Its `_initial_state`
    /// and `_next_state` are called back; its local state is a host value.
    Host(Arc<dyn HostOperator<O> + Send + Sync>),
}

impl<O> Operator<O> {
    /// `str(operator)` given `str()` of its children and, for `MultisetKeep`, of its index as it was given (a
    /// slice, list or tuple, which the core does not keep). `None` for the operators without a `__str__`
    /// (`MultisetMapCounts`, the pairing and versus operators, `MultisetForceOrder`, host operators): theirs is
    /// `object.__repr__`, a memory address.
    pub fn py_str(&self, children: &[String], keep_index: Option<&str>) -> Option<String> {
        let child = || children.first().map(String::as_str).unwrap_or_default();
        Some(match self {
            // `'(' + (' ' + self.symbol() + ' ').join(str(child) for child in self._children) + ')'`
            Operator::Binary(op) => format!("({})", children.join(&format!(" {} ", op.symbol()))),
            Operator::MultiplyCounts(n) => format!("({} * {n})", child()),
            Operator::FloordivCounts(n) => format!("({} // {n})", child()),
            Operator::ModuloCounts(n) => format!("({} % {n})", child()),
            Operator::Unique(n) if *n == 1 => format!("{}.unique()", child()),
            Operator::Unique(n) => format!("{}.unique({n})", child()),
            Operator::KeepCounts(c, n) => format!("{}.keep_counts('{}', {n})", child(), c.symbol()),
            // The unary filter shows neither its function nor its set of outcomes.
            Operator::FilterOutcomes { invert, .. } => {
                let name = if *invert { "drop" } else { "keep" };
                format!("{}.{name}_outcomes(...)", child())
            }
            Operator::FilterOutcomesBinary { invert } => {
                let name = if *invert { "drop" } else { "keep" };
                format!("{}.{name}_outcomes({})", child(), children.get(1)?)
            }
            Operator::Keep(_) => format!("{}[{}]", child(), keep_index?),
            _ => return None,
        })
    }
}

/// The methods of an [`Operator::Host`]: `_initial_state(order, outcomes, child_sizes, source_sizes, arg_sizes)`
/// and `_next_state(state, order, outcome, child_counts, source_counts, arg_counts)`, each returning the local
/// state and this node's size or count. Only an operator's children are its business: the source and argument
/// counts are for free variables and parameters, so the host gets none.
pub trait HostOperator<O> {
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        child_sizes: &[Option<Count>],
    ) -> Result<(crate::Value, Option<Count>)>;

    fn next_state(
        &self,
        state: &crate::Value,
        order: Order,
        outcome: &O,
        child_counts: &[Count],
    ) -> Result<(crate::Value, Count)>;
}

/// The local state of an operator node (`None` for the stateless ones), part of the evaluation's room key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Statelet {
    None,
    /// `(keep_tuple, keep_more)`: a keep tuple that pops from the left regardless of order, and whether items are
    /// kept after running out of it.
    Keep {
        keep_tuple: Arc<[Int]>,
        keep_more: bool,
    },
    /// `(lexi_tuple, countdown)`.
    Versus {
        lexi_tuple: (i64, i64, i64),
        countdown: Int,
    },
    /// `(lexi_tuple, forward_order, left_lead, countdown)`.
    SortPair {
        lexi_tuple: [i64; 4],
        forward_order: bool,
        left_lead: Int,
        countdown: Option<Int>,
    },
    /// `(lexi_tuple, left_lead, countdown)`; `lexi_tuple` and `left_lead` become `None` once the "while" ends.
    SortPairWhile {
        lexi_tuple: Option<[i64; 4]>,
        left_lead: Option<Int>,
        countdown: Option<Int>,
    },
    /// `prev_pairable`.
    MaxPairLate(Int),
    /// `(right_unpaired, right_remaining)`.
    MaxPairEarly(Int, Int),
    /// The state of a pairing operator that stored a tuple size (`countdown = right_size`): every `_next_state`
    /// computes with it first, which is Python's `TypeError`.
    TupleSize(Arc<[Int]>),
    /// The state of an [`Operator::Host`].
    Host(crate::Value),
}

/// A count (or size) an operator computes with: a tuple is Python's `TypeError` from its arithmetic or
/// comparison with an `int`.
pub(crate) fn int(c: &Count) -> Result<Int> {
    match c {
        Count::Int(c) => Ok(c.clone()),
        Count::Tuple(_) => Err(tuple_error()),
    }
}

/// What Python raises computing with a tuple count, e.g. `max((1, 0), 0)` or `(1, 0) - 1`.
pub(crate) fn tuple_error() -> Error {
    Error::Type("unsupported operand type(s) for a tuple count and an int".into())
}

/// Python's `count * n` for a tuple count: the tuple repeated `n` times (empty if `n <= 0`).
pub(crate) fn repeat(t: &[Int], n: &Int) -> Result<Count> {
    // Python builds any length it has memory for; past 2**24 elements the port says it does not cover it, and past
    // 64 bits Python cannot either (OverflowError, even for an empty tuple or a negative `n`).
    let n = n.to_len("a tuple count repeated")?;
    if t.len().saturating_mul(n) > 1 << 24 {
        return Err(Error::Unsupported("a tuple count of this length".into()));
    }
    Ok(Count::Tuple(
        t.iter().cycle().take(t.len() * n).cloned().collect(),
    ))
}

/// `count * n` for any count.
pub(crate) fn times(count: &Count, n: &Int) -> Result<Count> {
    match count {
        Count::Int(c) => Ok(Count::Int(c * n)),
        Count::Tuple(t) => repeat(t, n),
    }
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
        outcomes: &[O],
        child_sizes: &[Option<Count>],
    ) -> Result<(Statelet, Option<Count>)> {
        if let Operator::Host(op) = self {
            let (state, size) = op.initial_state(order, outcomes, child_sizes)?;
            return Ok((Statelet::Host(state), size));
        }
        let order = order.value();
        if !matches!(self, Operator::Binary(_) | Operator::MapCounts(_))
            && child_sizes.len() < self.arity()
        {
            return Err(Error::Index("list index out of range".into()));
        }
        // Each operator reads the sizes it computes with (`int`: a tuple size is a TypeError there); the pairing
        // operators may store one in their state instead.
        let size = |i: usize| child_sizes[i].as_ref().map(int).transpose();
        match self {
            Operator::Binary(op) => op.initial_state(child_sizes),
            Operator::MultiplyCounts(c) => {
                adjust_counts::multiply_initial_state(child_sizes[0].as_ref(), c)
            }
            Operator::Keep(index) => keep::initial_state(index, order, size(0)?),
            Operator::SortPair {
                comparison,
                sort_order,
                extra,
            } => pair::sort_pair_initial_state(comparison, *sort_order, extra, order, child_sizes),
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
                child_sizes,
            ),
            Operator::MaxPairLate { order: o, .. } => pair::max_pair_late_initial_state(*o, order),
            Operator::MaxPairEarly { order: o, .. } => {
                pair::max_pair_early_initial_state(*o, order, child_sizes)
            }
            Operator::Versus {
                lexi_tuple,
                order: o,
            } => versus::initial_state(*lexi_tuple, *o, order, child_sizes),
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
            Operator::Host(_) => unreachable!("handled above"),
        }
    }

    /// `_next_state(state, order, outcome, child_counts, source_counts, arg_counts)`: the next statelet and the
    /// count produced by this node.
    pub fn next_state(
        &self,
        state: &Statelet,
        order: Order,
        outcome: &O,
        child_counts: &[Count],
    ) -> Result<(Statelet, Count)> {
        if let Operator::Host(op) = self {
            // A host operator's state is always a host one: `initial_state` made it.
            let Statelet::Host(state) = state else {
                return Err(Error::Value("a host operator without its state".into()));
            };
            let (next, count) = op.next_state(state, order, outcome, child_counts)?;
            return Ok((Statelet::Host(next), count));
        }
        // Every operator but the binary ones has a fixed number of children (an expression with none fails to
        // prepare in Python, IndexError).
        if !matches!(self, Operator::Binary(_) | Operator::MapCounts(_))
            && child_counts.len() < self.arity()
        {
            return Err(Error::Index("list index out of range".into()));
        }
        if let Statelet::TupleSize(_) = state {
            return Err(tuple_error());
        }
        if child_counts.iter().any(|c| matches!(c, Count::Tuple(_))) {
            return self.next_state_tuple(state, outcome, child_counts);
        }
        let counts = child_counts.iter().map(int).collect::<Result<Vec<Int>>>()?;
        let stateless = |count: Result<Int>| Ok((Statelet::None, Count::Int(count?)));
        match self {
            Operator::Binary(op) => stateless(op.next_state(&counts)),
            Operator::MapCounts(f) => Ok((Statelet::None, f.call(outcome, child_counts)?)),
            Operator::MultiplyCounts(c) => stateless(Ok(&counts[0] * c)),
            Operator::FloordivCounts(c) => stateless(adjust_counts::floordiv(&counts[0], c)),
            Operator::ModuloCounts(c) => stateless(adjust_counts::modulo(&counts[0], c)),
            Operator::Unique(c) => stateless(Ok(counts[0].clone().min(c.clone()))),
            Operator::KeepCounts(comparison, c) => {
                stateless(Ok(if comparison.test(&counts[0], c) {
                    counts[0].clone()
                } else {
                    Int::ZERO
                }))
            }
            Operator::FilterOutcomes { filter, invert } => {
                stateless(Ok(if filter_outcomes::keeps(filter, *invert, outcome)? {
                    counts[0].clone()
                } else {
                    Int::ZERO
                }))
            }
            Operator::FilterOutcomesBinary { invert } => stateless(Ok(
                filter_outcomes::binary_next_state(*invert, &counts[0], &counts[1]),
            )),
            Operator::Keep(_) => keep::next_state(state, &counts[0]),
            Operator::SortPair { .. } => pair::sort_pair_next_state(state, &counts[0], &counts[1]),
            Operator::SortPairWhile { keep, .. } => {
                pair::sort_pair_while_next_state(*keep, state, &counts[0], &counts[1])
            }
            Operator::MaxPairLate {
                pair_equal, keep, ..
            } => pair::max_pair_late_next_state(*pair_equal, *keep, state, &counts[0], &counts[1]),
            Operator::MaxPairEarly {
                pair_equal, keep, ..
            } => pair::max_pair_early_next_state(*pair_equal, *keep, state, &counts[0], &counts[1]),
            Operator::Versus { .. } => versus::next_state(state, &counts[0], &counts[1]),
            Operator::ForceOrder(_) => Ok((Statelet::None, Count::Int(counts[0].clone()))),
            Operator::Host(_) => unreachable!("next_state calls the host first"),
        }
    }

    /// `_next_state` when some child count is a tuple: where Python's code lets the tuple through, and the
    /// `TypeError` everywhere else.
    fn next_state_tuple(
        &self,
        state: &Statelet,
        outcome: &O,
        child_counts: &[Count],
    ) -> Result<(Statelet, Count)> {
        let stateless = |count: Count| Ok((Statelet::None, count));
        match self {
            // `reduce(self.merge_counts, child_counts)`.
            Operator::Binary(op) => stateless(op.reduce(child_counts)?),
            Operator::MapCounts(f) => stateless(f.call(outcome, child_counts)?),
            Operator::MultiplyCounts(c) => stateless(times(&child_counts[0], c)?),
            // `count == constant` is False and `count != constant` True for a tuple; the others compare.
            Operator::KeepCounts(comparison, _) => stateless(match comparison {
                KeepCountsComparison::Eq => Count::Int(Int::ZERO),
                KeepCountsComparison::Ne => child_counts[0].clone(),
                _ => return Err(tuple_error()),
            }),
            Operator::FilterOutcomes { filter, invert } => {
                stateless(if filter_outcomes::keeps(filter, *invert, outcome)? {
                    child_counts[0].clone()
                } else {
                    Count::Int(Int::ZERO)
                })
            }
            // `target_count > 0` compares; the source count is only passed on.
            Operator::FilterOutcomesBinary { invert } => {
                let target = int(&child_counts[1])?;
                stateless(if target.is_positive() != *invert {
                    child_counts[0].clone()
                } else {
                    Count::Int(Int::ZERO)
                })
            }
            // Once the "while" has ended the right count is never looked at.
            Operator::SortPairWhile { keep, .. } => {
                let left = int(&child_counts[0])?;
                match state {
                    Statelet::SortPairWhile {
                        lexi_tuple: None, ..
                    } => pair::sort_pair_while_next_state(*keep, state, &left, &Int::ZERO),
                    _ => Err(tuple_error()),
                }
            }
            // `left_count * left_first` (or `tie`, `right_first`): a tuple left count is repeated 0 or 1 times.
            Operator::Versus { .. } => {
                let right = int(&child_counts[1])?;
                let Count::Tuple(left) = &child_counts[0] else {
                    unreachable!("a tuple count is the left one")
                };
                let (next, factor) = versus::next_state(state, &Int::ONE, &right)?;
                Ok((next, repeat(left, &int(&factor)?)?))
            }
            Operator::ForceOrder(_) => stateless(child_counts[0].clone()),
            // `count // constant`, `count % constant`, `min(count, constant)`, `max(child_counts[0], 0)`, ...
            Operator::FloordivCounts(_)
            | Operator::ModuloCounts(_)
            | Operator::Unique(_)
            | Operator::Keep(_)
            | Operator::SortPair { .. }
            | Operator::MaxPairLate { .. }
            | Operator::MaxPairEarly { .. } => Err(tuple_error()),
            Operator::Host(_) => unreachable!("next_state calls the host first"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_allows_tuple_counts_up_to_2_pow_24_elements() {
        // Python would build any length; past the limit the port says it does not cover it.
        let t = [Int::ZERO, Int::ONE];
        match repeat(&t, &Int::from(1i64 << 23)).unwrap() {
            Count::Tuple(t) => assert_eq!(t.len(), 1 << 24),
            Count::Int(_) => panic!("a tuple repeated is a tuple"),
        }
        assert!(matches!(
            repeat(&t, &Int::from((1i64 << 23) + 1)),
            Err(Error::Unsupported(_))
        ));
        // Past 64 bits Python raises OverflowError.
        assert!(matches!(
            repeat(&t, &(Int::from(i64::MAX) + 1)),
            Err(Error::Overflow(_))
        ));
        assert_eq!(
            repeat(&[Int::ONE], &Int::from(-2)).unwrap(),
            Count::Tuple(Arc::from([]))
        );
    }

    #[test]
    fn next_state_checks_the_number_of_children() {
        // The driver calls `initial_state` first, which checks the same; `next_state` stands on its own.
        let unique = Operator::<i64>::Unique(Int::ONE);
        let next =
            |counts: &[Count]| unique.next_state(&Statelet::None, Order::Ascending, &1, counts);
        assert!(matches!(next(&[]), Err(Error::Index(_))));
        // Extra children are not read (Python's `child_counts[0]`).
        assert_eq!(
            next(&[Count::Int(3.into()), Count::Int(5.into())]).unwrap(),
            (Statelet::None, Count::Int(Int::ONE))
        );
    }
}
