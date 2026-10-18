//! The WASM exports under the TypeScript API (js/src/), built with wasm-pack (see NOTES.md for the commands).
//!
//! The typed exports (`WasmDie`, ...) are in `api`; the interim JSON-op bridge the API still uses for the rest is in
//! `facade` (until task C6l). Two more entry points, each behind a feature that no production build (the npm
//! package) turns on:
//! - `run(programJson)` (feature `runner`, test-only, `runner.rs`): one program of the harness's JSON program
//!   language in, its JSON result out. `run-wasm.sh` drives it so every fixture and fuzz run can go through WASM.
//! - `calculate(engineInput)` (feature `khepri`, `khepri.rs`): khepri's engine, the drop-in for `f2f.calculate`.
//!
//! Their unit tests run in every test build (`cfg(test)`), where the dev-dependencies on icepool-core and
//! icepool-harness turn on the khepri features they need.
//!
//! A Rust panic is a trap in WASM (no unwinding): it reaches JS as a `RuntimeError`. The panic hook installed at
//! start-up keeps its message; the TypeScript API (js/src/wasm.ts) then calls `recover()` and throws an
//! `IcepoolError` ("PanicException", as the Python facade raises). Call `run` / `calculate` through wasm.ts's
//! wrappers of the same name (or recover.mjs's, as run-wasm.mjs and test-calculate.mjs do), not raw, to get that
//! recovery. Sizes from user input are
//! checked before allocating (`icepool_core::memory`), so huge inputs are a `MemoryError`, not a trap.

mod api;
mod facade;
#[cfg(any(test, feature = "khepri"))]
mod khepri;
#[cfg(any(test, feature = "runner"))]
mod runner;

pub use api::{
    live_dice, value_binary, value_round, value_truth, value_unary, BinaryOp, DieBuilder, UnaryOp, WasmDie,
};
pub use facade::{die_handle, 
    call, evaluate, from_wire, live_handles, multiset_function, recover, release, wire_result,
};
#[cfg(any(test, feature = "khepri"))]
pub use khepri::calculate;
#[cfg(any(test, feature = "runner"))]
pub use runner::{run, run_line};

use std::cell::Cell;

use wasm_bindgen::prelude::*;

thread_local! {
    /// The message of the last panic, for `recover()`.
    static PANIC: Cell<Option<String>> = const { Cell::new(None) };
}

/// Runs when the module is instantiated: panics record their message (the default hook would print it to a
/// console the caller may not see, and the trap itself only says "unreachable").
#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| {
        PANIC.with(|p| p.set(Some(info.to_string())))
    }));
}

/// The last panic's message, if any, forgotten once taken.
fn take_panic() -> Option<String> {
    PANIC.with(Cell::take)
}

// The JS globals we need, declared directly so wasm-bindgen is the only binding dependency.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Reflect, js_name = get)]
    fn reflect_get(target: &JsValue, key: &JsValue) -> JsValue;
}

/// The instance's shadow stack pointer, the `WebAssembly.Global` `__stack_pointer` (exported by build.rs), or
/// `undefined`. Rust keeps its frames' locals on this stack in linear memory; a trap abandons them without
/// popping them, so the JS facade saves its value before each outermost call and puts it back after a trap.
#[wasm_bindgen]
pub fn stack_pointer() -> JsValue {
    reflect_get(
        &wasm_bindgen::exports(),
        &JsValue::from_str("__stack_pointer"),
    )
}
