__docformat__ = 'google'

import icepool
import icepool.math
import icepool.order
from icepool.generator.multiset_generator import MultisetGenerator
from icepool.generator.keep import KeepGenerator, KeepSource, pop_max_from_keep_tuple, pop_min_from_keep_tuple
from icepool.collection.counts import CountsKeysView
from icepool.order import Order, OrderReason

from icepool.typing import T


class Deal(KeepGenerator[T]):
    """Represents an unordered deal of a single hand from a `Deck`."""

    _deck: 'icepool.Deck[T]'

    def __init__(self, deck: 'icepool.Deck[T]', hand_size: int) -> None:
        """Constructor.

        For algorithmic reasons, you must pre-commit to the number of cards to
        deal.

        It is permissible to deal zero cards from an empty deck, but not all
        evaluators will handle this case, especially if they depend on the
        outcome type. Dealing zero cards from a non-empty deck does not have
        this issue.

        Args:
            deck: The `Deck` to deal from.
            hand_size: How many cards to deal.
        """
        # icepool-rs: the checks and the keep tuple are the Rust core's.
        if icepool.backend.RUST:
            self._deck = deck
            self._rs = icepool.backend.RUST.deal_new(deck._rust_deck(),
                                                     hand_size)
            self._keep_tuple = self._rs.keep_tuple()
            return
        if hand_size < 0:
            raise ValueError('hand_size cannot be negative.')
        if hand_size > deck.size():
            raise ValueError(
                'The number of cards dealt cannot exceed the size of the deck.'
            )
        self._deck = deck
        self._keep_tuple = (1, ) * hand_size

    @classmethod
    def _new_raw(cls, deck: 'icepool.Deck[T]',
                 keep_tuple: tuple[int, ...]) -> 'Deal[T]':
        self = super(Deal, cls).__new__(cls)
        self._deck = deck
        self._keep_tuple = keep_tuple
        return self

    @classmethod
    def _new_rust(cls, deck: 'icepool.Deck[T]', rs) -> 'Deal[T]':
        """icepool-rs: wraps a deal of the Rust core (`icepool_rs.Deal`) dealt from the facade's `deck`."""
        self = super(Deal, cls).__new__(cls)
        self._deck = deck
        self._rs = rs
        self._keep_tuple = rs.keep_tuple()
        return self

    def _make_source(self):
        # icepool-rs: the source pops outcomes in the Rust core.
        if icepool.backend.RUST:
            return DealSource._new_rust(self._rs.source())
        return DealSource(self._deck, self._keep_tuple)

    def _set_keep_tuple(self, keep_tuple: tuple[int, ...]) -> 'Deal[T]':
        # icepool-rs: a core deal with this keep tuple (`keep` and
        # `multiply_counts` make theirs in the Rust core, see below and keep.py).
        if icepool.backend.RUST:
            return Deal._new_rust(self._deck,
                                  self._rs.with_keep_tuple(keep_tuple))
        return Deal._new_raw(self._deck, keep_tuple)

    def multiply_counts(self, constant: int, /) -> 'Deal[T]':
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            return Deal._new_rust(self._deck,
                                  self._rs.multiply_counts(constant))
        return super().multiply_counts(constant)

    def deck(self) -> 'icepool.Deck[T]':
        """The `Deck` the cards are dealt from."""
        return self._deck

    def hand_size(self) -> int:
        """The number of cards dealt to each hand as a tuple."""
        return len(self._keep_tuple)

    def outcomes(self) -> CountsKeysView[T]:
        """The outcomes of the `Deck` in ascending order.

        These are also the `keys` of the `Deck` as a `Mapping`.
        Prefer to use the name `outcomes`.
        """
        return self.deck().outcomes()

    def denominator(self) -> int:
        # icepool-rs: computed in the Rust core.
        if icepool.backend.RUST:
            return self._rs.denominator()
        return icepool.math.comb(self.deck().size(), self.hand_size())

    @property
    def hash_key(self):
        # icepool-rs: the Rust deal hashes and compares its deck and keep
        # tuple, as this tuple does.
        if icepool.backend.RUST:
            return Deal, self._rs
        return Deal, self._deck, self._keep_tuple

    def __repr__(self) -> str:
        return type(
            self
        ).__qualname__ + f'({repr(self.deck())}, hand_size={self.hand_size()})'

    def __str__(self) -> str:
        return type(
            self
        ).__qualname__ + f' of hand_size={self.hand_size()} from deck:\n' + str(
            self.deck())


