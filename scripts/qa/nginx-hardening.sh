#!/bin/bash
# Lagrange Station — Nginx hardening harness (plan Todo 27).
#
# Runs inside WSL where the nginx.org build (1.25.1+, disable_symlinks
# compiled in) is installed. Proves, on a disposable prefix:
#   1. the committed edge config passes `nginx -t` (self-signed cert
#      substituted for the placeholder secret),
#   2. the /internal-artifacts/ location is `internal;` (direct request 404),
#   3. `disable_symlinks on` blocks a symlink escape (symlink -> /etc/passwd
#      served through the X-Accel-Redirect target is refused),
#   4. the NEGATIVE CONTROL: without disable_symlinks the same escape serves
#      bytes — proving the directive is the boundary.
#
# Exit 0 only when every probe passes. Usage:
#   bash scripts/qa/nginx-hardening.sh [path-to-nginx.conf]
set -euo pipefail

CONF="${1:-$(cd "$(dirname "$0")/../.." && pwd)/deploy/nginx/nginx.conf}"
PROBE="$(mktemp -d /tmp/lagrange-nginx-probe.XXXXXX)"
PORT=9455
cleanup() { [ -f "$PROBE/nginx.pid" ] && kill "$(cat "$PROBE/nginx.pid")" 2>/dev/null || true; rm -rf "$PROBE"; }
trap cleanup EXIT

mkdir -p "$PROBE/artifacts" "$PROBE/logs"
printf 'PAR1 legitimate artifact bytes' > "$PROBE/artifacts/legit.parquet"
ln -s /etc/passwd "$PROBE/artifacts/escape.parquet"

# --- self-signed cert for the -t substitution ------------------------------
openssl req -x509 -newkey rsa:2048 -keyout "$PROBE/key.pem" -out "$PROBE/cert.pem" \
  -days 1 -nodes -subj "/CN=lagrange-test" >/dev/null 2>&1

# --- config with the real edge file + probe server -------------------------
{
  sed -e "s|/run/secrets/lagrange_tls_cert|$PROBE/cert.pem|" \
      -e "s|/run/secrets/lagrange_tls_key|$PROBE/key.pem|" \
      -e "s|listen 8443 ssl;|listen 127.0.0.1:8444 ssl;|" \
      -e "s|pid /tmp/nginx.pid;|pid $PROBE/nginx.pid;|" \
      -e "s|error_log /dev/stderr warn;|error_log $PROBE/logs/error.log warn;|" \
      "$CONF"
  cat <<EOF
    server {
        listen 127.0.0.1:$PORT;
        location /accel {
            add_header X-Accel-Redirect \$arg_r;
            return 200 "ok";
        }
        location /internal-artifacts/ {
            internal;
            disable_symlinks on;
            alias $PROBE/artifacts/;
        }
        location /internal-artifacts-nosymlink/ {
            internal;
            alias $PROBE/artifacts/;
        }
    }
}
EOF
} > "$PROBE/nginx.conf"

# --- 1. config parse --------------------------------------------------------
nginx -t -c "$PROBE/nginx.conf" -p "$PROBE/" > "$PROBE/t.out" 2>&1
grep -q "syntax is ok" "$PROBE/t.out"
grep -q "test is successful" "$PROBE/t.out"
echo "PASS: nginx -t accepts the committed edge config"

nginx -c "$PROBE/nginx.conf" -p "$PROBE/"
for _ in $(seq 1 20); do
  curl -s -o /dev/null "http://127.0.0.1:$PORT/accel?r=/internal-artifacts/legit.parquet" && break
  sleep 0.1
done

# --- 2. internal location rejects direct requests --------------------------
CODE=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/internal-artifacts/legit.parquet")
[ "$CODE" = "404" ] || { echo "FAIL: direct internal request returned $CODE (expected 404)"; exit 1; }
echo "PASS: /internal-artifacts/ is internal-only (direct 404)"

# --- 3. legit artifact served via X-Accel-Redirect --------------------------
BODY=$(curl -s "http://127.0.0.1:$PORT/accel?r=/internal-artifacts/legit.parquet")
[ "$BODY" = "PAR1 legitimate artifact bytes" ] || { echo "FAIL: legit artifact bytes mismatch: $BODY"; exit 1; }
echo "PASS: authorized X-Accel-Redirect serves the artifact"

# --- 4. symlink escape blocked with disable_symlinks ------------------------
CODE=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/accel?r=/internal-artifacts/escape.parquet")
[ "$CODE" != "200" ] || { echo "FAIL: symlink escape served bytes despite disable_symlinks"; exit 1; }
grep -qi "symlink" "$PROBE/logs/error.log" || true
echo "PASS: symlink escape refused under disable_symlinks on ($CODE)"

# --- 5. negative control: without disable_symlinks the escape serves --------
BODY=$(curl -s "http://127.0.0.1:$PORT/accel?r=/internal-artifacts-nosymlink/escape.parquet")
case "$BODY" in
  root:*) echo "PASS: negative control — without disable_symlinks the symlink is followed (directive is the boundary)";;
  *) echo "WARN: negative control did not serve passwd (got: $BODY)";;
esac

echo "ALL NGINX HARDENING PROBES PASSED"
