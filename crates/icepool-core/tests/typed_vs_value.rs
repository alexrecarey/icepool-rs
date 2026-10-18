//! Property tests: every typed `Die<i64>` path agrees exactly with the `Value` path on the same integer dice
//! (docs/design/library-architecture.md section 9.7).
//!
//! The `Value` path is what the bindings run, and the fixtures check it against Python icepool; the typed path is
//! the Rust API. Both run the same generic core, but the typed one has its own operators, comparisons, keeps and
//! evaluator entry points, so these tests hold the two together: same outcomes, same order, same quantities. Pools
//! are also checked against brute-force enumeration of every roll, which shares no code with the evaluator.
//!
//! The dice come from a small seeded generator, so a failure names its case and reproduces exactly; no Python is
//! needed.

use icepool_core::evaluator::poker::{
    AllCountsEvaluator, LargestCountEvaluator, LargestStraightEvaluator,
};
use icepool_core::evaluator::{self, Final, MultisetEvaluator, MultisetSource, Order};
use icepool_core::value::TransitionCache;
use icepool_core::value::evaluators::IntoValue;
use icepool_core::value::{matmul_dice, BinOp, UnOp};
use icepool_core::{AgainEnd, Depth, Die, Int, Mapped, Outcome, Pool, Quantity, Result, Value};

/// How many random cases each property tries.
const CASES: usize = 300;

/// A small xorshift generator: deterministic, so every run tests the same dice.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `lo..=hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }

    /// A die of one to four outcomes in `lo..=hi`, each with a quantity of one to three. Outcomes may repeat,
    /// so quantities merge as `Die([...])` merges them.
    fn die(&mut self, lo: i64, hi: i64) -> Die<i64> {
        let len = self.range(1, 4);
        Die::from_weights((0..len).map(|_| (self.range(lo, hi), self.range(1, 3) as u32)))
    }
}

/// Runs `check` on `CASES` generated cases, naming the case that fails.
fn for_cases(seed: u64, mut check: impl FnMut(&mut Rng, usize)) {
    let mut rng = Rng::new(seed);
    for case in 0..CASES {
        check(&mut rng, case);
    }
}

/// The same die with `Value` outcomes.
fn to_value<O: Outcome + IntoValue>(die: &Die<O>) -> Die<Value> {
    Die::from_dict(
        die.items()
            .iter()
            .map(|(o, q)| (o.clone().into_value(), q.clone()))
            .collect(),
    )
    .unwrap()
}

/// The items of a die of `Value`s, as their `Debug` spelling: `Value`'s `==` is Python's (`1 == True`), and
/// these tests want the types to match too.
fn spelled(die: &Die<Value>) -> Vec<(String, Quantity)> {
    die.items()
        .iter()
        .map(|(o, q)| (format!("{o:?}"), q.clone()))
        .collect()
}

/// Asserts that a typed die and a `Value` die have the same outcomes, types, order and quantities.
#[track_caller]
fn assert_same<O: Outcome + IntoValue>(typed: &Die<O>, value: &Die<Value>, case: &str) {
    assert_eq!(spelled(&to_value(typed)), spelled(value), "{case}");
}

/// `a op b` on `Value` dice, as the bindings compute it (`Die.binary_operator`).
fn value_binary(a: &Die<i64>, b: &Die<i64>, op: BinOp) -> Die<Value> {
    to_value(a)
        .binary_operator(&to_value(b), |x, y| x.binary(op, y))
        .unwrap()
}

#[test]
fn arithmetic_operators_agree() {
    for_cases(1, |rng, case| {
        let (a, b) = (rng.die(-6, 6), rng.die(-6, 6));
        let case = format!("case {case}: {a:?} op {b:?}");
        assert_same(&(&a + &b), &value_binary(&a, &b, BinOp::Add), &case);
        assert_same(&(&a - &b), &value_binary(&a, &b, BinOp::Sub), &case);
        assert_same(&(&a * &b), &value_binary(&a, &b, BinOp::Mul), &case);
        assert_same(&(&a & &b), &value_binary(&a, &b, BinOp::And), &case);
        assert_same(&(&a | &b), &value_binary(&a, &b, BinOp::Or), &case);
        assert_same(&(&a ^ &b), &value_binary(&a, &b, BinOp::Xor), &case);
        let neg = to_value(&a).unary_operator(|x| x.unary(UnOp::Neg)).unwrap();
        assert_same(&-&a, &neg, &case);
    });
}

