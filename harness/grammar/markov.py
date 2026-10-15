"""map(repeat), map_and_time, mean_time_to_absorb and Die.time_to_sum: Markov processes (map_tools/function.py,
markov_chain.py). Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs
small.

Generators build their own small int dice (not g.die()): a process over arbitrary dice reaches many states, and
NaN or equal states spelled differently (1, True, 1.0) make Python's repeat='inf' depend on set order.
"""
import icepool

from harness.catalogue.functions import fn_node, pair_die, step_die
from harness.core.grammar_api import node
from harness.grammar.maps import _star

# Catalogue functions that make sense as a process, and whether they take a die of steps as an extra argument.
_PROCESSES = {
    "cap_sum": 1, "break_at": 1, "restart_on": 1, "walk": 1, "pair_walk": 1, "add2": 1, "max2": 1, "const": 1,
    "break_eq": 1, "walk_die": 0, "collatz": 0, "clamp": 0, "die_if_k": 0, "reroll_below": 0, "restart_below": 0,
    "break_k": 0, "add_k": 0, "mapping": 0, "identity": 0,
}


def _start(g, name):
    if name == "pair_walk":
        return pair_die(g) if g.rng.random() < 0.7 else {"tuple": [g.rng.randint(0, 2), g.rng.randint(0, 2)]}
    if name == "collatz":
        return {"op": "d", "sides": g.rng.randint(1, 12)}
    r = g.rng.random()
    if r < 0.4:
        return g.rng.randint(-1, 3)
    if r < 0.8:
        return {"op": "d", "sides": g.rng.randint(1, 4)}
    return {"op": "die", "outcomes": [g.rng.randint(-2, 4) for _ in range(g.rng.randint(1, 3))]}


# Processes whose states can grow without bound: an absorbing chain over them never finishes exploring (in Python
# either), so the repeat='inf' / mean_time_to_absorb generators never pick them. (Such a program is not only a
# wasted test: a generator that peeks at a die program, like pool.py's _die_without_nan, would have to wait for it.)
_UNBOUNDED = {"add2", "max2", "break_eq", "add_k"}
# Processes that stay bounded whatever the steps: the rest of _PROCESSES need non-negative steps (they sum the
# steps until a cap; negative steps walk off to -inf).
_ANY_STEPS = {"walk"}


def _process(g, finite=False):
    """(f, start, extra args) for a process. `finite`: a process with finitely many states."""
    names = sorted(_PROCESSES)
    if finite:
        names = [n for n in names if n not in _UNBOUNDED]
    name = g.rng.choice(names)
    f, _ = fn_node(g, name=name)
    steps = (lambda: {"op": "d", "sides": g.rng.randint(1, 3)}) if finite and name not in _ANY_STEPS else (
        lambda: step_die(g))
    extra = [steps() for _ in range(_PROCESSES[name])]
    if g.rng.random() < 0.05:
        # A mapping with an extra argument raises; a single-argument function given one is a TypeError.
        extra.append(step_die(g))
    return f, _start(g, name), extra


def _population(g, die, pool=0.0):
    """Now and then a deck (same outcomes) or, with probability `pool`, a pool of two such dice in place of a die
    argument: `map` / `map_and_time` expand a pool into the die of its sorted tuples (`expand()`), and take a deck's
    items as a die's."""
    if die.get("op") not in ("d", "die"):
        return die
    r = g.rng.random()
    if r < pool:
        return {"op": "pool", "dice": [die, die], "times": 1}
    if r < pool + 0.1:
        outcomes = list(range(1, die["sides"] + 1)) if die["op"] == "d" else die["outcomes"]
        return {"op": "deck", "outcomes": outcomes, "times": g.rng.randint(1, 2)}
    return die


def _repeat(g):
    r = g.rng.random()
    if r < 0.45:
        return {"str": "inf"}
    if r < 0.9:
        return g.rng.randint(0, 5)
    return g.rng.choice([-1, {"float": "2.0"}, {"float": "0.0"}, {"bool": True}, {"str": "x"}])


