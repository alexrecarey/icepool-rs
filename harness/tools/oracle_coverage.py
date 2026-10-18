"""Which parts of the oracle do our tests never reach? Run from the repo root:

    uv run --with coverage python -m harness.tools.oracle_coverage [--out dev/reports/coverage.md] [--fuzz SEED:COUNT:DEPTH ...]

Every fixture program, plus the programs the gate's fuzz runs generate (same seeds, counts and depths by default),
is evaluated by the oracle (Python icepool) under coverage.py with branch coverage. The report lists, per oracle
module, the statements and branches never executed, the functions never entered, and the reached functions with
the most unexecuted code. Uncovered oracle code is behaviour the port is never compared against: a grammar node
(or a generator change) that reaches it is the cheapest way to grow the checks.

Only the oracle is measured (`$ICEPOOL_ORACLE/src/icepool`), never the port. The oracle is only *read*: coverage
data goes to a temporary file outside the checkout.
"""
import argparse
import json
import os
import random
import sys
import tempfile
import time
from collections import defaultdict

try:
    import coverage
except ImportError:  # not a project dependency (pyproject.toml is the loop's): uv adds it for the run
    sys.exit("coverage.py is missing: run as `uv run --with coverage python -m harness.tools.oracle_coverage`")

import harness  # puts the oracle on sys.path (it must come before anything imports icepool)

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SRC = os.path.join(harness.ORACLE, "src", "icepool")
# Coverage tracing with branches is several times slower than plain evaluation; the fuzzer's 3 s oracle budget
# would drop programs the fuzzer itself tests. Programs over this budget are skipped and counted.
TIMEOUT_S = 15
# The gate's fuzz runs (harness/core/gate.sh): fast = seed 1 x 2000 and a khepri sweep (seed 7 x 150), long =
# seeds 11..15 x 10000 at depth 4. Fresh seeds are random, so they are not reproduced here.
DEFAULT_FUZZ = ["1:2000:3", "11:10000:4", "khepri:7:150:3"]
# Code that is never meant to run: typing overloads, abstract stubs, type-checking imports.
EXCLUDE = [
    r"@(typing\.)?overload",
    r"@(abc\.)?abstractmethod",
    r"if TYPE_CHECKING:",
    r"raise NotImplementedError",
    r"^\s*\.\.\.\s*$",
]


def fuzz_programs(spec, G):
    """The programs `tool fuzz --seed S --count N --depth D [--ops khepri]` generates, in the same order."""
    parts = spec.split(":")
    khepri = parts[0] == "khepri"
    if khepri:
        parts = parts[1:]
    seed, count, depth = int(parts[0]), int(parts[1]), int(parts[2]) if len(parts) > 2 else 3
    allowed = G.ported_ops() & ({"khepri"} if khepri else set(G.NODES))
    g = G.Gen(random.Random(seed), allowed, depth)
    # Same kind selection as tool.cmd_fuzz, so the same seed gives the same programs.
    kinds = ["die"] if khepri else sorted({G.NODES[o].returns for o in allowed if G.NODES[o].gen})
    for _ in range(count):
        try:
            yield g.make(g.rng.choice(kinds))
        except LookupError:
            continue


def fixture_programs():
    from harness.core.tool import fixture_files
    for path in fixture_files():
        with open(path) as f:
            for line in f:
                yield json.loads(line)["program"]


def run(args):
    data_file = os.path.join(tempfile.mkdtemp(prefix="oracle-cov-"), ".coverage")
    cov = coverage.Coverage(data_file=data_file, branch=True, source=[SRC], omit=[], config_file=False)
    for pattern in EXCLUDE:
        cov.exclude(pattern)
    # Start before anything imports icepool, so module-level code (definitions) counts as reached.
    cov.start()
    from harness.core import grammar_api as G
    from harness.core.tool import oracle
    G.load_grammar()
    counts = {}
    t0 = time.time()
    sources = [("fixtures", fixture_programs())] + [(f"fuzz {s}", fuzz_programs(s, G)) for s in args.fuzz]
    for name, progs in sources:
        n = slow = 0
        for prog in progs:
            n += 1
            if oracle(prog, timeout=args.timeout) is None:
                slow += 1
        counts[name] = (n, slow)
        print(f"{name}: {n} programs ({slow} over {args.timeout}s), {time.time() - t0:.0f}s", file=sys.stderr)
    cov.stop()
    cov.save()
    json_path = data_file + ".json"
    cov.json_report(outfile=json_path, show_contexts=False)
    with open(json_path) as f:
        return json.load(f), counts


def _ranges(lines):
    """[1, 2, 3, 7, 9, 10] -> '1-3, 7, 9-10'."""
    out, start, prev = [], None, None
    for x in sorted(lines):
        if start is None:
            start = prev = x
        elif x == prev + 1:
            prev = x
        else:
            out.append(f"{start}-{prev}" if prev != start else f"{start}")
            start = prev = x
    if start is not None:
        out.append(f"{start}-{prev}" if prev != start else f"{start}")
    return ", ".join(out)


def _pct(covered, total):
    return f"{100 * covered / total:.0f}%" if total else "-"


