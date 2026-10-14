"""Multiset evaluation (evaluator/multiset_evaluator_base.py, basic.py, poker.py, keep.py, comparison.py): the
evaluation methods of a pool. Same rules as basic.py: the Python side is a direct call into icepool; generators
keep programs small.

Every node starts with pool.fresh_caches(): icepool's shared evaluators (sum_evaluator, ...) and PoolSource keep
==-keyed caches between calls, which in a long-lived oracle would leak spellings (1 for True) across programs.
"""
from harness.core.grammar_api import node
from harness.grammar.pool import fresh_caches, gen_pool


def _gen1(op, **params):
    """A generator for a one-pool evaluation; `params` maps argument names to generators g -> JSON value."""
    def gen(g, depth):
        program = {"op": op, "pool": gen_pool(g, depth)}
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
    # from it stay well below Python's recursion limit (~1000 outcomes) when pooled and evaluated again.
    n = g.rng.randint(0, 3)
    p = {"op": "pool", "dice": [g.die(0) for _ in range(n)], "times": g.rng.choice([1, 1, 2]) if n < 3 else 1}
    if g.rng.random() < 0.3:
        p = {"op": "pool_highest", "pool": p, "keep": g.rng.randint(0, 2), "drop": None}
    return {"op": "pool_expand", "pool": p, "order": g.rng.choice([1, -1, 0])}


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
