//! Port of icepool `evaluator/multiset_evaluator_base.py` and `evaluator/multiset_evaluator.py`: the dynamic
//! program that evaluates multisets one outcome at a time.
//!
//! A [`MultisetEvaluator`] is a state machine: `initial_state`, then `next_state` once per outcome (in ascending or
//! descending order) with the count of that outcome from each input, then `final_outcome`. The driver
//! ([`evaluate`]) pops outcomes off the sources (pools), weighting each possible count, and memoises every
//! intermediate "room" so that equal sub-problems are solved once.
//!
//! Each input is a prepared multiset expression (`crate::expression`): a generator alone is one free variable
//! whose source's count is passed straight through; operators between generators compute the input's count from
//! theirs, with their own local states ("statelets") carried alongside the evaluator's state. A multiset tuple
//! generator (`MultiDeal`) produces a tuple of counts per outcome and a tuple size ([`Count::Tuple`]).
//! A `@multiset_function` evaluation is a call tree (`crate::expression::CallTree`) whose counts and sizes reach
//! the evaluator as trees ([`multiset_function`]).

pub mod argsort;
pub mod basic;
pub mod comparison;
pub mod fold;
pub mod keep;
pub mod lexi;
pub mod multiset_function;
pub mod poker;

use std::hash::Hash;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::expression::{self, CallTree, CountTree, Flat, Statelets};
use crate::function::sorted_union;
use crate::order::merge_order_preferences;
use crate::outcome::OutcomeOrder;

// What the driver pops outcomes from, and the order it pops them in: part of the evaluator's interface.
pub use crate::deal::DealSource;
pub use crate::multi_deal::{HandGroup, MultiDealSource};
pub use crate::order::{Order, OrderReason};
pub use crate::pool::{Count, MultisetSource, PoolSource, Popped, SourcePopped};
use crate::Int;
use crate::Quantity;
use crate::Value;

/// What `final_outcome` produced: an outcome for the result `Die`, `icepool.Reroll` (dropped), or `None` (which
/// `finalize_evaluation` refuses with `TypeError`).
pub enum Final<F> {
    /// An outcome of the result die.
    Outcome(F),
    /// `icepool.Reroll`: the final state is dropped from the result.
    Reroll,
    /// Python's `None`, which the result die refuses.
    None,
}

/// Evaluates a multiset based on a state transition function (`MultisetEvaluator`).
///
/// Evaluator parameters (Python's `**kwargs` to `evaluate()`) are fields of the implementing type.
/// `UnsupportedOrder` is `Err(Error::UnsupportedOrder)`; the driver then retries in the opposite order.
pub trait MultisetEvaluator<O> {
    /// States must be hashable. `Eq`/`Hash` are Python's `==`/`hash`: equal states merge, keeping the first one.
    type State: Clone + Eq + Hash;
    /// What `final_outcome` returns (the result die's outcomes, or something the caller turns into one).
    type Final;

