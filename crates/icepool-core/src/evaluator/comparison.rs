//! Port of icepool `evaluator/comparison.py`: comparing the multisets produced by two generators.

use super::{Final, MultisetEvaluator};
use crate::error::Result;
use crate::order::Order;
use crate::Value;

/// Which comparison a `ComparisonEvaluator` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    IsProperSubset,
    IsSubset,
    IsProperSuperset,
    IsSuperset,
    IsEqualSet,
    IsNotEqualSet,
    IsDisjointSet,
}

/// Compares the multisets produced by two generators (`ComparisonEvaluator` and its subclasses).
pub struct ComparisonEvaluator(pub Comparison);

impl ComparisonEvaluator {
    /// Called for each outcome and produces a pair of bools. The final outcome is true iff any of the first and
    /// all of the second bool are `True`.
    fn any_all(&self, left: i64, right: i64) -> (bool, bool) {
        use Comparison::*;
        match self.0 {
            IsProperSubset => (left < right, left <= right),
            IsSubset => (true, left <= right),
            IsProperSuperset => (left > right, left >= right),
            IsSuperset => (true, left >= right),
            IsEqualSet => (true, left == right),
            IsNotEqualSet => (left != right, true),
            IsDisjointSet => (true, !(left > 0 && right > 0)),
        }
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

impl MultisetEvaluator<Value> for ComparisonEvaluator {
    /// `None` before the first outcome, then `(has_any, has_all)`.
    type State = Option<(bool, bool)>;
    type Final = Value;

    fn initial_state(&self, _: Order, _: &[Value], _: &[Option<i64>]) -> Result<Self::State> {
        Ok(None)
    }

    fn next_state(
        &self,
        state: &Self::State,
        _: Order,
        _: &Value,
        counts: &[i64],
    ) -> Result<Option<Self::State>> {
        let (has_any, has_all) = state.unwrap_or((false, true));
        let (this_any, this_all) = self.any_all(counts[0], counts[1]);
        let has_all = has_all && this_all;
        let has_any = has_all && (has_any || this_any);
        Ok(Some(Some((has_any, has_all))))
    }

    fn final_outcome(
        &self,
        final_state: &Self::State,
        _: Order,
        _: &[Value],
        _: &[Option<i64>],
    ) -> Result<Final<Value>> {
        Ok(Final::Outcome(Value::Bool(match final_state {
            None => self.default_outcome(),
            Some((has_any, has_all)) => *has_any && *has_all,
        })))
    }
}
