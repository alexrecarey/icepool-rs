# icepool-rs

A Rust port of the Python dice probability library [icepool](https://github.com/HighDiceRoller/icepool), built by
an unattended agent loop. Goal: **full parity** with Python icepool, as a Rust core with a Python facade (PyO3,
same API) and a JS facade (WASM). Background: docs/ANALYSIS.md (what icepool is, design) and docs/LOOP-PLAN.md (how
this loop works).

## The one rule: Python icepool is the oracle

The oracle is upstream icepool pinned in `oracle.lock`, checked out read-only at `$ICEPOOL_ORACLE`
(`~/code/alexrecarey/icepool-oracle`). The port must produce **identical** results: same outcomes, same order, same
types, same integer quantities, and raise where it raises. When Python does something odd or buggy, **copy it**
and log it in DIVERGENCES.md with a drafted upstream test. Never "fix" behaviour in the port.

You never decide whether your own work is correct; the checks do. Read the Python source you are porting before
writing Rust, and port its explanatory comments along with the code.

## Layout

| Path | What | Who may edit |
|---|---|---|
| `crates/icepool-core/` | the Rust core + `program/` (JSON program runner) + `src/bin/icepool-run.rs` | you |
| `crates/icepool-core/tests/fixtures.rs` | runs every fixture natively | **protected** |
| `harness/core/` | gate, guard, comparison, fuzzer, fixture tools | **protected** |
| `harness/grammar/` | program-language nodes (Python side) + `status.json` | you (add nodes; status only pending -> ported) |
| `harness/catalogue/` | named functions/evaluators usable in programs (Python side) | you |
| `harness/upstream_shims/` | shims for upstream tests that read private internals (Phase S) | you, sparingly |
| `fixtures/` | golden results computed by the oracle | **append-only**: add new files, never edit old ones |
| `tasks.json` | the task queue (definitions; status lives in `loop/state.json`) | you may add tasks, never remove one |
| `NOTES.md` | gotchas for future iterations | you: add what the next iteration must know |
| `DIVERGENCES.md` | Python behaviour copied but thought wrong | you |
| `PROGRESS.md`, `loop/`, `baselines/`, `oracle.lock`, this file, `.claude/` | the loop's own | **protected** |

`harness/core/guard.sh` rejects any iteration that touches a protected path, edits or deletes a fixture, turns a
ported node back to pending, or changes a task's status. Then a separate reviewer reads your diff.

## Commands

```bash
harness/core/gate.sh fast                                   # the gate the loop runs after you; must be GREEN
cargo test --release                                        # includes every fixture (tests/fixtures.rs)
uv run python -m harness.core.tool check '<program json>'   # oracle vs port for one program
uv run python -m harness.core.tool oracle '<program json>'  # what Python icepool says
uv run python -m harness.core.tool fuzz --ops d,add --count 5000 --fresh
uv run python -m harness.core.tool fixtures make <area>/<name> programs.jsonl   # oracle computes expectations
uv run python -m harness.core.tool fixtures import-failures <area>/<name>       # minimised fuzz failures -> fixtures
uv run python -m harness.core.tool ops                      # grammar nodes and their status
```

`CARGO_TARGET_DIR` is shared between worktrees (set by the loop). The program language and wire format are
documented in `harness/core/grammar_api.py` and `harness/core/encode.py`; Python semantics the port must copy are in
`docs/semantics.md`.

## How to do a task

1. Read your task in `tasks.json`, the last part of `PROGRESS.md`, `NOTES.md`, and `.loop/last-failure.md` if it
   exists (your previous attempt failed: start from that).
2. Read the Python you are porting in `$ICEPOOL_ORACLE/src/icepool/`.
3. Implement it in Rust. Wire it into `program/` so the grammar nodes run.
4. **Grow the tests in the same task:**
   - add grammar nodes for the feature (Python side: a direct call into icepool, nothing else; a generator that
     keeps programs small);
   - mark them `ported` in `status.json`;
   - add a fixtures file of hand-picked edge cases (`fixtures make`);
   - fuzz until clean, and import every failure you fixed as fixtures (`fixtures import-failures`).
5. Run `harness/core/gate.sh fast` until GREEN. Commit in small, well-described steps.
6. Write what the next iteration needs to know in `NOTES.md`.

**If the task is too big for one iteration:** do a coherent first part, then add the rest as new `todo` tasks in
`tasks.json` (with `deps`). Never leave a ported node half-working: a node is `ported` only when it fuzzes clean.

**If you are stuck on a genuine question about intended behaviour** (rare: the oracle answers almost everything),
add it to `QUESTIONS.md`, work around it, and continue.

## Code rules

- Stable Rust (see `rust-toolchain.toml`). `cargo clippy -D warnings` is part of the gate.
- Allowed dependencies: serde/serde_json, a big-integer crate (dashu or num-bigint/num-rational, your call, note
  why in NOTES.md), rustc-hash, smallvec, and later pyo3 and wasm-bindgen. Ask via QUESTIONS.md before adding others.
- Keep the core generic and static (see ANALYSIS.md "the design decisions"); Python dynamism belongs in the
  facades, never in the core.
- Short code, well commented: explain *why*, and carry over icepool's own explanatory comments.
- No `todo!()`/`unimplemented!()` reachable from a node marked ported. No special-casing of fixture or test inputs.
- Never call Python from the Rust backend. Never edit the oracle checkout.
- Don't push and don't touch other repositories: the loop script merges and pushes.
- The workspace-level `~/AGENTS.md` (UBS scanner, StayNexus) does not apply in this repository.
