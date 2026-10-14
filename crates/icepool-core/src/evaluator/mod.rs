//! Port of icepool `evaluator/multiset_evaluator_base.py` and `evaluator/multiset_evaluator.py`: the dynamic
//! program that evaluates multisets one outcome at a time.
//!
//! A [`MultisetEvaluator`] is a state machine: `initial_state`, then `next_state` once per outcome (in ascending or
//! descending order) with the count of that outcome from each input, then `final_outcome`. The driver
//! ([`evaluate`]) pops outcomes off the sources (pools), weighting each possible count, and memoises every
//! intermediate "room" so that equal sub-problems are solved once.
//!
//! Only generator inputs are supported so far: each argument is one source whose count is passed straight
//! through (Python's `MultisetFreeVariable` with a `MultisetGeneratorQuestlet` or a `MultisetTupleGeneratorQuestlet`,
//! whose statelets are always `None`). A multiset tuple generator (`MultiDeal`) produces a tuple of counts per
//! outcome and a tuple size ([`Count::Tuple`]). Multiset expressions (operators between inputs, the dungeonlet /
//! questlet trees) belong to a later task.

pub mod basic;
pub mod comparison;
pub mod keep;
pub mod poker;

use std::hash::Hash;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::function::sorted_union;
use crate::order::{merge_order_preferences, Order, OrderReason};
use crate::outcome::Outcome;
use crate::pool::{Count, MultisetSource};
use crate::Quantity;

/// What `final_outcome` produced: an outcome for the result `Die`, `icepool.Reroll` (dropped), or `None` (which
/// `finalize_evaluation` refuses with `TypeError`).
pub enum Final<F> {
    Outcome(F),
    Reroll,
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
        sizes: &[Option<i64>],
    ) -> Result<Self::State>;

    /// State transition function: the next state given the previous state, an outcome, and the count of that
    /// outcome produced by each input. `Ok(None)` is `icepool.Reroll`: the state is removed from consideration,
    /// effectively performing a full reroll.
    fn next_state(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &[i64],
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
        sizes: &[Option<i64>],
    ) -> Result<Final<Self::Final>>;

    /// What the driver calls: [`Self::initial_state`] with the sizes as the sources give them. An `int` size is
    /// passed on; a tuple size (a `MultiDeal` input) reaches only evaluators that override this, the others say
    /// `Unsupported`.
    fn initial_state_counts(
        &self,
        order: Order,
        outcomes: &[O],
        sizes: &[Option<Count>],
    ) -> Result<Self::State> {
        self.initial_state(order, outcomes, &int_sizes(sizes)?)
    }

    /// What the driver calls: [`Self::next_state`] with the counts as the sources give them (see
    /// [`Self::initial_state_counts`]).
    fn next_state_counts(
        &self,
        state: &Self::State,
        order: Order,
        outcome: &O,
        counts: &[Count],
    ) -> Result<Option<Self::State>> {
        let counts = counts
            .iter()
            .map(|c| match c {
                Count::Int(c) => Ok(*c),
                Count::Tuple(_) => Err(tuple_unsupported()),
            })
            .collect::<Result<Vec<i64>>>()?;
        self.next_state(state, order, outcome, &counts)
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
        self.final_outcome(final_state, order, outcomes, &int_sizes(sizes)?)
    }
}

fn tuple_unsupported() -> Error {
    Error::Unsupported("a multiset tuple input (MultiDeal) to this evaluator".into())
}

fn int_sizes(sizes: &[Option<Count>]) -> Result<Vec<Option<i64>>> {
    sizes
        .iter()
        .map(|s| match s {
            None => Ok(None),
            Some(Count::Int(s)) => Ok(Some(*s)),
            Some(Count::Tuple(_)) => Err(tuple_unsupported()),
        })
        .collect()
}

/// A Python `defaultdict(int)` from state to weight: insertion order, and a state equal to an earlier one adds to
/// it, keeping the earlier key.
#[derive(Debug)]
pub struct StateWeights<S> {
    index: FxHashMap<S, usize>,
    items: Vec<(S, Quantity)>,
}

impl<S: Clone + Eq + Hash> StateWeights<S> {
    fn new() -> Self {
        StateWeights {
            index: FxHashMap::default(),
            items: Vec::new(),
        }
    }

