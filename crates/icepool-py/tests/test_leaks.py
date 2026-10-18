"""Memory on the Rust backend: reference cycles through the core are collected, and long loops do not grow.

    uv run python -m pytest crates/icepool-py/tests -q -p no:cacheprovider

The core holds Python objects (outcomes the core has no variant for, users' multiset sources) inside its own
values, and the `icepool_rs` classes report them to Python's cycle collector (`__traverse__`, see
crates/icepool-core/src/ownership.rs). Without that, a cycle through one (an outcome referring back to its die)
was never freed. Each case runs in a subprocess on the Rust backend, so no other test's objects interfere.
"""
import os
import subprocess
import sys

import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
ENV = dict(os.environ, PYTHONPATH=os.path.join(ROOT, 'python'), ICEPOOL_BACKEND='rust')

PRELUDE = '''
import functools, gc, math, weakref
import icepool
from icepool import *
from icepool.generator.multiset_generator import MultisetSource
from icepool.generator.weightless import WeightlessSource
from icepool.order import OrderReason
assert icepool.backend.RUST, 'not the Rust backend'

@functools.total_ordering
class O:
    """An outcome the core holds as a host object, with a slot for a reference back."""
    def __init__(self, v):
        self.v = v
        self.ref = None
    def __hash__(self):
        return hash(self.v)
    def __eq__(self, other):
        return isinstance(other, O) and self.v == other.v
    def __lt__(self, other):
        return self.v < other.v
    def __repr__(self):
        return f'O({self.v})'

class Src(MultisetSource):
    """A user's source: two dice' worth of one outcome."""
    def __init__(self, outcomes):
        self._outcomes = tuple(outcomes)
        self.ref = None
    def outcomes(self):
        return self._outcomes
    def size(self):
        return 2
    def pop(self, order, outcome):
        yield Src(self._outcomes[1:]), 2 if len(self._outcomes) == 1 else 0, 1
    def order_preference(self):
        return Order.Any, OrderReason.NoPreference
    def is_resolvable(self):
        return True
    @property
    def hash_key(self):
        return Src, self._outcomes

def collected(make):
    """Whether `make(o)`'s result, with `o.ref` pointing back at it, is freed once both are unreachable."""
    o = O(1)
    x = make(o)
    o.ref = x
    alive = weakref.ref(o)
    del o, x
    gc.collect()
    return alive() is None
'''

# Each makes an object of the facade that holds `o` through the core. Still not collected: a cycle through two
# objects sharing core data that is not structural, such as a die and its popped die (`_pop_min`, cached on both
# sides), or a source (`_make_source()`) and the generator it came from; see dev/NOTES.md.
CYCLES = {
    'die': 'Die([o, O(2)])',
    'mapped die': 'd(2).map(lambda x: o)',
    'mixture': 'Die([Die([o]), Die([O(2), O(3)])])',
    'evaluated die': 'evaluated(Die([o, O(2), O(3)]))',
    'tuple outcome': 'Die([(o, 1), (O(2), 2)])',
    'vector outcome': 'Die([Vector([o, 1])])',
    'pool': 'Die([o, O(2)]).pool(3)',
    'pool source': 'Die([o, O(2)]).pool(3)._make_source()',
    'deck': 'Deck([o, O(2)])',
    'deal': 'Deck([o, O(2), O(3)]).deal(2)',
    'multi deal': 'Deck([o, O(2), O(3)]).deal((1, 1))',
    'merged pool': 'Die([o, O(2)]).pool(1) + Die([O(3)]).pool(1)',
    # A compound keep holds its inners' `_rs` objects, not copies of their dice or cards (R4b).
    'compound keep': 'Die([o, O(2)]).pool(2).highest(1) + Die([O(3)]).pool(1)',
    'compound keep of a deal': 'Deck([o, O(2)]).deal(1) + Die([O(3)]).pool(1)',
    'nested compound keep': '(Die([o, O(2)]).pool(2).highest(1) + Die([O(3)]).pool(1) + Deck([O(4)]).deal(1))[1:]',
    'compound keep source': '(Die([o, O(2)]).pool(2).highest(1) + Die([O(3)]).pool(1))._make_source()',
    # A pool holds its dice's `_rs` objects, not copies of the dice (R4b): the die and the pool, both alive.
    'a die and a pool of it': '(die := Die([o, O(2)]), die.pool(3))',
    'a die put in zero times': '(die := Die([o, O(2)]), Pool([die, Die([O(3)])], [0, 2]))',
    'a pool and a keep of it': '(p := Die([o, O(2)]).pool(3), p.highest(2), -p)',
    "a pool and its facade's dice": '(p := Pool([Die([o, O(2)]), Die([O(3)])]), p._dice)',
    'a pool and a merge of it': '(p := Die([o, O(2)]).pool(1), p + Die([O(3)]).pool(2))',
    'a die and a compound keep of it': '(die := Die([o, O(2)]), die.pool(2).highest(1) + Die([O(3)]).pool(1))',
    'a user source': 'weightless_of(o)',
}


