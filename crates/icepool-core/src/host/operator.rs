//! A multiset operator the host implements ([`Operator::Host`](crate::operator::Operator)): in the Python facade,
//! a `MultisetOperator` the core has no variant for (a user's subclass, or a subclass of a built-in one).

use crate::error::Result;
use crate::evaluator::{Count, Order};
use crate::Value;

/// The methods of a host operator node: `_initial_state(order, outcomes, child_sizes, source_sizes, arg_sizes)`
/// and `_next_state(state, order, outcome, child_counts, source_counts, arg_counts)`, each returning the local
/// state and this node's size or count. Only an operator's children are its business: the source and argument
/// counts are for free variables and parameters, so the host gets none.
///
/// ```
/// use std::sync::Arc;
/// use icepool_core::evaluator::{Count, Order};
/// use icepool_core::expression::{Dungeonlet, Operator};
/// use icepool_core::host::HostOperator;
/// use icepool_core::{Error, Result, Value};
/// /// A user's operator: doubles the count of every outcome (as `multiply_counts(2)`), with no state of its own.
/// struct Double;
/// fn double(count: &Count) -> Result<Count> {
///     match count {
///         Count::Int(c) => Ok(Count::Int(c * 2)),
///         Count::Tuple(_) => Err(Error::WrongType("expected an int count".into())),
///     }
/// }
/// impl HostOperator<i64> for Double {
///     fn initial_state(&self, _: Order, _: &[i64], child_sizes: &[Option<Count>]) -> Result<(Value, Option<Count>)> {
///         Ok((Value::None, child_sizes[0].as_ref().map(double).transpose()?))
///     }
///     fn next_state(&self, state: &Value, _: Order, _: &i64, child_counts: &[Count]) -> Result<(Value, Count)> {
///         Ok((state.clone(), double(&child_counts[0])?))
///     }
/// }
/// // `Double` over a pool's counts: Python's `d(3).pool(2).multiply_counts(2).sum()`, here with the operator node
/// // built by hand, as a binding prepares a user's expression.
/// use icepool_core::evaluator::{basic::SumEvaluator, evaluate_expressions, MultisetSource};
/// use icepool_core::{d, Die};
/// let flat = vec![
///     Dungeonlet::FreeVariable,
///     Dungeonlet::Operator { operator: Operator::Host(Arc::new(Double)), children: vec![0] },
/// ];
/// let sources = [MultisetSource::Pool(d(3).pool(2).source())];
/// let finals = evaluate_expressions(&SumEvaluator, &[flat.into()], &sources, |_| Ok(())).unwrap();
/// assert_eq!(Die::from_weights(finals), (&d(3) + &d(3)) * 2);
/// ```
pub trait HostOperator<O> {
    /// `_initial_state`: the node's first local state and its size (`None` if not known).
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use icepool_core::evaluator::{Count, Order};
    /// # use icepool_core::expression::{Dungeonlet, Operator};
    /// # use icepool_core::host::HostOperator;
    /// # use icepool_core::{Error, Result, Value};
    /// # // The `Double` operator of the trait's example.
    /// # struct Double;
    /// # fn double(count: &Count) -> Result<Count> {
    /// #     match count {
    /// #         Count::Int(c) => Ok(Count::Int(c * 2)),
    /// #         Count::Tuple(_) => Err(Error::WrongType("expected an int count".into())),
    /// #     }
    /// # }
    /// # impl HostOperator<i64> for Double {
    /// #     fn initial_state(
    /// #         &self, _: Order, _: &[i64], child_sizes: &[Option<Count>],
    /// #     ) -> Result<(Value, Option<Count>)> {
    /// #         Ok((Value::None, child_sizes[0].as_ref().map(double).transpose()?))
    /// #     }
    /// #     fn next_state(&self, state: &Value, _: Order, _: &i64, child_counts: &[Count]) -> Result<(Value, Count)> {
    /// #         Ok((state.clone(), double(&child_counts[0])?))
    /// #     }
    /// # }
    /// use icepool_core::Int;
    /// // The core calls back from the operator node, and keeps the host's state for the next call.
    /// let node = Operator::Host(Arc::new(Double));
    /// let (_, size) = node.initial_state(Order::Ascending, &[1, 2, 3], &[Some(Count::Int(Int::from(2)))]).unwrap();
    /// assert_eq!(size, Some(Count::Int(Int::from(4))));
    /// ```
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        child_sizes: &[Option<Count>],
    ) -> Result<(Value, Option<Count>)>;

    /// `_next_state`: the node's next local state and its count of `outcome`.
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use icepool_core::evaluator::{Count, Order};
    /// # use icepool_core::expression::{Dungeonlet, Operator};
    /// # use icepool_core::host::HostOperator;
    /// # use icepool_core::{Error, Result, Value};
    /// # // The `Double` operator of the trait's example.
    /// # struct Double;
    /// # fn double(count: &Count) -> Result<Count> {
    /// #     match count {
    /// #         Count::Int(c) => Ok(Count::Int(c * 2)),
    /// #         Count::Tuple(_) => Err(Error::WrongType("expected an int count".into())),
    /// #     }
    /// # }
    /// # impl HostOperator<i64> for Double {
    /// #     fn initial_state(
    /// #         &self, _: Order, _: &[i64], child_sizes: &[Option<Count>],
    /// #     ) -> Result<(Value, Option<Count>)> {
    /// #         Ok((Value::None, child_sizes[0].as_ref().map(double).transpose()?))
    /// #     }
    /// #     fn next_state(&self, state: &Value, _: Order, _: &i64, child_counts: &[Count]) -> Result<(Value, Count)> {
    /// #         Ok((state.clone(), double(&child_counts[0])?))
    /// #     }
    /// # }
    /// use icepool_core::Int;
    /// let node = Operator::Host(Arc::new(Double));
    /// let (state, _) = node.initial_state(Order::Ascending, &[1, 2, 3], &[None]).unwrap();
    /// let (_, count) = node.next_state(&state, Order::Ascending, &1, &[Count::Int(Int::from(1))]).unwrap();
    /// assert_eq!(count, Count::Int(Int::from(2)));
    /// ```
    fn next_state(
        &self,
        state: &Value,
        order: Order,
        outcome: &O,
        child_counts: &[Count],
    ) -> Result<(Value, Count)>;
}

/// A host's operator prints as its kind alone: what it computes is the host's code.
impl<O> std::fmt::Debug for dyn HostOperator<O> + Send + Sync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostOperator")
    }
}
