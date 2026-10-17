"""The facade on the Rust backend against the oracle, for what the program language cannot express: host outcomes
(enums, namedtuples, user classes), exceptions raised by them, big ints, callables with extra arguments,
user-defined evaluators (host states, keyword arguments, UnsupportedOrder subclasses), and the Python API of
Vector and Symbols (native core values) with Python arguments.

    uv run python -m pytest crates/icepool-py/tests -q -p no:cacheprovider
    (after `maturin develop`, which harness/core/upstream_suite.sh and run-facade.sh do)

Each case is an expression. All cases run in one subprocess per side (the oracle checkout, and python/icepool
with ICEPOOL_BACKEND=rust) and must agree: each item's repr, type name and quantity, or the exception type.
"""
import json
import os
import subprocess
import sys

import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
ORACLE = os.path.expanduser(os.environ.get('ICEPOOL_ORACLE', '~/code/alexrecarey/icepool-oracle'))

PRELUDE = '''
import enum, collections, math, operator
from fractions import Fraction
from icepool import *
from icepool import AgainExpression, MultisetMixture
import icepool as ip

class E(enum.IntEnum):
    A = 1
    B = 2

class S(str, enum.Enum):
    X = 'x'

P = collections.namedtuple('P', 'x y')

class Boom(Exception):
    pass

class Card:
    """Ordered and compared by rank only; + raises a custom exception."""
    def __init__(self, rank): self.rank = rank
    def __lt__(self, other): return self.rank < other.rank
    def __eq__(self, other): return isinstance(other, Card) and self.rank == other.rank
    def __hash__(self): return hash(self.rank)
    def __repr__(self): return f'Card({self.rank})'
    def __add__(self, other): raise Boom()
    def __neg__(self): return Card(-self.rank)

class HostState(MultisetEvaluator):
    """States are frozensets (host objects to the core)."""
    def next_state(self, state, order, outcome, count):
        s = state or frozenset()
        return s | {outcome} if count > 0 else s
    def final_outcome(self, final_state, order, outcomes, *sizes):
        return tuple(sorted(final_state or ()))

class KwEval(MultisetEvaluator):
    def initial_state(self, order, outcomes, *sizes, start=0, scale=1):
        return start
    def next_state(self, state, order, outcome, count):
        return state + count
    def final_outcome(self, final_state, order, outcomes, *sizes, start=0, scale=1):
        return final_state * scale

class BoomEval(MultisetEvaluator):
    def next_state(self, state, order, outcome, count):
        if outcome == 3:
            raise Boom()
        return outcome

class MyUnsupported(UnsupportedOrder):
    pass

class AscOnly(MultisetEvaluator):
    def initial_state(self, order, outcomes, *sizes):
        if order < 0:
            raise MyUnsupported('ascending only')
        return ()
    def next_state(self, state, order, outcome, count):
        return state + (outcome,) * count

class AscOnly2(AscOnly):
    def next_state(self, state, order, outcome, *counts):
        return state + (outcome,) * sum(counts)

class NeverOrder(MultisetEvaluator):
    def initial_state(self, order, outcomes, *sizes):
        raise MyUnsupported('never')
    def next_state(self, state, order, outcome, count):
        return state

class ListState(MultisetEvaluator):
    def next_state(self, state, order, outcome, count):
        return [outcome]

class OrderSeen(MultisetEvaluator):
    """What the callbacks receive: the Order member, the outcomes tuple, the sizes."""
    def initial_state(self, order, outcomes, *sizes):
        return (type(order).__name__, int(order), type(outcomes).__name__, outcomes, sizes)
    def next_state(self, state, order, outcome, *counts):
        return state + ((type(order).__name__, int(order), outcome, counts),)

class ExtraBoom(MultisetEvaluator):
    def extra_outcomes(self, outcomes):
        raise MyUnsupported('extra')
    def next_state(self, state, order, outcome, count):
        return outcome

class ExtraRange(MultisetEvaluator):
    def extra_outcomes(self, outcomes):
        return range(0, 5, 2)
    def next_state(self, state, order, outcome, count):
        return (state or ()) + (outcome,)

class RestartState(MultisetEvaluator):
    def next_state(self, state, order, outcome, count):
        return Restart if outcome == 2 and count == 2 else (state or 0) + count * outcome

class SubSum(evaluator.SumEvaluator):
    def final_outcome(self, final_state, order, outcomes, *sizes):
        return final_state * 10

class MyInt(int):
    """An int whose arithmetic keeps the subclass: a host Rational to the core."""
    def __mul__(self, o): return MyInt(int(self) * o)
    __rmul__ = __mul__
    def __add__(self, o): return MyInt(int(self) + o)
    __radd__ = __add__

def boom_count(outcome, *counts):
    raise Boom()

def boom_pred(outcome):
    raise Boom()

@multiset_function
def mf_kw(x, *, target):
    return x.keep_outcomes([target]).size()

@multiset_function
def mf_inner_kw(x, *, k):
    return x.keep_outcomes([k]).size()

@multiset_function
def mf_outer_kw(x):
    return mf_inner_kw(x, k=2), x.sum()

@multiset_function
def mf_user(a, b):
    return HostState().evaluate(a - b), KwEval().evaluate(a, start=10, scale=2)

@multiset_function
def mf_asc(a):
    return AscOnly().evaluate(a), a.sum()

@multiset_function
def mf_extra(a):
    return ExtraRange().evaluate(a), a.largest_straight()

@multiset_function
def mf_boom(a):
    return a.sum(), BoomEval().evaluate(a)

@multiset_function
def mf_restart(a):
    return RestartState().evaluate(a), a.size()

@multiset_function
def mf_hands(hands):
    a, b = hands
    return OrderSeen().evaluate(a, b), (a & b).size()

@multiset_function
def mf_never(a):
    return NeverOrder().evaluate(a)

@multiset_function
def mf_const(a):
    return Die([E.A, E.B]), a.size()

@multiset_function
def mf_joint(x):
    return x.sum(), x.size()

def same_counts(outcome, count):
    return count

HANDS = Deck([1, 2, 3, 3]).deal((1, 2))
HANDS2 = Deck([1, 2, 2, 3]).deal((2, 1))

import math
from icepool.generator.multiset_generator import MultisetSource
from icepool.operator.multiset_operator import MultisetOperator
from icepool.order import OrderReason

class UniformSource(MultisetSource):
    def __init__(self, n, outcomes, preference=(Order.Any, OrderReason.NoPreference)):
        self.n = n
        self._outcomes = tuple(outcomes)
        self.preference = preference
    def outcomes(self):
        return self._outcomes
    def size(self):
        return self.n
    def pop(self, order, outcome):
        if order > 0:
            target, rest = self._outcomes[0], self._outcomes[1:]
        else:
            target, rest = self._outcomes[-1], self._outcomes[:-1]
        if outcome != target:
            yield self, 0, 1
            return
        m = len(rest)
        for k in range(self.n + 1):
            if m == 0 and k < self.n:
                continue
            yield UniformSource(self.n - k, rest, self.preference), k, math.comb(self.n, k)
    def order_preference(self):
        return self.preference
    def is_resolvable(self):
        return True
    @property
    def hash_key(self):
        return type(self), self.n, self._outcomes, self.preference

class Uniform(MultisetGenerator):
    def __init__(self, n, outcomes, preference=(Order.Any, OrderReason.NoPreference)):
        self.n = n
        self.outcomes_ = tuple(outcomes)
        self.preference = preference
    def _make_source(self):
        return UniformSource(self.n, self.outcomes_, self.preference)
    @property
    def hash_key(self):
        return type(self), self.n, self.outcomes_, self.preference
    @property
    def _static_keepable(self):
        return False

class DoubleCounts(MultisetOperator):
    def __init__(self, child):
        self._children = (child,)
    def _next_state(self, state, order, outcome, child_counts, source_counts, arg_counts):
        return state + 1, child_counts[0] * 2
    def _initial_state(self, order, outcomes, child_sizes, source_sizes, arg_sizes):
        return 0, None if child_sizes[0] is None else child_sizes[0] * 2
    @property
    def _expression_key(self):
        return DoubleCounts

class MyUnion(ip.operator.MultisetUnion):
    pass

class BoomSource(UniformSource):
    """Raises from pop (inside the driver's try) and from outcomes (outside)."""
    def pop(self, order, outcome):
        raise Boom()

class BoomGen(Uniform):
    def _make_source(self):
        return BoomSource(self.n, self.outcomes_)

class UnsupportedSource(UniformSource):
    """Ascending pops only: the driver retries in the other order."""
    def pop(self, order, outcome):
        if order < 0:
            raise MyUnsupported('ascending only')
        return UniformSource.pop(self, order, outcome)

class AscGen(Uniform):
    def _make_source(self):
        return UnsupportedSource(self.n, self.outcomes_)

class AscOperator(DoubleCounts):
    def _initial_state(self, order, outcomes, child_sizes, source_sizes, arg_sizes):
        if order < 0:
            raise MyUnsupported('ascending only')
        return 0, None

def nest_vector(n, leaf=1):
    """`leaf` in `n` nested one-element vectors."""
    for _ in range(n):
        leaf = Vector([leaf])
    return leaf

def nest_tuple(n, leaf=1):
    for _ in range(n):
        leaf = (leaf,)
    return leaf

class Deep:
    """Adding to it adds two vectors nested 100 deep: a vector operator called back from inside one."""
    def __add__(self, other):
        return nest_vector(100) + nest_vector(100)
    def __repr__(self): return 'Deep()'

def items(x):
    if isinstance(x, Population):
        return [(repr(o), type(o).__name__, q) for o, q in x.items()]
    return repr(x)
'''

