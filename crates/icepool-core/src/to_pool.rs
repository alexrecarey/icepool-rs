//! Typed `Die.explode_to_pool` and `Die.reroll_to_pool` (population/die.py): mixtures of pools, one per way the
//! dice can end up, weighted by how likely it is.
//!
//! The arithmetic follows Python's step for step, with typed counts: Python's `Vector((exploded,
//! not_exploded))` is a pair of `usize`, its `(rerollable, rerolls_left)` states and `(rerollable, rerolled,
//! rerolled_to_rerollable)` compositions are tuples of `i64` (signed, as `n @ die` needs). The `Value` versions
//! (`value::explode_counts`, `value::reroll_composition`) keep Python's arithmetic on whatever `rolls` and
//! `max_rerolls` the caller passed; Python's argument handling (`star`, `_select_outcomes`, `'inf'`) stays there.

use crate::creation_args::DieArg;
use crate::die::{Depth, Die};
use crate::error::{Error, Result};
use crate::map_tools::{map_repeat, TransitionCache};
use crate::mixture::{KeepGenerator, PoolMixture};
use crate::outcome::{infallible, Outcome};
use crate::pool::Pool;
use crate::{Mapped, Quantity};

/// How `reroll_to_pool` chooses which dice to reroll if there are more eligible dice than `max_rerolls` (Python's
/// `mode` argument).
///
/// ```
/// use icepool_core::evaluator::basic::SumEvaluator;
/// use icepool_core::{d, Depth, Die, RerollMode};
/// // Python: `d(4).reroll_to_pool(3, [1, 2], max_rerolls=1, mode='low').sum()`.
/// let low = d(4).reroll_to_pool(3, |&x| x <= 2, Some(1), Depth::Finite(1), RerollMode::Low).unwrap();
/// let expected = [1u32, 4, 11, 23, 37, 46, 49, 45, 30, 10];
/// assert_eq!(low.evaluate(&SumEvaluator).unwrap(), Die::from_weights((3..=12).zip(expected)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RerollMode {
    /// `'random'` (Python's default): eligible dice are chosen uniformly at random.
    Random,
    /// `'low'`: the lowest eligible dice are rerolled.
    Low,
    /// `'high'`: the highest eligible dice are rerolled.
    High,
    /// `'drop'`: all dice that ended up on an outcome selected by `which` are dropped. This includes both dice
    /// that rolled into `which` initially and were not rerolled, and dice that were rerolled but rolled into
    /// `which` again. This can be considerably more efficient than the other modes.
    Drop,
}

