#!/usr/bin/env bash
# notify.sh TITLE [BODY]: tell Alex. PROTECTED.
#  1. a herdr notification (visible when attached to the herdr session);
#  2. a phone push through ntfy.sh to a private random topic, kept outside the repo in
#     ~/code/alexrecarey/icepool-rs-logs/ntfy-topic. (Moshi only pushes for interactive sessions, so it can't be used.)
TITLE="$1"; BODY="${2:-}"
LOGS="$HOME/code/alexrecarey/icepool-rs-logs"
echo "[$(date '+%F %T')] NOTIFY $TITLE: $BODY" >> "$LOGS/notifications.log"
herdr notification show "$TITLE" --body "$BODY" --sound request > /dev/null 2>&1 || true
TOPIC="$(cat "$LOGS/ntfy-topic" 2>/dev/null)"
if [ -n "$TOPIC" ]; then
  curl -fsS -m 20 -H "Title: $TITLE" -H "Tags: game_die" --data-binary "${BODY:-$TITLE}" \
    "https://ntfy.sh/$TOPIC" > /dev/null 2>> "$LOGS/notifications.log" || true
fi
