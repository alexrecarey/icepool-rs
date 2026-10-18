"""Long computations on the Rust backend: Ctrl-C stops them, and other Python threads go on meanwhile.

    uv run python -m pytest crates/icepool-py/tests -q -p no:cacheprovider

The core polls the check set with `icepool_core::host::set_interrupt_check` in its long loops, which the facade
answers with `PyErr_CheckSignals`, and releases the GIL around pure-Rust work (built-in evaluators, `@`, built-in
operators on native dice). Each case runs in a subprocess, so a signal or a stuck computation cannot take the test
run with it.
"""
import os
import signal
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
ENV = dict(os.environ, PYTHONPATH=os.path.join(ROOT, 'python'), ICEPOOL_BACKEND='rust')

# Each takes from a minute to hours on the Rust backend.
LONG = [
    'd(100).pool(400).highest(200).sum().mean()',  # a built-in evaluator
    'd(30000) + d(30000)',  # a convolution
    'Die([d(i) for i in range(1, 30000)])',  # a big mixture
    'map(lambda x: x + d(200) if x < 6000 else x, 0, repeat=300)',  # map(repeat), a Python callback
    'map(lambda x: x + d(2000) if x < 8000 else x, 0, repeat="inf")',  # an absorbing Markov chain
]

INTERRUPT_SCRIPT = '''
import sys
from icepool import *
print('started', flush=True)
try:
    eval(sys.argv[1])
    print('finished')
except KeyboardInterrupt:
    print('interrupted')
'''


def test_ctrl_c_stops_a_long_computation():
    """SIGINT raises KeyboardInterrupt within moments (the oracle takes 0.2 s), not once the computation is done."""
    for expr in LONG:
        p = subprocess.Popen([sys.executable, '-c', INTERRUPT_SCRIPT, expr], env=ENV, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True)
        try:
            assert p.stdout.readline().strip() == 'started', p.stderr.read()
            time.sleep(1)  # well into the computation
            p.send_signal(signal.SIGINT)
            start = time.monotonic()
            out, err = p.communicate(timeout=30)
            assert out.strip() == 'interrupted', (expr, out, err[-2000:])
            assert time.monotonic() - start < 5, expr
        finally:
            p.kill()
            p.wait()


THREAD_SCRIPT = '''
import threading, time
from icepool import d
ticks = 0
done = False
def tick():
    global ticks
    while not done:
        ticks += 1
        time.sleep(0.01)
ticker = threading.Thread(target=tick)
ticker.start()
start = time.monotonic()
for expr in [lambda: d(100).pool(20).highest(10).sum(), lambda: 40 @ d(100), lambda: d(1500) + d(1500)]:
    while time.monotonic() - start < 1:
        expr()
    seconds, before = time.monotonic() - start, ticks
    print(before / seconds)
    start, ticks = time.monotonic(), 0
done = True
ticker.join()
'''


def test_other_threads_run_during_a_long_computation():
    """The GIL is released while the core computes: a ticking thread keeps ticking (100 ticks a second, with
    the GIL held throughout it would barely tick at all)."""
    p = subprocess.run([sys.executable, '-c', THREAD_SCRIPT], env=ENV, capture_output=True, text=True, timeout=120)
    rates = [float(line) for line in p.stdout.split()]
    assert len(rates) == 3, p.stderr[-2000:]
    for rate in rates:
        assert rate > 50, rates
