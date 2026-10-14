"""icepool-rs: the evaluation driver on the Rust core (`icepool_rs.evaluate_builtin` / `evaluate_host`).

Not part of upstream icepool. `MultisetEvaluatorBase.evaluate` keeps its argument conversion, `_prepare`, the
`_cache` of dungeons and the final `Die(final_data)`; for each prepared evaluation whose inputs are generators
backed by core sources (a `Pool`, a `Deal`, a `MultiDeal` or a `CompoundKeepGenerator`, or a sequence / mapping
converted to a pool) and the multiset operators of operator/*.py over them, `Dungeon.evaluate` runs in Rust:

* A built-in evaluator the core has (`builtin_spec`) runs entirely in Rust.
* Any other `MultisetEvaluator`, user subclasses included, runs on the core driver with its `next_state`,
  `initial_state`, `extra_outcomes` and `final_outcome` called back in Python. So does any evaluation with a
  `MultiDeal` input, whose counts and size are tuples (the core's built-ins take `int` counts).

Each prepared input (its flat of dungeonlets) is described to the core as a list of nodes (`rust_flats`): `None` for
a free variable, `(spec, child_indexes)` for an operator, the spec naming the operator class with the attributes
its `_next_state` / `_initial_state` read. The operators then run in the core (crates/icepool-py/src/
expression.rs), `map_counts` functions and `keep_outcomes` callables as host callbacks.

Other expression kinds (a hand of a `MultiDeal` such as `deal[0]`, mixtures, `weightless()`) and
`@multiset_function`s are not ported yet (S4d): their evaluation is still upstream's `Dungeon.evaluate`.

Rooms are cached per call in the core (ANALYSIS.md decision 5); a dungeon's `ascending_cache` /
`descending_cache` are not filled by these evaluations.
"""

__docformat__ = 'google'

import sys

import icepool
from icepool.expression.multiset_expression_base import BodyDungeonlet, BodyQuestlet, MultisetFreeVariable
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


def _binary_operators():
    op = icepool.operator
    return {
        op.MultisetIntersection: 'intersection',
        op.MultisetDifferenceDropNegative: 'difference_drop_negative',
        op.MultisetDifferenceKeepNegative: 'difference_keep_negative',
        op.MultisetUnion: 'union',
        op.MultisetAdditiveUnion: 'additive_union',
        op.MultisetSymmetricDifference: 'symmetric_difference',
    }


def _operator_spec(operator) -> tuple | None:
    """The core's description of an operator node: its class (exactly; a subclass may override anything) and the
    attributes its `_next_state` / `_initial_state` read. `None` for an operator the core does not have."""
    op = icepool.operator
    t = type(operator)
    binary = _binary_operators()
    if t in binary:
        return 'binary', binary[t]
    if t is op.MultisetMapCounts:
        return 'map_counts', operator._function
    simple = {
        op.MultisetMultiplyCounts: 'multiply_counts',
        op.MultisetFloordivCounts: 'floordiv_counts',
        op.MultisetModuloCounts: 'modulo_counts',
        op.MultisetUnique: 'unique',
    }
    if t in simple:
        return simple[t], operator._constant
    if t is op.MultisetKeepCounts:
        return 'keep_counts', operator._comparison, operator._constant
    if t is op.MultisetFilterOutcomes:
        func = operator._func
        if getattr(func, '__qualname__', None) == (
                'MultisetFilterOutcomes.__init__.<locals>.function'):
            # A collection of outcomes, kept by the constructor as `target_set` in this closure.
            return 'filter_set', func.__closure__[0].cell_contents, operator._invert
        return 'filter_function', func, operator._invert
    if t is op.MultisetFilterOutcomesBinary:
        return 'filter_binary', operator._invert
    if t is op.MultisetKeep:
        return 'keep', operator._index
    if t is op.MultisetSortPair:
        return ('sort_pair', operator._comparison, operator._sort_order,
                operator._extra)
    if t is op.MultisetSortPairWhile:
        return ('sort_pair_while', operator._keep, operator._comparison,
                operator._sort_order, operator._extra)
    if t is op.MultisetMaxPairLate:
        return ('max_pair_late', operator._order, operator._pair_equal,
                operator._keep)
    if t is op.MultisetMaxPairEarly:
        return ('max_pair_early', operator._order, operator._pair_equal,
                operator._keep)
    if t is op.MultisetVersus:
        return 'versus', operator._lexi_tuple, operator._order
    if t is op.MultisetForceOrder:
        return 'force_order', operator._force_order
    return None


