//! The typed exports under the TypeScript API (js/src/): core objects as wasm-bindgen classes (`WasmDie`, ...)
//! whose methods take and return JS values converted straight to and from the core's `Value` (`convert`), and
//! throw core errors as `IcepoolError`s (`error`). The TypeScript classes do Python's argument juggling (what
//! icepool's own Python does before it reaches the work) and call these for the work.
//!
//! The test-only JSON-op bridge (`crate::facade`, feature `runner`) converts these objects to its handles and back
//! (`facade::die_handle`, `WasmDie::from_handle`, ...).

mod again;
mod convert;
mod deck;
mod die;
mod error;
mod evaluate;
mod evaluator;
mod function;
mod host;
mod map_tools;
mod multiset;
mod multiset_function;
mod to_pool;
mod value;

pub use die::{live_dice, BinaryOp, DieBuilder, UnaryOp, WasmDie};
pub use value::{value_binary, value_round, value_truth, value_unary};
// For the bridge (`crate::facade`).
#[cfg(any(test, feature = "runner"))]
pub(crate) use error::{kind as error_kind, python_type, to_js as to_js_error};
#[cfg(any(test, feature = "runner"))]
pub(crate) use {
    deck::{WasmDeck, WasmMultiDeal},
    multiset::{Generator, WasmGenerator},
};
