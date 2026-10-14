//! The `Outcome` trait: what the generic core needs from an outcome type.
//!
//! icepool requires outcomes to be hashable and totally orderable under Python's `<` (typing.py `Outcome`), and
//! merges outcomes that are `==` (and hash equal), e.g. `True` and `1`. The core is generic over this trait so that
//! Rust users can have a typed `Die<i64>`; both bindings instantiate it with the dynamic [`crate::Value`].

use std::fmt::Debug;
use std::hash::Hash;

use crate::error::Result;

pub trait Outcome: Clone + Debug + Hash + Eq {
    /// Python's `self < other`. Incomparable values (an `int` and a `str`) raise `TypeError`, as in Python.
    ///
    /// `Eq` is Python's `==`: values of different types may be equal (`True == 1 == 1.0`), and `Hash` must agree with
    /// it. (Like Python, `Value`'s float NaN is not equal to itself; see NOTES.md.)
    fn py_lt(&self, other: &Self) -> Result<bool>;

    /// Python's `self <= other` (`Die.clip`). The default suits types whose `<` is a total order.
    fn py_le(&self, other: &Self) -> Result<bool> {
        Ok(self == other || self.py_lt(other)?)
    }

    /// Python's `self > other` (`max()`). The default suits types whose `>` is `<` reflected.
    fn py_gt(&self, other: &Self) -> Result<bool> {
        other.py_lt(self)
    }

    /// Python's `self >= other`.
    fn py_ge(&self, other: &Self) -> Result<bool> {
        other.py_le(self)
    }

    /// `Counts` rejects some keys outright (`None`, `Reroll`) with `TypeError`. Most types accept everything.
    fn check_key(&self) -> Result<()> {
        Ok(())
    }
}

impl Outcome for i64 {
    fn py_lt(&self, other: &Self) -> Result<bool> {
        Ok(self < other)
    }
}

/// The arithmetic that `n @ die` (`Die._sum_all`) needs from an outcome type: `+`, unary `-`, and `x * 0`
/// (`Population.zero`, which zeros an outcome by multiplying it by `0`).
pub trait SumOutcome: Outcome {
    /// Python's `self + other`.
    fn add(&self, other: &Self) -> Result<Self>;
    /// Python's `-self`.
    fn neg(&self) -> Result<Self>;
    /// Python's `self * 0`.
    fn times_zero(&self) -> Result<Self>;
}

impl SumOutcome for i64 {
    fn add(&self, other: &Self) -> Result<Self> {
        self.checked_add(*other)
            .ok_or_else(|| crate::Error::Overflow("i64 addition overflowed".into()))
    }

    fn neg(&self) -> Result<Self> {
        self.checked_neg()
            .ok_or_else(|| crate::Error::Overflow("i64 negation overflowed".into()))
    }

    fn times_zero(&self) -> Result<Self> {
        Ok(0)
    }
}
