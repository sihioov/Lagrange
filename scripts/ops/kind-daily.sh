#!/usr/bin/env bash
# Owner-approved, one-calendar-day KIND evidence orchestration.
#
# The only network-capable step is the existing Playwright browser-control
# stage.  This wrapper never constructs a KIND request, probes a popup, parses
# provider HTML, or follows a correction relationship.  It keeps the browser
# staging private until the existing Rust Raw validators and normalizers have
# accepted it.  The default mode is a no-change plan.
set -euo pipefail
umask 077

script_dir=$(cd "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
default_repo_root=$(cd "$script_dir/../.." && pwd)
repo_root=${KIND_DAILY_REPO_ROOT:-$default_repo_root}
capture_root=${KIND_DAILY_CAPTURE_ROOT:-$repo_root/data-pipelines/kind-capture}
node_bin=${KIND_DAILY_NODE_BIN:-node}
kind_raw_bin=${KIND_DAILY_KIND_RAW_BIN:-$repo_root/target/release/kind-raw}
kind_correction_raw_bin=${KIND_DAILY_KIND_CORRECTION_RAW_BIN:-$repo_root/target/release/kind-correction-raw}
kind_normalize_bin=${KIND_DAILY_NORMALIZE_BIN:-$repo_root/target/release/kind-normalize}
raw_root=${KIND_DAILY_RAW_ROOT:-/var/lib/lagrange/data}
production_state_root=/var/lib/lagrange/state/kind-daily
state_root=$production_state_root
if [ "${KIND_DAILY_TEST_MODE:-0}" = 1 ]; then
  state_root=${KIND_DAILY_TEST_STATE_ROOT:-$production_state_root}
fi
capture_script=$capture_root/capture.mjs
correction_capture_script=$capture_root/capture-correction.mjs
run_dir=
cleanup_success=0

mode=plan
mode_seen=0
target_date_file=
operator_confirmation=
confirmation_seen=0
required_confirmation=KIND_DAILY_OPERATOR_CONFIRMATION

usage() {
  cat <<'EOF'
Usage: kind-daily.sh [--plan|--check|--execute] [--confirm KIND_DAILY_OPERATOR_CONFIRMATION]
                     [--target-date-file ABSOLUTE_PATH]

The date is exactly one validated calendar day. Without --target-date-file the
current Asia/Seoul date is used. A date file contains one YYYY-MM-DD value.
The correction candidate file is the date-specific private state file:
  /var/lib/lagrange/state/kind-daily/candidates/YYYY-MM-DD.txt
It contains zero to five sorted, unique, opaque 14-ASCII-digit acceptance
numbers; an empty file means no correction viewer capture for that day.

--plan and --check never launch Node, touch the browser, acquire the run lock,
or write staging/Raw/normalized data. --execute also requires the exact fixed
operator confirmation literal KIND_DAILY_OPERATOR_CONFIRMATION.
EOF
}

die() {
  local reason=$1
  printf 'KIND_DAILY status=incomplete reason=%s\n' "$reason" >&2
  if [ -n "${run_dir:-}" ]; then
    printf 'KIND_DAILY staging_retained=private\n' >&2
  fi
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--check|--execute)
      [ "$mode_seen" -eq 0 ] || die 'multiple_modes'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    --target-date-file)
      [ "$#" -ge 2 ] || die 'target_date_file_value_missing'
      [ -z "$target_date_file" ] || die 'target_date_file_repeated'
      target_date_file=$2
      shift 2
      ;;
    --confirm)
      [ "$#" -ge 2 ] || die 'confirmation_value_missing'
      [ "$confirmation_seen" -eq 0 ] || die 'confirmation_repeated'
      operator_confirmation=$2
      confirmation_seen=1
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die 'unknown_argument'
      ;;
  esac
done

if [ "$mode" = execute ]; then
  [ "$operator_confirmation" = "$required_confirmation" ] ||
    die 'execute_confirmation_required'
else
  [ "$confirmation_seen" -eq 0 ] || die 'confirmation_only_for_execute'
fi

