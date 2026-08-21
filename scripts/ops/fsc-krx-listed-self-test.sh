#!/usr/bin/env bash
# Offline static/shell contract checks. No key, network, provider, DB, or
# systemd operation is performed.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
collector="$root/data-pipelines/collectors/src/bin/fsc-krx-listed-raw.rs"
runner="$root/scripts/ops/run-fsc-krx-listed.sh"
collector_manifest="$root/data-pipelines/collectors/Cargo.toml"

[ -f "$collector" ] || {
  echo 'fsc-krx-listed-self-test: offline collector is missing' >&2
  exit 1
}
[ -f "$runner" ] || {
  echo 'fsc-krx-listed-self-test: offline runner is missing' >&2
  exit 1
}
[ ! -e "$root/scripts/ops/grant-fsc-krx-listed-temporary-access.sh" ] || {
  echo 'fsc-krx-listed-self-test: temporary grant path still exists' >&2
  exit 1
}
[ ! -e "$root/scripts/ops/provision-fsc-krx-listed-key.sh" ] || {
  echo 'fsc-krx-listed-self-test: credential provisioning path still exists' >&2
  exit 1
}

bash -n "$runner"

# Build the removed flag names from a suffix so this test does not preserve a
# live invocation spelling of its own.
for suffix in live live-probe; do
  forbidden="--approve-$suffix"
  if grep -Fq -- "$forbidden" "$collector" "$runner"; then
    echo "fsc-krx-listed-self-test: removed live flag survives: $suffix" >&2
    exit 1
  fi
done

for forbidden in \
  I_UNDERSTAND_READ_ONLY \
  "DataGo""Client" \
  "SystemCredential""Source" \
  "FscKrxListed""Provider" \
  "FscKrxListed""Availability" \
  "FSC_KRX_LISTED_KEY_""FILE" \
  "service""Key" \
  sudo \
  systemctl \
  curl \
  wget; do
  if grep -Fq -- "$forbidden" "$collector" "$runner"; then
    echo "fsc-krx-listed-self-test: forbidden live surface survives: $forbidden" >&2
    exit 1
  fi
done

if grep -Fq 'data-go-client' "$collector_manifest"; then
  echo 'fsc-krx-listed-self-test: unused direct data-go-client dependency survives' >&2
  exit 1
fi

grep -Fq 'Action::Plan' "$collector"
grep -Fq 'Action::Check' "$collector"
grep -Fq 'network=not-called' "$collector"
grep -Fq -- '--plan|--check' "$runner"

# The fixture/provider crates remain available for historical offline
# contract tests; this remediation removes only their executable live entry.
[ -f "$root/crates/data-go-client/src/lib.rs" ]
[ -f "$root/crates/market-data/src/providers/fsc_krx_listed.rs" ]

echo 'FSC_KRX_LISTED_SHELL_SELF_TEST: PASS offline-only'
