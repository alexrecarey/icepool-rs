"""@multiset_function (evaluator/multiset_function.py, expression/multiset_parameter.py,
expression/multiset_tuple_expression.py): functions of multisets traced with parameters, and indexing multiset
tuple expressions (a MultiDeal's hands). Same rules as basic.py: the Python side is a direct call into icepool;
generators keep programs small.

A function is `mf_def`: `params` positional parameters (plus `*rest` if `var`, plus keyword-only parameters named
in `kwonly`) and a lazy `body` program, run each time icepool traces the function. Inside the body `mf_var` is the
function's `i`-th argument (positional parameters first, then `*rest`): a MultisetParameter, or a
MultisetTupleParameter for an unstarred MultiDeal; `mf_kwarg` is a keyword argument, passed as it is. Variables
always belong to the innermost function being traced. `mf_call` calls a function on inputs, `mf_call_kw` with
keyword arguments.
"""
import icepool

from harness.core.grammar_api import evaluate, node
from harness.grammar.pool import fresh_caches, straight_safe_method

# The (positional, keyword) arguments of the bodies being traced, innermost last.
_VARS: list = []


def _make_function(params, var, body, kwonly):
    """A real Python function `f(p0, ..., [*rest], [*,] k...)` (icepool reads its signature) that evaluates `body`
    with its arguments as the variables."""
    names = [f"p{i}" for i in range(params)] + (["*rest"] if var else ["*"] if kwonly else []) + list(kwonly)
    args = "(" + "".join(f"p{i}, " for i in range(params)) + ")" + (" + rest" if var else "")
    kwargs = "{" + "".join(f"{k!r}: {k}, " for k in kwonly) + "}"
    scope = {"_run": lambda a, kw: _run_body(body, a, kw)}
    exec(f"def wrapped({', '.join(names)}):\n    return _run({args}, {kwargs})\n", scope)  # noqa: S102
    return scope["wrapped"]


def _run_body(body, args, kwargs):
    _VARS.append((args, kwargs))
    try:
        return evaluate(body)
    finally:
        _VARS.pop()


@node("mf_def", "mfunc", lazy=("body",))
def mf_def(params, var, body, kwonly=()):
    """icepool.multiset_function(f) for f(p0, ..., p{params-1}[, *rest][, *, k0, k1...]) returning the program
    `body`; `kwonly` (the names k0, k1...) may be left out. Its repr holds an address: no generator (mf_call and
    mf_call_kw make them)."""
    return icepool.multiset_function(_make_function(params, var, body, kwonly))


@node("mf_var", "mexpr")
def mf_var(i):
    """The i-th argument of the innermost function body being traced (IndexError past the end)."""
    return _VARS[-1][0][i]


@node("mf_kwarg", "value")
def mf_kwarg(name):
    """Keyword argument `name` of the innermost function body being traced (KeyError if it has none)."""
    return _VARS[-1][1][name]


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


def _body_expr(g, depth, kinds, kwonly=()):
    """A multiset expression over the variables (and now and then a pool of the body's own), now and then using a
    keyword argument."""
    from harness.grammar.multiset import gen_expression
    from harness.grammar.pool import gen_pool

    def leaf(g, depth):
        if not kinds or g.rng.random() < 0.15:
            return gen_pool(g, 0)
        return _single(g, kinds, g.rng.randrange(len(kinds)))
    e = gen_expression(g, depth, g.rng.choice([0, 0, 1, 1, 2]), leaf)
    if kwonly and g.rng.random() < 0.5:
        e = _with_kwarg(g, e, kwonly)
    return e


def _kwarg(name):
    return {"op": "mf_kwarg", "name": {"str": name}}


def _with_kwarg(g, e, kwonly):
    """An operation on `e` taking a keyword argument of the function as its parameter (a `target` is a collection
    of outcomes, an `n` an int)."""
    name = g.rng.choice(kwonly)
    if name == "target" or g.rng.random() < 0.1:
        return {"op": "mx_filter", "expr": e, "invert": {"bool": g.rng.random() < 0.5}, "target": _kwarg(name)}
    o = g.rng.choice(["mul", "unique", "keep_counts", "floordiv", "multiply_counts"])
    comparison = {"str": g.rng.choice(["==", "!=", "<=", "<", ">=", ">"])}
    return {"op": "mx_counts", "expr": e, "o": {"str": o}, "comparison": comparison, "n": _kwarg(name)}


