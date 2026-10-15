"""icepool-rs: the evaluation driver on the Rust core (`icepool_rs.evaluate_builtin` / `evaluate_host`).

Not part of upstream icepool. `MultisetEvaluatorBase.evaluate` keeps its argument conversion, `_prepare`, the
`_cache` of dungeons and the final `Die(final_data)`; every prepared evaluation then runs on the core driver in
Rust instead of `Dungeon.evaluate`. Its sources are the core's (a `Pool`, `Deal`, `MultiDeal`, `CompoundKeepGenerator`
or weightless generator's, or a sequence / mapping converted to a pool), or host sources: any other `MultisetSource`
(a user's `MultisetGenerator` subclass), whose `pop` / `size` / `outcomes` / `order_preference` / `is_resolvable`
the core calls back. Over them:

* A built-in evaluator the core has (`builtin_spec`) runs entirely in Rust.
* Any other `MultisetEvaluator`, user subclasses included, runs on the core driver with its `next_state`,
  `initial_state`, `extra_outcomes` and `final_outcome` called back in Python. So does any evaluation with a
  tuple source (a `MultiDeal`'s), whose counts and size are tuples (the core's built-ins take `int` counts).

Each prepared input (its flat of dungeonlets) is described to the core as a list of nodes (`rust_flats`): `None` for
a free variable, `('param', arg_index, star_index)` for a `@multiset_function` parameter, `(spec, child_indexes)`
for an operator or a `MultisetTupleIndex` / `MultisetTupleSlice` (a hand of a `MultiDeal`), the spec naming the
class with the attributes its `_next_state` / `_initial_state` read. The operators then run in the core
(crates/icepool-py/src/expression.rs), `map_counts` functions and `keep_outcomes` callables as host callbacks. An
operator the core has no spec for (a user's `MultisetOperator` subclass, or a subclass of a built-in one) is
`('host', next_state, initial_state)`: its own `_next_state` / `_initial_state`, called back.

A `@multiset_function` is traced by upstream's code (`MultisetFunctionEvaluator._prepare` calls the function with
parameters); the dungeon / quest it prepares is described as a call tree (`rust_call_tree`) and run by
`icepool_rs.evaluate_tree`: the core drives the expressions of every call and the main states of the function, its
joint results and the evaluators called in it (built-ins natively, others and constant results as host callbacks).
A `MultisetMixture` needs nothing: its `_prepare` yields one ordinary evaluation per inner expression.

What the core cannot take is an evaluation prepared into dungeons, quests or dungeonlets of other classes than
these (a user's subclass of `MultisetEvaluatorBase`, `Dungeon` or `Dungeonlet` with its own `_prepare`, internal
API): `MultisetEvaluatorBase.evaluate` raises `NotImplementedError` for it rather than run upstream's Python driver.

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


def _tuple_item_spec(operator) -> tuple | None:
    """The spec of a `MultisetTupleIndex` / `MultisetTupleSlice` node: its index (an int or a slice)."""
    from icepool.expression.multiset_tuple_expression import MultisetTupleIndex, MultisetTupleSlice
    if type(operator) in (MultisetTupleIndex, MultisetTupleSlice):
        return 'tuple_item', operator._index
    return None


def _core_source(q, source) -> bool:
    """Whether a free variable can be driven by the core: one left by a generator (`MultisetGeneratorQuestlet` /
    `MultisetTupleGeneratorQuestlet`). Its source is either the core's (a pool, deal, multi-deal, compound keep or
    weightless source, holding `_rs`) or any other, which the core calls back as a host source (a user's
    `MultisetGenerator` subclass; crates/icepool-py/src/host_source.rs)."""
    return type(q) in (MultisetGeneratorQuestlet, MultisetTupleGeneratorQuestlet)


def _rust_sources(sources) -> list:
    """The sources as the core takes them: the core's own (`_rs`), or the Python object for a host source."""
    from icepool.generator.compound_keep import _rust_source
    return [_rust_source(source) for source in sources]


