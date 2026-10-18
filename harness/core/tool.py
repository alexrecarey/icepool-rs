"""Harness CLI. PROTECTED. Run from the repo root:  uv run python -m harness.core.tool <command> ...

  oracle PROGRAM_JSON              print the oracle's result for one program
  port PROGRAM_JSON                print the Rust port's result for one program
  check PROGRAM_JSON               both, plus the verdict
  fuzz [--seed N] [--count N] [--ops a,b] [--fresh] [--depth N]
                                   random programs from ported nodes, oracle vs port; minimised failures are
                                   written to .loop/out/failures.jsonl
  fixtures verify [--oracle-sample F]
                                   every fixture through the port (fixtures whose ops are not all ported are
                                   skipped and counted); a random fraction F re-checked against the oracle
  fixtures make NAME PROGRAMS.jsonl
                                   compute expected results WITH THE ORACLE and write fixtures/NAME.jsonl (new
                                   file only: fixtures are append-only)
  fixtures import-failures NAME    turn .loop/out/failures.jsonl into fixtures/NAME.jsonl (oracle recomputed)
  ops                              list nodes and their status

The Rust side is the `icepool-run` binary: one JSON program per stdin line in, one JSON result per stdout line out.
Its path is $ICEPOOL_RUN or $CARGO_TARGET_DIR/release/icepool-run (default target/release/icepool-run).
"""
import argparse
import hashlib
import json
import os
import random
import signal
import subprocess
import sys
import time

import harness  # noqa: F401  (puts the oracle on sys.path)
from harness.core import grammar_api as G
from harness.core.compare import same
from harness.core.encode import error_of, result_of

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, ".loop/out")
FIXTURES = os.path.join(ROOT, "fixtures")
ORACLE_TIMEOUT_S = 3
PORT_TIMEOUT_S = int(os.environ.get("ICEPOOL_PORT_TIMEOUT", "20"))


class OracleTimeout(Exception):
    pass


def _alarm(signum, frame):
    raise OracleTimeout()


def oracle(program, timeout=ORACLE_TIMEOUT_S):
    """Oracle result, or None if the oracle itself is too slow (then the program is not a usable test)."""
    signal.signal(signal.SIGALRM, _alarm)
    signal.alarm(timeout)
    try:
        return result_of(G.evaluate(program))
    except OracleTimeout:
        return None
    except RecursionError as e:
        return error_of(e)
    except Exception as e:  # noqa: BLE001 - any raise is a valid oracle answer
        return error_of(e)
    finally:
        signal.alarm(0)


def run_binary():
    if os.environ.get("ICEPOOL_RUN"):
        return os.environ["ICEPOOL_RUN"]
    target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
    return os.path.join(target, "release", "icepool-run")


class Port:
    """A long-lived icepool-run process. Restarted after a crash or timeout."""

    def __init__(self):
        self.p = None

    def _start(self):
        self.p = subprocess.Popen([run_binary()], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True, bufsize=1)

    def run(self, program):
        if self.p is None or self.p.poll() is not None:
            self._start()
        line = json.dumps(program, separators=(",", ":"))
        try:
            self.p.stdin.write(line + "\n")
            self.p.stdin.flush()
        except BrokenPipeError:
            self.close()
            return {"ok": False, "error": "unimplemented: icepool-run crashed (broken pipe)"}
        import select
        ready, _, _ = select.select([self.p.stdout], [], [], PORT_TIMEOUT_S)
        if not ready:
            self.close()
            return {"ok": False, "error": f"unimplemented: icepool-run timed out after {PORT_TIMEOUT_S}s"}
        out = self.p.stdout.readline()
        if not out:
            err = self.p.stderr.read()[-500:] if self.p.stderr else ""
            self.close()
            return {"ok": False, "error": f"unimplemented: icepool-run crashed: {err}"}
        try:
            return json.loads(out)
        except json.JSONDecodeError:
            return {"ok": False, "error": f"unimplemented: unparseable output {out[:200]!r}"}

    def close(self):
        if self.p:
            self.p.kill()
            self.p.wait()
        self.p = None


# --- Shrinking ---------------------------------------------------------------------------------------------------

