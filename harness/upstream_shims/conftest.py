"""pytest plugin for running upstream's test suite against the facade (harness/core/upstream_suite.sh loads it
with `-p harness.upstream_shims.conftest`).

The oracle's pytest.ini has `pythonpath = src`, which pytest puts first on sys.path, and importing `harness` puts
the oracle's src/ first too (harness/__init__.py). Either would make `import icepool` the oracle, and the suite
would silently test Python icepool. So this plugin imports the facade (python/icepool) before pytest adds its
paths: once `icepool` is in sys.modules, every test gets the facade. It refuses to run otherwise.
"""
import os
import sys

_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_FACADE = os.path.join(_ROOT, 'python')

for _name in [m for m in sys.modules if m == 'icepool' or m.startswith('icepool.')]:
    del sys.modules[_name]
sys.path.insert(0, _FACADE)
import icepool  # noqa: E402

if not os.path.abspath(icepool.__file__).startswith(_FACADE + os.sep):
    raise RuntimeError(f'expected the facade in {_FACADE}, imported {icepool.__file__}')


def pytest_report_header(config):
    return f'icepool facade: {os.path.dirname(icepool.__file__)}, backend {icepool.backend.BACKEND}'