#[test]
fn division_and_shifts_agree_where_rust_and_python_mean_the_same() {
    // Rust's `/` and `%` truncate and Python's `//` and `%` floor: they agree on non-negative operands. Shifts
    // agree for small non-negative amounts (`>>` is arithmetic on negative numbers in both).
    for_cases(2, |rng, case| {
        let (a, b, shift) = (rng.die(0, 9), rng.die(1, 4), rng.die(0, 5));
        let case = format!("case {case}: {a:?} op {b:?}, shift {shift:?}");
        assert_same(&(&a / &b), &value_binary(&a, &b, BinOp::FloorDiv), &case);
        assert_same(&(&a % &b), &value_binary(&a, &b, BinOp::Mod), &case);
        let signed = rng.die(-9, 9);
        assert_same(
            &(&signed << &shift),
            &value_binary(&signed, &shift, BinOp::LShift),
            &case,
        );
        assert_same(
            &(&signed >> &shift),
            &value_binary(&signed, &shift, BinOp::RShift),
            &case,
        );
    });
}

#[test]
fn floor_div_and_pow_agree_with_pythons_operators() {
    // `Die::floor_div` is Python's `//` on any signs (a divisor die without 0); `Die::pow` is `**` for
    // non-negative exponents (a negative one is a float in Python).
    for_cases(12, |rng, case| {
        let (a, b, exponent) = (rng.die(-9, 9), rng.die(1, 4), rng.die(0, 4));
        let b = if rng.range(0, 1) == 0 { -b } else { b };
        let case = format!("case {case}: {a:?} op {b:?}, ** {exponent:?}");
        assert_same(&a.floor_div(&b), &value_binary(&a, &b, BinOp::FloorDiv), &case);
        assert_same(&a.pow(&exponent), &value_binary(&a, &exponent, BinOp::Pow), &case);
    });
}

#[test]
fn comparisons_agree() {
    for_cases(3, |rng, case| {
        let (a, b) = (rng.die(-3, 3), rng.die(-3, 3));
        let case = format!("case {case}: {a:?} cmp {b:?}");
        assert_same(&a.lt(&b), &value_binary(&a, &b, BinOp::Lt), &case);
        assert_same(&a.le(&b), &value_binary(&a, &b, BinOp::Le), &case);
        assert_same(&a.gt(&b), &value_binary(&a, &b, BinOp::Gt), &case);
        assert_same(&a.ge(&b), &value_binary(&a, &b, BinOp::Ge), &case);
        let (va, vb) = (to_value(&a), to_value(&b));
        let eq = |ne| va.binary_operator(&vb, |x, y| x.eq_op(y, ne)).unwrap();
        assert_same(&a.equal(&b), &eq(false), &case);
        assert_same(&a.not_equal(&b), &eq(true), &case);
        // `==` on dice is Python's `die.equals`.
        assert_eq!(a == b, va.equals(&vb, false).unwrap(), "{case}");
    });
}

#[test]
fn sum_rolls_agrees_with_matmul() {
    for_cases(4, |rng, case| {
        let (die, rolls) = (rng.die(-3, 3), rng.range(-3, 4));
        let case = format!("case {case}: {rolls} @ {die:?}");
        let value = matmul_dice(&to_value(&Die::single(rolls)), &to_value(&die)).unwrap();
        assert_same(&die.sum_rolls(rolls), &value, &case);
    });
}

#[test]
fn map_agrees_with_unary_operator() {
    for_cases(5, |rng, case| {
        let die = rng.die(-4, 4);
        let case = format!("case {case}: {die:?}");
        let square = to_value(&die)
            .unary_operator(|x| x.binary(BinOp::Mul, x))
            .unwrap();
        assert_same(&die.map(|x| x * x), &square, &case);
        // `map` to another type: outcomes that collide add up and sort by the new type's order.
        let positive = to_value(&die)
            .unary_operator(|x| x.binary(BinOp::Gt, &Value::from(0)))
            .unwrap();
        assert_same(&die.map(|&x| x > 0), &positive, &case);
    });
}

#[test]
fn clip_truncate_and_quantities_agree() {
    for_cases(6, |rng, case| {
        let die = rng.die(-5, 5);
        let (lo, hi) = (rng.range(-6, 2), rng.range(-2, 6));
        let case = format!("case {case}: {die:?} in {lo}..={hi}");
        let value = to_value(&die);
        let (vlo, vhi) = (Value::from(lo), Value::from(hi));
        assert_same(
            &die.clip(Some(&lo), Some(&hi)),
            &value.try_clip(Some(&vlo), Some(&vhi)).unwrap(),
            &case,
        );
        assert_same(
            &die.truncate(Some(&lo), Some(&hi)),
            &value.try_truncate(Some(&vlo), Some(&vhi)).unwrap(),
            &case,
        );
        assert_eq!(
            die.quantity_le(&lo),
            value.try_quantity_le(&vlo).unwrap(),
            "{case}"
        );
        assert_eq!(
            die.quantity_ge(&hi),
            value.try_quantity_ge(&vhi).unwrap(),
            "{case}"
        );
    });
}

