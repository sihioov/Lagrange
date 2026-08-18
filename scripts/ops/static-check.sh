#!/usr/bin/env bash
# Static contract check for production operator workflows; no Docker/root/API.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops="$root/scripts/ops"
die() { echo "OPS_STATIC: $*" >&2; exit 1; }

[ -f "$ops/lib/dotenv.sh" ] || die 'shared dotenv helper is missing'
bash -n "$ops/lib/dotenv.sh" || die 'shared dotenv helper has shell syntax errors'
grep -Fq 'uses Compose interpolation, quote, escape' "$ops/lib/dotenv.sh" \
  || die 'dotenv parser must reject Compose interpretation syntax'

for script in provision-linux.sh provision-db-secrets.sh provision-auth0-secret.sh \
  provision-crypto-secrets.sh provision-kis-credentials.sh validate-production-config.sh compose-release.sh \
  backfill-production.sh post-backfill-health.sh self-test.sh renew-tailscale-tls.sh \
  install-tailscale-tls-renewal.sh tailscale-tls-self-test.sh \
  build-production-images.sh build-production-images-static-check.sh \
  build-production-images-self-test.sh deploy-production-release.sh \
  run-production-backup.sh install-production-backup.sh \
  production-ops-static-check.sh production-ops-self-test.sh; do
  path="$ops/$script"
  [ -x "$path" ] || die "$script must be executable"
  [ ! -L "$path" ] || die "$script must not be a symlink"
  bash -n "$path" || die "$script has shell syntax errors"
done

bash "$ops/build-production-images-static-check.sh" >/dev/null ||
  die 'production image build static check failed'
bash "$ops/production-ops-static-check.sh" >/dev/null ||
  die 'production release/backup static check failed'

tls_static="$root/deploy/systemd/tailscale-tls-renewal-static-check.sh"
[ -x "$tls_static" ] || die 'Tailscale TLS renewal static check must be executable'
bash "$tls_static" >/dev/null || die 'Tailscale TLS renewal static check failed'

auth0_secret="$ops/provision-auth0-secret.sh"
grep -Fq 'mode=dry-run' "$auth0_secret" \
  || die 'Auth0 secret provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$auth0_secret" \
  || die 'Auth0 secret read-only check mode missing'
grep -Fq -- '--check must run as root' "$auth0_secret" \
  || die 'Auth0 secret check root guard missing'
grep -Fq -- '--apply must run as root' "$auth0_secret" \
  || die 'Auth0 secret apply root guard missing'
grep -Fq -- '--import-file must run as root' "$auth0_secret" \
  || die 'Auth0 secret import root guard missing'
grep -Fq -- '--import-file' "$auth0_secret" \
  || die 'Auth0 secret import mode missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$auth0_secret" \
  || die 'Auth0 secret source default missing'
grep -Fq 'target_name=auth0_client_secret' "$auth0_secret" \
  || die 'Auth0 secret target name missing'
grep -Fq 'must not contain' "$auth0_secret" \
  || die 'Auth0 secret dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$auth0_secret" \
  || die 'Auth0 secret ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$auth0_secret" \
  || die 'Auth0 secret source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$auth0_secret" \
  || die 'Auth0 secret source write fence missing'
grep -Fq 'read -r -s' "$auth0_secret" \
  || die 'Auth0 secret apply must use hidden terminal input'
grep -Fq '/dev/tty' "$auth0_secret" \
  || die 'Auth0 secret apply must read from a terminal'
grep -Fq 'placeholder_pattern' "$auth0_secret" \
  || die 'Auth0 secret placeholder rejection missing'
grep -Fq 'ln -T' "$auth0_secret" \
  || die 'Auth0 secret atomic no-clobber install missing'
grep -Fq 'AUTH0_SECRET_CHECK: PASS' "$auth0_secret" \
  || die 'Auth0 secret check pass output missing'
grep -Fq "'%u:%g:%a'" "$auth0_secret" \
  || die 'Auth0 secret check ownership/mode inspection missing'
grep -Fq 'wc -c' "$auth0_secret" \
  || die 'Auth0 secret check byte-length inspection missing'
grep -Fq 'legacy Auth0 secret source must not be group/other accessible' "$auth0_secret" \
  || die 'Auth0 legacy source mode fence missing'
grep -Fq 'cp -- "$import_file" "$staged"' "$auth0_secret" \
  || die 'Auth0 import staged-copy fence missing'
for forbidden in curl wget docker psql openssl; do
  if grep -Eiq "^[^#]*($forbidden)" "$auth0_secret"; then
    die "Auth0 secret provisioner must not reference $forbidden"
  fi
done

