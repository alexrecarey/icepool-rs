#!/usr/bin/env bash
# The porting loop. PROTECTED. Runs forever in a herdr pane on carapace:
#
#   loop/loop.sh            run until everything is done or blocked
#   loop/loop.sh --once     one task (dry runs)
#
# One task = one fresh `claude -p` in its own git worktree. The script, not the agent, runs guard + gate, gets a
# second agent to review, merges, pushes, and notifies. See docs/LOOP-PLAN.md.
set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WTROOT="$REPO-wt"
LOGS="$HOME/code/alexrecarey/icepool-rs-logs/iterations"
STATUS_FILE="$HOME/code/alexrecarey/icepool-rs-logs/STATUS"
export ICEPOOL_ORACLE="$HOME/code/alexrecarey/icepool-oracle"
export CARGO_TARGET_DIR="$HOME/.cache/icepool-rs/target"
MODEL="${LOOP_MODEL:-claude-opus-5-5}"
MAX_ATTEMPTS=3
LONG_EVERY=10
IMPL_TIMEOUT=5h
REVIEW_TIMEOUT=1h
LIMIT_SLEEP=1800
ONCE=0; [ "${1:-}" = "--once" ] && ONCE=1

# Fresh agents: no user-level hooks (no phone push per iteration, no claude-sync, no build offloading). The project's
# .claude/settings.json adds the destructive-command guard back.
CLAUDE=(claude -p --dangerously-skip-permissions --setting-sources project,local --model "$MODEL")

mkdir -p "$LOGS" "$WTROOT"
cd "$REPO"
# One loop at a time (the watchdog may try to start another).
exec 9> "$LOGS/../loop.lock"
flock -n 9 || { echo "another loop.sh is already running"; exit 0; }
T() { python3 loop/tasks.py "$@"; }
log() { echo "[$(date '+%F %T')] $*" | tee -a "$LOGS/loop.log"; }
status() { echo "$(date '+%F %T') $*" > "$STATUS_FILE"; }
notify() { "$REPO/loop/notify.sh" "$@" || true; }

commit_main() {  # commit_main MESSAGE: commit loop bookkeeping on main and push
  git add -A tasks.json loop/state.json PROGRESS.md baselines 2>/dev/null
  git commit -q -m "$1" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" || true
  push_main
}

push_main() {
  for i in 1 2 3; do git push -q origin main 2>>"$LOGS/push.log" && return 0; sleep 30; done
  log "push failed 3 times (see push.log); continuing locally"
}

# Did the run end on a usage limit (or an API outage) rather than on the agent's own terms?
hit_limit() {  # hit_limit LOGFILE
  python3 - "$1" <<'PY'
import json, re, sys
last = None
for line in open(sys.argv[1], errors="replace"):
    try:
        j = json.loads(line)
    except ValueError:
        continue
    if j.get("type") == "result":
        last = j
if last is None:
    text = open(sys.argv[1], errors="replace").read()[-3000:]
    sys.exit(0 if re.search(r"usage limit|rate.?limit|limit reached|overloaded|529|ECONNRESET|credit", text, re.I) else 1)
text = str(last.get("result", "")) + " " + str(last.get("subtype", ""))
sys.exit(0 if last.get("is_error") and re.search(r"usage limit|rate.?limit|limit reached|overloaded|529|credit", text, re.I) else 1)
PY
}

result_text() {  # the final text of a stream-json run
  python3 - "$1" <<'PY'
import json, sys
last = ""
for line in open(sys.argv[1], errors="replace"):
    try:
        j = json.loads(line)
    except ValueError:
        continue
    if j.get("type") == "result":
        last = str(j.get("result", ""))
print(last)
PY
}