def _tuple_source(source) -> bool:
    """Whether a source produces tuple counts (a `MultisetTupleSource`: a `MultiDeal`'s, or a user's)."""
    from icepool.generator.multiset_tuple_generator import MultisetTupleSource
    return isinstance(source, MultisetTupleSource)


def rust_flats(dungeon, quest, sources) -> list | None:
    """The prepared inputs as the core takes them, if `dungeon.evaluate(quest, sources, kwargs)` is a plain
    `MultisetEvaluator`'s (no `@multiset_function` calls) that can run on the core driver; else `None`.

    It can when every node of every input is either a free variable whose source is the core's or a `BodyDungeonlet`
    / `BodyQuestlet` pair of an operator the core has (`_operator_spec`).
    """
    from icepool.evaluator.multiset_evaluator import MultisetEvaluatorDungeon, MultisetEvaluatorQuest
    if type(dungeon) is not MultisetEvaluatorDungeon or type(
            quest) is not MultisetEvaluatorQuest:
        return None
    source_iter = iter(sources)
    flats = _flats(dungeon, quest, source_iter)
    if flats is None or next(source_iter, None) is not None:
        return None
    return flats


def _flats(dungeon, quest, source_iter) -> list | None:
    """The flats of one call (`dungeon.dungeonlet_flats` with `quest.questlet_flats`) as the core takes them,
    consuming their sources from `source_iter`; `None` if some node is not the core's."""
    from icepool.expression.multiset_parameter import MultisetParameterDungeonlet, MultisetParameterQuestlet
    if len(dungeon.dungeonlet_flats) != len(quest.questlet_flats):
        return None
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
            if type(d) is MultisetParameterDungeonlet:
                if (type(q) is not MultisetParameterQuestlet
                        or (d.arg_index, d.star_index) != (q.arg_index, q.star_index)
                        or type(d.arg_index) is not int
                        or type(d.star_index) not in (int, type(None))):
                    return None
                nodes.append(('param', d.arg_index, d.star_index))
                continue
            if type(d) is not BodyDungeonlet or type(q) is not BodyQuestlet:
                return None
            # Any other operator (a user's subclass, or a dungeonlet that does not agree with its questlet's
            # operator, below) runs on the core with its own methods called back.
            host = ('host', d.next_state, q.initial_state)
            # The questlet is this expression's operator. The dungeonlet may be an earlier, equal one's: a dungeon
            # found in the evaluator's `_cache` is equal by `_dungeonlet_key`, which leaves out what only
            # `_initial_state` reads (a `MultisetKeep`'s index, ...). So the spec is the questlet's operator, and
            # the dungeonlet's must agree on what `_next_state` reads.
            operator = getattr(q.initial_state, '__self__', None)
            d_operator = getattr(d.next_state, '__self__', None)
            spec = _tuple_item_spec(operator)
            if spec is not None:
                # `hash_key` is all `_next_state` / `_initial_state` read: the index.
                if d.child_indexes != q.child_indexes:
                    return None
                if (type(d_operator) is not type(operator)
                        or d_operator.hash_key != operator.hash_key
                        or q.initial_state != operator._initial_state
                        or d.next_state != d_operator._next_state):
                    spec = host
                nodes.append((spec, d.child_indexes))
                continue
            if d.child_indexes != q.child_indexes:
                return None
            spec = _operator_spec(operator)
            if (spec is None or type(d_operator) is not type(operator)
                    or q.initial_state != operator._initial_state
                    or d.next_state != d_operator._next_state
                    or d_operator._dungeonlet_key != operator._dungeonlet_key):
                spec = host
            nodes.append((spec, d.child_indexes))
        flats.append(nodes)
    return flats


def _is_bound(method, name: str) -> bool:
    """Whether `method` is the bound method `name` of some object (not an attribute replaced by hand)."""
    return getattr(getattr(method, '__self__', None), name, None) == method


