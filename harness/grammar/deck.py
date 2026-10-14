"""Decks (population/deck.py) and deals (generator/deal.py). Same rules as basic.py: the Python side is a direct
call into icepool; generators keep programs small.

A deck's repr has no address (`Deck({1: 2, 3: 1})`), so the deck-building nodes are fuzz roots like dice (the
result encodes as {"repr": ...}). So is a deal's, but a deal is generated inside the evaluation nodes (through
pool.py `gen_pool(..., deals=True)`, with the same keep-like operations as a pool) and read by `deal_info` /
`deal_pop`.

Decks stay small: `sequence` enumerates size**deals tuples and evaluating a deal pops every outcome.
"""
import icepool

from harness.catalogue.functions import ENTRIES
from harness.core.encode import enc
from harness.core.grammar_api import node
from harness.grammar import peek
from harness.grammar.pool import _die_without_nan, fresh_caches
from harness.grammar.statistics import STATS


def _small_outcome(g):
    return g.rng.randint(0, 6)


def _card(g, depth):
    """One element of Deck(outcomes): mostly a small int; sometimes a die (flattened by its quantities), a deck,
    Reroll, a str (which does not sort with ints), or a tuple."""
    r = g.rng.random()
    if r < 0.75:
        return _small_outcome(g)
    if r < 0.83:
        return _die_without_nan(g, min(depth, 1))
    if r < 0.88 and depth > 0:
        return g.make("deck", depth - 1)
    if r < 0.91:
        return {"op": "Reroll"}
    if r < 0.97:
        return {"tuple": [_small_outcome(g), _small_outcome(g)]}
    return {"str": g.rng.choice(["a", "b"])}


def _times(g, n):
    """`times` for n elements (None: a mapping or str, whose length the generator does not know): an int for all,
    or one per element; rarely 0, negative, a float, or the wrong length."""
    if n is None or g.rng.random() < 0.6:
        return g.rng.choice([1, 1, 1, 1, 2, 2, 3, 0]) if g.rng.random() < 0.97 else -1
    m = n if g.rng.random() < 0.95 else g.rng.randint(0, 3)
    special = [{"float": "0.0"}, {"float": "1.5"}, {"float": "2.0"}, {"bool": True}, -1]
    return [g.rng.choice([0, 1, 1, 1, 2, 3]) if g.rng.random() < 0.95 else g.rng.choice(special)
            for _ in range(m)]


def _gen_deck(g, depth):
    r = g.rng.random()
    if r < 0.75:
        outcomes = [_card(g, depth) for _ in range(g.rng.randint(0, 5))]
    elif r < 0.85:
        # A die is a mapping: its quantities become the duplicates.
        outcomes = _die_without_nan(g, min(depth, 1))
    elif r < 0.92 and depth > 0:
        # Deck(deck, times).
        outcomes = g.make("deck", depth - 1)
    else:
        outcomes = {"str": "".join(g.rng.choice("abc") for _ in range(g.rng.randint(0, 4)))}
    n = len(outcomes) if isinstance(outcomes, list) else None
    return {"op": "deck", "outcomes": outcomes, "times": _times(g, n)}


@node("deck", "deck", gen=_gen_deck, leaf=True)
def deck(outcomes, times):
    """icepool.Deck(outcomes, times): a sequence (each element once, dice and decks flattened, Reroll dropped) or
    a mapping (a die or deck), times an int for all or one per element."""
    return icepool.Deck(outcomes, times)


_OPS = {"+": lambda a, b: a + b, "-": lambda a, b: a - b, "&": lambda a, b: a & b, "|": lambda a, b: a | b,
        "^": lambda a, b: a ^ b}


def _other(g, depth):
    """The other operand of a deck operator: a deck, or anything Counter() takes (a list or str of outcomes, a
    die, None), or rarely an int (not iterable)."""
    r = g.rng.random()
    if r < 0.5:
        return g.make("deck", depth)
    if r < 0.75:
        return [_small_outcome(g) for _ in range(g.rng.randint(0, 4))]
    if r < 0.85:
        return _die_without_nan(g, min(depth, 1))
    if r < 0.9:
        return None
    if r < 0.95:
        return {"str": "".join(g.rng.choice("ab") for _ in range(g.rng.randint(0, 3)))}
    return g.rng.randint(0, 3)

