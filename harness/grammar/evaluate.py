"""Multiset evaluation (evaluator/multiset_evaluator_base.py, basic.py, poker.py, keep.py, comparison.py): the
evaluation methods of a pool. Same rules as basic.py: the Python side is a direct call into icepool; generators
keep programs small.

Every node starts with pool.fresh_caches(): icepool's shared evaluators (sum_evaluator, ...) and PoolSource keep
==-keyed caches between calls, which in a long-lived oracle would leak spellings (1 for True) across programs.
"""
import operator

import icepool

from harness.core.grammar_api import node
from harness.grammar import peek
from harness.grammar.pool import STRAIGHT_METHODS, fresh_caches, gen_pool as _gen_pool, huge_range


def gen_pool(g, depth, negative=True):
    """A pool or a deal (with keep-like operations) to evaluate."""
    return _gen_pool(g, depth, negative, deals=True)


def _straight_pool(g, depth):
    """A pool to ask for straights: not one whose outcomes span a huge range (pool.py `huge_range`)."""
    for _ in range(10):
        pool = gen_pool(g, depth)
        if not huge_range(pool):
            return pool
    return {"op": "d_pool", "die_sizes": [6, 6], "mapping": {"bool": False}}


def _gen1(op, **params):
    """A generator for a one-pool evaluation; `params` maps argument names to generators g -> JSON value. A
    straight is not asked of a pool whose outcomes span a huge range (pool.py `huge_range`)."""
    def gen(g, depth):
        pool = _straight_pool(g, depth) if any(op.endswith(m) for m in STRAIGHT_METHODS) else gen_pool(g, depth)
        program = {"op": op, "pool": pool}
        for k, f in params.items():
            program[k] = f(g)
        return program
    return gen


def _other(g, depth):
    """The second multiset of a two-input evaluation: a pool, or a list of outcomes (implicitly a Pool)."""
    if g.rng.random() < 0.7:
        return gen_pool(g, depth)
    return [g.outcome() for _ in range(g.rng.randint(0, 3))]


def _simple(op, method, doc):
    def run(pool):
        fresh_caches()
        return getattr(pool, method)()
    run.__doc__ = doc
    node(op, "die", gen=_gen1(op), weight=0.4)(run)


_simple("pool_sum", "sum", "pool.sum(): the sum of all elements (outcome * count; a lone count of 1 is the outcome).")
_simple("pool_size", "size", "pool.size(): the total number of elements, i.e. the sum of all counts.")
_simple("pool_empty", "empty", "pool.empty(): whether the multiset contains only zero counts.")
_simple("pool_any", "any", "pool.any(): whether the multiset contains any nonzero count.")
_simple("pool_product_of_counts", "product_of_counts", "pool.product_of_counts(): the product of all counts.")
_simple("pool_highest_outcome_and_count", "highest_outcome_and_count",
        "pool.highest_outcome_and_count(): the highest outcome with positive count, along with that count.")
_simple("pool_largest_count", "largest_count", "pool.largest_count(): the size of the largest matching set.")
_simple("pool_largest_count_and_outcome", "largest_count_and_outcome",
        "pool.largest_count_and_outcome(): the largest matching set and its outcome.")
_simple("pool_largest_straight", "largest_straight",
        "pool.largest_straight(): the size of the largest straight (int outcomes).")
_simple("pool_all_straights", "all_straights", "pool.all_straights(): the sizes of all straights, descending.")


@node("pool_all_counts", "die",
      gen=_gen1("pool_all_counts", filter=lambda g: g.rng.choice([1, 1, 2, 0, -1, 3, {"str": "all"}])), weight=0.4)
def pool_all_counts(pool, filter):
    """pool.all_counts(filter=filter): all counts >= filter (or 'all'), descending."""
    fresh_caches()
    return pool.all_counts(filter=filter)


