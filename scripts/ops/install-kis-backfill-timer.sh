#!/usr/bin/env bash
# Install the recurring, read-only KIS backfill timer for one immutable release.
# Default is a no-change plan. No KIS, Docker, DB, secret, or order call is
# made by this installer; --apply only writes the two systemd unit files,
# reloads systemd, and enables (but does not start) the timer (never the
# backfill service). Applying at/after 03:15 KST is refused because a later
# start of a Persistent timer could immediately catch up and call KIS.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
mode=dry-run
mode_seen=0
release_root=
code_commit=
state_file=
start_date=
end_date=
universe=etf
systemd_dir=/etc/systemd/system
replace_existing=0

timer_now_hms() {
  # This override is intentionally available only to the no-change dry-run
  # path, so a test can exercise the schedule boundary without making the
  # production apply gate bypassable.
  if [ "$mode" = dry-run ] && [ -n "${KIS_BACKFILL_TIMER_TEST_NOW:-}" ]; then
    [[ "$KIS_BACKFILL_TIMER_TEST_NOW" =~ ^[0-9]{2}:[0-9]{2}:[0-9]{2}$ ]] ||
      die 'KIS_BACKFILL_TIMER_TEST_NOW must be HH:MM:SS for dry-run tests'
    local hour=${KIS_BACKFILL_TIMER_TEST_NOW:0:2}
    local minute=${KIS_BACKFILL_TIMER_TEST_NOW:3:2}
    local second=${KIS_BACKFILL_TIMER_TEST_NOW:6:2}
    ((10#$hour < 24 && 10#$minute < 60 && 10#$second < 60)) ||
      die 'KIS_BACKFILL_TIMER_TEST_NOW is outside the clock range'
    printf '%s' "${KIS_BACKFILL_TIMER_TEST_NOW//:/}"
    return 0
  fi
  TZ=Asia/Seoul date +%H%M%S
}

apply_window_is_open() {
  local now_hms=$1
  ((10#$now_hms < 31500))
}

usage() {
  cat <<'EOF'
Usage: install-kis-backfill-timer.sh [--dry-run|--preflight|--check|--apply]
       --release-root /opt/lagrange/releases/<40-hex-commit>
       --code-commit <40-lowercase-hex>
       --state-file /var/lib/lagrange/data/backfill/state.tsv
       --start YYYY-MM-DD --end YYYY-MM-DD [--universe etf]
       [--systemd-dir PATH] [--replace-existing]

The timer runs the exact immutable release once per day at 03:15 KST with
--auto-resume. Only KIS_CALENDAR_SNAPSHOT_MISS and retryable errors may be
retried automatically; other permanent errors require a manual run without
--auto-resume. The unit contains no credential or secret value.
--apply requires root and an explicit --replace-existing when either target
unit already exists. It enables the timer but does not start the timer or
backfill service; an operator starts the timer after reviewing the unit.
EOF
}

die() { echo "install-kis-backfill-timer: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run|--preflight|--check|--apply)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}; mode_seen=1; shift ;;
    --release-root) [ "$#" -ge 2 ] || die '--release-root needs a path'; release_root=$2; shift 2 ;;
    --code-commit) [ "$#" -ge 2 ] || die '--code-commit needs a value'; code_commit=$2; shift 2 ;;
    --state-file) [ "$#" -ge 2 ] || die '--state-file needs a path'; state_file=$2; shift 2 ;;
    --start) [ "$#" -ge 2 ] || die '--start needs a date'; start_date=$2; shift 2 ;;
    --end) [ "$#" -ge 2 ] || die '--end needs a date'; end_date=$2; shift 2 ;;
    --universe) [ "$#" -ge 2 ] || die '--universe needs etf'; universe=$2; shift 2 ;;
    --systemd-dir) [ "$#" -ge 2 ] || die '--systemd-dir needs a path'; systemd_dir=$2; shift 2 ;;
    --replace-existing) replace_existing=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -n "$release_root" ] || die '--release-root is required'