@pytest.mark.parametrize('name', CYCLES)
def test_a_cycle_through_the_core_is_collected(name):
    script = PRELUDE + '''
def evaluated(die):
    die.pool(2).expand(), die.pool(3).highest(1).expand(), die.quantity('<=', O(2))
    return die

def weightless_of(o):
    src = Src([o])
    w = WeightlessSource(src)
    src.ref = w  # src <-> w through the core's host source
    return w

print(collected(lambda o: %s))
''' % CYCLES[name]
    r = subprocess.run([sys.executable, '-c', script], env=ENV, capture_output=True, text=True, timeout=120)
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip() == 'True', f'{name}: the cycle was not collected'


def test_objects_shared_with_live_dice_survive_collection():
    """A collection never frees what a live die still holds: the dying die shares `o` with a live one."""
    script = PRELUDE + '''
o = O(1)
dying = Die([o, O(2)])
live = dying.map(lambda x: x)  # shares the outcome objects with `dying` in the core
o.ref = dying
dying.pool(2).expand()
del o, dying
for _ in range(3):
    gc.collect()
first = live.outcomes()[0]
assert first == O(1) and first.ref is not None, first.__dict__
assert live.pool(2).expand().quantities() == (1, 2, 1), live.pool(2).expand()
print('ok')
'''
    r = subprocess.run([sys.executable, '-c', script], env=ENV, capture_output=True, text=True, timeout=120)
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip() == 'ok'


def test_collections_during_core_work_are_harmless():
    """The collector running at every allocation, including inside callbacks from the core, while the core holds
    clones of host outcomes: results are the same as without it."""
    script = PRELUDE + '''
def work():
    out = []
    for i in range(30):
        die = Die([O(j) for j in range(4)])
        mapped = die.map(lambda x: O(x.v % 3))
        out.append(repr(mapped.pool(3).highest(2).expand()))
        out.append(repr((d(4) + i).map(lambda x: (x, O(x % 2)))))
        out.append(repr(Deck([O(1), O(2), O(2)]).deal(2).expand()))
    return out
expected = work()
gc.set_threshold(1, 1, 1)
assert work() == expected
print('ok')
'''
    r = subprocess.run([sys.executable, '-c', script], env=ENV, capture_output=True, text=True, timeout=300)
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip() == 'ok'


def test_a_long_loop_does_not_grow():
    """The review's loop (a map with a callback holding a big default, then a pool sum), many times: after a
    warm-up (icepool's caches, lazy imports) neither traced allocations nor the resident set keep growing."""
    script = '''
import gc, resource, tracemalloc
import icepool
from icepool import d
assert icepool.backend.RUST

big = list(range(10000))
def step():
    d(6).map(lambda o, big=big: o)
    d(6).pool(3).sum()

for _ in range(1000):
    step()
gc.collect()
tracemalloc.start()
rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss  # KiB on Linux
before = tracemalloc.get_traced_memory()[0]
for _ in range(20000):
    step()
gc.collect()
print(tracemalloc.get_traced_memory()[0] - before, resource.getrusage(resource.RUSAGE_SELF).ru_maxrss - rss)
'''
    r = subprocess.run([sys.executable, '-c', script], env=ENV, capture_output=True, text=True, timeout=300)
    assert r.returncode == 0, r.stderr
    traced, rss = map(int, r.stdout.split())
    # 20000 leaked iterations would be megabytes (each holds a closure, a die and a pool); a few cache entries
    # are kilobytes. ru_maxrss also counts tracemalloc's own bookkeeping.
    assert traced < 100_000, f'{traced} bytes still traced after the loop'
    assert rss < 4096, f'maxrss grew {rss} KiB'