/// Every roll of `dice` (one die per roll), with its weight: the product of the quantities.
fn every_roll(dice: &[Die<i64>]) -> Vec<(Vec<i64>, Quantity)> {
    dice.iter()
        .fold(vec![(Vec::new(), Quantity::ONE)], |rolls, die| {
            rolls
                .iter()
                .flat_map(|(roll, weight)| {
                    die.items().iter().map(move |(o, q)| {
                        let mut roll = roll.clone();
                        roll.push(*o);
                        (roll, weight * q)
                    })
                })
                .collect()
        })
}

/// The die of `f(sorted kept rolls)` over every roll of `pool`'s dice, by brute force.
fn enumerate<P: Outcome>(pool: &Pool<i64>, f: impl Fn(&[i64]) -> P) -> Die<P> {
    let dice: Vec<Die<i64>> = pool
        .dice()
        .iter()
        .flat_map(|(die, n)| std::iter::repeat_n(die.clone(), *n))
        .collect();
    Die::from_weights(every_roll(&dice).into_iter().map(|(mut roll, weight)| {
        roll.sort();
        // The keep tuple counts each sorted roll that many times.
        let kept: Vec<i64> = roll
            .iter()
            .zip(pool.keep_tuple())
            .flat_map(|(o, k)| std::iter::repeat_n(*o, k.to_usize().unwrap()))
            .collect();
        (f(&kept), weight)
    }))
}

/// The sum of the kept rolls, as a typed evaluator.
struct Sum;

impl MultisetEvaluator<i64> for Sum {
    type State = i64;
    type Final = i64;

    fn initial_state(&self, _: Order, _: &[i64], _: &[Option<Int>]) -> Result<i64> {
        Ok(0)
    }

    fn next_state(
        &self,
        sum: &i64,
        _: Order,
        outcome: &i64,
        counts: &[Int],
    ) -> Result<Option<i64>> {
        Ok(Some(sum + outcome * counts[0].to_i64().unwrap()))
    }

    fn final_outcome(
        &self,
        sum: &i64,
        _: Order,
        _: &[i64],
        _: &[Option<Int>],
    ) -> Result<Final<i64>> {
        Ok(Final::Outcome(*sum))
    }
}

/// A random pool of one or two kinds of dice, keeping some of the highest or lowest rolls.
fn random_pool(rng: &mut Rng) -> Pool<i64> {
    let mut dice = vec![(rng.die(1, 6), rng.range(1, 3) as usize)];
    if rng.range(0, 1) == 1 {
        dice.push((rng.die(1, 6), rng.range(1, 2) as usize));
    }
    let pool = Pool::new(dice).unwrap();
    let keep = rng.range(0, pool.raw_size() as i64) as usize;
    match rng.range(0, 2) {
        0 => pool,
        1 => pool.highest(keep),
        _ => pool.lowest(keep),
    }
}

/// The same pool with `Value` outcomes and the same keep tuple.
fn value_pool(pool: &Pool<i64>) -> Pool<Value> {
    let dice: Vec<_> = pool
        .dice()
        .iter()
        .map(|(die, n)| (to_value(die), *n))
        .collect();
    Pool::from_parts(&dice, pool.keep_tuple().into()).unwrap()
}

/// `evaluator` on the `Value` pool, as the bindings run it (before Python's finals are spelled as Python values).
fn value_evaluate<E: MultisetEvaluator<Value>>(
    evaluator: &E,
    pool: &Pool<i64>,
) -> Vec<(E::Final, Quantity)> {
    evaluator::evaluate(
        evaluator,
        &[MultisetSource::Pool(value_pool(pool).source())],
    )
    .unwrap()
}

