"""Named multiset evaluators (Python side): custom `MultisetEvaluator` subclasses for the `evaluate` node.

Rust side: crates/icepool-core/src/program/catalogue_evaluators.rs, one variant per entry, same parameters, same
bodies. A program names one as {"op": "evaluate", "name": "count_eq", "params": [3], "args": [<pool>, ...]}, which
is EVALUATORS["count_eq"](3).evaluate(*args).

Together they reach the parts of the driver the built-in evaluators do not: `Reroll` from next_state and from
final_outcome, a final outcome that is a Die, UnsupportedOrder (the flipped and the forward algorithm, and
ConflictingOrderError), extra_outcomes, sizes, several inputs, and a next_state that sees the order.
"""
from dataclasses import dataclass
from typing import Callable

import icepool
from icepool import MultisetEvaluator, UnsupportedOrder


@dataclass
class Entry:
    cls: type
    params: Callable                    # params(g) -> list of JSON program values
    inputs: Callable                    # inputs(g) -> how many multisets to pass


EVALUATORS: dict[str, type] = {}
ENTRIES: dict[str, Entry] = {}


def evaluator(name, params=lambda g: [], inputs=lambda g: 1 if g.rng.random() < 0.95 else 2):
    def deco(cls):
        EVALUATORS[name] = cls
        ENTRIES[name] = Entry(cls, params, inputs)
        return cls
    return deco


@evaluator("count_eq", params=lambda g: [g.outcome()])
class CountEq(MultisetEvaluator):
    """How many elements equal k (counts as they come, so negative ones subtract)."""

    def __init__(self, k):
        self.k = k

    def initial_state(self, order, outcomes, *sizes):
        return 0

    def next_state(self, state, order, outcome, count):
        return state + count if outcome == self.k else state


@evaluator("reroll_at_least", params=lambda g: [g.rng.choice([1, 2, 2, 3])])
class RerollAtLeast(MultisetEvaluator):
    """Rerolls the whole multiset if any outcome comes up k or more times; otherwise the size. With no outcomes at
    all the final state is None (TypeError)."""

    def __init__(self, k):
        self.k = k

    def next_state(self, state, order, outcome, count):
        if count >= self.k:
            return icepool.Reroll
        return (state or 0) + count


@evaluator("final_die", params=lambda g: [g.rng.randint(0, 4)])
class FinalDie(MultisetEvaluator):
    """The size s, then: Reroll if s is 0, None (TypeError) if negative, d(s) if s <= n, else s itself."""

    def __init__(self, n):
        self.n = n

    def initial_state(self, order, outcomes, *sizes):
        return 0

    def next_state(self, state, order, outcome, count):
        return state + count

    def final_outcome(self, final_state, order, outcomes, *sizes):
        if final_state == 0:
            return icepool.Reroll
        if final_state < 0:
            return None
        if final_state <= self.n:
            return icepool.d(final_state)
        return final_state


@evaluator("seen", params=lambda g: [g.rng.choice([0, 1, 1, -1, -1, 2])],
           inputs=lambda g: g.rng.choice([1, 1, 1, 2]))
class Seen(MultisetEvaluator):
    """The outcomes with a positive count in any input, in the order next_state sees them.

    req 1: ascending only (UnsupportedOrder from initial_state otherwise); -1: descending only (UnsupportedOrder from
    next_state); 2: no order at all (ConflictingOrderError); 0: any order."""

    def __init__(self, req):
        self.req = req

    def initial_state(self, order, outcomes, *sizes):
        if self.req == 2 or (self.req == 1 and order < 0):
            raise UnsupportedOrder()
        return ()

    def next_state(self, state, order, outcome, *counts):
        if self.req == -1 and order > 0:
            raise UnsupportedOrder()
        if any(c > 0 for c in counts):
            return state + (outcome,)
        return state


@evaluator("count_diff", inputs=lambda g: g.rng.choice([1, 2, 2, 3]))
class CountDiff(MultisetEvaluator):
    """(sum of outcome * (first count - the other counts), the input sizes): sizes are None for negative counts."""

    def initial_state(self, order, outcomes, *sizes):
        return 0

    def next_state(self, state, order, outcome, *counts):
        return state + outcome * (counts[0] - sum(counts[1:]))

    def final_outcome(self, final_state, order, outcomes, *sizes):
        return (final_state,) + sizes


@evaluator("zero_runs")
class ZeroRuns(MultisetEvaluator):
    """How many outcomes between the min and the max (consecutive ints) have a zero count."""

    extra_outcomes = MultisetEvaluator.consecutive

    def initial_state(self, order, outcomes, *sizes):
        return 0

    def next_state(self, state, order, outcome, count):
        return state + (1 if count == 0 else 0)
