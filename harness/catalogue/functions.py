"""Named functions for map / reroll / filter / split (Python side).

Rust side: crates/icepool-core/src/program/catalogue.rs, one `Func` variant per entry, same parameters, same body.

A program names a function with the `fn` node: {"op": "fn", "name": "add_k", "params": [3]} is
FUNCTIONS["add_k"](3), i.e. `lambda x: x + 3`. Each entry is a factory taking the params and returning the
callable (or, for "mapping", a dict), so that `infer_star` sees the real signature.

Each entry also says how to generate it: `params(g)` gives a params list, `args(g, depth)` the arguments `map`
gets with it, and `pred` whether it makes sense as a predicate for reroll/filter/split (one outcome in, truthy out).
"""
from dataclasses import dataclass
from typing import Callable, Optional

import icepool

from harness.core.khepri import f2f


@dataclass
class Entry:
    factory: Callable
    params: Callable                    # params(g) -> list of JSON program values
    args: Callable                      # args(g, depth) -> list of map arguments
    pred: bool = False
    kw: bool = False                    # takes keyword-only arguments: only for the map_kw node


FUNCTIONS: dict[str, Callable] = {}
ENTRIES: dict[str, Entry] = {}


def entry(name, params=lambda g: [], args=None, pred=False, kw=False):
    def deco(factory):
        FUNCTIONS[name] = factory
        ENTRIES[name] = Entry(factory, params, args or (lambda g, depth: [g.die(depth)]), pred, kw)
        return factory
    return deco


# --- Argument generators -----------------------------------------------------------------------------------------

def _k(g):
    """A small int parameter, sometimes another scalar type (so the function raises or changes type)."""
    from harness.grammar.operators import scalar
    return g.rng.randint(-2, 6) if g.rng.random() < 0.8 else scalar(g)


def _sides(g):
    return g.rng.choice([1, 2, 3, 4, 6]) if g.rng.random() < 0.95 else g.rng.choice([0, -1])


def tuple_die(g, depth, width=2):
    """A die of `width`-tuples: a literal die, or a map that pairs (so the tuples come from icepool itself)."""
    r = g.rng
    if width == 2 and r.random() < 0.4:
        return {"op": "map", "f": {"op": "fn", "name": "pair_k", "params": [_k(g)]},
                "args": [g.die(min(depth, 1))], "star": None}
    outcomes = [{"tuple": [g.outcome() if r.random() < 0.9 else {"str": "a"} for _ in range(width)]}
                for _ in range(r.randint(1, 4))]
    return {"op": "die", "outcomes": outcomes}


def _small_die(g, depth):
    """A die small enough that a product of three stays cheap."""
    r = g.rng
    if r.random() < 0.5:
        return {"op": "d", "sides": r.randint(1, 6)}
    return g.die(min(depth, 1))


# --- The functions -----------------------------------------------------------------------------------------------

@entry("add_k", params=lambda g: [_k(g)])
def add_k(k):
    return lambda x: x + k


@entry("mul_k", params=lambda g: [_k(g)])
def mul_k(k):
    return lambda x: x * k


@entry("clamp", params=lambda g: sorted([g.rng.randint(-2, 8), g.rng.randint(-2, 8)]) if g.rng.random() < 0.9
       else [_k(g), _k(g)])
def clamp(lo, hi):
    return lambda x: min(max(x, lo), hi)


def _read_roll_args(g, depth):
    sv = g.rng.choice([0, 1, 5, 10, 12, 19, 20, 21, 23, 30])
    die = {"op": "d", "sides": 20} if g.rng.random() < 0.6 else g.die(depth)
    if g.rng.random() < 0.15:
        sv = {"op": "die", "outcomes": [g.rng.choice([5, 12, 21, 25]) for _ in range(g.rng.randint(1, 3))]}
    return [die, sv]


@entry("read_roll", args=_read_roll_args)
def read_roll():
    return f2f.read_roll


@entry("is_even", pred=True)
def is_even():
    return lambda x: x % 2 == 0


@entry("gt_k", params=lambda g: [_k(g)], pred=True)
def gt_k(k):
    return lambda x: x > k


@entry("identity", pred=True)
def identity():
    return lambda x: x


@entry("swap", args=lambda g, depth: [tuple_die(g, depth)])
def swap():
    return lambda a, b: (b, a)


