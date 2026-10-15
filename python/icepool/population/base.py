__docformat__ = 'google'

import icepool
from icepool.expand import Expandable
from icepool.collection.counts import CountsKeysView, CountsValuesView, CountsItemsView
from icepool.collection.vector import Vector
from icepool.math import try_fraction
from icepool.typing import U, T_co, infer_star

from abc import ABC, abstractmethod
import bisect
from collections import Counter, defaultdict
from fractions import Fraction
from functools import cached_property
import itertools
import math
import numbers
import operator
import random

from typing import Any, Callable, Collection, Generic, Hashable, Iterator, Literal, Mapping, MutableMapping, Sequence, Set, Sized, TypeVar, cast, overload

C = TypeVar('C', bound='Population')
"""Type variable representing a subclass of `Population`."""


# This typing is a compromise due to Mapping not support covariant key type.
def _rust_population(x) -> bool:
    """icepool-rs: whether `x` is a population whose `Population` methods run in the Rust core: a `Die`, or a
    `Deck` (its cards are a die's `Counts` to the core, see `Deck._rust_die`)."""
    return isinstance(x, (icepool.Die, icepool.Deck))


class Population(ABC, Expandable[T_co], Mapping[Any, int]):
    """A mapping from outcomes to `int` quantities.

    Outcomes with each instance must be hashable and totally orderable.

    Subclasses include `Die` and `Deck`.
    """

    # Abstract methods.

    @property
    @abstractmethod
    def _new_type(self) -> type:
        """The type to use when constructing a new instance."""

    @abstractmethod
    def keys(self) -> CountsKeysView[T_co]:
        """The outcomes within the population in sorted order."""

    @abstractmethod
    def values(self) -> CountsValuesView:
        """The quantities within the population in outcome order."""

    @abstractmethod
    def items(self) -> CountsItemsView[T_co]:
        """The (outcome, quantity)s of the population in sorted order."""

    @property
    def _items_for_cartesian_product(self) -> Sequence[tuple[T_co, int]]:
        return self.items()

    def _unary_operator(self, op: Callable, *args, **kwargs):
        data: MutableMapping[Any, int] = defaultdict(int)
        for outcome, quantity in self.items():
            new_outcome = op(outcome, *args, **kwargs)
            data[new_outcome] += quantity
        return self._new_type(data)

    # Outcomes.

    def outcomes(self) -> CountsKeysView[T_co]:
        """The outcomes of the mapping in ascending order.

        These are also the `keys` of the mapping.
        Prefer to use the name `outcomes`.
        """
        return self.keys()

    @cached_property
    def _common_outcome_length(self) -> int | None:
        # icepool-rs: a die's runs in the Rust core (format.rs).
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.common_outcome_length(
                self._rust_die())
        result = None
        for outcome in self.outcomes():
            if isinstance(outcome, Mapping):
                return None
            elif isinstance(outcome, Sized):
                if result is None:
                    result = len(outcome)
                elif len(outcome) != result:
                    return None
        return result

    def common_outcome_length(self) -> int | None:
        """The common length of all outcomes.

        If outcomes have no lengths or different lengths, the result is `None`.
        """
        return self._common_outcome_length

    def is_empty(self) -> bool:
        """`True` iff this population has no outcomes. """
        # icepool-rs: for a die, the statistics in this file run in the Rust
        # core (population.rs); a Deck stays Python until S4a.
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.is_empty(self._rust_die())
        return len(self) == 0

    def min_outcome(self) -> T_co:
        """The least outcome."""
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.min_outcome(self._rust_die())
        return self.outcomes()[0]

    def max_outcome(self) -> T_co:
        """The greatest outcome."""
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.max_outcome(self._rust_die())
        return self.outcomes()[-1]

    def nearest(self, comparison: Literal['<=', '<', '>=', '>'], outcome,
                /) -> T_co | None:
        """The nearest outcome in this population fitting the comparison.

        Args:
            comparison: The comparison which the result must fit. For example,
                '<=' would find the greatest outcome that is not greater than
                the argument.
            outcome: The outcome to compare against.
        
        Returns:
            The nearest outcome fitting the comparison, or `None` if there is
            no such outcome.
        """
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.nearest(self._rust_die(), comparison,
                                                outcome)
        match comparison:
            case '<=':
                if outcome in self:
                    return outcome
                index = bisect.bisect_right(self.outcomes(), outcome) - 1
                if index < 0:
                    return None
                return self.outcomes()[index]
            case '<':
                index = bisect.bisect_left(self.outcomes(), outcome) - 1
                if index < 0:
                    return None
                return self.outcomes()[index]
            case '>=':
                if outcome in self:
                    return outcome
                index = bisect.bisect_left(self.outcomes(), outcome)
                if index >= len(self):
                    return None
                return self.outcomes()[index]
            case '>':
                index = bisect.bisect_right(self.outcomes(), outcome)
                if index >= len(self):
                    return None
                return self.outcomes()[index]
            case _:
                raise ValueError(f'Invalid comparison {comparison}')

    @staticmethod
    def _zero(x):
        return x * 0

    def zero(self: C) -> C:
        """Zeros all outcomes of this population.

        This is done by multiplying all outcomes by `0`.

        The result will have the same denominator.

        Raises:
            ValueError: If the zeros did not resolve to a single outcome.
        """
        # icepool-rs: zero runs in the Rust core (`x * 0` per outcome), for a
        # die or a deck.
        if icepool.backend.RUST and isinstance(self, icepool.Die):
            return icepool.Die._new_rust(
                icepool.backend.RUST.zero(self._rust_die()))
        if icepool.backend.RUST and isinstance(self, icepool.Deck):
            return icepool.Deck._new_rust(
                icepool.backend.RUST.deck_zero(self._rust_deck()))
        result = self._unary_operator(Population._zero)
        if len(result) != 1:
            raise ValueError('zero() did not resolve to a single outcome.')
        return result

    def zero_outcome(self) -> T_co:
        """A zero-outcome for this population.

        E.g. `0` for a `Population` whose outcomes are `int`s.
        """
        return self.zero().outcomes()[0]

    # Quantities.

    @overload
    def quantity(self, outcome: Hashable, /) -> int:
        """The quantity of a single outcome."""

    @overload
    def quantity(self, comparison: Literal['==', '!=', '<=', '<', '>=', '>'],
                 outcome: Hashable, /) -> int:
        """The total quantity fitting a comparison to a single outcome."""

    def quantity(self,
                 comparison: Literal['==', '!=', '<=', '<', '>=', '>']
                 | Hashable,
                 outcome: Hashable | None = None,
                 /) -> int:
        """The quantity of a single outcome.

        A comparison can be provided, in which case this returns the total
        quantity fitting the comparison.
        
        Args:
            comparison: The comparison to use. This can be omitted, in which
                case it is treated as '=='.
            outcome: The outcome to query.
        """
        if outcome is None:
            outcome = comparison
            comparison = '=='
        else:
            comparison = cast(Literal['==', '!=', '<=', '<', '>=', '>'],
                              comparison)

        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.quantity(self._rust_die(), comparison,
                                                 outcome)
        match comparison:
            case '==':
                return self.get(outcome, 0)
            case '!=':
                return self.denominator() - self.get(outcome, 0)
            case '<=' | '<':
                threshold = self.nearest(comparison, outcome)
                if threshold is None:
                    return 0
                else:
                    return self._cumulative_quantities[threshold]
            case '>=':
                return self.denominator() - self.quantity('<', outcome)
            case '>':
                return self.denominator() - self.quantity('<=', outcome)
            case _:
                raise ValueError(f'Invalid comparison {comparison}')

    def quantity_where(self,
                       which: Callable[..., bool],
                       /,
                       star: bool | None = None) -> int:
        """The quantity fulfilling a boolean condition."""
        if star is None:
            star = infer_star(which)
        # icepool-rs: `which` is called here, the sum is the core's.
        if icepool.backend.RUST and _rust_population(self):
            if star:
                selected = [bool(which(*outcome)) for outcome in self.outcomes()]
            else:
                selected = [bool(which(outcome)) for outcome in self.outcomes()]
            return icepool.backend.RUST.quantity_where(self._rust_die(),
                                                       selected)
        if star:
            return sum(quantity  # type: ignore
                       for outcome, quantity in self.items()
                       if which(*outcome))  # type: ignore
        else:
            return sum(quantity for outcome, quantity in self.items()
                       if which(outcome))

    @overload
    def quantities(self, /) -> CountsValuesView:
        """All quantities in sorted order."""

    @overload
    def quantities(self, comparison: Literal['==', '!=', '<=', '<', '>=', '>'],
                   /) -> Sequence[int]:
        """The total quantities fitting the comparison for each outcome in sorted order.
        
        For example, '<=' gives the CDF.

        Args:
            comparison: One of `'==', '!=', '<=', '<', '>=', '>'`.
                May be omitted, in which case equality `'=='` is used.
            outcome: The outcome to compare to.
            percent: If set, the result will be a percentage expressed as a
                `float`.
        """

    def quantities(self,
                   comparison: Literal['==', '!=', '<=', '<', '>=', '>']
                   | None = None,
                   /) -> CountsValuesView | Sequence[int]:
        """The quantities of the mapping in sorted order.

        For example, '<=' gives the CDF.

        Args:
            comparison: One of `'==', '!=', '<=', '<', '>=', '>'`.
                May be omitted, in which case equality `'=='` is used.
        """
        if comparison is None:
            comparison = '=='

        # icepool-rs: '==' is the values view itself.
        if icepool.backend.RUST and _rust_population(self) and comparison != '==':
            return icepool.backend.RUST.quantities(self._rust_die(),
                                                   comparison)
        match comparison:
            case '==':
                return self.values()
            case '<=':
                return tuple(itertools.accumulate(self.values()))
            case '>=':
                return tuple(
                    itertools.accumulate(self.values()[:-1],
                                         operator.sub,
                                         initial=self.denominator()))
            case '!=':
                return tuple(self.denominator() - q for q in self.values())
            case '<':
                return tuple(self.denominator() - q
                             for q in self.quantities('>='))
            case '>':
                return tuple(self.denominator() - q
                             for q in self.quantities('<='))
            case _:
                raise ValueError(f'Invalid comparison {comparison}')

    @cached_property
    def _cumulative_quantities(self) -> Mapping[T_co, int]:
        result = {}
        cdf = 0
        for outcome, quantity in self.items():
            cdf += quantity
            result[outcome] = cdf
        return result

    @cached_property
    def _denominator(self) -> int:
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.denominator(self._rust_die())
        return sum(self.values())

    def denominator(self) -> int:
        """The sum of all quantities (e.g. weights or duplicates).

        For the number of unique outcomes, use `len()`.
        """
        return self._denominator

    def multiply_quantities(self: C, scale: int, /) -> C:
        """Multiplies all quantities by an integer."""
        if icepool.backend.RUST and _rust_population(self):
            return self._from_rust_data(
                icepool.backend.RUST.quantities_op(self._rust_die(),
                                                   'multiply', scale))
        if scale == 1:
            return self
        data = {
            outcome: quantity * scale
            for outcome, quantity in self.items()
        }
        return self._new_type(data)

    def divide_quantities(self: C, divisor: int, /) -> C:
        """Divides all quantities by an integer, rounding down.
        
        Resulting zero quantities are dropped.
        """
        if icepool.backend.RUST and _rust_population(self):
            return self._from_rust_data(
                icepool.backend.RUST.quantities_op(self._rust_die(), 'divide',
                                                   divisor))
        if divisor == 0:
            return self
        data = {
            outcome: quantity // divisor
            for outcome, quantity in self.items() if quantity >= divisor
        }
        return self._new_type(data)

    def modulo_quantities(self: C, divisor: int, /) -> C:
        """Modulus of all quantities with an integer."""
        if icepool.backend.RUST and _rust_population(self):
            return self._from_rust_data(
                icepool.backend.RUST.quantities_op(self._rust_die(), 'modulo',
                                                   divisor))
        data = {
            outcome: quantity % divisor
            for outcome, quantity in self.items()
        }
        return self._new_type(data)

    def pad_to_denominator(self: C, denominator: int, /,
                           outcome: Hashable) -> C:
        """Changes the denominator to a target number by changing the quantity of a specified outcome.
        
        Args:
            `target`: The denominator of the result.
            `outcome`: The outcome whose quantity will be adjusted.

        Returns:
            A `Population` like `self` but with the quantity of `outcome`
            adjusted so that the overall denominator is equal to  `target`.
            If the denominator is reduced to zero, it will be removed.

        Raises:
            `ValueError` if this would require the quantity of the specified
            outcome to be negative.
        """
        if icepool.backend.RUST and _rust_population(self):
            return self._from_rust_data(
                icepool.backend.RUST.pad_to_denominator(
                    self._rust_die(), denominator, outcome))
        adjustment = denominator - self.denominator()
        data = {outcome: quantity for outcome, quantity in self.items()}
        new_quantity = data.get(outcome, 0) + adjustment
        if new_quantity > 0:
            data[outcome] = new_quantity
        elif new_quantity == 0:
            del data[outcome]
        else:
            raise ValueError(
                f'Padding to denominator of {denominator} would require a negative quantity of {new_quantity} for {outcome}'
            )
        return self._new_type(data)

    def multiply_to_denominator(self: C, denominator: int, /) -> C:
        """Multiplies all quantities to reach the target denominiator.
        
        Raises:
            ValueError if this cannot be achieved using an integer scaling.
        """
        if icepool.backend.RUST and _rust_population(self):
            return self._from_rust_data(
                icepool.backend.RUST.multiply_to_denominator(
                    self._rust_die(), denominator))
        if denominator % self.denominator():
            raise ValueError(
                'Target denominator is not an integer factor of the current denominator.'
            )
        return self.multiply_quantities(denominator // self.denominator())

    def _from_rust_data(self: C, data) -> C:
        """icepool-rs: `self._new_type(data)` for the `dict` items the core computed, or `self` for `None` (where
        Python returns the population itself)."""
        if data is None:
            return self
        return self._new_type(dict(data))

    def append(self: C, outcome, quantity: int = 1, /) -> C:
        """This population with an outcome appended.
        
        Args:
            outcome: The outcome to append.
            quantity: The quantity of the outcome to append. Can be negative,
                which removes quantity (but not below zero).
        """
        # icepool-rs: the Counter arithmetic runs in the Rust core.
        if icepool.backend.RUST and _rust_population(self):
            return self._from_rust_data(
                icepool.backend.RUST.population_append(self._rust_die(),
                                                       outcome, quantity))
        data = Counter(self)
        data[outcome] = max(data[outcome] + quantity, 0)
        return self._new_type(data)

    def remove(self: C, outcome, quantity: int | None = None, /) -> C:
        """This population with an outcome removed.

        Args:
            outcome: The outcome to append.
            quantity: The quantity of the outcome to remove. If not set, all
                quantity of that outcome is removed. Can be negative, which adds
                quantity instead.
        """
        # icepool-rs: the Counter arithmetic runs in the Rust core.
        if icepool.backend.RUST and _rust_population(self):
            return self._from_rust_data(
                icepool.backend.RUST.population_remove(self._rust_die(),
                                                       outcome, quantity))
        if quantity is None:
            data = Counter(self)
            data[outcome] = 0
            return self._new_type(data)
        else:
            return self.append(outcome, -quantity)

    # Probabilities.

    @overload
    def probability(self, outcome: Hashable, /, *,
                    percent: Literal[False]) -> Fraction:
        """The probability of a single outcome, or 0.0 if not present."""

    @overload
    def probability(self, outcome: Hashable, /, *,
                    percent: Literal[True]) -> float:
        """The probability of a single outcome, or 0.0 if not present."""

    @overload
    def probability(self, outcome: Hashable, /) -> Fraction:
        """The probability of a single outcome, or 0.0 if not present."""

    @overload
    def probability(self, comparison: Literal['==', '!=', '<=', '<', '>=',
                                              '>'], outcome: Hashable, /, *,
                    percent: Literal[False]) -> Fraction:
        """The total probability of outcomes fitting a comparison."""

    @overload
    def probability(self, comparison: Literal['==', '!=', '<=', '<', '>=',
                                              '>'], outcome: Hashable, /, *,
                    percent: Literal[True]) -> float:
        """The total probability of outcomes fitting a comparison."""

    @overload
    def probability(self, comparison: Literal['==', '!=', '<=', '<', '>=',
                                              '>'], outcome: Hashable,
                    /) -> Fraction:
        """The total probability of outcomes fitting a comparison."""

    def probability(self,
                    comparison: Literal['==', '!=', '<=', '<', '>=', '>']
                    | Hashable,
                    outcome: Hashable | None = None,
                    /,
                    *,
                    percent: bool = False) -> Fraction | float:
        """The total probability of outcomes fitting a comparison.
        
        Args:
            comparison: One of `'==', '!=', '<=', '<', '>=', '>'`.
                May be omitted, in which case equality `'=='` is used.
            outcome: The outcome to compare to.
            percent: If set, the result will be a percentage expressed as a
                `float`. Otherwise, the result is a `Fraction`.
        """
        if outcome is None:
            outcome = comparison
            comparison = '=='
        else:
            comparison = cast(Literal['==', '!=', '<=', '<', '>=', '>'],
                              comparison)
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.probability(self._rust_die(),
                                                    comparison, outcome,
                                                    bool(percent))
        result = Fraction(self.quantity(comparison, outcome),
                          self.denominator())
        return result * 100.0 if percent else result

    @overload
    def probability_where(self, which: Callable[...,
                                                bool], /, star: bool | None,
                          percent: Literal[False]) -> Fraction:
        ...

    @overload
    def probability_where(self, which: Callable[..., bool], /,
                          star: bool | None, percent: Literal[True]) -> float:
        ...

    @overload
    def probability_where(self, which: Callable[...,
                                                bool], /, star: bool | None,
                          percent: bool) -> Fraction | float:
        ...

    def probability_where(self,
                          which: Callable[..., bool],
                          /,
                          star: bool | None = None,
                          percent: bool = False) -> Fraction | float:
        """The probability fulfilling a boolean condition."""
        # icepool-rs: `which` is called here, the rest is the core's.
        if icepool.backend.RUST and _rust_population(self):
            if star is None:
                star = infer_star(which)
            if star:
                selected = [bool(which(*outcome)) for outcome in self.outcomes()]
            else:
                selected = [bool(which(outcome)) for outcome in self.outcomes()]
            return icepool.backend.RUST.probability_where(
                self._rust_die(), selected, bool(percent))
        numerator = self.quantity_where(which, star=star)
        if percent:
            return 100.0 * numerator / self.denominator()
        else:
            return Fraction(numerator, self.denominator())

    @overload
    def probabilities(self, /, *,
                      percent: Literal[False]) -> Sequence[Fraction]:
        """All probabilities in sorted order."""

    @overload
    def probabilities(self, /, *, percent: Literal[True]) -> Sequence[float]:
        """All probabilities in sorted order."""

    @overload
    def probabilities(self, /) -> Sequence[Fraction]:
        """All probabilities in sorted order."""

    @overload
    def probabilities(self, comparison: Literal['==', '!=', '<=', '<', '>=',
                                                '>'], /, *,
                      percent: Literal[False]) -> Sequence[Fraction]:
        """The total probabilities fitting the comparison for each outcome in sorted order.
        
        For example, '<=' gives the CDF.
        """

    @overload
    def probabilities(self, comparison: Literal['==', '!=', '<=', '<', '>=',
                                                '>'], /, *,
                      percent: Literal[True]) -> Sequence[float]:
        """The total probabilities fitting the comparison for each outcome in sorted order.
        
        For example, '<=' gives the CDF.
        """

    @overload
    def probabilities(self, comparison: Literal['==', '!=', '<=', '<', '>=',
                                                '>'], /) -> Sequence[Fraction]:
        """The total probabilities fitting the comparison for each outcome in sorted order.
        
        For example, '<=' gives the CDF.
        """

    def probabilities(
            self,
            comparison: Literal['==', '!=', '<=', '<', '>=', '>']
        | None = None,
            /,
            *,
            percent: bool = False) -> Sequence[Fraction] | Sequence[float]:
        """The total probabilities fitting the comparison for each outcome in sorted order.
        
        For example, '<=' gives the CDF.

        Args:
            comparison: One of `'==', '!=', '<=', '<', '>=', '>'`.
                May be omitted, in which case equality `'=='` is used.
            percent: If set, the result will be a percentage expressed as a
                `float`. Otherwise, the result is a `Fraction`.
        """
        if comparison is None:
            comparison = '=='

        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.probabilities(self._rust_die(),
                                                      comparison,
                                                      bool(percent))
        result = tuple(
            Fraction(q, self.denominator())
            for q in self.quantities(comparison))

        if percent:
            return tuple(100.0 * x for x in result)
        else:
            return result

    # Scalar statistics.

    def mode(self) -> tuple:
        """A tuple containing the most common outcome(s) of the population.

        These are sorted from lowest to highest.
        """
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.mode(self._rust_die())
        return tuple(outcome for outcome, quantity in self.items()
                     if quantity == self.modal_quantity())

    def modal_quantity(self) -> int:
        """The highest quantity of any single outcome. """
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.modal_quantity(self._rust_die())
        return max(self.quantities())

    def kolmogorov_smirnov(self, other: 'Population') -> Fraction:
        """Kolmogorov–Smirnov statistic. The maximum absolute difference between CDFs. """
        if icepool.backend.RUST and _rust_population(self) and _rust_population(other):
            return icepool.backend.RUST.cdf_distance(self._rust_die(),
                                                     other._rust_die(), False)
        outcomes = icepool.sorted_union(self, other)
        return max(
            abs(
                self.probability('<=', outcome) -
                other.probability('<=', outcome)) for outcome in outcomes)

    def cramer_von_mises(self, other: 'Population') -> Fraction:
        """Cramér-von Mises statistic. The sum-of-squares difference between CDFs. """
        if icepool.backend.RUST and _rust_population(self) and _rust_population(other):
            return icepool.backend.RUST.cdf_distance(self._rust_die(),
                                                     other._rust_die(), True)
        outcomes = icepool.sorted_union(self, other)
        return sum(((self.probability('<=', outcome) -
                     other.probability('<=', outcome))**2
                    for outcome in outcomes),
                   start=Fraction(0, 1))

    def median(self):
        """The median, taking the mean in case of a tie.

        This will fail if the outcomes do not support division;
        in this case, use `median_low` or `median_high` instead.
        """
        return self.quantile(1, 2)

    def median_low(self) -> T_co:
        """The median, taking the lower in case of a tie."""
        return self.quantile_low(1, 2)

    def median_high(self) -> T_co:
        """The median, taking the higher in case of a tie."""
        return self.quantile_high(1, 2)

    def quantile(self, n: int, d: int = 100):
        """The outcome `n / d` of the way through the CDF, taking the mean in case of a tie.

        This will fail if the outcomes do not support addition and division;
        in this case, use `quantile_low` or `quantile_high` instead.
        """
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.quantile(self._rust_die(), n, d,
                                                 'mean')
        # Should support addition and division.
        return (self.quantile_low(n, d) +
                self.quantile_high(n, d)) / 2  # type: ignore

    def quantile_low(self, n: int, d: int = 100) -> T_co:
        """The outcome `n / d` of the way through the CDF, taking the lesser in case of a tie."""
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.quantile(self._rust_die(), n, d, 'low')
        index = bisect.bisect_left(self.quantities('<='),
                                   (n * self.denominator() + d - 1) // d)
        if index >= len(self):
            return self.max_outcome()
        return self.outcomes()[index]

    def quantile_high(self, n: int, d: int = 100) -> T_co:
        """The outcome `n / d` of the way through the CDF, taking the greater in case of a tie."""
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.quantile(self._rust_die(), n, d,
                                                 'high')
        index = bisect.bisect_right(self.quantities('<='),
                                    n * self.denominator() // d)
        if index >= len(self):
            return self.max_outcome()
        return self.outcomes()[index]

    @overload
    def mean(self: 'Population[numbers.Rational]') -> Fraction:
        ...

    @overload
    def mean(self: 'Population[float]') -> float:
        ...

    def mean(
        self: 'Population[numbers.Rational] | Population[float]'
    ) -> Fraction | float:
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.statistic(self._rust_die(), 'mean')
        return try_fraction(
            sum(outcome * quantity for outcome, quantity in self.items()),
            self.denominator())

    @overload
    def variance(self: 'Population[numbers.Rational]') -> Fraction:
        ...

    @overload
    def variance(self: 'Population[float]') -> float:
        ...

    def variance(
        self: 'Population[numbers.Rational] | Population[float]'
    ) -> Fraction | float:
        """This is the population variance, not the sample variance."""
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.statistic(self._rust_die(),
                                                  'variance')
        mean = self.mean()
        mean_of_squares = try_fraction(
            sum(quantity * outcome**2 for outcome, quantity in self.items()),
            self.denominator())
        return mean_of_squares - mean * mean

    def standard_deviation(
            self: 'Population[numbers.Rational] | Population[float]') -> float:
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.statistic(self._rust_die(),
                                                  'standard_deviation')
        return math.sqrt(self.variance())

    sd = standard_deviation

    def standardized_moment(
            self: 'Population[numbers.Rational] | Population[float]',
            k: int) -> float:
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.standardized_moment(
                self._rust_die(), k)
        sd = self.standard_deviation()
        mean = self.mean()
        ev = sum(p * (outcome - mean)**k  # type: ignore 
                 for outcome, p in zip(self.outcomes(), self.probabilities()))
        return ev / (sd**k)

    def skewness(
            self: 'Population[numbers.Rational] | Population[float]') -> float:
        return self.standardized_moment(3)

    def excess_kurtosis(
            self: 'Population[numbers.Rational] | Population[float]') -> float:
        return self.standardized_moment(4) - 3.0

    def entropy(self, base: float = 2.0) -> float:
        """The entropy of a random sample from this population.
        
        Args:
            base: The logarithm base to use. Default is 2.0, which gives the 
                entropy in bits.
        """
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.entropy(self._rust_die(), base)
        return -sum(p * math.log(p, base)
                    for p in self.probabilities() if p > 0.0)

    # Joint statistics.

    class _Marginals(Generic[C]):
        """Helper class for implementing `marginals()`."""

        _population: C

        def __init__(self, population, /):
            self._population = population

        def __len__(self) -> int:
            """The minimum len() of all outcomes."""
            # icepool-rs: `__getitem__` runs in the core too, through the
            # die's `_unary_operator` (operator.getitem is native there).
            if icepool.backend.RUST and isinstance(self._population,
                                                   icepool.Die):
                return icepool.backend.RUST.marginals_len(
                    self._population._rust_die())
            return min(len(x) for x in self._population.outcomes())

        def __getitem__(self, dims: int | slice, /):
            """Marginalizes the given dimensions."""
            return self._population._unary_operator(operator.getitem, dims)

        def __iter__(self) -> Iterator:
            for i in range(len(self)):
                yield self[i]

        def __getattr__(self, key: str):
            if key[0] == '_':
                raise AttributeError(key)
            return self._population._unary_operator(operator.attrgetter(key))

    @property
    def marginals(self: C) -> _Marginals[C]:
        """A property that applies the `[]` operator to outcomes.

        For example, `population.marginals[:2]` will marginalize the first two
        elements of sequence outcomes.

        Attributes that do not start with an underscore will also be forwarded.
        For example, `population.marginals.x` will marginalize the `x` attribute
        from e.g. `namedtuple` outcomes.
        """
        return Population._Marginals(self)

    @overload
    def covariance(self: 'Population[tuple[numbers.Rational, ...]]', i: int,
                   j: int) -> Fraction:
        ...

    @overload
    def covariance(self: 'Population[tuple[float, ...]]', i: int,
                   j: int) -> float:
        ...

    def covariance(
            self:
        'Population[tuple[numbers.Rational, ...]] | Population[tuple[float, ...]]',
            i: int, j: int) -> Fraction | float:
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.joint(self._rust_die(), i, j, False)
        mean_i = self.marginals[i].mean()
        mean_j = self.marginals[j].mean()
        return try_fraction(
            sum((outcome[i] - mean_i) * (outcome[j] - mean_j) * quantity
                for outcome, quantity in self.items()), self.denominator())

    def correlation(
            self:
        'Population[tuple[numbers.Rational, ...]] | Population[tuple[float, ...]]',
            i: int, j: int) -> float:
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.joint(self._rust_die(), i, j, True)
        sd_i = self.marginals[i].standard_deviation()
        sd_j = self.marginals[j].standard_deviation()
        return self.covariance(i, j) / (sd_i * sd_j)

    # Transformations.

    def _select_outcomes(self, which: Callable[..., bool] | Collection[T_co],
                         star: bool | None) -> Set[T_co]:
        """Returns a set of outcomes of self that fit the given condition."""
        if callable(which):
            if star is None:
                star = infer_star(which)
            if star:
                # Need TypeVarTuple to check this.
                return {
                    outcome
                    for outcome in self.outcomes()
                    if which(*outcome)  # type: ignore
                }
            else:
                return {
                    outcome
                    for outcome in self.outcomes() if which(outcome)
                }
        else:
            # Collection.
            return set(outcome for outcome in self.outcomes()
                       if outcome in which)

    def to_one_hot(self: C, outcomes: Sequence[T_co] | None = None) -> C:
        """Converts the outcomes of this population to a one-hot representation.

        Args:
            outcomes: If provided, each outcome will be mapped to a `Vector`
                where the element at `outcomes.index(outcome)` is set to `True`
                and the rest to `False`, or all `False` if the outcome is not
                in `outcomes`.
                If not provided, `self.outcomes()` is used.
        """
        # icepool-rs: the vectors are made and counted in the Rust core;
        # `outcomes`' `len`, `in` and `index` are Python's, called from there.
        if icepool.backend.RUST and _rust_population(self):
            return self._new_type._new_rust(
                icepool.backend.RUST.to_one_hot(
                    self._rust_die(), self.outcomes(), outcomes,
                    isinstance(self, icepool.Deck)))

        if outcomes is None:
            outcomes = self.outcomes()

        data: MutableMapping[Vector[bool], int] = defaultdict(int)
        for outcome, quantity in zip(self.outcomes(), self.quantities()):
            value = [False] * len(outcomes)
            if outcome in outcomes:
                value[outcomes.index(outcome)] = True
            data[Vector(value)] += quantity
        return self._new_type(data)

    def split(self,
              outcomes: Callable[..., bool] | Collection[T_co],
              /,
              *,
              star: bool | None = None) -> tuple[C, C]:
        """Splits this population into one containing selected items and another containing the rest.

        The sum of the denominators of the results is equal to the denominator
        of this population.

        If you want to split more than two ways, use `Population.group_by()`.

        Args:
            outcomes: Selects which outcomes to select. Options:
                * A callable that takes an outcome and returns `True` if it
                    should be selected.
                * A collection of outcomes to select.
            star: Whether outcomes should be unpacked into separate arguments
                before sending them to a callable `which`.
                If not provided, this will be guessed based on the function
                signature.

        Returns:
            A population consisting of the outcomes that were selected by
            `which`, and a population consisting of the unselected outcomes.
        """
        outcome_set = self._select_outcomes(outcomes, star)

        # icepool-rs: the split runs in the Rust core, given one flag per
        # outcome, for a die or a deck.
        if icepool.backend.RUST and isinstance(self, icepool.Die):
            mask = [outcome in outcome_set for outcome in self.outcomes()]
            a, b = icepool.backend.RUST.split(self._rust_die(), mask)
            return icepool.Die._new_rust(a), icepool.Die._new_rust(b)
        if icepool.backend.RUST and isinstance(self, icepool.Deck):
            mask = [outcome in outcome_set for outcome in self.outcomes()]
            a, b = icepool.backend.RUST.deck_split(self._rust_deck(), mask)
            return icepool.Deck._new_rust(a), icepool.Deck._new_rust(b)

        selected = {}
        not_selected = {}
        for outcome, count in self.items():
            if outcome in outcome_set:
                selected[outcome] = count
            else:
                not_selected[outcome] = count

        return self._new_type(selected), self._new_type(not_selected)

    class _GroupBy(Generic[C]):
        """Helper class for implementing `group_by()`."""

        _population: C

        def __init__(self, population, /):
            self._population = population

        def __call__(self,
                     key_map: Callable[..., U] | Mapping[T_co, U],
                     /,
                     *,
                     star: bool | None = None) -> Mapping[U, C]:
            if callable(key_map):
                if star is None:
                    star = infer_star(key_map)
                if star:
                    key_function = lambda o: key_map(*o)
                else:
                    key_function = key_map
            else:
                key_function = lambda o: key_map.get(o, o)

            # icepool-rs: the keys are Python's (called back per outcome),
            # the grouping and the dice are the core's.
            if icepool.backend.RUST and isinstance(self._population,
                                                   icepool.Die):
                return {
                    k: icepool.Die._new_rust(v)
                    for k, v in icepool.backend.RUST.group_by(
                        self._population._rust_die(), key_function)
                }

            result_datas: MutableMapping[U, MutableMapping[Any, int]] = {}
            outcome: Any
            for outcome, quantity in self._population.items():
                key = key_function(outcome)
                if key not in result_datas:
                    result_datas[key] = defaultdict(int)
                result_datas[key][outcome] += quantity
            return {
                k: self._population._new_type(v)
                for k, v in result_datas.items()
            }

        def __getitem__(self, dims: int | slice, /):
            """Marginalizes the given dimensions."""
            return self(lambda x: x[dims])

        def __getattr__(self, key: str):
            if key[0] == '_':
                raise AttributeError(key)
            return self(lambda x: getattr(x, key))

    @property
    def group_by(self: C) -> _GroupBy[C]:
        """A method-like property that splits this population into sub-populations based on a key function.
        
        The sum of the denominators of the results is equal to the denominator
        of this population.

        This can be useful when using the law of total probability.

        Example: `d10.group_by(lambda x: x % 3)` is
        ```python
        {
            0: Die([3, 6, 9]),
            1: Die([1, 4, 7, 10]),
            2: Die([2, 5, 8]),
        }
        ```

        You can also use brackets to group by indexes or slices; or attributes
        to group by those. Example:

        ```python
        Die([
            'aardvark',
            'alligator',
            'asp',
            'blowfish',
            'cat',
            'crocodile',
        ]).group_by[0]
        ```

        produces

        ```python
        {
            'a': Die(['aardvark', 'alligator', 'asp']),
            'b': Die(['blowfish']),
            'c': Die(['cat', 'crocodile']),
        }
        ```

        Args:
            key_map: A function or mapping that takes outcomes and produces the
                key of the corresponding outcome in the result. If this is
                a Mapping, outcomes not in the mapping are their own key.
            star: Whether outcomes should be unpacked into separate arguments
                before sending them to a callable `key_map`.
                If not provided, this will be guessed based on the function
                signature.
        """
        return Population._GroupBy(self)

    def sample(self) -> T_co:
        """A single random sample from this population.

        Note that this is always "with replacement" even for `Deck` since
        instances are immutable.

        This uses the standard `random` package and is not cryptographically
        secure.
        """
        # We don't use random.choices since that is based on floats rather than ints.
        r = random.randrange(self.denominator())
        # icepool-rs: the draw stays with Python's `random` (the user seeds
        # it); the bisect over the CDF runs in the Rust core.
        if icepool.backend.RUST and _rust_population(self):
            return icepool.backend.RUST.sample_at(self._rust_die(), r)
        index = bisect.bisect_right(self.quantities('<='), r)
        return self.outcomes()[index]

    def format(self, format_spec: str, /, **kwargs) -> str:
        """Formats this mapping as a string.

        `format_spec` should start with the output format,
        which can be:
        * `md` for Markdown (default)
        * `bbcode` for BBCode
        * `csv` for comma-separated values
        * `html` for HTML

        After this, you may optionally add a `:` followed by a series of
        requested columns. Allowed columns are:

        * `o`: Outcomes.
        * `*o`: Outcomes, unpacked if applicable.
        * `q==`, `q<=`, `q>=`: Quantities ==, <=, or >= each outcome.
        * `p==`, `p<=`, `p>=`: Probabilities (0-1).
        * `%==`, `%<=`, `%>=`: Probabilities (0%-100%).
        * `i==`, `i<=`, `i>=`: EXPERIMENTAL: "1 in N".

        Columns may optionally be separated using `|` characters.

        The default setting is equal to `f'{die:md:*o|q==|%==}'`. Here the 
        columns are the outcomes (unpacked if applicable) the quantities, and 
        the probabilities. The quantities are omitted from the default columns 
        if any individual quantity is 10**30 or greater.
        """
        # icepool-rs: a die's spec parsing and table run in the Rust core
        # (format.rs). Given csv writer arguments, the core makes the table and
        # Python's csv module writes it with them.
        if icepool.backend.RUST and _rust_population(self):
            result = icepool.backend.RUST.format(self._rust_die(),
                                                 type(self).__name__,
                                                 format_spec, bool(kwargs))
            if isinstance(result, str):
                return result
            headers, rows = result
            return icepool.population.format.write_csv(headers, rows, **kwargs)
        if not self.is_empty() and self.modal_quantity() < 10**30:
            default_column_spec = '*oq==%=='
        else:
            default_column_spec = '*o%=='
        if len(format_spec) == 0:
            format_spec = 'md:' + default_column_spec

        format_spec = format_spec.replace('|', '')

        parts = format_spec.split(':')

        if len(parts) == 1:
            output_format = parts[0]
            col_spec = default_column_spec
        elif len(parts) == 2:
            output_format = parts[0]
            col_spec = parts[1]
        else:
            raise ValueError('format_spec has too many colons.')

        match output_format:
            case 'md':
                return icepool.population.format.markdown(self, col_spec)
            case 'bbcode':
                return icepool.population.format.bbcode(self, col_spec)
            case 'csv':
                return icepool.population.format.csv(self, col_spec, **kwargs)
            case 'html':
                return icepool.population.format.html(self, col_spec)
            case _:
                raise ValueError(
                    f"Unsupported output format '{output_format}'")

    def __format__(self, format_spec: str, /) -> str:
        return self.format(format_spec)

    def __str__(self) -> str:
        return f'{self}'
