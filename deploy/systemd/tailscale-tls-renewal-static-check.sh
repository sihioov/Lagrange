#!/usr/bin/env bash
# Static contract check for the Tailscale TLS renewal artifacts.
# This never invokes tailscale, Docker, systemd, or the renewal helper.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops="$root/scripts/ops"
systemd_dir="$root/deploy/systemd"
renew="$ops/renew-tailscale-tls.sh"
installer="$ops/install-tailscale-tls-renewal.sh"
self_test="$ops/tailscale-tls-self-test.sh"
service="$systemd_dir/lagrange-tailscale-tls-renewal.service"
timer="$systemd_dir/lagrange-tailscale-tls-renewal.timer"
config="$systemd_dir/tailscale-tls-renewal.conf.example"

die() { echo "tailscale-tls-renewal-static: $*" >&2; exit 1; }

for path in "$renew" "$installer" "$self_test" "$service" "$timer" "$config"; do
  [ -f "$path" ] || die "missing artifact: $path"
done
[ -x "$renew" ] || die 'renewal helper must be executable'
[ -x "$installer" ] || die 'installer must be executable'
[ -x "$self_test" ] || die 'TLS self-test must be executable'
bash -n "$renew" || die 'renewal helper has shell syntax errors'
bash -n "$installer" || die 'installer has shell syntax errors'
bash -n "$self_test" || die 'TLS self-test has shell syntax errors'

grep -Fq 'expected_domain=l1nnx-sh.taild74a33.ts.net' "$renew" \
  || die 'renewal helper fixed domain missing'
grep -Fq 'mode=dry-run' "$renew" || die 'renewal helper must default to dry-run'
grep -Fq -- '--check' "$renew" || die 'renewal helper check mode missing'
grep -Fq -- '--renew' "$renew" || die 'renewal helper explicit renew mode missing'
grep -Fq -- 'flock -n 9' "$renew" || die 'renewal helper single-run lock missing'
grep -Fq -- '--min-validity=720h' "$renew" || die 'Tailscale minimum validity flag missing'
grep -Fq 'source_stage_dir/lagrange.crt' "$renew" || die 'Tailscale must write certificate to staging'
grep -Fq 'source_stage_dir/lagrange.key' "$renew" || die 'Tailscale must write key to staging'
grep -Fq 'checkend_seconds=2592000' "$renew" || die '30-day checkend contract missing'
grep -Fq 'subjectAltName' "$renew" || die 'SAN validation missing'
grep -Fq 'pubkey' "$renew" || die 'public-key extraction missing'
grep -Fq 'cmp -s' "$renew" || die 'public-key or pair comparison missing'
grep -Fq "'0:0:600'" "$renew" || die 'source TLS metadata contract missing'
grep -Fq "'101:101:440'" "$renew" || die 'runtime TLS metadata contract missing'
grep -Fq 'transaction_active' "$renew" || die 'TLS rollback transaction missing'
grep -Fq 'rollback_transaction' "$renew" || die 'TLS rollback implementation missing'
grep -Fq 'absent-no-start' "$renew" || die 'reverse-proxy absent/no-start contract missing'
grep -Fq -- '--no-deps' "$renew" || die 'reverse-proxy refresh must avoid dependencies'
grep -Fq -- '--force-recreate' "$renew" || die 'reverse-proxy refresh must force recreate'
grep -Fq 'reverse-proxy' "$renew" || die 'reverse-proxy identity missing'
grep -Fq 'LAGRANGE_CODE_COMMIT' "$renew" || die 'Compose commit propagation key missing'
grep -Fq 'LAGRANGE_CODE_COMMIT="$code_commit" docker compose' "$renew" \
  || die 'Compose commands must receive the protected code commit'
grep -Fq 'backup.metadata' "$renew" || die 'TLS rollback metadata preservation missing'
grep -Fq 'LAGRANGE_TLS_TEST_FAIL_AFTER_REPLACE' "$renew" \
  || die 'TLS rollback failure injection missing'
for forbidden in kis_app_key kis_app_secret auth0_client_secret psql; do
  if grep -Eiq "^[^#]*$forbidden" "$renew"; then
    die "renewal helper must not reference $forbidden"
  fi
