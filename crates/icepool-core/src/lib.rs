// The crate's documentation is the usage guide in README.md; its examples run as doctests.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

// Design: the core is generic over `OutcomeOrder` (see docs/ANALYSIS.md "the design decisions" and
// design/library-architecture.md section 1): any `Ord` type is an `Outcome`, whose comparisons cannot fail, so
// `Die<i64>` answers them with plain values; the bindings instantiate the core with the dynamic `Value`, which
// carries Python's semantics (its comparisons can raise). Module layout mirrors icepool's own files; see NOTES.md.
//
// `program` (feature `runner`, test-only) runs the harness's JSON program language (see
// harness/core/grammar_api.py and docs/semantics.md): it is how every check drives this crate, through
// icepool-harness. It moves there in task C5b.

// The public API (design/library-architecture.md section 4): the typed dice at the root, plus the evaluator
// trait with the built-in evaluators, multiset expressions, and `Value`, the dynamic outcome of the bindings.
pub mod evaluator;
pub mod expression;
pub mod value;

pub use deal::Deal;
pub use deck::Deck;
pub use die::Die;
pub use die_ops::IntoDie;
pub use error::{Error, Result};
pub use function::{d, z};
pub use int::Int;
pub use multi_deal::MultiDeal;
pub use order::Order;
pub use outcome::{Outcome, OutcomeArith, OutcomeOrder};
pub use pool::Pool;
pub use value::Value;

// Internals of the typed core.
mod deal;
mod deck;
mod die;
mod die_ops;
mod error;
mod int;
mod lexi;
mod math;
mod multi_deal;
mod order;
mod outcome;
mod pool;
// Python's seeded `random.Random`: only the runner samples with its own seed (the bindings draw from the host's
// generator and call `Die::sample_at`), so it is compiled with the runner.
#[cfg(feature = "runner")]
mod random;

// Internals the language bindings (icepool-py, icepool-wasm), the program runner and the fixtures test still
// reach directly. They are not part of the public API: hidden from the documentation, and to become `pub(crate)`
// once the bindings go through deliberate APIs in `value` (task C3b) and the runner leaves the crate (C5b).
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
pub mod host_source;
#[doc(hidden)]
pub mod interrupt;
#[doc(hidden)]
pub mod keep;
#[doc(hidden)]
pub mod keep_tuple;
#[cfg(feature = "khepri")]
#[doc(hidden)]
pub mod khepri;
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
#[cfg(feature = "runner")]
#[doc(hidden)]
pub mod program;
#[doc(hidden)]
pub mod to_pool;
#[doc(hidden)]
pub mod weightless;
#[doc(hidden)]
pub use creation_args::DieArg;
#[doc(hidden)]
pub use outcome::infallible;

/// Quantities are unbounded non-negative integers: khepri's heavy case reaches 258-bit denominators. dashu's `UBig`
/// stores values up to 128 bits inline, so the common small quantity never allocates.
pub type Quantity = dashu_int::UBig;
