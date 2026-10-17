//! `run(programJson)`, feature `runner` (test-only): one program of the harness's JSON program language in, its
//! JSON result out, exactly what `icepool-run` prints for that line. `run-wasm.sh` drives it so every fixture and
//! fuzz run can go through WASM (the long gate). Not in the npm package.

use serde_json::Value as Json;
use wasm_bindgen::prelude::*;

/// One program (JSON text) in, its result (JSON text) out: the icepool-run protocol for one line.
#[wasm_bindgen]
pub fn run(program_json: &str) -> String {
    run_line(program_json).to_string()
}

/// What icepool-run answers for one input line (minus its panic guard: WASM cannot catch panics).
pub fn run_line(program_json: &str) -> Json {
    match serde_json::from_str(program_json) {
        Ok(program) => icepool_harness::run(&program),
        Err(e) => {
            serde_json::json!({"ok": false, "error": format!("unimplemented: bad JSON: {e}")})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_line_matches_the_program_runner() {
        let p = r#"{"op":"d","sides":6}"#;
        let expected = icepool_harness::run(&serde_json::from_str(p).unwrap());
        assert_eq!(run_line(p), expected);
        assert_eq!(run_line("{nope")["ok"], Json::Bool(false));
    }
}
