//! `Die.map`, `reroll` and `filter` with Rust functions (population/die.py; map_tools/core_impl.py `map_simple`
//! for one die), generic over the outcome type.
//!
//! Python's `map` also takes several dice at once (their joint outcomes), a mapping instead of a function, `star`
//! and `**kwargs`. That argument handling is the `Value` layer's: it calls its function per joint outcome, turns
//! each result into a [`Mapped`] and builds the die with `Die::new` ([`Mapped::into_die_arg`]), as `map_simple`
//! builds `Die(final_states, final_quantities)`. Repeated maps (`repeat=`) are `map_tools`; `Again` is `again`.

use crate::creation_args::DieArg;
use crate::die::{Depth, Die};
use crate::error::{Error, Result};
use crate::outcome::OutcomeOrder;

/// What a mapping function returns for one outcome: Python's outcome, `Die`, `Reroll`, `Break` or `Restart`.
///
/// ```
/// use icepool_core::{d, Die, Mapped};
/// // `d(6).map(lambda x: icepool.Reroll if x == 1 else d(x))`
/// let mixed = d(6).try_map(|&x| Ok::<_, icepool_core::Error>(match x {
///     1 => Mapped::Reroll,
///     x => Mapped::Die(d(x)),
/// }));
/// assert_eq!(mixed.unwrap().denominator().to_string(), "300");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mapped<P> {
    /// A plain outcome.
    Outcome(P),
    /// A die, mixed into the result with its own probabilities (scaled by `weighted_lcm`).
    Die(Die<P>),
    /// `icepool.Reroll`: the outcome drops out, as if rerolled until something else comes up.
    Reroll,
    /// `Break(target)`: under `repeat`, the process stops at `target`; `None` (Python's `Break()`) stops at the
    /// current state. A single map unwraps it.
    Break(Option<Target<P>>),
    /// `icepool.Restart`: under `repeat`, the whole process starts over (conditioning against this sequence of
    /// transitions). A single map drops it, as `Die()` drops `Reroll`.
    Restart,
}

/// What a [`Mapped::Break`] stops at.
///
/// ```
/// use icepool_core::{Mapped, Target};
/// // `Break(5)`, `Break(d(4))`, `Break(Reroll)`; going on to a target is the mapped result it names.
/// assert_eq!(Mapped::from(Target::Outcome(5)), Mapped::Outcome(5));
/// assert_eq!(Mapped::<i64>::from(Target::Reroll), Mapped::Reroll);
/// let stop: Mapped<i64> = Mapped::Break(Some(Target::Die(icepool_core::d(4))));
/// assert!(matches!(stop, Mapped::Break(Some(Target::Die(_)))));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target<P> {
    /// An outcome.
    Outcome(P),
    /// A die of outcomes, each of which stops the process.
    Die(Die<P>),
    /// `Break(Reroll)`. A single map drops it; under `repeat` Python fails on it (see DIVERGENCES.md: a tuple
    /// holding `Reroll`).
    Reroll,
}

impl<P> From<Target<P>> for Mapped<P> {
    /// Going on to the target: an outcome, a die, or `Reroll`.
    fn from(target: Target<P>) -> Self {
        match target {
            Target::Outcome(p) => Mapped::Outcome(p),
            Target::Die(d) => Mapped::Die(d),
            Target::Reroll => Mapped::Reroll,
        }
    }
}

impl<P> Mapped<P> {
    /// What `Die(...)` makes of this result in a single map (`map_simple`): a `Break` is unwrapped, `Break()`
    /// being `current` (the outcome mapped), and `Restart` drops out like `Reroll`.
    ///
    /// ```
    /// use icepool_core::value::DieArg;
    /// use icepool_core::{Mapped, Target};
    /// // A single map unwraps `Break`: `Break()` is the outcome being mapped, here 3.
    /// assert!(matches!(Mapped::<i64>::Break(None).into_die_arg(|| 3), DieArg::Outcome(3)));
    /// assert!(matches!(Mapped::Break(Some(Target::Outcome(7))).into_die_arg(|| 3), DieArg::Outcome(7)));
    /// // `Restart` drops out, as `Reroll` does.
    /// assert!(matches!(Mapped::<i64>::Restart.into_die_arg(|| 3), DieArg::Reroll));
    /// ```
    pub fn into_die_arg(self, current: impl FnOnce() -> P) -> DieArg<P> {
        match self {
            Mapped::Outcome(p) | Mapped::Break(Some(Target::Outcome(p))) => DieArg::Outcome(p),
            Mapped::Break(None) => DieArg::Outcome(current()),
            Mapped::Die(d) | Mapped::Break(Some(Target::Die(d))) => DieArg::Die(d),
            Mapped::Reroll | Mapped::Restart | Mapped::Break(Some(Target::Reroll)) => {
                DieArg::Reroll
            }
        }
    }
}