[ -n "$code_commit" ] || die '--code-commit is required'
[ -n "$state_file" ] || die '--state-file is required'
[ -n "$start_date" ] && [ -n "$end_date" ] || die '--start and --end are required'
[[ "$code_commit" =~ ^[0-9a-f]{40}$ ]] || die '--code-commit must be 40 lowercase hexadecimal characters'
[[ "$release_root" != *[[:space:]]* && "$state_file" != *[[:space:]]* ]] ||
  die 'release/state paths must not contain whitespace'
case "$universe" in etf) ;; *) die '--universe must be etf' ;; esac
python3 - "$start_date" "$end_date" <<'PY'
import datetime as dt
import sys
try:
    start = dt.date.fromisoformat(sys.argv[1])
    end = dt.date.fromisoformat(sys.argv[2])
except ValueError as exc:
    raise SystemExit(f"invalid calendar date: {exc}")
if end < start:
    raise SystemExit("--end precedes --start")
PY

safe_path() {
  local path=$1 label=$2 probe
  case "$path" in /*) ;; *) die "$label must be absolute: $path" ;; esac
  case "$path" in */../*|*/..) die "$label must not contain '..': $path" ;; esac
  probe=${path%/}; [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}; [ -n "$probe" ] || probe=/
  done
}

safe_path "$release_root" release-root
safe_path "$state_file" state-file
safe_path "$systemd_dir" systemd-dir
[ -d "$release_root" ] && [ ! -L "$release_root" ] || die 'release root is missing or unsafe'
[ "$(basename -- "$release_root")" = "$code_commit" ] ||
  die 'release root basename must equal --code-commit'
[ -x "$release_root/scripts/ops/backfill-production.sh" ] ||
  die 'release backfill script is not executable'

short_commit=${code_commit:0:7}
service_name="lagrange-kis-backfill-${short_commit}.service"
timer_name="lagrange-kis-backfill-${short_commit}.timer"
service_target="$systemd_dir/$service_name"
timer_target="$systemd_dir/$timer_name"

if [ "$mode" = dry-run ]; then
  now_hms=$(timer_now_hms)
  if apply_window_is_open "$now_hms"; then
    apply_window='open (before 03:15:00 Asia/Seoul)'
  else
    apply_window='closed (at/after 03:15:00 Asia/Seoul; --apply is refused)'
  fi
  echo 'KIS_BACKFILL_TIMER_PLAN mode=dry-run'
  echo "  release=$release_root (commit=$code_commit)"
  echo "  range=$start_date..$end_date universe=$universe"
  echo "  state=$state_file"
  echo "  service=$service_target (manual review on permanent error)"
  echo "  timer=$timer_target (daily 03:15 Asia/Seoul, Persistent=true)"
  echo "  apply-window=$apply_window"
  echo '  --auto-resume permits only calendar snapshot deferral/retryable state'
  echo '  no KIS/Docker/DB/secret/order call; timer/service are never started by --apply'
  echo 'DRY_RUN: no unit write, systemd reload, or timer enable'
  exit 0
fi

[ "$(id -u)" -eq 0 ] || die "--$mode must run as root"
[ -d "$systemd_dir" ] && [ ! -L "$systemd_dir" ] || die 'systemd directory is missing or unsafe'

if [ "$mode" = apply ]; then
  # Persistent timers can catch up a missed activation when they are started.
  # Never install one at/after today's scheduled boundary: an operator may
  # legitimately start the enabled timer later, but that must be a deliberate
  # next-window action rather than an accidental immediate KIS call.
  now_hms=$(TZ=Asia/Seoul date +%H%M%S)
  if ! apply_window_is_open "$now_hms"; then
    die "--apply is allowed only before 03:15:00 Asia/Seoul (now=${now_hms:0:2}:${now_hms:2:2}:${now_hms:4:2}); schedule installation before the next window"
  fi
fi