def _gen_expand(g, depth):
    # The result has an outcome per possible multiset: from a pool of at most three leaf dice, so that dice made
    # from it stay well below Python's recursion limit (~1000 outcomes) when pooled and evaluated again. Leaf dice
    # go up to d99 (dz_attr), so ask the oracle and retry a result with too many outcomes.
    for _ in range(10):
        n = g.rng.randint(0, 3)
        p = {"op": "pool", "dice": [g.die(0) for _ in range(n)], "times": g.rng.choice([1, 1, 2]) if n < 3 else 1}
        if g.rng.random() < 0.3:
            p = {"op": "pool_highest", "pool": p, "keep": g.rng.randint(0, 2), "drop": None}
        program = {"op": "pool_expand", "pool": p, "order": g.rng.choice([1, -1, 0])}
        try:
            if len(peek(program)) > 500:
                continue
        except Exception:  # noqa: BLE001 - a program that raises is a fine program too
            pass
        return program
    return {"op": "pool_expand", "pool": {"op": "d_pool", "die_sizes": [6, 6], "mapping": {"bool": False}},
            "order": 1}


@node("pool_expand", "die", gen=_gen_expand, weight=0.3)
def pool_expand(pool, order):
    """pool.expand(order=order): all elements, sorted ascending iff order == Order.Ascending."""
    fresh_caches()
    return pool.expand(order=order)


@node("pool_largest_straight_and_outcome", "die",
      gen=_gen1("pool_largest_straight_and_outcome",
                priority=lambda g: {"str": g.rng.choice(["high", "high", "low", "low", "mid"])}), weight=0.4)
def pool_largest_straight_and_outcome(pool, priority):
    """pool.largest_straight_and_outcome(priority): the largest straight and its highest (or lowest) outcome."""
    fresh_caches()
    return pool.largest_straight_and_outcome(priority)


def _gen_straight_and_outcome_evaluate(g, depth):
    arg = _straight_pool(g, depth) if g.rng.random() < 0.85 else [g.outcome() for _ in range(g.rng.randint(0, 3))]
    priority = g.rng.choice(["high", "low", "mid", "", "HIGH"])
    return {"op": "straight_and_outcome_evaluate", "priority": {"str": priority}, "arg": arg}


@node("straight_and_outcome_evaluate", "die", gen=_gen_straight_and_outcome_evaluate, weight=0.2)
def straight_and_outcome_evaluate(priority, arg):
    """icepool.evaluator.LargestStraightAndOutcomeEvaluator(priority).evaluate(arg): the evaluator built directly,
    whose constructor rejects a priority other than 'low' / 'high' (the method checks it before)."""
    fresh_caches()
    return icepool.evaluator.LargestStraightAndOutcomeEvaluator(priority).evaluate(arg)


def _gen_count_subset(g, depth):
    empty_divisor = None if g.rng.random() < 0.5 else g.rng.randint(-1, 3)
    return {"op": "pool_count_subset", "pool": gen_pool(g, depth), "divisor": _other(g, depth),
            "empty_divisor": empty_divisor}


@node("pool_count_subset", "die", gen=_gen_count_subset, weight=0.4)
def pool_count_subset(pool, divisor, empty_divisor):
    """pool.count_subset(divisor, empty_divisor=empty_divisor): how many times divisor fits in pool."""
    fresh_caches()
    return pool.count_subset(divisor, empty_divisor=empty_divisor)


def _compare(op, method):
    def gen(g, depth):
        return {"op": op, "pool": gen_pool(g, depth), "other": _other(g, depth)}

    def run(pool, other):
        fresh_caches()
        return getattr(pool, method)(other)
    run.__doc__ = f"pool.{method}(other): other is a pool or a sequence of outcomes."
    node(op, "die", gen=gen, weight=0.3)(run)


_compare("pool_issubset", "issubset")
_compare("pool_issuperset", "issuperset")
_compare("pool_isdisjoint", "isdisjoint")


# --- Named evaluators (harness/catalogue/evaluators.py) ---------------------------------------------------------

