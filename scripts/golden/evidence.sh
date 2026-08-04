#!/usr/bin/env sh
# evidence.sh - POSIX twin of scripts/golden/evidence.ps1.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
if command -v uv >/dev/null 2>&1; then
  exec uv run --project "$root/nt" python "$root/scripts/golden/golden.py" evidence "$@"
fi
exec python3 "$root/scripts/golden/golden.py" evidence "$@"