    fn add(&mut self, state: S, weight: Quantity) {
        match self.index.get(&state) {
            Some(&i) => self.items[i].1 += weight,
            None => {
                self.index.insert(state.clone(), self.items.len());
                self.items.push((state, weight));
            }
        }
    }

    pub fn items(&self) -> &[(S, Quantity)] {
        &self.items
    }
}

/// `Room`: the remaining outcomes (a window `lo..hi` of the evaluation's sorted outcomes: equal windows are equal
/// outcome tuples), the remaining sources, and the state the room starts from. The memo key.
type RoomKey<O, S> = (usize, usize, Vec<MultisetSource<O>>, S);

/// The room memo: room -> final state weights.
type RoomCache<O, S> = FxHashMap<RoomKey<O, S>, Rc<StateWeights<S>>>;

/// One evaluation's `Dungeon`: the evaluator plus its room cache.
///
/// Python keeps two caches (by the order `next_state` sees outcomes) on a dungeon that may outlive one
/// `evaluate()` call. Within one call an attempt only ever uses one of them, and a room means the same thing to
/// the backward and forward algorithms (the final states reached from that state over those outcomes), so one
/// fresh cache per attempt computes the same. (Across calls Python's persistent cache can only change which of
/// several `==` spellings a state keeps; the harness evaluates each program with fresh evaluators.)
struct Dungeon<'a, O, E: MultisetEvaluator<O>> {
    evaluator: &'a E,
    outcomes: &'a [O],
    initial_state: E::State,
    cache: RoomCache<O, E::State>,
}

/// One way the sources can produce the popped outcome: `(remaining sources, counts, weight)`.
type RoomPop<O> = (Vec<MultisetSource<O>>, Vec<Count>, Quantity);

