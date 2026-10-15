"""Multiset expressions (operator/*.py, expression/multiset_expression.py, generator/compound_keep.py): operators
between pools, deals and other expressions. Same rules as basic.py: the Python side is a direct call into icepool;
generators keep programs small.

An expression's repr holds an address, like a pool's, so the expression nodes (kind "mexpr") have no generator of
their own: `gen_expression` builds them inside the evaluating nodes (`mx_evaluate` here, and every evaluation of
evaluate.py through `gen_pool(..., deals=True)`). `pool_add` (pool.py) now also makes compound keeps, and the
keep-like pool nodes (`pool_keep`, `pool_slice`, `pool_highest`, ...) take any expression (a `MultisetKeep`).
"""
import operator

import icepool

from harness.core.grammar_api import node
from harness.grammar.pool import fresh_caches, straight_safe_method

_BINARY = {"+": operator.add, "-": operator.sub, "&": operator.and_, "|": operator.or_, "^": operator.xor}


@node("mx_binary", "mexpr")
def mx_binary(a, o, b):
    """a o b for o in + - & | ^ with a multiset expression on at least one side (the other may be a list of
    outcomes: implicitly a Pool). Pool + pool merges or makes a CompoundKeepGenerator."""
    return _BINARY[o](a, b)


@node("mx_method", "mexpr")
def mx_method(name, args, keep_negative_counts):
    """icepool.MultisetExpression.<name>(*args) for additive_union, intersection, union, and difference (with
    keep_negative_counts=...). No args: an operator without children (TypeError from reduce() once it sees an
    outcome)."""
    f = getattr(icepool.MultisetExpression, name)
    if name == "difference":
        return f(*args, keep_negative_counts=keep_negative_counts)
    return f(*args)


@node("mx_counts", "mexpr")
def mx_counts(expr, o, comparison, n):
    """Count adjustments by o: mul (expr * n), rmul (n * expr), neg (-expr), pos (+expr), floordiv (expr // n), mod
    (expr % n), unique (expr.unique(n)), keep_counts (expr.keep_counts(comparison, n)), multiply_counts,
    divide_counts, modulo_counts (the methods). On a keep generator, * and - change its keep tuple."""
    match o:
        case "mul":
            return expr * n
        case "rmul":
            return n * expr
        case "neg":
            return -expr
        case "pos":
            return +expr
        case "floordiv":
            return expr // n
        case "mod":
            return expr % n
        case "unique":
            return expr.unique(n)
        case "keep_counts":
            return expr.keep_counts(comparison, n)
        case _:
            return getattr(expr, o)(n)


@node("mx_filter", "mexpr")
def mx_filter(expr, invert, target):
    """expr.drop_outcomes(target) if invert else expr.keep_outcomes(target): target is a collection of outcomes, a
    predicate from the catalogue, or another expression (kept where its count is positive)."""
    return expr.drop_outcomes(target) if invert else expr.keep_outcomes(target)


@node("mx_map_counts", "mexpr")
def mx_map_counts(args, function):
    """icepool.MultisetExpression.map_counts(*args, function=function): function(outcome, *counts) per outcome."""
    return icepool.MultisetExpression.map_counts(*args, function=function)


@node("mx_sort_pair", "mexpr")
def mx_sort_pair(expr, method, comparison, other, order, extra):
    """expr.<method>(comparison, other, Order(order), extra) for sort_pair, sort_pair_keep_while and
    sort_pair_drop_while. comparison and extra are only checked when evaluated."""
    return getattr(expr, method)(comparison, other, icepool.Order(order), extra)


@node("mx_max_pair", "mexpr")
def mx_max_pair(expr, keep, comparison, other, priority):
    """expr.max_pair_keep(comparison, other, priority) if keep else expr.max_pair_drop(...)."""
    method = expr.max_pair_keep if keep else expr.max_pair_drop
    return method(comparison, other, priority)


@node("mx_versus", "mexpr")
def mx_versus(expr, all, comparison, other):
    """expr.versus_all(comparison, other) if all else expr.versus_any(comparison, other)."""
    return expr.versus_all(comparison, other) if all else expr.versus_any(comparison, other)


@node("mx_mixture", "mexpr")
def mx_mixture(inners):
    """icepool.MultisetMixture(inners): one evaluation per inner expression (equal ones counted together), the
    results mixed by weight. keep and multiply_counts apply to each inner when all are keep generators."""
    return icepool.MultisetMixture(inners)


@node("mx_force_order", "mexpr")
def mx_force_order(expr, order):
    """expr.force_order(Order(order)): UnsupportedOrder for the other order when evaluated."""
    return expr.force_order(icepool.Order(order))