    /// Optional method to produce an initial evaluation state. If not overridden in Python, the initial state is
    /// `None`.
    ///
    /// `order` is the order in which outcomes will be seen by `next_state()`; `outcomes` all outcomes that will be
    /// seen; `sizes` the sizes of the input multisets, provided that the multiset has inferrable size with
    /// non-negative counts (otherwise `None`).
    fn initial_state(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Int>],
    ) -> Result<Self::State>;

    /// State transition function: the next state given the previous state, an outcome, and the count of that
    /// outcome produced by each input. `Ok(None)` is `icepool.Reroll`: the state is removed from consideration,
    /// effectively performing a full reroll.
    fn next_state(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &[Int],
    ) -> Result<Option<Self::State>>;

    /// Optional method to specify extra outcomes that should be seen as inputs to `next_state()`. These will be
    /// seen by `next_state` even if they do not appear in the input(s). The default is no additional outcomes.
    fn extra_outcomes(&self, _outcomes: &[O]) -> Result<Vec<O>> {
        Ok(Vec::new())
    }

    /// Generates a final outcome from a final state.
    fn final_outcome(
        &self,
        final_state: &Self::State,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Int>],
    ) -> Result<Final<Self::Final>>;

    /// Whether `initial_state` / `final_outcome` read the sizes at all. Python passes a tuple size (a `MultiDeal`
    /// input) to them like any other; an evaluator that never looks at its sizes says `false` here, and then a
    /// tuple size reaches it as `None` (unread). The default `true` keeps tuple sizes `Unsupported` for evaluators
    /// that were not written for them.
    fn reads_sizes(&self) -> bool {
        true
    }

    /// [`Self::next_state`] when some count is a tuple (a `MultiDeal` input, or tuple counts passed on by
    /// operators), every count as the Python value it is (`int` or `tuple` of `int`s). Python's evaluators have no
    /// special case for a tuple count: their arithmetic just runs on it (`outcome * count` repeats a tuple,
    /// `count >= 1` is `TypeError`, `count == 0` is `False`...), so an override ports `next_state` over `Value`s.
    /// The default says `Unsupported`.
    fn next_state_values(
        &self,
        _state: &Self::State,
        _order: Order,
        _outcome: &O,
        _counts: &[Value],
    ) -> Result<Option<Self::State>> {
        Err(tuple_unsupported())
    }

    /// What the driver calls: [`Self::initial_state`] with the sizes as the sources give them. An `int` size is
    /// passed on; a tuple size reaches only evaluators that override this or do not read sizes
    /// ([`Self::reads_sizes`]), the others say `Unsupported`.
    fn initial_state_counts(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<Self::State> {
        self.initial_state(order, outcomes, &int_sizes(sizes, self.reads_sizes())?)
    }

    /// What the driver calls: [`Self::next_state`] with the counts as the sources give them; if some count is a
    /// tuple, [`Self::next_state_values`].
    fn next_state_counts(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &[Count],
    ) -> Result<Option<Self::State>> {
        match int_counts(counts) {
            Some(counts) => self.next_state(state, order, outcome, &counts),
            None => {
                let counts: Vec<Value> = counts.iter().map(expression::count_value).collect();
                self.next_state_values(state, order, outcome, &counts)
            }
        }
    }

    /// What the driver calls: [`Self::final_outcome`] with the sizes as the sources give them (see
    /// [`Self::initial_state_counts`]).
    fn final_outcome_counts(
        &self,
        final_state: &Self::State,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<Final<Self::Final>> {
        self.final_outcome(final_state, order, outcomes, &int_sizes(sizes, self.reads_sizes())?)
    }

    /// What the driver calls: [`Self::initial_state_counts`] with the size tree of the evaluation's call tree. Only
    /// a `@multiset_function` evaluation has calls; a plain evaluator gets the sizes of its inputs.
    fn initial_state_tree(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &CountTree<Option<Count>>,
    ) -> Result<Self::State> {
        self.initial_state_counts(order, outcomes, sizes.leaf()?)
    }

    /// What the driver calls: [`Self::next_state_counts`] with the count tree (see [`Self::initial_state_tree`]).
    fn next_state_tree(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &CountTree<Count>,
    ) -> Result<Option<Self::State>> {
        self.next_state_counts(state, order, outcome, counts.leaf()?)
    }

    /// What the driver calls: [`Self::final_outcome_counts`] with the size tree (see [`Self::initial_state_tree`]).
    fn final_outcome_tree(
        &self,
        final_state: &Self::State,
        order: Order,
        outcomes: &[O],
        sizes: &CountTree<Option<Count>>,
    ) -> Result<Final<Self::Final>> {
        self.final_outcome_counts(final_state, order, outcomes, sizes.leaf()?)
    }
}

pub(crate) fn tuple_unsupported() -> Error {
    Error::Unsupported("a multiset tuple input (MultiDeal) to this evaluator".into())
}

/// The `TypeError` Python raises comparing a tuple count with an `int` (`count >= 1`; `max(count, 0)` compares
/// `0 > count`). A one-input evaluator whose `next_state` starts by such a comparison raises it for any tuple count
/// (its one count is the tuple whenever [`MultisetEvaluator::next_state_values`] is called).
pub(crate) fn tuple_compared_with_int(op: &str) -> Error {
    Error::Type(format!(
        "'{op}' not supported between instances of 'tuple' and 'int'"
    ))
}

/// The sizes as `int`s; a tuple size is `None` if the evaluator does not read sizes, else `Unsupported`.
fn int_sizes(sizes: &[Option<Count>], reads_sizes: bool) -> Result<Vec<Option<Int>>> {
    sizes
        .iter()
        .map(|s| match s {
            None => Ok(None),
            Some(Count::Int(s)) => Ok(Some(s.clone())),
            Some(Count::Tuple(_)) if !reads_sizes => Ok(None),
            Some(Count::Tuple(_)) => Err(tuple_unsupported()),
        })
        .collect()
}

/// The counts as `int`s, if none is a tuple.
pub(crate) fn int_counts(counts: &[Count]) -> Option<Vec<Int>> {
    counts
        .iter()
        .map(|c| match c {
            Count::Int(c) => Some(c.clone()),
            Count::Tuple(_) => None,
        })
        .collect()
}

/// A Python `defaultdict`: insertion order, and a key equal to an earlier one finds the earlier entry (keeping
/// the earlier key).
#[derive(Debug)]
pub struct OrderedMap<K, V> {
    index: FxHashMap<K, usize>,
    items: Vec<(K, V)>,
}

impl<K, V> Default for OrderedMap<K, V> {
    fn default() -> Self {
        OrderedMap {
            index: FxHashMap::default(),
            items: Vec::new(),
        }
    }
}

impl<K, V> OrderedMap<K, V> {
    /// Every `(key, value)` in insertion order.
    pub fn items(&self) -> &[(K, V)] {
        &self.items
    }
}

impl<K: Clone + Eq + Hash, V: Default> OrderedMap<K, V> {
    fn new() -> Self {
        Self::default()
    }

    /// `d[key]`, inserting the default value if missing.
    fn entry(&mut self, key: K) -> &mut V {
        let i = match self.index.get(&key) {
            Some(&i) => i,
            None => {
                self.index.insert(key.clone(), self.items.len());
                self.items.push((key, V::default()));
                self.items.len() - 1
            }
        };
        &mut self.items[i].1
    }
}

/// A Python `defaultdict(int)` from state to weight: equal states add up.
pub type StateWeights<S> = OrderedMap<S, Quantity>;

impl<S: Clone + Eq + Hash> StateWeights<S> {
    fn add(&mut self, state: S, weight: Quantity) {
        *self.entry(state) += weight;
    }
}

/// What both algorithms return for a room: statelets -> main state -> weight (Python's `defaultdict(lambda:
/// defaultdict(int))`). A statelet entry is created even when every main state under it rerolled, as in Python.
pub type FinalStates<S> = OrderedMap<Statelets, StateWeights<S>>;

/// `Room`: the remaining outcomes (a window `lo..hi` of the evaluation's sorted outcomes: equal windows are equal
/// outcome tuples), the remaining sources, and the statelets and main state the room starts from. The memo key.
type RoomKey<O, S> = (usize, usize, Vec<MultisetSource<O>>, Statelets, S);

/// The room memo: room -> final states.
type RoomCache<O, S> = FxHashMap<RoomKey<O, S>, Rc<FinalStates<S>>>;

/// One evaluation's `Dungeon`: the evaluator, the prepared inputs (the call tree of dungeonlets) plus its room
/// cache.
///
/// Python keeps two caches (by the order `next_state` sees outcomes) on a dungeon that may outlive one
/// `evaluate()` call. Within one call an attempt only ever uses one of them, and a room means the same thing to
/// the backward and forward algorithms (the final states reached from that state over those outcomes), so one
/// fresh cache per attempt computes the same. (Across calls Python's persistent cache can only change which of
/// several `==` spellings a state keeps; the harness evaluates each program with fresh evaluators.)
struct Dungeon<'a, O, E: MultisetEvaluator<O>> {
    evaluator: &'a E,
    tree: &'a CallTree<O>,
    outcomes: &'a [O],
    initial_statelets: Statelets,
    initial_state: E::State,
    cache: RoomCache<O, E::State>,
}

