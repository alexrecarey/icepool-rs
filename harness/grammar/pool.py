"""Pools (generator/pool.py), keep tuples (generator/keep.py) and the lowest / highest of dice (population/keep.py,
die.py). Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small.

A pool's repr holds an address, so a pool is never a program result: the pool nodes have no generator of their
own (the fuzzer would pick them as roots). Programs read pools through `pool_info` (its public accessors and the
sorted `(die, count)` pairs), `pool_pop` (one pop of its source, what the evaluator drives), `pool_index`
(`pool[int]`, the keep evaluator) and the evaluation nodes of evaluate.py.

A pool's keep tuple only goes negative in its last operation (keeping on a negative keep tuple builds a
`MultisetKeep` expression, not a pool: task S4), and never before `pool[int]`.
"""
import icepool

from harness.core.encode import enc
from harness.core.grammar_api import evaluate, node


@node("Ellipsis", "ellipsis")
def ellipsis_const():
    """... (Ellipsis), for keep indices."""
    return ...


@node("pool", "pool")
def pool(dice, times):
    """icepool.Pool(dice, times): dice (or outcomes) each put in times times (an int for all, or one per die)."""
    return icepool.Pool(dice, times)


@node("pool_dict", "pool")
def pool_dict(items, times):
    """icepool.Pool(dict(items), times): items are [die or outcome, count] pairs (equal keys merge as in a dict:
    first key, last count)."""
    return icepool.Pool(dict(items), times)


@node("d_pool", "pool")
def d_pool(die_sizes, mapping):
    """icepool.d_pool(dict(die_sizes) if mapping else die_sizes): one d(size) per size, or [size, count] pairs."""
    return icepool.d_pool(dict(die_sizes) if mapping else die_sizes)


@node("z_pool", "pool")
def z_pool(die_sizes, mapping):
    """icepool.z_pool(dict(die_sizes) if mapping else die_sizes): as d_pool with z(size)."""
    return icepool.z_pool(dict(die_sizes) if mapping else die_sizes)


@node("die_pool", "pool")
def die_pool(die, rolls):
    """die.pool(rolls): rolls copies of die; a sequence of counts is Pool({die: len(rolls)})[rolls]."""
    return die.pool(rolls)


@node("pool_keep", "pool")
def pool_keep(pool, index):
    """pool.keep(index): index is a sequence of counts (at most one ..., see make_keep_tuple) or an int (which
    evaluates the kept roll into a Die)."""
    fresh_caches()
    return pool.keep(index)


@node("pool_slice", "pool")
def pool_slice(pool, start, stop, step):
    """pool[start:stop:step]: the selected sorted rolls counted once each (a step raises IndexError)."""
    return pool[start:stop:step]


@node("pool_highest", "pool")
def pool_highest(pool, keep, drop):
    """pool.highest(keep, drop): drop the drop highest, keep the next keep (None: see highest_slice)."""
    return pool.highest(keep, drop)


@node("pool_lowest", "pool")
def pool_lowest(pool, keep, drop):
    """pool.lowest(keep, drop)"""
    return pool.lowest(keep, drop)


@node("pool_middle", "pool")
def pool_middle(pool, keep, tie):
    """pool.middle(keep, tie=tie): tie is 'error', 'high' or 'low'."""
    return pool.middle(keep, tie=tie)


@node("pool_mul", "pool")
def pool_mul(pool, n):
    """pool * n: every count multiplied by the int n."""
    return pool * n


@node("pool_neg", "pool")
def pool_neg(pool):
    """-pool: every count negated."""
    return -pool


@node("pool_add", "pool")
def pool_add(a, b):
    """a + b for pools whose sorted positions all count the same: one merged pool."""
    return a + b


