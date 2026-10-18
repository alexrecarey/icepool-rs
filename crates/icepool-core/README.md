# icepool-core

A Rust port of [icepool](https://github.com/HighDiceRoller/icepool), the Python dice probability library: exact
probabilities of dice, pools of dice and decks, with integer quantities that never round. It is the core of the
`icepool_rs` Python backend and of the `icepool-wasm` JavaScript package, and gives the same answers as Python
icepool (the version pinned in `oracle.lock`), down to the order of outcomes and every integer quantity.

## First dice

A die is a finite distribution: each outcome with an integer quantity (its weight). [`d`] makes a standard die.

```
use icepool_core::{d, Die};

let d6: Die<i64> = d(6);
assert_eq!(d6.len(), 6);
assert_eq!(d6.denominator().to_string(), "6");

// Other ways to make a die: from outcomes (equal ones add up), from weights, from a range, a single outcome.
assert_eq!(Die::from_iter([1, 2, 2, 3]), Die::from_weights([(1, 1u32), (2, 2), (3, 1)]));
assert_eq!(Die::from(1..=6), d6);
let five = Die::single(5);
assert_eq!(five.len(), 1);
```

Dice are immutable; every operation returns a new die. Cloning is cheap: a die shares its outcomes.

## Operators

The arithmetic operators combine every pair of outcomes, multiplying their quantities; equal results add up. A
plain outcome on either side stands for the die that always rolls it.

```
use icepool_core::{d, Die};

let two_d6 = &d(6) + &d(6);
assert_eq!(two_d6.quantity_le(&2).to_string(), "1");  // snake eyes: 1 way in 36
assert_eq!(two_d6.denominator().to_string(), "36");

assert_eq!(d(6) - 1, Die::from(0..=5));
assert_eq!(7 - d(6), d(6));                          // the die is symmetric
assert_eq!(d(6).sum_rolls(3), &two_d6 + d(6));       // Python's 3 @ d6
```

Integer overflow panics, in release builds too (`attempt to add with overflow`): the operators use checked
arithmetic, because a silently wrapped outcome would be a wrong probability. `/` and `%` are Rust's operators
(they truncate). Python's operators that Rust lacks are methods: `//` is [`Die::floor_div`] (it floors), `**` is
[`Die::pow`] and `@` is [`Die::sum_rolls`].

```
use icepool_core::{d, Die};

assert_eq!((-d(3)).floor_div(2), Die::from_iter([-1, -1, -2]));  // Python's -d(3) // 2
assert_eq!(d(3).pow(2), Die::from_iter([1, 4, 9]));
```

Comparisons are methods that return dice of `bool`; `==` compares two dice as distributions (Python's
`die.equals`).

```
use icepool_core::{d, Die};

let hits: Die<bool> = d(20).ge(11);
assert_eq!(hits, Die::from_weights([(false, 10u32), (true, 10)]));
assert_eq!(d(6).gt(d(6)), Die::from_weights([(false, 21u32), (true, 15)]));

// `map` applies any function to the outcomes; `binary` combines two dice with any function.
let damage = hits.map(|&hit| if hit { 6 } else { 0 });
assert_eq!(damage.outcomes().copied().collect::<Vec<_>>(), [0, 6]);
let highest = d(6).binary(&d(6), |a, b| *a.max(b));
assert_eq!(highest.quantity_ge(&6).to_string(), "11");
```

## Reading a die

Outcomes are always in ascending order. A die iterates over its `(outcome, quantity)` pairs, and prints as Python
prints it: a markdown table.

```
use icepool_core::d;

let d4 = d(4);
for (outcome, quantity) in &d4 {
    println!("{outcome}: {quantity}");
}
assert_eq!(d4.quantities().count(), 4);
assert_eq!(d4.quantity_ge(&3).to_string(), "2");
assert!(d4.to_string().starts_with("Die with denominator 4"));
```

## Statistics

Means, variances and probabilities are exact fractions, [`Ratio`] (Python's `Fraction`); comparisons are named
by [`Comparison`].

```
use icepool_core::{d, Comparison, Ratio};

let three_d6 = d(6).sum_rolls(3);
assert_eq!(three_d6.mean(), Ratio::from_parts(21.into(), 2u8.into()));
assert_eq!(three_d6.variance().to_string(), "35/4");
assert_eq!(three_d6.probability(Comparison::Ge, &15).to_string(), "5/54");
assert_eq!(three_d6.standard_deviation(), 2.958039891549808);           // the same float as Python's
```

[`pointwise_max`] and [`pointwise_min`] pick, for every outcome, the best or worst chance of rolling at least it
among several dice: useful to choose between moves when the target number may change.

```
use icepool_core::{d, pointwise_max, Ratio};

let best = pointwise_max(&[d(8), d(4) + 2]);
assert_eq!(best.mean(), Ratio::from(5));
```

## Pools and evaluators

A [`Pool`] is several dice rolled together, of which some may be kept. An evaluator walks the outcomes of a
pool one at a time, in order, with how many dice rolled each, and the result is a die of its final outcomes. The
built-in evaluators live in [`evaluator`].

```
use icepool_core::evaluator::poker::LargestCountEvaluator;
use icepool_core::{d, Die, Int};

// How many of a kind in 3d6.
let largest = d(6).pool(3).evaluate(&LargestCountEvaluator).unwrap();
assert_eq!(largest, Die::from_weights([(Int::from(1), 120u32), (Int::from(2), 90), (Int::from(3), 6)]));
```

Writing an evaluator is implementing [`evaluator::MultisetEvaluator`]: an initial state, the next state for each
outcome and its count, and the final outcome. Here, the sum of the three highest of 4d6:

```
use icepool_core::evaluator::{Final, MultisetEvaluator, Order};
use icepool_core::{d, Int, Result};

/// Sums the kept dice.
struct Sum;

impl MultisetEvaluator<i64> for Sum {
    type State = i64;
    type Final = i64;

    fn initial_state(&self, _order: Order, _outcomes: &[i64], _sizes: &[Option<Int>]) -> Result<i64> {
        Ok(0)
    }

    fn next_state(&self, sum: &i64, _order: Order, outcome: &i64, counts: &[Int]) -> Result<Option<i64>> {
        // One count per pool evaluated: how many of the kept dice rolled `outcome`.
        let count = counts[0].to_i64().expect("a pool of a few dice");
        Ok(Some(sum + outcome * count))
    }

    fn final_outcome(&self, sum: &i64, _order: Order, _outcomes: &[i64], _sizes: &[Option<Int>]) -> Result<Final<i64>> {
        Ok(Final::Outcome(*sum))
    }
}

let best_three = d(6).pool(4).highest(3).evaluate(&Sum).unwrap();
assert_eq!(best_three.denominator().to_string(), "1296");
assert_eq!(best_three.quantity_ge(&18).to_string(), "21");
```

The evaluator remembers every intermediate state, so a pool of many dice costs about as much as its distinct
states, not its rolls.

## Map, reroll, explode and `Again`

[`Die::map`] applies a function to each outcome. Python's mapping functions may also return a die, reroll, or
stop a repeated map: in Rust that is [`Die::try_map`] with [`Mapped`], and rerolling has its own methods, which
reroll up to a [`Depth`].

```
use icepool_core::{d, Depth, Die, Ratio};

// Reroll 1s, once: a 1 now needs two 1s in a row.
let rerolled = d(6).reroll(|&x| x == 1, Depth::Finite(1)).unwrap();
assert_eq!(rerolled, Die::from_weights([(1, 1u32), (2, 7), (3, 7), (4, 7), (5, 7), (6, 7)]));
assert_eq!(rerolled.mean(), Ratio::from_parts(47.into(), 12u8.into()));
```

[`Die::explode`] rolls again and adds when the outcome is one of the chosen ones, up to a depth; past the depth,
[`AgainEnd`] says what the last roll is (here zero, Python's default).

```
use icepool_core::{d, AgainEnd, Ratio};

let exploding = d(6).explode(|&x| x == 6, 2, AgainEnd::Zero).unwrap();
assert_eq!(exploding.denominator().to_string(), "216");
assert_eq!(exploding.mean(), Ratio::from_parts(301.into(), 72u8.into()));
assert_eq!(exploding.max_outcome().unwrap(), &18);
```

[`Again`] is the general form: an expression standing for the die rolled again, built with the arithmetic
operators. [`Die::map_again`] maps outcomes to plain outcomes or such expressions; [`Die::with_again_depth`]
builds a die from them directly.

```
use icepool_core::{d, Again, AgainEnd, AgainOutcome, Die};

// A d6 whose 6 is "6 + roll again": the exploding d6 above, once.
let six_again = d(6).map_again(
    |&x| if x == 6 { (6i64 + Again::new()).into() } else { AgainOutcome::from(x) },
    1,
    AgainEnd::Zero,
);
assert_eq!(six_again.unwrap(), d(6).explode(|&x| x == 6, 1, AgainEnd::Zero).unwrap());
```

[`Die::explode_to_pool`] keeps each explosion as a die of its own, and [`Die::reroll_to_pool`] shares a limited
number of rerolls across a pool. Both return a [`PoolMixture`]: a weighted mixture of pools, which keeps and
evaluates as a pool does.

```
use icepool_core::evaluator::basic::SumEvaluator;
use icepool_core::{d, Depth, Die, RerollMode};

// Python: `d(6).explode_to_pool(2, depth=1).highest(1).sum()`.
let best = d(6).explode_to_pool(2, |&x| x == 6, 1).unwrap().highest(1);
let expected = [36u32, 108, 180, 252, 324, 396];
assert_eq!(best.evaluate(&SumEvaluator).unwrap(), Die::from_weights((1..=6).zip(expected)));

// Python: `d(4).reroll_to_pool(3, [1, 2], max_rerolls=1, mode='low').sum()`.
let rerolled = d(4).reroll_to_pool(3, |&x| x <= 2, Some(1), Depth::Finite(1), RerollMode::Low).unwrap();
let expected = [1u32, 4, 11, 23, 37, 46, 49, 45, 30, 10];
assert_eq!(rerolled.evaluate(&SumEvaluator).unwrap(), Die::from_weights((3..=12).zip(expected)));
```

## Decks

A [`Deck`] holds cards; a [`Deal`] is a hand of cards dealt from it without replacement, which keeps and evaluates
as a pool does.

```
use icepool_core::evaluator::poker::LargestCountEvaluator;
use icepool_core::{Comparison, Deck, Int};

// Five cards from a standard deck: at least a pair in 1,281,072 of the 2,598,960 hands.
let deck = Deck::from_weights((1..=13).map(|rank| (rank, 4u32)));
let hand = deck.deal(5);
assert_eq!(hand.denominator().to_string(), "2598960");
let largest = hand.evaluate(&LargestCountEvaluator).unwrap();
assert_eq!(largest.quantity_ge(&Int::from(2)).to_string(), "1281072");
assert_eq!(largest.probability(Comparison::Ge, &Int::from(2)).to_string(), "2053/4165");

// Two hands of five (Python's `deck.deal([5, 5])`): a `MultiDeal`, which counts each card per hand.
let hands = deck.deal_hands(&[5, 5]);
assert_eq!(hands.denominator().to_string(), "3986646103440");
```

Pools, decks and deals print as Python prints them (`str(pool)`, `str(deck)`).

```
use icepool_core::{d, Deck};

assert_eq!(
    d(4).pool(3).highest(2).to_string(),
    "Pool of 3 dice with keep_tuple=(0, 1, 1)\n  Die({1: 1, 2: 1, 3: 1, 4: 1}) : 3,\n"
);
assert!(Deck::from_iter([1, 1, 2]).to_string().starts_with("Deck with denominator 3\n"));
```

## Exact quantities

Quantities are [`Quantity`] (an unbounded unsigned integer) and multiset counts are [`Int`] (an `i64` that grows
when it must): nothing is ever rounded, however large the denominator.

```
use icepool_core::d;

let many = d(6).sum_rolls(40);
assert_eq!(many.denominator().to_string(), "13367494538843734067838845976576"); // 6**40
```

## Typed outcomes and `Value`

Any type with a total order (`Ord + Hash + Clone + Debug + Send + Sync`) is an [`Outcome`]: integers, `bool`,
`char`, `String`, tuples and arrays of them, your own enums. On such dice, comparisons cannot fail and the methods
return plain values. The arithmetic operators need [`OutcomeArith`] (implemented for the signed integers).

The language bindings use [`Value`] instead: an outcome with Python's semantics (`1 == True`, `1 < 'a'` raises),
whose methods return a [`Result`]. Python behaviour lives only there; the typed API never sees it.

```
use icepool_core::{Die, Value};

let coin: Die<Value> = Die::from_dict(vec![(Value::Bool(false), 1u8.into()), (Value::from(1), 1u8.into())]).unwrap();
assert_eq!(coin.len(), 2);
```

## Errors and panics

Fallible operations return [`Result`] with an [`Error`] named after the Python exception icepool raises in the
same place (`d(0)` is a `ValueError` there). The typed conveniences panic instead where Python would raise for a
reason that is the caller's mistake (`d(0)`, overflow), and document it under *Panics*; each has a fallible form.
