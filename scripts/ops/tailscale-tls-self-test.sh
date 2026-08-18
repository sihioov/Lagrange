#!/usr/bin/env bash
# Fake-command self-test for the Tailscale TLS renewal artifacts.
# It never invokes a host tailscale, Docker, or systemd command.  The root
# fixture path is skipped for an unprivileged caller because the production
# contract intentionally requires root-owned source/runtime files.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
renew="$script_dir/renew-tailscale-tls.sh"
installer="$script_dir/install-tailscale-tls-renewal.sh"

dry_run=$(bash "$renew" --dry-run)
grep -Fq 'TLS_RENEWAL_PLAN mode=dry-run' <<<"$dry_run"
grep -Fq 'DRY_RUN:' <<<"$dry_run"

if [ "$(id -u)" -ne 0 ]; then
  if [ "${LAGRANGE_TLS_FAKEROOT_CHILD:-0}" != 1 ] && command -v fakeroot >/dev/null 2>&1; then
    export LAGRANGE_TLS_FAKEROOT_CHILD=1
    exec fakeroot bash -c '
      id() {
        if [ "${1:-}" = -u ]; then
          echo 0
        else
          command id "$@"
        fi
      }
      export -f id
      exec bash "$1" "${@:2}"
    ' _ "$script_dir/tailscale-tls-self-test.sh" "$@"
  fi
  echo 'TAILSCALE_TLS_SELF_TEST: PASS (root fixture skipped; fakeroot unavailable)'
  exit 0
fi

out_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-tailscale-tls-self-test.XXXXXX")
trap 'rm -rf -- "$out_dir"' EXIT

source_dir="$out_dir/etc/lagrange/secrets/tls"
runtime_dir="$out_dir/etc/lagrange/secrets/runtime/reverse-proxy"
compose_dir="$out_dir/opt/lagrange/deploy/compose"
lock_dir="$out_dir/run/lock"
config="$out_dir/etc/lagrange/tailscale-tls-renewal.conf"
fake_bin="$out_dir/fake-bin"
tailscale_log="$out_dir/tailscale.log"
openssl_log="$out_dir/openssl.log"
docker_log="$out_dir/docker.log"
systemctl_log="$out_dir/systemctl.log"
test_commit=0123456789abcdef0123456789abcdef01234567

mkdir -p "$source_dir" "$runtime_dir" "$compose_dir" "$lock_dir" \
  "$(dirname "$config")" "$fake_bin"
chown root:root "$source_dir" "$(dirname "$config")" "$compose_dir" "$lock_dir" "$out_dir"
chmod 0750 "$source_dir" "$(dirname "$config")" "$compose_dir" "$lock_dir"
chown 101:101 "$runtime_dir"
chmod 0750 "$runtime_dir"
printf 'services: {}\n' >"$compose_dir/compose.yml"
printf 'COMPOSE_TEST=1\n' >"$compose_dir/.env"
chown root:root "$compose_dir/compose.yml" "$compose_dir/.env"
chmod 0600 "$compose_dir/compose.yml" "$compose_dir/.env"

# The old pair is intentionally inside the 30-day renewal window.  The fake
# tailscale command supplies a fresh pair, so the first run exercises staging,
# validation, atomic replacement, runtime metadata, and proxy recreation.
old_cert="$out_dir/old.crt"
old_key="$out_dir/old.key"
new_cert="$out_dir/new.crt"
new_key="$out_dir/new.key"
openssl_bin=/usr/bin/openssl
"$openssl_bin" req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj '/CN=l1nnx-sh.taild74a33.ts.net' \
  -addext 'subjectAltName=DNS:l1nnx-sh.taild74a33.ts.net' \
  -keyout "$old_key" -out "$old_cert" >/dev/null 2>&1
"$openssl_bin" req -x509 -newkey rsa:2048 -nodes -days 90 \
  -subj '/CN=l1nnx-sh.taild74a33.ts.net' \
  -addext 'subjectAltName=DNS:l1nnx-sh.taild74a33.ts.net' \
  -keyout "$new_key" -out "$new_cert" >/dev/null 2>&1

