"""Test harness. The oracle is upstream icepool (pinned, see oracle.lock) checked out outside the repo, read-only.

ICEPOOL_ORACLE points at that checkout (default ~/code/alexrecarey/icepool-oracle). Importing harness puts its
src/ first on sys.path, so `import icepool` is always the oracle, never anything else.
"""
import os
import sys

ORACLE = os.path.expanduser(os.environ.get("ICEPOOL_ORACLE", "~/code/alexrecarey/icepool-oracle"))
sys.path.insert(0, os.path.join(ORACLE, "src"))
