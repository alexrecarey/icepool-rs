"""@multiset_function (evaluator/multiset_function.py, expression/multiset_parameter.py,
expression/multiset_tuple_expression.py): functions of multisets traced with parameters, and indexing multiset
tuple expressions (a MultiDeal's hands). Same rules as basic.py: the Python side is a direct call into icepool;
generators keep programs small.

A function is `mf_def`: `params` positional parameters (plus `*rest` if `var`) and a lazy `body` program, run each
time icepool traces the function. Inside the body `mf_var` is the function's `i`-th argument (positional
parameters first, then `*rest`): a MultisetParameter, or a MultisetTupleParameter for an unstarred MultiDeal.
Variables always belong to the innermost function being traced. `mf_call` calls a function on inputs.
"""
import icepool

from harness.core.grammar_api import evaluate, node
from harness.grammar.pool import fresh_caches

# The arguments of the bodies being traced, innermost last.
_VARS: list = []


def _make_function(params, var, body):
    """A real Python function `f(p0, ..., [*rest])` (icepool reads its signature) that evaluates `body` with its
    arguments as the variables."""
    names = [f"p{i}" for i in range(params)] + (["*rest"] if var else [])
    args = "(" + "".join(f"p{i}, " for i in range(params)) + ")" + (" + rest" if var else "")
    scope = {"_run": lambda a: _run_body(body, a)}
    exec(f"def wrapped({', '.join(names)}):\n    return _run({args})\n", scope)  # noqa: S102
    return scope["wrapped"]


def _run_body(body, args):
    _VARS.append(args)
    try:
        return evaluate(body)
    finally:
        _VARS.pop()


@node("mf_def", "mfunc", lazy=("body",))
def mf_def(params, var, body):
    """icepool.multiset_function(f) for f(p0, ..., p{params-1}[, *rest]) returning the program `body`. Its repr
    holds an address: no generator (mf_call makes them)."""
    return icepool.multiset_function(_make_function(params, var, body))


@node("mf_var", "mexpr")
def mf_var(i):
    """The i-th argument of the innermost function body being traced (IndexError past the end)."""
    return _VARS[-1][i]


@node("mx_tuple_index", "mexpr")
def mx_tuple_index(expr, index):
    """expr[index] for a multiset tuple expression (a MultiDeal, a tuple parameter or a slice of one): an int makes
    a MultisetTupleIndex (one hand); any other index makes __getitem__ return None."""
    return expr[index]


@node("mx_tuple_slice", "mexpr")
def mx_tuple_slice(expr, start, stop, step):
    """expr[start:stop:step] for a multiset tuple expression: a MultisetTupleSlice (some of the hands)."""
    return expr[start:stop:step]


# --- Generating functions ---------------------------------------------------------------------------------------

def _tuple_len(program):
    """len() of a generated MultiDeal program (its number of hands), or None for a multiset (or a list)."""
    if isinstance(program, dict):
        if program.get("op") == "multi_deal":
            sizes = program["hand_sizes"]
            return len(sizes["tuple"] if isinstance(sizes, dict) else sizes)
        if program.get("op") == "deal_groups":
            return sum(group for _, group in program["hand_groups"])
    return None


def _input(g, depth):
    """An input to a function: a pool or another expression, a list of outcomes, now and then a MultiDeal."""
    from harness.grammar.deck import gen_multi_deal
    from harness.grammar.pool import gen_pool
    r = g.rng.random()
    if r < 0.25:
        return gen_multi_deal(g, 0)
    if r < 0.33:
        return [g.outcome() for _ in range(g.rng.randint(0, 3))]
    return gen_pool(g, min(depth, 1), deals=True)


def _var(i):
    return {"op": "mf_var", "i": i}


def _single(g, kinds, i):
    """Variable i as a multiset expression: a hand of a tuple variable (now and then a bad index)."""
    k = kinds[i]
    if k is None:
        return _var(i)
    index = g.rng.randrange(k) if k and g.rng.random() < 0.95 else g.rng.choice([-1, k, 5])
    if g.rng.random() < 0.15:
        start = g.rng.choice([None, 0, 1, -1])
        stop = g.rng.choice([None, 1, 2, -1])
        sliced = {"op": "mx_tuple_slice", "expr": _var(i), "start": start, "stop": stop, "step": None}
        return {"op": "mx_tuple_index", "expr": sliced, "index": g.rng.choice([0, 0, 1, -1])}
    return {"op": "mx_tuple_index", "expr": _var(i), "index": index}


def _body_expr(g, depth, kinds):
    """A multiset expression over the variables (and now and then a pool of the body's own)."""
    from harness.grammar.multiset import gen_expression
    from harness.grammar.pool import gen_pool

    def leaf(g, depth):
        if not kinds or g.rng.random() < 0.15:
            return gen_pool(g, 0)
        return _single(g, kinds, g.rng.randrange(len(kinds)))
    return gen_expression(g, depth, g.rng.choice([0, 0, 1, 1, 2]), leaf)


_METHODS = ["sum", "size", "empty", "any", "product_of_counts", "highest_outcome_and_count", "largest_count",
            "largest_count_and_outcome", "expand", "all_counts", "largest_straight", "all_straights"]