cp -- "$old_cert" "$source_dir/lagrange.crt"
cp -- "$old_key" "$source_dir/lagrange.key"
chown root:root "$source_dir/lagrange.crt" "$source_dir/lagrange.key"
chmod 0600 "$source_dir/lagrange.crt" "$source_dir/lagrange.key"
cp -- "$old_cert" "$runtime_dir/lagrange_tls_cert"
cp -- "$old_key" "$runtime_dir/lagrange_tls_key"
chown 101:101 "$runtime_dir/lagrange_tls_cert" "$runtime_dir/lagrange_tls_key"
chmod 0440 "$runtime_dir/lagrange_tls_cert" "$runtime_dir/lagrange_tls_key"

cat >"$config" <<EOF
TLS_DOMAIN=l1nnx-sh.taild74a33.ts.net
TLS_SOURCE_DIR=$source_dir
TLS_RUNTIME_DIR=$runtime_dir
COMPOSE_FILE=$compose_dir/compose.yml
COMPOSE_ENV_FILE=$compose_dir/.env
COMPOSE_PROJECT=lagrange-test
LAGRANGE_CODE_COMMIT=$test_commit
LOCK_FILE=$lock_dir/renew.lock
EOF
chown root:root "$config"
chmod 0600 "$config"

cat >"$fake_bin/tailscale" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$TLS_TAILSCALE_LOG"
[ "${TLS_FAIL:-0}" = 1 ] && exit 1
cert_path=
key_path=
for arg in "$@"; do
  case "$arg" in
    --cert-file=*) cert_path=${arg#--cert-file=} ;;
    --key-file=*) key_path=${arg#--key-file=} ;;
  esac
done
[ -n "$cert_path" ] && [ -n "$key_path" ]
cp -- "$TLS_NEW_CERT" "$cert_path"
cp -- "$TLS_NEW_KEY" "$key_path"
EOF
cat >"$fake_bin/openssl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$TLS_OPENSSL_LOG"
exec /usr/bin/openssl "$@"
EOF
cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'commit=%s args=%s\n' "${LAGRANGE_CODE_COMMIT:-missing}" "$*" >>"$TLS_DOCKER_LOG"
joined=$*
if [[ "$joined" == *' ps '* ]] && [ "${TLS_DOCKER_RUNNING:-0}" = 1 ]; then
  printf '%s\n' reverse-proxy
fi
EOF
cat >"$fake_bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$TLS_SYSTEMCTL_LOG"
EOF
chmod 0755 "$fake_bin"/*

export PATH="$fake_bin:$PATH"
export TLS_TAILSCALE_LOG="$tailscale_log" TLS_OPENSSL_LOG="$openssl_log"
export TLS_DOCKER_LOG="$docker_log" TLS_SYSTEMCTL_LOG="$systemctl_log"
export TLS_NEW_CERT="$new_cert" TLS_NEW_KEY="$new_key"

reset_old_pair() {
  cp -- "$old_cert" "$source_dir/lagrange.crt"
  cp -- "$old_key" "$source_dir/lagrange.key"
  chown root:root "$source_dir/lagrange.crt" "$source_dir/lagrange.key"
  chmod 0600 "$source_dir/lagrange.crt" "$source_dir/lagrange.key"
  cp -- "$old_cert" "$runtime_dir/lagrange_tls_cert"
  cp -- "$old_key" "$runtime_dir/lagrange_tls_key"
  chown 101:101 "$runtime_dir/lagrange_tls_cert" "$runtime_dir/lagrange_tls_key"
  chmod 0440 "$runtime_dir/lagrange_tls_cert" "$runtime_dir/lagrange_tls_key"
}

TLS_DOCKER_RUNNING=1 bash "$renew" --renew --config-file "$config" >"$out_dir/renew.out"
grep -Fq 'TLS_RENEWAL: PASS' "$out_dir/renew.out"
grep -Fq 'proxy_action=force-recreate-reverse-proxy' "$out_dir/renew.out"
grep -Fq -- '--min-validity=720h' "$tailscale_log"
grep -Fq -- '--force-recreate' "$docker_log"
grep -Fq -- '--no-deps' "$docker_log"
grep -Fq -- 'reverse-proxy' "$docker_log"
grep -Fq -- "commit=$test_commit" "$docker_log"
[ "$(grep -Fc "commit=$test_commit args=" "$docker_log")" -eq 2 ]
[ "$(stat -c '%u:%g:%a' "$source_dir/lagrange.crt")" = '0:0:600' ]
[ "$(stat -c '%u:%g:%a' "$runtime_dir/lagrange_tls_cert")" = '101:101:440' ]
cmp -s "$new_cert" "$source_dir/lagrange.crt"
cmp -s "$new_key" "$source_dir/lagrange.key"
cmp -s "$source_dir/lagrange.crt" "$runtime_dir/lagrange_tls_cert"
cmp -s "$source_dir/lagrange.key" "$runtime_dir/lagrange_tls_key"
if grep -Eiq 'BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|TLS_SECRET_FIXTURE' \
  "$tailscale_log" "$openssl_log" "$docker_log" "$out_dir/renew.out"; then
  echo 'self-test: TLS private value leaked into command/output logs' >&2
  exit 1
fi

: >"$tailscale_log"
: >"$docker_log"
TLS_DOCKER_RUNNING=1 bash "$renew" --renew --config-file "$config" >"$out_dir/noop.out"
grep -Fq 'TLS_RENEWAL: NOOP' "$out_dir/noop.out"
[ ! -s "$tailscale_log" ]
[ ! -s "$docker_log" ]

# A failed issuance must leave the old source/runtime pair intact.  This also
# proves that tailscale never receives a final production path.
reset_old_pair
if TLS_FAIL=1 TLS_DOCKER_RUNNING=1 bash "$renew" --renew --config-file "$config" \
  >"$out_dir/failure.out" 2>&1; then
  echo 'self-test: failed fake tailscale renewal unexpectedly passed' >&2
  exit 1
fi
cmp -s "$old_cert" "$source_dir/lagrange.crt"
cmp -s "$old_key" "$source_dir/lagrange.key"
cmp -s "$old_cert" "$runtime_dir/lagrange_tls_cert"
cmp -s "$old_key" "$runtime_dir/lagrange_tls_key"
if grep -Eiq 'BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|TLS_SECRET_FIXTURE' \
  "$out_dir/failure.out" "$tailscale_log"; then
  echo 'self-test: failure path leaked TLS private value' >&2
  exit 1
fi

# Inject failure after each final replacement.  Rollback must restore both
# bytes and the original source/runtime ownership and mode, including the
# numeric 101:101 runtime pair.
for fail_after in 1 2 3 4; do
  reset_old_pair
  if LAGRANGE_TLS_TEST_FAIL_AFTER_REPLACE="$fail_after" \
    TLS_DOCKER_RUNNING=1 bash "$renew" --renew --config-file "$config" \
    >"$out_dir/rollback-$fail_after.out" 2>&1; then
    echo "self-test: replacement failure injection $fail_after unexpectedly passed" >&2
    exit 1
  fi
  cmp -s "$old_cert" "$source_dir/lagrange.crt"
  cmp -s "$old_key" "$source_dir/lagrange.key"
  cmp -s "$old_cert" "$runtime_dir/lagrange_tls_cert"
  cmp -s "$old_key" "$runtime_dir/lagrange_tls_key"
  [ "$(stat -c '%u:%g:%a' "$source_dir/lagrange.crt")" = '0:0:600' ]
  [ "$(stat -c '%u:%g:%a' "$source_dir/lagrange.key")" = '0:0:600' ]
  [ "$(stat -c '%u:%g:%a' "$runtime_dir/lagrange_tls_cert")" = '101:101:440' ]
  [ "$(stat -c '%u:%g:%a' "$runtime_dir/lagrange_tls_key")" = '101:101:440' ]
done

# A valid source with a stale runtime pair should repair only runtime files;
# with no running proxy, the helper must not start anything.
cp -- "$new_cert" "$source_dir/lagrange.crt"
cp -- "$new_key" "$source_dir/lagrange.key"
chown root:root "$source_dir/lagrange.crt" "$source_dir/lagrange.key"
chmod 0600 "$source_dir/lagrange.crt" "$source_dir/lagrange.key"
: >"$docker_log"
: >"$tailscale_log"
TLS_DOCKER_RUNNING=0 bash "$renew" --renew --config-file "$config" >"$out_dir/absent.out"
grep -Fq 'proxy_action=absent-no-start' "$out_dir/absent.out"
[ ! -s "$tailscale_log" ]
if grep -Fq ' up ' "$docker_log"; then
  echo 'self-test: absent reverse-proxy was unexpectedly started' >&2
  exit 1
fi
cmp -s "$new_cert" "$runtime_dir/lagrange_tls_cert"
cmp -s "$new_key" "$runtime_dir/lagrange_tls_key"

bash "$renew" --check --config-file "$config" >"$out_dir/check.out"
grep -Fq 'TLS_CHECK: PASS domain=l1nnx-sh.taild74a33.ts.net' "$out_dir/check.out"

# Exercise the explicit installer only against the fixture tree.  The fake
# systemctl log proves the apply phase performs only daemon-reload/timer enable;
# the real host units and service state are never touched by this test.
install_bin="$out_dir/opt/lagrange/bin"
systemd_target="$out_dir/etc/systemd/system"
mkdir -p "$install_bin" "$systemd_target"
chmod 0750 "$install_bin" "$systemd_target"
TLS_DOCKER_RUNNING=0 bash "$installer" --apply \
  --install-bin "$install_bin" --systemd-dir "$systemd_target" \
  --config-target "$out_dir/etc/lagrange/installed.conf" \
  --config-source "$config" \
  --helper-source "$renew" \
  --service-source "$root/deploy/systemd/lagrange-tailscale-tls-renewal.service" \
  --timer-source "$root/deploy/systemd/lagrange-tailscale-tls-renewal.timer" \
  >"$out_dir/install.out"
grep -Fq 'daemon-reload' "$systemctl_log"
grep -Fq 'enable lagrange-tailscale-tls-renewal.timer' "$systemctl_log"
if grep -Fq -- '--now' "$systemctl_log" || grep -Fq ' start ' "$systemctl_log"; then
  echo 'self-test: installer unexpectedly started the timer or a service' >&2
  exit 1
fi
[ "$(stat -c '%u:%g:%a' "$install_bin/renew-tailscale-tls.sh")" = '0:0:755' ]
[ "$(stat -c '%u:%g:%a' "$out_dir/etc/lagrange/installed.conf")" = '0:0:600' ]

if bash "$installer" --apply \
  --install-bin "$install_bin" --systemd-dir "$systemd_target" \
  --config-target "$out_dir/etc/lagrange/installed.conf" \
  --config-source "$config" \
  --helper-source "$renew" \
  --service-source "$root/deploy/systemd/lagrange-tailscale-tls-renewal.service" \
  --timer-source "$root/deploy/systemd/lagrange-tailscale-tls-renewal.timer" \
  >"$out_dir/install-existing.out" 2>&1; then
  echo 'self-test: installer unexpectedly overwrote an existing config target' >&2
  exit 1
fi
grep -Fq 'config-target already exists' "$out_dir/install-existing.out"

if bash "$installer" --apply \
  --install-bin "$out_dir/opt/lagrange/bin-invalid" \
  --systemd-dir "$out_dir/etc/systemd/invalid" \
  --config-target "$out_dir/etc/lagrange/invalid.conf" \
  --config-source "$root/deploy/systemd/tailscale-tls-renewal.conf.example" \
  --helper-source "$renew" \
  --service-source "$root/deploy/systemd/lagrange-tailscale-tls-renewal.service" \
  --timer-source "$root/deploy/systemd/lagrange-tailscale-tls-renewal.timer" \
  >"$out_dir/install-placeholder.out" 2>&1; then
  echo 'self-test: installer unexpectedly accepted the tracked placeholder config' >&2
  exit 1
fi
grep -Fq 'config-source must be root-owned with mode 0600' "$out_dir/install-placeholder.out"

echo 'TAILSCALE_TLS_SELF_TEST: PASS'