impl<O: OutcomeOrder> Die<O> {
    /// `die.map(f)` for an `f` that may return anything Python's may ([`Mapped`]) and may fail: the die of `f`'s
    /// results, each with its outcome's quantity (zero quantities included), dice mixed in with `weighted_lcm`
    /// so that the overall denominator is minimised while preserving the relative weighting, and `Reroll`s
    /// dropping out. `Break()` stands for the outcome itself, hence `O: Into<P>` (always true for `P = O`).
    ///
    /// The first error `f` returns is returned.
    ///
    /// ```
    /// use icepool_core::{d, Die, Error, Mapped};
    /// // `d(4).map(lambda x: icepool.Reroll if x == 4 else x + 10)`
    /// let shifted = d(4).try_map(|&x| Ok::<_, Error>(if x == 4 { Mapped::Reroll } else { Mapped::Outcome(x + 10) }));
    /// assert_eq!(shifted.unwrap(), Die::from(11..=13));
    /// ```
    pub fn try_map<P, E>(
        &self,
        mut f: impl FnMut(&O) -> std::result::Result<Mapped<P>, E>,
    ) -> std::result::Result<Die<P>, E>
    where
        P: OutcomeOrder,
        O: Into<P>,
        E: From<Error>,
    {
        let mut results = Vec::with_capacity(self.len());
        let mut quantities = Vec::with_capacity(self.len());
        for (outcome, quantity) in self.items() {
            results.push(f(outcome)?.into_die_arg(|| outcome.clone().into()));
            quantities.push(quantity.clone());
        }
        Ok(Die::new(results, quantities)?)
    }

    /// `die.map(f)` for an `f` that returns a die per outcome: the mixture of those dice, each weighted by its
    /// outcome's quantity (`weighted_lcm`, as `Die()` mixes dice).
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// // `d(2).map(lambda x: d(2 * x))`: d2 half the time, d4 the other half.
    /// let mixed = d(2).map_die(|&x| d(2 * x)).unwrap();
    /// assert_eq!(mixed, Die::from_weights([(1, 3u32), (2, 3), (3, 1), (4, 1)]));
    /// ```
    pub fn map_die<P: OutcomeOrder>(&self, mut f: impl FnMut(&O) -> Die<P>) -> Result<Die<P>> {
        let (dice, quantities) = self
            .items()
            .iter()
            .map(|(outcome, quantity)| (DieArg::Die(f(outcome)), quantity.clone()))
            .unzip();
        Die::new(dice, quantities)
    }

    /// Rerolls the outcomes for which `which` is true, at most `depth` times (Python's `die.reroll(which,
    /// depth=depth)` with a function). With [`Depth::Infinite`] they drop out; if every outcome would be
    /// rerolled, the result has no outcomes. A finite depth that rerolls every outcome raises
    /// `ZeroDivisionError` in Python, an [`Error::DivisionByZero`] here.
    ///
    /// ```
    /// use icepool_core::{d, Depth, Die};
    /// // `d(6).reroll(lambda x: x <= 2, depth=1)`: a 1 or 2 is rerolled once and may come up again.
    /// let once = Die::from_weights([(1, 2u32), (2, 2), (3, 8), (4, 8), (5, 8), (6, 8)]);
    /// assert_eq!(d(6).reroll(|&x| x <= 2, Depth::Finite(1)).unwrap(), once);
    /// ```
    pub fn reroll(&self, mut which: impl FnMut(&O) -> bool, depth: Depth) -> Result<Self> {
        let selected: Vec<bool> = self.outcomes().map(&mut which).collect();
        self.reroll_where(&selected, depth)
    }