def fresh_caches():
    """Clear icepool's caches that outlive one call: PoolSource._new_raw (@cache) and the room caches the shared
    evaluator instances (icepool.evaluator.sum_evaluator, keep_evaluator, ...) keep between evaluate() calls. Both
    are keyed by ==, so in a long-lived oracle process a result can come back with another program's spellings
    (1 for True). Called at the start of every node that evaluates a pool; the port caches within one evaluation
    only (see NOTES.md)."""
    icepool.Pool.clear_cache()
    for ev in vars(icepool.evaluator).values():
        if isinstance(ev, icepool.MultisetEvaluator) and hasattr(ev, "_cache"):
            del ev._cache


# --- Pool generators --------------------------------------------------------------------------------------------

def _has_nan(x):
    if isinstance(x, float):
        return x != x
    return isinstance(x, tuple) and any(_has_nan(y) for y in x)


def _die(g, depth):
    """A die for a pool. Popping costs a binomial row per distinct die: keep the dice themselves modest."""
    return _die_without_nan(g, min(depth, 1))


def _die_without_nan(g, depth):
    """A die program without NaN outcomes (inf arithmetic makes them), for anything that may end up in a pool: a
    pool's outcomes are sorted(set(...)), and NaN hashes by object identity, so where NaNs land in Python depends
    on memory addresses. Asks the oracle, like _gen_pool_pop."""
    for _ in range(10):
        p = g.die(depth)
        try:
            die = evaluate(p)
            if isinstance(die, icepool.Die) and any(_has_nan(o) for o in die.outcomes()):
                continue
        except Exception:  # noqa: BLE001 - a die that raises is a fine program too
            pass
        return p
    return {"op": "d", "sides": g.rng.randint(1, 6)}


def _times(g, n):
    """`times` for n keys: an int for all, or one per key (rarely the wrong length or negative, which raise)."""
    if g.rng.random() < 0.6:
        return g.rng.choice([1, 1, 0, 2, 3, -1 if g.rng.random() < 0.1 else 1])
    m = n if g.rng.random() < 0.9 else g.rng.randint(0, 3)
    return [g.rng.choice([0, 1, 1, 2, -1 if g.rng.random() < 0.05 else 1]) for _ in range(m)]


def _sizes(g):
    """Die sizes for d_pool / z_pool (rarely 0 or negative, which raise), and whether they are [size, count] pairs."""
    size = lambda: g.rng.choice([1, 2, 3, 4, 6, 6, 0 if g.rng.random() < 0.1 else 4])
    n = g.rng.choice([0, 1, 2, 2, 3])
    if g.rng.random() < 0.6:
        return [size() for _ in range(n)], False
    return [[size(), g.rng.choice([0, 1, 1, 2, 3, -1])] for _ in range(n)], True


def _other_pool(g, depth):
    """Pool(pool, times), Pool(dict, times), d_pool and z_pool."""
    r = g.rng.random()
    if r < 0.3:
        return {"op": "pool", "dice": _base_pool(g, depth), "times": _times(g, g.rng.randint(1, 2))}
    if r < 0.6:
        n = g.rng.choice([0, 1, 2, 2, 3])
        keys = [_die(g, depth) if g.rng.random() < 0.7 else g.outcome() for _ in range(n)]
        if n and g.rng.random() < 0.2:
            keys.append(keys[0])  # an equal key: the dict merges it
        items = [[k, g.rng.choice([0, 1, 1, 2, 3, -1 if g.rng.random() < 0.1 else 1])] for k in keys]
        return {"op": "pool_dict", "items": items, "times": _times(g, len(items))}
    sizes, mapping = _sizes(g)
    return {"op": g.rng.choice(["d_pool", "z_pool"]), "die_sizes": sizes, "mapping": {"bool": mapping}}


