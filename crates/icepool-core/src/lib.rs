//! Rust port of [icepool](https://github.com/HighDiceRoller/icepool), pinned at the version in `oracle.lock`.
//!
//! The core is generic over [`Outcome`] (see docs/ANALYSIS.md "the design decisions"); the bindings instantiate it
//! with the dynamic [`Value`], which carries Python's semantics. Module layout mirrors icepool's own files; see
//! NOTES.md.
//!
//! `program` runs the harness's JSON program language (see harness/core/grammar_api.py and docs/semantics.md):
//! it is how every check drives this crate, and later the WASM entry point.

pub mod counts;
pub mod creation_args;
pub mod die;
pub mod error;
pub mod function;
pub mod math;
pub mod outcome;
pub mod program;
pub mod pysort;
pub mod value;

pub use creation_args::DieArg;
pub use die::Die;
pub use error::{Error, Result};
pub use function::d;
pub use outcome::Outcome;
pub use value::Value;

/// Quantities are unbounded non-negative integers: khepri's heavy case reaches 258-bit denominators. dashu's `UBig`
/// stores values up to 128 bits inline, so the common small quantity never allocates.
pub type Quantity = dashu_int::UBig;
