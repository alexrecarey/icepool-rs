//! One JSON program per stdin line in, one JSON result per stdout line out. Used by the harness (harness/core/tool.py).

use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        if line.trim().is_empty() {
            continue;
        }
        let result = match serde_json::from_str(&line) {
            Ok(program) => {
                // A panic is a bug, but it must not take the whole run down: report it as an error result.
                std::panic::catch_unwind(|| icepool_core::program::run(&program)).unwrap_or_else(|_| {
                    serde_json::json!({"ok": false, "error": "unimplemented: panic in icepool-run"})
                })
            }
            Err(e) => {
                serde_json::json!({"ok": false, "error": format!("unimplemented: bad JSON: {e}")})
            }
        };
        writeln!(out, "{result}").expect("stdout");
        out.flush().expect("stdout");
    }
}
