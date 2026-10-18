# Questions for Alex

Genuine questions about intended behaviour that the oracle cannot answer. The loop works around them and carries on.

## V1: LGPL-derived libm ports in an MIT crate (resolved 2026-10-17)

Resolved by B9: Alex accepts Rust's built-in float math, so the glibc ports (`value/trig.rs`, `trig_tables.rs`,
`libm.rs`, `pow.rs`) are deleted and no LGPL-derived code is left. Native builds still call the platform libm;
WASM results are within 2 ULP of the oracle's (DIVERGENCES.md).

## S6c: put the JS suite in the long gate (harness/core/gate.sh is the loop's)

The JS facade's suite, `crates/icepool-wasm/js/build.sh --test`, runs every fixture through the JS API (the
idiomatic methods, JS callbacks and evaluators, JS Maps as dicts), the API tests and a `tsc` type check. It is GREEN
at S6c (all fixtures, 0 failing; ~40 s with a warm cargo cache) but no gate runs it, so a core change can break the
JS facade unnoticed. Please add it to the long gate, next to "fixtures through wasm" (it needs `wasm-pack` and
`node` >= 22; `tsc` is optional, the script skips it when missing):

```bash
  if command -v wasm-pack > /dev/null && command -v node > /dev/null; then
    step "JS suite" crates/icepool-wasm/js/build.sh --test
  fi
```
- S6c question: done 2026-10-16, JS suite added to the long gate (node 22 in ~/.local/node22).

## B9: compare.py's ULP tolerance should cover complex values (done 2026-10-17: complex repr leaves compared part by part)

With the glibc ports gone, 7 complex powers in `fixtures/v1/complex-edge-cases.jsonl` (lines 2, 5, 13, 15, 63, 76,
79) come out of WASM 1-2 ULP off in a part, e.g. `(0.6236155388561803-0.6236155388561802j)` for Python's
`...803j)`. A complex has no wire type: results encode it as `{"repr": "complex: (a+bj)"}`, which `compare.py`
compares as text even with `ICEPOOL_FLOAT_ULPS=2`, so `gate.sh long`'s "fixtures through wasm" step fails on them
(`ICEPOOL_FLOAT_ULPS=2 ICEPOOL_RUN=crates/icepool-wasm/run-wasm.sh harness/core/tool fixtures verify --oracle-sample 0`:
11555 checked, these 7 failures). Its "fuzz through wasm" step will too: `fuzz --ops pow,truediv,entropy,
standardized_moment,die,d,weights,die_typed --seed 21 --count 2000` through WASM gives 5 failures, all complex
results of `standardized_moment` with a fractional k, each 1 ULP off in one part (no plain float fails).
compare.py is protected, so: please extend `_eq` to compare two complex repr
leaves part by part, e.g.

```python
_PART = r"(?:inf|nan|[\d.]+(?:e[+-]\d+)?)"
_COMPLEX = re.compile(rf"^complex: (?:\((-?{_PART})([+-]{_PART})j\)|(-?{_PART})j)$")

def _complex_parts(r):
    m = isinstance(r, str) and _COMPLEX.match(r)
    return m and ((m[1], m[2]) if m[3] is None else ("0", m[3]))

# in _eq, next to the {"float"} case:
        if set(a) == {"repr"} and set(b) == {"repr"} and a != b:
            pa, pb = _complex_parts(a["repr"]), _complex_parts(b["repr"])
            return bool(pa and pb) and all(_float_close(x, y) for x, y in zip(pa, pb))
```

The JS fixture suite does exactly this already (`crates/icepool-wasm/js/test/fixtures.mjs` `complexParts`) and passes.

## D1: two protected-path moves the library design needs (done 2026-10-17: design doc in docs/design/, fixtures.rs in crates/icepool-harness/tests/, guard and mutants updated, C5b requeued)

