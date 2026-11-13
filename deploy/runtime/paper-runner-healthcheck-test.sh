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
native_helper="$work/bin/owner-beta-paper-ops-native-bin"
test_wrapper="$work/bin/paper-runner-entrypoint"
LAGRANGE_CODE_COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
write_elf_fixture() {
  python3 - "$@" <<'PY'
import struct
import sys

path, commit, section_name, flags = sys.argv[1:]
marker = commit.encode("ascii")
names = b"\x00" + section_name.encode("ascii") + b"\x00.shstrtab\x00"
section_name_offset = 1
strings_name_offset = names.index(b".shstrtab")
section_table_offset = 64
data_offset = section_table_offset + 3 * 64
strings_offset = data_offset + len(marker)
result = bytearray(strings_offset + len(names))
result[:16] = b"\x7fELF\x02\x01\x01" + bytes(9)
struct.pack_into("<HHIQQQIHHHHHH", result, 16,
                 2, 62, 1, 0, 0, section_table_offset, 0, 64, 0, 0, 64, 3, 2)
struct.pack_into("<IIQQQQIIQQ", result, section_table_offset + 64,
                 section_name_offset, 1, int(flags), 0, data_offset, len(marker), 0, 0, 1, 0)
struct.pack_into("<IIQQQQIIQQ", result, section_table_offset + 128,
                 strings_name_offset, 3, 0, 0, strings_offset, len(names), 0, 0, 1, 0)
result[data_offset:data_offset + len(marker)] = marker
result[strings_offset:] = names
with open(path, "wb") as output:
    output.write(result)
PY
}
write_elf_fixture "$native_helper" "$LAGRANGE_CODE_COMMIT" .rodata 2
chmod 0755 "$native_helper"
# This copy changes only the fixed image path and owner check for the local
# fixture. It does not weaken production validation or establish installed-ELF evidence.
sed -e "s|/usr/local/bin/owner-beta-paper-ops-native-bin|$native_helper|g" \
    -e 's/before.st_uid != 0/before.st_uid != os.geteuid()/g' \
    "$wrapper" >"$test_wrapper"
chmod 0755 "$test_wrapper"
printf '#!/bin/sh\nprintf "t\\n"\n' >"$work/bin/psql"
chmod 0755 "$work/bin/psql"
printf '%s\n' '{"dataset_id":"example","version":2,"source_batches":[],"bar_count":1,"action_count":0,"content_hash":"0000000000000000000000000000000000000000000000000000000000000000"}' \
  >"$work/dataset/curated/datasets/example/version=2/manifest.json"
printf 'fixture\n' >"$work/dataset/curated/bars/market=kr/example/version=2/bars.parquet"

sleep 60 &
runner_pid=$!
export APP_ENV=development
export LAGRANGE_CODE_COMMIT
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

"$test_wrapper" healthcheck >/dev/null

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
"$test_wrapper" healthcheck >/dev/null

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
if "$test_wrapper" healthcheck >/dev/null 2>&1; then
  die 'stale idle progress was accepted'
fi

# Runtime health rejects a helper whose actual bytes do not contain the exact
# release revision even if the file remains executable.
python3 - "$PAPER_HEALTH_STATE_PATH" <<'PY'
import datetime as dt
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
state = json.loads(path.read_text(encoding="utf-8"))
now = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
state["last_progress_at"] = now
state["heartbeat_at"] = now
path.write_text(json.dumps(state), encoding="utf-8")
PY
write_elf_fixture "$native_helper" "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" .rodata 2
if "$test_wrapper" healthcheck >/dev/null 2>&1; then
  die 'native helper with wrong compiled revision was accepted'
fi
write_elf_fixture "$native_helper" "$LAGRANGE_CODE_COMMIT" .text 6
if "$test_wrapper" healthcheck >/dev/null 2>&1; then
  die 'native helper with a non-rodata revision string was accepted'
fi
printf 'NOT-ELF%s' "$LAGRANGE_CODE_COMMIT" >"$native_helper"
if "$test_wrapper" healthcheck >/dev/null 2>&1; then
  die 'non-ELF native helper was accepted'
fi

echo 'PAPER_RUNNER_HEALTHCHECK: PASS'
