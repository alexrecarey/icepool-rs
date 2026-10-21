//! The WASM exports under the TypeScript API (js/src/), built with wasm-pack (see docs/NOTES.md for the commands).
//!
//! The typed exports (`WasmDie`, ...) under the TypeScript API are in `api`. More entry points, behind a feature
//! that no production build (the npm package) turns on:
//! - feature `runner` (test-only), on the harness's program runner (icepool-harness): `run(programJson)`
//!   (`runner.rs`), one program of the harness's JSON program language in, its JSON result out, which
//!   `run-wasm.sh` drives so every fixture and fuzz run can go through WASM; and the JSON-op bridge (`facade.rs`:
//!   `call`, `from_wire`, `wire_result`, ...), through which the JS fixture tests reach the nodes the TypeScript
//!   API has no counterpart for.
//!
//! Their unit tests run in every test build (`cfg(test)`), where the dev-dependency on icepool-harness brings in
//! what they need.
//!
//! A Rust panic is a trap in WASM (no unwinding): it reaches JS as a `RuntimeError`. The panic hook installed at
//! start-up keeps its message; the TypeScript API (js/src/wasm.ts) then calls `recover()` and throws an
//! `IcepoolError` ("PanicException", as the Python facade raises). Call `run` through wasm.ts's
//! wrapper of the same name (or recover.mjs's, as run-wasm.mjs and bench.mjs do), not raw, to get that
//! recovery. Sizes from user input are
//! checked before allocating (`icepool_core::value::{check_len, try_repeat}`), so huge inputs are a `MemoryError`,
//! not a trap.

mod api;
#[cfg(any(test, feature = "runner"))]
mod facade;
#[cfg(any(test, feature = "runner"))]
mod runner;

pub use api::{
    live_dice, value_binary, value_round, value_truth, value_unary, BinaryOp, DieBuilder, UnaryOp, WasmDie,
};
#[cfg(any(test, feature = "runner"))]
pub use facade::{call, die_handle, from_wire, live_handles, multiset_function, release, wire_result};
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

/// After a trap (a Rust panic: WASM has no unwinding), once JS has left every call into the instance: resets the
/// core's per-call state, which the trap skipped the destructors of, and returns the panic's message (empty if
/// none was recorded).
#[wasm_bindgen]
pub fn recover() -> String {
    icepool_core::value::recursion::reset();
    // The program runner's own state (the arguments of the multiset function bodies it traces); its handle table is
    // never borrowed across code that can panic, so it survives as it is.
    #[cfg(feature = "runner")]
    icepool_harness::reset_after_trap();
    PANIC.with(Cell::take).unwrap_or_default()
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
