//! A multiset operator the host implements ([`Operator::Host`](crate::operator::Operator)): in the Python facade,
//! a `MultisetOperator` the core has no variant for (a user's subclass, or a subclass of a built-in one).

use crate::error::Result;
use crate::evaluator::{Count, Order};
use crate::Value;

/// The methods of a host operator node: `_initial_state(order, outcomes, child_sizes, source_sizes, arg_sizes)`
/// and `_next_state(state, order, outcome, child_counts, source_counts, arg_counts)`, each returning the local
/// state and this node's size or count. Only an operator's children are its business: the source and argument
/// counts are for free variables and parameters, so the host gets none.
pub trait HostOperator<O> {
    /// `_initial_state`: the node's first local state and its size (`None` if not known).
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        child_sizes: &[Option<Count>],
    ) -> Result<(Value, Option<Count>)>;

    /// `_next_state`: the node's next local state and its count of `outcome`.
    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &O,
        child_counts: &[Count],
    ) -> Result<(Value, Count)>;
}
