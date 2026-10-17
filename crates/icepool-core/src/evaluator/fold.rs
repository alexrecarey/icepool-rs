//! [`CountFold`]: the shape most built-in evaluators share, written once.
//!
//! Python's poker evaluators override only `next_state` (and sometimes `extra_outcomes`, `initial_state` or
//! `final_outcome`): they fold each `(outcome, counts)` into a state, read the result off the final state, and
//! never look at the sizes. A [`CountFold`] says just that, and `count_fold_evaluators!` makes it a
//! `MultisetEvaluator`.

use std::hash::Hash;

use super::Final;
use crate::error::Result;
use crate::order::Order;
use crate::Int;

/// An evaluator that folds `(outcome, counts)` into a state and reads its result off the final state: no sizes,
/// no rerolls.
pub trait CountFold<O> {
    /// The state. Python's starts as `None` unless `initial_state` says otherwise; here that is `Default` (an
    /// `Option`, or a value Python's `state or ...` treats the same as `None`).
    type State: Clone + Eq + Hash + Default;
    /// The final outcome.
    type Final;

    /// `initial_state`. The default is `State::default()`.
    fn initial(&self, _order: Order, _outcomes: &[O]) -> Result<Self::State> {
        Ok(Self::State::default())
    }

    /// `extra_outcomes`: outcomes `fold` should also see (with zero counts). The default is none.
    fn extra(&self, _outcomes: &[O]) -> Result<Vec<O>> {
        Ok(Vec::new())
    }

    /// `next_state`: the next state given the previous state, an outcome, and the count of that outcome produced
    /// by each input.
    fn fold(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Self::State>;

    /// `final_outcome`.
    fn finish(&self, state: &Self::State) -> Result<Final<Self::Final>>;
}

/// `impl MultisetEvaluator<O> for` each listed type through its [`CountFold`] (`Type` or `Type<Params>`).
///
/// A blanket `impl<T: CountFold<O>> MultisetEvaluator<O> for T` would be simpler, but Rust's coherence rules
/// would then forbid any other generic `impl MultisetEvaluator<O>` (a downstream crate could make that type a
/// `CountFold` too).
macro_rules! count_fold_evaluators {
    ($($t:ident $(<$($p:ident),*>)?),* $(,)?) => {$(
        impl<O: $crate::outcome::OutcomeOrder $($(, $p)*)?> $crate::evaluator::MultisetEvaluator<O>
            for $t $(<$($p),*>)?
        where
            Self: $crate::evaluator::fold::CountFold<O>,
        {
            type State = <Self as $crate::evaluator::fold::CountFold<O>>::State;
            type Final = <Self as $crate::evaluator::fold::CountFold<O>>::Final;

            fn initial_state(
                &self,
                order: $crate::order::Order,
                outcomes: &[O],
                _: &[Option<$crate::Int>],
            ) -> $crate::Result<Self::State> {
                self.initial(order, outcomes)
            }

            fn next_state(
                &self,
                state: &Self::State,
                order: $crate::order::Order,
                outcome: &O,
                counts: &[$crate::Int],
            ) -> $crate::Result<Option<Self::State>> {
                self.fold(state, order, outcome, counts).map(Some)
            }

            fn extra_outcomes(&self, outcomes: &[O]) -> $crate::Result<Vec<O>> {
                self.extra(outcomes)
            }

            fn final_outcome(
                &self,
                state: &Self::State,
                _: $crate::order::Order,
                _: &[O],
                _: &[Option<$crate::Int>],
            ) -> $crate::Result<$crate::evaluator::Final<Self::Final>> {
                self.finish(state)
            }

            fn reads_sizes(&self) -> bool {
                false
            }
        }
    )*};
}

pub(crate) use count_fold_evaluators;