done

grep -Fq 'TLS_DOMAIN=l1nnx-sh.taild74a33.ts.net' "$config" \
  || die 'example config fixed domain missing'
for key in TLS_SOURCE_DIR TLS_RUNTIME_DIR COMPOSE_FILE COMPOSE_ENV_FILE \
  COMPOSE_PROJECT LAGRANGE_CODE_COMMIT LOCK_FILE; do
  grep -Eq "^${key}=" "$config" || die "example config missing $key"
done
grep -Fq 'REPLACE_WITH_40_HEX_CODE_COMMIT' "$config" \
  || die 'example config must visibly require commit customization'

grep -Fq 'Type=oneshot' "$service" || die 'systemd service must be oneshot'
grep -Fq 'User=root' "$service" || die 'systemd service must run renewal as root'
grep -Fq -- '--renew --config-file /etc/lagrange/tailscale-tls-renewal.conf' "$service" \
  || die 'systemd service must invoke explicit renewal with protected config'
grep -Fq 'ProtectSystem=strict' "$service" || die 'systemd service system protection missing'
grep -Fq 'NoNewPrivileges=true' "$service" || die 'systemd service privilege hardening missing'
grep -Fq 'ReadWritePaths=/etc/lagrange/secrets/tls' "$service" \
  || die 'systemd service source write boundary missing'
grep -Fq '/etc/lagrange/secrets/runtime/reverse-proxy' "$service" \
  || die 'systemd service runtime write boundary missing'
grep -Fq 'OnCalendar=*-*-* 03:15:00 Asia/Seoul' "$timer" \
  || die 'systemd timer KST schedule missing'
grep -Fq 'RandomizedDelaySec=1h' "$timer" || die 'systemd timer jitter missing'
grep -Fq 'Persistent=true' "$timer" || die 'systemd timer persistence missing'

grep -Fq 'mode=dry-run' "$installer" || die 'installer must default to dry-run'
grep -Fq 'systemctl daemon-reload' "$installer" || die 'installer daemon-reload action missing'
grep -Fq 'enable lagrange-tailscale-tls-renewal.timer' "$installer" \
  || die 'installer timer enable action missing'
if grep -Fq -- '--now' "$installer" || grep -Eq 'systemctl[[:space:]]+start' "$installer"; then
  die 'installer must not start the renewal timer during apply'
fi
grep -Fq 'config-target already exists' "$installer" \
  || die 'installer protected config overwrite refusal missing'
grep -Fq 'config-source must be root-owned with mode 0600' "$installer" \
  || die 'installer config source protection missing'
grep -Fq 'LAGRANGE_CODE_COMMIT must be exactly 40' "$installer" \
  || die 'installer commit shape validation missing'
if grep -Eq '^[[:space:]]*(tailscale|docker)([[:space:]]|$)' "$installer"; then
  die 'installer must not issue certificates or call Docker'
fi
if grep -Fq -- '--renew' "$installer"; then
  die 'installer must not invoke the renewal helper'
fi

grep -Fq 'TLS_FAIL=1' "$self_test" || die 'TLS self-test failure fixture missing'
grep -Fq 'LAGRANGE_TLS_TEST_FAIL_AFTER_REPLACE' "$self_test" \
  || die 'TLS self-test rollback fixture missing'
grep -Fq 'TLS_DOCKER_RUNNING=0' "$self_test" || die 'TLS self-test absent-proxy fixture missing'
grep -Fq 'proxy_action=absent-no-start' "$self_test" || die 'TLS self-test no-start assertion missing'
grep -Fq 'force-recreate-reverse-proxy' "$self_test" || die 'TLS self-test proxy refresh assertion missing'
grep -Fq 'BEGIN (RSA ' "$self_test" || die 'TLS self-test no-leak assertion missing'
grep -Fq 'commit=$test_commit' "$self_test" || die 'TLS self-test commit propagation assertion missing'
grep -Fq 'config-target already exists' "$self_test" || die 'TLS self-test config overwrite refusal missing'

echo 'TAILSCALE_TLS_RENEWAL_STATIC: PASS'