safe_absolute_path() {
  local path=$1
  case "$path" in
    /*) ;;
    *) die 'path_not_absolute' ;;
  esac
  case "$path" in
    */../*|*/..|../*|..|*$'\n'*|*$'\r'*) die 'unsafe_path_shape' ;;
  esac

  local probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die 'path_traverses_symlink'
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

require_regular_file() {
  local path=$1
  [ -f "$path" ] && [ ! -L "$path" ] || die 'required_file_missing_or_unsafe'
}

require_private_dir() {
  local path=$1 mode_bits
  [ -d "$path" ] && [ ! -L "$path" ] || die 'required_directory_missing_or_unsafe'
  require_root_owned_no_write "$path"
  mode_bits=$(stat -c '%a' -- "$path" 2>/dev/null) || die 'directory_metadata_unreadable'
  (( (8#$mode_bits & 0077) == 0 )) || die 'directory_not_private'
}

require_root_owned_no_write() {
  local path=$1 metadata uid mode_bits
  [ -e "$path" ] && [ ! -L "$path" ] || die 'required_path_missing_or_unsafe'
  metadata=$(stat -c '%u:%a' -- "$path" 2>/dev/null) || die 'path_metadata_unreadable'
  uid=${metadata%%:*}
  mode_bits=${metadata#*:}
  [ "$uid" = 0 ] || die 'path_not_root_owned'
  (( (8#$mode_bits & 0022) == 0 )) || die 'path_group_or_other_writable'
}

require_root_owned_parent_chain() {
  local path=$1 probe
  probe=$(dirname -- "$path")
  while :; do
    [ ! -L "$probe" ] || die 'path_traverses_symlink'
    if [ -e "$probe" ]; then
      [ -d "$probe" ] || die 'path_parent_not_directory'
      require_root_owned_no_write "$probe"
    fi
    [ "$probe" = / ] && break
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

require_raw_root() {
  local path=$1
  [ -d "$path" ] && [ ! -L "$path" ] || die 'raw_directory_missing_or_unsafe'
  require_root_owned_no_write "$path"
  # The production Raw root is root:lagrange 0750.  It is not itself the
  # private staging area; only group/other write access is forbidden here.
}

require_private_file() {
  local path=$1 mode_bits
  require_regular_file "$path"
  require_root_owned_no_write "$path"
  mode_bits=$(stat -c '%a' -- "$path" 2>/dev/null) || die 'file_metadata_unreadable'
  (( (8#$mode_bits & 0077) == 0 )) || die 'file_not_private'
}

require_command() {
  local command_name=$1
  if [[ "$command_name" == */* ]]; then
    [ -x "$command_name" ] && [ ! -L "$command_name" ] || die 'required_command_missing'
  else
    command -v "$command_name" >/dev/null 2>&1 || die 'required_command_missing'
  fi
}

read_target_date() {
  local value
  if [ -n "$target_date_file" ]; then
    safe_absolute_path "$target_date_file"
    require_regular_file "$target_date_file"
    if ! value=$(python3 - "$target_date_file" 2>/dev/null <<'PY'
import datetime as dt
import pathlib
import re
import sys

data = pathlib.Path(sys.argv[1]).read_bytes()
if data.endswith(b"\n"):
    data = data[:-1]
if b"\n" in data or b"\r" in data:
    raise SystemExit(1)
try:
    value = data.decode("ascii")
except UnicodeDecodeError:
    raise SystemExit(1)
if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
    raise SystemExit(1)
try:
    dt.date.fromisoformat(value)
except ValueError:
    raise SystemExit(1)
sys.stdout.write(value)
PY
    ); then
      die 'target_date_file_invalid'
    fi
  else
    value=$(TZ=Asia/Seoul date +%F) || die 'current_kst_date_unavailable'
  fi
  if ! python3 - "$value" >/dev/null 2>&1 <<'PY'
import datetime as dt
import re
import sys

value = sys.argv[1]
if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
    raise SystemExit(1)
dt.date.fromisoformat(value)
PY
  then
    die 'target_date_invalid'
  fi
  printf '%s' "$value"
}

validate_candidate_file() {
  local path=$1
  if ! python3 - "$path" 2>/dev/null <<'PY'
import pathlib
import re
import sys

data = pathlib.Path(sys.argv[1]).read_bytes()
if len(data) > 4096:
    raise SystemExit(1)
if not data:
    print(0)
    raise SystemExit(0)
if not data.endswith(b"\n"):
    raise SystemExit(1)
try:
    text = data.decode("ascii")
except UnicodeDecodeError:
    raise SystemExit(1)
values = text.split("\n")[:-1]
if not values or any(not re.fullmatch(r"[0-9]{14}", value) for value in values):
    raise SystemExit(1)
if len(values) > 5 or len(values) != len(set(values)) or values != sorted(values):
    raise SystemExit(1)
print(len(values))
PY
  then
    return 1
  fi
}

check_dependencies() {
  require_command python3
  require_command flock
  require_command mktemp
  require_command stat
  require_command cp
  require_command chmod
  require_command mkdir
  require_command rm
  require_command date
  require_command "$node_bin"
  require_regular_file "$capture_script"
  require_regular_file "$correction_capture_script"
  require_command "$kind_raw_bin"
  require_command "$kind_correction_raw_bin"
  require_command "$kind_normalize_bin"
}