def _base_pool(g, depth, uniform=False):
    """A new pool. `uniform`: every count 1, so that two of them add up to one pool."""
    r = g.rng.random()
    if r < 0.25 and not uniform:
        return _other_pool(g, depth)
    if r < 0.45:
        n = g.rng.choice([0, 1, 1, 2, 2, 3, 4])
        dice = [_die(g, depth) if g.rng.random() < 0.8 else g.outcome() for _ in range(n)]
        if g.rng.random() < 0.3:
            times = [g.rng.choice([0, 1, 1, 2]) for _ in range(n)]
        else:
            times = g.rng.choice([1, 1, 1, 1, 0, 2, 3])
        return {"op": "pool", "dice": dice, "times": times}
    if r < 0.85:
        if uniform or g.rng.random() < 0.8:
            rolls = g.rng.randint(0, 5)
        else:
            rolls = [g.rng.choice([0, 1, 1, 2]) for _ in range(g.rng.randint(0, 4))]
        return {"op": "die_pool", "die": _die(g, depth), "rolls": rolls}
    if uniform:
        return {"op": "die_pool", "die": _die(g, depth), "rolls": g.rng.randint(0, 3)}
    a, b = _base_pool(g, depth, True), _base_pool(g, depth, True)
    return {"op": "pool_add", "a": a, "b": b}


def _count(g, final):
    return g.rng.choice([0, 1, 1, 1, 2] + ([-1, -2] if final else []))


def _keep_index(g, final):
    """A sequence of counts, usually with one `...` (without one it must match the keep size exactly)."""
    counts = [_count(g, final) for _ in range(g.rng.randint(0, 4))]
    r = g.rng.random()
    if r < 0.75:
        counts.insert(g.rng.randint(0, len(counts)), {"op": "Ellipsis"})
    elif r < 0.8:
        counts.insert(g.rng.randint(0, len(counts)), {"op": "Ellipsis"})
        counts.insert(g.rng.randint(0, len(counts)), {"op": "Ellipsis"})
    return counts


def _opt(g, lo, hi):
    return None if g.rng.random() < 0.35 else g.rng.randint(lo, hi)


def _keep_op(g, p, final):
    """One keep-like operation on pool program p. Only a final one may make counts negative."""
    r = g.rng.random()
    if r < 0.25:
        step = 1 if g.rng.random() < 0.05 else None
        return {"op": "pool_slice", "pool": p, "start": _opt(g, -5, 5), "stop": _opt(g, -5, 5), "step": step}
    if r < 0.45:
        return {"op": "pool_keep", "pool": p, "index": _keep_index(g, final)}
    if r < 0.65:
        op = g.rng.choice(["pool_highest", "pool_lowest"])
        return {"op": op, "pool": p, "keep": _opt(g, -1 if g.rng.random() < 0.05 else 0, 4),
                "drop": _opt(g, 0, 3)}
    if r < 0.8:
        tie = g.rng.choice(["error", "high", "high", "low", "low", "bad"])
        return {"op": "pool_middle", "pool": p, "keep": g.rng.randint(-1 if g.rng.random() < 0.05 else 0, 5),
                "tie": {"str": tie}}
    if r < 0.92 or not final:
        return {"op": "pool_mul", "pool": p, "n": g.rng.randint(-2 if final else 0, 3)}
    return {"op": "pool_neg", "pool": p}


def gen_pool(g, depth, negative=True):
    """A pool program: a pool, then up to three keep-like operations. `negative`: the last one may make counts
    negative."""
    p = _base_pool(g, depth)
    for _ in range(g.rng.choice([0, 1, 1, 2])):
        p = _keep_op(g, p, final=False)
    if g.rng.random() < 0.4:
        p = _keep_op(g, p, final=negative)
    return p


# --- Reading pools ----------------------------------------------------------------------------------------------

@node("pool_info", "value", gen=lambda g, depth: {"op": "pool_info", "pool": gen_pool(g, depth)}, weight=0.7)
def pool_info(pool):
    """(keep_tuple(), keep_size(), raw_size(), denominator(), outcomes(), _dice): _dice is the (die, count)
    pairs sorted by the die's hash_key, the order the pool's source pops them in."""
    return (pool.keep_tuple(), pool.keep_size(), pool.raw_size(), pool.denominator(), pool.outcomes(),
            pool._dice)