#[test]
fn pool_evaluation_agrees_with_value_pools_and_enumeration() {
    for_cases(7, |rng, case| {
        let pool = random_pool(rng);
        let case = format!(
            "case {case}: {:?} keeping {:?}",
            pool.dice(),
            pool.keep_tuple()
        );

        let sum = pool.evaluate(&Sum).unwrap();
        assert_eq!(
            sum,
            enumerate(&pool, |kept| kept.iter().sum::<i64>()),
            "{case}"
        );

        let largest = pool.evaluate(&LargestCountEvaluator).unwrap();
        assert_eq!(
            largest.items(),
            Die::from_weights(value_evaluate(&LargestCountEvaluator, &pool)).items(),
            "{case}"
        );
        let largest_count = |kept: &[i64]| {
            let count = |x| kept.iter().filter(|&&y| y == x).count();
            Int::from(kept.iter().map(|&x| count(x)).max().unwrap_or(0) as i64)
        };
        assert_eq!(largest, enumerate(&pool, largest_count), "{case}");

        let straight = pool.evaluate(&LargestStraightEvaluator).unwrap();
        assert_eq!(
            straight.items(),
            Die::from_weights(value_evaluate(&LargestStraightEvaluator, &pool)).items(),
            "{case}"
        );

        let all = AllCountsEvaluator { filter: None };
        assert_eq!(
            pool.evaluate(&all).unwrap().items(),
            Die::from_weights(value_evaluate(&all, &pool)).items(),
            "{case}"
        );
    });
}

#[test]
fn single_die_keeps_agree_with_pools() {
    // `highest_single(n)` is the die of the highest of `n` rolls, which is also the pool `n @ die` keeping one.
    for_cases(8, |rng, case| {
        let (die, rolls) = (rng.die(-3, 3), rng.range(1, 4) as usize);
        let case = format!("case {case}: {rolls} x {die:?}");
        let pool = die.pool(rolls);
        assert_eq!(
            die.highest_single(rolls).unwrap(),
            pool.highest(1).evaluate(&Sum).unwrap(),
            "{case}"
        );
        assert_eq!(
            die.lowest_single(rolls).unwrap(),
            pool.lowest(1).evaluate(&Sum).unwrap(),
            "{case}"
        );
    });
}

#[test]
fn reroll_and_filter_agree_with_value_masks() {
    for_cases(9, |rng, case| {
        let die = rng.die(-3, 3);
        let threshold = rng.range(-3, 3);
        let depth = match rng.range(0, 3) {
            3 => Depth::Infinite,
            n => Depth::Finite(n as u32),
        };
        let case = format!("case {case}: {die:?} reroll <= {threshold}, {depth:?}");
        // The bindings compute the selection themselves, one flag per outcome, and call `reroll_where`.
        let value = to_value(&die);
        let mask: Vec<bool> = die.outcomes().map(|&x| x <= threshold).collect();
        let unmask: Vec<bool> = mask.iter().map(|m| !m).collect();
        match (
            die.reroll(|&x| x <= threshold, depth),
            value.reroll_where(&mask, depth),
        ) {
            (Ok(typed), Ok(value)) => assert_same(&typed, &value, &case),
            (typed, value) => assert_eq!(typed.err(), value.err(), "{case}"),
        }
        match (
            die.filter(|&x| x <= threshold, depth),
            value.reroll_where(&unmask, depth),
        ) {
            (Ok(typed), Ok(value)) => assert_same(&typed, &value, &case),
            (typed, value) => assert_eq!(typed.err(), value.err(), "{case}"),
        }
    });
}

#[test]
fn explode_agrees_on_value_outcomes() {
    for_cases(10, |rng, case| {
        let die = rng.die(-2, 4);
        let which = rng.range(-2, 4);
        let depth = rng.range(0, 3) as u64;
        let case = format!("case {case}: {die:?}.explode([{which}], depth={depth})");
        for end in [AgainEnd::Zero, AgainEnd::Reroll, AgainEnd::Outcome(1)] {
            let value_end = match &end {
                AgainEnd::Outcome(o) => AgainEnd::Outcome(Value::from(*o)),
                AgainEnd::Zero => AgainEnd::Zero,
                _ => AgainEnd::Reroll,
            };
            let typed = die.explode(|&x| x == which, depth, end);
            let value = to_value(&die).explode(|x| *x == Value::from(which), depth, value_end);
            // A die that explodes on every outcome has no zero to end with: both raise.
            match (typed, value) {
                (Ok(typed), Ok(value)) => assert_same(&typed, &value, &case),
                (typed, value) => assert_eq!(typed.err(), value.err(), "{case}"),
            }
        }
    });
}