crypto_secrets="$ops/provision-crypto-secrets.sh"
grep -Fq 'mode=dry-run' "$crypto_secrets" \
  || die 'crypto secret provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$crypto_secrets" \
  || die 'crypto secret read-only check mode missing'
grep -Fq -- '--check must run as root' "$crypto_secrets" \
  || die 'crypto secret check root guard missing'
grep -Fq -- '--apply must run as root' "$crypto_secrets" \
  || die 'crypto secret apply root guard missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$crypto_secrets" \
  || die 'crypto secret source default missing'
grep -Fq 'session_secret' "$crypto_secrets" \
  || die 'session secret inventory missing'
grep -Fq 'csrf_secret' "$crypto_secrets" \
  || die 'CSRF secret inventory missing'
grep -Fq 'cursor_secret' "$crypto_secrets" \
  || die 'cursor secret inventory missing'
grep -Fq 'backup_encryption_key' "$crypto_secrets" \
  || die 'backup encryption key inventory missing'
grep -Fq 'must not contain' "$crypto_secrets" \
  || die 'crypto secret dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$crypto_secrets" \
  || die 'crypto secret ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$crypto_secrets" \
  || die 'crypto secret source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$crypto_secrets" \
  || die 'crypto secret source write fence missing'
grep -Fq 'openssl rand -hex 32' "$crypto_secrets" \
  || die 'crypto secret generator must use 256-bit OpenSSL values'
grep -Fq 'cmp -s' "$crypto_secrets" \
  || die 'crypto secret distinctness check missing'
grep -Fq 'CRYPTO_SECRET_CHECK: PASS' "$crypto_secrets" \
  || die 'crypto secret check pass output missing'
grep -Fq 'ln -T' "$crypto_secrets" \
  || die 'crypto secret atomic no-clobber install missing'
grep -Fq "'%u:%g:%a'" "$crypto_secrets" \
  || die 'crypto secret ownership/mode inspection missing'
grep -Fq 'wc -c' "$crypto_secrets" \
  || die 'crypto secret shape inspection missing'
for forbidden in curl wget docker psql; do
  if grep -Eiq "^[^#]*($forbidden)" "$crypto_secrets"; then
    die "crypto secret provisioner must not reference $forbidden"
  fi
done

kis_credentials="$ops/provision-kis-credentials.sh"
grep -Fq 'mode=dry-run' "$kis_credentials" \
  || die 'KIS credential provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$kis_credentials" \
  || die 'KIS credential read-only check mode missing'
grep -Fq -- '--check must run as root' "$kis_credentials" \
  || die 'KIS credential check root guard missing'
grep -Fq -- '--apply must run as root' "$kis_credentials" \
  || die 'KIS credential apply root guard missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$kis_credentials" \
  || die 'KIS credential source default missing'
grep -Fq 'key_name=kis_app_key' "$kis_credentials" \
  || die 'KIS app-key inventory missing'
grep -Fq 'secret_name=kis_app_secret' "$kis_credentials" \
  || die 'KIS app-secret inventory missing'
grep -Fq 'must not contain' "$kis_credentials" \
  || die 'KIS credential dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$kis_credentials" \
  || die 'KIS credential ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$kis_credentials" \
  || die 'KIS credential source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$kis_credentials" \
  || die 'KIS credential source write fence missing'
grep -Fq 'read -r -s -u 3' "$kis_credentials" \
  || die 'KIS credential apply must use hidden terminal input'
grep -Fq '/dev/tty' "$kis_credentials" \
  || die 'KIS credential apply must read from a terminal'
grep -Fq 'placeholder_pattern' "$kis_credentials" \
  || die 'KIS credential placeholder rejection missing'
grep -Fq 'max_secret_bytes=4096' "$kis_credentials" \
  || die 'KIS credential local length guard missing'
grep -Fq 'cmp -s' "$kis_credentials" \
  || die 'KIS credential pair distinctness check missing'
grep -Fq 'ln -T' "$kis_credentials" \
  || die 'KIS credential atomic no-clobber install missing'
grep -Fq 'installed_signatures' "$kis_credentials" \
  || die 'KIS credential pair rollback tracking missing'
grep -Fq 'KIS_CREDENTIAL_CHECK: PASS' "$kis_credentials" \
  || die 'KIS credential check pass output missing'
grep -Fq 'source directory is absent or protected from current user' "$kis_credentials" \
  || die 'KIS credential dry-run must not infer absence from access denial'
grep -Fq "'%u:%g:%a'" "$kis_credentials" \
  || die 'KIS credential ownership/mode inspection missing'
grep -Fq 'wc -c' "$kis_credentials" \
  || die 'KIS credential byte-length inspection missing'
