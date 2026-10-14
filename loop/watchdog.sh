#!/usr/bin/env bash
# Watchdog, run by cron every 20 minutes and at boot. PROTECTED.
# If the loop isn't running and didn't stop on purpose (STATUS "done" / "stopped: ..."), start it again in the herdr
# workspace "icepool-rs" (creating the workspace if herdr lost it) and tell Alex.
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
REPO="$HOME/code/alexrecarey/icepool-rs"
LOGS="$HOME/code/alexrecarey/icepool-rs-logs"
pgrep -f "loop/loop.sh" > /dev/null && exit 0
case "$(cut -d' ' -f3- "$LOGS/STATUS" 2>/dev/null)" in done*|stopped*) exit 0 ;; esac
for i in $(seq 1 30); do herdr status server 2>/dev/null | grep -q "status: running" && break; sleep 10; done
WS="$(herdr workspace list 2>/dev/null | python3 -c "
import json,sys
try:
    for w in json.load(sys.stdin)['result']['workspaces']:
        if w.get('label') == 'icepool-rs': print(w['workspace_id']); break
except Exception: pass")"
if [ -z "$WS" ]; then
  PANE="$(herdr workspace create --cwd "$REPO" --label icepool-rs --no-focus | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['root_pane']['pane_id'])")"
else
  PANE="$(herdr pane list --workspace "$WS" | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['panes'][0]['pane_id'])")"
fi
herdr pane run "$PANE" "loop/loop.sh 2>&1 | tee -a $LOGS/console.log" > /dev/null
echo "$PANE" > "$LOGS/PANE"
echo "[$(date '+%F %T')] watchdog restarted the loop in $PANE" >> "$LOGS/iterations/loop.log"
"$REPO/loop/notify.sh" "icepool-rs: loop restarted by the watchdog" "It was not running (reboot or crash). Pane $PANE."
