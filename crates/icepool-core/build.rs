//! Rebuild when the checkout moves. The loop shares one `CARGO_TARGET_DIR` between worktrees, and cargo neither
//! hashes a path package's location into its artifact names nor tracks `env!("CARGO_MANIFEST_DIR")`; it checks
//! source mtimes by *relative* path. So a test binary compiled in one worktree (with that worktree's path baked in
//! by `env!`) was reused from another and read the first worktree's fixtures, or failed outright once that
//! worktree was deleted. Tying this crate's fingerprint to the directory cargo runs from (gate.sh runs it from the
//! checkout root) and to this checkout's own path forces a rebuild whenever either changes.
fn main() {
    println!("cargo:rerun-if-env-changed=PWD");
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo:rerun-if-changed={dir}/build.rs");
}
