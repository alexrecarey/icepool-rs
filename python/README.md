# The Python facade

`python/icepool` is the `icepool` package: a copy of upstream icepool at the commit pinned in `oracle.lock`, whose
modules call the Rust core for their work when `ICEPOOL_BACKEND=rust`. The API is upstream's, and so is the
documentation: the docstrings are kept verbatim, so upstream's [documentation](https://highdiceroller.github.io/icepool/)
and [API reference](https://highdiceroller.github.io/icepool/apidoc/latest/icepool.html) apply as they are.
[DIFFERENCES.md](DIFFERENCES.md) lists what differs on the Rust backend.

## How it works

- `icepool/backend.py` reads `ICEPOOL_BACKEND` once, at import: `python` (the default) runs upstream's code
  unchanged, `rust` imports the extension `icepool_rs` (crates/icepool-py) as `icepool.backend.RUST`. Any other
  value is an `ImportError`, and so is a `rust` backend whose extension cannot be imported: there is no silent
  fallback.
- Each switched module keeps upstream's code and adds `if icepool.backend.RUST:` branches, each marked
  `icepool-rs:`. The Python side keeps upstream's argument handling (`itemize`, `Again`, `DieWithTruth`, selections
  made with Python's `in` and predicates, `@multiset_function` tracing) and calls `icepool_rs` for the computation.
  Python callables reach the core as callbacks; outcomes the core has no type for (enums, namedtuples, user
  classes, `complex`) are held as Python objects and use Python's own operators.
- A die made by the core holds it as `_rs`; a die made by upstream's Python is converted once when the core needs
  it. Pools, decks, deals and their sources work the same way.
- `icepool/evaluator/evaluator_rust.py` decides which evaluations go to the core's driver: built-in evaluators
  natively, user evaluators, sources and operators as callbacks. What the core cannot take raises
  `NotImplementedError("icepool_rs: ...")`.

[MODULE-MAP.md](../MODULE-MAP.md) says which files are *verbatim* copies and which are *switched*, with the PyO3
file each switched module calls; [FACADE-AUDIT.md](../FACADE-AUDIT.md) says what of each module still runs in
Python under `rust`, and why.

## Using it from a checkout

The wheel does not ship this package yet (task C5c), so install the extension into a virtualenv and put this
directory on the path:

```sh
uv run maturin develop --release -m crates/icepool-py/Cargo.toml   # builds icepool_rs into .venv
ICEPOOL_BACKEND=rust PYTHONPATH=python uv run python -c 'import icepool; print(icepool.d6.pool(4).highest(3).sum().mean())'
```

## Tests

- Upstream's test suite through the facade: `harness/core/upstream_suite.sh fast` (builds the extension, runs the
  oracle's `tests/` with `ICEPOOL_BACKEND=rust`; needs the oracle checkout at `$ICEPOOL_ORACLE`).
- Oracle against facade for what the program language cannot express (host outcomes, exceptions, big ints,
  callbacks with arguments, user evaluators): `uv run python -m pytest crates/icepool-py/tests -q -p no:cacheprovider`.
- Every fixture and fuzz programs through the facade:
  `ICEPOOL_RUN=$PWD/crates/icepool-py/run-facade.sh uv run python -m harness.core.tool fixtures verify --oracle-sample 0`.

When editing a module here, read the oracle's version of it first, keep upstream's code and docstrings, and mark
every change `icepool-rs:` (AGENTS.md, NOTES.md "Python facade (S1)").