1. **The design document.** D1 asks for `docs/design/library-architecture.md`, but `docs/` is protected, so it is
   at `docs/design/library-architecture.md` (as FACADE-AUDIT.md went to the root). Please `git mv` it to
   `docs/design/library-architecture.md`; tasks.json's C details already say "to move to docs/design/".
2. **The fixtures test** (needed by C5b, not before). The JSON program runner leaves icepool-core (design section 5),
   but the protected `crates/icepool-core/tests/fixtures.rs` calls `icepool_core::program::run`. Until it moves, C5
   keeps the runner in core behind a non-default `runner` feature that the new `icepool-harness` crate turns on, so
   `cargo test --workspace` still compiles and runs it. Please, once C5 is merged: move it to
   `crates/icepool-harness/tests/fixtures.rs` with one change, `icepool_core::program::run` ->
   `icepool_harness::run` (C5 makes that function exist), keep it in guard.sh's protected list under its new path,
   and update its `root()` (`../..` stays right: the crate is also two levels down). C5b then moves `program/`.

   **C5 status (2026-10-17): ready to move.** `crates/icepool-harness` exists with `pub use ... run` and a
   build.rs identical to icepool-core's (the `env!("CARGO_MANIFEST_DIR")` rebuild guard), so the move is exactly:

   ```bash
   git mv crates/icepool-core/tests/fixtures.rs crates/icepool-harness/tests/fixtures.rs
   sed -i 's/icepool_core::program::run/icepool_harness::run/' crates/icepool-harness/tests/fixtures.rs
   # guard.sh: protect crates/icepool-harness/tests/fixtures.rs instead of the old path
   ```

   (`serde_json` is already a normal dependency of icepool-harness; no Cargo.toml change is needed.) One knock-on
   in the protected gate.sh: its long-gate mutants step runs `cargo mutants ... -p icepool-core`, which tests only
   icepool-core's package; once the fixtures test lives in icepool-harness, mutants in core would no longer be
   checked against the fixtures. Please add `--test-package icepool-core,icepool-harness` (or
   `--test-workspace true`) to that line when you move the file. Until then core's dev-dependency on
   icepool-harness keeps `cargo test -p icepool-core` running every fixture.

   **C5b status (2026-10-17): waiting on this move.** C5b was scheduled twice with fixtures.rs still in
   icepool-core, so both attempts could only record that they are blocked. Please hold C5b until the move is done.

## L202610172222: the long gate's "wasm build" step no longer warms run-wasm.sh's build (gate.sh is the loop's)

Since C5 (2026-10-17 20:34) icepool-core's `khepri` engine is behind a feature. The long gate's "wasm build" step,
`cargo build --release -p icepool-wasm --target wasm32-unknown-unknown`, builds icepool-core for wasm32 with
`runner` only (icepool-wasm's default), but run-wasm.sh builds `--features runner,khepri`, which compiles
icepool-core again: ~25 s here (`touch crates/icepool-core/src/lib.rs; cargo build --lib --release --target
wasm32-unknown-unknown -p icepool-wasm --features runner,khepri`). The harness starts `$ICEPOOL_RUN` lazily on the
first program and gives it 20 s, so whenever the node package is stale (after any merge that touches core) the
first fixture of "fixtures through wasm" times out (long-20261017-2209: `s1b/die-methods-edge-cases.jsonl:1`, a
plain `d6.zero()`; it passes once built). Earlier long gates only passed because some worktree had happened to
build the package after main's files were written. Making `runner`/`khepri` default would put test code in a plain
`wasm-pack build`, so the fix belongs in the gate. Please change the step to build what run-wasm.sh builds:

```bash
  step "wasm build" cargo build --release -p icepool-wasm --target wasm32-unknown-unknown --features runner,khepri --quiet
```

(check-production.sh already builds the npm package's feature set just before, so nothing else loses its warm
cache.) With that, run-wasm.sh's own build is wasm-bindgen only (~1 s), as its header comment says.

**Still red (long-20261018-0454, task L202610180503):** the same `s1b/die-methods-edge-cases.jsonl:1` timeout.
Measured on wasm32 release with `--features runner,khepri`, core already built: touching only icepool-wasm's
sources costs 12 s, touching icepool-harness's 25 s, so nothing short of the gate building run-wasm.sh's feature set
gets a stale rebuild under 20 s (C2f, khepri as its own crate, would stop core recompiling but not these). The
one-line gate change above is still the fix.

## L202610172222: complex `**` under WASM can be more than 2 ULP off (seed 21 of "fuzz through wasm")

`complex(-0.00024988505290350285, -0.5) ** complex(-5.0, 2.5)` is `(1605.4982660299447-258.3902050878144j)` in
Python and natively, `(1605.4982660299454-258.3902050878131j)` through WASM: 3 ULP off in the real part, 23 in the
imaginary. Not a port bug: CPython's `_Py_c_pow` computes `phase = atan2(b, a) * br + bi * log(hypot(a, b))` and
returns `len * cos(phase)`, `len * sin(phase)`. Rust's libm (what WASM gets) is 1 ULP off glibc in `hypot`, `pow`,
`atan2`, `exp` and `log` for these inputs, and a 1-ULP change in `phase` (6.12) moves `len * sin(phase)` by about
`len * cos(phase) * ulp(6.12)` = 1.4e-12 = 23 ULP of 258. No order of operations avoids it: only bit-exact
transcendentals would, and B9 decided against porting libm again. The long gate's "fuzz through wasm" (seed 21,
3000 programs) generates exactly this program, so that step stays red until one of:

