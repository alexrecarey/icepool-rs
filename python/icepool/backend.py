"""The facade's backend switch (not part of upstream icepool).

This package is a copy of upstream icepool (pinned in oracle.lock) whose modules call the Rust core (the
`icepool_rs` extension, crates/icepool-py) for their work. `ICEPOOL_BACKEND` selects the backend when icepool is
imported:

* `python` (the default in a checkout): upstream's pure-Python code everywhere, exactly as upstream ships it.
* `rust`: each module keeps its Python API and argument juggling, and its computational work runs in the Rust core;
    Python callables (map functions, user evaluators, user sources and operators, `outcome_type`...) are called
    back from the core. There is no silent fallback to upstream's code path: if the extension cannot be imported,
    importing icepool fails, and what the core cannot take raises `NotImplementedError("icepool_rs: ...")`.
    The default in the wheel (scripts/build-wheel.sh), which ships the extension with this package.

The default is a build-time constant: the wheel adds `icepool/_wheel.py` (crates/icepool-py/wheel/) with
`DEFAULT_BACKEND = 'rust'`; this checkout has no such module, so the harness, which always sets ICEPOOL_BACKEND,
runs as it always has.

`RUST` is the extension module when the backend is `rust`, else `None`; switched code tests `if RUST:` (each such
place is marked `icepool-rs:`). FACADE-AUDIT.md at the repository root lists every module, what of it still runs in
Python under `rust` and why (by design, or the task that covers it).
"""

import os

try:
    from icepool._wheel import DEFAULT_BACKEND
except ModuleNotFoundError:
    DEFAULT_BACKEND = 'python'

BACKEND = os.environ.get('ICEPOOL_BACKEND', DEFAULT_BACKEND)

if BACKEND == 'rust':
    import icepool_rs as RUST
elif BACKEND == 'python':
    RUST = None
else:
    raise ImportError(
        f"ICEPOOL_BACKEND must be 'python' or 'rust', got {BACKEND!r}")
