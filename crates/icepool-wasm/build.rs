//! Exports the WASM shadow stack pointer (`__stack_pointer`, a mutable global) from the cdylib, so that the JS
//! facade can put it back after a trap: a trap abandons the Rust frames on the shadow stack without popping them
//! (see `recover` in src/facade.rs).
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-cdylib-link-arg=--export=__stack_pointer");
    }
}
