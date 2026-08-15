#!/usr/bin/env bash
# validate-foundation.sh — POSIX twin of scripts/validate-foundation.ps1:
# assert every documented workspace boundary (design §20 + Todo 1 list) and its
# pin files exist. Exit 0 when the full tree is present; exit 1 listing each
# missing path. Used by tests/foundation/test_pins.sh.
set -u
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
missing=0

dirs="apps/web apps/api-server
crates/domain crates/auth crates/market-data crates/factor-engine crates/selector
crates/portfolio-model crates/job-queue crates/result-model crates/risk-gateway crates/kis-client
nt/strategies nt/custom-data nt/backtest-worker nt/paper-runner nt/live-node
data-pipelines/collectors data-pipelines/validators data-pipelines/normalizers data-pipelines/nt-catalog-builder
migrations configs
tests/fixtures tests/golden tests/integration tests/e2e tests/failure
deploy/compose deploy/nginx deploy/backup
scripts scripts/qa"

files="rust-toolchain.toml .python-version Cargo.toml Cargo.lock package.json package-lock.json nt/pyproject.toml .gitignore"

for d in $dirs; do
  [ -d "$root/$d" ] || { echo "MISSING dir: $d"; missing=1; }
done
for f in $files; do
  [ -f "$root/$f" ] || { echo "MISSING file: $f"; missing=1; }
done

if [ "$missing" -ne 0 ]; then
  echo "FOUNDATION VALIDATION FAILED"
  exit 1
fi
echo "FOUNDATION OK: documented workspace topology and pin files present"
exit 0
