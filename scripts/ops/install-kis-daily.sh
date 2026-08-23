#!/usr/bin/env bash
# Install the once-daily, read-only KIS runner for one immutable release.
#
# The runner is intentionally kept in the release's scripts/ops tree: it
# sources helpers relative to that tree and delegates to the release's
# backfill-production.sh. This installer only renders commit-suffixed units;
# --apply writes them, reloads systemd, and enables the timer, but never starts
# either unit. No mode calls KIS, Docker, a database, or a credential helper.
set -euo pipefail
umask 077

mode=dry-run
mode_seen=0
release_root=
code_commit=
production_env_file=/opt/lagrange/deploy/compose/.env
calendar_dir=/var/lib/lagrange/state/kis-daily/calendar
state_dir=/var/lib/lagrange/state/backfill
lock_file=/var/lib/lagrange/state/backfill/kis-daily.lock
systemd_dir=/etc/systemd/system
replace_existing=0
service_temp=
timer_temp=

die() {
  printf 'install-kis-daily: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: scripts/ops/install-kis-daily.sh [--dry-run|--preflight|--check|--apply]
       --release-root /opt/lagrange/releases/<40-lowercase-hex-commit>
       --code-commit <40-lowercase-hex>
       [--env-file /opt/lagrange/deploy/compose/.env]
       [--production-env-file /opt/lagrange/deploy/compose/.env]
       [--calendar-dir /var/lib/lagrange/state/kis-daily/calendar]
       [--state-dir /var/lib/lagrange/state/backfill]
       [--lock-file /var/lib/lagrange/state/backfill/kis-daily.lock]
       [--systemd-dir /etc/systemd/system] [--replace-existing]

The service and timer names are suffixed with the first seven characters of
the exact release commit. The service executes the release-local
scripts/ops/kis-daily-production.sh and uses only path-valued environment
entries; the production dotenv is read by the wrapper, not embedded here.
--apply requires root and an explicit --replace-existing for existing regular
unit files. It may reload systemd and enable the timer, but never starts the
service or timer. Persistent=true makes --apply unsafe at or after 16:30
Asia/Seoul, so installation must happen before that boundary.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run|--preflight|--check|--apply)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    --release-root)
      [ "$#" -ge 2 ] || die '--release-root needs a path'
      [ -z "$release_root" ] || die '--release-root was repeated'
      release_root=$2
      shift 2
      ;;
    --code-commit)
      [ "$#" -ge 2 ] || die '--code-commit needs a value'
      [ -z "$code_commit" ] || die '--code-commit was repeated'
      code_commit=$2
      shift 2
      ;;
    --env-file|--production-env-file)
      [ "$#" -ge 2 ] || die '--env-file needs a path'
      production_env_file=$2
      shift 2
      ;;
    --calendar-dir|--xkrx-calendar-dir)
      [ "$#" -ge 2 ] || die '--calendar-dir needs a path'
      calendar_dir=$2
      shift 2
      ;;
    --state-dir)
      [ "$#" -ge 2 ] || die '--state-dir needs a path'
      state_dir=$2
      shift 2
      ;;
    --lock-file)
      [ "$#" -ge 2 ] || die '--lock-file needs a path'
      lock_file=$2
      shift 2
      ;;
    --systemd-dir)
      [ "$#" -ge 2 ] || die '--systemd-dir needs a path'
      systemd_dir=$2
      shift 2
      ;;
    --replace-existing)
      replace_existing=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

safe_path() {
  local path=$1 label=$2 probe
  [ -n "$path" ] || die "$label must not be empty"
  case "$path" in
    /*) ;;
    *) die "$label must be absolute: $path" ;;
  esac
  case "$path" in
    */../*|*/..|*[[:space:]]*|*%*|*\\*)
      die "$label has an unsafe path shape: $path"
      ;;
  esac
  [ "$path" = / ] || case "$path" in */) die "$label must not end with '/': $path" ;; esac
  probe=${path%/}
  [ -n "$probe" ] || probe=/
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || die "$label must not traverse a symlink: $probe"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
}

