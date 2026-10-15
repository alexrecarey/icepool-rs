"""Again (depth and count mode) and explode. Same rules as basic.py: the Python side is a direct call into icepool; generators
keep programs small.

An `Again` expression is built with the ordinary operator nodes (add, matmul, neg, ...) applied to the `Again`
node. A bare expression is never a program result (its repr holds an address), so it is only generated inside the
outcomes of `again`.
"""
import math

import icepool

from harness.core.grammar_api import node
from harness.grammar import peek
from harness.grammar.maps import _star, _which
from harness.grammar.operators import _small as small_operand


@node("Again", "again_expr")
def again_const():
    """icepool.Again"""
    return icepool.Again


def _small(g):
    return g.rng.randint(-2, 4)


# Every binary operator `AgainExpression` defines (`@` and `==` / `!=` aside), and the unary ones.
_BINARY = ["add", "add", "sub", "mul", "truediv", "floordiv", "mod", "and", "or", "xor", "lt", "le", "gt", "ge"]
_UNARY = ["neg", "pos", "invert", "abs"]


def _other(g, small_only=False):
    """The non-expression operand: a small int or (unless `small_only`) a die."""
    return _small(g) if small_only or g.rng.random() < 0.7 else g.die(0)


def _again_expr(g, depth, counts):
    """`Again`, or an operator applied to an expression and a small int or die (on either side, so both the
    expression's forward and reflected operators run), or to two expressions. `counts` allows `Again @ die`."""
    if depth <= 0 or g.rng.random() < 0.3:
        return {"op": "Again"}
    inner = _again_expr(g, depth - 1, counts)
    r = g.rng.random()
    if r < 0.4:
        op = g.rng.choice(_BINARY)
        # Multiply only by small ints: products grow at every level, and a die of big counts on the left of `@`
        # makes Python's `_sum_all` recurse past its limit.
        other = _other(g, small_only=op in ("mul", "floordiv"))
        a, b = (inner, other) if g.rng.random() < 0.6 else (other, inner)
        return {"op": op, "a": a, "b": b}
    if r < 0.48:
        op = g.rng.choice(["pow", "lshift", "rshift"])
        if g.rng.random() < 0.6:
            # A small exponent or shift count, as in operators.py.
            return {"op": op, "a": inner, "b": small_operand(g)}
        # The expression as the exponent or shift count: its outcomes can be anything the again die rolls (huge
        # ints too), so `% k` keeps them small first.
        b = {"op": "mod", "a": inner, "b": g.rng.randint(2, 4)} if op != "rshift" else inner
        a = _small(g) if g.rng.random() < 0.6 else {"op": "d", "sides": g.rng.randint(1, 3)}
        return {"op": op, "a": a, "b": b}
    if r < 0.58:
        # `==` / `!=`: against another expression (often the same one: equal hash_keys) or a non-expression.
        op = g.rng.choice(["eq", "ne"])
        x = g.rng.random()
        if x < 0.25:
            other = inner
        elif x < 0.5:
            other = _again_expr(g, depth - 1, counts)
        else:
            other = _other(g)
        a, b = (inner, other) if g.rng.random() < 0.6 else (other, inner)
        return {"op": op, "a": a, "b": b}
    if r < 0.68:
        return {"op": g.rng.choice(_UNARY), "a": inner}
    if r < 0.8:
        # `k @ Again`: roll again k times and add those up.
        return {"op": "matmul", "n": g.rng.randint(0, 2), "die": inner}
    if r < 0.88 and counts:
        # `Again @ die`: roll again, then roll die that many times. Only on a bare Again, and with a small die.
        return {"op": "matmul", "n": {"op": "Again"}, "die": {"op": "d", "sides": g.rng.randint(1, 3)}}
    return {"op": "add", "a": inner, "b": _again_expr(g, depth - 1, counts)}


def _again_end(g):
    r = g.rng.random()
    if r < 0.5:
        return None
    if r < 0.7:
        return g.outcome()
    if r < 0.85:
        return {"op": "Reroll"}
    return g.die(0)


def _gen_again(g, depth):
    n = g.rng.randint(1, 4)
    depth_arg = g.rng.choice([None, None, 0, 1, 2, 3, -1 if g.rng.random() < 0.2 else 1])
    # `Again @ die` rolls the die as many times as the level below rolled: keep that to one level.
    counts = depth_arg in (None, 0, 1, -1)
    outcomes = []
    for _ in range(n):
        r = g.rng.random()
        if r < 0.4:
            outcomes.append(_again_expr(g, 2, counts))
        elif r < 0.55:
            outcomes.append(g.die(0))
        elif r < 0.6:
            outcomes.append({"op": "Reroll"})
        else:
            outcomes.append(g.outcome())
    return {"op": "again", "outcomes": outcomes, "depth": depth_arg, "end": _again_end(g)}


@node("again", "die", gen=_gen_again, weight=0.7)
def again(outcomes, depth, end):
    """icepool.Die(outcomes, again_depth=depth, again_end=end): outcomes may be Again expressions, rolled again up
    to depth times (None: the default, 1); past that, Again is end (None: a zero; Reroll: those outcomes drop)."""
    return icepool.Die(outcomes, again_depth=depth, again_end=end)


