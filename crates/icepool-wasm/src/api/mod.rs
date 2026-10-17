//! The typed exports under the TypeScript API (js/src/): core objects as wasm-bindgen classes (`WasmDie`, ...)
//! whose methods take and return JS values converted straight to and from the core's `Value` (`convert`), and
//! throw core errors as `IcepoolError`s (`error`). The TypeScript classes do Python's argument juggling (what
//! icepool's own Python does before it reaches the work) and call these for the work.
//!
//! Interim (task C6, see NOTES.md): the TypeScript API is moving onto these exports class by class; what has not
//! moved yet still goes through the JSON-op bridge in `crate::facade`, which converts a `WasmDie` to a handle and
//! back (`facade::die_handle`, `WasmDie::from_handle`).

mod convert;
mod die;
mod error;
mod host;
mod value;

pub use die::{live_dice, BinaryOp, DieBuilder, UnaryOp, WasmDie};
pub use value::{value_binary, value_round, value_truth, value_unary};
pub(crate) use error::{kind as error_kind, python_type, to_js as to_js_error};