def _two(g, depth):
    if g.rng.random() < 0.2:
        return [tuple_die(g, depth)]  # star inferred: one die of pairs
    other = _small_die(g, depth) if g.rng.random() < 0.85 else g.rng.randint(-2, 5)
    return [_small_die(g, depth), other]


@entry("add2", args=_two)
def add2():
    return lambda a, b: a + b


@entry("max2", args=_two)
def max2():
    return lambda a, b: max(a, b)


def _three(g, depth):
    if g.rng.random() < 0.2:
        return [tuple_die(g, depth, width=3)]
    if g.rng.random() < 0.2:
        return [tuple_die(g, depth), _small_die(g, depth)]  # star: (a, b) unpacked, then c
    return [_small_die(g, depth) for _ in range(3)]


@entry("sum3", args=_three)
def sum3():
    return lambda a, b, c: a + b + c


@entry("first", args=lambda g, depth: [tuple_die(g, depth, width=g.rng.choice([1, 2, 2]))] if g.rng.random() < 0.8
       else [g.die(depth)])
def first():
    return lambda t: t[0]


@entry("pair_k", params=lambda g: [_k(g) if g.rng.random() < 0.8 else {"op": "d", "sides": g.rng.randint(1, 3)}])
def pair_k(k):
    return lambda x: (x, k)


@entry("die_if_k", params=lambda g: [g.rng.randint(0, 6), _sides(g)])
def die_if_k(k, n):
    return lambda x: icepool.d(n) if x == k else x


@entry("plus_d", params=lambda g: [_sides(g)])
def plus_d(n):
    return lambda x: x + icepool.d(n)


@entry("reroll_below", params=lambda g: [_k(g)])
def reroll_below(k):
    return lambda x: icepool.Reroll if x < k else x


@entry("reroll_in_tuple", params=lambda g: [g.rng.randint(0, 6)])
def reroll_in_tuple(k):
    return lambda x: (x, icepool.Reroll) if x == k else (x, x)


@entry("tuple_d", params=lambda g: [_sides(g)])
def tuple_d(n):
    return lambda x: (x, icepool.d(n))


def _const_params(g):
    r = g.rng.random()
    if r < 0.3:
        return [{"op": "d", "sides": g.rng.randint(1, 4)}]
    if r < 0.4:
        return [{"op": "Reroll"}]
    return [_k(g)]


def _const_args(g, depth):
    return [g.die(depth) for _ in range(g.rng.choice([1, 1, 2]))]


@entry("const", params=_const_params, args=_const_args)
def const(v):
    return lambda *args: v


@entry("restart_below", params=lambda g: [_k(g)])
def restart_below(k):
    """Outside map(repeat), Restart drops out of the result like Reroll."""
    return lambda x: icepool.Restart if x < k else x


def _break_params(g):
    r = g.rng.random()
    if r < 0.3:
        v = None
    elif r < 0.5:
        v = {"op": "d", "sides": _sides(g)}
    elif r < 0.6:
        v = {"op": "Reroll"}
    else:
        v = _k(g)
    return [g.rng.randint(0, 6), v]


@entry("break_k", params=_break_params)
def break_k(k, v):
    """Break(v) (Break() for None: the outcome itself) is unwrapped by map without repeat."""
    return lambda x: icepool.Break(v) if x == k else icepool.Break()


@entry("break_eq", args=_two)
def break_eq():
    """With star, Break() stands for the first argument before unpacking: the whole tuple."""
    return lambda a, b: icepool.Break() if a == b else a + b


def _int_dice(g, depth):
    return [{"op": "d", "sides": g.rng.randint(1, 4)} for _ in range(g.rng.choice([1, 2, 2, 3]))]


@entry("count_diff", args=_int_dice)
def count_diff():
    """A map_counts function (outcome, *counts): the first count minus the others."""
    return lambda outcome, *counts: counts[0] - sum(counts[1:])


@entry("count_if_gt_k", params=lambda g: [_k(g)], args=_int_dice)
def count_if_gt_k(k):
    """A map_counts function (outcome, *counts): the total count of outcomes above k."""
    return lambda outcome, *counts: sum(counts) if outcome > k else 0


@entry("add_kw", kw=True)
def add_kw():
    """A keyword-only parameter: map_kw passes it as-is (a die is not expanded)."""
    return lambda x, *, k: x + k