# The keyword-only parameters a function may have (`star` is taken by icepool: it never reaches the body).
_KW_NAMES = ["target", "n", "star"]


def _kw_value(g, name):
    if name == "target":
        return [g.outcome() for _ in range(g.rng.randint(0, 3))]
    if name == "star":
        return {"bool": g.rng.random() < 0.5}
    return g.rng.choice([0, 1, 1, 2, 3, -1])


_METHODS = ["sum", "size", "empty", "any", "product_of_counts", "highest_outcome_and_count", "largest_count",
            "largest_count_and_outcome", "expand", "all_counts", "largest_straight", "all_straights"]


def _evaluation(g, depth, kinds, nested, kwonly=()):
    """One result of a body: mostly an evaluation over the variables."""
    from harness.catalogue.evaluators import ENTRIES
    if kinds and g.rng.random() < 0.06:
        return _param_str(g, depth, kinds)
    r = g.rng.random()
    body_expr = lambda: _body_expr(g, depth, kinds, kwonly)  # noqa: E731
    tuples = [i for i, k in enumerate(kinds) if k is not None]

    def counts_expr():
        # Now and then operators over whole tuple variables: the built-in evaluations see tuple counts (S5d).
        if tuples and g.rng.random() < 0.2:
            return tuple_count_expression(g, depth, lambda: _var(g.rng.choice(tuples)))
        return body_expr()
    if r < 0.5:
        expr = counts_expr()
        return {"op": "mx_evaluate", "expr": expr, "method": {"str": straight_safe_method(g, expr, _METHODS)}}
    if r < 0.72:
        name = g.rng.choice(["record", "record", "count_eq", "count_diff", "final_die", "seen", "reroll_at_least"])
        e = ENTRIES[name]
        n = e.inputs(g) if name != "record" else g.rng.choice([1, 1, 2])
        args = []
        for _ in range(n):
            # The evaluators written for any count see a tuple variable whole too, or operators over it (tuple
            # counts).
            if tuples and g.rng.random() < (0.3 if name == "record" else 0.1):
                t = lambda: _var(g.rng.choice(tuples))  # noqa: E731
                args.append(t() if g.rng.random() < 0.5 else tuple_count_expression(g, depth, t))
            else:
                args.append(body_expr())
        return {"op": "evaluate", "name": {"str": name}, "params": e.params(g), "args": args}
    if r < 0.76:
        return {"op": "pool_issubset", "pool": counts_expr(), "other": counts_expr()}
    if r < 0.8:
        return {"op": "pool_count_subset", "pool": counts_expr(), "divisor": counts_expr(), "empty_divisor": None}
    if r < 0.86:
        return {"op": "keep_evaluate", "pool": counts_expr(), "index": g.rng.choice([None, 0, 1, -1])}
    if r < 0.93 and nested:
        # A nested function over expressions of the variables (its keyword arguments now and then this
        # function's own).
        n = g.rng.choice([1, 1, 2])
        args = [body_expr() for _ in range(n)]
        return _call(g, depth, args, [None] * n, nested=False, kw=g.rng.random() < 0.3, outer_kwonly=kwonly)
    if "n" in kwonly and r < 0.96:
        # A keyword argument as the result: a constant (an int: a list in a joint result would make tuple
        # outcomes holding lists, which the port does not model).
        return _kwarg("n")
    # A constant: a die (or now and then a plain value), not an evaluation.
    return g.die(0) if g.rng.random() < 0.8 else g.rng.randint(0, 3)


def _param_str(g, depth, kinds):
    """str() of an expression over the variables (a constant result: a str): a parameter's str is its name (`p0`,
    `rest[2]`), a tuple variable's too. Not a hand of a tuple variable: MultisetTupleIndex has no __str__."""
    from harness.grammar.multiset import gen_str_expression
    from harness.grammar.pool import gen_pool

    def leaf(g, depth):
        if g.rng.random() < 0.1:
            return gen_pool(g, 0)
        return _var(g.rng.randrange(len(kinds)))
    return {"op": "mx_str", "expr": gen_str_expression(g, depth, g.rng.choice([0, 0, 1, 2]), leaf)}


