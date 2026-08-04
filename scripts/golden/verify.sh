#!/usr/bin/env sh
# verify.sh - POSIX twin of scripts/golden/verify.ps1.
# Exit 0 when unchanged; exit 1 with a field-level diff on any drift.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
if command -v uv >/dev/null 2>&1; then
  exec uv run --project "$root/nt" python "$root/scripts/golden/golden.py" verify "$@"
fi
exec python3 "$root/scripts/golden/golden.py" verify "$@"