def _gen_deck_op(g, depth):
    operator = {"str": g.rng.choice(sorted(_OPS))}
    d = g.make("deck", depth)
    other = _other(g, depth)
    if g.rng.random() < 0.25 and not (isinstance(other, dict) and "op" in other):
        # A literal on the left, the deck on the right: the deck's reflected operator.
        return {"op": "deck_op", "operator": operator, "a": other, "b": d}
    return {"op": "deck_op", "operator": operator, "a": d, "b": other}


@node("deck_op", "deck", gen=_gen_deck_op)
def deck_op(operator, a, b):
    """a <operator> b for operator in + - & | ^ with a deck on at least one side; the other side is anything
    Counter() takes (a deck or die is a mapping, a sequence is counted, None is empty)."""
    return _OPS[operator](a, b)


_MUL = {"*": lambda d, n: d * n, "r*": lambda d, n: n * d, "//": lambda d, n: d // n, "%": lambda d, n: d % n}


def _gen_deck_mul(g, depth):
    n = g.rng.choice([0, 1, 2, 2, 3, -1]) if g.rng.random() < 0.92 else g.rng.choice(
        [{"float": "2.0"}, {"str": "a"}, {"bool": True}, None])
    return {"op": "deck_mul", "deck": g.make("deck", depth), "operator": {"str": g.rng.choice(sorted(_MUL))},
            "n": n}


@node("deck_mul", "deck", gen=_gen_deck_mul)
def deck_mul(deck, operator, n):
    """deck * n, n * deck (operator 'r*'), deck // n, deck % n: the quantities multiplied, floor-divided (those
    below n dropped; n == 0 returns the deck) or reduced mod n, for an int n (else TypeError)."""
    return _MUL[operator](deck, n)


# Functions that make sense on deck outcomes (small ints): their results may be dice (flattened into cards), tuples
# holding dice or Reroll, Reroll / Restart, or another type.
_MAP_FUNCTIONS = ["add_k", "mul_k", "clamp", "identity", "is_even", "gt_k", "pair_k", "die_if_k", "plus_d",
                  "reroll_below", "reroll_in_tuple", "tuple_d", "const", "mapping", "restart_below"]


def _gen_deck_map(g, depth):
    name = g.rng.choice(_MAP_FUNCTIONS)
    f = {"op": "fn", "name": name, "params": ENTRIES[name].params(g)}
    star = None if g.rng.random() < 0.8 else {"bool": g.rng.random() < 0.5}
    return {"op": "deck_map", "deck": g.make("deck", depth), "repl": f, "star": star}


@node("deck_map", "deck", gen=_gen_deck_map)
def deck_map(deck, repl, star):
    """deck.map(repl, star=star): each outcome mapped (a mapping leaves the unmapped ones), then
    Deck(results, times=deck.quantities())."""
    return deck.map(repl, star=star)


def _gen_deck_pad(g, depth):
    return {"op": "deck_pad_to_denominator", "deck": g.make("deck", depth),
            "denominator": g.rng.randint(0, 10), "outcome": _small_outcome(g)}


@node("deck_pad_to_denominator", "deck", gen=_gen_deck_pad, weight=0.4)
def deck_pad_to_denominator(deck, denominator, outcome):
    """deck.pad_to_denominator(denominator, outcome): the quantity of outcome changed to make the size
    denominator (ValueError if it would go negative)."""
    return deck.pad_to_denominator(denominator, outcome)


# --- Reading decks ----------------------------------------------------------------------------------------------

@node("deck_info", "value", gen=lambda g, depth: {"op": "deck_info", "deck": g.make("deck", depth)}, weight=0.5)
def deck_info(deck):
    """(tuple(deck.items()), deck.size(), len(deck))."""
    return tuple(deck.items()), deck.size(), len(deck)


@node("deck_pop", "value", gen=lambda g, depth: {"op": "deck_pop", "deck": g.make("deck", depth),
                                                 "max": {"bool": g.rng.random() < 0.5}}, weight=0.4)
def deck_pop(deck, max):
    """deck._pop_max() if max else deck._pop_min(), as (tuple(popped.items()), quantity of the popped outcome)."""
    popped, quantity = deck._pop_max() if max else deck._pop_min()
    return tuple(popped.items()), quantity


@node("deck_stat", "value", gen=lambda g, depth: {"op": "deck_stat", "deck": g.make("deck", depth),
                                                  "stat": {"str": g.rng.choice(STATS)}}, weight=0.5)
def deck_stat(deck, stat):
    """getattr(deck, stat)(): a Population statistic of the deck (as of a die with these quantities)."""
    result = getattr(deck, stat)()
    return tuple(result) if stat == "mode" else result


def _small_deck(g):
    """A literal deck of at most 4 cards, for sequence (size**deals sequences)."""
    return {"op": "deck", "outcomes": [g.rng.randint(0, 3) for _ in range(g.rng.randint(0, 4))], "times": 1}


@node("deck_sequence", "die", gen=lambda g, depth: {"op": "deck_sequence", "deck": _small_deck(g),
                                                    "deals": g.rng.choice([0, 1, 1, 2, 2, 3, -1])}, weight=0.4)
def deck_sequence(deck, deals):
    """deck.sequence(deals): every sequence of deals cards dealt one by one, as a die of tuples (deals == 0 raises
    UnboundLocalError: the loop never runs)."""
    return deck.sequence(deals)


# --- Deals ------------------------------------------------------------------------------------------------------

@node("deal", "deal")
def deal(deck, hand_size):
    """deck.deal(hand_size) for an int hand_size: a Deal."""
    return deck.deal(hand_size)


def gen_deal(g, depth):
    """A deal from a modest deck; the hand size mostly fits the deck (asks the oracle for its size)."""
    d = g.make("deck", min(depth, 1))
    hand_size = g.rng.randint(0, 3)
    try:
        size = peek(d).size()
        if g.rng.random() < 0.9:
            hand_size = g.rng.randint(0, min(size, 5))
    except Exception:  # noqa: BLE001 - a deck that raises is a fine program too
        pass
    if g.rng.random() < 0.03:
        hand_size = -1
    return {"op": "deal", "deck": d, "hand_size": hand_size}


@node("deal_info", "value", gen=lambda g, depth: {"op": "deal_info", "deal": gen_deal(g, depth)}, weight=0.4)
def deal_info(deal):
    """(tuple(deck items), keep_tuple(), hand_size(), keep_size(), denominator(), tuple(outcomes()))."""
    return (tuple(deal.deck().items()), deal.keep_tuple(), deal.hand_size(), deal.keep_size(), deal.denominator(),
            tuple(deal.outcomes()))


def _gen_deal_pop(g, depth):
    from harness.grammar.pool import _keep_op
    p = gen_deal(g, depth)
    for _ in range(g.rng.choice([0, 0, 1, 2])):
        p = _keep_op(g, p, final=False)
    order = g.rng.choice([1, 1, -1, -1, 0])
    outcome = g.outcome()
    # Mostly pop the outcome the source pops (otherwise it yields itself unchanged).
    try:
        outcomes = peek(p).outcomes()
        if outcomes and g.rng.random() < 0.85:
            outcome = enc(outcomes[0] if order > 0 else outcomes[-1])
    except Exception:  # noqa: BLE001
        pass
    return {"op": "deal_pop", "deal": p, "order": order, "outcome": outcome}


@node("deal_pop", "value", gen=_gen_deal_pop, weight=0.5)
def deal_pop(deal, order, outcome):
    """deal._make_source().pop(Order(order), outcome) as a tuple of (deck items, keep_tuple, count, weight): each
    number of the popped outcome's cards the hand can get, last the possibilities that keep nothing more."""
    fresh_caches()
    return tuple((tuple(s.deck.items()), s.keep_tuple, count, weight)
                 for s, count, weight in deal._make_source().pop(icepool.Order(order), outcome))


# --- MultiDeal (S4b) --------------------------------------------------------------------------------------------

@node("multi_deal", "multi_deal")
def multi_deal(deck, hand_sizes):
    """deck.deal(hand_sizes) for a sequence of int hand sizes: a MultiDeal with one group (hand_size, 1) per
    hand (ValueError if more cards are dealt than the deck has)."""
    return deck.deal(hand_sizes)


@node("deal_groups", "multi_deal")
def deal_groups(deck, hand_groups):
    """deck.deal_groups(*hand_groups), each group a (hand_size, group_size) pair: the hands of a group are
    indistinguishable until they get different cards."""
    return deck.deal_groups(*(tuple(group) for group in hand_groups))


# Cards dealt in total stay small: popping an outcome deals its cards to the hands in every way.
_MAX_DEALT = 6


def gen_multi_deal(g, depth):
    """A multi-deal from a modest deck: mostly hands that fit the deck (asks the oracle for its size), now and
    then more cards than it has; a third of the time hand groups."""
    d = g.make("deck", min(depth, 1))
    room = g.rng.randint(0, 4)
    try:
        size = peek(d).size()
        if g.rng.random() < 0.92:
            room = min(size, _MAX_DEALT)
    except Exception:  # noqa: BLE001 - a deck that raises is a fine program too
        pass
    if g.rng.random() < 0.35:
        groups = []
        for _ in range(g.rng.choice([0, 1, 1, 2, 2, 3])):
            group_size = g.rng.choice([0, 1, 2, 2, 3])
            hand_size = g.rng.randint(0, room // max(group_size, 1)) if room >= 0 else 0
            room -= hand_size * group_size
            groups.append([hand_size, group_size])
        return {"op": "deal_groups", "deck": d, "hand_groups": groups}
    hand_sizes = []
    for _ in range(g.rng.choice([0, 1, 2, 2, 2, 3, 3])):
        hand_size = g.rng.randint(0, max(room, 0))
        room -= hand_size
        hand_sizes.append(hand_size)
    hand_sizes = hand_sizes if g.rng.random() < 0.8 else {"tuple": hand_sizes}
    return {"op": "multi_deal", "deck": d, "hand_sizes": hand_sizes}


@node("multi_deal_info", "value", gen=lambda g, depth: {"op": "multi_deal_info",
                                                        "multi_deal": gen_multi_deal(g, depth)}, weight=0.4)
def multi_deal_info(multi_deal):
    """(tuple(deck items), hand_sizes(), total_cards_dealt(), len(md), denominator(), tuple(outcomes()))."""
    md = multi_deal
    return (tuple(md.deck().items()), md.hand_sizes(), md.total_cards_dealt(), len(md), md.denominator(),
            tuple(md.outcomes()))


def _gen_multi_deal_pop(g, depth):
    md = gen_multi_deal(g, depth)
    order = g.rng.choice([1, 1, -1, -1, 0])
    outcome = g.outcome()
    try:
        outcomes = peek(md).outcomes()
        if outcomes and g.rng.random() < 0.85:
            outcome = enc(outcomes[0] if order > 0 else outcomes[-1])
    except Exception:  # noqa: BLE001
        pass
    return {"op": "multi_deal_pop", "multi_deal": md, "order": order, "outcome": outcome}


@node("multi_deal_pop", "value", gen=_gen_multi_deal_pop, weight=0.5)
def multi_deal_pop(multi_deal, order, outcome):
    """multi_deal._make_source().pop(Order(order), outcome) as a tuple of (deck items, hand_groups, counts,
    weight): each way the popped outcome's cards go to the hands (counts sorted descending within a group), last
    the ways that fill every hand."""
    fresh_caches()
    return tuple((tuple(s.deck.items()), s.hand_groups, counts, weight)
                 for s, counts, weight in multi_deal._make_source().pop(icepool.Order(order), outcome))


def _gen_multi_deal_evaluate(g, depth):
    from harness.catalogue.evaluators import ENTRIES
    from harness.grammar.evaluate import gen_pool
    # Mostly the evaluators written for tuple counts; the others see a tuple where they expect an int.
    name = g.rng.choice(["hand_sums", "record", "record"]) if g.rng.random() < 0.75 else g.rng.choice(
        sorted(ENTRIES))
    e = ENTRIES[name]
    n = e.inputs(g) if name != "record" else g.rng.choice([1, 1, 1, 2])
    args = [gen_multi_deal(g, depth)]
    for _ in range(n - 1):
        args.append(gen_multi_deal(g, depth) if g.rng.random() < 0.5 else gen_pool(g, 0))
    g.rng.shuffle(args)
    return {"op": "multi_deal_evaluate", "name": {"str": name}, "params": e.params(g), "args": args}


@node("multi_deal_evaluate", "die", gen=_gen_multi_deal_evaluate, weight=0.5)
def multi_deal_evaluate(name, params, args):
    """EVALUATORS[name](*params).evaluate(*args) with a MultiDeal among the inputs: its count for each outcome is
    a tuple (one count per hand), and its size the tuple of hand sizes."""
    from harness.catalogue.evaluators import EVALUATORS
    fresh_caches()
    return EVALUATORS[name](*params).evaluate(*args)
