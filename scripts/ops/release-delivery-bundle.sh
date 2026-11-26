#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  exec python3 "$script_dir/lib/release-delivery-bundle.py" "$@"
fi

if [ "$(id -u)" -ne 0 ]; then
  echo 'release-delivery-bundle: must run as root' >&2
  exit 1
fi

exec python3 "$script_dir/lib/release-delivery-bundle.py" "$@"