def _evaluation(g, depth, kinds, nested):
    """One result of a body: mostly an evaluation over the variables."""
    from harness.catalogue.evaluators import ENTRIES
    r = g.rng.random()
    if r < 0.5:
        return {"op": "mx_evaluate", "expr": _body_expr(g, depth, kinds), "method": {"str": g.rng.choice(_METHODS)}}
    if r < 0.72:
        name = g.rng.choice(["record", "record", "count_eq", "count_diff", "final_die", "seen", "reroll_at_least"])
        e = ENTRIES[name]
        n = e.inputs(g) if name != "record" else g.rng.choice([1, 1, 2])
        args = []
        for _ in range(n):
            # The evaluators written for any count see a tuple variable whole, too.
            tuples = [i for i, k in enumerate(kinds) if k is not None]
            if tuples and name == "record" and g.rng.random() < 0.3:
                args.append(_var(g.rng.choice(tuples)))
            else:
                args.append(_body_expr(g, depth, kinds))
        return {"op": "evaluate", "name": {"str": name}, "params": e.params(g), "args": args}
    if r < 0.76:
        return {"op": "pool_issubset", "pool": _body_expr(g, depth, kinds), "other": _body_expr(g, depth, kinds)}
    if r < 0.8:
        return {"op": "pool_count_subset", "pool": _body_expr(g, depth, kinds),
                "divisor": _body_expr(g, depth, kinds), "empty_divisor": None}
    if r < 0.86:
        return {"op": "keep_evaluate", "pool": _body_expr(g, depth, kinds), "index": g.rng.choice([None, 0, 1, -1])}
    if r < 0.93 and nested:
        # A nested function over expressions of the variables.
        n = g.rng.choice([1, 1, 2])
        args = [_body_expr(g, depth, kinds) for _ in range(n)]
        return _call(g, depth, args, [None] * n, nested=False)
    # A constant: a die (or now and then a plain value), not an evaluation.
    return g.die(0) if g.rng.random() < 0.8 else g.rng.randint(0, 3)


def _body(g, depth, kinds, nested=True):
    """A function body: one result, or now and then a joint result (a list of them, or a non-sequence)."""
    r = g.rng.random()
    if r < 0.7:
        return _evaluation(g, depth, kinds, nested)
    if r < 0.97:
        return [_evaluation(g, depth, kinds, nested) for _ in range(g.rng.choice([1, 2, 2, 3]))]
    # Odd results: an expression without an evaluation (TypeError), an empty joint result (ValueError), an int
    # (TypeError: not iterable).
    return g.rng.choice([_body_expr(g, depth, kinds), [], 5])


def _call(g, depth, args, kinds, nested=True):
    """mf_call of a fresh function on `args` (whose tuple lengths are `kinds`). Mostly the function's arity fits
    its arguments (unpacking the first if it is starred), and `star` is left to Python's inference when that agrees
    with the body's view of its variables, else given."""
    tuple_first = bool(kinds) and kinds[0] is not None
    star = None if g.rng.random() < 0.7 else {"bool": g.rng.random() < (0.7 if tuple_first else 0.1)}
    starred = tuple_first and (star == {"bool": True} or star is None and g.rng.random() < 0.5)
    var_kinds = ([None] * kinds[0] + kinds[1:]) if starred else list(kinds)
    params = len(var_kinds)
    var = g.rng.random() < 0.15
    if var and params:
        params = g.rng.randint(0, params)
    if g.rng.random() < 0.04:
        params = max(params + g.rng.choice([-1, 1]), 0)  # TypeError from the arity
    if star is None and (params > len(args)) != bool(starred):
        star = {"bool": bool(starred)}
    f = {"op": "mf_def", "params": params, "var": {"bool": var}, "body": _body(g, depth, var_kinds, nested)}
    return {"op": "mf_call", "f": f, "args": args, "star": star}


def _gen_mf_call(g, depth):
    n = g.rng.choice([1, 1, 1, 2, 2, 3])
    args = [_input(g, depth) for _ in range(n)]
    kinds = [_tuple_len(a) for a in args]
    return _call(g, depth, args, kinds)


@node("mf_call", "die", gen=_gen_mf_call, weight=1.0)
def mf_call(f, args, star):
    """f(*args), or f(*args, star=star) unless star is None: the body traced with parameters, then evaluated."""
    fresh_caches()
    if star is None:
        return f(*args)
    return f(*args, star=star)


def _gen_tuple_evaluate(g, depth):
    """A hand (or a slice of the hands) of a MultiDeal, evaluated outside any function."""
    from harness.grammar.deck import gen_multi_deal
    md = gen_multi_deal(g, depth)
    k = _tuple_len(md) or 0
    index = g.rng.randrange(k) if k and g.rng.random() < 0.9 else g.rng.choice([-1, k, 2])
    e = {"op": "mx_tuple_index", "expr": md, "index": index}
    if g.rng.random() < 0.2:
        sliced = {"op": "mx_tuple_slice", "expr": md, "start": g.rng.choice([None, 1, -1]),
                  "stop": g.rng.choice([None, 2, -1]), "step": g.rng.choice([None, None, -1])}
        e = {"op": "mx_tuple_index", "expr": sliced, "index": g.rng.choice([0, 1, -1])}
    if g.rng.random() < 0.4:
        from harness.grammar.multiset import gen_expression
        e = gen_expression(g, depth, 1, lambda g, depth: e)
    return {"op": "mx_tuple_evaluate", "expr": e, "method": {"str": g.rng.choice(_METHODS)}}


@node("mx_tuple_evaluate", "die", gen=_gen_tuple_evaluate, weight=0.5)
def mx_tuple_evaluate(expr, method):
    """getattr(expr, method)() (as mx_evaluate) for an expression over a hand of a MultiDeal, outside any
    function."""
    fresh_caches()
    return getattr(expr, method)()