def report(cov, counts, args):
    files = {os.path.relpath(path, SRC): f for path, f in cov["files"].items()}
    t = cov["totals"]
    L = ["# Oracle coverage", "",
         "Generated by `uv run --with coverage python -m harness.tools.oracle_coverage` (do not edit by hand). Which code of the",
         "oracle (`$ICEPOOL_ORACLE/src/icepool`, pinned in `oracle.lock`) the fixtures and the gate's fuzz programs",
         "execute. Code no program reaches is behaviour the port is never compared against.", "",
         f"Oracle commit: `{_oracle_commit()}`. Coverage.py {coverage.__version__}, branch coverage, excluded: "
         "overloads, abstract methods, `raise NotImplementedError`, `...` bodies.", "",
         "| programs | count | over the time budget |", "|---|---:|---:|"]
    for name, (n, slow) in counts.items():
        L.append(f"| {name} | {n} | {slow} |")
    L += ["", f"**Total: {t['covered_lines']}/{t['num_statements']} statements "
          f"({_pct(t['covered_lines'], t['num_statements'])}), "
          f"{t['covered_branches']}/{t['num_branches']} branches ({_pct(t['covered_branches'], t['num_branches'])}).**",
          "", "## Modules, by unexecuted statements", "",
          "| module | statements | missed | covered | branches | missed branches | unreached functions |",
          "|---|---:|---:|---:|---:|---:|---:|"]
    unreached, partial = [], []
    for rel, f in files.items():
        for qual, fn in f.get("functions", {}).items():
            s = fn["summary"]
            if not qual or s["num_statements"] == 0:
                continue  # "" is the module level
            if s["covered_lines"] == 0:
                unreached.append((rel, qual, s["num_statements"], fn))
            elif s["missing_lines"] or s.get("missing_branches"):
                partial.append((rel, qual, s, fn))
    unreached_by_file = defaultdict(list)
    for rel, qual, n, fn in unreached:
        unreached_by_file[rel].append((qual, n, fn))
    for rel, f in sorted(files.items(), key=lambda kv: -kv[1]["summary"]["missing_lines"]):
        s = f["summary"]
        if not s["missing_lines"] and not s.get("missing_branches"):
            continue
        L.append(f"| `{rel}` | {s['num_statements']} | {s['missing_lines']} | {_pct(s['covered_lines'], s['num_statements'])} "
                 f"| {s['num_branches']} | {s['missing_branches']} | {len(unreached_by_file.get(rel, ()))} |")
    full = [rel for rel, f in files.items() if not f["summary"]["missing_lines"] and not f["summary"].get("missing_branches")]
    if full:
        L += ["", "Fully covered: " + ", ".join(f"`{r}`" for r in sorted(full)) + "."]

    L += ["", "## Functions never entered", "",
          "Grouped by module, biggest first (statement count in brackets). `<locals>` are nested functions.", ""]
    for rel in sorted(unreached_by_file, key=lambda r: -sum(n for _, n, _ in unreached_by_file[r])):
        fns = sorted(unreached_by_file[rel], key=lambda x: -x[1])
        total = sum(n for _, n, _ in fns)
        L.append(f"- **`{rel}`** ({total} statements): " + ", ".join(
            f"`{q}` [{n}, l.{min(fn['missing_lines'])}]" for q, n, fn in fns))

    L += ["", f"## Reached functions with the most unexecuted code (top {args.top})", "",
          "Missed statements, then missed branch arcs (`from->to`; a negative target is a function exit).", "",
          "| function | missed / statements | missed branches | unexecuted lines |", "|---|---:|---:|---|"]
    partial.sort(key=lambda x: -(x[2]["missing_lines"] + x[2].get("missing_branches", 0) / 2))
    for rel, qual, s, fn in partial[:args.top]:
        arcs = ", ".join(f"{a}->{b}" for a, b in fn.get("missing_branches", [])[:8])
        if len(fn.get("missing_branches", [])) > 8:
            arcs += ", ..."
        lines = _ranges(fn["missing_lines"]) or "-"
        L.append(f"| `{rel}` `{qual}` | {s['missing_lines']} / {s['num_statements']} | "
                 f"{s.get('missing_branches', 0)} ({arcs or '-'}) | {lines} |")
    L += ["", "## Every unexecuted line, per module", "",
          "For looking up a specific area. Line numbers are in the pinned oracle checkout.", ""]
    for rel, f in sorted(files.items()):
        if f["missing_lines"]:
            L.append(f"- `{rel}`: {_ranges(f['missing_lines'])}")
    return "\n".join(L) + "\n"


def _oracle_commit():
    try:
        with open(os.path.join(ROOT, "oracle.lock")) as f:
            for line in f:
                for tok in line.replace("=", " ").split():
                    if len(tok) == 40 and all(c in "0123456789abcdef" for c in tok):
                        return tok
    except OSError:
        pass
    return "?"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", default=os.path.join(ROOT, "dev/reports", "coverage.md"))
    ap.add_argument("--fuzz", action="append", default=None,
                    help="SEED:COUNT[:DEPTH] or khepri:SEED:COUNT; repeatable (default: the gate's fixed seeds)")
    ap.add_argument("--timeout", type=int, default=TIMEOUT_S)
    ap.add_argument("--top", type=int, default=60)
    ap.add_argument("--json", help="also write coverage.py's raw JSON report (and program counts) here")
    ap.add_argument("--from-json", help="re-render the report from a --json file instead of running programs")
    args = ap.parse_args(argv)
    if args.fuzz is None:
        args.fuzz = DEFAULT_FUZZ
    if args.from_json:
        with open(args.from_json) as f:
            saved = json.load(f)
        cov, counts = saved["coverage"], saved["counts"]
    else:
        cov, counts = run(args)
    if args.json:
        with open(args.json, "w") as f:
            json.dump({"coverage": cov, "counts": counts}, f)
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as f:
        f.write(report(cov, counts, args))
    print(f"wrote {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