    /// Rerolls the outcomes for which `which` is false, at most `depth` times (Python's `die.filter(which,
    /// depth=depth)`): the opposite selection of [`Die::reroll`].
    ///
    /// ```
    /// use icepool_core::{d, Depth, Die};
    /// // `d(6).filter(lambda x: x % 2 == 0)`
    /// assert_eq!(d(6).filter(|&x| x % 2 == 0, Depth::Infinite).unwrap(), Die::from_iter([2, 4, 6]));
    /// ```
    pub fn filter(&self, mut which: impl FnMut(&O) -> bool, depth: Depth) -> Result<Self> {
        self.reroll(|outcome| !which(outcome), depth)
    }
}

#[cfg(test)]
mod tests {
    //! Typed maps on `Die<i64>`; the expected dice are Python icepool's answers, the Python in the comment above.

    use super::*;
    use crate::d;

    type Result<T> = std::result::Result<T, Error>;

    #[test]
    fn try_map_mixes_dice_and_drops_rerolls() {
        // `d(3).map(lambda x: icepool.Reroll if x == 1 else (d(2) if x == 2 else x))`
        let mapped: Result<Die<i64>> = d(3).try_map(|&x| {
            Ok(match x {
                1 => Mapped::Reroll,
                2 => Mapped::Die(d(2)),
                x => Mapped::Outcome(x),
            })
        });
        assert_eq!(
            mapped.unwrap(),
            Die::from_weights([(1, 1u32), (2, 1), (3, 2)])
        );
    }

    #[test]
    fn try_map_unwraps_break_and_drops_restart() {
        // `d(4).map(lambda x: icepool.Break() if x == 1 else icepool.Break(9) if x == 2 else icepool.Restart if
        // x == 3 else x)`
        let mapped: Result<Die<i64>> = d(4).try_map(|&x| {
            Ok(match x {
                1 => Mapped::Break(None),
                2 => Mapped::Break(Some(Target::Outcome(9))),
                3 => Mapped::Restart,
                x => Mapped::Outcome(x),
            })
        });
        assert_eq!(mapped.unwrap(), Die::from_iter([1, 4, 9]));
    }

    #[test]
    fn try_map_changes_the_outcome_type_and_returns_the_first_error() {
        // `d(6).map(lambda x: x * 10)`, from `i32` outcomes to `i64` ones (`Into`).
        let mapped: Result<Die<i64>> =
            Die::<i32>::from(1..=6).try_map(|&x| Ok(Mapped::Outcome(i64::from(x) * 10)));
        assert_eq!(mapped.unwrap(), Die::from_iter([10, 20, 30, 40, 50, 60]));
        let mut calls = 0;
        let failed: Result<Die<i64>> = d(6).try_map(|&x| {
            calls += 1;
            if x == 2 {
                Err(Error::InvalidArgument("two".into()))
            } else {
                Ok(Mapped::Outcome(x))
            }
        });
        assert_eq!(failed, Err(Error::InvalidArgument("two".into())));
        assert_eq!(calls, 2);
    }

    #[test]
    fn map_die_weights_each_die_by_its_outcome() {
        // `d(3).map(lambda x: d(x) if x < 3 else icepool.Die([]))`: the empty die drops out with its outcome.
        let mixed = d(3)
            .map_die(|&x| {
                if x < 3 {
                    d(x)
                } else {
                    Die::from_dict(vec![]).unwrap()
                }
            })
            .unwrap();
        assert_eq!(mixed, Die::from_weights([(1, 3u32), (2, 1)]));
    }

    #[test]
    fn reroll_and_filter_select_with_functions() {
        // `d(6).reroll(lambda x: x == 6, depth='inf')`, `d(6).filter(lambda x: x == 6, depth=1)`
        assert_eq!(
            d(6).reroll(|&x| x == 6, Depth::Infinite).unwrap(),
            Die::from(1..=5)
        );
        let once = Die::from_weights([(1, 5u32), (2, 5), (3, 5), (4, 5), (5, 5), (6, 11)]);
        assert_eq!(d(6).filter(|&x| x == 6, Depth::Finite(1)).unwrap(), once);
        // Rerolling everything a finite number of times divides by zero, as Python does.
        assert!(matches!(
            d(6).reroll(|_| true, Depth::Finite(2)),
            Err(Error::DivisionByZero(_))
        ));
    }
}
