"""The facade's backend switch (not part of upstream icepool).

This package is a copy of upstream icepool (pinned in oracle.lock) whose modules are switched one at a time to the
Rust core (the `icepool_rs` extension, crates/icepool-py). `ICEPOOL_BACKEND` selects the backend when icepool is
imported:

* `python` (the default): upstream's pure-Python code everywhere, exactly as upstream ships it.
* `rust`: every switched module runs on the Rust core. There is no silent fallback: if the extension cannot be
    imported, importing icepool fails.

`RUST` is the extension module when the backend is `rust`, else `None`; switched code tests `if RUST:`.
Which modules are switched, and what work still runs in Python under `rust` (with the task that covers it), is
listed in FACADE-AUDIT.md at the repository root.
"""

import os

BACKEND = os.environ.get('ICEPOOL_BACKEND', 'python')

if BACKEND == 'rust':
    import icepool_rs as RUST
elif BACKEND == 'python':
    RUST = None
else:
    raise ImportError(
        f"ICEPOOL_BACKEND must be 'python' or 'rust', got {BACKEND!r}")
