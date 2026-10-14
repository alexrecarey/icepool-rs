# icepool-rs: a hands-off porting loop

2026-10-14. Builds on [ANALYSIS.md](ANALYSIS.md).

**As built** (differences from the plan below): the loop runs on carapace in herdr workspace `icepool-rs`; the
oracle is a separate read-only checkout (`~/code/alexrecarey/icepool-oracle`, pinned by `oracle.lock`) rather than
a submodule; task status lives in `loop/state.json` (script-owned) while `tasks.json` holds definitions; agents run
with permission checks off (carapace is a sandbox) and without user-level hooks; phone pushes go through Moshi.

Decisions taken:

| Question | Decision |
|---|---|
| Runner | Local Mac, headless: a script runs `claude -p` iterations in a terminal tab, each with a fresh context |
| Repo | New **private** GitHub repo `alexrecarey/icepool-rs`; the loop commits, pushes and merges its own green work |
| Order | khepri core first (ANALYSIS phase 2), then the PyO3 strangler toward full parity |
| Reporting | Push notification on milestones and when it stops |
| Divergence | **Rust matches Python bit for bit.** Python bugs and odd semantics get logged in `DIVERGENCES.md` with a drafted upstream test; you decide later |
| Budget | Run until done; on a usage limit, sleep until reset and continue |
| Toolchain | rustup alongside Homebrew (wasm32 target), maturin, wasm-pack, cargo-mutants, node |
| Upstream | Start now, privately; you open the discussion with the maintainer in parallel |

## 1. The core idea

The agent never decides whether its own work is correct. **Python icepool v2.2.2 is the oracle**, an outer script runs
the checks, and the agent cannot edit the oracle, the checks or the recorded expectations. "Identical" means
*identical*: the same outcomes in the same order with the same integer quantities, zero-quantity entries included,
not just equal probabilities. icepool doesn't reduce fractions and its `equals` is quantity-sensitive, so the
comparison has to be too.

## 2. Repo layout

```
icepool-rs/
  AGENTS.md              rules for every iteration (short; points at docs/)
  docs/                  ANALYSIS.md, semantics.md (Python rules Rust must copy), architecture.md
  TASKS.md               the ordered task queue (status, owner iteration, attempts)
  PROGRESS.md            append-only log the loop keeps for itself (one line per iteration)
  DIVERGENCES.md         Python behaviour we copied but think is wrong, each with a drafted upstream test
  crates/
    icepool-core/        the static Rust core: generic Outcome + host-callback traits, big-int quantities
    icepool-py/          PyO3 extension (maturin) exposing the core to Python
    icepool-wasm/        wasm-bindgen build + JS facade
  python/icepool/        the facade: a copy of upstream 2.2.2 whose modules are switched one by one to call Rust
  oracle/                git submodule: your fork at fix/multiset-bugs = v2.2.2 + the 3 fixes (PROTECTED)
  harness/core/          gate.sh, guard.sh, the comparison function, fixture verifier (PROTECTED)
  harness/grammar/       fuzzer node types: the loop extends these as features land
  harness/catalogue/     named map functions and evaluators, Python + Rust side by side: the loop extends these
  fixtures/              golden JSON, generated only by the oracle (APPEND-ONLY)
  baselines/             upstream test pass list, fuzz grammar coverage, benchmark history
  loop/                  loop.sh, prompts/, logs/
```

`ICEPOOL_BACKEND=python|rust` selects the facade's backend, so the same test command runs both ways.

**The oracle is pinned to the fork, not bare 2.2.2.** The facade carries the three `fix/multiset-bugs` fixes (a
crash, dropped results, a mutation: nothing worth reproducing). So the oracle has to carry them too, or the two
disagree on day one. `DIVERGENCES.md` records that the oracle is fork-pinned and why. Once upstream merges the
fixes, re-pin to the upstream release.

**A program runner lives in the core from the start.** `icepool-core` gets a small interpreter: JSON program in,
exact outcome/quantity table out. It is exposed as a test binary (`icepool-run`) and later as the WASM entry point.
That is how checks 2, 3 and 5 drive Rust in Phase K, before the PyO3 facade exists. It is the same code the JS facade
needs, so it isn't throwaway.