def _gen_map_repeat(g, depth):
    repeat = _repeat(g)
    f, start, extra = _process(g, finite=repeat == {"str": "inf"})
    if f["name"] == "pair_walk" and g.rng.random() < 0.2:
        # A pool of two dice as the first argument: its sorted pairs are the initial states.
        start = {"op": "pool", "dice": [{"op": "d", "sides": g.rng.randint(1, 3)}] * 2, "times": 1}
    extra = [_population(g, x, pool=0.03) for x in extra]
    args = [start] + extra if g.rng.random() < 0.97 else []
    return {"op": "map_repeat", "f": f, "args": args, "star": _star(g), "repeat": repeat}


@node("map_repeat", "die", gen=_gen_map_repeat)
def map_repeat(f, args, star, repeat):
    """icepool.map(f, *args, star=star, repeat=repeat): repeat an int (stops early once every state is absorbed)
    or 'inf' (the absorbing Markov chain, in simplest form). Break, Reroll, Restart allowed."""
    return icepool.map(f, *args, star=star, repeat=repeat)


def _gen_map_and_time(g, depth):
    f, start, extra = _process(g)
    extra = [_population(g, x, pool=0.03) for x in extra]
    repeat = g.rng.randint(0, 5) if g.rng.random() < 0.9 else g.rng.choice([-1, {"float": "1.0"}])
    return {"op": "map_and_time", "f": f, "initial": start, "args": extra, "star": _star(g), "repeat": repeat}


@node("map_and_time", "die", gen=_gen_map_and_time)
def map_and_time(f, initial, args, star, repeat):
    """icepool.map_and_time(f, initial, *args, star=star, repeat=repeat): a die of (state, time to absorb)."""
    return icepool.map_and_time(f, initial, *args, star=star, repeat=repeat)


def _gen_mean_time_to_absorb(g, depth):
    f, start, extra = _process(g, finite=True)
    # No pool here: mean_time_to_absorb does not expand one (it reaches the function as-is).
    extra = [_population(g, x) for x in extra]
    return {"op": "mean_time_to_absorb", "f": f, "initial": start, "args": extra, "star": _star(g)}


@node("mean_time_to_absorb", "value", gen=_gen_mean_time_to_absorb, weight=0.7)
def mean_time_to_absorb(f, initial, args, star):
    """icepool.mean_time_to_absorb(f, initial, *args, star=star): a Fraction (NotImplementedError with Restart)."""
    return icepool.mean_time_to_absorb(f, initial, *args, star=star)


def _gen_die_mean_time_to_absorb(g, depth):
    f, start, extra = _process(g, finite=True)
    if not isinstance(start, dict) or start.get("op") not in ("d", "die"):
        # The method needs a die as `self` (pair_walk's tuple start state becomes a die of it).
        start = {"op": "die", "outcomes": [start]}
    extra = [_population(g, x) for x in extra]
    return {"op": "die_mean_time_to_absorb", "die": start, "f": f, "args": extra, "star": _star(g)}


@node("die_mean_time_to_absorb", "value", gen=_gen_die_mean_time_to_absorb, weight=0.3)
def die_mean_time_to_absorb(die, f, args, star):
    """die.mean_time_to_absorb(f, *args, star=star): as icepool.mean_time_to_absorb(f, die, *args, star=star)."""
    return die.mean_time_to_absorb(f, *args, star=star)


def _gen_time_to_sum(g, depth):
    r = g.rng.random()
    die = step_die(g) if r < 0.8 else {"op": "die", "outcomes": [g.rng.randint(0, 4) for _ in range(3)]}
    max_time = None if g.rng.random() < 0.5 else g.rng.randint(-1, 5)
    dnf = g.rng.choice([None, None, -1, 0, 9, {"op": "Reroll"}])
    return {"op": "time_to_sum", "die": die, "target": g.rng.randint(-1, 10), "max_time": max_time, "dnf": dnf}


@node("time_to_sum", "die", gen=_gen_time_to_sum, weight=0.7)
def time_to_sum(die, target, max_time, dnf):
    """die.time_to_sum(target, max_time, dnf): the number of rolls until the running sum reaches target."""
    return die.time_to_sum(target, max_time, dnf)