_METHODS = ["sum", "size", "empty", "any", "product_of_counts", "highest_outcome_and_count", "largest_count",
            "largest_count_and_outcome", "expand", "all_counts", "largest_straight", "all_straights"]


# --- Generating expressions -------------------------------------------------------------------------------------

def _leaf(g, depth):
    """A pool or deal (with keep-like operations) to build on."""
    from harness.grammar.pool import gen_pool
    p = gen_pool(g, depth, negative=g.rng.random() < 0.3, deals=True, expressions=False)
    if g.rng.random() < 0.05:
        # Now and then its weightless() (weightless.py), so every evaluation node meets weightless sources.
        return {"op": "weightless", "generator": p}
    return p


def _outcomes(g):
    return [g.outcome() for _ in range(g.rng.randint(0, 3))]


def _other(g, depth, budget, leaf=None):
    """The other operand of a binary operation: an expression, or (now and then) a list of outcomes."""
    if g.rng.random() < 0.2:
        return _outcomes(g)
    return gen_expression(g, depth, budget, leaf)


def _non_multiset(g):
    """An operand that is not a multiset and cannot be implicitly converted to one (ImplicitConversionError, so the
    expression's operator returns NotImplemented): a number, None, or a die (Die's operator converts the expression
    instead: TypeError). Not Again: the AgainExpression it makes over the expression would be the result of nodes
    that pass a reflected comparison through (mx_compare), and its repr holds an address; fixtures/t1b has those."""
    return g.rng.choice([
        lambda: g.rng.randint(-1, 3), lambda: {"bool": True}, lambda: {"float": "1.5"}, lambda: None,
        lambda: g.die(0)])()


def _comparison(g, choices):
    return {"str": g.rng.choice(choices) if g.rng.random() < 0.95 else "<>"}