**"Identical"** means:
- the same outcomes in the same order, with the same integer quantities, zero-quantity entries included;
- for error cases, both sides raise (the messages needn't match);
- `Fraction` and float returns compared exactly (floats bit for bit).

## 3. Six ways of comparing new code against old

Every one of these compares against the Python oracle. None compares Rust against itself.

| # | Check | What it catches | Runs |
|---|---|---|---|
| 1 | **Upstream test suite through the facade.** All 1,712 tests with `ICEPOOL_BACKEND=rust`. The pass list may only grow (`baselines/upstream-pass.txt`) | API behaviour, truth-value magic, everything the maintainer thought to test | every iteration (touched modules), full suite on long gate |
| 2 | **Golden fixtures.** JSON `{program, expected}` produced by running the oracle; executed by `cargo test`, by the PyO3 build and by the WASM build in node | regressions; also makes the WASM build testable without Python | every iteration (cargo + py), long gate (wasm) |
| 3 | **Differential fuzzer.** Random programs in a small JSON expression language, run through the oracle and the port, compared exactly. Fixed seeds (regression) plus fresh seeds (discovery). Failures are minimised and saved as fixtures | combinations nobody wrote a test for; edge sizes, negative keeps, mixed dice | every iteration: 2k fixed + 1k fresh; long gate: 100k |
| 4 | **Algebraic properties** (hypothesis, run on both backends). E.g. `highest(n,k) + lowest(n,n-k) == n @ die`; forced ascending and descending order agree; `pool.expand().map(sum) == pool.sum()`; multiset identities | bugs where the oracle and port happen to share a path; shrinking gives minimal cases | every iteration (small), long gate (large) |
| 5 | **khepri sweep.** `test_f2f.py`, then a stratified grid of ~5,000 engine inputs (burst, bonus, SV 0–30, ammo, Continuous, templates, fixed F2F). Phase K runs it two ways: f2f.py on the oracle vs the native Rust FaceToFace (then WASM). Phase S adds a third: f2f.py on the Rust facade. Exact table equality | the case we actually ship | from the khepri-core milestone onward, long gate |
| 6 | **Notebook replay.** Upstream's 39 notebooks re-executed on both backends with cell outputs compared (nbval-style) | real-game workloads: Cortex, BitD, L5R, Neon City, Risk… | long gate, from the strangler phase |

Two more checks guard the tests themselves:

- **Mutation testing** (`cargo-mutants` on changed files, long gate). A surviving mutant means no check noticed a
  deliberate bug. The next iteration's task is to add a fixture or property that kills it. This is how the suite
  grows where it is weak, not just where it is big.
- **Coverage pressure.** The fuzzer's grammar records which oracle branches its programs reach (coverage.py on the
  oracle). Uncovered oracle branches become fuzzer-grammar tasks.

## 4. The fuzzer language (the piece that grows most)

