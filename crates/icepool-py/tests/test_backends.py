"""The facade on the Rust backend against the oracle, for what the program language cannot express: host outcomes
(enums, namedtuples, user classes, Vector, Symbols), exceptions raised by them, big ints, callables with extra
arguments.

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