1. a looser tolerance for complex parts (e.g. relative error ~1e-14 of the complex's magnitude rather than ULPs of
   each part) in compare.py and the JS suite's `complexParts`;
2. accepting it as a known failure of the long gate (no fixture added: natively it already matches);
3. bringing back exact libm ports for wasm32 only.

The divergence is recorded in DIVERGENCES.md ("Floats under WASM").

**Still red (long-20261018-0454, task L202610180503):** "fuzz through wasm" seed 21 fails on exactly this program.
compare.py's per-part 2-ULP tolerance (bf3a2f3) covers 1-ULP complex differences but not this 3/23-ULP one.

## C5b: the design doc's section 5 and 9.5 after the runner moved (docs/ is protected)

C5b moved `program/` into crates/icepool-harness and dropped icepool-core's `runner` feature. Two things in
docs/design/library-architecture.md are now out of date, and I cannot edit docs/:

1. Section 5's "Interim, until the protected `crates/icepool-core/tests/fixtures.rs` moves" paragraph and the
   `icepool-core/runner` entry under "Feature flags" describe the state before C5b; section 9.5 item 5 says
   check-production.sh keys INTERIM on `icepool_core::program` (it now keys on facade.rs using `icepool_harness`).
2. Section 5 says icepool-wasm's `runner` feature becomes "an optional dependency on icepool-harness after C5b".
   That assumed C6 had taken the JS API off the runner, but C6 was split and the TypeScript API's bridge
   (src/facade.rs, until C6g) still calls the runner's nodes in the npm package. So for now icepool-wasm depends on
   icepool-harness **non-optionally**, with `default-features = false` (no khepri engine in the npm package), and
   check-production.sh reports the harness in the npm package as `interim`, exactly as it reported
   `icepool-core/runner` before (the wheel stays strict: icepool-py must not depend on icepool-harness). C6g makes
   the dependency optional, behind `runner`. Please confirm that interim is acceptable, and update the design doc.

The runner still reaches core's `#[doc(hidden)] pub mod`s, as the bindings do; task C3d (after C3b) moves it onto
deliberate APIs.

## C8: MODULE-MAP.md belongs in docs/ (docs/ is protected)

C8 asks for `docs/MODULE-MAP.md`: every upstream module -> the Rust, Python facade, PyO3, TypeScript and WASM files
that port it, with a status and the oracle commit. `docs/` is protected, so it is at the root as `MODULE-MAP.md`
(as FACADE-AUDIT.md went to the root). Please move it:

```bash
git mv MODULE-MAP.md docs/MODULE-MAP.md
sed -i 's|^DEFAULT_MAP = os.path.join(ROOT, "MODULE-MAP.md")|DEFAULT_MAP = os.path.join(ROOT, "docs", "MODULE-MAP.md")|' scripts/upstream_changes.py
sed -i 's|(MODULE-MAP.md)|(docs/MODULE-MAP.md)|g' README.md
sed -i 's|(\.\./MODULE-MAP.md)|(../docs/MODULE-MAP.md)|' python/README.md
```

and drop the paragraph in its header that says it waits for the move. Its links to sibling files (DIVERGENCES.md,
FACADE-AUDIT.md, QUESTIONS.md) are plain names, not links, so nothing else breaks. Once it is in docs/ the guard
protects it, so updating it (each re-pin of the oracle, each new module) becomes the loop owner's job, or the map
should go to an unprotected place instead (see the next entry).

## C8: proposed layout for the loop's files at the repo root

Today the root mixes the library with the loop's machinery and records: `PROGRESS.md`, `tasks.json`,
`QUESTIONS.md`, `NOTES.md`, `FACADE-AUDIT.md`, `harness-reports/`, `harness-out/` (gitignored), `loop/`,
`baselines/`, plus `.loop/` (gitignored scratch). A visitor sees the loop before the library. Proposal, nothing
moved yet:

| Today | Proposed | Why |
|---|---|---|
| `loop/` (scripts, prompts, `state.json`) | `loop/` (unchanged) | the loop's machinery, protected |
| `PROGRESS.md` | `loop/PROGRESS.md` | written only by loop.sh; protected either way, and `loop` is |
| `baselines/` | `loop/baselines/` | written only by loop.sh (upstream-pass.txt); protected, and `loop` is |
| `tasks.json` | `dev/tasks.json` | agents add tasks, so it cannot sit under the protected `loop/` |
| `QUESTIONS.md` | `dev/QUESTIONS.md` | agents append |
| `NOTES.md` | `dev/NOTES.md` | agents write it every iteration; the root `AGENTS.md` keeps pointing at it |
| `harness-reports/` | `dev/reports/` | oracle_coverage.py's output, agent-maintained |
| `harness-out/` (gitignored) | `.loop/out/` | one gitignored scratch directory per worktree instead of two |
| `FACADE-AUDIT.md`, `MODULE-MAP.md` | `docs/` | library documentation (but see below) |
| `DIVERGENCES.md` | stays at the root | user-facing (README, python/DIFFERENCES.md link it) and appended by agents |

The root would then show `README.md`, `AGENTS.md`/`CLAUDE.md` (agents look for them there), `DIVERGENCES.md`, the
Cargo/uv/toolchain files, `oracle.lock`, and the directories `crates/ python/ docs/ scripts/ fixtures/ harness/
loop/ dev/`. (`dev/` is a placeholder name; `work/` or `records/` would do.)

One decision first: `docs/` is protected as a whole, but FACADE-AUDIT.md and MODULE-MAP.md must change whenever the
facade or a module does. Either protect only the loop's own documents (`docs/ANALYSIS.md`, `docs/LOOP-PLAN.md`,
`docs/semantics.md`, `docs/design/`, `docs/reviews/`) and let agents edit the rest of `docs/`, or keep those two in
`dev/` instead.

What each consumer would need to change:

- **harness/core/guard.sh**: the `PROTECTED` list (`PROGRESS.md` and `baselines` disappear from it, being under
  `loop`; `docs` narrowed if decided above); the Python block reads `tasks.json` at both revisions: make it
  `dev/tasks.json` (for the commit that moves it, read the old path at BASE, or the removed-task check sees an
  empty list at BASE and passes vacuously).
