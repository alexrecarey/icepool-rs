#!/usr/bin/env bash
# notify.sh TITLE [BODY]: tell Alex. PROTECTED.
#  1. a herdr notification (visible when attached to the herdr session);
#  2. a phone push through Moshi: a throwaway Claude run *with* the user-level hooks, whose Stop hook pushes the
#     reply to the paired phone. (The loop's own agents run without user hooks so they don't push.)
TITLE="$1"; BODY="${2:-}"
echo "[$(date '+%F %T')] NOTIFY $TITLE: $BODY" >> "$HOME/code/alexrecarey/icepool-rs-logs/notifications.log"
herdr notification show "$TITLE" --body "$BODY" --sound request > /dev/null 2>&1 || true
cd "$HOME" && timeout 120 claude -p --model claude-haiku-5-5 \
  "Reply with exactly this text and nothing else: $TITLE. $BODY" > /dev/null 2>&1 || true