def _gen_evaluate(g, depth):
    from harness.catalogue.evaluators import ENTRIES
    name = g.rng.choice(sorted(ENTRIES))
    e = ENTRIES[name]
    args = [gen_pool(g, depth) if g.rng.random() < 0.85 else [g.outcome() for _ in range(g.rng.randint(0, 3))]
            for _ in range(e.inputs(g))]
    return {"op": "evaluate", "name": {"str": name}, "params": e.params(g), "args": args}


@node("evaluate", "die", gen=_gen_evaluate, weight=0.6)
def evaluate_named(name, params, args):
    """EVALUATORS[name](*params).evaluate(*args): a named custom evaluator on pools (or sequences of outcomes)."""
    from harness.catalogue.evaluators import EVALUATORS
    fresh_caches()
    return EVALUATORS[name](*params).evaluate(*args)


# --- S1e: the rest of the built-in comparison evaluators, the keep evaluator, keyword arguments ------------------

_COMPARISONS = {
    "proper_subset": "IsProperSubsetEvaluator",
    "proper_superset": "IsProperSupersetEvaluator",
    "equal_set": "IsEqualSetEvaluator",
    "not_equal_set": "IsNotEqualSetEvaluator",
}


def _gen_pool_compare(g, depth):
    return {"op": "pool_compare", "pool": gen_pool(g, depth), "other": _other(g, depth),
            "cmp": {"str": g.rng.choice(sorted(_COMPARISONS))}}


@node("pool_compare", "die", gen=_gen_pool_compare, weight=0.3)
def pool_compare(pool, other, cmp):
    """icepool.evaluator.<Is...Evaluator>().evaluate(pool, other) for cmp in proper_subset (pool < other),
    proper_superset (>), equal_set (==, without the truth value) and not_equal_set (!=)."""
    import icepool
    fresh_caches()
    return getattr(icepool.evaluator, _COMPARISONS[cmp])().evaluate(pool, other)


def _gen_keep_evaluate(g, depth):
    index = None if g.rng.random() < 0.15 else g.rng.randint(-4, 4)
    return {"op": "keep_evaluate", "pool": gen_pool(g, depth), "index": index}


@node("keep_evaluate", "die", gen=_gen_keep_evaluate, weight=0.3)
def keep_evaluate(pool, index):
    """icepool.evaluator.keep_evaluator.evaluate(pool, index=index): the element at that sorted index (negative:
    from the top; None: the first seen), IndexError if the multiset ends first; negative counts count as zero."""
    import icepool
    fresh_caches()
    return icepool.evaluator.keep_evaluator.evaluate(pool, index=index)


def _gen_kwargs(g):
    from harness.grammar.operators import scalar
    kwargs = []
    for name in ("start", "scale"):
        if g.rng.random() < 0.6:
            kwargs.append([{"str": name}, g.rng.randint(-2, 4) if g.rng.random() < 0.7 else scalar(g)])
    if g.rng.random() < 0.1:
        kwargs.append([{"str": "bad"}, 1])
    g.rng.shuffle(kwargs)
    return kwargs


def _gen_evaluate_kw(g, depth):
    from harness.catalogue.evaluators import ENTRIES
    name = g.rng.choice(sorted(n for n, e in ENTRIES.items() if e.kw))
    e = ENTRIES[name]
    args = [gen_pool(g, depth) if g.rng.random() < 0.85 else [g.outcome() for _ in range(g.rng.randint(0, 3))]
            for _ in range(e.inputs(g))]
    return {"op": "evaluate_kw", "name": {"str": name}, "params": e.params(g), "args": args,
            "kwargs": _gen_kwargs(g)}


@node("evaluate_kw", "die", gen=_gen_evaluate_kw, weight=0.3)
def evaluate_kw(name, params, args, kwargs):
    """EVALUATORS[name](*params).evaluate(*args, **dict(kwargs)): keyword arguments (a list of [name, value]) go
    to the evaluator's initial_state and final_outcome."""
    from harness.catalogue.evaluators import EVALUATORS
    fresh_caches()
    return EVALUATORS[name](*params).evaluate(*args, **{k: v for k, v in kwargs})


# --- K6c: evaluations with callables, argsort, lexi, the comparison operators -----------------------------------

