//! Test-only: the JSON program runner that every check drives (harness/core/tool.py, the fixtures test, the fuzzer)
//! and the `icepool-run` binary over it. The program language and its wire format are documented in
//! harness/core/grammar_api.py and harness/core/encode.py.
//!
//! The runner's code is still `icepool_core::program`, compiled only with icepool-core's `runner` feature, which
//! this crate turns on: it moves here once the protected `crates/icepool-core/tests/fixtures.rs` does (task C5b,
//! QUESTIONS.md "D1"). This crate is the runner's home already, so callers name `icepool_harness::run`.

pub use icepool_core::program::run;
