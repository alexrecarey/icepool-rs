# The extension's package, as maturin generates it for an extension alone (crates/icepool-py/pyproject.toml); the
# wheel's mixed layout (python-source) needs it written out.
from .icepool_rs import *

__doc__ = icepool_rs.__doc__
if hasattr(icepool_rs, "__all__"):
    __all__ = icepool_rs.__all__