def gen_expression(g, depth, budget=None, leaf=None):
    """A small multiset expression: a pool or deal (or what `leaf(g, depth)` makes, e.g. a @multiset_function
    parameter), under up to `budget` operators (each operand counts)."""
    from harness.catalogue.functions import fn_node
    if budget is None:
        budget = g.rng.choice([1, 1, 2, 2, 3])
    if budget <= 0 or g.rng.random() < 0.15:
        e = (leaf or _leaf)(g, depth)
        if g.rng.random() < 0.08:
            # A mixture of leaves, now and then one of them twice (equal inners are counted together).
            inners = [e] + [(leaf or _leaf)(g, depth) for _ in range(g.rng.choice([0, 1, 1, 2]))]
            if g.rng.random() < 0.3:
                inners.append(e)
            e = {"op": "mx_mixture", "inners": inners}
        return e
    r = g.rng
    budget -= 1
    sub = lambda: gen_expression(g, depth, budget // 2, leaf)
    if r.random() < 0.06:
        # A mixture of expressions, now and then one of them twice: inners with equal hash keys (operators of the
        # same class and parameters over equal children; a function is only equal to itself) count as one.
        inners = [sub() for _ in range(r.choice([1, 2, 2, 3]))]
        if r.random() < 0.6:
            inners.insert(r.randint(0, len(inners)), r.choice(inners))
        return {"op": "mx_mixture", "inners": inners}
    k = r.random()
    if k < 0.2:
        op = r.choice(list(_BINARY))
        a, b = sub(), _other(g, depth, budget // 2, leaf)
        x = r.random()
        if x < 0.1:
            # The reflected operator: a list (or now and then a tuple: also a Sequence) on the left.
            a, b = _outcomes(g), sub()
            if r.random() < 0.3:
                a = {"tuple": a}
        elif x < 0.16:
            # An operand that fails implicit conversion, on either side.
            a, b = (sub(), _non_multiset(g)) if r.random() < 0.5 else (_non_multiset(g), sub())
        return _binary(a, op, b)
    if k < 0.3:
        name = r.choice(["additive_union", "difference", "intersection", "union"])
        n = r.choice([1, 2, 2, 3, 0 if r.random() < 0.1 else 2])
        args = [sub() if r.random() < 0.85 else _outcomes(g) for _ in range(n)]
        return {"op": "mx_method", "name": {"str": name}, "args": args,
                "keep_negative_counts": {"bool": r.random() < 0.4}}
    if k < 0.5:
        op = r.choice(["mul", "neg", "pos", "floordiv", "mod", "unique", "unique", "keep_counts", "keep_counts",
                       "multiply_counts", "divide_counts", "modulo_counts"])
        if op == "mul" and r.random() < 0.3:
            op = "rmul"
        n = r.choice([0, 1, 1, 2, 2, 3, -1])
        if op in ("floordiv", "divide_counts", "mod", "modulo_counts") and n == 0 and r.random() < 0.7:
            n = 2
        if op in ("mul", "rmul", "mod") and r.random() < 0.15:
            # Not an int: NotImplemented, and the other operand's reflected operator decides.
            # (Not Again: see _non_multiset.)
            n = r.choice([{"float": "2.0"}, {"str": "ab"}, [1], None, g.die(0)])
        comparison = _comparison(g, ["==", "!=", "<=", "<", ">=", ">"])
        return {"op": "mx_counts", "expr": sub(), "o": {"str": op}, "comparison": comparison, "n": n}
    if k < 0.6:
        t = r.random()
        if t < 0.4:
            target = _outcomes(g)
        elif t < 0.7:
            target = fn_node(g, pred=True)[0]
        else:
            target = sub()
        return {"op": "mx_filter", "expr": sub(), "invert": {"bool": r.random() < 0.5}, "target": target}
    if k < 0.67:
        # Functions of the counts only: their results are ints whatever the outcomes (add2 / max2 over bool or
        # float outcomes would make bool / float counts, which the port does not model; see NOTES.md).
        f = fn_node(g, name=r.choice(["count_diff", "count_if_gt_k"]))[0]
        n = r.choice([1, 2, 2, 3])
        return {"op": "mx_map_counts", "args": [sub() for _ in range(n)], "function": f}
    if k < 0.8:
        return _keep(g, sub())
    if k < 0.88:
        method = r.choice(["sort_pair", "sort_pair_keep_while", "sort_pair_drop_while"])
        extras = ["early", "late", "low", "high", "equal"] + (
            ["keep", "drop"] if method == "sort_pair" else ["continue", "break"])
        return {"op": "mx_sort_pair", "expr": sub(), "method": {"str": method},
                "comparison": _comparison(g, ["==", "!=", "<=", "<", ">=", ">"]), "other": _other(g, depth, budget // 2, leaf),
                "order": r.choice([1, -1, -1, 0]), "extra": {"str": r.choice(extras) if r.random() < 0.95 else "x"}}
    if k < 0.94:
        priority = r.choice([None, None, {"str": "low"}, {"str": "high"}, {"str": "mid"} if r.random() < 0.1 else None])
        return {"op": "mx_max_pair", "expr": sub(), "keep": {"bool": r.random() < 0.5},
                "comparison": _comparison(g, ["==", "<=", "<", ">=", ">"]), "other": _other(g, depth, budget // 2, leaf),
                "priority": priority}
    if k < 0.98:
        return {"op": "mx_versus", "expr": sub(), "all": {"bool": r.random() < 0.5},
                "comparison": _comparison(g, ["<=", "<", ">=", ">"]), "other": _other(g, depth, budget // 2, leaf)}
    return {"op": "mx_force_order", "expr": sub(), "order": r.choice([1, -1, 0])}


def _binary(a, op, b):
    return {"op": "mx_binary", "a": a, "o": {"str": op}, "b": b}


def _keep(g, e):
    """A keep-like operation on an expression (pool.py's nodes, which build a MultisetKeep for these)."""
    r = g.rng
    k = r.random()
    if k < 0.3:
        start, stop = (None if r.random() < 0.35 else r.randint(-4, 4) for _ in range(2))
        return {"op": "pool_slice", "pool": e, "start": start, "stop": stop,
                "step": 1 if r.random() < 0.03 else None}
    if k < 0.55:
        counts = [r.choice([0, 1, 1, 2, -1]) for _ in range(r.randint(0, 3))]
        x = r.random()
        if x < 0.8:
            counts.insert(0 if r.random() < 0.5 else len(counts), {"op": "Ellipsis"})
        elif x < 0.9:
            # Two Ellipses (at both ends, or one inside).
            counts.insert(0, {"op": "Ellipsis"})
            counts.insert(r.randint(1, len(counts)), {"op": "Ellipsis"})
        elif x < 0.95:
            # Neither a slice nor a sequence (TypeError when a MultisetKeep is evaluated; a keep generator
            # iterates it: a small die's outcomes are a keep tuple).
            die = {"op": "d", "sides": r.randint(1, 3)}
            return {"op": "pool_keep", "pool": e, "index": r.choice([None, {"float": "1.5"}, die])}
        return {"op": "pool_keep", "pool": e, "index": counts}
    if k < 0.85:
        return {"op": r.choice(["pool_highest", "pool_lowest"]), "pool": e,
                "keep": None if r.random() < 0.35 else r.randint(0, 3), "drop": None if r.random() < 0.5 else r.randint(0, 2)}
    return {"op": "pool_mul", "pool": e, "n": r.randint(-2, 3)}


def _gen_mx_evaluate(g, depth):
    expr = gen_expression(g, depth)
    return {"op": "mx_evaluate", "expr": expr, "method": {"str": straight_safe_method(g, expr, _METHODS)}}


@node("mx_evaluate", "die", gen=_gen_mx_evaluate, weight=1.0)
def mx_evaluate(expr, method):
    """getattr(expr, method)() for the evaluation methods without arguments (sum, size, expand, all_counts, ...):
    the multiset expression evaluated, one outcome at a time."""
    fresh_caches()
    return getattr(expr, method)()


# --- str() of expressions (T1c) ---------------------------------------------------------------------------------

def gen_str_expression(g, depth, budget=None, leaf=None):
    """A small multiset expression whose str() holds no address: only the classes with a `__str__` (pools, deals,
    compound keeps, binary operators, count adjustments, filters, MultisetKeep, mixtures, parameters). Not
    weightless(), map_counts, the pairing or versus operators, force_order, or a hand of a tuple expression."""
    from harness.catalogue.functions import fn_node
    r = g.rng
    if budget is None:
        budget = r.choice([0, 1, 1, 2, 2, 3])
    if leaf is None:
        from harness.grammar.pool import gen_pool
        leaf = lambda g, depth: gen_pool(g, depth, negative=r.random() < 0.3, deals=True, expressions=False)  # noqa: E731
    if budget <= 0 or r.random() < 0.1:
        return leaf(g, depth)
    budget -= 1
    sub = lambda: gen_str_expression(g, depth, budget // 2, leaf)  # noqa: E731
    k = r.random()
    if k < 0.08:
        # Equal inners are counted together: the weight shows.
        inners = [sub() for _ in range(r.choice([1, 2, 2, 3]))]
        if r.random() < 0.5:
            inners.insert(r.randint(0, len(inners)), r.choice(inners))
        return {"op": "mx_mixture", "inners": inners}
    if k < 0.3:
        a, b = sub(), sub() if r.random() < 0.8 else _outcomes(g)
        if r.random() < 0.1:
            a, b = _outcomes(g), sub()
        return _binary(a, r.choice(list(_BINARY)), b)
    if k < 0.4:
        name = r.choice(["additive_union", "difference", "intersection", "union"])
        args = [sub() if r.random() < 0.85 else _outcomes(g) for _ in range(r.choice([0, 1, 2, 2, 3]))]
        return {"op": "mx_method", "name": {"str": name}, "args": args,
                "keep_negative_counts": {"bool": r.random() < 0.4}}
    if k < 0.6:
        op = r.choice(["mul", "rmul", "neg", "pos", "floordiv", "mod", "unique", "unique", "keep_counts",
                       "keep_counts", "multiply_counts", "divide_counts", "modulo_counts"])
        n = r.choice([0, 1, 1, 2, 3, -1, -2])
        if op in ("floordiv", "divide_counts", "mod", "modulo_counts") and n == 0:
            n = 2
        comparison = _comparison(g, ["==", "!=", "<=", "<", ">=", ">"])
        return {"op": "mx_counts", "expr": sub(), "o": {"str": op}, "comparison": comparison, "n": n}
    if k < 0.72:
        t = r.random()
        target = _outcomes(g) if t < 0.3 else fn_node(g, pred=True)[0] if t < 0.5 else sub()
        return {"op": "mx_filter", "expr": sub(), "invert": {"bool": r.random() < 0.5}, "target": target}
    if k < 0.92:
        # Mostly over an operator (a MultisetKeep, whose str shows the index as given; a keep generator stays one).
        inner = gen_str_expression(g, depth, max(budget // 2, 1), leaf) if r.random() < 0.7 else sub()
        return _keep(g, inner)
    if k < 0.97:
        # max_pair_keep / max_pair_drop with '==' are `+self & +other` / `self - other`.
        return {"op": "mx_max_pair", "expr": sub(), "keep": {"bool": r.random() < 0.5},
                "comparison": {"str": "=="}, "other": sub(), "priority": None}
    # force_order(Order.Any) is the expression itself.
    return {"op": "mx_force_order", "expr": sub(), "order": 0}


def _gen_mx_str(g, depth):
    if g.rng.random() < 0.1:
        from harness.grammar.deck import gen_multi_deal
        return {"op": "mx_str", "expr": gen_multi_deal(g, depth)}
    return {"op": "mx_str", "expr": gen_str_expression(g, depth)}


@node("mx_str", "value", gen=_gen_mx_str, weight=0.5)
def mx_str(expr):
    """str(expr) of a multiset expression (or a MultiDeal, or the die a pool's int index evaluates to). Their repr
    is object.__repr__ (an address) except a deal's (deal_str), and so is the str of the classes without a
    `__str__` (weightless(), map_counts, sort_pair, ...): the generator leaves those out."""
    return str(expr)
