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
