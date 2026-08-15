#!/usr/bin/env bash
# Deterministic contract test for the Paper runtime healthcheck.  It uses a
# fake psql and a temporary curated layout, so no Docker daemon or PostgreSQL
# instance is required.  The state PID is a live sleep process to exercise the
# wrapper's process-liveness check as well as stale/progress handling.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
wrapper="$root/deploy/runtime/paper-runner-entrypoint"
die() {
  echo "paper-runner-healthcheck-test: $*" >&2
  exit 1
}

work=$(mktemp -d)
runner_pid=''
cleanup() {
  if [ -n "$runner_pid" ]; then
    kill "$runner_pid" 2>/dev/null || true
    wait "$runner_pid" 2>/dev/null || true
  fi
  rm -rf -- "$work"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$work/bin" \
  "$work/dataset/curated/datasets/example/version=2" \
  "$work/dataset/curated/bars/market=kr/example/version=2"
printf '#!/bin/sh\nprintf "t\\n"\n' >"$work/bin/psql"
chmod 0755 "$work/bin/psql"
printf '%s\n' '{"dataset_id":"example","version":2,"source_batches":[],"bar_count":1,"action_count":0,"content_hash":"0000000000000000000000000000000000000000000000000000000000000000"}' \
  >"$work/dataset/curated/datasets/example/version=2/manifest.json"
printf 'fixture\n' >"$work/dataset/curated/bars/market=kr/example/version=2/bars.parquet"

sleep 60 &
runner_pid=$!
export APP_ENV=development
export DATABASE_URL=postgresql://fixture:fixture@localhost/fixture
export WORKER_DATABASE_URL="$DATABASE_URL"
export ADMIN_DATABASE_URL="$DATABASE_URL"
export AUDIT_DATABASE_URL="$DATABASE_URL"
export LAGRANGE_DATASET_ROOT="$work/dataset"
export PAPER_HEALTH_STATE_PATH="$work/health.json"
export PAPER_HEALTH_MAX_AGE_SECS=30
export PATH="$work/bin:$PATH"

python3 - "$PAPER_HEALTH_STATE_PATH" "$runner_pid" <<'PY'
import datetime as dt
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
now = dt.datetime.now(dt.timezone.utc)
state = {
    "pid": int(sys.argv[2]),
    "heartbeat_at": now.isoformat().replace("+00:00", "Z"),
    "last_progress_at": now.isoformat().replace("+00:00", "Z"),
    "phase": "cycle_completed",
    "cycle_id": 1,
    "cycle_in_progress": False,
    "cycle_started_at": None,
    "cycle_deadline_at": None,
    "last_cycle_completed_at": now.isoformat().replace("+00:00", "Z"),
    "last_cycle_outcome": "succeeded",
}
path.write_text(json.dumps(state), encoding="utf-8")
PY

"$wrapper" healthcheck >/dev/null

# An active cycle may have an old phase update while its explicit bounded
# deadline is still valid.  This must remain healthy.
python3 - "$PAPER_HEALTH_STATE_PATH" <<'PY'
import datetime as dt
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
state = json.loads(path.read_text(encoding="utf-8"))
now = dt.datetime.now(dt.timezone.utc)
state["cycle_in_progress"] = True
state["cycle_started_at"] = (now - dt.timedelta(seconds=120)).isoformat().replace("+00:00", "Z")
state["cycle_deadline_at"] = (now + dt.timedelta(seconds=20)).isoformat().replace("+00:00", "Z")
state["last_progress_at"] = (now - dt.timedelta(seconds=120)).isoformat().replace("+00:00", "Z")
path.write_text(json.dumps(state), encoding="utf-8")
PY
"$wrapper" healthcheck >/dev/null

# An idle loop with no progress for longer than the configured window must be
# unhealthy even though the database and dataset probes still succeed.
python3 - "$PAPER_HEALTH_STATE_PATH" <<'PY'
import datetime as dt
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
state = json.loads(path.read_text(encoding="utf-8"))
stale = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(seconds=120)).isoformat().replace("+00:00", "Z")
state["cycle_in_progress"] = False
state["cycle_started_at"] = None
state["cycle_deadline_at"] = None
state["last_progress_at"] = stale
path.write_text(json.dumps(state), encoding="utf-8")
PY
if "$wrapper" healthcheck >/dev/null 2>&1; then
  die 'stale idle progress was accepted'
fi

echo 'PAPER_RUNNER_HEALTHCHECK: PASS'
