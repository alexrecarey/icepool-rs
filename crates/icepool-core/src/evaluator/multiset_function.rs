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
use crate::outcome::OutcomeOrder;
use crate::pool::Count;
use crate::Int;

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
///
/// ```
/// use icepool_core::evaluator::basic::SizeEvaluator;
/// use icepool_core::evaluator::multiset_function::{DynEvaluator, Erased};
/// use icepool_core::evaluator::Count;
/// use icepool_core::{Int, Order};
/// // Erased states compare as their own type does.
/// let size = Erased { evaluator: SizeEvaluator, map_final: |n: Int| Ok(n) };
/// let sizes = [Some(Count::Int(Int::from(2)))];
/// let a = DynEvaluator::<i64, Int>::initial_state(&size, Order::Ascending, &[1, 2], &sizes).unwrap();
/// let b = DynEvaluator::<i64, Int>::initial_state(&size, Order::Ascending, &[1, 2], &sizes).unwrap();
/// assert!(a == b);
/// let counts = [Count::Int(Int::from(1))];
/// let c = size.next_state(&a, Order::Ascending, &1, &counts).unwrap().unwrap();
/// assert!(a != c);
/// ```
#[derive(Clone)]
pub struct AnyState(Arc<dyn StateObject>);

/// The state's type is erased (and need not be `Debug`): it prints as a placeholder.
impl std::fmt::Debug for AnyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AnyState(..)")
    }
}

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
///
/// ```
/// use icepool_core::evaluator::basic::SizeEvaluator;
/// use icepool_core::evaluator::multiset_function::{DynEvaluator, Erased};
/// use icepool_core::evaluator::{Count, Final};
/// use icepool_core::{Int, Order};
/// // Evaluators of different state types behind one trait object, finals converted to a shared type.
/// let size: Box<dyn DynEvaluator<i64, String>> =
///     Box::new(Erased { evaluator: SizeEvaluator, map_final: |n: Int| Ok(n.to_string()) });
/// let (order, outcomes) = (Order::Ascending, [1, 2]);
/// let mut state = size.initial_state(order, &outcomes, &[None]).unwrap();
/// for (outcome, count) in [(1, 2), (2, 1)] {
///     state = size.next_state(&state, order, &outcome, &[Count::Int(Int::from(count))]).unwrap().unwrap();
/// }
/// let Final::Outcome(result) = size.final_outcome(&state, order, &outcomes, &[None]).unwrap() else { panic!() };
/// assert_eq!(result, "3");
/// ```
pub trait DynEvaluator<O, F> {
    /// [`MultisetEvaluator::initial_state_counts`].
    ///
    /// ```
    /// use icepool_core::evaluator::basic::SizeEvaluator;
    /// use icepool_core::evaluator::multiset_function::{DynEvaluator, Erased};
    /// use icepool_core::evaluator::{Count, Final};
    /// use icepool_core::{Int, Order};
    /// let size: Box<dyn DynEvaluator<i64, String>> =
    ///     Box::new(Erased { evaluator: SizeEvaluator, map_final: |n: Int| Ok(n.to_string()) });
    /// let state = size.initial_state(Order::Ascending, &[1, 2], &[None]).unwrap();
    /// // A fresh size is 0.
    /// let Final::Outcome(result) = size.final_outcome(&state, Order::Ascending, &[1, 2], &[None]).unwrap() else {
    ///     panic!()
    /// };
    /// assert_eq!(result, "0");
    /// ```
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<AnyState>;
    /// [`MultisetEvaluator::next_state_counts`].
    ///
    /// ```
    /// use icepool_core::evaluator::basic::SizeEvaluator;
    /// use icepool_core::evaluator::multiset_function::{DynEvaluator, Erased};
    /// use icepool_core::evaluator::{Count, Final};
    /// use icepool_core::{Int, Order};
    /// let size: Box<dyn DynEvaluator<i64, String>> =
    ///     Box::new(Erased { evaluator: SizeEvaluator, map_final: |n: Int| Ok(n.to_string()) });
    /// let state = size.initial_state(Order::Ascending, &[4], &[None]).unwrap();
    /// let state = size.next_state(&state, Order::Ascending, &4, &[Count::Int(Int::from(2))]).unwrap().unwrap();
    /// let Final::Outcome(result) = size.final_outcome(&state, Order::Ascending, &[4], &[None]).unwrap() else {
    ///     panic!()
    /// };
    /// assert_eq!(result, "2");
    /// ```
    fn next_state(
        &self,
        state: &AnyState,
        order: Order,
        outcome: &O,
        counts: &[Count],
    ) -> Result<Option<AnyState>>;
    /// [`MultisetEvaluator::extra_outcomes`].
    ///
    /// ```
    /// use icepool_core::evaluator::multiset_function::{DynEvaluator, Erased};
    /// use icepool_core::evaluator::poker::LargestStraightEvaluator;
    /// use icepool_core::Int;
    /// let straight: Box<dyn DynEvaluator<i64, Int>> =
    ///     Box::new(Erased { evaluator: LargestStraightEvaluator, map_final: Ok });
    /// assert_eq!(straight.extra_outcomes(&[1, 3]).unwrap(), vec![1, 2, 3]);
    /// ```
    fn extra_outcomes(&self, outcomes: &[O]) -> Result<Vec<O>>;
    /// [`MultisetEvaluator::final_outcome_counts`], converted to `F`.
    ///
    /// ```
    /// use icepool_core::evaluator::basic::SizeEvaluator;
    /// use icepool_core::evaluator::multiset_function::{DynEvaluator, Erased};
    /// use icepool_core::evaluator::{Count, Final};
    /// use icepool_core::{Int, Order};
    /// let size: Box<dyn DynEvaluator<i64, String>> =
    ///     Box::new(Erased { evaluator: SizeEvaluator, map_final: |n: Int| Ok(n.to_string()) });
    /// let state = size.initial_state(Order::Descending, &[6], &[None]).unwrap();
    /// let state = size.next_state(&state, Order::Descending, &6, &[Count::Int(Int::from(3))]).unwrap().unwrap();
    /// // The size, an `Int`, comes out as `map_final` turns it: a `String`.
    /// let Final::Outcome(result) = size.final_outcome(&state, Order::Descending, &[6], &[None]).unwrap() else {
    ///     panic!()
    /// };
    /// assert_eq!(result, "3");
    /// ```
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
///
/// ```
/// use icepool_core::evaluator::basic::SumEvaluator;
/// use icepool_core::evaluator::multiset_function::{DynEvaluator, Erased};
/// use icepool_core::evaluator::{Count, Final};
/// use icepool_core::{Int, Order};
/// // A sum whose final outcome is converted to a string.
/// let sum = Erased { evaluator: SumEvaluator, map_final: |s: i64| Ok(format!("sum {s}")) };
/// let state = DynEvaluator::<i64, String>::initial_state(&sum, Order::Ascending, &[4], &[None]).unwrap();
/// // Nothing summed yet: Python's `None`, which the result die refuses.
/// assert!(matches!(sum.final_outcome(&state, Order::Ascending, &[4], &[None]), Ok(Final::None)));
/// // Two 4s.
/// let state = sum.next_state(&state, Order::Ascending, &4, &[Count::Int(Int::from(2))]).unwrap().unwrap();
/// let Final::Outcome(result) = sum.final_outcome(&state, Order::Ascending, &[4], &[None]).unwrap() else { panic!() };
/// assert_eq!(result, "sum 8");
/// ```
#[derive(Debug)]
pub struct Erased<E, M> {
    /// The evaluator.
    pub evaluator: E,
    /// Converts its final outcomes.
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
///
/// ```
/// use icepool_core::evaluator::basic::SumEvaluator;
/// use icepool_core::evaluator::multiset_function::{Call, Erased, MultisetFunction};
/// use icepool_core::evaluator::{evaluate_call_tree, MultisetSource};
/// use icepool_core::expression::{free_variable, CallTree, Dungeonlet};
/// use icepool_core::{d, Die};
/// use std::sync::Arc;
/// // `@multiset_function def f(x): return x.sum()`, evaluated on `d(4).pool(2)`: the outer call passes its input
/// // to one inner call, whose input is the parameter `x`.
/// let x = Arc::from(vec![Dungeonlet::Parameter { arg_index: 0, star_index: None }]);
/// let tree = CallTree { flats: vec![free_variable()], calls: vec![CallTree::leaf(vec![x])] };
/// let sum = Call::Evaluator(Box::new(Erased { evaluator: SumEvaluator, map_final: Ok }));
/// let f = MultisetFunction { call: Call::Function(Box::new(sum)), tupleize: Box::new(|_| unreachable!()) };
/// let sources = [MultisetSource::Pool(d(4).pool(2).source())];
/// let finals = evaluate_call_tree(&f, &tree, &sources, |_| Ok(())).unwrap();
/// assert_eq!(Die::from_weights(finals), &d(4) + &d(4));
/// ```
pub enum Call<'a, O, F> {
    /// `MultisetEvaluatorDungeon` / `Quest` (an evaluation in the body, its kwargs bound in the evaluator), or
    /// `ConstantDungeon` / `Quest` (a result that is not an evaluation): no calls inside.
    Evaluator(Box<dyn DynEvaluator<O, F> + 'a>),
    /// `MultisetFunctionDungeon` / `Quest`: one call, whose state is this one's.
    Function(Box<Call<'a, O, F>>),
    /// `MultisetFunctionJointDungeon` / `Quest`: one call per result.
    Joint(Vec<Call<'a, O, F>>),
}

/// An evaluator is a trait object: it prints as its kind, the tree of calls as itself.
impl<O, F> std::fmt::Debug for Call<'_, O, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Call::Evaluator(_) => f.write_str("Evaluator(..)"),
            Call::Function(call) => f.debug_tuple("Function").field(call).finish(),
            Call::Joint(calls) => f.debug_tuple("Joint").field(calls).finish(),
        }
    }
}

/// The main state of a call: an evaluator's own, or the tuple of a joint call's inner states.
///
/// ```
/// use icepool_core::evaluator::basic::SizeEvaluator;
/// use icepool_core::evaluator::multiset_function::{CallState, DynEvaluator, Erased};
/// use icepool_core::{Int, Order};
/// let size = Erased { evaluator: SizeEvaluator, map_final: |n: Int| Ok(n) };
/// let leaf = CallState::Leaf(DynEvaluator::<i64, Int>::initial_state(&size, Order::Ascending, &[1], &[None]).unwrap());
/// // A joint call of two sizes starts from both their initial states.
/// let joint = CallState::Joint(vec![leaf.clone(), leaf.clone()].into());
/// assert!(joint == CallState::Joint(vec![leaf.clone(), leaf.clone()].into()));
/// assert!(joint != leaf);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CallState {
    /// An evaluator's own state.
    Leaf(AnyState),
    /// A joint call's inner states, one per call.
    Joint(Arc<[CallState]>),
}

