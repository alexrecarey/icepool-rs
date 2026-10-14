//! Rust port of [icepool](https://github.com/HighDiceRoller/icepool), pinned at the version in `oracle.lock`.
//!
//! `program` runs the harness's JSON program language (see harness/core/grammar_api.py and docs/semantics.md):
//! it is how every check drives this crate, and later the WASM entry point.

pub mod program;