grep -Fq 'wc -l' "$kis_credentials" \
  || die 'KIS credential newline-shape inspection missing'
grep -Fq 'install -o root -g root -m 0600' "$kis_credentials" \
  || die 'KIS credential owner/mode install fence missing'
for forbidden in curl wget docker psql openssl tailscale; do
  if grep -Eiq "^[^#]*($forbidden)" "$kis_credentials"; then
    die "KIS credential provisioner must not reference $forbidden"
  fi
done

db_secrets="$ops/provision-db-secrets.sh"
grep -Fq 'mode=dry-run' "$db_secrets" \
  || die 'DB secret provisioner must default to a dry-run plan'
grep -Fq 'mode=check' "$db_secrets" \
  || die 'DB secret read-only check mode missing'
grep -Fq -- '--check must run as root' "$db_secrets" \
  || die 'DB secret check root guard missing'
grep -Fq 'mode=normalize' "$db_secrets" \
  || die 'DB secret newline normalizer mode missing'
grep -Fq -- '--strip-trailing-newline' "$db_secrets" \
  || die 'DB secret newline normalizer option missing'
grep -Fq -- '--apply must run as root' "$db_secrets" \
  || die 'DB secret apply root guard missing'
grep -Fq 'default_source_dir=/etc/lagrange/secrets' "$db_secrets" \
  || die 'DB secret source default missing'
grep -Fq 'must not contain' "$db_secrets" \
  || die 'DB secret dot-dot path fence missing'
grep -Fq 'must not traverse a symlink' "$db_secrets" \
  || die 'DB secret ancestor symlink fence missing'
grep -Fq 'source directory must be owned by uid 0' "$db_secrets" \
  || die 'DB secret source ownership fence missing'
grep -Fq 'source directory must not be group/other writable' "$db_secrets" \
  || die 'DB secret source write fence missing'
grep -Fq 'source_mode_bits & 0022' "$db_secrets" \
  || die 'DB secret source write mask must preserve group/other read access'
grep -Fq 'openssl rand -hex 32' "$db_secrets" \
  || die 'DB secret generator must use 256-bit OpenSSL values'
grep -Fq 'cmp -s' "$db_secrets" \
  || die 'DB secret distinctness check missing'
grep -Fq 'cmp -s --' "$db_secrets" \
  || die 'DB secret read-only equality check must use silent cmp'
grep -Fq 'DB_SECRET_CHECK: PASS' "$db_secrets" \
  || die 'DB secret check pass output missing'
grep -Fq 'DB_SECRET_NORMALIZE: PASS' "$db_secrets" \
  || die 'DB secret normalizer pass output missing'
grep -Fq 'base64 --decode' "$db_secrets" \
  || die 'DB secret Base64 decoder check missing'
grep -Fq 'has_single_trailing_newline' "$db_secrets" \
  || die 'DB secret newline-shape check missing'
grep -Fq 'mv -T' "$db_secrets" \
  || die 'DB secret normalizer atomic replacement missing'
grep -Fq "'%u:%g:%a'" "$db_secrets" \
  || die 'DB secret check ownership/mode inspection missing'
grep -Fq "wc -c <\"\$target\"" "$db_secrets" \
  || die 'DB secret check byte-length inspection missing'
grep -Fq 'install -o root -g root -m 0600' "$db_secrets" \
  || die 'DB secret owner fence missing'
grep -Fq '0600' "$db_secrets" \
  || die 'DB secret mode fence missing'
for forbidden in docker curl psql kis api; do
  if grep -Eiq "^[^#]*(\\$forbidden|$forbidden)" "$db_secrets"; then
    die "DB secret provisioner must not reference $forbidden"
  fi
done

grep -Fq 'DRY_RUN: no host changes made' "$ops/provision-linux.sh" || die 'provision dry-run contract missing'
grep -Fq -- '--apply must run as root' "$ops/provision-linux.sh" || die 'provision root guard missing'
grep -Fq -- '--preflight must run as root' "$ops/provision-linux.sh" || die 'provision preflight root guard missing'
grep -Fq 'must not traverse a symlink' "$ops/provision-linux.sh" || die 'provision ancestor symlink fence missing'
grep -Fq 'service user is not a member of service group' "$ops/provision-linux.sh" || die 'service group membership fence missing'
grep -Fq 'BLOCKED_EXTERNAL' "$ops/validate-production-config.sh" || die 'config blocker contract missing'
grep -Fq -- '--scope infrastructure|serving-prereqs|backfill|release' "$ops/validate-production-config.sh" || die 'config scope contract missing'
grep -Fq -- 'validation must run as root to inspect protected production paths' "$ops/validate-production-config.sh" \
  || die 'config validator root guard missing'
