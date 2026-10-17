"""Named random variables for `icepool.from_rv` (Python side), duck-typed like `scipy.stats` ones: `cdf` takes a
sequence and returns a list, and a continuous one has a `pdf` attribute (from_rv tests `hasattr(rv, 'pdf')`).

Rust side: crates/icepool-harness/src/function.rs `Rv`, same formulas with the same Python operators.
"""
import math


class DiscreteUniform:
    """The ints lo..hi-1 (hi excluded), like `scipy.stats.randint(lo, hi)`."""

    def __init__(self, lo, hi):
        self.lo, self.hi = lo, hi

    def cdf(self, xs):
        return [min(max((math.floor(x) - self.lo + 1) / (self.hi - self.lo), 0.0), 1.0) for x in xs]


class ContinuousUniform:
    """Uniform on [lo, hi]."""

    def __init__(self, lo, hi):
        self.lo, self.hi = lo, hi

    def pdf(self, x):
        return 1 / (self.hi - self.lo) if self.lo <= x <= self.hi else 0.0

    def cdf(self, xs):
        return [min(max((x - self.lo) / (self.hi - self.lo), 0.0), 1.0) for x in xs]


RVS = {"discrete_uniform": DiscreteUniform, "continuous_uniform": ContinuousUniform}
