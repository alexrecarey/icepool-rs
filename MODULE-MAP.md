# Module map: upstream icepool -> icepool-rs

Where each module of upstream icepool lives in this port. Oracle: `70168f56d807e454e6a86855b0fd4b9c25f51ee9`
(oracle.lock: alexrecarey/icepool `fix/multiset-bugs` = upstream v2.2.2 + 3 fixes, see DIVERGENCES.md "The oracle
itself"). Written by task C8, 2026-10-18, by reading the files (the Rust files cite their Python source in their
module comments).

This file belongs in `docs/MODULE-MAP.md`; `docs/` is protected, so it is at the root until the loop owner moves it
(QUESTIONS.md, "C8: MODULE-MAP.md belongs in docs/").

**Reading the table.** One row per `.py` file under upstream's `src/icepool/`, upstream path in backticks in the
first column. `scripts/upstream_changes.py` parses these rows (any table row whose first cell is a backticked
`.py` path), so keep that shape when editing.

- **Rust core**: files under `crates/icepool-core/src/`. `value/ops/` is a directory (Python's operators on
  `Value`).
- **Python facade**: `python/icepool/` holds a copy of every upstream file at the same path. *verbatim* = identical
  to the oracle's; *switched* = upstream's code with `if icepool.backend.RUST:` branches (marked `icepool-rs:`)
  that call the extension. Then the PyO3 files of that extension, `icepool_rs`, under `crates/icepool-py/src/`.
- **TypeScript**: files under `crates/icepool-wasm/js/src/`, then the wasm-bindgen exports they call, under
  `crates/icepool-wasm/src/` (mostly `api/`).
- **Status**:
  - *ported*: the core does the work, the Python facade calls it under `ICEPOOL_BACKEND=rust`, and the TypeScript
    API calls typed exports for it;
  - *facade*: upstream's Python composes calls that are on the core (declarations, base classes, enums, classes
    whose behaviour lives elsewhere); nothing of its own to port;
  - *package*: a subpackage's `__init__.py`, imports only;
  - *internal*: helpers with no public API of their own.
- **Oracle**: the upstream commit the row was last checked against.

FACADE-AUDIT.md says, module by module, what of the Python facade still runs in Python under `rust` and why.

