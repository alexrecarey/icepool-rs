// The crate's documentation is the usage guide in README.md; its examples run as doctests.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

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
pub use function::{d, pointwise_max, pointwise_min, try_d, try_z, z};
pub use int::Int;
pub use multi_deal::MultiDeal;
pub use order::Order;
pub use outcome::{Outcome, OutcomeArith, OutcomeOrder, ToRatio};
pub use population::Comparison;
pub use pool::Pool;
pub use value::Value;

// Internals: everything else is reached through the re-exports above and in `evaluator`, `expression`, `host`
// and `value` (whose binding-only items say so).
mod again;
mod compound_keep;
mod counts;
mod creation_args;
mod deal;
mod deck;
mod die;
mod die_map;
mod die_ops;
mod error;
mod expand;
mod format;
mod function;
mod int;
mod interrupt;
mod keep;
mod keep_tuple;
mod lexi;
mod map_tools;
mod math;
mod memory;
mod multi_deal;
mod operator;
mod order;
mod outcome;
mod ownership;
mod pool;
mod population;
mod statistics;
mod weightless;

/// Quantities are unbounded non-negative integers: khepri's heavy case reaches 258-bit denominators. dashu's `UBig`
/// stores values up to 128 bits inline, so the common small quantity never allocates.
///
/// ```
/// use icepool_core::{d, Quantity};
/// // Python: `(d(6) + d(6)).denominator() == 36`.
/// assert_eq!((d(6) + d(6)).denominator(), Quantity::from(36u8));
/// // Unbounded: `d(6).pool(50)` has 6**50 rolls.
/// assert_eq!(d(6).pool(50).denominator(), Ok(Quantity::from(6u8).pow(50)));
/// ```
pub type Quantity = dashu_int::UBig;

/// Exact fractions: means and probabilities of typed dice (Python's `Fraction`).
///
/// ```
/// use icepool_core::{d, Ratio};
/// // Python: `(d(6) + d(6)).mean() == 7`, `d(6).mean() == Fraction(7, 2)`.
/// assert_eq!((d(6) + d(6)).mean(), Ratio::from(7));
/// assert_eq!(d(6).mean(), Ratio::from_parts(7.into(), 2u8.into()));
/// ```
pub type Ratio = dashu_ratio::RBig;