Lambdas can't cross between Python, Rust and JS, so programs are JSON ASTs. Mapping functions come from a **named
catalogue**: `add_const`, `clamp`, `read_roll` (khepri's), `tuple_swap`, `is_even`, `pick_max_of_pair`… Each one is
implemented in Python (for the oracle) and in Rust/JS, ~30 to start. Custom evaluators come from a catalogue too:
`sum`, `count`, `largest_count`, `straights`, `FaceToFace`.

Node types arrive with the features: `die`, `weights`, arithmetic, comparisons, `@`, `map`, `explode`, `reroll`,
`filter`, `pool`/keep tuples, `evaluate`, multiset operators, `deck`/`deal`, `again`, `map(repeat)`… **A task that
ports a feature must also add that feature's node to the grammar**, so new code is fuzzed from the iteration it lands.
Size limits keep each oracle run under ~50 ms.

## 5. The loop

```
loop.sh  (in a Terminal panel tab, under `caffeinate -dims`)
  forever:
    sync main; if the oracle's own tests fail on main  -> notify, STOP (infrastructure broken)
    pick the first `todo` task in TASKS.md whose deps are `done`
      none left at all          -> notify "full parity", STOP
      only blocked ones left    -> notify with the blocked list, STOP
    (milestones such as "khepri core done" notify and carry on)
    git worktree add ../wt/<task> -b task/<id>
    claude -p "$(cat loop/prompts/implement.md)" --model claude-opus-5-5 \
           --permission-mode acceptEdits --max-turns 200 --output-format stream-json > logs/<id>.jsonl
    harness/gate.sh fast            # run by the SCRIPT, not trusted from the agent
    harness/guard.sh                # protected paths untouched, fixtures append-only, pass list not shrunk
    claude -p "$(cat loop/prompts/review.md)"   # a second, fresh agent reviews the diff for cheating
    all green -> rebase-merge to main, push, mark task done, append PROGRESS.md
    red       -> retry with the failure log (max 3 attempts); then mark `blocked`, note why, move on
    every 10 merges (or nightly): harness/gate.sh long; any red -> the next task is "fix the long gate"
    usage limit hit -> sleep until reset (poll every 30 min), resume
```

**One iteration = one task = one fresh context.** The agent learns where things stand from files, not memory:
`AGENTS.md`, `docs/semantics.md`, its task's entry in `TASKS.md`, the last 30 lines of `PROGRESS.md`, and a
`NOTES.md` of gotchas earlier iterations wrote down.

**The implement prompt, in short:**
1. Read the task and the Python source it names.
2. Write the Rust and switch the facade module over.
3. Add the feature's fuzzer node and catalogue entries.
4. Generate new fixtures *with the oracle*.
5. Run `gate.sh fast` until green.
6. Commit in small steps.
7. If Python does something odd: copy it, log it in `DIVERGENCES.md` with a test, keep going.

The agent may split a task in two when it is too big for one iteration, but it may not drop work.

**The review prompt** looks for the things a hands-off loop is prone to:
- weakened or skipped tests;
- hardcoded fixture answers;
- special-casing on test inputs;
- `todo!()` or `unimplemented!()` reachable from ported paths;
- quietly falling back to Python inside the "rust" backend;
- Python comments not carried over (your "keep why comments" rule);
- in `harness/grammar/` and `harness/catalogue/`, which the agent writes on *both* sides: each grammar node's Python
  side must be a direct one-line call into oracle icepool, and each catalogue function must be short enough to check
  by eye. A matching mistake on both sides would otherwise pass every check.

A rejected review sends the iteration back the same way a red gate does.

### Guardrails (enforced by `guard.sh`, not by asking nicely)

- **No edits** to `oracle/`, `harness/core/`, `loop/` or `baselines/` (pass list and fixture manifest only grow, and only
  `gate.sh` writes them).
- **Fixtures are append-only.** A checksum manifest catches edited or deleted fixtures. A random 5% sample of all
  fixtures is re-verified against the oracle on every gate, so a fabricated expectation can't survive.
- **The rust backend may not call Python icepool.** The facade's switched modules are checked with an import guard.
- Task branches only; main moves only through the script's merge step.
- **Permissions:** `--permission-mode acceptEdits` plus a project `.claude/settings.json` allowlist for `cargo`, `uv`, `maturin`, `wasm-pack`, `node`,
  `git` (not push), `python`, file edits inside the worktree, and crates.io/PyPI/npm fetches. The *script* does the
  pushing. No other network.

### When it notifies you

- **Milestone:** khepri core green on all checks; WASM build passing; each 10% of the upstream pass list; full parity.
- **Stopped:** the oracle or harness is broken on main, every remaining task is blocked, or the long gate has been
  red for 3 fixes in a row.
- **Blocked task:** batched into one daily summary, not one notification each.

You don't get notified per iteration; `PROGRESS.md` and `git log` are there when you want to look.

## 6. Task queue (initial shape; the loop refines it)

**Phase K: khepri core.** Each task in this phase ends with all six checks that apply to it green.
1. `Counts`/`Die` over `Value`, big-int quantities, construction and `weighted_lcm` mixing
2. Unary and binary arithmetic and comparisons, with Python's mixed-type rules (bool is int, tuple `+` concatenates)
3. `map` (non-repeat) with mixture flattening; `Reroll`; `reroll`/`filter`/`split`/`clip`
4. `@` and `_sum_all`; `explode` via depth-mode `Again`
5. `Pool` with keep tuples, skip collapse and binomial popping
6. Evaluator driver: backward/forward DP, room memo, order negotiation (matching upstream's heuristic exactly)
7. Built-in evaluators: sum, size, keep-index, plus FaceToFace in Rust
8. khepri sweep green, then the WASM build and a JS `calculate()` matching khepri's worker interface. **Milestone.**

**Phase S: strangler to full parity.** Ordered by module dependency, each switching one facade module:
- the remaining Population methods and statistics
- the full `Again` (count mode)
- `map(repeat)` and the Markov solver, `map_and_time`
- `Deck`, `Deal`, `MultiDeal`
- multiset operators and the generic `MultisetKeep`
- mixtures, tuple expressions, `@multiset_function` (tracing stays in Python, Rust runs the DAG)
- `Vector` and `Symbols`
- formatting
- the JS facade over the full surface

**Running alongside:** test-growth tasks generated by mutants, uncovered oracle branches and minimised fuzz failures.

## 7. Bootstrap: what we do together before it runs unattended (one session)

1. Install rustup (wasm32 target), maturin, wasm-pack, cargo-mutants. Check that node is present.
2. Create the private repo `alexrecarey/icepool-rs` and add the oracle submodule pinned to `fix/multiset-bugs`. Copy
   the facade from the oracle.
   - Set a shared `CARGO_TARGET_DIR`, so worktrees don't each rebuild from scratch.
   - Check the oracle out per worktree.
3. Write by hand, not by the loop, because these are the trust anchors:
   - `gate.sh`, `guard.sh`
   - the fixture harvester (`conftest.py` that records both sides of every equality)
   - the comparison function
   - the program runner skeleton (`icepool-run`)
   - fuzzer v0 (die + arithmetic only)
   - the khepri sweep generator
4. Write `AGENTS.md`, `docs/semantics.md` (from ANALYSIS.md) and the Phase K task list.
5. Write `loop.sh` and the two prompts. Verify headless runs can send push notifications; fall back to another
   channel if not.
6. **Dry run:** one supervised iteration on task K1, watched end to end. It must confirm:
   - no tool call stalls on a permission prompt;
   - a deliberately broken commit is caught by `guard.sh` and by the review.
7. Start the loop in a Terminal panel tab and leave it.

## 8. What I can't promise

- **Iteration count.** Phase K is maybe 30–80 iterations; full parity several hundred. That's a guess until the dry
  run gives a real time per iteration.
- **Speed.** Rust may be slower than Python in places at first (big-int churn, naive memo keys). Benchmarks are
  tracked from the start, but speed is not gated until correctness is done.
- **Upstream moving on.** If upstream ships 2.3 mid-port, the oracle stays pinned; catching up becomes a task.