| Upstream | Rust core | Python facade | PyO3 | TypeScript | WASM exports | Status | Oracle |
|---|---|---|---|---|---|---|---|
| `__init__.py` | `lib.rs` | switched | `lib.rs` | `index.ts` | `api/mod.rs` | ported | 70168f56 |
| `collection/__init__.py` | — | verbatim | — | — | — | package | 70168f56 |
| `collection/counts.py` | `counts.rs` | switched | — | — | — | internal (the facade keeps a Python copy of the core's items) | 70168f56 |
| `collection/symbols.py` | `value/symbols.rs` | switched | `collection.rs` | `values.ts` | `api/value.rs` | ported | 70168f56 |
| `collection/vector.py` | `value/vector.rs`, `value/ops/` | switched | `collection.rs` | `values.ts` | `api/value.rs` | ported | 70168f56 |
| `collection/vector_with_truth_only.py` | `value/vector.rs` | verbatim | `convert.rs` | `values.ts` | `api/convert.rs`, `api/host.rs` | ported | 70168f56 |
| `creation_args.py` | `creation_args.rs` | switched | `lib.rs`, `deck.rs` | `die.ts`, `again.ts`, `deck.ts` | `api/die.rs`, `api/deck.rs`, `api/function.rs` | ported | 70168f56 |
| `evaluator/__init__.py` | — | verbatim | — | — | — | package | 70168f56 |
| `evaluator/argsort.py` | `evaluator/argsort.rs` | verbatim | `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `evaluator/basic.py` | `evaluator/basic.rs`, `value/evaluators.rs` | verbatim | `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `evaluator/comparison.py` | `evaluator/comparison.rs` | verbatim | `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `evaluator/keep.py` | `evaluator/keep.rs` | verbatim | `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `evaluator/lexi.py` | `evaluator/lexi.rs` | verbatim | `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `evaluator/multiset_evaluator.py` | `evaluator/mod.rs` | verbatim | `evaluate.rs` | `evaluator.ts` | `api/evaluator.rs` | ported | 70168f56 |
| `evaluator/multiset_evaluator_base.py` | `evaluator/mod.rs` | switched (with the facade's own `evaluator/evaluator_rust.py`) | `evaluate.rs` | `evaluator.ts` | `api/evaluator.rs`, `api/evaluate.rs` | ported | 70168f56 |
| `evaluator/multiset_function.py` | `evaluator/multiset_function.rs` | verbatim (tracing stays Python by design) | `evaluate.rs` | `evaluator.ts` | `api/multiset_function.rs` | ported | 70168f56 |
| `evaluator/poker.py` | `evaluator/poker.rs`, `evaluator/fold.rs`, `value/evaluators.rs` | verbatim | `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `expand.py` | `expand.rs` | switched | `map.rs`, `population.rs` | `functions.ts` | `api/function.rs` | ported | 70168f56 |
| `expression/__init__.py` | — | verbatim | — | — | — | package | 70168f56 |
| `expression/multiset_expression.py` | `expression.rs`, `operator/mod.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/multiset.rs`, `api/evaluate.rs` | ported | 70168f56 |
| `expression/multiset_expression_base.py` | `expression.rs`, `host/source.rs` | verbatim | `expression.rs`, `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `expression/multiset_mixture.py` | — (one core evaluation per inner) | verbatim | `evaluate.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `expression/multiset_parameter.py` | `expression.rs` | verbatim | `evaluate.rs` | `evaluator.ts`, `multiset.ts` | `api/multiset_function.rs`, `api/evaluate.rs` | ported | 70168f56 |
| `expression/multiset_tuple_expression.py` | `expression.rs` | verbatim | `expression.rs`, `evaluate.rs` | `multiset.ts`, `deck.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `function.py` | `function.rs`, `math.rs` | switched | `function.rs` | `functions.ts` | `api/function.rs` | ported | 70168f56 |
| `generator/__init__.py` | — | verbatim | — | — | — | package | 70168f56 |
| `generator/compound_keep.py` | `compound_keep.rs` | switched | `expression.rs` | `multiset.ts` | `api/multiset.rs` | ported | 70168f56 |
| `generator/deal.py` | `deal.rs` | switched | `deck.rs` | `deck.ts` | `api/deck.rs`, `api/multiset.rs` | ported | 70168f56 |
| `generator/keep.py` | `keep_tuple.rs`, `pool.rs` | switched | `pool.rs` | `multiset.ts` | `api/multiset.rs` | ported | 70168f56 |
| `generator/multi_deal.py` | `multi_deal.rs` | switched | `multi_deal.rs` | `deck.ts` | `api/deck.rs` | ported | 70168f56 |
| `generator/multiset_generator.py` | `host/source.rs`, `pool.rs` | verbatim | `host_source.rs` | `multiset.ts` | `api/multiset.rs` | ported (TS: the built-in generators only, no subclasses written in JS) | 70168f56 |
| `generator/multiset_tuple_generator.py` | `multi_deal.rs`, `expression.rs` | verbatim | `host_source.rs` | `deck.ts` | `api/deck.rs` | ported (TS: `MultiDeal` only, no subclasses written in JS) | 70168f56 |
| `generator/pool.py` | `pool.rs` | switched | `pool.rs` | `multiset.ts` | `api/multiset.rs` | ported | 70168f56 |
| `generator/weightless.py` | `weightless.rs` | switched | `weightless.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `lexi.py` | `lexi.rs`, `evaluator/lexi.rs` | verbatim | `expression.rs`, `evaluate.rs` | `multiset.ts` | `api/multiset.rs`, `api/evaluate.rs` | ported | 70168f56 |
| `map_tools/__init__.py` | — | verbatim | — | — | — | package | 70168f56 |
| `map_tools/common.py` | `map_tools/core_impl.rs` | verbatim | `map.rs`, `map_tools.rs` | `functions.ts`, `values.ts` | `api/map_tools.rs` | ported | 70168f56 |
| `map_tools/core_impl.py` | `map_tools/core_impl.rs` | switched | `map.rs`, `map_tools.rs` | `functions.ts` | `api/function.rs`, `api/map_tools.rs` | ported | 70168f56 |
| `map_tools/function.py` | `map_tools/mod.rs`, `to_pool.rs` | switched | `map_tools.rs`, `map.rs`, `to_pool.rs` | `functions.ts` | `api/map_tools.rs`, `api/function.rs` | ported | 70168f56 |
| `map_tools/markov_chain.py` | `map_tools/markov_chain.rs` | switched | `map_tools.rs` | `functions.ts`, `die.ts` | `api/map_tools.rs` | ported | 70168f56 |
| `math.py` | `math.rs` | verbatim (unused under `rust`) | — | — | — | internal | 70168f56 |
| `operator/__init__.py` | — | verbatim | — | — | — | package | 70168f56 |
| `operator/adjust_counts.py` | `operator/adjust_counts.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `operator/binary.py` | `operator/binary.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `operator/debug.py` | `operator/debug.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `operator/filter_outcomes.py` | `operator/filter_outcomes.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `operator/keep.py` | `operator/keep.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `operator/multiset_operator.py` | `operator/mod.rs`, `host/operator.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported (TS: the built-in operators only, no subclasses written in JS) | 70168f56 |
| `operator/pair.py` | `operator/pair.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `operator/versus.py` | `operator/versus.rs` | verbatim | `expression.rs` | `multiset.ts` | `api/evaluate.rs` | ported | 70168f56 |
| `order.py` | `order.rs` | verbatim | `evaluate.rs` | `values.ts`, `errors.ts` | — | ported | 70168f56 |
| `population/__init__.py` | — | verbatim | — | — | — | package | 70168f56 |
| `population/again.py` | `again.rs` | switched | `again.rs` | `again.ts` | `api/again.rs` | ported | 70168f56 |
| `population/base.py` | `population.rs`, `value/builtins.rs` | switched | `stats.rs`, `population.rs`, `format.rs`, `lib.rs` | `die.ts`, `deck.ts` | `api/die.rs`, `api/deck.rs` | ported | 70168f56 |
| `population/deck.py` | `deck.rs` | switched | `deck.rs`, `population.rs` | `deck.ts` | `api/deck.rs` | ported | 70168f56 |
| `population/die.py` | `die.rs`, `die_ops.rs`, `to_pool.rs`, `value/matmul.rs` | switched | `lib.rs`, `ops.rs`, `to_pool.rs`, `population.rs`, `format.rs` | `die.ts`, `operators.ts` | `api/die.rs` | ported | 70168f56 |
| `population/die_with_truth.py` | — | verbatim | — | `die.ts` | `api/die.rs` | facade | 70168f56 |
| `population/format.py` | `format.rs`, `value/pystr.rs` | switched (CSV still written by Python's `csv`) | `format.rs` | `die.ts`, `deck.ts` | `api/die.rs`, `api/deck.rs` | ported | 70168f56 |
| `population/keep.py` | `keep.rs`, `keep_tuple.rs` | switched | `lib.rs` | `die.ts`, `functions.ts` | `api/multiset.rs` (the slices only) | ported | 70168f56 |
| `typing.py` | `outcome.rs` | verbatim | — | `values.ts` | — | facade (protocols and enums; `infer_star` stays in each binding) | 70168f56 |
| `wallenius.py` | — (composes deck and die calls) | verbatim | — | `deck.ts` | `api/function.rs` | facade | 70168f56 |

## Not in upstream

Rust core modules with no upstream counterpart: `die_ops.rs` (the typed `std::ops` surface), `error.rs`, `int.rs`
(unbounded counts), `outcome.rs` (the outcome traits; Python's are protocols in `typing.py`), `value/` (`Value`,
the dynamic outcome with Python's semantics: CPython's numeric tower, `sorted()`, `str()`/`repr()`, `complex`,
builtins, recursion limits), `host/` (callbacks a binding implements), `interrupt.rs` (Ctrl-C), `memory.rs`
(allocation size checks), `ownership.rs` (cycle collection support), `random.rs` (CPython's Mersenne Twister, for
the runner and the JS `sample`), `khepri.rs` (application code behind the `khepri` feature; task C2f moves it to
its own crate).

Python facade files with no upstream counterpart: `python/icepool/backend.py` (the `ICEPOOL_BACKEND` switch) and
`python/icepool/evaluator/evaluator_rust.py` (which evaluations go to the core). PyO3: `convert.rs` (Python objects
to and from `Value`, exceptions), `ops.rs` (Python's operator functions mapped to the core's).

TypeScript files with no upstream counterpart: `wasm.ts`, `node.ts`, `web.ts` (loading the module), `host.ts`,
`errors.ts`, `operators.ts` (Python's operator dispatch), `internal.ts` and `rpc.ts` (test-only: the fixture tests'
bridge to the program runner, left out of the npm package); WASM: `src/facade.rs` (that bridge, feature `runner`),
`src/runner.rs` (test-only `run`), `src/khepri.rs` (`calculate`).

The test-only program runner, `crates/icepool-harness/src/`, mirrors the same upstream modules as JSON program
nodes (see `harness/grammar/status.json` for every node and its status).
