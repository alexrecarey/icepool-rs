"""Build-time constants of the wheel (not part of upstream icepool).

scripts/build-wheel.sh adds this module to the wheel's copy of python/icepool; the repository's own copy has none,
so backend.py's defaults there stay those of a checkout.
"""

# `ICEPOOL_BACKEND` when it is unset: the wheel ships the Rust core, so it runs on it.
DEFAULT_BACKEND = 'rust'