def _subtrees(x, path=()):
    if isinstance(x, dict):
        if "op" in x:
            yield path, x
        for k, v in x.items():
            if k != "op":
                yield from _subtrees(v, path + (k,))
    elif isinstance(x, list):
        for i, v in enumerate(x):
            yield from _subtrees(v, path + (i,))


def _replace(x, path, new):
    if not path:
        return new
    x = json.loads(json.dumps(x))
    cur = x
    for p in path[:-1]:
        cur = cur[p]
    cur[path[-1]] = new
    return x


def _candidates(prog):
    subs = list(_subtrees(prog))
    # Replace a node by one of its own descendants (same position, simpler program).
    for path, sub in subs:
        for dpath, desc in _subtrees(sub):
            if dpath:
                yield _replace(prog, path, desc)
    # Shorten lists and shrink integers.
    def walk(x, path):
        if isinstance(x, list):
            for i in range(len(x)):
                yield path, x[:i] + x[i + 1:]
            for i, v in enumerate(x):
                yield from walk(v, path + (i,))
        elif isinstance(x, dict):
            for k, v in x.items():
                if k != "op":
                    yield from walk(v, path + (k,))
        elif isinstance(x, int) and not isinstance(x, bool) and abs(x) > 1:
            yield path, x // 2
            yield path, 1
    for path, new in walk(prog, ()):
        yield _replace(prog, path, new)


def shrink(prog, port, budget=300, seconds=60):
    deadline = time.time() + seconds

    def fails(p):
        e = oracle(p)
        return e is not None and not same(e, port.run(p))[0]
    best = prog
    improved = True
    while improved and budget > 0:
        improved = False
        for c in _candidates(best):
            budget -= 1
            if budget <= 0 or time.time() > deadline:
                budget = 0
                break
            if len(json.dumps(c)) < len(json.dumps(best)) and fails(c):
                best, improved = c, True
                break
    return best


# --- Commands ----------------------------------------------------------------------------------------------------

def cmd_fuzz(a):
    G.load_grammar()
    allowed = G.ported_ops()
    if a.ops:
        allowed &= set(a.ops.split(","))
    if not allowed:
        print("fuzz: no ported ops (nothing to do)")
        return 0
    seed = a.seed if not a.fresh else random.SystemRandom().randrange(2**32)
    g = G.Gen(random.Random(seed), allowed, a.depth)
    kinds = sorted({G.NODES[o].returns for o in allowed if G.NODES[o].gen})
    if a.ops and "khepri" in a.ops.split(","):
        kinds = ["die"]
    port = Port()
    os.makedirs(OUT, exist_ok=True)
    tested = skipped = 0
    failures = []
    t0 = time.time()
    for i in range(a.count):
        try:
            prog = g.make(g.rng.choice(kinds))
        except LookupError:
            continue
        exp = oracle(prog)
        if exp is None:
            skipped += 1
            continue
        tested += 1
        ok, why = same(exp, port.run(prog))
        if not ok:
            small = shrink(prog, port)
            e2 = oracle(small)
            failures.append({"program": small, "expected": e2, "reason": same(e2, port.run(small))[1],
                             "original": prog, "seed": seed, "index": i})
            if len(failures) >= a.max_failures:
                break
    port.close()
    with open(os.path.join(OUT, "failures.jsonl"), "a") as f:
        for x in failures:
            f.write(json.dumps(x) + "\n")
    print(f"fuzz seed={seed} ops={','.join(sorted(allowed))}: {tested} tested, {skipped} skipped (oracle slow), "
          f"{len(failures)} failures, {time.time() - t0:.1f}s")
    for x in failures[:5]:
        print("  FAIL", x["reason"], json.dumps(x["program"])[:300])
    return 1 if failures else 0


def fixture_files():
    for dirpath, _, files in os.walk(FIXTURES):
        for fn in sorted(files):
            if fn.endswith(".jsonl"):
                yield os.path.join(dirpath, fn)