long_gate() {
  local base; base="$(T meta last_long)"
  log "long gate on main (mutants since ${base:-none})"
  status "long gate"
  if MUTANTS_BASE="$base" harness/core/gate.sh long > "$LOGS/long-$(date +%Y%m%d-%H%M).log" 2>&1; then
    log "long gate GREEN"
  else
    local n; n="$(date +%Y%m%d%H%M)"
    T add-front "L$n" "Fix the long gate" "The long gate went red on main. Log: $(ls -t "$LOGS"/long-*.log | head -1). Failing steps: $(grep -- '--- .*FAILED' "$(ls -t "$LOGS"/long-*.log | head -1)" | tr '\n' ' '). Reproduce, fix the port (not the checks), import any fuzz failures as fixtures."
    log "long gate RED: added task L$n"
  fi
  local missed="harness-out/mutants.out/missed.txt"
  if [ -s "$missed" ]; then
    local n; n="$(date +%Y%m%d%H%M)"
    T add-front "M$n" "Kill surviving mutants" "cargo-mutants found mutations no check catches (harness-out/mutants.out/missed.txt on carapace, copied here): $(head -40 "$missed" | tr '\n' ';'). For each: add fixtures (programs computed by the oracle) or grammar/fuzzer coverage that would fail on that mutation. Do not change the code under test unless a mutant reveals a real bug."
    log "mutants survived: added task M$n"
  fi
  T meta last_long "$(git rev-parse HEAD)"
  commit_main "loop: long gate"
}

while :; do
  git checkout -q main
  ID="$(T next)"
  if [ "$ID" = NONE ]; then log "all tasks done"; notify "icepool-rs: all tasks done" "$(T summary)"; status "done"; exit 0; fi
  if [ "$ID" = BLOCKED ]; then log "only blocked tasks left"; notify "icepool-rs: stopped, all remaining tasks blocked" "$(T summary)"; status "stopped: blocked"; exit 1; fi

  WT="$WTROOT/$ID"
  if [ ! -d "$WT" ]; then
    git worktree add -q -B "task/$ID" "$WT" main
    mkdir -p "$WT/.loop"
    git rev-parse main > "$WT/.loop/base"
  fi
  BASE="$(cat "$WT/.loop/base")"
  ATTEMPT=$(( $(python3 -c "import json;print(json.load(open('loop/state.json'))['tasks'].get('$ID',{}).get('attempts',0))") + 1 ))
  TAG="$ID-a$ATTEMPT-$(date +%m%d-%H%M)"
  log "task $ID attempt $ATTEMPT: $(T field "$ID" title)"
  status "task $ID attempt $ATTEMPT: implementing (log $LOGS/$TAG-impl.jsonl)"

  (cd "$WT" && uv sync -q) > /dev/null 2>&1
  PROMPT="$(cat loop/prompts/implement.md)

Your task: $ID: $(T field "$ID" title)
$(T field "$ID" details)