/// One way the sources can produce the popped outcome: `(remaining sources, counts, weight)`.
type RoomPop<O> = (Vec<MultisetSource<O>>, Vec<Count>, Quantity);

impl<'a, O: OutcomeOrder, E: MultisetEvaluator<O>> Dungeon<'a, O, E> {
    fn new(
        evaluator: &'a E,
        tree: &'a CallTree<O>,
        outcomes: &'a [O],
        initial_statelets: Statelets,
        initial_state: E::State,
    ) -> Self {
        Dungeon {
            evaluator,
            tree,
            outcomes,
            initial_statelets,
            initial_state,
            cache: FxHashMap::default(),
        }
    }

    /// `Room.pop`: pops the next outcome from the sources. Descending order pops from the top and ascending from
    /// the bottom. (The room pops the min for any `order >= 0` while the sources pop the max unless `order > 0`:
    /// with `Order.Any` they disagree, as in Python.)
    ///
    /// Returns the popped outcome, the remaining window, and every way the sources produce it.
    fn pop(
        &self,
        order: Order,
        lo: usize,
        hi: usize,
        sources: &[MultisetSource<O>],
    ) -> Result<(&'a O, usize, usize, Vec<RoomPop<O>>)> {
        let outcomes: &'a [O] = self.outcomes;
        let (outcome, lo, hi) = if order.value() < 0 {
            (&outcomes[hi - 1], lo, hi - 1)
        } else {
            (&outcomes[lo], lo + 1, hi)
        };
        let pops = sources
            .iter()
            .map(|s| s.pop(order, outcome))
            .collect::<Result<Vec<_>>>()?;
        // `itertools.product` over the sources' possibilities: the last source varies fastest.
        let mut result: Vec<RoomPop<O>> = vec![(Vec::new(), Vec::new(), Quantity::ONE)];
        for possibilities in &pops {
            let mut next = crate::memory::try_with_capacity(
                result.len().checked_mul(possibilities.len()),
                "the sources' joint pops",
            )?;
            for (sources, counts, weight) in &result {
                for (source, count, w) in possibilities {
                    let mut sources = sources.clone();
                    sources.push(source.clone());
                    let mut counts = counts.clone();
                    counts.push(count.clone());
                    next.push((sources, counts, weight * w));
                }
            }
            result = next;
        }
        Ok((outcome, lo, hi, result))
    }

