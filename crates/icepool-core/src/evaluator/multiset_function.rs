//! Port of the evaluation side of icepool `evaluator/multiset_function.py`: the dungeons and quests a
//! `@multiset_function` evaluation is prepared into (`MultisetFunctionDungeon` / `Quest`, the joint variants, and
//! the plain evaluators called in the body), run by the core driver.
//!
//! Tracing the function body (calling it with parameters to find what it evaluates) is the caller's business: the
//! Python facade and the program layer trace, then describe the result as a [`Call`] (the main state machine) and
//! a matching `CallTree` (the expressions, `crate::expression`). The driver hands the evaluation the count tree
//! of the call tree, and [`MultisetFunction`] walks both trees together:
//!
//! - [`Call::Evaluator`]: an evaluator called in the body (`MultisetEvaluatorDungeon` / `Quest`), or a constant
//!   result (`ConstantDungeon` / `Quest`, an evaluator with no inputs); it gets the counts of its own flats.
//! - [`Call::Function`]: a (possibly nested) `@multiset_function` with one result; its state is its inner call's.
//! - [`Call::Joint`]: one returning a tuple of results; its state is the tuple of the inner states, its final
//!   outcome `icepool.tupleize` of theirs.
//!
//! The evaluators of one function differ in type, so they are type-erased ([`DynEvaluator`], states as
//! [`AnyState`]); final outcomes share one type `F` (the caller's: a program value, a Python object).

use std::any::Any;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{Final, MultisetEvaluator};
use crate::error::Result;
use crate::expression::CountTree;
use crate::function::sorted_union;
use crate::order::Order;
use crate::outcome::Outcome;
use crate::pool::Count;

/// A state of any hashable type: `==` and hash are the type's own (states of different types are never compared:
/// each position of the call tree has one evaluator).
trait StateObject: Any {
    fn eq_dyn(&self, other: &dyn StateObject) -> bool;
    fn hash_dyn(&self, state: &mut dyn Hasher);
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Eq + Hash> StateObject for T {
    fn eq_dyn(&self, other: &dyn StateObject) -> bool {
        other.as_any().downcast_ref::<T>() == Some(self)
    }

    fn hash_dyn(&self, mut state: &mut dyn Hasher) {
        self.hash(&mut state);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The state of a type-erased evaluator.
#[derive(Clone)]
pub struct AnyState(Arc<dyn StateObject>);

impl PartialEq for AnyState {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_dyn(&*other.0)
    }
}

impl Eq for AnyState {}

impl Hash for AnyState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash_dyn(state);
    }
}

impl AnyState {
    fn get<T: 'static>(&self) -> &T {
        self.0
            .as_any()
            .downcast_ref()
            .expect("a state goes back to the evaluator that made it")
    }
}

/// A [`MultisetEvaluator`] with its state type erased and its final outcomes converted to `F`.
pub trait DynEvaluator<O, F> {
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<AnyState>;
    fn next_state(
        &self,
        state: &AnyState,
        order: Order,
        outcome: &O,
        counts: &[Count],
    ) -> Result<Option<AnyState>>;
    fn extra_outcomes(&self, outcomes: &[O]) -> Result<Vec<O>>;
    fn final_outcome(
        &self,
        state: &AnyState,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<Final<F>>;
}

/// A shared evaluator (one recorded in a function body and reused by every trace of it).
impl<O, F, T: DynEvaluator<O, F> + ?Sized> DynEvaluator<O, F> for Arc<T> {
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<AnyState> {
        (**self).initial_state(order, outcomes, sizes)
    }

    fn next_state(
        &self,
        state: &AnyState,
        order: Order,
        outcome: &O,
        counts: &[Count],
    ) -> Result<Option<AnyState>> {
        (**self).next_state(state, order, outcome, counts)
    }

    fn extra_outcomes(&self, outcomes: &[O]) -> Result<Vec<O>> {
        (**self).extra_outcomes(outcomes)
    }

