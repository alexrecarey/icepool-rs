"""The icepool-run protocol (one JSON program per stdin line, one JSON result per stdout line) through the Python
facade (python/icepool) on the Rust backend. Use it as ICEPOOL_RUN (via run-facade.sh) to send every fixture and
fuzz run through the facade: programs are evaluated by the harness's own grammar (the oracle side, whose nodes are
direct calls into icepool), but `icepool` is the facade instead of the oracle.

A NotImplementedError raised by icepool_rs (a case the core does not cover yet) is reported as unimplemented, as
icepool-run reports `Error::Unported`; every other exception is a raise, encoded as the oracle encodes it.
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FACADE = os.path.join(ROOT, 'python')
os.environ['ICEPOOL_BACKEND'] = 'rust'
sys.path.insert(0, FACADE)
sys.path.insert(1, ROOT)
import icepool  # noqa: E402  (the facade: imported before harness puts the oracle on sys.path)

assert os.path.abspath(icepool.__file__).startswith(FACADE + os.sep), icepool.__file__
assert icepool.backend.RUST is not None

from harness.core import grammar_api as G  # noqa: E402
from harness.core.encode import error_of, result_of  # noqa: E402

G.load_grammar()


def run(program):
    try:
        return result_of(G.evaluate(program))
    except NotImplementedError as e:
        if str(e).startswith('icepool_rs:'):
            return {'ok': False, 'error': f'unimplemented: {e}'}
        return error_of(e)
    except Exception as e:  # noqa: BLE001 - any raise is an answer, as for the oracle
        return error_of(e)


for line in sys.stdin:
    if line.strip():
        print(json.dumps(run(json.loads(line)), separators=(',', ':')), flush=True)
