//! Port of icepool `evaluator/comparison.py`: comparing the multisets produced by two generators.

use super::{Final, MultisetEvaluator};
use crate::error::Result;
use crate::order::Order;
use crate::value::BinOp::{Ge, Gt, Le, Lt};
use crate::Int;
use crate::Value;

/// Which comparison a `ComparisonEvaluator` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// `<`: a proper subset.
    IsProperSubset,
    /// `<=`: a subset.
    IsSubset,
    /// `>`: a proper superset.
    IsProperSuperset,
    /// `>=`: a superset.
    IsSuperset,
    /// `==`: the same multiset.
    IsEqualSet,
    /// `!=`: different multisets.
    IsNotEqualSet,
    /// `isdisjoint`: no element in common.
    IsDisjointSet,
}

/// Compares the multisets produced by two generators (`ComparisonEvaluator` and its subclasses).
pub struct ComparisonEvaluator(pub Comparison);

impl ComparisonEvaluator {
    /// Called for each outcome and produces a pair of bools. The final outcome is true iff any of the first and
    /// all of the second bool are `True`.
    fn any_all(&self, left: &Int, right: &Int) -> (bool, bool) {
        use Comparison::*;
        match self.0 {
            IsProperSubset => (left < right, left <= right),
            IsSubset => (true, left <= right),
            IsProperSuperset => (left > right, left >= right),
            IsSuperset => (true, left >= right),
            IsEqualSet => (true, left == right),
            IsNotEqualSet => (left != right, true),
            IsDisjointSet => (true, !(left.is_positive() && right.is_positive())),
        }
    }

    /// [`Self::any_all`] for counts that may be tuples, with Python's operators (`<` between tuples is
    /// lexicographic, giving a bool for tuples of ints; between a tuple and an int `TypeError`; `==` just `False`). Python evaluates the pair left to
    /// right, and `left > 0 and right > 0` stops at a false left.
    fn any_all_values(&self, left: &Value, right: &Value) -> Result<(bool, bool)> {
        use Comparison::*;
        let cmp = |op, a: &Value, b: &Value| a.binary(op, b)?.truthy();
        let zero = Value::from(0);
        Ok(match self.0 {
            IsProperSubset => (cmp(Lt, left, right)?, cmp(Le, left, right)?),
            IsSubset => (true, cmp(Le, left, right)?),
            IsProperSuperset => (cmp(Gt, left, right)?, cmp(Ge, left, right)?),
            IsSuperset => (true, cmp(Ge, left, right)?),
            IsEqualSet => (true, left == right),
            IsNotEqualSet => (left != right, true),
            IsDisjointSet => (true, !(cmp(Gt, left, &zero)? && cmp(Gt, right, &zero)?)),
        })
    }

    /// The final outcome if both left and right have no outcomes.
    fn default_outcome(&self) -> bool {
        use Comparison::*;
        match self.0 {
            IsProperSubset | IsProperSuperset | IsNotEqualSet => false,
            IsSubset | IsSuperset | IsEqualSet | IsDisjointSet => true,
        }
    }
}

/// `next_state` given this outcome's `(this_any, this_all)`.
fn step(state: Option<(bool, bool)>, (this_any, this_all): (bool, bool)) -> Option<(bool, bool)> {
    let (has_any, has_all) = state.unwrap_or((false, true));
    let has_all = has_all && this_all;
    let has_any = has_all && (has_any || this_any);
    Some((has_any, has_all))
}

impl MultisetEvaluator<Value> for ComparisonEvaluator {
    /// `None` before the first outcome, then `(has_any, has_all)`.
    type State = Option<(bool, bool)>;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<Int>]) -> Result<Self::State> {
        Ok(None)
    }

    fn next_state(
        &self,
        state: &Self::State,
        _: Order,
        _: &Value,
        counts: &[Int],
    ) -> Result<Option<Self::State>> {
        Ok(Some(step(*state, self.any_all(&counts[0], &counts[1]))))
    }

    fn next_state_values(
        &self,
        state: &Self::State,
        _: Order,
        _: &Value,
        counts: &[Value],
    ) -> Result<Option<Self::State>> {
        Ok(Some(step(
            *state,
            self.any_all_values(&counts[0], &counts[1])?,
        )))
    }

    fn reads_sizes(&self) -> bool {
        false
    }

    fn final_outcome(
        &self,
        final_state: &Self::State,
        _: Order,
        _: &[Value],
        _: &[Option<Int>],
    ) -> Result<Final<Value>> {
        Ok(Final::Outcome(Value::Bool(match final_state {
            None => self.default_outcome(),
            Some((has_any, has_all)) => *has_any && *has_all,
        })))
    }
}

#[cfg(test)]
mod tests {
    //! The expected dice are Python icepool's answers; the comment above each gives the Python.

    use super::*;
    use crate::evaluator::test_util::{evaluate_repr, expected, multiset, pool};

    fn compare(comparison: Comparison, right: &[i64]) -> Vec<(String, u32)> {
        evaluate_repr(
            &ComparisonEvaluator(comparison),
            &[pool(3, &[1, 1]), multiset(right)],
        )
    }

    #[test]
    fn subsets_and_disjoint_sets() {
        // `d(3).pool(2).issubset([1, 2])`
        assert_eq!(
            compare(Comparison::IsSubset, &[1, 2]),
            expected(&[("False", 7), ("True", 2)])
        );
        // `d(3).pool(2) < [1, 2, 3]`
        let proper = compare(Comparison::IsProperSubset, &[1, 2, 3]);
        assert_eq!(proper, expected(&[("False", 3), ("True", 6)]));
        // `d(3).pool(2).isdisjoint([3])`
        assert_eq!(
            compare(Comparison::IsDisjointSet, &[3]),
            expected(&[("False", 5), ("True", 4)])
        );
    }
}