impl<'a, O: Outcome, E: MultisetEvaluator<O>> Dungeon<'a, O, E> {
    fn new(evaluator: &'a E, outcomes: &'a [O], initial_state: E::State) -> Self {
        Dungeon {
            evaluator,
            outcomes,
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
            let mut next = Vec::with_capacity(result.len() * possibilities.len());
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
    /// Returns `{state: weight}` describing the probability distribution over states after the room's outcomes.
    fn evaluate_backward(
        &mut self,
        pop_order: Order,
        lo: usize,
        hi: usize,
        sources: Vec<MultisetSource<O>>,
    ) -> Result<Rc<StateWeights<E::State>>> {
        let key = (lo, hi, sources, self.initial_state.clone());
        if let Some(result) = self.cache.get(&key) {
            return Ok(result.clone());
        }
        let mut result = StateWeights::new();
        if lo == hi {
            result.add(self.initial_state.clone(), Quantity::ONE);
        } else {
            let eval_order = -pop_order;
            let (outcome, prev_lo, prev_hi, pops) = self.pop(pop_order, lo, hi, &key.2)?;
            for (prev_sources, counts, weight) in pops {
                let prev = self.evaluate_backward(pop_order, prev_lo, prev_hi, prev_sources)?;
                for (prev_state, prev_weight) in prev.items() {
                    let state = self
                        .evaluator
                        .next_state_counts(prev_state, eval_order, outcome, &counts)?;
                    if let Some(state) = state {
                        result.add(state, prev_weight * &weight);
                    }
                }
            }
        }
        let result = Rc::new(result);
        self.cache.insert(key, result.clone());
        Ok(result)
    }

    /// Internal algorithm for iterating in the less-preferred order: `next_state` sees outcomes in `pop_order`,
    /// starting from `state`. All intermediate return values are cached.
    fn evaluate_forward(
        &mut self,
        pop_order: Order,
        lo: usize,
        hi: usize,
        sources: Vec<MultisetSource<O>>,
        state: E::State,
    ) -> Result<Rc<StateWeights<E::State>>> {
        let key = (lo, hi, sources, state);
        if let Some(result) = self.cache.get(&key) {
            return Ok(result.clone());
        }
        let mut result = StateWeights::new();
        if lo == hi {
            result.add(key.3.clone(), Quantity::ONE);
        } else {
            let (outcome, next_lo, next_hi, pops) = self.pop(pop_order, lo, hi, &key.2)?;
            for (next_sources, counts, weight) in pops {
                let next_state = self
                    .evaluator
                    .next_state_counts(&key.3, pop_order, outcome, &counts)?;
                if let Some(next_state) = next_state {
                    let fin = self.evaluate_forward(
                        pop_order,
                        next_lo,
                        next_hi,
                        next_sources,
                        next_state,
                    )?;
                    for (final_state, final_weight) in fin.items() {
                        result.add(final_state.clone(), &weight * final_weight);
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
/// Returns the final outcomes with their weights, in the order `finalize_evaluation` produces them (`Reroll`s
/// dropped), ready for `Die(final_outcomes, final_weights)`. If some source has no probability, no outcomes
/// (`Die([])`).
pub fn evaluate<O: Outcome, E: MultisetEvaluator<O>>(
    evaluator: &E,
    sources: &[MultisetSource<O>],
) -> Result<Vec<(E::Final, Quantity)>> {
    evaluate_with_depth_check(evaluator, sources, |_| Ok(()))
}

/// [`evaluate`], calling `check_depth(number of outcomes)` just before each attempt recurses: both algorithms
/// recurse once per outcome, which in Python hits the recursion limit near 1000 outcomes. The Python facade
/// passes a check that raises where Python would; the core itself has no such limit.
pub fn evaluate_with_depth_check<O: Outcome, E: MultisetEvaluator<O>>(
    evaluator: &E,
    sources: &[MultisetSource<O>],
    check_depth: impl Fn(usize) -> Result<()>,
) -> Result<Vec<(E::Final, Quantity)>> {
    if !sources.iter().all(MultisetSource::is_resolvable) {
        return Ok(Vec::new());
    }

    let (pop_order, pop_order_reason) = merge_order_preferences(
        std::iter::once((Order::Descending, OrderReason::Default))
            .chain(sources.iter().map(MultisetSource::order_preference)),
    )?;

    let source_outcomes: Vec<O> = sorted_union(sources.iter().map(|s| s.outcomes().iter()))?;
    let extra_outcomes = evaluator.extra_outcomes(&source_outcomes)?;
    let all_outcomes: Vec<O> = sorted_union([source_outcomes.iter(), extra_outcomes.iter()])?;
    let sizes = sources
        .iter()
        .map(MultisetSource::size)
        .collect::<Result<Vec<_>>>()?;

    let attempt = |algorithm, pop_order: Order| {
        // The order in which `next_state` sees outcomes.
        let eval_order = match algorithm {
            Algorithm::Backward => -pop_order,
            Algorithm::Forward => pop_order,
        };
        let initial_state = evaluator.initial_state_counts(eval_order, &all_outcomes, &sizes)?;
        let mut dungeon = Dungeon::new(evaluator, &all_outcomes, initial_state.clone());
        let n = all_outcomes.len();
        check_depth(n)?;
        let final_states = match algorithm {
            Algorithm::Backward => dungeon.evaluate_backward(pop_order, 0, n, sources.to_vec())?,
            Algorithm::Forward => {
                dungeon.evaluate_forward(pop_order, 0, n, sources.to_vec(), initial_state)?
            }
        };
        finalize_evaluation(evaluator, &final_states, eval_order, &all_outcomes, &sizes)
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

/// `Quest.finalize_evaluation`: each final state's final outcome, in order.
fn finalize_evaluation<O, E: MultisetEvaluator<O>>(
    evaluator: &E,
    final_states: &StateWeights<E::State>,
    order: Order,
    outcomes: &[O],
    sizes: &[Option<Count>],
) -> Result<Vec<(E::Final, Quantity)>> {
    let mut result = Vec::new();
    for (state, weight) in final_states.items() {
        match evaluator.final_outcome_counts(state, order, outcomes, sizes)? {
            Final::None => {
                return Err(Error::Type(
                    "None is not a valid final outcome.\nThis may have been a result of not supplying any \
                     input with an outcome."
                        .into(),
                ))
            }
            Final::Reroll => {}
            Final::Outcome(outcome) => result.push((outcome, weight.clone())),
        }
    }
    Ok(result)
}
