#!/usr/bin/env bash
# The guard. PROTECTED. Checks what an iteration changed between BASE and HEAD, independently of the gate:
# the loop cannot make checks pass by editing the checks, the oracle, or the recorded expectations.
#
#   harness/core/guard.sh BASE HEAD
set -uo pipefail
BASE="$1"; HEAD="$2"
cd "$(dirname "$0")/../.."
BAD=0
fail() { echo "GUARD: $*"; BAD=1; }

PROTECTED=(harness/core harness/__init__.py crates/icepool-harness/tests/fixtures.rs loop baselines oracle.lock
           AGENTS.md CLAUDE.md .claude rust-toolchain.toml pyproject.toml uv.lock docs/ANALYSIS.md docs/LOOP-PLAN.md docs/semantics.md docs/reviews PROGRESS.md)
changed="$(git diff --name-only "$BASE" "$HEAD" -- "${PROTECTED[@]}")"
[ -z "$changed" ] || fail "protected files changed:"$'\n'"$changed"

# Fixtures are append-only: new files only. Existing fixture files may not be modified, renamed or deleted.
touched="$(git diff --diff-filter=MDRTCU --name-only "$BASE" "$HEAD" -- fixtures)"
[ -z "$touched" ] || fail "existing fixtures modified or removed:"$'\n'"$touched"

python3 - "$BASE" "$HEAD" <<'PY' || BAD=1
import json, subprocess, sys
base, head = sys.argv[1:3]
def load(rev, path, default):
    try:
        return json.loads(subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True, text=True, check=True).stdout)
    except subprocess.CalledProcessError:
        return default
bad = False
s0, s1 = load(base, "harness/grammar/status.json", {}), load(head, "harness/grammar/status.json", {})
for op, st in s0.items():
    if op not in s1:
        print(f"GUARD: grammar node {op!r} removed from status.json"); bad = True
    elif st == "ported" and s1[op] != "ported":
        print(f"GUARD: grammar node {op!r} went from ported back to {s1[op]!r}"); bad = True
t0 = {t["id"] for t in load(base, "tasks.json", [])}
t1 = {t["id"] for t in load(head, "tasks.json", [])}
for tid in sorted(t0 - t1):
    print(f"GUARD: task {tid} removed from tasks.json"); bad = True
sys.exit(1 if bad else 0)
PY

if [ "$BAD" = 0 ]; then echo "GUARD: ok"; fi
exit $BAD
