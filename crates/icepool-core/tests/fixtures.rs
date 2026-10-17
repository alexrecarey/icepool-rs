//! Every golden fixture (fixtures/**/*.jsonl, produced by the Python oracle) through `program::run`. PROTECTED.
//!
//! Same rules as harness/core/compare.py: both raised = match; "unimplemented: ..." never matches; otherwise the
//! results must be equal as JSON values (serde_json keeps bool and number distinct, and with arbitrary_precision
//! compares big integers exactly). Fixtures that use an op not yet marked "ported" in harness/grammar/status.json
//! are skipped. This test is what cargo-mutants runs, so it is the Rust-side oracle.

use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ported() -> HashSet<String> {
    let s = fs::read_to_string(root().join("harness/grammar/status.json")).expect("status.json");
    let v: Value = serde_json::from_str(&s).expect("status.json");
    v.as_object()
        .unwrap()
        .iter()
        .filter(|(_, s)| s.as_str() == Some("ported"))
        .map(|(k, _)| k.clone())
        .collect()
}

fn ops(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(op)) = m.get("op") {
                out.insert(op.clone());
            }
            m.values().for_each(|x| ops(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| ops(x, out)),
        _ => {}
    }
}

fn same(expected: &Value, actual: &Value) -> Result<(), String> {
    let ok = |v: &Value| v.get("ok").and_then(Value::as_bool);
    let (Some(e_ok), Some(a_ok)) = (ok(expected), ok(actual)) else {
        return Err(format!("malformed: {actual}"));
    };
    if !a_ok {
        let err = actual.get("error").and_then(Value::as_str).unwrap_or("");
        if err.starts_with("unimplemented") {
            return Err(format!("port: {err}"));
        }
    }
    if !e_ok || !a_ok {
        return if e_ok == a_ok { Ok(()) } else { Err(format!("expected {expected}, got {actual}")) };
    }
    for key in ["die", "value"] {
        if expected.get(key).is_some() {
            return if expected.get(key) == actual.get(key) {
                Ok(())
            } else {
                let short = |v: Option<&Value>| v.map(|v| v.to_string().chars().take(300).collect::<String>());
                Err(format!("{key}: expected {:?}, got {:?}", short(expected.get(key)), short(actual.get(key))))
            };
        }
    }
    Err("expected result has neither die nor value".into())
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|e| e == "jsonl") {
            out.push(p);
        }
    }
}

#[test]
fn fixtures_match_oracle() {
    let ported = ported();
    let mut paths = Vec::new();
    files(&root().join("fixtures"), &mut paths);
    let (mut checked, mut skipped, mut failures) = (0, 0, Vec::new());
    for path in &paths {
        let text = fs::read_to_string(path).unwrap();
        for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let fx: Value = serde_json::from_str(line).unwrap();
            let mut used = HashSet::new();
            ops(&fx["program"], &mut used);
            if !used.is_subset(&ported) {
                skipped += 1;
                continue;
            }
            checked += 1;
            let got = icepool_core::program::run(&fx["program"]);
            if let Err(why) = same(&fx["expected"], &got) {
                failures.push(format!("{}:{}: {why}", path.display(), i + 1));
            }
        }
    }
    eprintln!("fixtures: {checked} checked, {skipped} skipped (ops not ported), {} failures", failures.len());
    assert!(failures.is_empty(), "{} fixture failures, first ones:\n{}", failures.len(), failures[..failures.len().min(10)].join("\n"));
}