check_static_inputs() {
  local candidate_policy=${1:-required}
  safe_absolute_path "$repo_root"
  safe_absolute_path "$capture_root"
  safe_absolute_path "$capture_script"
  safe_absolute_path "$correction_capture_script"
  safe_absolute_path "$kind_raw_bin"
  safe_absolute_path "$kind_correction_raw_bin"
  safe_absolute_path "$kind_normalize_bin"
  safe_absolute_path "$raw_root"
  safe_absolute_path "$state_root"
  safe_absolute_path "$state_root/candidates"
  safe_absolute_path "$candidate_file"
  require_regular_file "$capture_script"
  require_regular_file "$correction_capture_script"
  require_raw_root "$raw_root"
  require_root_owned_parent_chain "$state_root"
  require_root_owned_parent_chain "$state_root/candidates"
  require_private_dir "$state_root"
  require_private_dir "$state_root/candidates"
  if [ "$candidate_policy" = required ]; then
    require_private_file "$candidate_file"
  fi
  [ -n "${KIND_DAILY_ENTITLEMENT_REFERENCE:-}" ] || die 'entitlement_reference_missing'
  case "${KIND_DAILY_ENTITLEMENT_REFERENCE}" in
    *$'\n'*|*$'\r'*) die 'entitlement_reference_has_line_break' ;;
  esac
  check_dependencies
}

validate_list_capture() {
  local staging=$1 expected_date=$2 page_count
  if ! page_count=$(python3 - "$staging" "$expected_date" 2>/dev/null <<'PY'
import json
import pathlib
import sys

staging = pathlib.Path(sys.argv[1])
expected_date = sys.argv[2]
if not staging.is_dir() or staging.is_symlink():
    raise SystemExit(1)
metadata = staging / "capture.json"
if not metadata.is_file() or metadata.is_symlink() or metadata.stat().st_size > 65536:
    raise SystemExit(1)
try:
    capture = json.loads(metadata.read_text(encoding="utf-8"))
except (OSError, UnicodeError, json.JSONDecodeError):
    raise SystemExit(1)
if not isinstance(capture, dict):
    raise SystemExit(1)
if capture.get("source") != "kind.krx.co.kr":
    raise SystemExit(1)
if capture.get("entry_url") != "https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf":
    raise SystemExit(1)
if capture.get("surface") != "etf-disclosure-list":
    raise SystemExit(1)
requested = capture.get("requested_range")
if not isinstance(requested, dict) or requested.get("from") != expected_date or requested.get("to") != expected_date:
    raise SystemExit(1)
if capture.get("termination") != "clamped_duplicate":
    raise SystemExit(1)
pages = capture.get("pages")
if not isinstance(pages, list) or not 1 <= len(pages) <= 40:
    raise SystemExit(1)
for index, page in enumerate(pages, 1):
    if not isinstance(page, dict) or page.get("page_index") != index:
        raise SystemExit(1)
    name = page.get("file")
    if not isinstance(name, str) or pathlib.PurePath(name).name != name or ".." in pathlib.PurePath(name).parts:
        raise SystemExit(1)
    page_path = staging / name
    if not page_path.is_file() or page_path.is_symlink():
        raise SystemExit(1)
print(len(pages))
PY
  ); then
    return 1
  fi
  printf '%s' "$page_count"
}

