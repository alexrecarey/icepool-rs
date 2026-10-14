//! The JSON program runner: one program in, one exact result out (wire format: harness/core/encode.py).

use serde_json::{json, Value};

/// Run one program. Never panics on bad input: errors are results too (`{"ok": false, "error": ...}`).
pub fn run(program: &Value) -> Value {
    match program.get("op").and_then(Value::as_str) {
        // Every op starts out unimplemented. The harness never counts "unimplemented: ..." as a match.
        Some(op) => json!({"ok": false, "error": format!("unimplemented: {op}")}),
        None => json!({"ok": false, "error": "unimplemented: not a program node"}),
    }
}
