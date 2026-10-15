__docformat__ = 'google'

import icepool
from icepool.generator.keep import KeepGenerator, KeepSource, pop_max_from_keep_tuple, pop_min_from_keep_tuple
from icepool.generator.multiset_generator import MultisetSource
from icepool.order import Order, OrderReason, merge_order_preferences

import itertools
import math

from typing import Sequence
from icepool.typing import T


class CompoundKeepGenerator(KeepGenerator[T]):
    _inner_generators: tuple[KeepGenerator[T], ...]

    def __init__(self, inners: Sequence[KeepGenerator[T]],
                 keep_tuple: Sequence[int]):
        self._inner_generators = tuple(inners)
        # icepool-rs: the generator is the Rust core's, over the inners'
        # sources; keeping, multiplying counts and popping run there.
        if icepool.backend.RUST:
            self._rs = icepool.backend.RUST.compound_keep_new(
                [_rust_source(inner._make_source()) for inner in self._inner_generators],
                list(keep_tuple))
            self._keep_tuple = self._rs.keep_tuple()
            return
        self._keep_tuple = tuple(keep_tuple)

    @classmethod
    def _new_rust(cls, inners: tuple[KeepGenerator[T], ...],
                  rs) -> 'CompoundKeepGenerator[T]':
        """icepool-rs: wraps a compound keep of the Rust core (`icepool_rs.CompoundKeep`) over the facade's `inners`."""
        self = super(CompoundKeepGenerator, cls).__new__(cls)
        self._inner_generators = inners
        self._rs = rs
        self._keep_tuple = rs.keep_tuple()
        return self

    def _make_source(self):
        # icepool-rs: the source pops outcomes in the Rust core.
        if icepool.backend.RUST:
            return CompoundKeepSource._new_rust(self._rs.source())
        inner_sources = tuple(generator._make_source()
                              for generator in self._inner_generators)
        return CompoundKeepSource(inner_sources, self._keep_tuple)

    def _set_keep_tuple(self, keep_tuple: tuple[int,
                                                ...]) -> 'KeepGenerator[T]':
        # icepool-rs: a core compound keep with this keep tuple.
        if icepool.backend.RUST:
            return CompoundKeepGenerator._new_rust(
                self._inner_generators,
                self._rs.with_keep_tuple(list(keep_tuple)))
        return CompoundKeepGenerator(self._inner_generators, keep_tuple)

    def multiply_counts(self, constant: int, /) -> 'KeepGenerator[T]':
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            return CompoundKeepGenerator._new_rust(
                self._inner_generators, self._rs.multiply_counts(constant))
        return super().multiply_counts(constant)

    @property
    def hash_key(self):
        # icepool-rs: the Rust compound keep hashes and compares its inners'
        # sources (which compare as their generators do) and keep tuple.
        if icepool.backend.RUST:
            return CompoundKeepGenerator, self._rs
        return CompoundKeepGenerator, self._inner_generators, self._keep_tuple

    def __str__(self) -> str:
        return ('CompoundKeep([' +
                ', '.join(str(inner) for inner in self._inner_generators) +
                '], keep_tuple=' + str(self.keep_tuple()) + ')')


def _rust_source(source):
    """icepool-rs: what the Rust core takes for a facade source: its `_rs`, or else the source itself, which the
    core calls back as a host source (a user's `MultisetSource` subclass; see crates/icepool-py/src/host_source.rs).
    """
    return getattr(source, '__dict__', {}).get('_rs', source)


def _wrap_rust_source(rs):
    """icepool-rs: a facade source around a source of the Rust core (a host source is its own Python object)."""
    rust = icepool.backend.RUST
    if isinstance(rs, rust.PoolSource):
        return icepool.generator.pool.PoolSource._new_rust(rs)
    if isinstance(rs, rust.DealSource):
        return icepool.generator.deal.DealSource._new_rust(rs)
    if isinstance(rs, rust.MultiDealSource):
        return icepool.generator.multi_deal.MultiDealSource._new_rust(rs)
    if isinstance(rs, rust.WeightlessSource):
        return icepool.generator.weightless.WeightlessSource._new_rust(rs)
    if isinstance(rs, rust.CompoundKeepSource):
        return CompoundKeepSource._new_rust(rs)
    return rs


class CompoundKeepSource(KeepSource[T]):

    def __init__(self, inner_sources: tuple[MultisetSource, ...],
                 keep_tuple: tuple[int, ...]):
        self.inner_sources = inner_sources
        self.keep_tuple = keep_tuple

    @classmethod
    def _new_rust(cls, rs) -> 'CompoundKeepSource[T]':
        """icepool-rs: wraps a source of the Rust core (`icepool_rs.CompoundKeepSource`).

        `inner_sources` and `keep_tuple` are converted from it on first use
        (see `__getattr__`); the evaluator only needs the outcomes.
        """
        self = super(CompoundKeepSource, cls).__new__(cls)
        self._rs = rs
        return self

    def __getattr__(self, name: str):
        """icepool-rs: only called for an attribute not set yet; see `_new_rust`."""
        rs = self.__dict__.get('_rs')
        if rs is not None:
            if name == 'inner_sources':
                self.inner_sources = tuple(
                    _wrap_rust_source(s) for s in rs.inner_sources())
                return self.inner_sources
            if name == 'keep_tuple':
                self.keep_tuple = rs.keep_tuple()
                return self.keep_tuple
        raise AttributeError(
            f"'{type(self).__name__}' object has no attribute '{name}'")

    def outcomes(self):
        # icepool-rs: read off the Rust core.
        if icepool.backend.RUST:
            return self._rs.outcomes()
        return icepool.sorted_union(*(inner.outcomes()
                                      for inner in self.inner_sources))

    def pop(self, order: Order, outcome: T):
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            for rs, count, weight in self._rs.pop(order, outcome):
                yield CompoundKeepSource._new_rust(rs), count, weight
            return

        if order > 0:
            pop_from_keep_tuple = pop_min_from_keep_tuple
        else:
            pop_from_keep_tuple = pop_max_from_keep_tuple

        for t in itertools.product(*(inner.pop(order, outcome)
                                     for inner in self.inner_sources)):
            sources, counts, weights = zip(*t)
            total_count = sum(counts)
            total_weight = math.prod(weights)
            popped_keep_tuple, result_count = pop_from_keep_tuple(
                self.keep_tuple, total_count)
            yield CompoundKeepSource(
                sources, popped_keep_tuple), result_count, total_weight

    def size(self):
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            return self._rs.size()
        return super().size()

    def order_preference(self) -> tuple[Order, OrderReason]:
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            order, reason = self._rs.order_preference()
            return Order(order), OrderReason(reason)
        return merge_order_preferences(*(inner.order_preference()
                                         for inner in self.inner_sources))

    def is_resolvable(self) -> bool:
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            return self._rs.is_resolvable()
        return all(inner.is_resolvable() for inner in self.inner_sources)

    @property
    def hash_key(self):
        # icepool-rs: the Rust source hashes and compares its inner sources
        # and keep tuple, as this tuple does.
        if icepool.backend.RUST:
            return CompoundKeepSource, self._rs
        return CompoundKeepSource, self.inner_sources, self.keep_tuple
