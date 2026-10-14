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
