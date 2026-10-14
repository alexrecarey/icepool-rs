//! Port of icepool `population/again.py`: the `Again` expression, which indicates that the die should be rolled
//! again, usually with some operation applied, and its evaluation in `again_depth` mode.
//!
//! In Python, `Again` is an `AgainExpression` whose operators build bigger expressions (`6 + Again`,
//! `Again @ d6`, ...); `Die(...)` then replaces each `Again` with the die rolled again. Here an expression is a
//! closed enum over two type parameters, so the core stays static: `F` is the operation (the facade's operator
//! enum, playing the role of Python's `operator.add` & co.) and `L` the facade's object type (an outcome, a die,
//! `Reroll`). The facade supplies how to apply an operation and how to build a die.
//!
//! Only depth mode (`evaluate_agains_using_depth`) is ported; count mode (`again_count`, which needs the
//! `is_additive` flag and `map(repeat=...)`) is a later task.

use std::sync::Arc;

/// An expression indicating that the die should be rolled again (`AgainExpression`).
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum Again<F, L> {
    /// The `Again` symbol itself: represents the again roll directly.
    Again,
    /// `function(*args)`, where any args that are themselves `Again` expressions resolve first. These are
    /// considered to be at the same level for purposes of `again_depth`.
    Apply(F, Vec<AgainArg<F, L>>),
}

/// One argument of an `Again::Apply`: another expression, or any other object, passed as-is.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum AgainArg<F, L> {
    Again(Arc<Again<F, L>>),
    Other(L),
}

impl<F, L: Clone> Again<F, L> {
    /// `_evaluate`: recursively replaces the `Again` placeholders with the provided die (or other object), applying
    /// each function to its evaluated arguments, innermost first and arguments left to right, as Python does.
    pub fn evaluate<E>(
        &self,
        die: &L,
        apply: &impl Fn(&F, Vec<L>) -> Result<L, E>,
    ) -> Result<L, E> {
        match self {
            Again::Again => Ok(die.clone()),
            Again::Apply(f, args) => {
                let args = args
                    .iter()
                    .map(|arg| match arg {
                        AgainArg::Again(a) => a.evaluate(die, apply),
                        AgainArg::Other(x) => Ok(x.clone()),
                    })
                    .collect::<Result<_, _>>()?;
                apply(f, args)
            }
        }
    }
}

/// `evaluate_agains_using_depth`: the die for outcomes (one per element of the facade's itemized `Die(outcomes,
/// times)` arguments) some of which are `Again` expressions, rolling again at most `again_depth` times.
///
/// The innermost level replaces every `Again` expression using `again_end`: `None` means `Reroll` (the whole
/// expression becomes `reroll`, so those outcomes drop out), `Some(end)` evaluates the expression with `end` in
/// place of `Again` (the facade resolves Python's default `None` to the zero outcome first). Each further level
/// replaces `Again` with the die built by the level below: `Die(outcomes, times, again_depth=0, again_end=tail)`.
///
/// `die` builds `Die(objects, times)` from one object per outcome; `apply` applies one operation.
pub fn evaluate_agains_using_depth<F, L: Clone, E>(
    outcomes: &[AgainArg<F, L>],
    again_depth: u64,
    again_end: Option<&L>,
    reroll: &L,
    die: impl Fn(Vec<L>) -> Result<L, E>,
    apply: impl Fn(&F, Vec<L>) -> Result<L, E>,
) -> Result<L, E> {
    let replace = |end: Option<&L>| -> Result<Vec<L>, E> {
        outcomes
            .iter()
            .map(|outcome| match (outcome, end) {
                (AgainArg::Again(a), Some(end)) => a.evaluate(end, &apply),
                (AgainArg::Again(_), None) => Ok(reroll.clone()),
                // A tuple or simple arg that is not Again.
                (AgainArg::Other(x), _) => Ok(x.clone()),
            })
            .collect()
    };
    let mut tail = die(replace(again_end)?)?;
    for _ in 0..again_depth {
        tail = die(replace(Some(&tail))?)?;
    }
    Ok(tail)
}