- **harness/core/gate.sh**: every `harness-out` in the mutants step -> `.loop/out`.
- **harness/core/upstream_suite.sh**: `harness-out` (output, junit XML, pass list) -> `.loop/out`;
  `baselines/upstream-pass.txt` -> `loop/baselines/upstream-pass.txt`.
- **harness/core/tool.py**: `OUT = os.path.join(ROOT, "harness-out")` and its usage text -> `.loop/out`.
- **loop/loop.sh**: `commit_main`'s `git add -A tasks.json loop/state.json PROGRESS.md baselines`; the merge step's
  `PROGRESS.md` (three places), `$WT/harness-out/upstream-pass.txt` and `baselines/upstream-pass.txt`; the long
  gate's `harness-out/mutants.out/missed.txt` and the task text quoting it; `git clean ... -e harness-out` (no
  longer needed once it is inside the excluded `.loop`).
- **loop/tasks.py**: `TASKS = "tasks.json"` and its docstring.
- **loop/prompts/implement.md** (and review.md if it names them): the paths of PROGRESS.md, NOTES.md, tasks.json.
- **AGENTS.md**: the layout table and "How to do a task" (paths of tasks.json, PROGRESS.md, NOTES.md,
  QUESTIONS.md, DIVERGENCES.md, baselines).
- **Unprotected, an agent can do them in the same change**: `.gitignore` (`harness-out/` -> nothing, `.loop/` is
  there), `harness/tools/oracle_coverage.py`'s default `--out`, README.md's layout table, python/README.md,
  `.github/workflows/ci.yml` (the `harness-out` artifact path), scripts/upstream_changes.py (map path), NOTES.md's
  own references.
- tasks.json details and PROGRESS.md entries name the old paths; leave them, they are history.

## C8: the design doc and AGENTS.md against what exists now (both protected)

docs/design/library-architecture.md, in the sections C8 reads (and a few around them):

1. **Section 5** lists `crates/icepool-khepri/`: it does not exist yet (task C2f); khepri's engine is
   `crates/icepool-core/src/khepri.rs` behind core's non-default `khepri` feature. Its "Interim, until ... fixtures.rs
   moves" paragraph and the `icepool-core/runner` feature are pre-C5b (as the C5b entry above says).
2. **Section 6, "Migration"**: C6g no longer removes the bridge; C6g was split and C6l does it (after C6h-C6k).
3. **Section 7**: true now, except that the JS package's README and guide say `npm install icepool-wasm` and the
   package is not published; python/DIFFERENCES.md exists (written by hand from DIVERGENCES.md, NOTES.md and
   FACADE-AUDIT.md, not generated); the `RUSTDOCFLAGS=-D warnings cargo doc` check now runs in
   `.github/workflows/ci.yml` (it passes today).
4. **Section 8, "What ships"**: the wheel is the extension alone; the facade in the wheel and `ICEPOOL_BACKEND`
   defaulting to `rust` there are task C5c. Check 1 says "without the `runner` feature": now "without `khepri`, and
   no icepool-harness" (with the npm package's interim exception until C6l). Check 3 describes `npm pack --dry-run
   --json`; the script runs `js/build.sh --dev`, `npm pack` and lists the tarball. Check 4's exports list also
   forbids `calculate`.
5. **Section 9.8**: MODULE-MAP is at the root (first entry above), there is now `scripts/upstream_changes.py` and
   `.github/workflows/ci.yml`; the files to split are `crates/icepool-harness/src/ops.rs` and `expression.rs` (the
   runner left core's `program/` in C5b) and `crates/icepool-core/src/value/ops.rs`, split into `ops/`,
   `expression/` and `value/ops/` directories.

**AGENTS.md** (= CLAUDE.md) "Layout" still says the runner is core's `program/` with `src/bin/icepool-run.rs`, and
that `crates/icepool-core/tests/fixtures.rs` is protected; both moved to `crates/icepool-harness/` (the guard
protects `crates/icepool-harness/tests/fixtures.rs`). Its "How to do a task" step 3 says "Wire it into
`program/`": now `crates/icepool-harness/src/`.