def _gen_again_truth(g, depth):
    """Mostly `==` / `!=` (between two expressions, often the same one, or against a non-expression), which have a
    truth value; sometimes any other expression, which raises."""
    a = _again_expr(g, 2, True)
    if g.rng.random() < 0.15:
        return {"op": "again_truth", "x": a}
    x = g.rng.random()
    b = a if x < 0.35 else _again_expr(g, 2, True) if x < 0.65 else _other(g)
    if g.rng.random() < 0.3:
        a, b = b, a
    return {"op": "again_truth", "x": {"op": g.rng.choice(["eq", "ne"]), "a": a, "b": b}}


@node("again_truth", "value", gen=_gen_again_truth, weight=0.3)
def again_truth(x):
    """bool(x) for an Again expression x: its truth value if it came from == or != (between two expressions:
    whether their hash_keys are equal; against anything else: False for ==), else ValueError."""
    return bool(x)


def _gen_explode(g, depth):
    return {"op": "explode", "die": g.die(depth), "which": _which(g, depth, none_ok=True), "star": _star(g),
            "depth": g.rng.choice([0, 1, 1, 2, 3, 4, -1 if g.rng.random() < 0.2 else 2]),
            "end": _again_end(g)}


@node("explode", "die", gen=_gen_explode)
def explode(die, which, star, depth, end):
    """die.explode(which, star=star, depth=depth, end=end): the selected outcomes (None: the max) roll again and add,
    at most depth more times; past that the next roll counts as end (None: a zero; Reroll: reroll it)."""
    return die.explode(which, star=star, depth=depth, end=end)


@node("Restart", "restart")
def restart_const():
    """icepool.Restart"""
    return icepool.Restart


def _additive_expr(g, depth):
    """Mostly additive expressions (what `again_count` accepts): `Again`, `+` with a small int, die or another
    expression, `k @ Again` (its `_again_count` is `k @ 1`, a TypeError for an int k) and `d @ Again`; sometimes
    one that is not additive (`-`, `*`, `Again @ d`, a die with negative outcomes on the left of `@`)."""
    if depth <= 0 or g.rng.random() < 0.35:
        return {"op": "Again"}
    inner = _additive_expr(g, depth - 1)
    r = g.rng.random()
    if r < 0.5:
        other = _small(g) if g.rng.random() < 0.7 else {"op": "d", "sides": g.rng.randint(1, 3)}
        a, b = (inner, other) if g.rng.random() < 0.5 else (other, inner)
        return {"op": "add", "a": a, "b": b}
    if r < 0.72:
        return {"op": "add", "a": inner, "b": _additive_expr(g, depth - 1)}
    # The rest raise in count mode: `_again_count` passes 0 for every argument that is not an expression, so any
    # `@` is `int @ int` (TypeError), and the others are not additive (ValueError).
    if r < 0.78:
        # `d @ Again`: additive iff the die's min outcome is >= 0.
        n = {"op": "d", "sides": 2} if g.rng.random() < 0.6 else {"op": "sub", "a": {"op": "d", "sides": 2},
                                                                   "b": g.rng.randint(0, 2)}
        return {"op": "matmul", "n": n, "die": inner}
    if r < 0.82:
        return {"op": "matmul", "n": g.rng.randint(0, 2), "die": inner}
    if r < 0.86:
        return {"op": "matmul", "n": inner, "die": {"op": "d", "sides": 2}}
    if r < 0.93:
        return {"op": g.rng.choice(["sub", "mul"]), "a": inner, "b": _small(g)}
    return {"op": "neg", "a": inner}


def _finite_die(g):
    """A die program without inf or NaN outcomes. In count mode the zero outcome of a die holding inf is NaN, and
    Python's tuples holding the *same* NaN object compare equal (identity), so how the step states merge depends on
    object identity (the port says unimplemented). Asks the oracle, like pool.py's _die_without_nan."""
    for _ in range(10):
        p = g.die(0)
        try:
            die = peek(p)
            if isinstance(die, icepool.Die) and any(_has_inf_or_nan(o) for o in die.outcomes()):
                continue
        except Exception:  # noqa: BLE001 - a die that raises is a fine program too
            pass
        return p
    return {"op": "d", "sides": g.rng.randint(1, 6)}


def _has_inf_or_nan(x):
    if isinstance(x, float):
        return not math.isfinite(x)
    return isinstance(x, tuple) and any(_has_inf_or_nan(y) for y in x)


def _gen_again_count(g, depth):
    n = g.rng.randint(1, 4)
    outcomes = []
    for _ in range(n):
        r = g.rng.random()
        if r < 0.45:
            outcomes.append(_additive_expr(g, 2))
        elif r < 0.55:
            outcomes.append(_finite_die(g))
        elif r < 0.6:
            outcomes.append({"op": "Reroll"})
        else:
            outcomes.append(g.outcome())
    count = g.rng.choice([0, 1, 1, 2, 2, 3, -1 if g.rng.random() < 0.2 else 1])
    # Both given raises ValueError (when there is an Again).
    depth_arg = None if g.rng.random() < 0.95 else 1
    end = _again_end(g) if g.rng.random() < 0.8 else {"op": "Restart"}
    return {"op": "again_count", "outcomes": outcomes, "count": count, "depth": depth_arg, "end": end}


@node("again_count", "die", gen=_gen_again_count, weight=0.7)
def again_count(outcomes, count, depth, end):
    """icepool.Die(outcomes, again_count=count, again_depth=depth, again_end=end): outcomes may be additive Again
    expressions, rolled again up to count times in total; past that, Again is end (None: a zero; Reroll: the
    non-Again outcomes; Restart: the whole roll restarts)."""
    return icepool.Die(outcomes, again_count=count, again_depth=depth, again_end=end)