/// `icepool.tupleize(*final_outcomes)` for a joint call, as the caller does it.
///
/// ```
/// use icepool_core::evaluator::multiset_function::Tupleize;
/// use icepool_core::evaluator::Final;
/// // Results that are vectors, tupleized by concatenating them; a reroll in any result rerolls the tuple.
/// let tupleize: Box<Tupleize<Vec<i64>>> = Box::new(|finals| {
///     let mut tuple = Vec::new();
///     for f in finals {
///         match f {
///             Final::Outcome(f) => tuple.extend(f),
///             other => return Ok(other),
///         }
///     }
///     Ok(Final::Outcome(tuple))
/// });
/// let Ok(Final::Outcome(t)) = tupleize(vec![Final::Outcome(vec![1]), Final::Outcome(vec![2])]) else { panic!() };
/// assert_eq!(t, vec![1, 2]);
/// ```
pub type Tupleize<'a, F> = dyn Fn(Vec<Final<F>>) -> Result<Final<F>> + 'a;

/// A `@multiset_function` evaluation's main side: its outermost [`Call`] (always a `Function` or `Joint`) and how
/// to tupleize joint results.
///
/// ```
/// use icepool_core::evaluator::basic::{SizeEvaluator, SumEvaluator};
/// use icepool_core::evaluator::multiset_function::{Call, Erased, MultisetFunction};
/// use icepool_core::evaluator::{evaluate_call_tree, Final, MultisetSource};
/// use icepool_core::expression::{free_variable, CallTree, Dungeonlet};
/// use icepool_core::{Die, Int, Pool};
/// use std::sync::Arc;
/// // `@multiset_function def f(x): return x.sum(), x.size()` on the multiset `[2, 3]`: a joint call of two
/// // evaluators, whose results are tupleized (here as a `Vec<i64>`).
/// let x = || Arc::from(vec![Dungeonlet::Parameter { arg_index: 0, star_index: None }]);
/// let tree = CallTree { flats: vec![free_variable()], calls: vec![CallTree::leaf(vec![x()]), CallTree::leaf(vec![x()])] };
/// let sum = Erased { evaluator: SumEvaluator, map_final: |s: i64| Ok(vec![s]) };
/// let size = Erased { evaluator: SizeEvaluator, map_final: |n: Int| Ok(vec![n.to_i64().unwrap()]) };
/// let f = MultisetFunction {
///     call: Call::Joint(vec![Call::Evaluator(Box::new(sum)), Call::Evaluator(Box::new(size))]),
///     tupleize: Box::new(|finals| {
///         let mut tuple = Vec::new();
///         for f in finals {
///             match f {
///                 Final::Outcome(f) => tuple.extend(f),
///                 other => return Ok(other),
///             }
///         }
///         Ok(Final::Outcome(tuple))
///     }),
/// };
/// let pool = Pool::new(vec![(Die::single(2), 1), (Die::single(3), 1)]).unwrap();
/// let finals = evaluate_call_tree(&f, &tree, &[MultisetSource::Pool(pool.source())], |_| Ok(())).unwrap();
/// assert_eq!(Die::from_weights(finals), Die::single(vec![5, 2]));
/// ```
pub struct MultisetFunction<'a, O, F> {
    /// The outermost call.
    pub call: Call<'a, O, F>,
    /// How to tupleize the results of a joint call.
    pub tupleize: Box<Tupleize<'a, F>>,
}

/// `tupleize` is a closure: only the calls print.
impl<O, F> std::fmt::Debug for MultisetFunction<'_, O, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultisetFunction")
            .field("call", &self.call)
            .finish_non_exhaustive()
    }
}

fn joint_states(state: &CallState) -> &[CallState] {
    match state {
        CallState::Joint(s) => s,
        CallState::Leaf(_) => &[],
    }
}

impl<'a, O: OutcomeOrder, F> MultisetFunction<'a, O, F> {
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

impl<'a, O: OutcomeOrder, F> MultisetEvaluator<O> for MultisetFunction<'a, O, F> {
    type State = CallState;
    type Final = F;

    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Int>],
    ) -> Result<CallState> {
        let sizes = sizes.iter().map(|s| s.clone().map(Count::Int)).collect();
        self.initial_state_tree(order, outcomes, &CountTree::Leaf(sizes))
    }

    fn next_state(
        &self,
        state: &CallState,
        order: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Option<CallState>> {
        let counts = counts.iter().map(|c| Count::Int(c.clone())).collect();
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
        sizes: &[Option<Int>],
    ) -> Result<Final<F>> {
        let sizes = sizes.iter().map(|s| s.clone().map(Count::Int)).collect();
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