def _body(g, depth, kinds, nested=True, kwonly=()):
    """A function body: one result, or now and then a joint result (a list of them, or a non-sequence)."""
    r = g.rng.random()
    if r < 0.7:
        return _evaluation(g, depth, kinds, nested, kwonly)
    if r < 0.97:
        return [_evaluation(g, depth, kinds, nested, kwonly) for _ in range(g.rng.choice([1, 2, 2, 3]))]
    # Odd results: an expression without an evaluation (TypeError), an empty joint result (ValueError), an int
    # (TypeError: not iterable).
    return g.rng.choice([_body_expr(g, depth, kinds), [], 5])


def _call(g, depth, args, kinds, nested=True, kw=False, outer_kwonly=()):
    """mf_call of a fresh function on `args` (whose tuple lengths are `kinds`). Mostly the function's arity fits
    its arguments (unpacking the first if it is starred), and `star` is left to Python's inference when that agrees
    with the body's view of its variables, else given. With `kw`, the function has keyword-only parameters and
    mf_call_kw passes them (mostly exactly those; a value is now and then a keyword argument of the function being
    traced, `outer_kwonly`)."""
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
    kwonly = []
    if kw:
        kwonly = g.rng.sample(_KW_NAMES[:2], g.rng.choice([1, 1, 2])) if g.rng.random() < 0.97 else ["star"]
    body = _body(g, depth, var_kinds, nested, [k for k in kwonly if k != "star"])
    f = {"op": "mf_def", "params": params, "var": {"bool": var}, "body": body}
    if not kw:
        return {"op": "mf_call", "f": f, "args": args, "star": star}
    f["kwonly"] = [{"str": k} for k in kwonly]
    names = [k for k in kwonly if k != "star"]
    if g.rng.random() < 0.05 and names:
        names.remove(g.rng.choice(names))  # TypeError: a missing keyword argument
    if g.rng.random() < 0.05:
        names.append(g.rng.choice(_KW_NAMES[:2]))  # TypeError: an unexpected one (or the same one twice)
    kwargs = [[{"str": k}, _kwarg(k) if k in outer_kwonly and g.rng.random() < 0.5 else _kw_value(g, k)]
              for k in names]
    if star is not None:
        kwargs.insert(g.rng.randint(0, len(kwargs)), [{"str": "star"}, star])
    return {"op": "mf_call_kw", "f": f, "args": args, "kwargs": kwargs}


def _gen_mf_call(g, depth):
    # Now and then no arguments at all: no multiset parameters, the body's evaluations over its own pools.
    n = g.rng.choice([0, 1, 1, 1, 2, 2, 3])
    args = [_input(g, depth) for _ in range(n)]
    kinds = [_tuple_len(a) for a in args]
    return _call(g, depth, args, kinds)


def _gen_mf_call_kw(g, depth):
    n = g.rng.choice([0, 1, 1, 1, 2, 2, 3])
    args = [_input(g, depth) for _ in range(n)]
    kinds = [_tuple_len(a) for a in args]
    return _call(g, depth, args, kinds, kw=True)


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
    return {"op": "mx_tuple_evaluate", "expr": e, "method": {"str": straight_safe_method(g, e, _METHODS)}}


@node("mf_call_kw", "die", gen=_gen_mf_call_kw, weight=0.5)
def mf_call_kw(f, args, kwargs):
    """f(*args, **dict(kwargs)) for a list of [name, value] pairs: keyword arguments for the function's keyword-only
    parameters (passed to the body as they are), and `star`."""
    fresh_caches()
    return f(*args, **{k: v for k, v in kwargs})


@node("mx_tuple_evaluate", "die", gen=_gen_tuple_evaluate, weight=0.5)
def mx_tuple_evaluate(expr, method):
    """getattr(expr, method)() (as mx_evaluate) for an expression over a hand of a MultiDeal, outside any
    function."""
    fresh_caches()
    return getattr(expr, method)()


# --- S4f: multiset operators over whole multiset tuple expressions ---------------------------------------------

