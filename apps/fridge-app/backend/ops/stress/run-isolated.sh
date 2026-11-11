#!/bin/bash
# Start a throwaway backend for load testing. NEVER point load at the real fridge.db or :8080.
#
#   ops/stress/run-isolated.sh WORKDIR [BINARY]
#
# What makes it safe, and why each line is there:
#   - `sqlite3 .backup`, not `cp`: the live database is in WAL mode, and a plain copy of a file
#     another process is writing can be torn.
#   - Credentials are deleted from the copy: Gmail refresh tokens, sessions and extension
#     tokens have no business in a database that is about to receive hostile traffic.
#   - `env -i` from WORKDIR: no `.env` is in reach, so Google OAuth never loads and nothing
#     can talk to Gmail even if a job were left on.
#   - Every background job is switched off: the collector would scrape third-party job boards,
#     and all of them would contend for SQLite and corrupt the measurements.
#   - PORT=18080: the developer's own backend keeps :8080.
#   - BIND_ADDRESS=127.0.0.1: the copy still holds your applications and email addresses, and
#     the default is every interface, which would serve them to the LAN for the length of a run.
#
# Never call POST /internships/collect against it: that one performs a real scrape.
set -euo pipefail
WORKDIR=${1:?usage: run-isolated.sh WORKDIR [BINARY]}
HERE=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${2:-$HERE/target/release/fridge_backend}
mkdir -p "$WORKDIR"
if [ ! -f "$WORKDIR/stress.db" ]; then
  sqlite3 "$HERE/fridge.db" ".backup '$WORKDIR/stress.db'"
  sqlite3 "$WORKDIR/stress.db" "DELETE FROM gmail_accounts; DELETE FROM sessions; DELETE FROM hunt_tokens;"
fi
pid=$(lsof -nP -iTCP:18080 -sTCP:LISTEN -t || true)
[ -n "$pid" ] && kill "$pid" && while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
cd "$WORKDIR"
env -i HOME="$HOME" PATH="$PATH" \
  DATABASE_URL="sqlite://$WORKDIR/stress.db?mode=rwc" PORT=18080 BIND_ADDRESS=127.0.0.1 \
  BLOG_SYNC_INTERVAL_SECS=0 INTERNSHIP_COLLECT_INTERVAL_SECS=0 INTERNSHIP_EXPIRY_INTERVAL_SECS=0 \
  INBOX_SYNC_INTERVAL_SECS=0 INBOX_APPLY_LABELS=false \
  HUNT_NUDGE_INTERVAL_SECS=0 HUNT_DEADLINE_INTERVAL_SECS=0 \
  nohup "$BIN" > server.log 2>&1 &
until curl -sf -o /dev/null http://127.0.0.1:18080/health; do sleep 0.3; done
grep -E "OAuth|disabled|off|listening" server.log
