//! Port of icepool `evaluator/comparison.py`: comparing the multisets produced by two generators.

use super::fold::{count_fold_evaluators, CountFold};
use super::Final;
use crate::error::Result;
use crate::order::Order;
use crate::Int;

count_fold_evaluators!(ComparisonEvaluator);

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
    pub fn any_all(&self, left: &Int, right: &Int) -> (bool, bool) {
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

    /// The final outcome if both left and right have no outcomes.
    pub fn default_outcome(&self) -> bool {
        use Comparison::*;
        match self.0 {
            IsProperSubset | IsProperSuperset | IsNotEqualSet => false,
            IsSubset | IsSuperset | IsEqualSet | IsDisjointSet => true,
        }
    }

    /// `final_outcome` of a final state.
    pub fn final_of(&self, state: Option<(bool, bool)>) -> bool {
        match state {
            None => self.default_outcome(),
            Some((has_any, has_all)) => has_any && has_all,
        }
    }
}

/// `next_state` given this outcome's `(this_any, this_all)`.
pub fn step(state: Option<(bool, bool)>, (this_any, this_all): (bool, bool)) -> (bool, bool) {
    let (has_any, has_all) = state.unwrap_or((false, true));
    let has_all = has_all && this_all;
    let has_any = has_all && (has_any || this_any);
    (has_any, has_all)
}

impl<O> CountFold<O> for ComparisonEvaluator {
    /// `None` before the first outcome, then `(has_any, has_all)`.
    type State = Option<(bool, bool)>;
    type Final = bool;

    fn fold(&self, state: &Self::State, _: Order, _: &O, counts: &[Int]) -> Result<Self::State> {
        Ok(Some(step(*state, self.any_all(&counts[0], &counts[1]))))
    }

    fn finish(&self, state: &Self::State) -> Result<Final<bool>> {
        Ok(Final::Outcome(self.final_of(*state)))
    }
}

#[cfg(test)]
mod tests {
    //! The expected dice are Python icepool's answers; the comment above each gives the Python.

    use super::*;
    use crate::evaluator::test_util::{evaluate_repr, expected, multiset, pool};
    use crate::value::evaluators::PyEvaluator;

    fn compare(comparison: Comparison, right: &[i64]) -> Vec<(String, u32)> {
        evaluate_repr(
            &PyEvaluator(ComparisonEvaluator(comparison)),
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
