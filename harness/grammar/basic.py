"""Grammar v0: dice and arithmetic. The loop extends this package as features land (one module per area is fine).

Rules for every node (the reviewer checks them):
  - The Python side is a direct call into icepool: no logic of its own beyond unpacking arguments.
  - The generator keeps programs small enough that the oracle answers in well under a second.
  - The Rust side lives in crates/icepool-harness/src/ and must produce identical results.
"""
import icepool

from harness.core.grammar_api import node


def _gen_d(g, depth):
    return {"op": "d", "sides": g.rng.randint(1, 12)}


def fresh_dz():
    """Clear the @cache of icepool.d and icepool.z. It is keyed by ==, so in a long-lived oracle process z(True)
    makes a later z(Fraction(1)) (or z(1.0)) return the cached die instead of raising TypeError: the result would
    depend on what the process computed before (DIVERGENCES.md). Called by every node that calls d or z."""
    icepool.d.cache_clear()
    icepool.z.cache_clear()


@node("d", "die", gen=_gen_d, leaf=True, weight=3)
def d(sides):
    """icepool.d(sides): a fair die numbered 1..sides."""
    fresh_dz()
    return icepool.d(sides)


def _gen_die(g, depth):
    n = g.rng.choice([0, 1, 1, 2, 3, 4, 6])
    return {"op": "die", "outcomes": [g.outcome() for _ in range(n)]}


@node("die", "die", gen=_gen_die, leaf=True, weight=2)
def die(outcomes):
    """icepool.Die(list): each listed outcome once; repeats add up; [] is the empty die."""
    return icepool.Die(outcomes)


def _gen_weights(g, depth):
    outs = g.rng.sample(range(-4, 13), g.rng.randint(1, 5))
    if g.rng.random() < 0.04:
        # An `Again` expression as a key (hashed by its hash_key; an equal one merges), rolled again by `Die()`.
        from harness.grammar.again import _again_expr
        outs[0] = _again_expr(g, 1, False)
        if g.rng.random() < 0.3:
            outs.append(outs[0])
    return {"op": "weights", "items": [[o, g.rng.choice([0, 1, 1, 2, 3, 5, 10**20])] for o in outs]}


@node("weights", "die", gen=_gen_weights, leaf=True)
def weights(items):
    """icepool.Die({outcome: quantity}): zero quantities are kept, huge ones are exact; an Again key rolls again."""
    return icepool.Die({o: q for o, q in items})


def _binary(op):
    # Shared with operators.py: the other operand may be a die, an int, or a typed scalar (float, str, ...).
    from harness.grammar.operators import binary
    return binary(op)


@node("add", "die", gen=_binary("add"))
def add(a, b):
    """a + b"""
    return a + b


@node("sub", "die", gen=_binary("sub"))
def sub(a, b):
    """a - b"""
    return a - b


@node("mul", "die", gen=_binary("mul"))
def mul(a, b):
    """a * b"""
    return a * b


@node("neg", "die", gen=lambda g, depth: {"op": "neg", "a": g.die(depth)})
def neg(a):
    """-a"""
    return -a


@node("lt", "die", gen=_binary("lt"), weight=0.5)
def lt(a, b):
    """a < b, a Die of bools"""
    return a < b


@node("ge", "die", gen=_binary("ge"), weight=0.5)
def ge(a, b):
    """a >= b, a Die of bools"""
    return a >= b


def _gen_matmul(g, depth):
    n = g.rng.randint(0, 4) if g.rng.random() < 0.6 else {"op": "die", "outcomes": [g.rng.randint(0, 3) for _ in range(g.rng.randint(1, 3))]}
    return {"op": "matmul", "n": n, "die": g.die(min(depth, 1))}


@node("matmul", "die", gen=_gen_matmul, weight=0.7)
def matmul(n, die):
    """n @ die: roll n (an int or a Die), then sum that many rolls of die."""
    return n @ die