def _plain_fn(g, names):
    """A catalogue `fn` node whose results are plain values (no dice or Reroll: an evaluation state is a value),
    or a mapping to small ints."""
    from harness.catalogue.functions import fn_node
    if g.rng.random() < 0.2:
        pairs = [[g.outcome(), g.rng.randint(-2, 8)] for _ in range(g.rng.randint(0, 4))]
        return {"op": "fn", "name": "mapping", "params": [pairs]}
    return fn_node(g, name=g.rng.choice(names))[0]


def _gen_sum_map(g, depth):
    r = g.rng.random()
    if r < 0.1:
        m = None
    elif r < 0.15:
        m = g.rng.randint(0, 3)  # neither callable nor a mapping: AttributeError
    else:
        m = _plain_fn(g, ["add_k", "mul_k", "clamp", "is_even", "gt_k", "identity", "add2"])
    return {"op": "pool_sum_map", "pool": gen_pool(g, depth), "map": m}


@node("pool_sum_map", "die", gen=_gen_sum_map, weight=0.4)
def pool_sum_map(pool, map):
    """pool.sum(map=map): each outcome mapped just before summing (a mapping leaves unmapped outcomes as-is)."""
    fresh_caches()
    return pool.sum(map=map)


def _wild(g):
    r = g.rng.random()
    if r < 0.55:
        return None
    if r < 0.8:
        return [g.outcome() for _ in range(g.rng.randint(0, 3))]
    if r < 0.97:
        return _plain_fn(g, ["is_even", "gt_k", "identity"])
    return g.rng.choice([{"str": "35"}, {"tuple": [1, 2]}, 3])


def _gen_largest_count_wild(g, depth):
    return {"op": "pool_largest_count_wild", "pool": gen_pool(g, depth), "wild": _wild(g), "wild_low": _wild(g),
            "wild_high": _wild(g)}


@node("pool_largest_count_wild", "die", gen=_gen_largest_count_wild, weight=0.4)
def pool_largest_count_wild(pool, wild, wild_low, wild_high):
    """pool.largest_count(wild=, wild_low=, wild_high=): each None, a callable, or a collection (a frozenset of
    outcomes); wild_low wilds only combine with higher outcomes, wild_high with lower ones."""
    fresh_caches()
    return pool.largest_count(wild=wild, wild_low=wild_low, wild_high=wild_high)


def _gen_reduce_counts(g, depth):
    from harness.catalogue.functions import fn_node
    name = g.rng.choice(["mul2", "mul2", "add2", "max2"])
    return {"op": "pool_all_straights_reduce_counts", "pool": _straight_pool(g, depth),
            "reducer": fn_node(g, name=name)[0]}


@node("pool_all_straights_reduce_counts", "die", gen=_gen_reduce_counts, weight=0.3)
def pool_all_straights_reduce_counts(pool, reducer):
    """pool.all_straights_reduce_counts(reducer): (run_length, run_score) per straight, descending; the score is
    the counts reduced by reducer (operator.mul is mul2)."""
    fresh_caches()
    return pool.all_straights_reduce_counts(reducer)


def _argsort_args(g, depth):
    return [gen_pool(g, depth) if g.rng.random() < 0.75 else [g.outcome() for _ in range(g.rng.randint(0, 3))]
            for _ in range(g.rng.choice([1, 2, 2, 3]))]


def _limit(g):
    return None if g.rng.random() < 0.4 else g.rng.randint(-1, 4)


def _gen_argsort(g, depth):
    from harness.grammar.operators import scalar
    pad = None if g.rng.random() < 0.4 else g.rng.randint(-1, 9) if g.rng.random() < 0.8 else scalar(g)
    tie = g.rng.choice(["drop", "left", "right"]) if g.rng.random() < 0.97 else "up"
    return {"op": "argsort", "args": _argsort_args(g, depth), "order": g.rng.choice([-1, -1, 1, 1, 0]),
            "limit": _limit(g), "limit_pad": pad, "tie": {"str": tie}}