validate_correction_capture() {
  local staging=$1 expected_date=$2 expected_anchor=$3
  python3 - "$staging" "$expected_date" "$expected_anchor" >/dev/null 2>&1 <<'PY'
import json
import pathlib
import sys

staging = pathlib.Path(sys.argv[1])
expected_date = sys.argv[2]
expected_anchor = sys.argv[3]
if not staging.is_dir() or staging.is_symlink():
    raise SystemExit(1)
metadata = staging / "capture.json"
if not metadata.is_file() or metadata.is_symlink() or metadata.stat().st_size > 65536:
    raise SystemExit(1)
try:
    capture = json.loads(metadata.read_text(encoding="utf-8"))
except (OSError, UnicodeError, json.JSONDecodeError):
    raise SystemExit(1)
if not isinstance(capture, dict):
    raise SystemExit(1)
if capture.get("source") != "kind.krx.co.kr":
    raise SystemExit(1)
if capture.get("entry_url") != "https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf":
    raise SystemExit(1)
if capture.get("surface") != "etf-disclosure-correction-viewer":
    raise SystemExit(1)
requested = capture.get("requested_range")
if not isinstance(requested, dict) or requested.get("from") != expected_date or requested.get("to") != expected_date:
    raise SystemExit(1)
if capture.get("anchor_acceptance_number") != expected_anchor:
    raise SystemExit(1)
if capture.get("viewer_origin_path") != "/common/disclsviewer.do":
    raise SystemExit(1)
if capture.get("artifact_kind") != "rendered_dom_snapshot":
    raise SystemExit(1)
if capture.get("termination") != "viewer_loaded" or capture.get("termination_stage") != "viewer":
    raise SystemExit(1)
diagnostics = capture.get("response_diagnostics")
if not isinstance(diagnostics, dict):
    raise SystemExit(1)
for key in ("body_size", "form_field_count", "target_handler_occurrences"):
    if not isinstance(diagnostics.get(key), int) or diagnostics[key] < 1:
        raise SystemExit(1)
if diagnostics["body_size"] > 1024 * 1024:
    raise SystemExit(1)
if capture.get("file") != "viewer.html":
    raise SystemExit(1)
viewer = staging / "viewer.html"
if not viewer.is_file() or viewer.is_symlink() or not 0 < viewer.stat().st_size <= 1024 * 1024:
    raise SystemExit(1)
PY
}

extract_batch_id() {
  local result=$1 line value= count=0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "batch_id: "*)
        value=${line#batch_id: }
        count=$((count + 1))
        ;;
    esac
  done <"$result"
  [ "$count" -eq 1 ] || return 1
  [[ "$value" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]] || return 1
  printf '%s' "$value"
}

ensure_private_state() {
  safe_absolute_path "$state_root"
  safe_absolute_path "$state_root/candidates"
  require_root_owned_parent_chain "$state_root"
  require_root_owned_parent_chain "$state_root/candidates"
  mkdir -p -- "$state_root/candidates" || die 'state_directory_create_failed'
  chmod 700 -- "$state_root" "$state_root/candidates" || die 'state_directory_private_mode_failed'
  require_private_dir "$state_root"
  require_private_dir "$state_root/candidates"
}

ensure_empty_candidate_file() {
  # O_EXCL + O_NOFOLLOW makes the first daily empty candidate file
  # no-clobber and leaf-symlink-safe.  An existing file is never opened for
  # writing here; it is checked by require_private_file below.
  if ! python3 - "$candidate_file" >/dev/null 2>&1 <<'PY'
import os
import sys

path = sys.argv[1]
flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
try:
    fd = os.open(path, flags, 0o600)
except FileExistsError:
    pass
else:
    try:
        os.fchmod(fd, 0o600)
    finally:
        os.close(fd)
PY
  then
    die 'candidate_file_create_failed'
  fi
  require_private_file "$candidate_file"
}

ensure_run_lock() {
  local lock_file=$1
  safe_absolute_path "$lock_file"
  require_root_owned_parent_chain "$lock_file"
  if [ -e "$lock_file" ] || [ -L "$lock_file" ]; then
    require_private_file "$lock_file"
  elif ! python3 - "$lock_file" >/dev/null 2>&1 <<'PY'
import os
import sys

path = sys.argv[1]
flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
fd = os.open(path, flags, 0o600)
try:
    os.fchmod(fd, 0o600)
finally:
    os.close(fd)
PY
  then
    die 'run_lock_create_failed'
  fi
  [ ! -L "$lock_file" ] || die 'run_lock_symlink'
  require_private_file "$lock_file"
}