CASES = [
    'Die([E.A, E.B, 1, 2, 2])',
    'Die([1, E.A]) + E.B',
    'd(3) + E.A',
    'Die([E.A, E.B]) * 2.5',
    'Die([P(1, 2), P(0, 5), P(1, 2)])',
    'Die([P(1, 2), P(0, 5)]) + (9,)',
    'Die([Card(3), Card(1), Card(3)])',
    '-Die([Card(3), Card(1)])',
    'Die([Card(3), Card(1)]) + Die([Card(2)])',
    'Die([Card(1), 1])',
    'Die([(1, Card(2)), (1, Card(1))])',
    'Die([Vector((1, 2)), Vector((0, 5))]) + Vector((1, 1))',
    'Die([Vector((1, 2)), Vector((0, 5))]) + Vector((1, 1, 1))',
    'Die([Symbols("ab"), Symbols("b")]) + Symbols("a")',
    'Die([[1, 2]])',
    'Die([1, None])',
    'Die([(1, None), (0, None)])',
    'Die([Reroll, 1, 2])',
    'Die([2**70, -2**80, 3]) * 2**65',
    'Die({2**70: 2**66, 1: 1})',
    'Die([1, 2], times=[2**70, 3])',
    'Die([Fraction(1, 3), 0.25, True]) / 3',
    'Die([1, 2]) ** Fraction(1, 2)',
    'Die([-1, 4]) ** 0.5',
    'Die([complex(1, 2)])',
    'Die(["%d"]) % 3',
    'round(Die([1.25, 2.5, 13]), 1)',
    'round(Die([1.25, 2.5]), E.A)',
    'math.floor(Die([Fraction(7, 2), -1.5]))',
    'Die([(1, 2), (3, 4)]).marginals[1]',
    'Die([0.0, 1]).binary_operator(Die([2]), operator.truediv)',
    'Die([1, 2]).binary_operator(Die([1, 3]), lambda a, b, c=0: a * b + c, c=10)',
    'Die([1, 2]).unary_operator(lambda a, k: a - k, 5)',
    'Die({1: 0.0, 2: 3})',
    'Die([Die([]), 3], times=[1.5, 1])',
    'Die([1, 2], times=[1.5, 1])',
    'Die([d(2), d(3)], times=[True, 2])',
    'Die([Deck([1, 1, 2]), 5])',
    'Die([1, 2]) == Die([2, 1])',
    'Die([1, 2]) != 1',
    'Die([1, 2]).cmp(Die([2]))',
    'Die([float("inf"), 1]) - float("inf")',
    # S1b: Die methods on the core. Selections are Python's (callables, membership), the rest Rust's.
    'd(6).reroll(lambda x: x < 3, depth=2)',
    'd(6).reroll(lambda x: 1 / 0, depth=2)',
    'Die([(1, 2), (3, 4)]).reroll(lambda a, b: a == 1, depth="inf")',
    'Die([E.A, E.B, 3]).reroll([1], depth=1)',
    'Die([Card(1), Card(2), Card(3)]).reroll(depth="inf")',
    'd(6).filter(lambda x, k=4: x >= k, depth=3)',
    'd(6).reroll([1], depth=1.5)',
    'd(6).reroll([1], depth=-1)',
    'd(6).reroll([1], depth=Fraction(2))',
    'd(6).reroll([1], depth=Fraction(1, 2))',
    'd(6).reroll([1], depth=E.B)',
    'd(6).reroll([1], depth=float("inf"))',
    'd(6).reroll([1], depth=float("nan"))',
    'd(6).reroll([1], depth=1e300)',
    'd(6).reroll([1, 2, 3, 4, 5, 6], depth=0.5)',
    'Die([]).reroll([1], depth=0.5)',
    'd(6).reroll([1], depth="x")',
    'Die([E.A, E.B, 3]).split([E.B])',
    'Die([Card(1), Card(2), Card(3)]).split(lambda c: c.rank > 1)',
    'Die([E.A, E.B, 3]).clip(E.B, None)',
    'Die([1, 2, 3]).clip(E.B, 2.5)',
    'Die([Card(1), Card(2), Card(3)]).truncate(Card(2))',
    'Die([Card(1), Card(2), Card(3)]).clip(None, Card(2))',
    'Die([1, 2]).truncate(Card(2))',
    'Die([Card(1), Card(3)])._pop_max()',
    'Die({1: 2**70, 2: 0})._pop_min()',
    'Die([])._pop_min()',
    'Die([Vector((1, 2)), Vector((0, 5))]).zero()',
    'Die([Card(1)]).zero()',
    'Die([2**70, 2**71], times=[2**65, 2**66]).simplify()',
    '3 @ Die([Vector((1, 2)), Vector((0, 5))])',
    '2 @ Die([Card(1), Card(2)])',
    'Die([0, 2]) @ Die([Card(1)])',
    'Die([-1, 2]) @ Die([E.A, E.B])',
    'Die([E.B]) @ d(6)',
    'Die([3.0, 1]) @ d(2)',
    'Die([Fraction(2, 1)]) @ d(2)',
    'Die([1100]) @ d(2)',
    'Die([Card(1), Card(2), Card(3)]).highest(2)',
    'Die([E.A, E.B, 3]).lowest(3)',
    'd(6)._lowest_single(Fraction(2))',
    'd(6)._highest_single(2.0)',
    'Die({1: 0, 2: 1})._highest_single(-1)',
    'Die({1: 0, 2: 1})._lowest_single(-1)',
    'd(6)._lowest_single(Fraction(-1))',
    'd(6)._lowest_single(1e300)',
    'd(6)._lowest_single(E.B)',
    'Die([])._lowest_single(2.5)',
    'Die([1, 2]).pool(1)._dice[0][0]._lowest_single(True)',
    'highest(Die([Card(1), Card(3)]), Die([Card(2)]))',
    'lowest(Die([E.A, 3]), d(2), 2)',
    # S1c: map (no repeat) on the core: the product and the calls in Rust, the mixing in Die().
    'map(lambda a, b, k=0: a * b + k, d(3), d(2), k=10)',
    'd(6).map(lambda x, *, base: x + base, base=E.B)',
    'map(lambda x, d=None: x + d, d(3), d=d(2))',
    'map(lambda x: Break() if x < 3 else Break(x * 10), d(6))',
    'map(lambda x: Break(d(2)) if x == 1 else x, d(3))',
    'map(lambda x: Break(Reroll) if x == 1 else x, d(3))',
    'map(lambda x: Restart if x == 2 else x, d(3))',
    'map(lambda x: x + Again if x == 3 else x, d(3), again_depth=2)',
    'map(lambda x: x + Again if x == 3 else x, d(3), again_count=2)',
    # S3a: Again on the core in both modes: expression functions and Die() are host callbacks.
    'Die([1, 2, AgainExpression(lambda a, b: a * 10 + b, Again, 3)], again_depth=2)',
    'Die([1, AgainExpression(lambda x: 1 / 0, Again)])',
    'Die([E.A, E.B + Again], again_depth=2)',
    'Die([E.A, E.B + Again], again_count=2)',
    'Die([Card(1), -Again], again_end=Card(5))',
    'Die(["a", Again + "b"], again_depth=2)',
    'Die(["a", Again + "b"], again_count=2)',
    'Die([Fraction(1, 2), Again + Fraction(1, 3)], again_count=2)',
    'Die([1, 2, 3 + Again], again_count=2, again_end=Restart)',
    'Die([1, Again + Again, d(2)], again_count=3, again_end=Reroll)',
    'Die([1, Again + d(2)], again_count=2, again_end=d(3))',
    'Die([Reroll, 2 + Again, 1, Again], again_count=1)',
    'Die([1, 2 * Again], again_count=1)',
    'Die([1, d(2) @ Again], again_count=1)',
    'Die([1, 3 + Again], again_count=1.0)',
    # S3: a bare `AgainExpression()` is not additive (only `Again` is): count mode raises ValueError at it, after
    # the expressions before it have been evaluated; nested in an additive one, its own flag is never checked.
    'Die([1, AgainExpression()], again_count=1)',
    'Die([1, AgainExpression()], again_depth=2)',
    'Die([1, 2 + AgainExpression(is_additive=True)], again_count=2)',
    'Die([1, AgainExpression(operator.add, AgainExpression(), 3, is_additive=True)], again_count=2)',
    'Die([1, AgainExpression(lambda x: 1 / 0, Again, is_additive=True), AgainExpression()], again_count=1)',
    'Die([1, AgainExpression(), AgainExpression(lambda x: 1 / 0, Again, is_additive=True)], again_count=1)',
    'Die([1, 3 + Again], again_depth=1.5)',
    'Die([1, 3 + Again], again_count=-1)',
    'Die([1, 3 + Again], again_count=1, again_depth=1)',
    'Die([1, 3 + Again], again_depth=2, again_end=Restart)',
    'Die([1, 3 + Again], again_depth=2, again_end=Reroll)',
    'Die({1: 2, 3 + Again: 1}, 3, again_depth=2)',
    'd(6).explode(depth=2, end=Reroll)',
    'd(4).explode(lambda x: x >= 3, depth=3)',
    'Die([Again, Again])',
    'map(lambda a, b: a + b, Die(["ab", "c"]))',
    'map(lambda a, b: a + b, Die([5]))',
    'map(lambda a, b: (b, a), Die([P(1, 2), P(3, 4)]))',
    'map(lambda c: c.rank, Die([Card(1), Card(2)]))',
    'map(lambda c, d: c + d, Die([Card(1)]), Die([Card(2)]))',
    'map(lambda x: 1 / 0, d(2))',
    'map(lambda x: x, [1, 2])',
    'map(lambda x, y: (x, y), d(2), [1, 2])',
    'map(lambda x, y: x + y, d(2), Deck([1, 1, 5]))',
    'map(lambda x, y: x, d(2), Deck([1, 1]))',
    'map(lambda x: (x, d(2)), d(2))',
    'map(lambda x: (x, Reroll), d(2))',
    'map({1: d(2), 2: Reroll}, d(3))',
    'map({1: 2}, d(3), d(2))',
    'map(lambda x: x, Pool([d(2), d(2)]))',
    'map(lambda x: x, Die({2**70: 2**65}), Die({1: 2**66, 2: 0}))',
    'map(lambda x: [x], d(2))',
    'map(lambda: 5)',
    'map(lambda x: x, d(2), star=True)',
    'map_function(lambda x, y: x * y)(d(2), d(3))',
    'tupleize(d(2), E.A, Card(3))',
    'tupleize(d(2), Die([True, 2]))',
    'tupleize(d(2), Deck([1]))',
    'tupleize(d(2), Reroll)',
    'tupleize(d(2), [1])',
    'tupleize(Die({1: 0, 2: 1}), d(2))',
    'tupleize(d(2), Pool([d(2)]))',
    'd(2).sequence(3)',
    'Die([(d(2), Card(1))])',
    # Pools (S1d): the facade's Pool wraps a core pool; host outcomes, IntEnum counts and indices, odd indices.
    '(Pool([E.A, E.B, 1]).outcomes(), Pool([E.A, E.B, 1])._dice)',
    'Pool([Fraction(1, 2), 0.5, 1]).outcomes()',
    'Pool([Card(3), Card(1)]).sum()',
    'Pool([Card(3), Card(1), Card(2)])[-1]',
    'Pool([P(1, 2), P(0, 1)]).highest(1).sum()',
    'd(6).pool(3)[E.B]',
    'd(6).pool(3)[E.A:].keep_tuple()',
    'd(6).pool(3)[1.5:]',
    'd(6).pool(3)[::1]',
    'd(6).pool(3)[1.5:2:1]',
    'd(6).pool(3)[-2**70:2**70].keep_tuple()',
    'd(6).pool(3)[2**70]',
    'd(6).pool(3)[E.A, ..., E.A].keep_tuple()',
    'd(6).pool(3)[True, ...].keep_tuple()',
    'd(6).pool(3)[..., ...]',
    'd(6).pool([1, 0, 2]).keep_tuple()',
    'd(6).pool(2**70)',
    'Pool([d(6)], times=[E.B]).keep_tuple()',
    'Pool([d(6)], times=[2.0])',
    'Pool({d(4): E.B}).raw_size()',
    'Pool(Pool([d(4), d(6)])[1:], E.B)._dice',
    'd_pool({4: E.B}).sum()',
    'z_pool([E.B, 3]).sum()',
    '(Pool([1, 2]) + Pool([3])).keep_tuple()',
    '(Pool([1, 2]) + Pool([3])).sum()',
    '(d(4).pool(2)[1:] + d(4).pool(2)).sum()',
    '(d(4).pool(2) + [1, 5]).sum()',
    '(E.B * d(4).pool(2)).keep_tuple()',
    '(-d(4).pool(2)[1:]).keep_tuple()',
    'd(4).pool(2) * 1.5',
    'str(d(6).pool(2)[-1:])',
    'd(6).pool(3).equals(d(6).pool(3))',
    'd(6).pool(3).equals(d(6).pool(3)[1:])',
    'hash(d(6).pool(3)) == hash(Pool([d(6)] * 3))',
    'd(6).pool(3)._make_source().order_preference()',
    'Pool([d(4), d(6)])[1:]._make_source().order_preference()',
    '(d(6).pool(3)._make_source().size(), (-d(6).pool(3))._make_source().size())',
    'Pool([]).sum()',
    '(Pool([d(2)], 0)._make_source().is_resolvable(), Pool([Die([])])._make_source().is_resolvable())',
    '[(s.dice, s.outcomes(), s.keep_tuple, c, w) for s, c, w in d(3).pool(2)[1:]._make_source().pop(Order.Ascending, 1)]',
    '[(s.dice, s.outcomes(), s.keep_tuple, c, w) for s, c, w in Pool([d(2), d(3)])._make_source().pop(Order.Descending, 3)]',
    'Pool([d(4), d(6)]).middle(1, tie="low").sum()',
    'Pool([d(4), Die([E.A, E.B])]).highest(1).sum()',
    # Evaluation (S1e): built-ins on the core, any other evaluator's methods as host callbacks.
    'HostState().evaluate(d(4).pool(2))',
    'KwEval().evaluate(d(4).pool(2), start=E.B, scale=Fraction(1, 2))',
    'KwEval().evaluate(d(4).pool(2), bad=1)',
    'BoomEval().evaluate(d(4).pool(2))',
    'AscOnly().evaluate(d(3).pool(2))',
    'AscOnly2().evaluate(d(3).pool(2), d(3).pool(2)[1:])',
    'NeverOrder().evaluate(d(3).pool(2))',
    'ListState().evaluate(d(3).pool(1))',
    'OrderSeen().evaluate(d(2).pool(2), -d(3).pool(1))',
    'ExtraBoom().evaluate(d(3).pool(1))',
    'ExtraRange().evaluate(d(3).pool(1))',
    'RestartState().evaluate(d(2).pool(2))',
    'SubSum().evaluate(d(3).pool(2))',
    'evaluator.SumEvaluator(lambda x: x * 2).evaluate(d(3).pool(2))',
    'evaluator.SumEvaluator({1: 10}).evaluate(d(3).pool(2))',
    'evaluator.sum_evaluator(d(3).pool(2))',
    'evaluator.sum_evaluator.evaluate(d(3).pool(2), d(3).pool(1))',
    'evaluator.sum_evaluator.evaluate(d(3).pool(2), foo=1)',
    'evaluator.sum_evaluator.evaluate()',
    'evaluator.size_evaluator.evaluate(d(3).pool(2), foo=1)',
    'Pool([Die([E.A, E.B])] * 2).sum()',
    'Pool([Card(3), Card(1), Card(3)]).largest_count_and_outcome()',
    'Pool([Card(3), Card(1)]).sum()',
    'Pool([Card(3), Card(1), Card(3)]).sum()',
    'Pool([Vector((1, 2)), Vector((0, 5))]).sum()',
    'd(4).pool(2).expand(order=Order.Descending)',
    'd(4).pool(2).expand(order=E.A)',
    'evaluator.ExpandEvaluator().evaluate(d(3).pool(2), order=-1)',
    'evaluator.empty_evaluator.evaluate(d(2).pool(1), any=1)',
    'evaluator.empty_evaluator.evaluate(d(2).pool(1), any=False, other=1)',
    'evaluator.keep_evaluator.evaluate(d(4).pool(3), index=-1)',
    'evaluator.keep_evaluator.evaluate(-d(4).pool(3), index=-1)',
    'evaluator.keep_evaluator.evaluate(d(4).pool(3), index=E.A)',
    'evaluator.keep_evaluator.evaluate(d(4).pool(3), index=2**70)',
    'evaluator.keep_evaluator.evaluate(d(4).pool(3), index=5)',
    'evaluator.keep_evaluator.evaluate(d(4).pool(3))',
    'd(6).pool(2).count_subset([1], empty_divisor=None)',
    'd(6).pool(2).count_subset([])',
    'd(6).pool(2).count_subset([], empty_divisor=E.B)',
    'd(6).pool(2).all_counts(filter=E.B)',
    'd(6).pool(2).all_counts(filter="x")',
    'd(6).pool(3).largest_straight_and_outcome("low")',
    'd(3).pool(2) < [1, 2]',
    'd(3).pool(2) > [1]',
    'd(3).pool(2) == [1, 2]',
    'd(3).pool(2) != [1]',
    'bool(d(3).pool(2) == d(3).pool(2))',
    'Die(range(400)).pool(1).sum()',
    'Die(range(3000)).pool(1).sum()',
    'HostState().evaluate(Die(range(3000)).pool(1))',
    'Pool([Die([])]).sum()',
    # Statistics (population/base.py) over host outcomes and arguments.
    'Die([Vector((1, 2)), Vector((3, 1)), Vector((2, 2))]).covariance(0, 1)',
    'Die([Vector((1, 2)), Vector((3, 1)), Vector((2, 2))]).correlation(0, 1)',
    'Die([Vector((1, 2)), Vector((3, 1))]).mean()',
    'Die([Vector((1, 2)), Vector((3, 1))]).variance()',
    'Die([Vector((1, 2)), Vector((3, 1))]).sd()',
    'Die([Vector((1, 2)), Vector((3, 1))]).marginals[1]',
    'Die([Vector((1, 2)), Vector((3, 1, 0))]).marginals[0:2]',
    'len(Die([Vector((1, 2)), Vector((3, 1, 0))]).marginals)',
    'tuple(Die([Vector((1, 2)), Vector((3, 1, 0))]).marginals)',
    'Die([P(1, 2), P(0, 5), P(1, 3)]).marginals.x',
    'Die([P(1, 2), P(0, 5), P(1, 3)]).marginals[1]',
    'len(Die([P(1, 2), P(0, 5)]).marginals)',
    'Die([P(1, 2), P(0, 5), P(1, 3)]).covariance(0, 1)',
    'Die([P(1, 2), P(0, 5)]).mean()',
    'Die([Symbols("ab"), Symbols("b")]).marginals["a"]',
    'Die([Symbols("ab"), Symbols("b")]).marginals["ab"]',
    'len(Die([Symbols("ab"), Symbols("b")]).marginals)',
    'Die([(1, Card(2)), (1, Card(1))]).marginals[1]',
    'Die([(1, 2), (3, 4)]).marginals[E.A]',
    'Die([(1, 2), (3, 4)]).covariance(E.A, 0)',
    'Die([E.A, E.B, E.B]).mean()',
    'Die([E.A, E.B, E.B]).variance()',
    'Die([E.A, E.B, E.B]).skewness()',
    'Die([E.A, E.B, E.B]).entropy()',
    'Die([E.A, E.B, E.B]).median()',
    'Die([E.A, E.B, E.B]).quantile(E.A, E.B)',
    'Die([E.A, E.B, E.B]).nearest("<=", 1.5)',
    'Die([E.A, E.B, E.B]).quantity(">", E.A)',
    'Die([E.A, E.B, E.B]).probability(">=", E.B, percent=1)',
    'Die([E.A, E.B, E.B]).mode()',
    'Die([MyInt(3), MyInt(5)]).mean()',
    'Die([MyInt(3), MyInt(5)]).sd()',
    'Die([MyInt(3), MyInt(5)]).covariance(0, 0)',
    'Die([Card(3), Card(1), Card(2)]).nearest("<", Card(2))',
    'Die([Card(3), Card(1), Card(2)]).nearest(">=", Card(2))',
    'Die([Card(3), Card(1), Card(2)]).quantity("<=", Card(2))',
    'Die([Card(3), Card(1), Card(2)]).median_low()',
    'Die([Card(3), Card(1), Card(2)]).median()',
    'Die([Card(3), Card(1), Card(2)]).mean()',
    'Die([Card(3), Card(1)]).kolmogorov_smirnov(Die([Card(2)]))',
    'Die([Card(3), Card(1)]).pad_to_denominator(5, Card(2))',
    'd(6).entropy(E.B)',
    'd(6).entropy(Fraction(1, 2))',
    'd(6).entropy(1)',
    'd(6).entropy("x")',
    'd(6).entropy(2**2000)',
    'd(6).entropy(Card(2))',
    'Die([]).entropy("x")',
    'Die([1]).entropy()',
    'Die([2**70, 1]).sd()',
    'Die([10**400, 1]).sd()',
    'Die([10**400, 1]).mean()',
    'Die([1.5, 2, Fraction(1, 3)]).variance()',
    'Die([float("inf"), 1]).sd()',
    'Die({1: 0, 2: 3}).modal_quantity()',
    'Die([]).mode()',
    'Die([]).modal_quantity()',
    'd(6).quantities("<>")',
    'Die([]).quantities(">=")',
    'Die([]).probabilities(">=")',
    'd(6).probabilities("<", percent=True)',
    'd(6).quantity_where(lambda x: x > 3)',
    'd(6).quantity_where(lambda x: Boom() if x > 3 else 0)',
    'Die([(1, 2), (3, 4)]).quantity_where(lambda a, b: a + b > 4)',
    'd(6).probability_where(lambda x: x % 2, percent=True)',
    'd(6).probability_where(lambda x: [] if x > 3 else [1])',
    'tuple(d(10).group_by(lambda x: x % 3).items())',
    'tuple(d(6).group_by(lambda x: d(2) if x > 3 else d(3)).items())',
    'tuple(d(6).group_by({1: E.A, 2: 1, 3: "a"}).items())',
    'tuple(d(6).group_by(lambda x: [x]).items())',
    'tuple(Die([(1, "a"), (2, "b"), (1, "c")]).group_by[0].items())',
    'tuple(Die([P(1, 2), P(0, 5), P(1, 3)]).group_by.x.items())',
    'd(6).multiply_quantities(E.B)',
    'd(6).multiply_quantities(MyInt(1))',
    'd(6).divide_quantities(E.A)',
    'd(6).pad_to_denominator(10, E.A)',
    'd(6).pad_to_denominator(6, 7)',
    'd(6).multiply_to_denominator(E.B * 6)',
    'd(6).kolmogorov_smirnov(Deck([1, 2, 2]))',
    'd(6).cramer_von_mises(Deck([1, 2, 2]))',
    'Deck([1, 2, 2]).mean()',
    'Deck([1, 2, 2]).group_by(lambda x: x % 2)',
    # S2b: formatting (csv writer arguments go to Python's csv module), host outcomes' str() and len().
    'd(6).format("csv:o|q==", delimiter=";")',
    'Die(["a;b", "c"]).format("csv:o", delimiter=";", lineterminator="\\n")',
    'd(6).format("csv", dialect="unix")',
    'd(6).format("csv", dialect="nope")',
    'Die(["a b"]).format("csv:o", quoting=1)',
    'd(6).format("md", foo=1)',
    'd(6).format("csv", foo=1)',
    'Die([]).format("csv", dialect="excel-tab")',
    'str(Die([E.A, E.B]))',
    'str(Die([P(1, 2), P(3, 4)]))',
    'f"{Die([P(1, 2), P(3, 4)]):html:o}"',
    'str(Die([S.X]))',
    'str(Die([Card(1), Card(2)]))',
    'format(Die([(Card(1), "a")]), "csv:o|*o")',
    'Die([P(1, 2)]).common_outcome_length()',
    'Die([frozenset({1, 2})]).common_outcome_length()',
    'Die([Card(1), Card(2)]).common_outcome_length()',
    'format_probability_inverse(E.A)',
    'format_probability_inverse(Fraction(1, 6), E.B)',
    'format_probability_inverse(MyInt(1), 20)',
    'format_probability_inverse(Card(1))',
    # sampling draws from Python's own (user-seeded) random module
    '(__import__("random").seed(3), [d(6).sample() for _ in range(5)])[1]',
    '(__import__("random").seed(4), [Die([E.A, E.B]).sample() for _ in range(5)])[1]',
    '(__import__("random").seed(5), Die({1: 2**80, 2: 1}).sample())[1]',
    'Die([]).sample()',
    # equals / hash_key on the core
    'd(6).equals(Die([1, 2, 3, 4, 5, 6]))',
    'Die([E.A]).equals(Die([1]))',
    'Die({1: 2, 2: 2}).equals(coin(1, 2), simplify=True)',
    'Die({0: 2, 1: 2}).equals(coin(1, 2), simplify=E.A)',
    'd(6).equals(6)',
    'sorted([d(8), d(6), Die([E.A]), d(4)], key=lambda x: x.hash_key)',
    'd(6).hash_key < Die(["a"]).hash_key',
    'd(6).hash_key == (Die, (1, 2))',
    'hash(d(6)) == hash(Die([1, 2, 3, 4, 5, 6]))',
    'len({d(6), Die([1, 2, 3, 4, 5, 6]), d(4)})',
    '{d(6): 1}[Die([1, 2, 3, 4, 5, 6])]',
    'bool(d(6) == Die([1, 2, 3, 4, 5, 6]))',
    'bool(d(6) != d(6))',
    'bool(Die([Card(1)]) == Die([Card(1)]))',
    'Die([Card(1)]) == Die([Card(1)])',
    'bool(d(6) == 3)',
    'bool(3 != d(6))',
    # map(repeat), Markov chains, map_and_time on the core: the callable is a host callback
    "map(lambda s, r, k=0: min(s + r + k, 6), 0, d(3), repeat='inf', k=1)",
    "map(lambda s, r, k=0: min(s + r + k, 6), 0, d(3), repeat=2, k=1)",
    "map(lambda c: c if c.rank >= 3 else Card(c.rank + 1), Card(0), repeat='inf')",
    "map(lambda c: c if c.rank >= 3 else Die([Card(c.rank + 1), Card(c.rank + 2)]), Card(0), repeat=2)",
    "map(lambda x: E.B, Die([E.A, 2]), repeat=2)",
    "map(lambda p, r: p if p.x >= 3 else P(p.x + r, p.y), P(0, 0), d(2), repeat='inf')",
    "map_and_time(lambda p, r: p if p.x >= 3 else P(p.x + r, p.y), P(0, 0), d(2), repeat=5)",
    "map(lambda x: (_ for _ in ()).throw(Boom()) if x == 2 else min(x + 1, 4), 0, repeat='inf')",
    "map(lambda x: (_ for _ in ()).throw(Boom()) if x == 2 else min(x + 1, 4), 0, repeat=1)",
    "map(lambda x: (_ for _ in ()).throw(Boom()) if x == 2 else min(x + 1, 4), 0, repeat=2)",
    "mean_time_to_absorb(lambda x, y: Restart if y == 1 else min(x + y, 4), 0, d(3))",
    "mean_time_to_absorb(lambda c, r: c if c.rank >= 4 else Card(c.rank + r), Card(0), d(3))",
    "map_and_time(lambda s, t: min(s + sum(t), 6), 0, d(2).pool(2), repeat=3)",
    "map(lambda s, t: min(s + sum(t), 6), 0, d(2).pool(2), repeat='inf')",
    "map(lambda x: [x], 0, repeat='inf')",
    "map(lambda x: Break(Reroll) if x == 1 else x + 1, 0, repeat=3)",
    "map(lambda x: Break(d(2)) if x == 1 else x + 1, 0, repeat='inf')",
    "map(lambda x: x, 0, repeat=10**30)",
    "map(lambda t, x: min(t + x, 5), 0, d(3), repeat=Fraction(2))",
    "map(lambda t, x: min(t + x, 5), 0, d(3), repeat=E.B)",
    "Die([0]).map_and_time(lambda x: x + 1, repeat=3)",
    "Die([0]).map_and_time(lambda x: x + 1, repeat=2.5)",
    "d(6).time_to_sum(10, dnf=Reroll)",
    "d(6).mean_time_to_sum(8)",
    "map_function(repeat='inf')(lambda x: min(x + 1, 3))(d(2))",
    "d(6).map({1: 2, 2: d(3)}, repeat='inf')",
    # Deck and Deal on the core: host outcomes, Counter() operands Python counts (mappings, dice and Reroll as
    # keys), the Python paths that build decks (MultiDeal, CompoundKeepGenerator, append, split...)
    'Deck([E.A, E.B, 1, 2, 2])',
    'Deck([Card(3), Card(1), Card(3)])',
    'Deck([1, 2], times=[Fraction(0), E.B])',
    'Deck([2**70, 1], times=[2**65, 1]).size()',
    'Deck([1, 2, 3]) + {1: 2, 5: 1}',
    'Deck([1, 2, 3]) - {1: -2, 3: 1}',
    '{1: 3} - Deck([1, 2, 3])',
    'Deck([1, 2]) + Deck([Card(1)])',
    'Deck([1, 2]) & [E.A]',
    '[E.A] + Deck([1])',
    'Deck([d(2), Reroll]) + [d(2)]',
    'Deck([1]) + [Reroll]',
    'Deck([1]) + [(1, Reroll)]',
    'Deck([1, 2]) | [(1, d(2))]',
    'Deck([1, 1, 2]) * E.B',
    'Deck([1, 1, 2]) * MyInt(2)',
    'Deck([1, 2, 3]) == Deck([3, 2, 1])',
    'hash(Deck([1, 2])) == hash(Deck([2, 1]))',
    '{Deck([1]): 1}[Deck([1])]',
    'Die(Deck([1, 1, 2]))',
    'Die([Deck([1, 1, 2]), 3])',
    'Deck([1, 2, 3]).map(lambda x: Deck([x, x]) if x > 1 else Reroll)',
    'Deck([1, 2, 3]).map({2: Card(5)})',
    'Deck([1, 2, 2]).sequence(2)',
    'Deck([1, 2, 2]).mean()',
    'Deck([1, 2, 2]).kolmogorov_smirnov(d(2))',
    'd(2).cramer_von_mises(Deck([1, 2, 2]))',
    'f"{Deck([1, 2, 2]):md}"',
    'Deck([1, 2, 2]).append(3)',
    'Deck([1, 2, 2]).remove(2)',
    'Deck([1, 2, 2]).group_by(lambda x: x % 2)',
    'Deck([1, 2, 2]).split([1])',
    'Deck([1, 2, 2]).zero()',
    'Deck([1, 2, 2]).pad_to_denominator(5, 1)',
    'Deck([1, 2, 2]) // 2',
    'Deck([1, 1, 2]).deal(2).sum()',
    'Deck([Card(1), Card(2), Card(2)]).deal(2).expand()',
    'HostState().evaluate(Deck([1, 2, 2, 3]).deal(2))',
    'Deck([1, 2, 3]).deal(2)',
    '-Deck([1, 2, 3]).deal(2)',
    'Deck([1, 2, 3]).deal(2)[1]',
    'Deck([1, 2, 3]).deal(2)[..., 2].sum()',
    'Deck([1, 2, 3]).deal(2).keep_tuple()',
    'Deck([1, 2, 3]).deal(E.B).sum()',
    'Deal(Deck([1, 2]), 1.0)',
    'Deal(Deck([1, 2]), 5.0)',
    'Deal(Deck([1, 2]), -1)',
    '(Deck([1, 2, 3]).deal(2) + Deck([4, 5]).deal(1)).sum()',
    'Deck([1, 2, 3]).deal(2).issubset([1, 2])',
    'Deck([1, 2, 3, 3]).deal(2).largest_count()',
    'Pool(Deck([1, 2]))',
    'Deck(range(4), times=4).deal((2, 4)).denominator()',
    'Deck([1, 2, 2, 3]).deal_groups((1, 2)).denominator()',
    'Deck([1, 2, 3]).deal(2).deck() == Deck([1, 2, 3])',
    'list(Deck([1, 1, 2]).deal(2)._make_source().pop(Order.Ascending, 1))[0][1:]',
    # MultiDeal on the core: tuple counts and sizes reach user evaluators (host callbacks) as tuples; a built-in
    # evaluator given one runs through the callbacks too; a hand (`deal[0]`, an expression) is still upstream's
    # driver, over the core's MultiDealSource.
    'OrderSeen().evaluate(Deck([1, 2, 2, 3]).deal((1, 2)))',
    'OrderSeen().evaluate(Deck([1, 2, 3]).deal((1, 1)), Deck([2, 3]).deal(1), [1, 4])',
    'OrderSeen().evaluate(Deck([Card(1), Card(2), Card(2)]).deal((1, 1)))',
    'OrderSeen().evaluate(Deck([E.A, E.B, 2]).deal_groups((1, 2)))',
    'OrderSeen().evaluate(Deck([]).deal(()))',
    'HostState().evaluate(Deck([1, 2, 3]).deal((1, 1)))',
    'AscOnly2().evaluate(Deck([1, 2, 3]).deal((1, 1)))',
    'evaluator.sum_evaluator.evaluate(Deck([1, 2, 2]).deal((1, 1)))',
    'evaluator.size_evaluator.evaluate(Deck([1, 2, 2]).deal((1, 1)))',
    'Deck(range(5), times=2).deal((2, 2))[0].sum()',
    'Deck(range(4), times=2).deal((2, 1)).denominator()',
    'Deck([1, 2]).deal((True, 1)).denominator()',
    'Deck([1, 2]).deal([3])',
    'len(Deck([1, 2, 3]).deal_groups((1, 2), (0, 1)))',
    'Deck([1, 2, 3]).deal_groups([1, 2]).hand_sizes()',
    '[(s.hand_groups, c, w) for s, c, w in Deck([1, 1, 2]).deal_groups((1, 2))._make_source().pop(Order.Descending, 2)]',
    '[(tuple(s.outcomes()), c, w) for s, c, w in Deck([1, 1, 2]).deal((1, 2))._make_source().pop(Order.Ascending, 1)]',
    # S4c: multiset operators on the core. map_counts functions and keep_outcomes callables are host callbacks;
    # a collection target is a frozenset compared with Python's == (host outcomes included).
    'Pool([1, 2, 2, 3]).map_counts(function=lambda o, c: c if c >= 2 else 0).expand()',
    'Pool.map_counts(d(6).pool(2), d(4).pool(2), function=lambda o, a, b: min(a, b)).size()',
    'MultisetExpression.map_counts(function=lambda o: 1).size()',
    'd(6).pool(2).map_counts(function=boom_count).sum()',
    'd(6).pool(2).map_counts(function=len).sum()',
    'd(6).pool(3).keep_outcomes(lambda x: x > 3).sum()',
    'd(6).pool(3).drop_outcomes(lambda x: x % 3).expand()',
    'd(6).pool(2).keep_outcomes(boom_pred).sum()',
    'd(6).pool(3).keep_outcomes({2, 4}).size()',
    'd(6).pool(3).drop_outcomes(range(3)).sum()',
    "Pool(['a', 'b', 'c', 'b']).keep_outcomes('ab').expand()",
    'Pool([E.A, E.B, 3]).keep_outcomes([1]).expand()',
    'Pool([Card(1), Card(2), Card(2)]).drop_outcomes([Card(2)]).expand()',
    'Pool([P(1, 2), P(2, 1)]).keep_outcomes([(1, 2)]).expand()',
    'd(6).pool(3).keep_outcomes(d(4).pool(1)).sum()',
    # MultisetKeep (keep of an expression, or of a generator with negative keeps).
    '(d(6).pool(3) - d(4).pool(1))[1:].sum()',
    'd(6).pool(3)[-1, 0, 1][..., 1].sum()',
    'd(6).pool(3)[-1, 0, 1][1:].sum()',
    'd(6).pool(3)[-1, 0, 1][-2]',
    'd(6).pool(3).unique()[1:2:1].sum()',
    'd(6).pool(3).unique()[[1, 2]].sum()',
    'd(6).pool(3).unique()[[]].sum()',
    '(d(6).pool(2) + d(6).pool(2).unique())[1:-1].sum()',
    'd(6).pool(2).unique()[1:-1].sum()',
    # CompoundKeepGenerator on the core.
    '(d(6).pool(2) + d(4).pool(2)[-1:])[-2:].sum()',
    '(d(6).pool(2) + d(4).pool(2)[-1:]).keep_tuple()',
    '(d(6).pool(2) + d(4).pool(2)[-1:])[1:].keep_tuple()',
    '(-(d(6).pool(2) + d(4).pool(2)[-1:])).keep_tuple()',
    '((d(6).pool(2) + d(4).pool(2)[-1:]) * 2).sum()',
    '((d(6).pool(2) + d(4).pool(2)[-1:]) * 2)[1:].keep_tuple()',
    'type(d(6).pool(2) + d(4).pool(2)[-1:]).__name__',
    '(d(6).pool(2) + d(4).pool(2)[-1:]).hash_key == (d(6).pool(2) + d(4).pool(2)[-1:]).hash_key',
    '(d(6).pool(2) + Deck([1, 2, 3]).deal(2)).middle(2).sum()',
    'str(d(6).pool(2) + d(4).pool(2)[-1:])',
    '[type(x).__name__ for x in (d(6).pool(1) + Deck([1, 2]).deal(1))._make_source().inner_sources]',
    '[(type(s).__name__, s.keep_tuple, c, w) for s, c, w in (d(6).pool(1) + d(4).pool(2)[-1:])._make_source().pop(Order.Descending, 6)]',
    '(d(6).pool(1) + d(4).pool(2)[-1:])._make_source().order_preference()',
    '(d(6).pool(1) + d(4).pool(2)[-1:])._make_source().size()',
    '(d(6).pool(1) + d(4).pool(2)[-1:])._make_source().outcomes()',
    # User evaluators over expressions: counts and sizes as the operators produce them.
    'HostState().evaluate(d(6).pool(2) - d(4).pool(1))',
    'OrderSeen().evaluate(d(4).pool(2).unique(), [1, 2] + d(3).pool(1))',
    'OrderSeen().evaluate(d(4).pool(2)[-1:] + d(4).pool(1))',
    'AscOnly().evaluate(d(4).pool(3).highest(2))',
    'AscOnly().evaluate(d(4).pool(3).unique().highest(2))',
    # Order errors from the operators.
    "d(6).pool(2).unique().sort_pair('>', d(6).pool(2).unique(), extra='late').sum()",
    "d(6).pool(2).sort_pair('>', d(6).pool(2), extra='middle').sum()",
    "d(6).pool(2).sort_pair('>', d(6).pool(1), extra='late').sum()",
    "d(6).pool(3).sort_pair_keep_while('>=', d(6).pool(2), Order.Ascending).sum()",
    'd(6).pool(2).force_order(Order.Ascending).force_order(Order.Descending).sum()',
    "Pool([Card(1), Card(3)]).versus_all('>', [Card(2)]).expand()",
    "Pool([Card(1), Card(3)]).max_pair_keep('>=', [Card(2)], 'high').expand()",
    # S4d: @multiset_function on the core: keyword arguments bound per call, user evaluators (host callbacks) in
    # the body, UnsupportedOrder retries, extra outcomes and Restart in joint results, constants, MultiDeal hands.
    'mf_kw(d(6).pool(2), target=3)',
    'mf_outer_kw(d(4).pool(2))',
    'mf_user(d(6).pool(2), d(4).pool(1))',
    'mf_user(Pool([Card(1), Card(2)]), Pool([Card(2)]))',
    'mf_asc(d(4).pool(3))',
    'mf_extra(d(4).pool(2))',
    'mf_extra(Pool([1.5, 2]))',
    'mf_boom(d(4).pool(2))',
    'mf_restart(d(3).pool(3))',
    'mf_hands(Deck([1, 2, 2, 3]).deal((1, 2)))',
    'mf_hands(Deck([1, 2, 2, 3]).deal((1, 2)), star=False)',
    'mf_hands(Deck([1, 2, 2, 3]).deal((1, 2)), star=True)',
    'mf_never(d(4).pool(1))',
    'mf_const(d(4).pool(1))',
    'mf_joint(d(6).pool(2)), mf_joint(d(6).pool(3)), mf_joint(d(6).pool(2))',
    'mf_joint(Pool([E.A, E.B, 2]))',
    # Mixtures: each inner expression is an ordinary evaluation on the core.
    'mf_joint(MultisetMixture([d(6).pool(1), d(6).pool(2)]))',
    'MultisetMixture([d(6).pool(1), d(4).pool(2)]).sum()',
    'MultisetMixture({d(6).pool(1): 2, d(4).pool(1): 1}).expand()',
    'MultisetMixture([d(6).pool(2), d(4).pool(2)]).keep(1)',
    'MultisetMixture([d(6).pool(2), d(4).pool(2)])[1:].sum()',
    '(MultisetMixture([d(6).pool(2), d(4).pool(2)]) * 2).sum()',
    '(MultisetMixture([d(6).pool(2), d(4).pool(2)]) - [1]).sum()',
    'MultisetMixture([]).sum()',
    # Vector and Symbols (S5a): native values on the core, with Python callables and Python arguments.
    'Vector((1, 2)).binary_operator(Vector((3, 4)), lambda a, b: a * 10 + b)',
    'Vector((1, 2)).binary_operator(5, operator.add, compare_for_truth=True)',
    'bool(Vector((1, 2)).binary_operator(Vector((1, 3)), operator.lt, compare_for_truth=True))',
    'Vector((1, 2)).binary_operator(Vector((3,)), lambda a, b: a)',
    'Vector((1, 2)).reverse_binary_operator(Vector((3, 4)), operator.sub)',
    'Vector((1, 2)).reverse_binary_operator(Vector((3,)), operator.sub)',
    'Vector((1.5, 2)).unary_operator(round, 0)',
    'Vector((1, 2)).unary_operator(lambda x, k=0: x * 2 + k, k=1)',
    'Vector((E.A, 2)) + E.B',
    'Vector((Card(1), Card(2))) + 1',
    '-Vector((Card(1), Card(2)))',
    'Vector((Card(1),)) < Vector((Card(2),))',
    'bool(Vector((Card(1),)) < Vector((Card(2),)))',
    'repr(Vector((1, 2)) < Vector((1, 2, 3)))',
    'repr(Vector((Vector((1,)),)) < Vector((Vector((1, 2)),)))',
    'bool(Vector((Vector((1,)),)) < Vector((Vector((2,)),)))',
    'Vector((1, 2)) == (1, 2)',
    'bool(Vector((1, 2)) != (1, 2))',
    '[x for x in Vector((1, 2)) + Vector((3, 4))]',
    'Vector((1, 2)) + [3]',
    'hash(Vector((1, 2))) == hash(Vector((1.0, True + 1)))',
    'hash(Symbols("ab")) == hash(Symbols(["b", "a"]))',
    'Die([Vector((1, 2)), Vector((1.0, 2)), Vector((0, 9))])',
    'Die([Vector((1, 2)), Vector((1, 2, 3))])',
    'Die([Vector((1, "a")), Vector((2, 3))])',
    'Die([Vector((1, 2)), 3])',
    'Die([Vector((1, 2)) < Vector((1, 2, 3))])',
    'Die([Vector((2,)), Vector((1,))]) == Die([Vector((1,)), Vector((2,))])',
    'bool(Die([Vector((2,))]) == Die([Vector((2, 3))]))',
    'Die([Vector((2,))]) == Die([Vector((2, 3))])',
    'vectorize(d(2), 0, d(2))',
    'vectorize(d(2), Reroll)',
    'vectorize(1, 2)',
    'd(3).map(lambda x: Vector((x, -x)))',
    'Symbols({"b": 2, "a": E.A})',
    'Symbols({"b": 2, "a": 1.5})',
    'Symbols({"ab": 1})',
    'Symbols(x for x in "abca")',
    'Symbols(["a", ["b"]])',
    'Symbols(["a", 1])',
    'Symbols([("a",)])',
    'Symbols(5)',
    'Symbols("a b")',
    'Symbols("ab - c - d")',
    'Symbols("ab") + ["c", "a"]',
    'Symbols("ab") + {"c": 3}',
    '["c"] + Symbols("ab")',
    '"c" - Symbols("ab")',
    'Symbols("ab") * E.B',
    'Symbols("ab") * 2.0',
    'Symbols("aab") % 2',
    'Symbols("aab") // 2',
    'Symbols("aab") // ["a", "b"]',
    'Symbols("aab") // {"a": 2}',
    'Symbols("aab") // collections.Counter("ab")',
    '"aaab" // Symbols("ab")',
    'Symbols("aab").count_subset("")',
    'Symbols("aab").count_subset("", empty_divisor=7)',
    'Symbols("aab").divide_counts(2.0)',
    'Symbols("a - bb").has_negative_counts(), Symbols("a").size(), Symbols("a - bb").size()',
    '+Symbols("a - bb"), -Symbols("a - bb")',
    'Symbols("ab") <= ["a", "b", "c"], Symbols("ab") >= "a"',
    'Symbols("ab") < "b"',
    'Symbols("ab") == ["a", "b"], Symbols("ab") != ("a", "b"), Symbols("ab") == 3',
    'Symbols("ab").isdisjoint("b"), Symbols("ab").isdisjoint("c")',
    'Symbols("aab")["ba"], list(Symbols("aab")["ba"]), Symbols("aab")["a"], Symbols("aab").b',
    'Symbols("aab")[("a",)]',
    'Symbols("aab")[3]',
    'list(Symbols("ba") + "ca"), len(Symbols("ab") - "ab"), bool(Symbols("a") - "a")',
    'str(Symbols("ab") - "ab"), repr(Symbols("ab - c"))',
    'Die([Symbols("ab"), Symbols("ba"), Symbols("a - b")])',
    'Die([Symbols("a"), "a"])',
    'Die([Symbols("ab"), Symbols("c")]).marginals.a',
    'Die([Symbols("ab"), Symbols("c")]) // Symbols("a")',
    'Symbols("ab") // Die([Symbols("ab")])',
    'Symbols("ab") // d(3)',
    'Die([Symbols("ab")]) * 2, 2 * Die([Symbols("ab")])',
    'Die([Symbols("ab"), Symbols("c")]) <= "abc"',
    # function.py helpers (S5b) with Python arguments.
    'coin("1/3")',
    'coin(0.3, max_denominator=4)',
    'coin(E.A, 3)',
    'coin(Fraction(1, 3), 2)',
    'stochastic_round(Fraction(5, 2), max_denominator=1)',
    'one_hot(E.B)',
    'one_hot(2.0)',
    'consecutive(iter([1, 3]))',
    'consecutive((x for x in [4, 2]), [E.B])',
    'consecutive({5, 3}, Die([1]))',
    'sorted_union(Die([Card(3), Card(1)]), [Card(2)])',
    'sorted_union({3, 1}, (E.A, 2))',
    'sorted_union([[1]])',
    'min_outcome(Die([Card(3), Card(1)]), Card(2))',
    'max_outcome([E.A, 2, Die([E.B])])',
    'min_outcome(iter([]))',
    'pointwise_max(iter([d(6), d(8)]))',
    'pointwise_min(d(6), Die([E.A, E.B]))',
    'pointwise_max(Die([Card(3), Card(1)]), Die([Card(2)]))',
    'harmonize_denominators((d(6), 3), iter([2, 1]))',
    'harmonize_denominators([d(6), d(4)], [E.B, 1])',
    'harmonize_denominators((x for x in [d(6)]))',
    'reduce(lambda a, b: a * b, (x for x in [d(2), d(3)]), initial=E.B)',
    'tuple(accumulate(operator.add, iter([d(2), 1]), initial=None))',
    'reduce(max, [])',
    'from_cumulative([E.A, E.B], [1, 3])',
    'from_cumulative([1, 2], [Die([False, True]), Die([True])])',
    'Wallenius({E.A: 2, 3: (1, 2)}).deal(2).sum()',
    'Wallenius([(Card(1), 1), (Card(2), 3)]).deal(1).expand()',
    'Wallenius([("a", 1), ("b", 2)]).deal(2).highest(1).sum()',
    # S2: sign and cmp on the core, over host outcomes, vectors and outcomes that cannot be compared.
    'Die([-2, 0, 3, -0.0, float("nan"), Fraction(-1, 2), True]).sign()',
    'Die([(1, 2), ()]).sign()',
    'Die(["a", ""]).sign()',
    'Die([Vector((1, -2)), Vector((0, 0))]).sign()',
    'Die([Card(3), Card(1)]).sign()',
    'Die([E.A, E.B]).sign()',
    'Die([None]).sign()',
    'Die([Card(3), Card(1)]).cmp(Card(2))',
    'Die([E.A, E.B]).cmp(Die([E.B]))',
    'Die([1, 2]).cmp("a")',
    'Die([Vector((1, 2)), Vector((3, 0))]).cmp(Vector((2, 2)))',
    'Die([]).cmp(3)',
    # S5: exports without a node before (middle, tupleize, map_function, Die's Counts views), host outcomes.
    'middle([d(6), d(8), Die([Card(1), Card(2)])])',
    'middle([d(6), d(8), d(10)], keep=2)',
    'middle(d(6), d(8), tie="low")',
    'middle([], default=Card(3))',
    'Die([E.A, E.B]).middle(3)',
    'd(6).middle(4, 2, tie="high")',
    'tupleize(Die([E.A, E.B]), Card(1))',
    'tupleize(d(2), Reroll, d(3))',
    'tupleize(Card(1), 2)',
    'map_function(lambda x, y: (x, y), star=True)(Die([P(1, 2), P(3, 4)]))',
    'map_function(lambda x, y: x + y)(Die([P(1, 2), P(3, 4)]))',
    'map_function(star=False)(lambda x, y: x + y)(Die([P(1, 2)]))',
    'map_function(again_depth=1)(lambda x: 6 + Again if x == 6 else x)(d(6))',
    'map_function(lambda x, *, k: x + k, k=Card(1))(d(2))',
    'Die([Card(2), Card(1)]).keys()[0]',
    'Die([Card(2), Card(1)]).items()[::-1]',
    'Die([E.A, E.B]).values() == (1, 1)',
    'Die([E.A, E.B]).keys() == [E.A, E.B]',
    'Card(1) in Die([Card(1)])',
    '[1] in d(6)',
    'd(6)[7]',
    # S3c: map_to_pool (host callbacks, kwargs, Reroll / Restart, mappings), explode_to_pool / reroll_to_pool with
    # host outcomes and counts of int subclasses, and the errors Python raises from their arithmetic.
    'd(6).explode_to_pool(3, depth=4).sum() == 3 @ d(6).explode(depth=4)',
    'd(6).explode_to_pool(3, [5, 6], depth=4).sum() == 3 @ d(6).explode([5, 6], depth=4)',
    'Die([E.A, E.B]).explode_to_pool(2, [E.B], depth=2).sum()',
    'Die([Card(1), Card(2)]).explode_to_pool(2, depth=1).expand()',
    'Die([Card(1), Card(2)]).explode_to_pool(2, lambda c: c.rank > 1, depth=1).sum()',
    'd(4).explode_to_pool(E.B, depth=2).sum()',
    'd(4).explode_to_pool(2, depth=E.A).sum()',
    'd(4).explode_to_pool(2, depth=1.5)',
    'd(4).explode_to_pool(2, depth=-2)',
    'd(4).explode_to_pool(2, lambda x: 1 / 0)',
    'Die([]).explode_to_pool(2)',
    'Die([E.A, E.B, 3]).reroll_to_pool(2, max_rerolls=E.A).sum()',
    'd(6).reroll_to_pool(E.B, [1], max_rerolls=1, mode="low").sum()',
    'd(6).reroll_to_pool(3, lambda x: x < 3, max_rerolls=2, depth=E.B, mode="high").sum()',
    'd(6).reroll_to_pool(3, lambda x: x < 3, max_rerolls=1, depth=2.0)',
    'd(6).reroll_to_pool(3, max_rerolls=1.5).sum()',
    'd(6).reroll_to_pool(3, max_rerolls=1, mode=None)',
    'Die([Card(1), Card(2), Card(3)]).reroll_to_pool(2, max_rerolls=1, mode="drop").expand()',
    'd(6).pool(3).expand().map_to_pool().expand() == d(6).pool(3).expand()',
    'map_to_pool(lambda x, *, k: [x] * k, d(2), k=2).sum()',
    'map_to_pool(lambda x, *, k: [x] * k, d(2), k=d(2))',
    'd(2).map_to_pool({1: [1, 1], 2: Reroll}).sum()',
    'd(3).map_to_pool(lambda x: Restart if x == 1 else [x]).sum()',
    'map_to_pool(lambda x: [x], Deck([1, 2, 2])).sum()',
    'map_to_pool(lambda p: p, d(6).pool(2)).sum()',
    'map_to_pool(lambda p: [p, p], d(2).pool(2)).expand()',
    'Die([Card(1), Card(2)]).map_to_pool(lambda c: [c, c]).expand()',
    'Die([P(1, 2), P(3, 4)]).map_to_pool().expand()',
    'Die([P(1, 2), P(3, 4)]).map_to_pool(lambda x, y: [y]).sum()',
    'd(2).map_to_pool(lambda x: 1 / 0)',
    'd(2).map_to_pool(lambda x: Die([x]))',
    'map_to_pool(lambda: [Card(7), Card(8)]).expand()',
    'map_to_pool(lambda x: [x])',
    'map_to_pool(42, d(2))',
    # S4e: weightless generators on the core: each distinct multiset counts once; a pop yielding the same count
    # twice raises UnsupportedOrder, which the driver retries in the other order.
    'd(6).pool(3).weightless().expand()',
    "Deck([1, 2, 2, 3, 3, 3, 4, 4, 4, 4]).deal(4).weightless().expand()",
    'd(6).pool(3)[-1:].weightless().sum()',
    'Pool([d(4), d(6)]).weightless().sum()',
    '(d(6).pool(2) + Deck([1, 2, 3]).deal(2)).weightless().sum()',
    'Pool([Card(1), Card(2), Card(2)]).weightless().expand()',
    'Pool([E.A, E.B]).weightless().expand()',
    'HostState().evaluate(d(4).pool(2).weightless())',
    'AscOnly().evaluate(d(4).pool(3).weightless())',
    'OrderSeen().evaluate(d(4).pool(2).weightless(), d(3).pool(1))',
    '(d(4).pool(2).weightless() + d(4).pool(1)).sum()',
    'd(4).pool(3).weightless()[1:].sum()',
    'MultisetMixture([d(4).pool(2).weightless(), d(4).pool(2).weightless(), d(3).pool(2)]).sum()',
    'mf_user(d(6).pool(2).weightless(), d(4).pool(1))',
    '(lambda g: g.weightless() is g)(d(6).pool(2).weightless())',
    'type(d(6).pool(2).weightless()).__name__',
    'd(6).pool(2).weightless().hash_key == d(6).pool(2).weightless().hash_key',
    'd(6).pool(2).weightless().hash_key == d(6).pool(3).weightless().hash_key',
    'd(6).pool(2).weightless()._make_source().hash_key == d(6).pool(2).weightless()._make_source().hash_key',
    'type(d(6).pool(2).weightless()._make_source()._wrapped).__name__',
    'd(6).pool(2).weightless()._make_source()._wrapped.dice',
    'd(6).pool(2).weightless()._make_source().size()',
    'd(6).pool(2).weightless()._make_source().outcomes()',
    'd(6).pool(2).weightless()._make_source().order_preference()',
    'd(6).pool(2).weightless()._make_source().is_resolvable()',
    'Deck([1, 2]).deal(1).weightless()._make_source().order_preference()',
    '[(type(s).__name__, c, w) for s, c, w in d(6).pool(2).weightless()._make_source().pop(Order.Descending, 6)]',
    '[(type(s._wrapped).__name__, c, w) for s, c, w in d(6).pool(2).weightless()._make_source().pop(Order.Ascending, 1)]',
    '[(c, w) for s, c, w in Pool([d(4), d(6)]).weightless()._make_source().pop(Order.Descending, 6)]',
    '[(c, w) for s, c, w in Pool([d(4), d(6)]).weightless()._make_source().pop(Order.Ascending, 1)]',
    '[(c, w) for s, c, w in d(6).pool(3)[-1:].weightless()._make_source().pop(Order.Descending, 6)]',
    '[(c, w) for s, c, w in d(6).pool(3)[-1:].weightless()._make_source().pop(Order.Ascending, 1)]',
    # S4f: operators over tuple counts (a MultiDeal as an operand), host functions given and returning tuples.
    'MultisetExpression.map_counts(HANDS, function=lambda o, c: sum(c)).expand()',
    'MultisetExpression.map_counts(HANDS, function=lambda o, c: c[::-1]).sum()',
    'MultisetExpression.map_counts(HANDS, HANDS2, function=lambda o, a, b: a + b).sum()',
    'MultisetExpression.map_counts(HANDS, function=lambda o, c: c + c).sum()',
    'MultisetExpression.union(HANDS, HANDS2).keep_outcomes(lambda o: o > 1).sum()',
    'MultisetExpression.intersection(HANDS, HANDS2).multiply_counts(2).sum()',
    '(HANDS + d(3).pool(1)).sum()',
    'd(3).pool(2).sort_pair(\'<\', HANDS).sum()',
    'HostState().evaluate(MultisetExpression.union(HANDS).keep_counts(\'==\', 1))',
    # A mixture's inners are equal when their functions are the same object.
    'MultisetMixture([d(3).pool(2).map_counts(function=same_counts)] * 2 + [d(3).pool(2).map_counts(function=same_counts)]).sum()',
    'MultisetMixture([d(3).pool(2).map_counts(function=lambda o, c: c), d(3).pool(2).map_counts(function=lambda o, c: c)]).sum()',
    'mf_kw(HANDS[1], target=3)',
    'mf_kw(d(3).pool(2), target=3, star=False)',
    # S5c: append / remove / to_one_hot, deck zero / split / map, mean_time_to_sum and the Cartesian products of
    # expand.py on the core, with host outcomes, host callables and Python sequences.
    'Die([E.A, E.B]).append(E.A)',
    'Die([E.A, E.B]).append(1, 2)',
    'Die([Card(1), Card(2)]).append(Card(2), -1)',
    'Deck([Card(1), Card(2)]).append(Card(3), 2)',
    'd(3).append(d(2))',
    'Deck([1, 2]).append(Deck([5]), 2)',
    'd(3).append(2, E.A)',
    'd(3).append(2, Fraction(1, 2))',
    'd(3).append([1])',
    'd(3).remove(2)',
    'Deck([1, 2, 2]).remove(2, 1)',
    'Die([E.A, E.B]).remove(E.B, -2)',
    'd(3).remove(5)',
    'd(3).remove(2, "a")',
    'd(4).to_one_hot()',
    'd(4).to_one_hot((1, 1, 3))',
    'd(3).to_one_hot(range(2, 5))',
    'Die(["a", "b", "ab"]).to_one_hot("ab")',
    'Deck([Card(1), Card(2), Card(2)]).to_one_hot([Card(2)])',
    'Die([E.A, E.B]).to_one_hot([1])',
    'd(3).to_one_hot(5)',
    'Die([]).to_one_hot(5)',
    'd(3).to_one_hot(d(2))',
    'Deck([P(1, 2), P(0, 5), P(1, 2)]).zero()',
    'Deck([Card(1), Card(2)]).zero()',
    'Deck([1, "a"]).zero()',
    'Deck([Card(1), Card(2), Card(2)]).split(lambda c: c.rank > 1)',
    'Deck([E.A, E.B, 3]).split([1])',
    'Deck([1, 2]).map({1: E.A})',
    'Deck([Card(1), Card(2)]).map(lambda c: c.rank)',
    'Deck([1, 2]).map(lambda x: d(2) if x > 1 else x)',
    'Deck([1, 2]).map(lambda x: 1 / (x - 1))',
    'd(6).mean_time_to_sum(10)',
    'Die([E.A, E.B]).mean_time_to_sum(5)',
    'Die([0, 0, 2, 3]).mean_time_to_sum(Fraction(4))',
    'Die([0, 1.5]).mean_time_to_sum(3)',
    'Die([-1, 2]).mean_time_to_sum(0)',
    'Die([-1, 2]).mean_time_to_sum(3)',
    'Die([0]).mean_time_to_sum(2)',
    '(lambda x: (x.mean_time_to_sum(8), x.mean_time_to_sum(3), len(x._mean_time_to_sum_cache)))(Die([1, 3]))',
    '(lambda x: (x.mean_time_to_sum(8), x.mean_time_to_sum(True), x.mean_time_to_sum(-3)))(Die([1, 3]))',
    '(lambda x: (x.mean_time_to_sum(3), x._mean_time_to_sum_cache))(Die([1, 1, 4]))',
    'list(ip.iter_cartesian_product(d(2), P(1, 2), Deck([Card(1)])))',
    'list(ip.iter_cartesian_product(Pool([d(2), d(2)]), "x"))',
    'list(ip.iter_cartesian_product())',
    'list(ip.iter_cartesian_product(5))',
    'ip.cartesian_product(d(2), 3, outcome_type=P._make)',
    'ip.cartesian_product(Deck([1, 2]), Deck([E.A]), outcome_type=Vector)',
    'tupleize(Deck([Card(1)]), d(2).pool(1))',
    'tupleize(Deck([1, 2]), d(2))',
    'vectorize(d(2), Pool([d(2), d(2)]))',
    'tupleize(Deck([1, 2]), Reroll)',
    'ip.cartesian_product(d(2), outcome_type=lambda t: t[0] % 2)',
    # S5c: user generators (host sources), user operators and subclasses of built-in ones (host operators) on the
    # core driver.
    'Uniform(2, [1, 2, 3]).sum()',
    'Uniform(2, [1, 2, 3]).expand()',
    'Uniform(3, [1, 2, 3]).highest(2).sum()',
    'Uniform(2, [1, 2]).size()',
    '(Uniform(2, [1, 2]) + d(2).pool(1)).sum()',
    'Uniform(2, [E.A, E.B]).expand()',
    'Uniform(2, [Card(1), Card(2)]).expand()',
    'HostState().evaluate(Uniform(2, [1, 2, 3]))',
    'OrderSeen().evaluate(Uniform(2, [1, 2]), d(3).pool(1))',
    'Uniform(2, [1, 2, 3]).weightless().expand()',
    'Uniform(2, [1, 2, 3], (Order.Ascending, OrderReason.Mandatory)).sum()',
    'AscOnly().evaluate(Uniform(2, [1, 2, 3], (Order.Descending, OrderReason.Mandatory)))',
    'AscOnly().evaluate(Uniform(2, [1, 2, 3]))',
    'AscGen(2, [1, 2, 3]).sum()',
    'BoomGen(2, [1, 2]).sum()',
    'Uniform(0, []).sum()',
    'mf_user(Uniform(2, [1, 2]), d(2).pool(1))',
    'MultisetMixture([Uniform(1, [1, 2]), Uniform(1, [1, 2]), d(2).pool(1)]).sum()',
    'DoubleCounts(d(3).pool(2)).sum()',
    'DoubleCounts(d(3).pool(2)).size()',
    'DoubleCounts(Uniform(1, [1, 2])).expand()',
    'DoubleCounts(d(3).pool(2)).keep_counts(\'>=\', 4).sum()',
    'HostState().evaluate(DoubleCounts(d(3).pool(2)))',
    'MyUnion(d(3).pool(1), d(3).pool(2)).sum()',
    'MyUnion(d(3).pool(1), Uniform(1, [2, 4])).expand()',
    'AscOperator(d(3).pool(2)).sum()',
    'mf_user(DoubleCounts(d(2).pool(1)), d(2).pool(1))',
    'DoubleCounts(MultisetExpression.union(HANDS)).sum()',
    # K6e: argsort, argsort_grouped and lexi as native built-ins (exact Order / int parameters), else callbacks.
    'MultisetExpression.argsort(d(4).pool(2), d(4).pool(2), tie="drop")',
    'MultisetExpression.argsort(d(4).pool(2), d(4).pool(2), tie="left")',
    'MultisetExpression.argsort(d(3).pool(2), d(3).pool(1), d(3).pool(2), tie="right")',
    'MultisetExpression.argsort(d(4).pool(2), [2, 3], order=Order.Ascending, tie="drop")',
    'MultisetExpression.argsort(d(4).pool(3), d(4).pool(2), limit=2, tie="drop")',
    'MultisetExpression.argsort(d(4).pool(1), d(4).pool(1), limit=3, limit_pad=-1, tie="drop")',
    'MultisetExpression.argsort(d(4).pool(1), d(4).pool(1), tie="left", limit=3, limit_pad=2**70)',
    'MultisetExpression.argsort(d(4).pool(2), d(4).pool(1), tie="right", limit=1)',
    'MultisetExpression.argsort(d(4).pool(2), d(4).pool(1), limit=0, limit_pad=7, tie="drop")',
    'MultisetExpression.argsort(d(4).pool(2), d(4).pool(1), tie="left", limit=-1, limit_pad=7)',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), limit=2**70, limit_pad=0, tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), limit=2, limit_pad=E.A, tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), limit=2, limit_pad="x", tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), limit=E.B, tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), order=1, tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), order=E.A, tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), order=Order.Any, tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), order="x", tie="drop")',
    'MultisetExpression.argsort(d(3).pool(1), d(3).pool(1), order=Order.Ascending, tie="right")',
    'MultisetExpression.argsort(d(3).pool(2))',
    'MultisetExpression.argsort(d(3).pool(2), tie="left")',
    'MultisetExpression.argsort(Pool([Card(1), Card(2)]), Pool([Card(2)]), tie="drop")',
    'evaluator.ArgsortEvaluatorDrop().evaluate(d(3).pool(1), d(3).pool(1), foo=1)',
    'evaluator.ArgsortEvaluatorLeft().evaluate()',
    'MultisetExpression.argsort_grouped(d(4).pool(2), d(4).pool(2))',
    'MultisetExpression.argsort_grouped(d(4).pool(2), d(4).pool(1), d(2).pool(1), limit=2)',
    'MultisetExpression.argsort_grouped(d(4).pool(2), [1, 4], order=Order.Ascending, limit=1)',
    'MultisetExpression.argsort_grouped(d(4).pool(2), d(4).pool(1), limit=0)',
    'MultisetExpression.argsort_grouped(d(4).pool(2), d(4).pool(1), limit=-2)',
    'MultisetExpression.argsort_grouped(d(4).pool(2), d(4).pool(1), limit=2**70)',
    'MultisetExpression.argsort_grouped(d(4).pool(2), d(4).pool(1), order=-1)',
    'MultisetExpression.argsort_grouped(d(4).pool(2), d(4).pool(1), order=Order.Any)',
    'MultisetExpression.argsort(Deck([1, 2, 3]).deal((1, 1))[0], Deck([1, 2, 3]).deal((1, 1))[1], tie="drop")',
    'd(4).pool(3).leximin("<", d(4).pool(2))',
    'd(4).pool(3).leximax(">=", [2, 3], "drop")',
    'd(4).pool(2).leximin("==", d(4).pool(3), "low")',
    'd(4).pool(2).leximax("cmp", d(4).pool(2), "high")',
    '(-d(4).pool(2)).leximin("cmp", d(4).pool(2))',
    'd(4).pool(2)[1:].leximax("!=", d(6).pool(1))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Descending, lexi_tuple=(2**70, 2, 3, 4, -5))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=-1, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Any, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Ascending, lexi_tuple=(1, 2, E.A, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Ascending, lexi_tuple=[1, 2, 3, 4, 5])',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Ascending)',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), d(3).pool(1), sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(d(3).pool(2), d(3).pool(1), sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4, 5), x=1)',
    'multiset_function(lambda a, b: (MultisetExpression.argsort(a, b, limit=2, tie="drop"), a.leximax("<", b)))(d(4).pool(2), d(4).pool(2))',
    'multiset_function(lambda a, b: MultisetExpression.argsort_grouped(a + b, b, order=Order.Ascending))(d(3).pool(2), d(3).pool(1))',
    # S5d: the built-in evaluators over tuple counts (a MultiDeal, operators over one, a host operator) run natively
    # (evaluate_builtin / a builtin leaf of evaluate_tree); argsort and lexi since S5e.
    'evaluator.product_of_counts_evaluator.evaluate(HANDS)',
    'evaluator.empty_evaluator.evaluate(Deck([1, 2]).deal(()))',
    'evaluator.empty_evaluator.evaluate(HANDS, any=True)',
    'evaluator.ExpandEvaluator().evaluate(HANDS)',
    'evaluator.keep_evaluator.evaluate(HANDS, index=None)',
    'evaluator.keep_evaluator.evaluate(HANDS, index=0)',
    'evaluator.keep_evaluator.evaluate(HANDS, index=-1)',
    'evaluator.AllCountsEvaluator(filter="all").evaluate(HANDS)',
    'evaluator.AllCountsEvaluator().evaluate(HANDS)',
    'evaluator.LargestCountEvaluator().evaluate(HANDS)',
    'evaluator.LargestCountEvaluator().evaluate(Deck([1, 2]).deal(()))',
    'evaluator.largest_count_and_outcome_evaluator.evaluate(HANDS)',
    'evaluator.highest_outcome_and_count_evaluator.evaluate(HANDS)',
    'evaluator.largest_straight_evaluator.evaluate(HANDS)',
    'evaluator.LargestStraightAndOutcomeEvaluator("high").evaluate(HANDS)',
    'evaluator.LargestCountWithWildEvaluator([1], None, None).evaluate(HANDS)',
    'evaluator.CountSubsetEvaluator(empty_divisor=2).evaluate(HANDS, HANDS2)',
    'evaluator.CountSubsetEvaluator().evaluate(d(3).pool(2), Deck([1, 2]).deal((0, 0)))',
    'evaluator.IsSubsetEvaluator().evaluate(HANDS, HANDS2)',
    'evaluator.IsEqualSetEvaluator().evaluate(HANDS, HANDS)',
    'evaluator.IsProperSupersetEvaluator().evaluate(HANDS, HANDS2)',
    'evaluator.IsDisjointSetEvaluator().evaluate(HANDS, d(3).pool(1))',
    'evaluator.IsDisjointSetEvaluator().evaluate(Deck([1, 2]).deal((0, 0)), d(3).pool(1))',
    'MultisetExpression.union(HANDS).largest_count()',
    'MultisetExpression.union(HANDS).sum()',
    'MultisetExpression.additive_union(HANDS, HANDS2).sum()',
    'DoubleCounts(MultisetExpression.union(HANDS)).product_of_counts()',
    'DoubleCounts(MultisetExpression.union(HANDS)).issubset(MultisetExpression.union(HANDS2))',
    'multiset_function(lambda h: MultisetExpression.union(h).largest_count_and_outcome())(HANDS)',
    'multiset_function(lambda h, p: (MultisetExpression.union(h).sum(), p.sum()))(HANDS, d(3).pool(1))',
    'multiset_function(lambda h: evaluator.keep_evaluator.evaluate(h, index=1))(HANDS)',
    'MultisetExpression.argsort(MultisetExpression.union(HANDS), d(3).pool(1), tie="drop")',
    # S5e: argsort and lexi over tuple counts. Argsort looks at a tuple count only below the limit (and left /
    # right return once they reach it); lexi's next_state is `max(left, 0)`, its reverse order compares sizes.
    'evaluator.ArgsortEvaluatorDrop().evaluate(HANDS)',
    'evaluator.ArgsortEvaluatorDrop(limit=0, limit_pad=7).evaluate(HANDS, d(3).pool(1))',
    'evaluator.ArgsortEvaluatorDrop(order=Order.Ascending, limit=0).evaluate(HANDS)',
    'evaluator.ArgsortEvaluatorLeft(limit=1).evaluate(Pool([5, 5]), Deck([5, 5, 5]).deal((1, 1)))',
    'evaluator.ArgsortEvaluatorLeft(limit=1).evaluate(d(3).pool(2), HANDS)',
    'evaluator.ArgsortEvaluatorLeft().evaluate(HANDS)',
    'evaluator.ArgsortEvaluatorRight(limit=1).evaluate(Deck([5, 5, 5]).deal((1, 1)), Pool([5, 5]))',
    'evaluator.ArgsortEvaluatorRight(limit=-1, limit_pad=0).evaluate(HANDS, HANDS2)',
    'evaluator.ArgsortGroupedEvaluator(limit=0).evaluate(HANDS, d(3).pool(1))',
    'evaluator.ArgsortGroupedEvaluator().evaluate(Pool([3]), HANDS)',
    'evaluator.ArgsortGroupedEvaluator().evaluate(HANDS, Pool([3]))',
    'MultisetExpression.argsort_grouped(MultisetExpression.union(HANDS), limit=0)',
    'evaluator.lexi_comparison_evaluator.evaluate(HANDS, d(3).pool(2), sort_order=Order.Descending, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(HANDS, d(3).pool(2), sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(HANDS, HANDS2, sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(Deck([1]).deal(()), Deck([1]).deal(()), sort_order=Order.Descending, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(Deck([1]).deal(()), Deck([1]).deal(()), sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4, 5))',
    'evaluator.lexi_comparison_evaluator.evaluate(HANDS, d(3).pool(2).keep_outcomes([1]), sort_order=Order.Ascending, lexi_tuple=(1, 2, 3, 4, 5))',
    'multiset_function(lambda h, p: MultisetExpression.argsort(h, p, limit=0, limit_pad=1, tie="left"))(HANDS, d(3).pool(1))',
    'multiset_function(lambda h, p: (p.sum(), MultisetExpression.argsort_grouped(h, limit=0)))(HANDS, d(3).pool(1))',
    # R1: recursion on nested values raises RecursionError where Python's stack would overflow. Vector operators
    # are Python methods in icepool, 4 frames a level: at top level `+` raises from 250 levels. (Comparisons of
    # vectors nested 30 to 500 deep take exponential time in Python.)
    'nest_vector(249) + nest_vector(249)',
    'nest_vector(250) + nest_vector(250)',
    'nest_vector(249) + 1',
    'nest_vector(250) + 1',
    '1 - nest_vector(249)',
    '1 - nest_vector(250)',
    '-nest_vector(249)',
    '-nest_vector(250)',
    'round(nest_vector(249), 1)',
    'round(nest_vector(250), 1)',
    'nest_vector(10) < nest_vector(10, 2)',
    'nest_vector(600) == nest_vector(600)',
    'nest_vector(5000) + nest_vector(5000)',
    'nest_vector(100000) + 1',
    'abs(nest_vector(100000))',
    'nest_vector(249, E.A) + 1',
    'nest_vector(250, E.A) + 1',
    'nest_vector(148, Deep()) + 1',
    'nest_vector(149, Deep()) + 1',
    'nest_vector(150, Deep()) + 1',
    # `Die()` checks `arg in REROLL_TYPES`: `Vector.__eq__` broadcast, from 249 levels.
    'Die([nest_vector(248)])',
    'Die([nest_vector(249)])',
    'Die([nest_vector(100000)])',
    'Die([nest_vector(200)]) + Die([nest_vector(200)])',
    'str(Die([nest_vector(200)]))',
    'hash(nest_vector(5000))',
    # Nested tuples: CPython compares and reprs them in C, raising from about 10000 levels.
    'Die([nest_tuple(5000)])',
    'Die([nest_tuple(5000), nest_tuple(5000, 2)])',
    'str(Die([nest_tuple(5000)]))',
    'Die([nest_tuple(5000)]) + Die([nest_tuple(5000)])',
    'Die([nest_tuple(9000)]).map(lambda x: (x,))',
    'nest_tuple(20000) == nest_tuple(20000)',
    'str(Die([nest_tuple(20000)]))',
    'Die([nest_tuple(20000), nest_tuple(20000, 2)])',
]

SCRIPT = PRELUDE + '''
import json, sys
out = []
for src in json.loads(sys.argv[1]):
    try:
        out.append(items(eval(src)))
    except Exception as e:
        out.append('raises ' + type(e).__name__)
print(json.dumps(out))
'''


def run(pythonpath, backend):
    env = dict(os.environ, PYTHONPATH=pythonpath, ICEPOOL_BACKEND=backend)
    p = subprocess.run([sys.executable, '-c', SCRIPT, json.dumps(CASES)], env=env, capture_output=True,
                       text=True, check=True)
    return json.loads(p.stdout)


@pytest.fixture(scope='module')
def results():
    return run(os.path.join(ORACLE, 'src'), 'python'), run(os.path.join(ROOT, 'python'), 'rust')


@pytest.mark.parametrize('i', range(len(CASES)), ids=CASES)
def test_rust_facade_matches_oracle(results, i):
    expected, actual = results
    assert actual[i] == expected[i]
