//! Rust port of [icepool](https://github.com/HighDiceRoller/icepool), pinned at the version in `oracle.lock`.
//!
//! The core is generic over [`Outcome`] (see docs/ANALYSIS.md "the design decisions"); the bindings instantiate it
//! with the dynamic [`Value`], which carries Python's semantics. Module layout mirrors icepool's own files; see
//! NOTES.md.
//!
//! `program` runs the harness's JSON program language (see harness/core/grammar_api.py and docs/semantics.md):
//! it is how every check drives this crate, and later the WASM entry point.

pub mod again;
pub mod compound_keep;
pub mod counts;
pub mod creation_args;
pub mod deal;
pub mod deck;
pub mod die;
pub mod error;
pub mod evaluator;
pub mod expand;
pub mod expression;
pub mod format;
pub mod function;
pub mod host_source;
pub mod keep;
pub mod keep_tuple;
pub mod khepri;
pub mod lexi;
pub mod map_tools;
pub mod math;
pub mod multi_deal;
pub mod operator;
pub mod order;
pub mod outcome;
pub mod pool;
pub mod population;
pub mod program;
pub mod pysort;
pub mod random;
pub mod to_pool;
pub mod value;
pub mod weightless;

pub use creation_args::DieArg;
pub use deal::Deal;
pub use deck::Deck;
pub use die::Die;
pub use error::{Error, Result};
pub use function::d;
pub use multi_deal::MultiDeal;
pub use outcome::{Outcome, SumOutcome};
pub use pool::Pool;
pub use value::Value;

/// Quantities are unbounded non-negative integers: khepri's heavy case reaches 258-bit denominators. dashu's `UBig`
/// stores values up to 128 bits inline, so the common small quantity never allocates.
pub type Quantity = dashu_int::UBig;
