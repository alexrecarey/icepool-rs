"""The facade on the Rust backend against the oracle, for what the program language cannot express: host outcomes
(enums, namedtuples, user classes, Vector, Symbols), exceptions raised by them, big ints, callables with extra
arguments, user-defined evaluators (host states, keyword arguments, UnsupportedOrder subclasses).

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
