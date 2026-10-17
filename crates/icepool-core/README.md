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
(they truncate; Python's `//` floors).

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
