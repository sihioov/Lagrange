#!/usr/bin/env sh
# generate.sh - POSIX twin of scripts/golden/generate.ps1 for CI / clean containers.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
if command -v uv >/dev/null 2>&1; then
  exec uv run --project "$root/nt" python "$root/scripts/golden/golden.py" generate "$@"
fi
exec python3 "$root/scripts/golden/golden.py" generate "$@"