    /// Internal algorithm for iterating so that `next_state` sees outcomes in backwards order (the opposite of
    /// `pop_order`). All intermediate return values are cached.
    ///
    /// Returns `{statelets: {state: weight}}` describing the probability distribution over states after the
    /// room's outcomes.
    fn evaluate_backward(
        &mut self,
        pop_order: Order,
        lo: usize,
        hi: usize,
        sources: Vec<MultisetSource<O>>,
    ) -> Result<Rc<FinalStates<E::State>>> {
        let key = (
            lo,
            hi,
            sources,
            self.initial_statelets.clone(),
            self.initial_state.clone(),
        );
        if let Some(result) = self.cache.get(&key) {
            return Ok(result.clone());
        }
        // A level per outcome (`value::recursion`): the callers' depth checks stop at Python's recursion limit, which
        // a user may raise far beyond what the stack holds.
        let _level = crate::value::recursion::level()?;
        let mut result = FinalStates::new();
        if lo == hi {
            result
                .entry(self.initial_statelets.clone())
                .add(self.initial_state.clone(), Quantity::ONE);
        } else {
            let eval_order = -pop_order;
            let (outcome, prev_lo, prev_hi, pops) = self.pop(pop_order, lo, hi, &key.2)?;
            for (prev_sources, source_counts, weight) in pops {
                crate::interrupt::poll()?;
                let prev = self.evaluate_backward(pop_order, prev_lo, prev_hi, prev_sources)?;
                for (prev_statelets, prev_main) in prev.items() {
                    let (statelets, counts) = expression::next_state(
                        self.tree,
                        prev_statelets,
                        eval_order,
                        outcome,
                        &source_counts,
                    )?;
                    let subresult = result.entry(statelets);
                    for (prev_state, prev_weight) in prev_main.items() {
                        let state = self
                            .evaluator
                            .next_state_tree(prev_state, eval_order, outcome, &counts)?;
                        if let Some(state) = state {
                            subresult.add(state, prev_weight * &weight);
                        }
                    }
                }
            }
        }
        let result = Rc::new(result);
        self.cache.insert(key, result.clone());
        Ok(result)
    }