grep -Fq 'LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT"' "$ops/validate-production-config.sh" \
  || die 'config validator sudo commit-preservation guidance missing'
grep -Fq 'validator fixture checks skipped for non-root caller' "$ops/self-test.sh" \
  || die 'self-test must account for the validator root contract'
grep -Fq 'dotenv_validate_shell_overrides' "$ops/validate-production-config.sh" || die 'shell/env-file precedence fence missing'
grep -Fq 'KIS read-only' "$ops/validate-production-config.sh" || die 'KIS read-only contract missing'
grep -Fq 'mode 0400 or 0600' "$ops/validate-production-config.sh" || die 'source secret mode contract missing'
grep -Fq 'runtime secret' "$ops/validate-production-config.sh" || die 'runtime secret validation missing'
grep -Fq 'serving-prereqs scope checks Auth0/TLS' "$ops/validate-production-config.sh" \
  || die 'serving-prereqs readiness contract missing'
grep -Fq 'backup_encryption_key' "$ops/validate-production-config.sh" \
  || die 'serving-prereqs source inventory missing backup key'
grep -Fq 'research-worker/db_research_password:10001:10001:440' "$ops/validate-production-config.sh" \
  || die 'serving-prereqs runtime inventory missing research DB copy'
grep -Fq 'crypto_placeholder_pattern' "$ops/validate-production-config.sh" \
  || die 'validator crypto placeholder contract is missing'
grep -Fq "grep -Eq '^[0-9a-f]{64}$'" "$ops/validate-production-config.sh" \
  || die 'validator crypto lowercase-hex contract is missing'
grep -Fq 'crypto source secrets must be distinct' "$ops/validate-production-config.sh" \
  || die 'validator crypto distinctness contract is missing'
grep -Fq 'db_secret_names=' "$ops/validate-production-config.sh" || die 'DB secret distinctness inventory missing'
grep -Fq 'DB source secrets must be distinct' "$ops/validate-production-config.sh" || die 'DB secret distinctness blocker missing'
grep -Fq 'cmp -s' "$ops/validate-production-config.sh" || die 'DB secret equality check missing'
grep -Fq 'run --rm --no-deps db-role-bootstrap' "$ops/compose-release.sh" || die 'role bootstrap ordering missing'
grep -Fq 'run --rm --no-deps db-migrate' "$ops/compose-release.sh" || die 'migration ordering missing'
grep -Fq 'build --pull=false \' "$ops/compose-release.sh" || die 'Compose build gate missing'
grep -Fq 'db-role-bootstrap db-migrate' "$ops/compose-release.sh" || die 'one-shot images are not built before run'
grep -Fq 'up --wait --no-deps api-server' "$ops/compose-release.sh" || die 'serving stage must not rerun removed one-shots'
grep -Fq -- '--scope infrastructure|backfill|release' "$ops/compose-release.sh" || die 'Compose scope contract missing'
if grep -Fq 'serving-prereqs' "$ops/compose-release.sh"; then
  die 'serving-prereqs must remain copy/readiness-only and absent from Compose execution'
fi
grep -Fq 'LAGRANGE_DATA_ROOT="$data_dir"' "$ops/compose-release.sh" || die 'Compose preflight must use env-file data root'
grep -Fq 'COMPOSE_BACKFILL_BOOTSTRAP_ORDER' "$ops/compose-release.sh" || die 'backfill Compose bootstrap order missing'
grep -Fq 'COMPOSE_INFRASTRUCTURE_ORDER' "$ops/compose-release.sh" || die 'infrastructure Compose order missing'
grep -Fq 'compose build --pull=false db-role-bootstrap db-migrate' "$ops/compose-release.sh" \
  || die 'infrastructure Compose build gate missing'
grep -Fq 'COMPOSE_INFRASTRUCTURE: PASS' "$ops/compose-release.sh" \
  || die 'infrastructure Compose apply gate missing'
grep -Fq 'RESEARCH_APP_ENV=infrastructure-disabled' "$ops/compose-release.sh" \
  || die 'infrastructure Compose research sentinel missing'
grep -Fq 'RESEARCH_ENTITLEMENT_REFERENCE=infrastructure-disabled' "$ops/compose-release.sh" \
  || die 'infrastructure Compose entitlement sentinel missing'
for key in BACKTEST_MIN_FREE_BYTES BACKTEST_MAX_QUEUED_BACKTESTS \
  BACKTEST_RECONCILE_GRACE_SECS BACKTEST_RECONCILE_INTERVAL_SECS; do
  grep -Fq "$key=0" "$ops/compose-release.sh" \
    || die "infrastructure Compose $key sentinel missing"