class DealSource(KeepSource[T]):

    def __init__(self, deck: 'icepool.Deck[T]', keep_tuple: tuple[int, ...]):
        self.deck = deck
        self.keep_tuple = keep_tuple

    @classmethod
    def _new_rust(cls, rs) -> 'DealSource[T]':
        """icepool-rs: wraps a source of the Rust core (`icepool_rs.DealSource`).

        `deck` and `keep_tuple` are converted from it on first use (see
        `__getattr__`); the evaluator only needs the outcomes.
        """
        self = super(DealSource, cls).__new__(cls)
        self._rs = rs
        return self

    def __getattr__(self, name: str):
        """icepool-rs: only called for an attribute not set yet; see `_new_rust`."""
        rs = self.__dict__.get('_rs')
        if rs is not None:
            if name == 'deck':
                self.deck = icepool.Deck._new_rust(rs.deck())
                return self.deck
            if name == 'keep_tuple':
                self.keep_tuple = rs.keep_tuple()
                return self.keep_tuple
        raise AttributeError(
            f"'{type(self).__name__}' object has no attribute '{name}'")

    def outcomes(self):
        # icepool-rs: read off the Rust core.
        if icepool.backend.RUST:
            return self._rs.outcomes()
        return self.deck.outcomes()

    def pop(self, order: Order, outcome: T):
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            for rs, count, weight in self._rs.pop(order, outcome):
                yield DealSource._new_rust(rs), count, weight
            return

        if not self.outcomes():
            yield self, 0, 1
            return

        if order > 0:
            pop_outcome = self.outcomes()[0]
            popped_deck, deck_count = self.deck._pop_min()
            pop_from_keep_tuple = pop_min_from_keep_tuple
        else:
            pop_outcome = self.outcomes()[-1]
            popped_deck, deck_count = self.deck._pop_max()
            pop_from_keep_tuple = pop_max_from_keep_tuple

        if outcome != pop_outcome:
            yield self, 0, 1
            return

        if order > 0:
            popped_deck, deck_count = self.deck._pop_min()
            pop_from_keep_tuple = pop_min_from_keep_tuple
        else:
            popped_deck, deck_count = self.deck._pop_max()
            pop_from_keep_tuple = pop_max_from_keep_tuple

        min_count = max(0,
                        deck_count + len(self.keep_tuple) - self.deck.size())
        max_count = min(deck_count, len(self.keep_tuple))
        skip_weight = None

        popped_deal: DealSource[T]
        for count in range(min_count, max_count + 1):
            popped_keep_tuple, result_count = pop_from_keep_tuple(
                self.keep_tuple, count)
            popped_deal = DealSource(popped_deck, popped_keep_tuple)
            weight = icepool.math.comb(deck_count, count)
            if not any(popped_keep_tuple):
                # Dump all dice in exchange for the denominator.
                skip_weight = (skip_weight
                               or 0) + weight * popped_deal.denominator()
                continue
            yield popped_deal, result_count, weight

        if skip_weight is not None:
            popped_deal = DealSource(popped_deck, ())
            yield popped_deal, sum(self.keep_tuple), skip_weight

    def order_preference(self) -> tuple[Order, OrderReason]:
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            order, reason = self._rs.order_preference()
            return Order(order), OrderReason(reason)
        lo_skip, hi_skip = icepool.order.lo_hi_skip(self.keep_tuple)
        if lo_skip > hi_skip:
            return Order.Descending, OrderReason.KeepSkip
        if hi_skip > lo_skip:
            return Order.Ascending, OrderReason.KeepSkip

        return Order.Any, OrderReason.NoPreference

    @property
    def hash_key(self):
        # icepool-rs: the Rust source hashes and compares its deck and keep
        # tuple, as this tuple does.
        if icepool.backend.RUST:
            return DealSource, self._rs
        return DealSource, self.deck, self.keep_tuple

    def denominator(self) -> int:
        # icepool-rs: computed in the Rust core.
        if icepool.backend.RUST:
            return self._rs.denominator()
        return icepool.math.comb(self.deck.size(), len(self.keep_tuple))

    def size(self):
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            return self._rs.size()
        return super().size()

    def is_resolvable(self) -> bool:
        # icepool-rs: runs in the Rust core.
        if icepool.backend.RUST:
            return self._rs.is_resolvable()
        return len(self.outcomes()) != 0
