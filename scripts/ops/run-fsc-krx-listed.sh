#!/usr/bin/env bash
# Offline one-date plan/check wrapper. It never loads credentials or starts a
# provider action.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
action=--plan
date_value=
bin=${LAGRANGE_FSC_KRX_LISTED_BIN:-$root/target/release/fsc-krx-listed-raw}

die() {
  echo 'run-fsc-krx-listed: invalid invocation' >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: scripts/ops/run-fsc-krx-listed.sh --date YYYY-MM-DD [--plan|--check]

The default is --plan. Both actions are offline and make no provider or Raw
write. This wrapper has no range or backfill mode and does not install or
start systemd.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --date)
      [ "$#" -ge 2 ] || die
      [ -z "$date_value" ] || die
      date_value=$2
      shift 2
      ;;
    --plan|--check)
      [ "$action" = --plan ] || die
      action=$1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die
      ;;
  esac
done

[ -n "$date_value" ] || die
[ -x "$bin" ] || {
  echo 'run-fsc-krx-listed: collector binary is not executable' >&2
  exit 1
}

export FSC_KRX_LISTED_RAW_ROOT=${FSC_KRX_LISTED_RAW_ROOT:-$root/data}

exec "$bin" --date "$date_value" "$action"
