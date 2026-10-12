#!/usr/bin/env bash
# Provider-free candidate E2E wrapper. Keep this CI entrypoint stable.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

PYTHONDONTWRITEBYTECODE=1 python3 -B -m unittest discover \
  -s scripts/qa -p 'test_candidate_web_e2e_runner.py' -v

exec python3 -B "$root/scripts/qa/candidate-web-e2e-runner.py" "$@"
