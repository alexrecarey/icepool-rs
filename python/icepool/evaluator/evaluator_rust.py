"""icepool-rs: the evaluation driver on the Rust core (`icepool_rs.evaluate_builtin` / `evaluate_host`).

Not part of upstream icepool. `MultisetEvaluatorBase.evaluate` keeps its argument conversion, `_prepare`, the
`_cache` of dungeons and the final `Die(final_data)`; for each prepared evaluation whose inputs are all generators
backed by core sources (a `Pool`, a `Deal` or a `MultiDeal`, or a sequence / mapping converted to a pool),
`Dungeon.evaluate` runs in Rust:

* A built-in evaluator the core has (`builtin_spec`) runs entirely in Rust.
* Any other `MultisetEvaluator`, user subclasses included, runs on the core driver with its `next_state`,
  `initial_state`, `extra_outcomes` and `final_outcome` called back in Python. So does any evaluation with a
  `MultiDeal` input, whose counts and size are tuples (the core's built-ins take `int` counts).

Inputs that are multiset expressions (operators between multisets, `unique`, `keep` of an expression, a hand of a
`MultiDeal` such as `deal[0]`...), other generators (mixtures...) and `@multiset_function`s are not ported yet (Phase
S4): their evaluation is still upstream's `Dungeon.evaluate`.

Rooms are cached per call in the core (ANALYSIS.md decision 5); a dungeon's `ascending_cache` /
`descending_cache` are not filled by these evaluations.
"""

__docformat__ = 'google'

import sys

import icepool
from icepool.expression.multiset_expression_base import MultisetFreeVariable
from icepool.generator.multiset_generator import MultisetGeneratorQuestlet
from icepool.generator.multiset_tuple_generator import MultisetTupleGeneratorQuestlet
from icepool.order import Order

from typing import Any, Mapping

# Python frames that upstream's driver adds around its once-per-outcome recursion: `evaluate`,
# `Dungeon.evaluate`, the done room's `evaluate_backward`, and the pop / next_state calls of the deepest room.
# Calibrated against the oracle at the boundary (see NOTES.md, S1e).
_DRIVER_FRAMES = 8


def _depth_budget() -> int:
    """How many outcomes the evaluation can recurse over before Python's recursion limit."""
    depth = 0
    frame: Any = sys._getframe(1)
    while frame is not None:
        depth += 1
        frame = frame.f_back
    return sys.getrecursionlimit() - depth - _DRIVER_FRAMES


def is_rust_evaluable(dungeon, quest, sources) -> bool:
    """Whether `dungeon.evaluate(quest, sources, kwargs)` can run on the core driver.

    It can when the evaluator is a plain `MultisetEvaluator` (no `@multiset_function` calls) and every input is a
    generator (one free variable, one generator questlet) whose source is the core's: a pool or deal source with a
    `MultisetGeneratorQuestlet`, a multi-deal source with a `MultisetTupleGeneratorQuestlet`.
    """
    from icepool.evaluator.multiset_evaluator import MultisetEvaluatorDungeon, MultisetEvaluatorQuest
    if type(dungeon) is not MultisetEvaluatorDungeon or type(
            quest) is not MultisetEvaluatorQuest:
        return False
    if len(dungeon.dungeonlet_flats) != len(sources) or len(
            quest.questlet_flats) != len(sources):
        return False
    def generator(q, source) -> bool:
        if type(q) is MultisetGeneratorQuestlet:
            return isinstance(source, (icepool.generator.pool.PoolSource,
                                       icepool.generator.deal.DealSource))
        if type(q) is MultisetTupleGeneratorQuestlet:
            return isinstance(source,
                              icepool.generator.multi_deal.MultiDealSource)
        return False

    return all(
        len(d) == 1 and type(d[0]) is MultisetFreeVariable and len(q) == 1
        and generator(q[0], source) and '_rs' in source.__dict__
        for d, q, source in zip(dungeon.dungeonlet_flats,
                                quest.questlet_flats, sources))


def _exact_int(x) -> bool:
    return type(x) is int and -2**63 <= x < 2**63