done
grep -Fq 'process-local, fail-closed sentinels' "$ops/compose-release.sh" \
  || die 'infrastructure Compose sentinel scope documentation missing'
grep -Fq 'up --no-deps -d research-worker recommendation-runner candidate-runner' "$ops/compose-release.sh" \
  || die 'data-dependent services must bootstrap without a clean-install health wait'
grep -Fq 'post-backfill-health.sh --check' "$ops/compose-release.sh" \
  || die 'post-backfill data readiness gate is not documented in Compose release'
grep -Fq 'research-worker healthcheck' "$ops/post-backfill-health.sh" \
  || die 'post-backfill gate must invoke the existing worker healthcheck'
[ "$(stat -c '%a' "$ops/post-backfill-health.sh")" = 755 ] \
  || die 'post-backfill-health.sh must have exact mode 0755'
grep -Fq -- '--scope backfill|release' "$ops/post-backfill-health.sh" \
  || die 'post-backfill scope contract missing'
grep -Fq 'run --rm --no-deps research-worker healthcheck' "$ops/post-backfill-health.sh" \
  || die 'post-backfill gate must avoid dependency restarts'
grep -Fq 'does not require a worker daemon' "$ops/post-backfill-health.sh" \
  || die 'post-backfill gate must not require a worker daemon'
grep -Fq 'PLAN_ONLY: no KIS call' "$ops/backfill-production.sh" || die 'backfill must default to no-call plan'
grep -Fq 'KOSPI200/KOSDAQ150 credentialed candidate bridge' "$ops/backfill-production.sh" || die 'candidate blocker missing'
grep -Fq 'LAGRANGE_BACKFILL_STATE_V3' "$ops/backfill-production.sh" || die 'backfill state identity schema missing'
grep -Fq -- '--scope backfill' "$ops/backfill-production.sh" || die 'backfill must use backfill config scope'
grep -Fq 'state_file="$data_dir/backfill/state.tsv"' "$ops/backfill-production.sh" \
  || die 'backfill state default must derive from LAGRANGE_DATA_DIR'
grep -Fq 'dotenv_validate_shell_overrides' "$ops/backfill-production.sh" \
  || die 'backfill must share shell/env-file precedence fence'
grep -Fq 'start_date=$start_date' "$ops/backfill-production.sh" || die 'backfill identity must bind the requested date range'
grep -Fq 'dataset_version_id' "$ops/backfill-production.sh" && die 'backfill identity must not bind future dataset pins'
grep -Fq 'flock -n 9' "$ops/backfill-production.sh" || die 'backfill state lock missing'
grep -Fq -- '--backfill-range --start "$start_date" --end "$end_date"' \
  "$ops/backfill-production.sh" || die 'backfill must use one bounded range worker process'
if grep -Fq -- 'research-worker --once --date "$date"' "$ops/backfill-production.sh"; then
  die 'backfill must not create one token-owning worker process per date'
fi
grep -Fq 'ps --status running --services' "$ops/backfill-production.sh" \
  || die 'backfill must refuse a concurrently running research-worker daemon'
grep -Fq 'token_window_file="${state_file}.token-window"' "$ops/backfill-production.sh" \
  || die 'backfill cross-process token issue window missing'
grep -Fq 'chmod 0600 "$token_window_tmp"' "$ops/backfill-production.sh" \
  || die 'backfill token issue window mode contract missing'
grep -Fq 'MIN_ISSUE_INTERVAL_MS: i64 = 60_000' "$root/crates/kis-client/src/auth.rs" \
  || die 'KIS token manager one-minute issue safeguard missing'
grep -Fq 'DEFAULT_TTL_SECS: i64 = 86_400' "$root/crates/kis-client/src/token_issuer.rs" \
  || die 'KIS token fallback TTL must match the documented 24-hour lifetime'
grep -Fq 'backfill-progress.py' "$ops/backfill-production.sh" \
  || die 'backfill must durably consume per-date worker progress'
grep -Fq 'os.fsync(state.fileno())' "$ops/lib/backfill-progress.py" \
  || die 'backfill per-date progress must be durable before the next date'
grep -Fq 'record.get("phase") == "canonical_publication"' \
  "$ops/lib/backfill-progress.py" \
  || die 'backfill progress must distinguish canonical EOD from final Curated recovery'
if grep -Eq 'compose[^#]*--profile[[:space:]]+live|--profile[[:space:]]+live' "$ops"/*.sh; then
  die 'operator workflow must not enable the live profile'
fi
echo 'OPS_STATIC: PASS'