run_execute() {
  ensure_private_state
  check_static_inputs allow-missing-candidate
  local candidate_count

  local lock_file=$state_root/run.lock
  ensure_run_lock "$lock_file"
  if ! exec 9>>"$lock_file"; then
    die 'run_lock_open_failed'
  fi
  chmod 600 -- "$lock_file" || die 'run_lock_private_mode_failed'
  flock -n 9 || die 'run_already_active'

  ensure_empty_candidate_file
  if ! candidate_count=$(validate_candidate_file "$candidate_file"); then
    die 'candidate_file_invalid'
  fi

  run_dir=$(mktemp -d -- "$state_root/run-$target_date.XXXXXX") || die 'private_staging_create_failed'
  chmod 700 -- "$run_dir" || die 'private_staging_private_mode_failed'
  cleanup_success=0
  cleanup_run() {
    if [ "$cleanup_success" -eq 1 ] && [ -n "${run_dir:-}" ]; then
      rm -rf -- "$run_dir" || true
    fi
  }
  trap cleanup_run EXIT

  local candidate_snapshot=$run_dir/candidates.txt
  cp -- "$candidate_file" "$candidate_snapshot" || die 'candidate_snapshot_failed'
  chmod 600 -- "$candidate_snapshot" || die 'candidate_snapshot_private_mode_failed'
  if ! candidate_count=$(validate_candidate_file "$candidate_snapshot"); then
    die 'candidate_snapshot_invalid'
  fi

  local list_staging=$run_dir/list
  if ! "$node_bin" "$capture_script" \
      --from "$target_date" --to "$target_date" --out "$list_staging" \
      --confirm KIND_ETF_DISCLOSURE_CAPTURE --max-pages 40 \
      >/dev/null 2>&1; then
    die 'list_capture_failed_or_incomplete'
  fi
  local page_count
  if ! page_count=$(validate_list_capture "$list_staging" "$target_date"); then
    die 'list_capture_metadata_incomplete'
  fi

  local list_result=$run_dir/list-raw.stdout
  if ! KIND_RAW_ROOT="$raw_root" \
      KIND_ENTITLEMENT_REFERENCE="$KIND_DAILY_ENTITLEMENT_REFERENCE" \
      KIND_CONFIRM=I_UNDERSTAND_READ_ONLY_KIND_INGEST \
      "$kind_raw_bin" --staging "$list_staging" --execute \
      >"$list_result" 2>/dev/null; then
    die 'list_raw_ingest_failed'
  fi
  local list_batch
  if ! list_batch=$(extract_batch_id "$list_result"); then
    die 'list_raw_batch_id_missing'
  fi

  if ! "$kind_normalize_bin" --raw-root "$raw_root" \
      --source-batch-id "$list_batch" --mode disclosure \
      --candidate-file "$candidate_snapshot" >/dev/null 2>&1; then
    die 'list_normalization_failed'
  fi

  local candidate correction_staging correction_result correction_batch
  while IFS= read -r candidate || [ -n "$candidate" ]; do
    [ -n "$candidate" ] || die 'candidate_snapshot_blank_line'
    correction_staging=$run_dir/correction-$candidate
    if ! "$node_bin" "$correction_capture_script" \
        --from "$target_date" --to "$target_date" \
        --acceptance "$candidate" --out "$correction_staging" \
        --confirm KIND_CORRECTION_EVIDENCE_CAPTURE \
        >/dev/null 2>&1; then
      die 'correction_capture_failed_or_incomplete'
    fi
    validate_correction_capture "$correction_staging" "$target_date" "$candidate" \
      || die 'correction_capture_metadata_incomplete'

    correction_result=$run_dir/correction-$candidate-raw.stdout
    if ! KIND_CORRECTION_RAW_ROOT="$raw_root" \
        KIND_CORRECTION_ENTITLEMENT_REFERENCE="$KIND_DAILY_ENTITLEMENT_REFERENCE" \
        KIND_CORRECTION_CONFIRM=KIND_CORRECTION_EVIDENCE_CAPTURE \
        "$kind_correction_raw_bin" --staging "$correction_staging" --execute \
        >"$correction_result" 2>/dev/null; then
      die 'correction_raw_ingest_failed'
    fi
    if ! correction_batch=$(extract_batch_id "$correction_result"); then
      die 'correction_raw_batch_id_missing'
    fi
    if ! "$kind_normalize_bin" --raw-root "$raw_root" \
        --source-batch-id "$correction_batch" --mode correction \
        >/dev/null 2>&1; then
      die 'correction_normalization_failed'
    fi
  done <"$candidate_snapshot"

  cleanup_success=1
  printf 'KIND_DAILY status=complete target_date=%s list_pages=%s correction_candidates=%s\n' \
    "$target_date" "$page_count" "$candidate_count"
}

target_date=$(read_target_date)
candidate_file=$state_root/candidates/$target_date.txt
safe_absolute_path "$raw_root"
safe_absolute_path "$state_root"
safe_absolute_path "$candidate_file"

case "$mode" in
  plan)
    printf 'KIND_DAILY_PLAN target_date=%s window_days=1 stored_page_cap=40 correction_candidate_cap=5\n' \
      "$target_date"
    printf 'KIND_DAILY_PLAN candidate_file=date_specific_private_state\n'
    printf 'KIND_DAILY_PLAN no_browser=true no_network=true no_write=true\n'
    ;;
  check)
    check_static_inputs
    candidate_count=$(validate_candidate_file "$candidate_file") || die 'candidate_file_invalid'
    printf 'KIND_DAILY_CHECK status=pass target_date=%s candidate_count=%s\n' \
      "$target_date" "$candidate_count"
    ;;
  execute)
    run_execute
    ;;
  *)
    die 'invalid_mode'
    ;;
esac
