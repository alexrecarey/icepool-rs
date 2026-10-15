__docformat__ = 'google'

import icepool

from icepool.generator.multiset_generator import MultisetGenerator, MultisetSource

from icepool.order import Order, OrderReason, UnsupportedOrder
from icepool.typing import ImplicitConversionError, T


class WeightlessGenerator(MultisetGenerator[T]):
    """EXPERIMENTAL: A generator wrapper that sets all weights to 1.
    
    This only works properly if the wrapped generator never takes multiple paths
    to the same output multiset.
    """

    def __init__(self, wrapped: MultisetGenerator[T], /):
        self._wrapped = wrapped

    def _make_source(self):
        return WeightlessSource(self._wrapped._make_source())

    @property
    def _static_keepable(self) -> bool:
        return False

    @property
    def hash_key(self):
        return WeightlessGenerator, self._wrapped.hash_key


class WeightlessSource(MultisetSource[T]):

    def __init__(self, wrapped: MultisetSource[T], /):
        self._wrapped = wrapped
        # icepool-rs: the source pops outcomes in the Rust core, over the
        # wrapped source's (every built-in generator's source has one; a
        # source without is not the core's, and evaluations over it stay on
        # upstream's driver, see evaluator_rust._core_source).
        if icepool.backend.RUST and '_rs' in wrapped.__dict__:
            self._rs = icepool.backend.RUST.weightless_source(wrapped._rs)

    @classmethod
    def _new_rust(cls, rs) -> 'WeightlessSource[T]':
        """icepool-rs: wraps a source of the Rust core (`icepool_rs.WeightlessSource`).

        `_wrapped` is converted from it on first use (see `__getattr__`); the
        evaluator only needs the outcomes.
        """
        self = super(WeightlessSource, cls).__new__(cls)
        self._rs = rs
        return self

    def __getattr__(self, name: str):
        """icepool-rs: only called for an attribute not set yet; see `_new_rust`."""
        rs = self.__dict__.get('_rs')
        if rs is not None and name == '_wrapped':
            from icepool.generator.compound_keep import _wrap_rust_source
            self._wrapped = _wrap_rust_source(rs.wrapped())
            return self._wrapped
        raise AttributeError(
            f"'{type(self).__name__}' object has no attribute '{name}'")

    def outcomes(self):
        # icepool-rs: read off the Rust core.
        if '_rs' in self.__dict__:
            return self._rs.outcomes()
        return self._wrapped.outcomes()

    def pop(self, order, outcome):
        # icepool-rs: runs in the Rust core (UnsupportedOrder included).
        if '_rs' in self.__dict__:
            for rs, count, weight in self._rs.pop(order, outcome):
                yield WeightlessSource._new_rust(rs), count, weight
            return
        seen_counts = set()
        for source, count, weight in self._wrapped.pop(order, outcome):
            if count in seen_counts:
                raise UnsupportedOrder(
                    'weightless cannot handle calls to pop() that produce the same count multiple times.'
                )
            seen_counts.add(count)
            yield WeightlessSource(source), count, 1

    def size(self):
        # icepool-rs: runs in the Rust core.
        if '_rs' in self.__dict__:
            return self._rs.size()
        return self._wrapped.size()

    def order_preference(self):
        # icepool-rs: runs in the Rust core.
        if '_rs' in self.__dict__:
            order, reason = self._rs.order_preference()
            return Order(order), OrderReason(reason)
        return self._wrapped.order_preference()

    def is_resolvable(self):
        # icepool-rs: runs in the Rust core.
        if '_rs' in self.__dict__:
            return self._rs.is_resolvable()
        return self._wrapped.is_resolvable()

    @property
    def hash_key(self):
        # icepool-rs: the Rust source hashes and compares its wrapped source,
        # as this tuple does.
        if '_rs' in self.__dict__:
            return WeightlessSource, self._rs
        return WeightlessSource, self._wrapped.hash_key