    fn final_outcome(
        &self,
        state: &AnyState,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<Final<F>> {
        (**self).final_outcome(state, order, outcomes, sizes)
    }
}

/// `evaluator` as a [`DynEvaluator`], its final outcomes converted by `map_final`.
pub struct Erased<E, M> {
    pub evaluator: E,
    pub map_final: M,
}

impl<O, F, E, M> DynEvaluator<O, F> for Erased<E, M>
where
    E: MultisetEvaluator<O>,
    E::State: 'static,
    M: Fn(E::Final) -> Result<F>,
{
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<AnyState> {
        let state = self
            .evaluator
            .initial_state_counts(order, outcomes, sizes)?;
        Ok(AnyState(Arc::new(state)))
    }

    fn next_state(
        &self,
        state: &AnyState,
        order: Order,
        outcome: &O,
        counts: &[Count],
    ) -> Result<Option<AnyState>> {
        let next = self
            .evaluator
            .next_state_counts(state.get(), order, outcome, counts)?;
        Ok(next.map(|s| AnyState(Arc::new(s))))
    }

    fn extra_outcomes(&self, outcomes: &[O]) -> Result<Vec<O>> {
        self.evaluator.extra_outcomes(outcomes)
    }

    fn final_outcome(
        &self,
        state: &AnyState,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<Final<F>> {
        Ok(
            match self
                .evaluator
                .final_outcome_counts(state.get(), order, outcomes, sizes)?
            {
                Final::Outcome(f) => Final::Outcome((self.map_final)(f)?),
                Final::Reroll => Final::Reroll,
                Final::None => Final::None,
            },
        )
    }
}

/// The main side of a prepared `@multiset_function` evaluation, shaped like its call tree.
pub enum Call<'a, O, F> {
    /// `MultisetEvaluatorDungeon` / `Quest` (an evaluation in the body, its kwargs bound in the evaluator), or
    /// `ConstantDungeon` / `Quest` (a result that is not an evaluation): no calls inside.
    Evaluator(Box<dyn DynEvaluator<O, F> + 'a>),
    /// `MultisetFunctionDungeon` / `Quest`: one call, whose state is this one's.
    Function(Box<Call<'a, O, F>>),
    /// `MultisetFunctionJointDungeon` / `Quest`: one call per result.
    Joint(Vec<Call<'a, O, F>>),
}

/// The main state of a call: an evaluator's own, or the tuple of a joint call's inner states.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum CallState {
    Leaf(AnyState),
    Joint(Arc<[CallState]>),
}

/// `icepool.tupleize(*final_outcomes)` for a joint call, as the caller does it.
pub type Tupleize<'a, F> = dyn Fn(Vec<Final<F>>) -> Result<Final<F>> + 'a;

/// A `@multiset_function` evaluation's main side: its outermost [`Call`] (always a `Function` or `Joint`) and how
/// to tupleize joint results.
pub struct MultisetFunction<'a, O, F> {
    pub call: Call<'a, O, F>,
    pub tupleize: Box<Tupleize<'a, F>>,
}

fn joint_states(state: &CallState) -> &[CallState] {
    match state {
        CallState::Joint(s) => s,
        CallState::Leaf(_) => &[],
    }
}

impl<'a, O: Outcome, F> MultisetFunction<'a, O, F> {
    /// `Quest.initial_state_main`.
    fn initial(
        call: &Call<'a, O, F>,
        order: Order,
        outcomes: &[O],
        sizes: &CountTree<Option<Count>>,
    ) -> Result<CallState> {
        Ok(match call {
            Call::Evaluator(e) => {
                CallState::Leaf(e.initial_state(order, outcomes, sizes.leaf()?)?)
            }
            // `self.inner_quest.initial_state_main(order, outcomes, *arg_sizes[0], **self.inner_kwargs)`
            Call::Function(inner) => Self::initial(inner, order, outcomes, &sizes.calls()[0])?,
            Call::Joint(inners) => CallState::Joint(
                inners
                    .iter()
                    .zip(sizes.calls())
                    .map(|(inner, sizes)| Self::initial(inner, order, outcomes, sizes))
                    .collect::<Result<_>>()?,
            ),
        })
    }

    /// `Dungeon.next_state_main`: `None` is Reroll (a joint call returns `Restart` as soon as any inner call
    /// rerolls, without stepping the rest).
    fn next(
        call: &Call<'a, O, F>,
        state: &CallState,
        order: Order,
        outcome: &O,
        counts: &CountTree<Count>,
    ) -> Result<Option<CallState>> {
        Ok(match (call, state) {
            (Call::Evaluator(e), CallState::Leaf(s)) => e
                .next_state(s, order, outcome, counts.leaf()?)?
                .map(CallState::Leaf),
            (Call::Function(inner), state) => {
                Self::next(inner, state, order, outcome, &counts.calls()[0])?
            }
            (Call::Joint(inners), state) => {
                let mut next_state = Vec::with_capacity(inners.len());
                for ((inner, state), counts) in
                    inners.iter().zip(joint_states(state)).zip(counts.calls())
                {
                    match Self::next(inner, state, order, outcome, counts)? {
                        Some(s) => next_state.push(s),
                        None => return Ok(None),
                    }
                }
                Some(CallState::Joint(next_state.into()))
            }
            (Call::Evaluator(_), CallState::Joint(_)) => {
                unreachable!("states follow the shape of the call tree")
            }
        })
    }

    /// `Quest.extra_outcomes`: a joint call's are the sorted union of its inner calls'.
    fn extra(call: &Call<'a, O, F>, outcomes: &[O]) -> Result<Vec<O>> {
        match call {
            Call::Evaluator(e) => e.extra_outcomes(outcomes),
            Call::Function(inner) => Self::extra(inner, outcomes),
            Call::Joint(inners) => {
                let extras = inners
                    .iter()
                    .map(|inner| Self::extra(inner, outcomes))
                    .collect::<Result<Vec<_>>>()?;
                sorted_union(extras.iter().map(|e| e.iter()))
            }
        }
    }

    /// `Quest.final_outcome`: a joint call's is `icepool.tupleize` of its inner calls' (all computed first).
    fn finish(
        &self,
        call: &Call<'a, O, F>,
        state: &CallState,
        order: Order,
        outcomes: &[O],
        sizes: &CountTree<Option<Count>>,
    ) -> Result<Final<F>> {
        match (call, state) {
            (Call::Evaluator(e), CallState::Leaf(s)) => {
                e.final_outcome(s, order, outcomes, sizes.leaf()?)
            }
            (Call::Function(inner), state) => {
                self.finish(inner, state, order, outcomes, &sizes.calls()[0])
            }
            (Call::Joint(inners), state) => {
                let finals = inners
                    .iter()
                    .zip(joint_states(state))
                    .zip(sizes.calls())
                    .map(|((inner, state), sizes)| {
                        self.finish(inner, state, order, outcomes, sizes)
                    })
                    .collect::<Result<Vec<_>>>()?;
                (self.tupleize)(finals)
            }
            (Call::Evaluator(_), CallState::Joint(_)) => {
                unreachable!("states follow the shape of the call tree")
            }
        }
    }
}

impl<'a, O: Outcome, F> MultisetEvaluator<O> for MultisetFunction<'a, O, F> {
    type State = CallState;
    type Final = F;

    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<i64>],
    ) -> Result<CallState> {
        let sizes = sizes.iter().map(|s| s.map(Count::Int)).collect();
        self.initial_state_tree(order, outcomes, &CountTree::Leaf(sizes))
    }

