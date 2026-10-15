"""What the facade still ran in Python before S5c, now on the core: `append` / `remove` / `to_one_hot` of a die or
deck (population/base.py), the deck forms of `zero` / `split`, `Die.mean_time_to_sum`, and the Cartesian products
of expand.py over decks and multiset expressions (`cartesian_product`, `iter_cartesian_product`).

Same rules as basic.py: the Python side is a direct call into icepool; generators keep programs small. The results
are dice or decks (or tuples / lists of them), so most nodes are fuzz roots ("value").
"""
import icepool

from harness.core.grammar_api import node
from harness.grammar.construction import _typed_outcome
from harness.grammar.deck import _small_outcome
from harness.grammar.maps import _star, _which
from harness.grammar.pool import _die_without_nan


def _population(g, depth):
    """A die (60%) or a deck."""
    if g.rng.random() < 0.6:
        return _die_without_nan(g, min(depth, 1))
    return g.make("deck", depth)


def _outcome(g):
    """An outcome to append / remove: mostly a small int (often one the population has), sometimes another type, or
    rarely a list (unhashable)."""
    r = g.rng.random()
    if r < 0.8:
        return g.rng.randint(-1, 7)
    if r < 0.97:
        return _typed_outcome(g, "any")
    return [1]


def _quantity(g):
    r = g.rng.random()
    if r < 0.85:
        return g.rng.randint(-3, 3)
    return g.rng.choice([{"bool": True}, {"float": "1.5"}, {"float": "0.0"}, {"frac": [1, 2]},
                         {"frac": [4, 2]}, {"str": "a"}, None])


@node("pop_append", "value", weight=0.5,
      gen=lambda g, depth: {"op": "pop_append", "population": _population(g, depth), "outcome": _outcome(g),
                            "quantity": _quantity(g)})
def pop_append(population, outcome, quantity):
    """population.append(outcome, quantity): Counter(population) with data[outcome] = max(data[outcome] +
    quantity, 0), then Die(data) / Deck(data) (zero quantities drop out)."""
    return population.append(outcome, quantity)


@node("pop_remove", "value", weight=0.5,
      gen=lambda g, depth: {"op": "pop_remove", "population": _population(g, depth), "outcome": _outcome(g),
                            "quantity": None if g.rng.random() < 0.4 else _quantity(g)})
def pop_remove(population, outcome, quantity):
    """population.remove(outcome, quantity): all of that outcome removed (quantity None), else
    append(outcome, -quantity)."""
    return population.remove(outcome, quantity)


def _one_hot_outcomes(g, depth):
    """`outcomes` for to_one_hot: None (the population's own), a list or tuple of outcomes (repeats, missing ones),
    a str (substring `in` / `index`), a die (no `index`: AttributeError once an outcome is in it), or an int (no
    `len`)."""
    r = g.rng.random()
    items = [g.rng.randint(0, 6) if g.rng.random() < 0.9 else _typed_outcome(g, "any")
             for _ in range(g.rng.randint(0, 5))]
    if r < 0.35:
        return None
    if r < 0.7:
        return items
    if r < 0.85:
        return {"tuple": items}
    if r < 0.92:
        return {"str": g.rng.choice(["", "a", "ab", "abc", "ba"])}
    if r < 0.97:
        return _die_without_nan(g, 0)
    return g.rng.randint(0, 3)


@node("to_one_hot", "value", weight=0.5,
      gen=lambda g, depth: {"op": "to_one_hot", "population": _population(g, depth),
                            "outcomes": _one_hot_outcomes(g, depth)})
def to_one_hot(population, outcomes):
    """population.to_one_hot(outcomes): each outcome becomes a Vector of len(outcomes) bools, True at
    outcomes.index(outcome) if it is in outcomes; equal vectors add up."""
    return population.to_one_hot(outcomes)


@node("deck_zero", "deck", weight=0.3, gen=lambda g, depth: {"op": "deck_zero", "deck": g.make("deck", depth)})
def deck_zero(deck):
    """deck.zero(): every card times 0, which must resolve to a single outcome (same size)."""
    return deck.zero()


@node("deck_split", "value", weight=0.4,
      gen=lambda g, depth: {"op": "deck_split", "deck": g.make("deck", depth),
                            "which": _which(g, depth, none_ok=False), "star": _star(g)})
def deck_split(deck, which, star):
    """deck.split(which, star=star): (selected cards, the rest), a tuple of two decks."""
    return deck.split(which, star=star)