def _core_source(q, source) -> bool:
    """Whether a free variable's source is the core's: a pool, deal or compound keep source with a
    `MultisetGeneratorQuestlet`, a multi-deal source with a `MultisetTupleGeneratorQuestlet`."""
    if '_rs' not in source.__dict__:
        return False
    if type(q) is MultisetGeneratorQuestlet:
        return isinstance(
            source,
            (icepool.generator.pool.PoolSource,
             icepool.generator.deal.DealSource,
             icepool.generator.compound_keep.CompoundKeepSource))
    if type(q) is MultisetTupleGeneratorQuestlet:
        return isinstance(source, icepool.generator.multi_deal.MultiDealSource)
    return False


def rust_flats(dungeon, quest, sources) -> list | None:
    """The prepared inputs as the core takes them, if `dungeon.evaluate(quest, sources, kwargs)` can run on the core
    driver; else `None`.

    It can when the evaluator is a plain `MultisetEvaluator` (no `@multiset_function` calls) and every node of every
    input is either a free variable whose source is the core's or a `BodyDungeonlet` / `BodyQuestlet` pair of an
    operator the core has (`_operator_spec`).
    """
    from icepool.evaluator.multiset_evaluator import MultisetEvaluatorDungeon, MultisetEvaluatorQuest
    if type(dungeon) is not MultisetEvaluatorDungeon or type(
            quest) is not MultisetEvaluatorQuest:
        return None
    if len(dungeon.dungeonlet_flats) != len(quest.questlet_flats):
        return None
    source_iter = iter(sources)
    flats = []
    for dungeonlets, questlets in zip(dungeon.dungeonlet_flats,
                                      quest.questlet_flats):
        if len(dungeonlets) != len(questlets):
            return None
        nodes: list = []
        for d, q in zip(dungeonlets, questlets):
            if type(d) is MultisetFreeVariable:
                source = next(source_iter, None)
                if source is None or not _core_source(q, source):
                    return None
                nodes.append(None)
                continue
            if type(d) is not BodyDungeonlet or type(q) is not BodyQuestlet:
                return None
            # The questlet is this expression's operator. The dungeonlet may be an earlier, equal one's: a dungeon
            # found in the evaluator's `_cache` is equal by `_dungeonlet_key`, which leaves out what only
            # `_initial_state` reads (a `MultisetKeep`'s index, ...). So the spec is the questlet's operator, and
            # the dungeonlet's must agree on what `_next_state` reads.
            operator = getattr(q.initial_state, '__self__', None)
            d_operator = getattr(d.next_state, '__self__', None)
            spec = _operator_spec(operator)
            if (spec is None or type(d_operator) is not type(operator)
                    or q.initial_state != operator._initial_state
                    or d.next_state != d_operator._next_state
                    or d_operator._dungeonlet_key != operator._dungeonlet_key
                    or d.child_indexes != q.child_indexes):
                return None
            nodes.append((spec, d.child_indexes))
        flats.append(nodes)
    if next(source_iter, None) is not None:
        return None
    return flats


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


def evaluate(evaluator, dungeon, quest, sources, kwargs: Mapping[str, Any],
             flats: list) -> 'icepool.Die':
    """`dungeon.evaluate(quest, sources, kwargs)` on the core driver, given `rust_flats(dungeon, quest, sources)`."""
    rust = icepool.backend.RUST
    rs_sources = [source._rs for source in sources]
    budget = _depth_budget()
    spec = None if any(
        isinstance(source, icepool.generator.multi_deal.MultiDealSource)
        for source in sources) else builtin_spec(evaluator, len(flats),
                                                 kwargs)
    if spec is not None:
        name, params = spec
        final_outcomes, final_weights = rust.evaluate_builtin(
            name, list(params), rs_sources, budget, flats)
    else:
        final_outcomes, final_weights = rust.evaluate_host(
            dungeon.next_state_main, quest.initial_state_main,
            quest.extra_outcomes, quest.final_outcome,
            dict(kwargs) or None, rs_sources, budget, flats)
    # `Quest.finalize_evaluation`'s result.
    return icepool.Die(final_outcomes, final_weights)
