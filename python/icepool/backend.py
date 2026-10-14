"""The facade's backend switch (not part of upstream icepool).

This package is a copy of upstream icepool (pinned in oracle.lock) whose modules are switched one at a time to the
Rust core (the `icepool_rs` extension, crates/icepool-py). `ICEPOOL_BACKEND` selects the backend when icepool is
imported:

* `python` (the default): upstream's pure-Python code everywhere, exactly as upstream ships it.
* `rust`: every switched module runs on the Rust core. There is no silent fallback: if the extension cannot be
    imported, importing icepool fails.

`RUST` is the extension module when the backend is `rust`, else `None`; switched code tests `if RUST:`.
Switched so far: Die construction (`Die.__new__`'s merge and sort) and Die arithmetic (`unary_operator`,
`binary_operator`).
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