    /// Internal algorithm for iterating in the less-preferred order: `next_state` sees outcomes in `pop_order`,
    /// starting from `statelets` and `state`. All intermediate return values are cached.
    fn evaluate_forward(
        &mut self,
        pop_order: Order,
        lo: usize,
        hi: usize,
        sources: Vec<MultisetSource<O>>,
        statelets: Statelets,
        state: E::State,
    ) -> Result<Rc<FinalStates<E::State>>> {
        let key = (lo, hi, sources, statelets, state);
        if let Some(result) = self.cache.get(&key) {
            return Ok(result.clone());
        }
        let _level = crate::value::recursion::level()?;
        let mut result = FinalStates::new();
        if lo == hi {
            result
                .entry(key.3.clone())
                .add(key.4.clone(), Quantity::ONE);
        } else {
            let (outcome, next_lo, next_hi, pops) = self.pop(pop_order, lo, hi, &key.2)?;
            for (next_sources, source_counts, weight) in pops {
                crate::interrupt::poll()?;
                let (next_statelets, counts) =
                    expression::next_state(self.tree, &key.3, pop_order, outcome, &source_counts)?;
                let next_state = self
                    .evaluator
                    .next_state_tree(&key.4, pop_order, outcome, &counts)?;
                if let Some(next_state) = next_state {
                    let fin = self.evaluate_forward(
                        pop_order,
                        next_lo,
                        next_hi,
                        next_sources,
                        next_statelets,
                        next_state,
                    )?;
                    for (final_statelets, final_main) in fin.items() {
                        let subresult = result.entry(final_statelets.clone());
                        for (final_state, final_weight) in final_main.items() {
                            subresult.add(final_state.clone(), &weight * final_weight);
                        }
                    }
                }
            }
        }
        let result = Rc::new(result);
        self.cache.insert(key, result.clone());
        Ok(result)
    }
}

/// Which algorithm an attempt uses.
#[derive(Clone, Copy)]
enum Algorithm {
    Backward,
    Forward,
}

/// `Dungeon.evaluate`: runs the backward algorithm in the preferred pop order, then on `UnsupportedOrder` either
/// the backward algorithm in the other order (if the preference was only the default) or the forward algorithm.
///
/// Every input is one source (a generator). Returns the final outcomes with their weights, in the order
/// `finalize_evaluation` produces them (`Reroll`s dropped), ready for `Die(final_outcomes, final_weights)`. If
/// some source has no probability, no outcomes (`Die([])`).
pub fn evaluate<O: OutcomeOrder, E: MultisetEvaluator<O>>(
    evaluator: &E,
    sources: &[MultisetSource<O>],
) -> Result<Vec<(E::Final, Quantity)>> {
    evaluate_with_depth_check(evaluator, sources, |_| Ok(()))
}

/// [`evaluate`], calling `check_depth(number of outcomes)` just before each attempt recurses: both algorithms
/// recurse once per outcome, which in Python hits the recursion limit near 1000 outcomes. The Python facade
/// passes a check that raises where Python would; the core itself has no such limit.
pub fn evaluate_with_depth_check<O: OutcomeOrder, E: MultisetEvaluator<O>>(
    evaluator: &E,
    sources: &[MultisetSource<O>],
    check_depth: impl Fn(usize) -> Result<()>,
) -> Result<Vec<(E::Final, Quantity)>> {
    let flats: Vec<Flat<O>> = sources
        .iter()
        .map(|_| expression::free_variable())
        .collect();
    evaluate_expressions(evaluator, &flats, sources, check_depth)
}

/// [`evaluate_with_depth_check`] for inputs that are multiset expressions: each input is a flat of dungeonlets
/// (`crate::expression`), and `sources` are the sources of all their free variables, in traversal order.
pub fn evaluate_expressions<O: OutcomeOrder, E: MultisetEvaluator<O>>(
    evaluator: &E,
    flats: &[Flat<O>],
    sources: &[MultisetSource<O>],
    check_depth: impl Fn(usize) -> Result<()>,
) -> Result<Vec<(E::Final, Quantity)>> {
    evaluate_call_tree(
        evaluator,
        &CallTree::leaf(flats.to_vec()),
        sources,
        check_depth,
    )
}