def tuple_count_expression(g, depth, tuple_leaf):
    """A multiset expression over whole multiset tuple expressions (`tuple_leaf()` makes one: a MultiDeal, a tuple
    variable): its counts are tuples, which Python's operators compute with as it does with any tuple (`min` /
    `max` / `+` of two tuples, `tuple * n`, `==`), mostly raising TypeError. Then now and then more operators on
    top (`gen_expression`, whose other operands are pools or more of these)."""
    from harness.catalogue.functions import fn_node
    from harness.grammar.multiset import gen_expression
    from harness.grammar.pool import gen_pool
    r = g.rng

    def operand():
        k = r.random()
        if k < 0.6:
            return tuple_leaf()
        if k < 0.85:
            return gen_pool(g, 0)
        return [g.outcome() for _ in range(r.randint(0, 2))]

    def base(g, depth):
        k = r.random()
        if k < 0.45:
            # One child: `reduce` passes its tuple counts through; two tuples: min / max / concatenated / TypeError.
            name = r.choice(["union", "intersection", "difference", "additive_union", "union", "intersection"])
            args = [tuple_leaf()] + [operand() for _ in range(r.choice([0, 0, 1, 1, 2]))]
            r.shuffle(args)
            return {"op": "mx_method", "name": {"str": name}, "args": args,
                    "keep_negative_counts": {"bool": r.random() < 0.3}}
        if k < 0.65:
            # A pool (or a list, or another tuple expression) and a tuple expression: the reflected operator if
            # the tuple expression is on the left.
            a, b = tuple_leaf(), operand()
            if r.random() < 0.5:
                a, b = b, a
            return {"op": "mx_binary", "a": a, "o": {"str": r.choice(["+", "-", "&", "|", "^"])}, "b": b}
        if k < 0.85:
            f = fn_node(g, name=r.choice(["count_max", "count_max", "count_if_gt_k", "count_diff"]))[0]
            return {"op": "mx_map_counts", "args": [tuple_leaf()] + [operand() for _ in range(r.choice([0, 1]))],
                    "function": f}
        if k < 0.93:
            return gen_pool(g, 0)
        # The tuple expression itself: an operator's other operand, or its methods (AttributeError).
        return tuple_leaf()
    if r.random() < 0.4:
        return base(g, depth)
    return gen_expression(g, depth, r.choice([1, 1, 2]), base)


def _gen_tuple_op_evaluate(g, depth):
    from harness.catalogue.evaluators import ENTRIES
    from harness.grammar.deck import gen_multi_deal

    def tuple_leaf():
        md = gen_multi_deal(g, 0)
        if g.rng.random() < 0.1:
            return {"op": "mx_tuple_slice", "expr": md, "start": g.rng.choice([None, 1, -1]),
                    "stop": g.rng.choice([None, 2, -1]), "step": None}
        return md
    # Evaluators written for any count (the built-in ones are not: they see a tuple where they expect an int).
    name = g.rng.choice(["record", "record", "record", "hand_sums", "count_eq", "seen", "final_die"])
    e = ENTRIES[name]
    n = e.inputs(g) if name != "record" else g.rng.choice([1, 1, 2])
    args = [tuple_count_expression(g, depth, tuple_leaf) for _ in range(n)]
    return {"op": "tuple_op_evaluate", "name": {"str": name}, "params": e.params(g), "args": args}


@node("tuple_op_evaluate", "die", gen=_gen_tuple_op_evaluate, weight=0.5)
def tuple_op_evaluate(name, params, args):
    """EVALUATORS[name](*params).evaluate(*args) (as evaluate) where the args are multiset operators applied to
    whole multiset tuple expressions (MultiDeals): the operators get tuple counts and sizes."""
    from harness.catalogue.evaluators import EVALUATORS
    fresh_caches()
    return EVALUATORS[name](*params).evaluate(*args)


# --- S5d: the built-in evaluators over tuple counts --------------------------------------------------------------

def _no_args(g):
    return [], [], []


def _straight_and_outcome_args(g):
    return [{"str": g.rng.choice(["high", "low"])}], [], []


def _wild_args(g):
    def which():
        return None if g.rng.random() < 0.4 else [g.outcome() for _ in range(g.rng.randint(0, 2))]
    return [which(), which(), which()], [], []


def _all_counts_args(g):
    return [], [[{"str": "filter"}, g.rng.choice([1, 1, 2, 0, -1, {"str": "all"}])]], []


def _count_subset_args(g):
    return [], [[{"str": "empty_divisor"}, g.rng.choice([None, 0, 2])]], []


def _empty_args(g):
    return [], [], ([[{"str": "any"}, {"bool": g.rng.random() < 0.5}]] if g.rng.random() < 0.7 else [])


def _expand_args(g):
    return [], [], ([[{"str": "order"}, g.rng.choice([1, -1])]] if g.rng.random() < 0.7 else [])