impl<O: Outcome> Die<O> {
    /// EXPERIMENTAL: causes outcomes to be rolled again, keeping that outcome as an individual die in a pool
    /// (Python's `die.explode_to_pool(rolls, which, depth=depth)`).
    ///
    /// `rolls` is the number of initial dice, `which` selects the outcomes to explode (Python's default is the max
    /// outcome: `|&x| x == max`), `depth` the maximum depth of explosions for an individual die (Python's default
    /// 9). The result is a mixture of pools; it is not technically a pool, though it supports most of the same
    /// operations. With `depth == 0` or nothing to explode it is just `self.pool(rolls)`.
    ///
    /// ```
    /// use icepool_core::evaluator::basic::SumEvaluator;
    /// use icepool_core::{d, Die};
    /// // Python: `d(6).explode_to_pool(2, depth=1).sum()`.
    /// let sum = d(6).explode_to_pool(2, |&x| x == 6, 1).unwrap().evaluate(&SumEvaluator).unwrap();
    /// let expected = [36u32, 72, 108, 144, 180, 144, 120, 96, 72, 48, 60, 60, 49, 38, 27, 16, 5, 6, 5, 4, 3, 2, 1];
    /// assert_eq!(sum, Die::from_weights((2..=24).zip(expected)));
    /// ```
    pub fn explode_to_pool(
        &self,
        rolls: usize,
        which: impl FnMut(&O) -> bool,
        depth: u32,
    ) -> Result<PoolMixture<O>> {
        if depth == 0 {
            return Ok(Pool::new(vec![(self.clone(), rolls)])?.into());
        }
        let selected: Vec<bool> = self.outcomes().map(which).collect();
        if !selected.contains(&true) {
            return Ok(Pool::new(vec![(self.clone(), rolls)])?.into());
        }
        let (explode, not_explode) = infallible(self.split(&selected));
        let count_die = explode_counts(
            &self.denominator(),
            &explode.denominator(),
            &not_explode.denominator(),
            rolls,
            depth,
        )?;
        // `count_die.map_to_pool(lambda x, nx: [explode] * x + [not_explode] * nx)`.
        let inners = count_die
            .items()
            .iter()
            .map(|((x, nx), quantity)| {
                let pool = pool_of(vec![(&explode, *x), (&not_explode, *nx)])?;
                Ok((pool.into(), quantity.clone()))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(PoolMixture::new(inners))
    }

    /// EXPERIMENTAL: applies a limited number of rerolls shared across a pool (Python's
    /// `die.reroll_to_pool(rolls, which, max_rerolls=max_rerolls, depth=depth, mode=mode)`).
    ///
    /// Each die can only be rerolled once (effectively `depth=1`), and no more than `max_rerolls` dice may be
    /// rerolled. `rolls` is how many dice are in the pool, `which` selects the outcomes eligible to be rerolled
    /// (Python's default is the single minimum outcome: `|&x| x == min`). `max_rerolls` is the maximum total
    /// number of rerolls; `None` (Python's `'inf'`) is the same as `self.reroll(which, depth).pool(rolls)`.
    /// `depth` is EXTRA EXPERIMENTAL: the maximum depth of rerolls ([`Depth::Infinite`] = `max_rerolls`). `mode`
    /// says which eligible dice are rerolled if there are more of them than rerolls left.
    ///
    /// A finite `depth` of 0 (with a finite `max_rerolls`) is Python's `ValueError` from `icepool.map`:
    /// [`Error::InvalidArgument`].
    ///
    /// ```
    /// use icepool_core::evaluator::basic::SumEvaluator;
    /// use icepool_core::{d, Depth, Die, RerollMode};
    /// // Python: `d(4).reroll_to_pool(3, [1, 2], max_rerolls=1).sum()`.
    /// let pool = d(4).reroll_to_pool(3, |&x| x <= 2, Some(1), Depth::Finite(1), RerollMode::Random).unwrap();
    /// let expected = [2u32, 6, 14, 26, 36, 44, 46, 42, 30, 10];
    /// assert_eq!(pool.evaluate(&SumEvaluator).unwrap(), Die::from_weights((3..=12).zip(expected)));
    /// ```
    pub fn reroll_to_pool(
        &self,
        rolls: usize,
        mut which: impl FnMut(&O) -> bool,
        max_rerolls: Option<usize>,
        depth: Depth,
        mode: RerollMode,
    ) -> Result<PoolMixture<O>> {
        let Some(max_rerolls) = max_rerolls else {
            return Ok(Pool::new(vec![(self.reroll(which, depth)?, rolls)])?.into());
        };
        let selected: Vec<bool> = self.outcomes().map(&mut which).collect();
        if !selected.contains(&true) {
            return Ok(Pool::new(vec![(self.clone(), rolls)])?.into());
        }
        let (rerollable_die, not_rerollable_die) = infallible(self.split(&selected));
        let depth = match depth {
            Depth::Finite(depth) => depth as usize,
            Depth::Infinite => max_rerolls,
        };
        let pool_composition = reroll_composition(
            &self.denominator(),
            &rerollable_die.denominator(),
            count(rolls)?,
            count(max_rerolls)?,
            depth,
        )?;

        let make_pool = |rerollable: i64,
                         rerolled: i64,
                         rerolled_to_rerollable: i64|
         -> Result<KeepGenerator<O>> {
            let rerolls_ran_out = rerollable - rerolled;
            let not_rerollable = count(rolls)? - rerolls_ran_out - rerolled_to_rerollable;
            let die_pool = |die: &Die<O>, n: i64| -> Result<KeepGenerator<O>> {
                Ok(Pool::new(vec![(die.clone(), dice(n))])?.into())
            };
            let common = die_pool(&rerollable_die, rerolled_to_rerollable)?
                .additive_union(&die_pool(&not_rerollable_die, not_rerollable)?)?;
            let ran_out = dice(rerolls_ran_out);
            let all_rerollable = || die_pool(&rerollable_die, rerollable);
            Ok(match mode {
                RerollMode::Random => {
                    common.additive_union(&die_pool(&rerollable_die, rerolls_ran_out)?)?
                }
                RerollMode::Low => {
                    common.additive_union(&pool(all_rerollable()?).highest(ran_out).into())?
                }
                RerollMode::High => {
                    common.additive_union(&pool(all_rerollable()?).lowest(ran_out).into())?
                }
                RerollMode::Drop => die_pool(&not_rerollable_die, not_rerollable)?,
            })
        };
        // `pool_composition.map_to_pool(make_pool, star=True)`.
        let inners = pool_composition
            .items()
            .iter()
            .map(|((rerollable, rerolled, again), quantity)| {
                Ok((make_pool(*rerollable, *rerolled, *again)?, quantity.clone()))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(PoolMixture::new(inners))
    }
}

/// A count of dice as the `i64` that `n @ die` takes.
fn count(n: usize) -> Result<i64> {
    i64::try_from(n).map_err(|_| Error::Overflow("too many dice in the pool".into()))
}

/// A count computed from the compositions, which are never negative.
fn dice(n: i64) -> usize {
    usize::try_from(n).expect("reroll_to_pool counts are not negative")
}

/// The pool `die.pool(n)` made of a keep generator that is known to be one.
fn pool<O: Outcome>(generator: KeepGenerator<O>) -> Pool<O> {
    match generator {
        KeepGenerator::Pool(pool) => pool,
        KeepGenerator::CompoundKeep(_) => unreachable!("die.pool(n) is a pool"),
    }
}

/// `Pool([die] * count + ...)`: a die put in zero times is left out, outcomes and all.
fn pool_of<O: Outcome>(dice: Vec<(&Die<O>, usize)>) -> Result<Pool<O>> {
    let present = dice
        .into_iter()
        .filter(|&(_, count)| count > 0)
        .map(|(die, count)| (die.clone(), count))
        .collect();
    Pool::new(present)
}

/// `explode_to_pool`'s `count_die = rolls @ Die(single_data)`, where each die explodes up to `depth` times: `(i,
/// 1)` is `i` explosions followed by a roll that does not explode, and `(depth + 1, 0)` the chain that explodes
/// every time. `die_den`, `explode_den` and `not_explode_den` are the denominators of the die and of its two
/// halves after `split`.
fn explode_counts(
    die_den: &Quantity,
    explode_den: &Quantity,
    not_explode_den: &Quantity,
    rolls: usize,
    depth: u32,
) -> Result<Die<(usize, usize)>> {
    let top = (depth as usize)
        .checked_add(1)
        .ok_or_else(|| Error::Unported("a huge explode_to_pool depth".into()))?;
    // Each weight is a product of `top` denominators.
    crate::memory::check_pow(&(explode_den * die_den), top, "explode_to_pool(depth)")?;
    let mut single_data =
        crate::memory::try_with_capacity(top.checked_add(1), "explode_to_pool(depth)")?;
    for i in 0..top {
        let weight = explode_den.pow(i) * die_den.pow(top - 1 - i) * not_explode_den;
        single_data.push(((i, 1), weight));
    }
    single_data.push(((top, 0), explode_den.pow(top)));
    // `Die(single_data)` drops the zero weights (an empty half has denominator 0).
    let single_count_die = infallible(Die::from_dict(single_data));
    // `rolls @ single_count_die`: Python adds one roll at a time (`_sum_all`); `0 @ die` is the zero vector.
    let mut sum = Die::single((0, 0));
    for _ in 0..rolls {
        sum = sum.binary(&single_count_die, |a, b| (a.0 + b.0, a.1 + b.1));
    }
    Ok(sum)
}

/// `reroll_to_pool`'s `pool_composition` for a finite `max_rerolls`: a die of `(rerollable, rerolled,
/// rerolled_to_rerollable)`, multiplied to the denominator `die_den**(rolls + max_rerolls)`. `rerollable_den` is
/// the denominator of the rerollable half of the die.
fn reroll_composition(
    die_den: &Quantity,
    rerollable_den: &Quantity,
    rolls: i64,
    max_rerolls: i64,
    depth: usize,
) -> Result<Die<(i64, i64, i64)>> {
    // `icepool.coin(rerollable_die.denominator(), self.denominator())`, with `False`/`True` as 0/1 (`n @ coin`
    // sums them into ints anyway). `Die(data)` drops a zero side.
    let single_is_rerollable = infallible(Die::from_dict(vec![
        (0i64, die_den - rerollable_den),
        (1, rerollable_den.clone()),
    ]));
    // `n @ single_is_rerollable`.
    let rerolled = |n: i64| single_is_rerollable.matmul(&[(n, Quantity::ONE)]);

    // Advances one step of rerolling if there are enough rerolls left to cover all rerollable dice. Returns the
    // number of dice showing rerollable outcomes and the number of remaining rerolls.
    let step = |&(rerollable, rerolls_left): &(i64, i64), _: usize| -> Result<Mapped<(i64, i64)>> {
        if rerollable == 0 {
            return Ok(Mapped::Outcome((0, 0)));
        }
        if rerolls_left < rerollable {
            return Ok(Mapped::Outcome((rerollable, rerolls_left)));
        }
        let left = rerolls_left - rerollable;
        Ok(Mapped::Die(rerolled(rerollable)?.map(|&n| (n, left))))
    };

    // `initial_state = icepool.tupleize(rolls @ single_is_rerollable, max_rerolls)`.
    let initial_state = rerolled(rolls)?.map(|&n| (n, max_rerolls));
    // `icepool.map(step, initial_state, star=True, repeat=depth - 1)`.
    let mid_pool_composition = match depth {
        0 => return Err(Error::InvalidArgument("repeat cannot be negative.".into())),
        // `Die([first_arg])` of a die is the die.
        1 => initial_state,
        _ => {
            let mut cache = TransitionCache::new(step, vec![Quantity::ONE]);
            map_repeat(&mut cache, &initial_state, Some(depth as u64 - 1))?
        }
    };

    // Performs the final reroll, which might not have enough rerolls to cover all rerollable dice. Returns the
    // number of dice that had a rerollable outcome, the number of dice that were rerolled due to max_rerolls,
    // the number of rerolled dice that landed on a rerollable outcome again.
    // (`mid_pool_composition.map(final_step, star=True)`: one die of tuples per state, mixed by `Die`.)
    let mut final_dice = Vec::with_capacity(mid_pool_composition.len());
    let mut quantities = Vec::with_capacity(mid_pool_composition.len());
    for ((rerollable, rerolls_left), quantity) in mid_pool_composition.items() {
        let (rerollable, rerolls_left) = (*rerollable, *rerolls_left);
        let rerolled_now = rerollable.min(rerolls_left);
        let again = rerolled(rerolled_now)?;
        final_dice.push(DieArg::Die(
            again.map(|&again| (rerollable, rerolled_now, again)),
        ));
        quantities.push(quantity.clone());
    }
    let pool_composition = Die::new(final_dice, quantities)?;

    // `pool_composition.multiply_to_denominator(self.denominator()**(rolls + max_rerolls))`.
    let exponent = usize::try_from(rolls + max_rerolls).expect("counts are not negative");
    crate::memory::check_pow(die_den, exponent, "reroll_to_pool(rolls + max_rerolls)")?;
    let denominator = die_den.pow(exponent);
    let current = pool_composition.denominator();
    if !(&denominator % &current).is_zero() {
        return Err(Error::InvalidArgument(
            "Target denominator is not an integer factor of the current denominator.".into(),
        ));
    }
    let factor = denominator / current;
    let items = pool_composition
        .items()
        .iter()
        .map(|(o, q)| (*o, q * &factor))
        .collect();
    Ok(infallible(Die::from_dict(items)))
}
