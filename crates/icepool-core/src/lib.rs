// The crate's documentation is the usage guide in README.md; its examples run as doctests.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

// Design: the core is generic over `OutcomeOrder` (see docs/ANALYSIS.md "the design decisions" and
// docs/design/library-architecture.md section 1): any `Ord` type is an `Outcome`, whose comparisons cannot fail, so
// `Die<i64>` answers them with plain values; the bindings instantiate the core with the dynamic `Value`, which
// carries Python's semantics (its comparisons can raise). Module layout mirrors icepool's own files; see NOTES.md.

// The public API (docs/design/library-architecture.md section 4): the typed dice at the root, plus the evaluator
// trait with the built-in evaluators, multiset expressions, and `Value`, the dynamic outcome of the bindings.
pub mod evaluator;
pub mod expression;
pub mod host;
pub mod random;
pub mod value;

pub use again::{Again, AgainEnd, AgainOutcome};
pub use deal::Deal;
pub use deck::Deck;
pub use die::{Depth, Die};
pub use die_map::{Mapped, Target};
pub use die_ops::IntoDie;
pub use error::{Error, Result};
pub use format::FormatOutcome;
pub use function::{d, z};
pub use int::Int;
pub use multi_deal::MultiDeal;
pub use order::Order;
pub use outcome::{Outcome, OutcomeArith, OutcomeOrder, ToRatio};
pub use population::Comparison;
pub use pool::Pool;
pub use value::Value;

// Internals of the typed core.
mod deal;
mod deck;
mod die;
mod die_map;
mod die_ops;
mod error;
mod int;
mod lexi;
mod math;
mod multi_deal;
mod order;
mod outcome;
mod pool;
mod statistics;

// Internals the language bindings (icepool-py, icepool-wasm) and the program runner (icepool-harness) still reach
// directly. They are not part of the public API: hidden from the documentation, and to become `pub(crate)` once
// the bindings and the runner go through deliberate APIs in `value` (tasks C3b, C3d).
#[doc(hidden)]
pub mod again;
#[doc(hidden)]
pub mod compound_keep;
#[doc(hidden)]
pub mod counts;
#[doc(hidden)]
pub mod creation_args;
#[doc(hidden)]
pub mod expand;
#[doc(hidden)]
pub mod format;
#[doc(hidden)]
pub mod function;
#[doc(hidden)]
pub mod interrupt;
#[doc(hidden)]
pub mod keep;
#[doc(hidden)]
pub mod keep_tuple;
#[doc(hidden)]
pub mod map_tools;
#[doc(hidden)]
pub mod memory;
#[doc(hidden)]
pub mod operator;
#[doc(hidden)]
pub mod ownership;
#[doc(hidden)]
pub mod population;
#[doc(hidden)]
pub mod weightless;
#[doc(hidden)]
pub use creation_args::DieArg;
#[doc(hidden)]
pub use outcome::infallible;

/// Quantities are unbounded non-negative integers: khepri's heavy case reaches 258-bit denominators. dashu's `UBig`
/// stores values up to 128 bits inline, so the common small quantity never allocates.
pub type Quantity = dashu_int::UBig;

/// Exact fractions: means and probabilities of typed dice (Python's `Fraction`).
pub type Ratio = dashu_ratio::RBig;