def _gen_pool_pop(g, depth):
    p = gen_pool(g, depth)
    order = g.rng.choice([1, 1, -1, -1, 0])
    outcome = g.outcome()
    # Mostly pop the outcome the source pops (otherwise it yields itself unchanged): find it by asking the oracle.
    try:
        outcomes = evaluate(p).outcomes()
        if outcomes and g.rng.random() < 0.85:
            outcome = enc(outcomes[0] if order > 0 else outcomes[-1])
    except Exception:  # noqa: BLE001 - a pool that raises is a fine program too
        pass
    return {"op": "pool_pop", "pool": p, "order": order, "outcome": outcome}


@node("pool_pop", "value", gen=_gen_pool_pop, weight=0.7)
def pool_pop(pool, order, outcome):
    """pool._make_source().pop(Order(order), outcome) as a tuple of (dice, outcomes, keep_tuple, count, weight):
    every way the dice can roll the popped outcome (the min for order 1, else the max), last the skipped ones.

    PoolSource._new_raw is @cache'd on (dice, outcomes, keep_tuple) compared with ==, so in a long-lived process a
    pop can return a source cached earlier with other spellings (1 for True). Clearing the cache (public API)
    gives what a fresh process gives, which is what the port does (see DIVERGENCES.md)."""
    fresh_caches()
    return tuple((s.dice, s.outcomes(), s.keep_tuple, count, weight)
                 for s, count, weight in pool._make_source().pop(icepool.Order(order), outcome))


def _gen_pool_index(g, depth):
    index = g.rng.randint(-4, 4) if g.rng.random() < 0.95 else g.rng.choice([10, -10])
    return {"op": "pool_index", "pool": gen_pool(g, depth, negative=False), "index": index}


@node("pool_index", "die", gen=_gen_pool_index, weight=0.5)
def pool_index(pool, index):
    """pool[index] for an int: the roll at that sorted position (of the kept ones) as a Die (the keep evaluator)."""
    fresh_caches()
    return pool[index]


# --- Lowest / highest of dice -----------------------------------------------------------------------------------

def _keep_drop(g):
    """keep and drop counts (None: see highest_slice / lowest_slice); rarely a negative keep, which raises."""
    return _opt(g, -1 if g.rng.random() < 0.05 else 0, 3), _opt(g, 0, 3)


def _gen_die_keep(op):
    def gen(g, depth):
        rolls = g.rng.randint(1, 5)
        keep, drop = _keep_drop(g)
        return {"op": op, "die": _die_without_nan(g, depth), "rolls": rolls, "keep": keep, "drop": drop}
    return gen


@node("die_highest", "die", gen=_gen_die_keep("die_highest"), weight=0.5)
def die_highest(die, rolls, keep, drop):
    """die.highest(rolls, keep, drop): roll rolls of die, sum the keep highest after dropping the drop highest."""
    fresh_caches()
    return die.highest(rolls, keep, drop)


@node("die_lowest", "die", gen=_gen_die_keep("die_lowest"), weight=0.5)
def die_lowest(die, rolls, keep, drop):
    """die.lowest(rolls, keep, drop)"""
    fresh_caches()
    return die.lowest(rolls, keep, drop)


def _gen_free_keep(op):
    def gen(g, depth):
        n = g.rng.choice([0, 1, 2, 2, 3, 4])
        args = [_die_without_nan(g, depth) if g.rng.random() < 0.8 else g.outcome() for _ in range(n)]
        keep, drop = _keep_drop(g)
        default = g.outcome() if n == 0 and g.rng.random() < 0.6 else None
        return {"op": op, "args": args, "keep": keep, "drop": drop, "default": default}
    return gen


@node("highest", "die", gen=_gen_free_keep("highest"), weight=0.5)
def highest(args, keep, drop, default):
    """icepool.highest(args, keep=keep, drop=drop, default=default): the dice (or outcomes) in one list."""
    fresh_caches()
    return icepool.highest(args, keep=keep, drop=drop, default=default)


@node("lowest", "die", gen=_gen_free_keep("lowest"), weight=0.5)
def lowest(args, keep, drop, default):
    """icepool.lowest(args, keep=keep, drop=drop, default=default)"""
    fresh_caches()
    return icepool.lowest(args, keep=keep, drop=drop, default=default)