def _mts_die(g, depth):
    """A die for mean_time_to_sum: mostly small non-negative ones (some with 0, which takes time without
    progress), sometimes negative, all-zero, empty or typed ones (which raise)."""
    r = g.rng.random()
    if r < 0.3:
        return {"op": "d", "sides": g.rng.randint(1, 8)}
    if r < 0.75:
        return {"op": "die", "outcomes": [g.rng.randint(0, 5) for _ in range(g.rng.randint(1, 4))]}
    if r < 0.8:
        return {"op": "die", "outcomes": [g.rng.choice([0, -1, 2])]}
    if r < 0.85:
        return {"op": "die", "outcomes": []}
    if r < 0.9:
        return {"op": "die_typed", "outcomes": [_typed_outcome(g, "num") for _ in range(g.rng.randint(1, 3))]}
    return _die_without_nan(g, min(depth, 1))


def _target(g):
    r = g.rng.random()
    if r < 0.9:
        return g.rng.randint(-2, 20)
    return g.rng.choice([{"bool": True}, {"float": "2.5"}, {"float": "-1.0"}, {"frac": [3, 1]},
                         {"str": "a"}])


@node("mean_time_to_sum", "value", weight=0.4,
      gen=lambda g, depth: {"op": "mean_time_to_sum", "die": _mts_die(g, depth),
                            "targets": [_target(g) for _ in range(g.rng.randint(1, 3))]})
def mean_time_to_sum(die, targets):
    """tuple(die.mean_time_to_sum(t) for t in targets): Fractions, sharing the die's cache of earlier targets."""
    return tuple(die.mean_time_to_sum(t) for t in targets)


def _small_pool(g):
    sides = [g.rng.randint(1, 3) for _ in range(g.rng.randint(1, 2))]
    return {"op": "pool", "dice": [{"op": "d", "sides": s} for s in sides], "times": 1}


def _cartesian_arg(g, depth, decks):
    """One argument of a Cartesian product: a small die or deck, a plain outcome, a small pool (expanded into its
    sorted tuples), rarely Reroll."""
    r = g.rng.random()
    if r < 0.4:
        return g.make("deck", 0) if decks else _die_without_nan(g, 0)
    if r < 0.65:
        return _small_outcome(g)
    if r < 0.8:
        return _small_pool(g)
    if r < 0.88:
        # The other kind of population: a mixture raises TypeError.
        return _die_without_nan(g, 0) if decks else g.make("deck", 0)
    if r < 0.92:
        return {"op": "Reroll"}
    return _typed_outcome(g, "any")


def _gen_cartesian(g, depth):
    decks = g.rng.random() < 0.6
    args = [_cartesian_arg(g, depth, decks) for _ in range(g.rng.randint(1, 3))]
    if g.rng.random() < 0.8:
        # At least one population, so the result is one (a tuple of pools cannot be encoded).
        args[0] = g.make("deck", 0) if decks else _die_without_nan(g, 0)
    elif any(isinstance(a, dict) and a.get("op") == "pool" for a in args):
        args = [a for a in args if not (isinstance(a, dict) and a.get("op") == "pool")] or [1]
    return {"op": "cartesian_product", "args": args, "vector": {"bool": g.rng.random() < 0.4}}


@node("cartesian_product", "value", weight=0.5, gen=_gen_cartesian)
def cartesian_product(args, vector):
    """icepool.cartesian_product(*args, outcome_type=Vector if vector else tuple): Reroll among the args is the
    result; dice and decks must not mix; with neither, the outcome_type of the args; otherwise a die or deck of the
    joint outcomes (a multiset expression contributes its expand() tuples)."""
    return icepool.cartesian_product(*args, outcome_type=icepool.Vector if vector else tuple)


def _gen_iter_cartesian(g, depth):
    args = [_cartesian_arg(g, depth, g.rng.random() < 0.5) for _ in range(g.rng.randint(0, 3))]
    return {"op": "iter_cartesian_product",
            "args": [a for a in args if not (isinstance(a, dict) and a.get("op") == "Reroll")]}


@node("iter_cartesian_product", "value", weight=0.4, gen=_gen_iter_cartesian)
def iter_cartesian_product(args):
    """tuple(icepool.iter_cartesian_product(*args)): (outcomes, quantity) per joint outcome, in itertools.product
    order (a die's or deck's items, a multiset expression's expand() items, anything else as itself)."""
    return tuple(icepool.iter_cartesian_product(*args))