require_directory_if_present() {
  local path=$1 label=$2
  if [ -e "$path" ] || [ -L "$path" ]; then
    [ -d "$path" ] && [ ! -L "$path" ] || die "$label is missing, not a directory, or unsafe: $path"
  fi
}

require_file_if_present() {
  local path=$1 label=$2
  if [ -e "$path" ] || [ -L "$path" ]; then
    [ -f "$path" ] && [ ! -L "$path" ] || die "$label is not a regular file or is unsafe: $path"
  fi
}

require_release_file() {
  local relative=$1 kind=$2 path="$release_root/$1"
  safe_path "$path" "release $relative"
  [ -f "$path" ] && [ ! -L "$path" ] || die "release $relative is missing or unsafe"
  if [ "$kind" = executable ]; then
    [ -x "$path" ] || die "release $relative is not executable"
  fi
}

check_source_inputs() {
  [ -n "$release_root" ] || die '--release-root is required'
  [ -n "$code_commit" ] || die '--code-commit is required'
  [[ "$code_commit" =~ ^[0-9a-f]{40}$ ]] ||
    die '--code-commit must be 40 lowercase hexadecimal characters'

  safe_path "$release_root" release-root
  safe_path "$production_env_file" production-env-file
  safe_path "$calendar_dir" operational-calendar-directory
  safe_path "$state_dir" protected-state-directory
  safe_path "$lock_file" protected-lock-file
  safe_path "$systemd_dir" systemd-directory

  [ -d "$release_root" ] && [ ! -L "$release_root" ] ||
    die 'release root is missing or unsafe'
  [ "$(basename -- "$release_root")" = "$code_commit" ] ||
    die 'release root basename must equal --code-commit'
  [ "$state_dir" != / ] || die 'protected state directory is too broad'
  [ "$calendar_dir" != / ] || die 'operational calendar directory is too broad'
  [ "$calendar_dir" != "$state_dir" ] ||
    die 'operational calendar and protected state directories must differ'
  [ "$(dirname -- "$lock_file")" = "$state_dir" ] ||
    die 'protected lock file must be directly below --state-dir'
  case "$calendar_dir/" in
    "$state_dir"/*) die 'operational calendar must not be inside protected state' ;;
  esac
  case "$state_dir/" in
    "$calendar_dir"/*) die 'protected state must not be inside operational calendar' ;;
  esac

  require_directory_if_present "$calendar_dir" operational-calendar-directory
  require_directory_if_present "$state_dir" protected-state-directory
  require_directory_if_present "$systemd_dir" systemd-directory
  require_file_if_present "$production_env_file" production-env-file
  require_file_if_present "$lock_file" protected-lock-file

  require_release_file scripts/ops/kis-daily-production.sh executable
  require_release_file scripts/ops/backfill-production.sh executable
  require_release_file scripts/ops/validate-production-config.sh executable
  require_release_file scripts/ops/xkrx-calendar-bootstrap.py executable
  require_release_file scripts/ops/lib/dotenv.sh regular
  require_release_file scripts/ops/lib/db.sh regular
  require_release_file scripts/ops/lib/kis-daily-state.py regular
}

timer_now_hms() {
  local value
  if [ "$mode" = dry-run ] && [ -n "${KIS_DAILY_TIMER_TEST_NOW:-}" ]; then
    [[ "$KIS_DAILY_TIMER_TEST_NOW" =~ ^[0-9]{2}:[0-9]{2}:[0-9]{2}$ ]] ||
      die 'KIS_DAILY_TIMER_TEST_NOW must be HH:MM:SS for dry-run tests'
    value=${KIS_DAILY_TIMER_TEST_NOW//:/}
  else
    value=$(TZ=Asia/Seoul date +%H%M%S)
  fi
  [[ "$value" =~ ^[0-9]{6}$ ]] || die 'timer clock must be HH:MM:SS'
  (( 10#${value:0:2} < 24 && 10#${value:2:2} < 60 && 10#${value:4:2} < 60 )) ||
    die 'timer clock is outside the clock range'
  printf '%s' "$value"
}

apply_window_is_open() {
  local now_hms=$1
  (( 10#$now_hms < 163000 ))
}

short_commit=${code_commit:0:7}
service_name="lagrange-kis-daily-${short_commit}.service"
timer_name="lagrange-kis-daily-${short_commit}.timer"
service_target="$systemd_dir/$service_name"
timer_target="$systemd_dir/$timer_name"
wrapper="$release_root/scripts/ops/kis-daily-production.sh"

render_service() {
  cat <<EOF
[Unit]
Description=Lagrange daily KIS read-only incremental ETF EOD ($short_commit)
After=docker.service network-online.target
Wants=network-online.target
Requires=docker.service
ConditionFileIsExecutable=$wrapper
ConditionPathExists=$production_env_file
ConditionPathIsDirectory=$calendar_dir

[Service]
Type=oneshot
User=root
Group=root
WorkingDirectory=$release_root
Environment=LAGRANGE_CODE_COMMIT=$code_commit
Environment=LAGRANGE_ENV_FILE=$production_env_file
Environment=LAGRANGE_XKRX_CALENDAR_DIR=$calendar_dir
Environment=KIS_DAILY_CALENDAR_DIR=$calendar_dir
Environment=KIS_DAILY_STATE_DIR=$state_dir
Environment=KIS_DAILY_LOCK_FILE=$lock_file
Environment=BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS
ExecStart=$wrapper --execute
UMask=0077
TimeoutStartSec=12h
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
LockPersonality=true
RestrictAddressFamilies=AF_UNIX
ReadOnlyPaths=$release_root $production_env_file $calendar_dir
ReadWritePaths=$state_dir
SuccessExitStatus=74 75

[Install]
WantedBy=multi-user.target
EOF
}

render_timer() {
  cat <<EOF
[Unit]
Description=Daily Lagrange KIS read-only incremental ETF EOD ($short_commit)

[Timer]
OnCalendar=*-*-* 16:30:00 Asia/Seoul
Persistent=true
AccuracySec=1min
RandomizedDelaySec=0
Unit=$service_name

[Install]
WantedBy=timers.target
EOF
}

service_body=$(render_service)
timer_body=$(render_timer)

cleanup() {
  [ -z "$service_temp" ] || rm -f -- "$service_temp"
  [ -z "$timer_temp" ] || rm -f -- "$timer_temp"
}
trap cleanup EXIT

check_source_inputs

if [ "$mode" = dry-run ]; then
  now_hms=$(timer_now_hms)
  if apply_window_is_open "$now_hms"; then
    apply_window='open (before 16:30:00 Asia/Seoul)'
  else
    apply_window='closed (at/after 16:30:00 Asia/Seoul; --apply is refused)'
  fi
  printf 'KIS_DAILY_INSTALL_PLAN mode=dry-run release=%s code_commit=%s\n' "$release_root" "$code_commit"
  printf '  service=%s\n' "$service_target"
  printf '  timer=%s (daily 16:30 Asia/Seoul, Persistent=true)\n' "$timer_target"
  printf '  WorkingDirectory=%s\n' "$release_root"
  printf '  ExecStart=%s --execute\n' "$wrapper"
  printf '  LAGRANGE_ENV_FILE=%s\n' "$production_env_file"
  printf '  calendar=%s state=%s lock=%s\n' "$calendar_dir" "$state_dir" "$lock_file"
  printf '  apply-window=%s\n' "$apply_window"
  echo '  no unit write, systemd call, KIS/Docker/DB call, or credential read'
  echo 'DRY_RUN: no unit write, systemd reload, or timer enable'
  exit 0
fi

if [ "$mode" = preflight ]; then
  printf 'KIS_DAILY_INSTALL_PREFLIGHT: PASS service=%s timer=%s release=%s\n' \
    "$service_name" "$timer_name" "$release_root"
  exit 0
fi

if [ "$mode" = check ]; then
  [ -d "$systemd_dir" ] && [ ! -L "$systemd_dir" ] ||
    die 'systemd directory is missing or unsafe'
  [ -f "$service_target" ] && [ ! -L "$service_target" ] ||
    die 'service unit is missing or unsafe'
  [ -f "$timer_target" ] && [ ! -L "$timer_target" ] ||
    die 'timer unit is missing or unsafe'
  [ "$(stat -c '%u:%g:%a' -- "$service_target")" = 0:0:644 ] ||
    die 'service unit metadata mismatch'
  [ "$(stat -c '%u:%g:%a' -- "$timer_target")" = 0:0:644 ] ||
    die 'timer unit metadata mismatch'
  cmp -s <(printf '%s\n' "$service_body") "$service_target" ||
    die 'service unit differs from requested contract'
  cmp -s <(printf '%s\n' "$timer_body") "$timer_target" ||
    die 'timer unit differs from requested contract'
  printf 'KIS_DAILY_INSTALL_CHECK: PASS service=%s timer=%s release=%s\n' \
    "$service_name" "$timer_name" "$release_root"
  exit 0
fi

# This gate intentionally runs before the root/systemd checks. A late apply is
# unsafe regardless of caller privileges, and must fail before any systemd
# command can be reached. KIS_DAILY_TIMER_TEST_NOW is ignored outside dry-run.
now_hms=$(timer_now_hms)
if ! apply_window_is_open "$now_hms"; then
  die "--apply is allowed only before 16:30:00 Asia/Seoul (now=${now_hms:0:2}:${now_hms:2:2}:${now_hms:4:2}); schedule installation before the next window"
fi

[ "$(id -u)" -eq 0 ] || die '--apply must run as root'
[ -d "$systemd_dir" ] && [ ! -L "$systemd_dir" ] ||
  die 'systemd directory is missing or unsafe'
command -v systemctl >/dev/null 2>&1 || die 'systemctl is required for --apply'

for target in "$service_target" "$timer_target"; do
  if [ -e "$target" ] || [ -L "$target" ]; then
    [ "$replace_existing" -eq 1 ] ||
      die "target exists; pass --replace-existing after review: $target"
    [ ! -L "$target" ] || die "refusing to replace symlink target: $target"
    [ -f "$target" ] || die "refusing to replace non-regular target: $target"
  fi
done

# Replacing an active timer must first remove its in-memory schedule. Stopping
# the timer is allowed during replacement; neither the service nor timer is
# ever started by this installer.
if [ "$replace_existing" -eq 1 ] && systemctl is-active --quiet "$service_name"; then
  die 'daily service is active; stop it and review before replacing its timer'
fi
if [ "$replace_existing" -eq 1 ] && systemctl is-active --quiet "$timer_name"; then
  systemctl stop "$timer_name"
fi

stage_unit() {
  local body=$1 slot=$2 temp
  temp=$(mktemp "$systemd_dir/.lagrange-kis-daily.XXXXXX") || die 'cannot stage systemd unit'
  if ! printf '%s\n' "$body" >"$temp"; then
    rm -f -- "$temp"
    die 'cannot write staged systemd unit'
  fi
  chmod 0644 -- "$temp"
  chown 0:0 -- "$temp"
  [ "$(stat -c '%u:%g:%a' -- "$temp")" = 0:0:644 ] || die 'staged unit metadata mismatch'
  if [ "$slot" = service ]; then
    service_temp=$temp
  else
    timer_temp=$temp
  fi
}

stage_unit "$service_body" service
stage_unit "$timer_body" timer

move_staged_unit() {
  local staged=$1 target=$2
  if [ "$replace_existing" -eq 1 ]; then
    mv -fT -- "$staged" "$target"
  else
    mv -nT -- "$staged" "$target"
    [ ! -e "$staged" ] || die "target appeared during no-clobber install: $target"
  fi
}

move_staged_unit "$service_temp" "$service_target"
service_temp=
move_staged_unit "$timer_temp" "$timer_target"
timer_temp=

systemctl daemon-reload
systemctl enable "$timer_name"
printf 'KIS_DAILY_INSTALL_APPLY: PASS timer=%s enabled-not-started service=not-started release=%s\n' \
  "$timer_name" "$release_root"