def builtin_spec(evaluator, inputs: int,
                 kwargs: Mapping[str, Any]) -> tuple[str, tuple] | None:
    """The core's name and parameters for a built-in evaluator, or `None` to call back the evaluator's methods.

    Only an evaluator whose type is exactly the built-in class qualifies (a subclass may override any method),
    given the number of inputs its `next_state` takes and only the keyword arguments it reads. Anything else
    (e.g. two inputs to `sum_evaluator`, which raises `TypeError` in `next_state`) goes through the callbacks,
    which raise what Python raises.
    """
    ev = icepool.evaluator
    t = type(evaluator)
    if inputs == 1:
        if not kwargs:
            if t is ev.SumEvaluator and evaluator._next_state_key == (
                    ev.SumEvaluator, ):
                # `SumEvaluator()` without a map.
                return 'sum', ()
            simple = {
                ev.SizeEvaluator: 'size',
                ev.ProductOfCountsEvaluator: 'product_of_counts',
                ev.HighestOutcomeAndCountEvaluator:
                'highest_outcome_and_count',
                ev.LargestCountEvaluator: 'largest_count',
                ev.LargestCountAndOutcomeEvaluator:
                'largest_count_and_outcome',
                ev.LargestStraightEvaluator: 'largest_straight',
                ev.AllStraightsEvaluator: 'all_straights',
            }
            if t in simple:
                return simple[t], ()
            if t is ev.AllCountsEvaluator:
                return 'all_counts', (evaluator._filter, )
            if t is ev.LargestStraightAndOutcomeEvaluator:
                return 'largest_straight_and_outcome', (
                    evaluator._prioritize_highest, )
        if t is ev.EmptyEvaluator and set(kwargs) <= {'any'}:
            any = kwargs.get('any', False)
            if type(any) is bool:
                return 'empty', (any, )
        if t is ev.ExpandEvaluator and set(kwargs) <= {'order'}:
            order = kwargs.get('order', Order.Ascending)
            if type(order) in (Order, int, bool):
                # `reverse = order != Order.Ascending` in `final_outcome`.
                return 'expand', (not (order != Order.Ascending), )
        if t is ev.KeepEvaluator and set(kwargs) == {'index'}:
            index = kwargs['index']
            if index is None or _exact_int(index):
                return 'keep', (index, )
    if inputs == 2 and not kwargs:
        if t is ev.CountSubsetEvaluator:
            return 'count_subset', (evaluator._empty_divisor, )
        comparisons = {
            ev.IsProperSubsetEvaluator: 'is_proper_subset',
            ev.IsSubsetEvaluator: 'is_subset',
            ev.IsProperSupersetEvaluator: 'is_proper_superset',
            ev.IsSupersetEvaluator: 'is_superset',
            ev.IsEqualSetEvaluator: 'is_equal_set',
            ev.IsNotEqualSetEvaluator: 'is_not_equal_set',
            ev.IsDisjointSetEvaluator: 'is_disjoint_set',
        }
        if t in comparisons:
            return comparisons[t], ()
    return None


def evaluate(evaluator, dungeon, quest, sources,
             kwargs: Mapping[str, Any]) -> 'icepool.Die':
    """`dungeon.evaluate(quest, sources, kwargs)` on the core driver (`is_rust_evaluable` must hold)."""
    rust = icepool.backend.RUST
    rs_sources = [source._rs for source in sources]
    budget = _depth_budget()
    spec = None if any(
        isinstance(source, icepool.generator.multi_deal.MultiDealSource)
        for source in sources) else builtin_spec(evaluator, len(sources),
                                                 kwargs)
    if spec is not None:
        name, params = spec
        final_outcomes, final_weights = rust.evaluate_builtin(
            name, list(params), rs_sources, budget)
    else:
        final_outcomes, final_weights = rust.evaluate_host(
            dungeon.next_state_main, quest.initial_state_main,
            quest.extra_outcomes, quest.final_outcome,
            dict(kwargs) or None, rs_sources, budget)
    # `Quest.finalize_evaluation`'s result.
    return icepool.Die(final_outcomes, final_weights)
