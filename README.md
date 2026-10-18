# icepool-rs

A Rust port of [icepool](https://github.com/HighDiceRoller/icepool), the Python library for exact dice
probabilities, aiming at full parity: a Rust core, the same Python API on top of it (PyO3), and a JavaScript and
TypeScript API (WASM). Results are Python icepool's exactly: the same outcomes in the same order, the same types and
the same integer quantities, and an error wherever Python raises one. It is built by an unattended agent loop
that checks every result against Python icepool itself.

- What and why: [docs/ANALYSIS.md](docs/ANALYSIS.md)
- How the loop works: [docs/LOOP-PLAN.md](docs/LOOP-PLAN.md)
- Rules for agents: [AGENTS.md](AGENTS.md)
- Where it's at: [PROGRESS.md](PROGRESS.md), [tasks.json](tasks.json)

Nothing is published yet (no crate, wheel or npm package); everything below builds from this repository.

## Three bindings, one core

| | Package | Where | Documentation |
|---|---|---|---|
| Rust | crate `icepool-core` | `crates/icepool-core` | its [crate guide](crates/icepool-core/README.md) and rustdoc (`cargo doc -p icepool-core --open`) |
| Python | package `icepool` (the facade) over the extension `icepool_rs` | `python/icepool`, `crates/icepool-py` | upstream's [documentation](https://highdiceroller.github.io/icepool/); [python/README.md](python/README.md), [python/DIFFERENCES.md](python/DIFFERENCES.md) |
| JS / TS | npm package `icepool-wasm` | `crates/icepool-wasm`, `crates/icepool-wasm/js` | its [README](crates/icepool-wasm/js/README.md) and [guide](crates/icepool-wasm/js/docs/guide.md) |

### Rust

The core is generic: any `Ord` type is an outcome, so `Die<i64>` answers with plain values. The bindings use
`Value`, an outcome with Python's semantics, and that instantiation is what the oracle checks.

```toml
[dependencies]
icepool-core = { git = "https://github.com/alexrecarey/icepool-rs" }  # or a path to crates/icepool-core
```

```rust
use icepool_core::evaluator::poker::LargestCountEvaluator;
use icepool_core::{d, Die, Int};

let two_d6 = &d(6) + &d(6);
assert_eq!(two_d6.denominator().to_string(), "36");
assert_eq!(two_d6.quantity_le(&2).to_string(), "1"); // snake eyes: 1 way in 36
assert_eq!(d(6).sum_rolls(3), &two_d6 + d(6)); // Python's 3 @ d6

let hits: Die<bool> = d(20).ge(11);
assert_eq!(hits, Die::from_weights([(false, 10u32), (true, 10)]));

// How many of a kind in 3d6, by a built-in evaluator.
let largest = d(6).pool(3).evaluate(&LargestCountEvaluator).unwrap();
assert_eq!(largest, Die::from_weights([(Int::from(1), 120u32), (Int::from(2), 90), (Int::from(3), 6)]));
```

The crate guide covers pools, writing an evaluator, exact quantities, typed outcomes and `Value`, and errors.

### Python

`python/icepool` is upstream icepool (the version in `oracle.lock`) with its docstrings kept verbatim, whose
modules call the Rust core for their work. `ICEPOOL_BACKEND` picks the backend when `icepool` is imported:
`python` (the default today) runs upstream's pure-Python code unchanged, `rust` runs the core. Code written for
icepool runs as it is:

```python
import icepool
from icepool import d6, d

(d6 + d6).probability(7)              # Fraction(1, 6)
d6.pool(4).highest(3).sum().mean()    # Fraction(15869, 1296)
d6.pool(5).largest_count().probability('>=', 3)   # Fraction(23, 108)
d(20).explode(depth=2).denominator()  # 8000
icepool.Deck([1, 2, 3, 4], times=2).deal(3).sum().mean()  # Fraction(15, 2)
```

The wheel ships the facade with the extension, and runs on the Rust backend unless `ICEPOOL_BACKEND` says
otherwise:

```sh
scripts/build-wheel.sh dist                         # dist/icepool_rs-*.whl: the icepool package and icepool_rs
pip install dist/icepool_rs-*.whl && python -c 'import icepool; print(icepool.d6 + 1)'
```

From a checkout (the default there is `python`, upstream's code), install the extension alone into a virtualenv and
put `python/` on the path:

```sh
uv run maturin develop --release -m crates/icepool-py/Cargo.toml   # icepool_rs into .venv
ICEPOOL_BACKEND=rust PYTHONPATH=python uv run python -c 'import icepool; print(icepool.d6 + 1)'
```

See [python/README.md](python/README.md) for how the switch works and [python/DIFFERENCES.md](python/DIFFERENCES.md)
for what differs on the Rust backend.

### JavaScript and TypeScript

A TypeScript API mirroring Python icepool's, in JS's conventions: camelCase names, operators as methods, keyword
arguments as a trailing options object, BigInt quantities.

```sh
cd crates/icepool-wasm/js && npm ci && ./build.sh   # wasm-pack and tsc; the package is in this directory
```

```ts
import { d, Die, Deck, Again } from "icepool-wasm";

const d6 = d(6);
d6.add(d6).probability(7).toString(); //=> "1/6"
d6.pool(4).highest(3).sum().mean().toString(); //=> "15869/1296"
new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 2 }).equals(d6.explode([6], { depth: 2 })); //=> true
new Deck([1, 2, 3, 4], 2).deal(3).sum().mean().toString(); //=> "15/2"
```

Some methods still reach the core through an interim JSON-op bridge rather than typed exports (tasks C6i-C6l); the
results are the same, and the bridge leaves the package in C6l.

## Parity: Python icepool is the oracle

The oracle is upstream icepool pinned by commit in [oracle.lock](oracle.lock) (alexrecarey/icepool
`fix/multiset-bugs`: upstream v2.2.2 plus three bug fixes on their way upstream), checked out read-only beside the
repository (`$ICEPOOL_ORACLE`). Nothing decides by itself that the port is right; these checks do:

- **Fixtures**: about 11,900 programs in a small JSON language (`harness/grammar`, 243 nodes), each with the result
  the oracle computed, in `fixtures/` (append-only). `cargo test` runs every one natively; the JS suite runs every
  one through the TypeScript API.
- **Fuzzer**: random programs run by both the oracle and the port, compared exactly (`harness/core`); failures are
  minimised and become fixtures.
- **Upstream's test suite**, run unmodified through the Python facade with `ICEPOOL_BACKEND=rust` (1716 passing, as
  on the oracle; `baselines/upstream-pass.txt` may only grow), and translated test by test into JS
  (`crates/icepool-wasm/js/test/upstream-*.test.mjs`).
- **Typed against `Value`**: property tests check that each typed `Die<i64>` path agrees with the `Value` path the
  fixtures check (`crates/icepool-core/tests/typed_vs_value.rs`).

Where Python does something odd or wrong, the port copies it and [DIVERGENCES.md](DIVERGENCES.md) records it with
a drafted upstream test. The only results that are not bit-exact are some floats under WASM (within 2 ULP, also
recorded there).

[MODULE-MAP.md](docs/MODULE-MAP.md) maps every upstream module to the Rust, Python and TypeScript files that port it,
and `scripts/upstream_changes.py OLD NEW` lists what changed upstream between two releases and which of those files
to revisit.

## Repository layout

| Path | What |
|---|---|
| `crates/icepool-core/` | the library: dice, pools, decks, evaluators, multiset expressions, `Value` |
| `crates/icepool-py/` | the PyO3 extension `icepool_rs` (and its Python tests in `tests/`) |
| `python/icepool/` | the Python facade: upstream's modules, switched to `icepool_rs` under `ICEPOOL_BACKEND=rust` |
| `crates/icepool-wasm/` | wasm-bindgen exports; `js/` is the npm package (TypeScript API, docs, tests) |
| `crates/icepool-harness/` | test only: the JSON program runner, the `icepool-run` binary, the fixture test |
| `harness/` | the Python side of the checks: `core/` (gate, guard, comparison, fuzzer, fixture tools), `grammar/` (program nodes), `catalogue/`, `upstream_shims/` |
| `fixtures/` | golden results computed by the oracle |
| `baselines/` | the upstream tests known to pass through the facade |
| `scripts/` | `check-production.sh` (no test code in what ships), `upstream_changes.py` |
| `docs/` | analysis, loop plan, Python semantics the port copies, the library design, reviews |
| `.github/workflows/` | CI |
| `loop/`, `tasks.json`, `PROGRESS.md`, `NOTES.md`, `QUESTIONS.md` | the loop's machinery and records |

## Building and testing

Needs the Rust toolchain from `rust-toolchain.toml` (stable, with the wasm32 target), [uv](https://docs.astral.sh/uv/),
maturin, wasm-pack and node 22; the oracle comparisons also need the oracle checkout (see oracle.lock).

```sh
cargo build --release --workspace
cargo test --release --workspace          # unit tests, doctests, and every fixture (no oracle needed)
cargo clippy --release --workspace -- -D warnings
crates/icepool-wasm/js/build.sh --test    # the JS suite: every fixture through the TS API, API and upstream tests
scripts/check-production.sh               # the wheel and the npm package contain no test code
harness/core/gate.sh fast                 # everything the loop checks after each iteration (needs the oracle)
```

With the oracle checked out at `$ICEPOOL_ORACLE`:

```sh
uv run python -m harness.core.tool fixtures verify --oracle-sample 0.05   # fixtures, 5% recomputed by the oracle
uv run python -m harness.core.tool fuzz --seed 1 --count 2000             # oracle vs port on random programs
uv run python -m harness.core.tool check '<program json>'                 # one program
harness/core/upstream_suite.sh fast                                       # upstream's tests through the facade
```

CI (`.github/workflows/ci.yml`) runs the fast gate's checks on every push and pull request, with the oracle checked out at
the commit in oracle.lock; the gate's fresh-seed fuzz runs and the long gate (big fuzz runs, WASM runs, mutants)
stay with the loop.

## Further reading

- [MODULE-MAP.md](docs/MODULE-MAP.md): upstream module -> Rust, Python and TypeScript files
- [python/DIFFERENCES.md](python/DIFFERENCES.md): what differs from upstream icepool on the Rust backend
- [docs/design/library-architecture.md](docs/design/library-architecture.md): the typed core, APIs, crates and builds
- [docs/semantics.md](docs/semantics.md): the Python semantics the port must copy
- [DIVERGENCES.md](DIVERGENCES.md): Python behaviour copied but thought wrong
- [FACADE-AUDIT.md](docs/FACADE-AUDIT.md): what of the Python facade still runs in Python, module by module
- [docs/ANALYSIS.md](docs/ANALYSIS.md), [docs/LOOP-PLAN.md](docs/LOOP-PLAN.md), [AGENTS.md](AGENTS.md),
  [NOTES.md](NOTES.md): the project and the loop that builds it
