#!/usr/bin/env bash
# verify-all.sh - POSIX twin of scripts/verify-all.ps1 for CI / clean containers.
# Fails fast (exit 1) on the first failing gate. Every gate is a hard gate.
set -u
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 1
step=0

step=$((step+1)); echo; echo "[$step] check-pins (approved toolchain/package pins)"
bash "$root/scripts/check-pins.sh" || { echo "FAILED: check-pins"; exit 1; }

step=$((step+1)); echo; echo "[$step] committed lockfiles (Cargo.lock, package-lock.json)"
for lf in Cargo.lock package-lock.json; do
  [ -f "$root/$lf" ] || { echo "FAILED: missing committed lockfile $lf"; exit 1; }
done
echo "Cargo.lock and package-lock.json present"

step=$((step+1)); echo; echo "[$step] validate-foundation (documented workspace topology)"
bash "$root/scripts/validate-foundation.sh" || { echo "FAILED: validate-foundation"; exit 1; }

step=$((step+1)); echo; echo "[$step] cargo fmt --all --check"
cargo fmt --all --check || { echo "FAILED: cargo fmt --all --check"; exit 1; }

step=$((step+1)); echo; echo "[$step] cargo clippy --workspace --all-targets --all-features -- -D warnings"
cargo clippy --workspace --all-targets --all-features -- -D warnings || { echo "FAILED: cargo clippy -D warnings"; exit 1; }

step=$((step+1)); echo; echo "[$step] cargo test --workspace"
cargo test --workspace || { echo "FAILED: cargo test --workspace"; exit 1; }

step=$((step+1)); echo; echo "[$step] npm run lint --workspaces --if-present"
npm run lint --workspaces --if-present || { echo "FAILED: npm lint"; exit 1; }

step=$((step+1)); echo; echo "[$step] npm run typecheck --workspaces --if-present"
npm run typecheck --workspaces --if-present || { echo "FAILED: npm typecheck"; exit 1; }

step=$((step+1)); echo; echo "[$step] npm test --workspaces --if-present"
npm test --workspaces --if-present || { echo "FAILED: npm test"; exit 1; }

step=$((step+1)); echo; echo "[$step] uv run --project nt pytest -q"
uv run --project nt pytest -q || { echo "FAILED: uv run --project nt pytest -q"; exit 1; }

# Backup POLICY tests only. They need no Docker and no prebuilt backup set, so
# they belong in a clean-container gate. The restore DRILL
# (scripts/backup/tests/test-restore-failures.*) needs a live Docker daemon and
# a real backup set, so it runs from the operational suites instead.
step=$((step+1)); echo; echo "[$step] backup policy validator tests"
bash "$root/scripts/backup/tests/test-validate-policy.sh" || { echo "FAILED: backup policy tests"; exit 1; }

echo; echo "ALL GATES PASSED"
exit 0
