# Progress

Appended by loop/loop.sh after every iteration (protected). Newest at the bottom.

- 2026-10-14 bootstrap: harness, gate, guard, loop, grammar v0 (11 nodes, all pending), 1,279 seed fixtures
  (fixtures/v0: 879 dice/arithmetic programs, 400 khepri engine inputs), Rust runner skeleton.
- 2026-10-14 02:48 **K1** done (attempt 1, 1 commits): K1 is finished: the fast gate passes and so does the guard. Everything is in one commit, `f25b030`.  **What I built** (in `crates/icepool-core/src/`, file names follow icepool's own): - **Outcome trait and Python-style values.** The core is generic over an `Outcome` trait. The dynamic `Value` type f
- 2026-10-14 03:08 **K2** done (attempt 1, 3 commits): K2 is finished: the fast gate is GREEN and the guard passes against the merge base. It's in two commits on `task/K2`.  **What I built** - **Operators on outcomes** (`value/ops.rs`): all the binary operators (`+ - * / // % ** << >> & | ^` and `< <= > >=`) and unary ones (`- + ~ abs`, `round` with `nd
- 2026-10-14 03:24 **K3** done (attempt 1, 2 commits): K3 is finished. The fast gate is GREEN and the guard passes against `main`. The work is in two commits on `task/K3`.  **What I built** - **Core (generic):**   - `expand.rs` has `iter_cartesian_product`.   - `DieArg::Reroll` covers `Reroll` dropping out of `Die(...)`.   - New `Die` methods: `reroll` 
- 2026-10-14 03:45 **K4** done (attempt 1, 2 commits): K4 is finished: the fast gate is GREEN, the guard passes against the merge base, and the work is in two commits on `task/K4`.  **What I built** - **`n @ die`:** `Die::matmul` in the core ports `_sum_all` and the `den**(max|n|-|n|)` scaling. Each sum is computed once per call, and only the sums Pytho