    fn next_state(
        &self,
        state: &CallState,
        order: Order,
        outcome: &O,
        counts: &[i64],
    ) -> Result<Option<CallState>> {
        let counts = counts.iter().map(|&c| Count::Int(c)).collect();
        self.next_state_tree(state, order, outcome, &CountTree::Leaf(counts))
    }

    fn extra_outcomes(&self, outcomes: &[O]) -> Result<Vec<O>> {
        Self::extra(&self.call, outcomes)
    }

    fn final_outcome(
        &self,
        final_state: &CallState,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<i64>],
    ) -> Result<Final<F>> {
        let sizes = sizes.iter().map(|s| s.map(Count::Int)).collect();
        self.final_outcome_tree(final_state, order, outcomes, &CountTree::Leaf(sizes))
    }

    fn initial_state_counts(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<CallState> {
        self.initial_state_tree(order, outcomes, &CountTree::Leaf(sizes.to_vec()))
    }

    fn next_state_counts(
        &self,
        state: &CallState,
        order: Order,
        outcome: &O,
        counts: &[Count],
    ) -> Result<Option<CallState>> {
        self.next_state_tree(state, order, outcome, &CountTree::Leaf(counts.to_vec()))
    }

    fn final_outcome_counts(
        &self,
        final_state: &CallState,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<Final<F>> {
        self.final_outcome_tree(
            final_state,
            order,
            outcomes,
            &CountTree::Leaf(sizes.to_vec()),
        )
    }

    fn initial_state_tree(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &CountTree<Option<Count>>,
    ) -> Result<CallState> {
        Self::initial(&self.call, order, outcomes, sizes)
    }

    fn next_state_tree(
        &self,
        state: &CallState,
        order: Order,
        outcome: &O,
        counts: &CountTree<Count>,
    ) -> Result<Option<CallState>> {
        Self::next(&self.call, state, order, outcome, counts)
    }

    fn final_outcome_tree(
        &self,
        final_state: &CallState,
        order: Order,
        outcomes: &[O],
        sizes: &CountTree<Option<Count>>,
    ) -> Result<Final<F>> {
        self.finish(&self.call, final_state, order, outcomes, sizes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn h(s: &AnyState) -> u64 {
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn erased_states_hash_as_their_type_does() {
        // The driver keys its state tables by these: equal states must hash alike, and distinct ones should not
        // all collide.
        let state = |x: i64| AnyState(Arc::new(x));
        assert_eq!(h(&state(3)), h(&state(3)));
        assert_ne!(h(&state(3)), h(&state(4)));
        assert!(state(3) == state(3) && state(3) != state(4));
    }
}
