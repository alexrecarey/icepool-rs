//! The WASM (JS) facade over icepool-core, built with wasm-pack (see NOTES.md for the commands).
//!
//! Two entry points:
//! - `run(programJson)`: one program of the harness's JSON program language in, its JSON result out, exactly what
//!   `icepool-run` prints for that line. `run-wasm.sh` drives it so every fixture and fuzz run can go through WASM.
//! - `calculate(engineInput)`: khepri's engine, the drop-in for `f2f.calculate` that khepri's worker calls through
//!   Pyodide today. Same input object, same output shape.
//!
//! A Rust panic is a trap in WASM (no unwinding): it reaches JS as a `RuntimeError`. The panic hook installed at
//! start-up keeps its message; the JS facade (js/core.js) then calls `recover()` and throws an `IcepoolError`
//! ("PanicException", as the Python facade raises). Call `run` / `calculate` through core.js's wrappers of the same
//! name (run-wasm.mjs and test-calculate.mjs do), not raw, to get that recovery. Sizes from user input are
//! checked before allocating (`icepool_core::memory`), so huge inputs are a `MemoryError`, not a trap.

mod facade;

pub use facade::{
    call, evaluate, from_wire, live_handles, multiset_function, recover, release, wire_result,
};

use std::cell::Cell;

use serde_json::Value as Json;
use wasm_bindgen::prelude::*;

thread_local! {
    /// The message of the last panic, for `recover()`.
    static PANIC: Cell<Option<String>> = const { Cell::new(None) };
}

/// Runs when the module is instantiated: panics record their message (the default hook would print it to a
/// console the caller may not see, and the trap itself only says "unreachable").
#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| PANIC.with(|p| p.set(Some(info.to_string())))));
}

/// The last panic's message, if any, forgotten once taken.
fn take_panic() -> Option<String> {
    PANIC.with(Cell::take)
}

// The JS globals we need, declared directly so wasm-bindgen is the only binding dependency.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = JSON, js_name = parse, catch)]
    fn json_parse(s: &str) -> Result<JsValue, JsValue>;
    /// `undefined` (not a string) for values JSON cannot hold, such as `undefined` itself or a function.
    #[wasm_bindgen(js_namespace = JSON, js_name = stringify, catch)]
    fn json_stringify(v: &JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = BigInt)]
    fn big_int(digits: &str) -> JsValue;
    #[wasm_bindgen(js_namespace = Reflect, js_name = set)]
    fn reflect_set(target: &JsValue, key: &JsValue, value: &JsValue) -> bool;
    #[wasm_bindgen(js_namespace = Reflect, js_name = get)]
    fn reflect_get(target: &JsValue, key: &JsValue) -> JsValue;
}

/// The instance's shadow stack pointer, the `WebAssembly.Global` `__stack_pointer` (exported by build.rs), or
/// `undefined`. Rust keeps its frames' locals on this stack in linear memory; a trap abandons them without
/// popping them, so the JS facade saves its value before each outermost call and puts it back after a trap.
#[wasm_bindgen]
pub fn stack_pointer() -> JsValue {
    reflect_get(&wasm_bindgen::exports(), &JsValue::from_str("__stack_pointer"))
}

/// One program (JSON text) in, its result (JSON text) out: the icepool-run protocol for one line.
#[wasm_bindgen]
pub fn run(program_json: &str) -> String {
    run_line(program_json).to_string()
}

/// What icepool-run answers for one input line (minus its panic guard: WASM cannot catch panics).
pub fn run_line(program_json: &str) -> Json {
    match serde_json::from_str(program_json) {
        Ok(program) => icepool_core::program::run(&program),
        Err(e) => {
            serde_json::json!({"ok": false, "error": format!("unimplemented: bad JSON: {e}")})
        }
    }
}

/// khepri's `f2f.calculate`: the engine input object in,
/// `{rolls, outcomes: [{player, wounds, chance}, ...]}` out. Throws an `Error` where Python raises.
///
/// `rolls` is a Number when it is a safe integer and a BigInt beyond 2**53, as Pyodide's `to_js` converts a Python
/// int. `chance` is `float(Fraction(q, denominator))`, bit for bit.
#[wasm_bindgen]
pub fn calculate(engine_input: &JsValue) -> Result<JsValue, JsError> {
    let text = json_stringify(engine_input)
        .ok()
        .and_then(|s| s.as_string())
        .ok_or_else(|| JsError::new("TypeError: the engine input is not a JSON object"))?;
    let input: Json = serde_json::from_str(&text).map_err(|e| JsError::new(&e.to_string()))?;
    let out =
        icepool_core::khepri::calculate_json(&input).map_err(|e| JsError::new(&e.to_string()))?;
    let js =
        json_parse(&out.to_string()).map_err(|_| JsError::new("calculate: result is not JSON"))?;
    let rolls = &out["rolls"];
    if !is_safe_integer(rolls) {
        // JSON.parse rounded it to the nearest double: replace it with the exact value.
        reflect_set(
            &js,
            &JsValue::from_str("rolls"),
            &big_int(&rolls.to_string()),
        );
    }
    Ok(js)
}

/// Does this JSON integer fit a JS Number exactly (`Number.isSafeInteger`, |n| < 2**53)?
fn is_safe_integer(n: &Json) -> bool {
    const MAX_SAFE: u64 = (1 << 53) - 1;
    n.as_u64().is_some_and(|n| n <= MAX_SAFE)
        || n.as_i64().is_some_and(|n| n.unsigned_abs() <= MAX_SAFE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_integer_bound() {
        let n = |s: &str| serde_json::from_str::<Json>(s).unwrap();
        assert!(is_safe_integer(&n("9007199254740991")));
        assert!(!is_safe_integer(&n("9007199254740992")));
        assert!(!is_safe_integer(&n("1267650600228229401496703205376")));
        assert!(is_safe_integer(&n("-9007199254740991")));
    }

    #[test]
    fn run_line_matches_the_program_runner() {
        let p = r#"{"op":"d","sides":6}"#;
        let expected = icepool_core::program::run(&serde_json::from_str(p).unwrap());
        assert_eq!(run_line(p), expected);
        assert_eq!(run_line("{nope")["ok"], Json::Bool(false));
    }
}