for target in "$service_target" "$timer_target"; do
  if [ -e "$target" ] || [ -L "$target" ]; then
    [ "$replace_existing" -eq 1 ] || die "target exists; pass --replace-existing after review: $target"
    [ ! -L "$target" ] || die "refusing to replace symlink target: $target"
  fi
done

service_body=$(cat <<EOF
[Unit]
Description=Lagrange recurring KIS read-only ETF backfill ($short_commit)
After=docker.service network-online.target
Wants=network-online.target
Requires=docker.service
ConditionFileIsExecutable=$release_root/scripts/ops/backfill-production.sh

[Service]
Type=oneshot
User=root
Group=root
WorkingDirectory=$release_root
Environment=LAGRANGE_CODE_COMMIT=$code_commit
Environment=LAGRANGE_BACKFILL_STATE=$state_file
Environment=BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS
ExecStart=$release_root/scripts/ops/backfill-production.sh --auto-resume --start $start_date --end $end_date --universe $universe --execute
UMask=0077
TimeoutStartSec=12h
NoNewPrivileges=true
SuccessExitStatus=74 75

[Install]
WantedBy=multi-user.target
EOF
)
timer_body=$(cat <<EOF
[Unit]
Description=Daily Lagrange KIS read-only ETF backfill ($short_commit)

[Timer]
OnCalendar=*-*-* 03:15:00 Asia/Seoul
Persistent=true
AccuracySec=1min
RandomizedDelaySec=0
Unit=$service_name

[Install]
WantedBy=timers.target
EOF
)

if [ "$mode" = preflight ]; then
  echo "KIS_BACKFILL_TIMER_PREFLIGHT: PASS service=$service_name timer=$timer_name"
  exit 0
fi

write_unit() {
  local target=$1 body=$2 temp
  temp=$(mktemp "$systemd_dir/.lagrange-kis-backfill.XXXXXX") || die 'cannot stage systemd unit'
  chmod 0644 "$temp"
  printf '%s\n' "$body" >"$temp"
  chown 0:0 "$temp"
  mv -fT -- "$temp" "$target"
}

if [ "$mode" = check ]; then
  [ -f "$service_target" ] && [ ! -L "$service_target" ] || die 'service unit missing or unsafe'
  [ -f "$timer_target" ] && [ ! -L "$timer_target" ] || die 'timer unit missing or unsafe'
  [ "$(stat -c '%u:%g:%a' "$service_target")" = 0:0:644 ] || die 'service unit metadata mismatch'
  [ "$(stat -c '%u:%g:%a' "$timer_target")" = 0:0:644 ] || die 'timer unit metadata mismatch'
  cmp -s <(printf '%s\n' "$service_body") "$service_target" || die 'service unit differs from requested contract'
  cmp -s <(printf '%s\n' "$timer_body") "$timer_target" || die 'timer unit differs from requested contract'
  echo "KIS_BACKFILL_TIMER_CHECK: PASS service=$service_name timer=$timer_name"
  exit 0
fi

# Replacing an active timer without stopping it leaves the old in-memory
# schedule loaded. Stop only the timer (never the backfill service), then write
# and reload the new recurring unit. The timer remains enabled but stopped so
# Persistent=true cannot trigger a KIS run during installation.
command -v systemctl >/dev/null 2>&1 || die 'systemctl is required for --apply'
if [ "$replace_existing" -eq 1 ] && systemctl is-active --quiet "$service_name"; then
  die 'backfill service is active; stop it and review before replacing its timer'
fi
if [ "$replace_existing" -eq 1 ] && systemctl is-active --quiet "$timer_name"; then
  systemctl stop "$timer_name"
fi
write_unit "$service_target" "$service_body"
write_unit "$timer_target" "$timer_body"
systemctl daemon-reload
systemctl enable "$timer_name"
echo "KIS_BACKFILL_TIMER_APPLY: PASS timer=$timer_name enabled-not-started service=not-started"