def cmd_fixtures(a):
    G.load_grammar()
    if a.action == "verify":
        allowed = G.ported_ops()
        rng = random.SystemRandom()
        port = Port()
        n = skipped = bad = recheck = 0
        for path in fixture_files():
            with open(path) as f:
                for lineno, line in enumerate(f, 1):
                    fx = json.loads(line)
                    if not G.ops_in(fx["program"]) <= allowed:
                        skipped += 1
                        continue
                    n += 1
                    ok, why = same(fx["expected"], port.run(fx["program"]))
                    if not ok:
                        bad += 1
                        if bad <= 10:
                            print(f"  FAIL {os.path.relpath(path, ROOT)}:{lineno} {why}")
                    if rng.random() < a.oracle_sample:
                        recheck += 1
                        exp = oracle(fx["program"], timeout=30)
                        if exp is None or not same(fx["expected"], exp)[0]:
                            bad += 1
                            print(f"  FIXTURE DISAGREES WITH ORACLE {os.path.relpath(path, ROOT)}:{lineno}")
        port.close()
        print(f"fixtures: {n} checked, {skipped} skipped (ops not ported yet), {recheck} re-checked against the "
              f"oracle, {bad} failures")
        return 1 if bad else 0
    if a.action in ("make", "import-failures"):
        dest = os.path.join(FIXTURES, a.name + ".jsonl")
        if os.path.exists(dest):
            print(f"{dest} exists: fixtures are append-only, pick a new name")
            return 2
        if a.action == "make":
            progs = [json.loads(l) for l in open(a.programs) if l.strip()]
        else:
            src = os.path.join(OUT, "failures.jsonl")
            progs = [json.loads(l)["program"] for l in open(src) if l.strip()]
        seen, rows = set(), []
        for p in progs:
            key = json.dumps(p, sort_keys=True)
            if key in seen:
                continue
            seen.add(key)
            exp = oracle(p, timeout=30)
            if exp is None:
                print("  skipped (oracle too slow):", key[:200])
                continue
            rows.append({"program": p, "expected": exp})
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "w") as f:
            for r in rows:
                f.write(json.dumps(r, separators=(",", ":")) + "\n")
        print(f"wrote {len(rows)} fixtures to {os.path.relpath(dest, ROOT)}")
        if a.action == "import-failures":
            os.rename(src, src + f".imported-{int(time.time())}")
        return 0


def cmd_single(a):
    G.load_grammar()
    prog = json.loads(a.program)
    port = Port()
    if a.cmd in ("oracle", "check"):
        exp = oracle(prog, timeout=60)
        print("oracle:", json.dumps(exp)[:4000])
    if a.cmd in ("port", "check"):
        got = port.run(prog)
        print("port:  ", json.dumps(got)[:4000])
    if a.cmd == "check":
        ok, why = same(exp, got)
        print("SAME" if ok else f"DIFFERENT: {why}")
        port.close()
        return 0 if ok else 1
    port.close()
    return 0


def cmd_ops(a):
    G.load_grammar()
    status = G.load_status()
    for op, n in sorted(G.NODES.items()):
        print(f"{status.get(op, 'MISSING'):8s} {op:20s} -> {n.returns:8s} {n.doc.splitlines()[0] if n.doc else ''}")
    missing = set(G.NODES) - set(status)
    return 1 if missing else 0


def main(argv=None):
    ap = argparse.ArgumentParser(prog="harness")
    sp = ap.add_subparsers(dest="cmd", required=True)
    for c in ("oracle", "port", "check"):
        s = sp.add_parser(c)
        s.add_argument("program")
        s.set_defaults(fn=cmd_single)
    s = sp.add_parser("fuzz")
    s.add_argument("--seed", type=int, default=1)
    s.add_argument("--count", type=int, default=1000)
    s.add_argument("--ops", default="")
    s.add_argument("--fresh", action="store_true")
    s.add_argument("--depth", type=int, default=3)
    s.add_argument("--max-failures", type=int, default=20)
    s.set_defaults(fn=cmd_fuzz)
    s = sp.add_parser("fixtures")
    s.add_argument("action", choices=["verify", "make", "import-failures"])
    s.add_argument("name", nargs="?")
    s.add_argument("programs", nargs="?")
    s.add_argument("--oracle-sample", type=float, default=0.05)
    s.set_defaults(fn=cmd_fixtures)
    s = sp.add_parser("ops")
    s.set_defaults(fn=cmd_ops)
    a = ap.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