@node("argsort", "die", gen=_gen_argsort, weight=0.4)
def argsort(args, order, limit, limit_pad, tie):
    """icepool.MultisetExpression.argsort(*args, order=, limit=, limit_pad=, tie=): the index of the originating
    multiset for each rank of their additive union (args may be sequences: implicitly Pools)."""
    fresh_caches()
    return icepool.MultisetExpression.argsort(*args, order=order, limit=limit, limit_pad=limit_pad, tie=tie)


@node("argsort_grouped", "die", weight=0.3,
      gen=lambda g, depth: {"op": "argsort_grouped", "args": _argsort_args(g, depth),
                            "order": g.rng.choice([-1, -1, 1, 1, 0]), "limit": _limit(g)})
def argsort_grouped(args, order, limit):
    """icepool.MultisetExpression.argsort_grouped(*args, order=, limit=): as argsort, ties grouped into a tuple
    per rank."""
    fresh_caches()
    return icepool.MultisetExpression.argsort_grouped(*args, order=order, limit=limit)


_LEXI_COMPARISONS = ["==", "!=", "<=", "<", ">=", ">", "cmp"]
_LEXI_EXTRAS = ["high", "low", "drop", "early", "late", "equal", "keep", "continue", "break"]


def _lexi(op, method):
    def gen(g, depth):
        comparison = g.rng.choice(_LEXI_COMPARISONS) if g.rng.random() < 0.97 else "<>"
        extra = g.rng.choice(_LEXI_EXTRAS) if g.rng.random() < 0.97 else "none"
        return {"op": op, "pool": gen_pool(g, depth), "comparison": {"str": comparison},
                "other": _other(g, depth), "extra": {"str": extra}}

    def run(pool, comparison, other, extra):
        fresh_caches()
        return getattr(pool, method)(comparison, other, extra)
    run.__doc__ = (f"pool.{method}(comparison, other, extra): lexicographic comparison after sorting each multiset "
                   f"({'descending' if method == 'leximax' else 'ascending'}).")
    node(op, "die", gen=gen, weight=0.3)(run)


_lexi("pool_leximin", "leximin")
_lexi("pool_leximax", "leximax")


_CMP = {"<": operator.lt, "<=": operator.le, ">": operator.gt, ">=": operator.ge, "==": operator.eq,
        "!=": operator.ne}


def _operand(g, depth):
    """The other operand of a multiset operator: a pool, a sequence, a scalar or a die."""
    from harness.grammar.operators import scalar
    r = g.rng.random()
    if r < 0.45:
        return gen_pool(g, depth)
    if r < 0.75:
        return [g.outcome() for _ in range(g.rng.randint(0, 3))]
    if r < 0.92:
        return scalar(g)
    return g.die(0)


def _gen_mx_compare(g, depth):
    a, b = gen_pool(g, depth), _operand(g, depth)
    if g.rng.random() < 0.3:
        a, b = b, a
    return {"op": "mx_compare", "a": a, "o": {"str": g.rng.choice(sorted(_CMP))}, "b": b}


@node("mx_compare", "die_truth", gen=_gen_mx_compare, weight=0.6)
def mx_compare(a, o, b):
    """a o b for o in < <= > >= == != with a multiset expression on at least one side: the subset / superset /
    equal-set evaluations (TypeError -> NotImplemented -> the reflected operator); == and != give a DieWithTruth
    whose truth value is hash-key equality. Kind "die_truth", like format.py's eq / ne: a DieWithTruth only goes
    where its type name and truth value are what is tested."""
    fresh_caches()
    return _CMP[o](a, b)


@node("mx_rfloordiv", "die", weight=0.3,
      gen=lambda g, depth: {"op": "mx_rfloordiv", "other": _operand(g, depth), "pool": gen_pool(g, depth)})
def mx_rfloordiv(other, pool):
    """other // pool: MultisetExpression.__rfloordiv__, i.e. implicit_convert(other).count_subset(pool) (an
    expression on the left: its own __floordiv__, the same count_subset)."""
    fresh_caches()
    return other // pool