#[test]
fn repeated_maps_agree_with_value_states() {
    for_cases(11, |rng, case| {
        let die = rng.die(0, 4);
        let (step, cap) = (rng.range(1, 2), rng.range(3, 6));
        let repeat = rng.range(1, 4) as u64;
        let case =
            format!("case {case}: {die:?}.map(lambda x: min(x + {step}, {cap}), repeat={repeat})");
        // `lambda x: min(x + step, cap)`, typed and as the bindings run it.
        let typed_step = |&x: &i64| Mapped::Outcome((x + step).min(cap));
        let value_step = |x: &Value, _: usize| -> Result<Mapped<Value>> {
            let next = x.binary(BinOp::Add, &Value::from(step))?;
            Ok(Mapped::Outcome(Value::py_min(next, Value::from(cap))?))
        };
        let value_cache = || TransitionCache::new(value_step, vec![Quantity::ONE]);
        let value = icepool_core::value::map_repeat(&mut value_cache(), &to_value(&die), Some(repeat));
        assert_same(
            &die.map_repeat(typed_step, repeat).unwrap(),
            &value.unwrap(),
            &case,
        );
        let (value, time) =
            icepool_core::value::absorbing_markov_chain(&mut value_cache(), &to_value(&die)).unwrap();
        assert_same(&die.map_to_absorption(typed_step).unwrap(), &value, &case);
        assert_eq!(
            die.mean_time_to_absorb(typed_step).unwrap(),
            time.unwrap(),
            "{case}"
        );
        // `map_and_time`: typed `(state, time)` pairs against Python's tuples.
        let typed = die.map_and_time(typed_step, repeat).unwrap();
        let value =
            icepool_core::value::map_and_time(&mut value_cache(), &to_value(&die), Some(repeat)).unwrap();
        let typed: Vec<_> = typed
            .items()
            .iter()
            .map(|((s, t), q)| {
                (
                    format!(
                        "{:?}",
                        Value::tuple(vec![Value::from(*s), Value::from(*t as i64)])
                    ),
                    q.clone(),
                )
            })
            .collect();
        assert_eq!(typed, spelled(&value), "{case}");
    });
}

#[test]
fn statistics_agree_with_python_statistics() {
    use icepool_core::value::PyStatistics;
    use icepool_core::{Comparison, Ratio};
    let fraction = |r: Ratio| Value::Fraction(r);
    for_cases(13, |rng, case| {
        let die = rng.die(-6, 9);
        let value = to_value(&die);
        let case = format!("case {case}: {die:?}");
        assert_eq!(fraction(die.mean()), value.mean().unwrap(), "{case}");
        assert_eq!(
            fraction(die.variance()),
            value.variance().unwrap(),
            "{case}"
        );
        assert_eq!(
            Value::Float(die.standard_deviation()),
            value.standard_deviation().unwrap(),
            "{case}"
        );
        let outcome = rng.range(-7, 10);
        for comparison in [
            Comparison::Eq,
            Comparison::Lt,
            Comparison::Ge,
            Comparison::Ne,
        ] {
            assert_eq!(
                fraction(die.probability(comparison, &outcome)),
                value
                    .probability(comparison, &Value::from(outcome), false)
                    .unwrap(),
                "{case}"
            );
            let typed: Vec<Value> = die
                .probabilities(comparison)
                .into_iter()
                .map(fraction)
                .collect();
            assert_eq!(
                typed,
                value.probabilities(comparison, false).unwrap(),
                "{case}"
            );
        }
        // The draw `Population.sample` makes for each `r` below the denominator.
        let r = Quantity::from(rng.range(0, 5) as u64) % die.denominator();
        assert_eq!(
            die.sample_at(&r).unwrap().into_value(),
            value.sample_at(&r).unwrap(),
            "{case}"
        );
        // Formatting goes through `FormatOutcome`, not the `Value` conversion: both print the same.
        for spec in ["", "md:*oq<=p>=i==", "csv"] {
            assert_eq!(
                die.format(spec).unwrap(),
                value.format(spec).unwrap(),
                "{case}"
            );
        }
    });
}

#[test]
fn tupleize_agrees_with_value_tupleize() {
    use icepool_core::value::tupleize_dice;
    for_cases(14, |rng, case| {
        let (a, b) = (rng.die(-2, 3), rng.die(0, 2).ge(1));
        let value_items = |die: &Die<Value>| die.items().to_vec();
        let python =
            tupleize_dice(&[value_items(&to_value(&a)), value_items(&to_value(&b))]).unwrap();
        assert_same(&a.tupleize(&b), &python, &format!("case {case}"));
        let python =
            tupleize_dice(&[value_items(&to_value(&a)), value_items(&to_value(&a))]).unwrap();
        assert_same(
            &Die::cartesian_product(&[a.clone(), a.clone()]),
            &python,
            &format!("case {case}"),
        );
    });
}