/// [`evaluate_expressions`] for a call tree (a `@multiset_function` evaluation, see [`multiset_function`]):
/// `sources` are the sources of the free variables of the whole tree, in traversal order.
pub fn evaluate_call_tree<O: OutcomeOrder, E: MultisetEvaluator<O>>(
    evaluator: &E,
    tree: &CallTree<O>,
    sources: &[MultisetSource<O>],
    check_depth: impl Fn(usize) -> Result<()>,
) -> Result<Vec<(E::Final, Quantity)>> {
    // `all(...)` stops at the first source without probability, as Python's does.
    for source in sources {
        if !source.is_resolvable()? {
            return Ok(Vec::new());
        }
    }

    let preferences = sources
        .iter()
        .map(MultisetSource::order_preference)
        .collect::<Result<Vec<_>>>()?;
    let (pop_order, pop_order_reason) = merge_order_preferences(
        std::iter::once((Order::Descending, OrderReason::Default)).chain(preferences),
    )?;

    let source_outcomes = sources
        .iter()
        .map(MultisetSource::outcomes)
        .collect::<Result<Vec<_>>>()?;
    let source_outcomes: Vec<O> = sorted_union(source_outcomes.into_iter().map(|o| o.iter()))?;
    let extra_outcomes = evaluator.extra_outcomes(&source_outcomes)?;
    let all_outcomes: Vec<O> = sorted_union([source_outcomes.iter(), extra_outcomes.iter()])?;

    let attempt = |algorithm, pop_order: Order| {
        // The order in which `next_state` sees outcomes.
        let eval_order = match algorithm {
            Algorithm::Backward => -pop_order,
            Algorithm::Forward => pop_order,
        };
        // `initial_room`: the statelets (and so the sizes of the inputs), then the main state.
        let (initial_statelets, arg_sizes) = expression::initial_state(
            tree,
            eval_order,
            &all_outcomes,
            sources.iter().map(MultisetSource::size),
        )?;
        let initial_state = evaluator.initial_state_tree(eval_order, &all_outcomes, &arg_sizes)?;
        let mut dungeon = Dungeon::new(
            evaluator,
            tree,
            &all_outcomes,
            initial_statelets.clone(),
            initial_state.clone(),
        );
        let n = all_outcomes.len();
        check_depth(n)?;
        let final_states = match algorithm {
            Algorithm::Backward => dungeon.evaluate_backward(pop_order, 0, n, sources.to_vec())?,
            Algorithm::Forward => dungeon.evaluate_forward(
                pop_order,
                0,
                n,
                sources.to_vec(),
                initial_statelets,
                initial_state,
            )?,
        };
        finalize_evaluation(
            evaluator,
            &final_states,
            eval_order,
            &all_outcomes,
            &arg_sizes,
        )
    };

    match attempt(Algorithm::Backward, pop_order) {
        Err(Error::UnsupportedOrder(backwards_unsupported)) => {
            let retry = if pop_order_reason == OrderReason::Default {
                // Flip the pop order.
                attempt(Algorithm::Backward, -pop_order)
            } else {
                // Use the alternate algorithm.
                attempt(Algorithm::Forward, pop_order)
            };
            match retry {
                Err(Error::UnsupportedOrder(forwards_unsupported)) => Err(Error::Order(format!(
                    "Neither ascending nor descending order is compatable with the evaluation.\n\
                     Preferred input order was {} with reason {}.\n\
                     Backwards evaluation could not be done because: {backwards_unsupported}\n\
                     Forwards evaluation could not be done because: {forwards_unsupported}",
                    pop_order.name(),
                    pop_order_reason.name()
                ))),
                result => result,
            }
        }
        result => result,
    }
}

/// `Quest.finalize_evaluation`: each final state's final outcome, in order (grouped by statelets).
fn finalize_evaluation<O, E: MultisetEvaluator<O>>(
    evaluator: &E,
    final_states: &FinalStates<E::State>,
    order: Order,
    outcomes: &[O],
    sizes: &CountTree<Option<Count>>,
) -> Result<Vec<(E::Final, Quantity)>> {
    let mut result = Vec::new();
    for (_, main_states) in final_states.items() {
        for (state, weight) in main_states.items() {
            match evaluator.final_outcome_tree(state, order, outcomes, sizes)? {
                Final::None => {
                    return Err(Error::Type(
                        "None is not a valid final outcome.\nThis may have been a result of not supplying \
                         any input with an outcome."
                            .into(),
                    ))
                }
                Final::Reroll => {}
                Final::Outcome(outcome) => result.push((outcome, weight.clone())),
            }
        }
    }
    Ok(result)
}