This is attempt $ATTEMPT of $MAX_ATTEMPTS."
  (cd "$WT" && timeout "$IMPL_TIMEOUT" "${CLAUDE[@]}" --output-format stream-json --verbose "$PROMPT") > "$LOGS/$TAG-impl.jsonl" 2>&1
  if hit_limit "$LOGS/$TAG-impl.jsonl"; then
    log "usage limit or API trouble; sleeping $((LIMIT_SLEEP / 60)) min (attempt not counted)"
    status "sleeping on usage limit since $(date '+%T')"
    sleep "$LIMIT_SLEEP"
    continue
  fi

  # Leftovers get committed so the checks see exactly what the agent left behind.
  if [ -n "$(git -C "$WT" status --porcelain)" ]; then
    git -C "$WT" add -A
    git -C "$WT" commit -q -m "loop: uncommitted changes at the end of $TAG" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
  fi
  HEAD_SHA="$(git -C "$WT" rev-parse HEAD)"
  REPORT="$WT/.loop/last-failure.md"
  VERDICT=""

  if [ "$HEAD_SHA" = "$BASE" ]; then
    VERDICT="no changes were made"
  else
    status "task $ID attempt $ATTEMPT: guard + gate"
    # The guard runs from main's copy, so a branch can't weaken it.
    if ! bash "$REPO/harness/core/guard.sh" "$BASE" "$HEAD_SHA" > "$LOGS/$TAG-guard.log" 2>&1; then
      VERDICT="guard: $(grep GUARD "$LOGS/$TAG-guard.log" | head -5 | tr '\n' ' ')"
    elif ! (cd "$WT" && harness/core/gate.sh fast) > "$LOGS/$TAG-gate.log" 2>&1; then
      VERDICT="gate: $(tail -1 "$LOGS/$TAG-gate.log")"
    else
      status "task $ID attempt $ATTEMPT: review"
      (cd "$WT" && timeout "$REVIEW_TIMEOUT" "${CLAUDE[@]}" --output-format stream-json --verbose \
        "$(cat loop/prompts/review.md)

BASE=$BASE  HEAD=$HEAD_SHA  task $ID: $(T field "$ID" title)") > "$LOGS/$TAG-review.jsonl" 2>&1
      if hit_limit "$LOGS/$TAG-review.jsonl"; then
        log "usage limit during review; sleeping (attempt not counted)"; sleep "$LIMIT_SLEEP"; continue
      fi
      result_text "$LOGS/$TAG-review.jsonl" > "$LOGS/$TAG-review.txt"
      # Tolerant of markdown around the verdict line (**VERDICT: APPROVE**, indentation, ...).
      LAST_VERDICT="$(grep -iE 'VERDICT:?[[:space:]*_]*(APPROVE|REJECT)' "$LOGS/$TAG-review.txt" | tail -1)"
      if echo "$LAST_VERDICT" | grep -qiE 'VERDICT:?[[:space:]*_]*APPROVE'; then
        VERDICT=APPROVE
      else
        VERDICT="review: $LAST_VERDICT"
        [ "$VERDICT" = "review: " ] && VERDICT="review: no verdict (see $TAG-review.txt)"
      fi
    fi
  fi

  if [ "$VERDICT" = APPROVE ]; then
    # Drop anything the reviewer may have touched: merge exactly what was checked.
    git -C "$WT" reset -q --hard "$HEAD_SHA" && git -C "$WT" clean -fdq -e .venv -e .loop -e harness-out
    # Rebase onto main (only loop bookkeeping can have landed there), then fast-forward.
    if git -C "$WT" rebase -q main && git merge -q --ff-only "task/$ID"; then
      T status "$ID" done
      SUMMARY="$(result_text "$LOGS/$TAG-impl.jsonl" | tr '\n' ' ' | cut -c1-300)"
      echo "- $(date '+%F %H:%M') **$ID** done (attempt $ATTEMPT, $(git rev-list --count "$BASE..HEAD") commits): $SUMMARY" >> PROGRESS.md
      [ -f "$WT/harness-out/upstream-pass.txt" ] && sort -u "$WT/harness-out/upstream-pass.txt" baselines/upstream-pass.txt 2>/dev/null -o baselines/upstream-pass.txt
      commit_main "loop: $ID done"
      git worktree remove --force "$WT"; git branch -q -D "task/$ID"
      log "task $ID merged"
      MS="$(T field "$ID" milestone)"
      [ -n "$MS" ] && notify "icepool-rs milestone: $MS" "Task $ID merged. $(T summary)"
      M="$(T merged)"; git add loop/state.json; git commit -q -m "loop: merge count" || true
      [ $((M % LONG_EVERY)) -eq 0 ] && long_gate
    else
      git -C "$WT" rebase --abort 2>/dev/null
      VERDICT="merge: could not rebase onto main"
    fi
  fi

  if [ "$VERDICT" != APPROVE ]; then
    N="$(T bump "$ID")"
    {
      echo "# Attempt $ATTEMPT of $ID failed"
      echo
      echo "**Why:** $VERDICT"
      echo
      echo "Your previous work is still on this branch. Fix the problem, keep what was good."
      for f in guard gate; do
        [ -f "$LOGS/$TAG-$f.log" ] && { echo; echo "## $f output (tail)"; echo '```'; tail -60 "$LOGS/$TAG-$f.log"; echo '```'; }
      done
      [ -f "$LOGS/$TAG-review.txt" ] && { echo; echo "## Reviewer"; cat "$LOGS/$TAG-review.txt"; }
    } > "$REPORT"
    if [ "$N" -ge "$MAX_ATTEMPTS" ]; then
      T status "$ID" blocked
      echo "- $(date '+%F %H:%M') **$ID** blocked after $N attempts: $VERDICT" >> PROGRESS.md
      notify "icepool-rs: task $ID blocked" "$VERDICT"
      log "task $ID blocked: $VERDICT"
    else
      echo "- $(date '+%F %H:%M') $ID attempt $N failed: $VERDICT" >> PROGRESS.md
      log "task $ID attempt $N failed: $VERDICT"
    fi
    commit_main "loop: $ID attempt $N failed"
  fi

  [ "$ONCE" = 1 ] && { status "stopped: after --once"; exit 0; }
done