def _keep_args(g):
    return [], [], [[{"str": "index"}, g.rng.choice([None, 0, 1, 2, -1, -2])]]


# Built-in evaluator class -> (number of inputs, generator of (constructor args, constructor kwargs, evaluate
# kwargs)).
_BUILTIN_EVALUATORS = {
    "SumEvaluator": (1, _no_args),
    "SizeEvaluator": (1, _no_args),
    "ProductOfCountsEvaluator": (1, _no_args),
    "EmptyEvaluator": (1, _empty_args),
    "ExpandEvaluator": (1, _expand_args),
    "KeepEvaluator": (1, _keep_args),
    "HighestOutcomeAndCountEvaluator": (1, _no_args),
    "AllCountsEvaluator": (1, _all_counts_args),
    "LargestCountEvaluator": (1, _no_args),
    "LargestCountWithWildEvaluator": (1, _wild_args),
    "LargestCountAndOutcomeEvaluator": (1, _no_args),
    "LargestStraightEvaluator": (1, _no_args),
    "LargestStraightAndOutcomeEvaluator": (1, _straight_and_outcome_args),
    "AllStraightsEvaluator": (1, _no_args),
    "AllStraightsReduceCountsEvaluator": (1, _no_args),
    "CountSubsetEvaluator": (2, _count_subset_args),
    "IsProperSubsetEvaluator": (2, _no_args),
    "IsSubsetEvaluator": (2, _no_args),
    "IsProperSupersetEvaluator": (2, _no_args),
    "IsSupersetEvaluator": (2, _no_args),
    "IsEqualSetEvaluator": (2, _no_args),
    "IsNotEqualSetEvaluator": (2, _no_args),
    "IsDisjointSetEvaluator": (2, _no_args),
}

_STRAIGHT_EVALUATORS = {"LargestStraightEvaluator", "LargestStraightAndOutcomeEvaluator", "AllStraightsEvaluator",
                        "AllStraightsReduceCountsEvaluator"}


def _gen_builtin_evaluate(g, depth):
    from harness.grammar.deck import gen_multi_deal
    from harness.grammar.pool import gen_pool, huge_range

    def tuple_leaf():
        md = gen_multi_deal(g, 0)
        if g.rng.random() < 0.1:
            return {"op": "mx_tuple_slice", "expr": md, "start": g.rng.choice([None, 1, -1]),
                    "stop": g.rng.choice([None, 2, -1]), "step": None}
        return md

    def arg():
        # Mostly a whole MultiDeal (tuple counts and sizes), or operators over them; now and then a pool, so that
        # two-input evaluators compare tuple counts with int ones.
        k = g.rng.random()
        if k < 0.45:
            return tuple_leaf()
        if k < 0.85:
            return tuple_count_expression(g, depth, tuple_leaf)
        return gen_pool(g, 0)
    name = g.rng.choice(sorted(_BUILTIN_EVALUATORS))
    inputs, gen_params = _BUILTIN_EVALUATORS[name]
    args = [arg() for _ in range(inputs)]
    if name in _STRAIGHT_EVALUATORS and huge_range(args):
        # `consecutive` over a huge span of outcomes (pool.py `huge_range`).
        name, inputs, gen_params = "SumEvaluator", 1, _no_args
        args = args[:1]
    params, ctor_kwargs, kwargs = gen_params(g)
    return {"op": "builtin_evaluate", "evaluator": {"str": name}, "params": params, "ctor_kwargs": ctor_kwargs,
            "kwargs": kwargs, "args": args}


@node("builtin_evaluate", "die", gen=_gen_builtin_evaluate, weight=0.5)
def builtin_evaluate(evaluator, params, ctor_kwargs, kwargs, args):
    """getattr(icepool.evaluator, evaluator)(*params, **dict(ctor_kwargs)).evaluate(*args, **dict(kwargs)): a
    built-in evaluator class, mostly over MultiDeals or operators over them, so it sees tuple counts and sizes
    (Python's arithmetic just runs on them: `outcome * count` repeats a tuple, `count >= 1` is TypeError...)."""
    fresh_caches()
    ev = getattr(icepool.evaluator, evaluator)(*params, **{k: v for k, v in ctor_kwargs})
    return ev.evaluate(*args, **{k: v for k, v in kwargs})