def _call_tree(dungeon, quest, source_iter, kwargs, builtins: bool):
    """`(kind, flats, ...)`: the description of one call of a prepared evaluation, as `icepool_rs.evaluate_tree`
    takes it, or `None` if some part is not the core's.

    * `('function', flats, inner)`: `MultisetFunctionDungeon` / `Quest` (one result).
    * `('joint', flats, [inner, ...])`: `MultisetFunctionJointDungeon` / `Quest` (a tuple of results).
    * `('builtin', flats, name, params)`: a built-in evaluator the core has (`builtin_spec`).
    * `('host', flats, next_state, initial_state, extra_outcomes, final_outcome, kwargs)`: any other evaluator
      called in the function, or a constant result (`ConstantDungeon` / `Quest`), called back in Python.

    `kwargs` are the ones its quest's methods get: the evaluation's own at the top, the function's `inner_kwargs`
    inside it.
    """
    from icepool.evaluator.multiset_evaluator import MultisetEvaluatorDungeon, MultisetEvaluatorQuest
    from icepool.evaluator import multiset_function as mf
    flats = _flats(dungeon, quest, source_iter)
    if flats is None:
        return None
    t, q = type(dungeon), type(quest)
    if t is mf.MultisetFunctionDungeon and q is mf.MultisetFunctionQuest:
        inner = _call_tree(dungeon.inner_dungeon, quest.inner_quest,
                           source_iter, quest.inner_kwargs, builtins)
        if inner is None:
            return None
        return 'function', flats, inner
    if t is mf.MultisetFunctionJointDungeon and q is mf.MultisetFunctionJointQuest:
        if not (len(dungeon.inner_dungeons) == len(quest.inner_quests) ==
                len(quest.inner_kwargses)):
            return None
        inners = []
        for d, iq, inner_kwargs in zip(dungeon.inner_dungeons,
                                       quest.inner_quests,
                                       quest.inner_kwargses):
            inner = _call_tree(d, iq, source_iter, inner_kwargs, builtins)
            if inner is None:
                return None
            inners.append(inner)
        return 'joint', flats, inners
    if dungeon.calls or quest.calls:
        return None
    if t is MultisetEvaluatorDungeon and q is MultisetEvaluatorQuest:
        evaluator = getattr(quest.initial_state_main, '__self__', None)
        # The dungeon may be an equal one from the evaluator's cache: the same `next_state` by its key.
        if (builtins and evaluator is not None and type(
                getattr(dungeon.next_state_main, '__self__', None)) is type(evaluator)
                and _is_bound(quest.initial_state_main, 'initial_state')):
            spec = builtin_spec(evaluator, len(flats), kwargs)
            if spec is not None:
                return ('builtin', flats) + spec
    elif not (t is mf.ConstantDungeon and q is mf.ConstantQuest):
        return None
    return ('host', flats, dungeon.next_state_main, quest.initial_state_main,
            quest.extra_outcomes, quest.final_outcome, dict(kwargs) or None)


def rust_call_tree(dungeon, quest, sources, kwargs):
    """The description of a `@multiset_function` evaluation (see `_call_tree`), if it can run on the core; else
    `None`. With a tuple source every evaluator is called back (built-ins would get tuple counts)."""
    from icepool.evaluator import multiset_function as mf
    if type(dungeon) not in (mf.MultisetFunctionDungeon,
                             mf.MultisetFunctionJointDungeon):
        return None
    builtins = not any(_tuple_source(source) for source in sources)
    source_iter = iter(sources)
    tree = _call_tree(dungeon, quest, source_iter, kwargs, builtins)
    if tree is None or next(source_iter, None) is not None:
        return None
    return tree


def evaluate_tree(tree, sources) -> 'icepool.Die':
    """`dungeon.evaluate(quest, sources, kwargs)` of a `@multiset_function` on the core, given its
    `rust_call_tree`."""
    final_outcomes, final_weights = icepool.backend.RUST.evaluate_tree(
        tree, _rust_sources(sources), _depth_budget())
    return icepool.Die(final_outcomes, final_weights)


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
    rs_sources = _rust_sources(sources)
    budget = _depth_budget()
    spec = None if any(_tuple_source(source)
                       for source in sources) else builtin_spec(
                           evaluator, len(flats), kwargs)
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