# --- Markov chains: processes for map(repeat), map_and_time, mean_time_to_absorb (harness/grammar/markov.py) ---

def step_die(g):
    """A small die of steps for a process: mostly non-negative (so sums reach a cap), sometimes not."""
    r = g.rng.random()
    if r < 0.45:
        return {"op": "d", "sides": g.rng.randint(1, 3)}
    if r < 0.75:
        return {"op": "die", "outcomes": g.rng.choice([[-1, 1], [-1, 1, 1], [0, 1], [-1, 0, 1], [1, 2], [0, 0, 2]])}
    return {"op": "die", "outcomes": [g.rng.randint(-1, 3) for _ in range(g.rng.randint(1, 3))]}


def _process_args(g, depth):
    """A start (an int or a small die) and one die of steps."""
    start = g.rng.randint(0, 2) if g.rng.random() < 0.6 else {"op": "d", "sides": g.rng.randint(1, 3)}
    return [start, step_die(g)]


def _break_value(g):
    r = g.rng.random()
    if r < 0.4:
        return None
    if r < 0.55:
        return {"op": "d", "sides": g.rng.randint(1, 3)}
    if r < 0.6:
        return {"op": "Reroll"}
    return g.rng.randint(-1, 9)


@entry("cap_sum", params=lambda g: [g.rng.randint(0, 8)], args=_process_args)
def cap_sum(cap):
    """Absorbs at `cap` (time_to_sum's step)."""
    return lambda total, roll: min(total + roll, cap)


@entry("break_at", params=lambda g: [g.rng.randint(0, 8), _break_value(g)], args=_process_args)
def break_at(k, v):
    """Break(v) once the total reaches k (Break() for None: the total itself)."""
    return lambda total, roll: icepool.Break(v) if total >= k else total + roll


@entry("restart_on", params=lambda g: [g.rng.randint(0, 3), g.rng.randint(0, 6)], args=_process_args)
def restart_on(k, cap):
    """Restart the whole process when the roll is k."""
    return lambda total, roll: icepool.Restart if roll == k else min(total + roll, cap)


def _bounds(g):
    return [g.rng.randint(-3, 0), g.rng.randint(0, 4)]


@entry("walk", params=_bounds, args=_process_args)
def walk(lo, hi):
    """A random walk absorbed at lo and hi."""
    return lambda x, step: x if x <= lo or x >= hi else x + step


@entry("walk_die", params=_bounds)
def walk_die(lo, hi):
    """A random walk whose step is a die returned by the function."""
    return lambda x: x if x <= lo or x >= hi else x + icepool.Die([-1, 1])


@entry("collatz", args=lambda g, depth: [{"op": "d", "sides": g.rng.randint(1, 12)}])
def collatz():
    return lambda x: 1 if x <= 1 else x // 2 if x % 2 == 0 else 3 * x + 1


def pair_die(g):
    """A die of (a, b) states for pair_walk."""
    return {"op": "die", "outcomes": [{"tuple": [g.rng.randint(0, 2), g.rng.randint(0, 2)]}
                                      for _ in range(g.rng.randint(1, 3))]}


@entry("pair_walk", params=lambda g: [g.rng.randint(0, 6)],
       args=lambda g, depth: [pair_die(g), step_die(g)])
def pair_walk(k):
    """A state that is a pair: with one die of steps, star is inferred and the state unpacked."""
    return lambda a, b, roll: (a, b) if a + b >= k else (b, a + roll)


def _mapping_params(g):
    r = g.rng
    pairs = []
    for _ in range(r.randint(0, 4)):
        v = r.random()
        if v < 0.6:
            val = r.randint(-2, 8)
        elif v < 0.8:
            val = {"op": "d", "sides": r.randint(1, 4)}
        else:
            val = {"op": "Reroll"}
        pairs.append([g.outcome(), val])
    return [pairs]


@entry("mapping", params=_mapping_params)
def mapping(pairs):
    return {k: v for k, v in pairs}


def fn_node(g, name=None, pred=False, kw=False):
    """A random `fn` node (a predicate if `pred`; keyword-only entries only if `kw`), and its catalogue entry."""
    if name is None:
        names = [n for n, e in ENTRIES.items() if (e.pred or not pred) and (kw or not e.kw)]
        name = g.rng.choice(names)
    e = ENTRIES[name]
    return {"op": "fn", "name": name, "params": e.params(g)}, e
