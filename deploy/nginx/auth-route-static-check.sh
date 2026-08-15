#!/usr/bin/env bash
# Static contract check for the public OIDC/session edge route.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
conf="$root/deploy/nginx/nginx.conf"
compose="$root/deploy/compose/compose.yml"
env_example="$root/deploy/compose/.env.example"

test -f "$conf"
test -f "$compose"
test -f "$env_example"

auth_line="$(grep -nF 'location ^~ /auth/ {' "$conf" | cut -d: -f1)"
login_line="$(grep -nF 'location = /auth/login {' "$conf" | cut -d: -f1)"
fallback_line="$(grep -nF 'location / {' "$conf" | tail -n1 | cut -d: -f1)"
test -n "$auth_line"
test -n "$login_line"
test -n "$fallback_line"
test "$auth_line" -lt "$fallback_line"
test "$login_line" -lt "$auth_line"

login_block="$(awk -v start="$login_line" -v end="$auth_line" \
  'NR >= start && NR < end { print }' "$conf")"
for required in \
  'limit_req zone=auth_login burst=3 nodelay;' \
  'limit_req_status 429;' \
  'proxy_pass http://api-server:8080;' \
  'proxy_set_header X-Real-IP $remote_addr;' \
  'proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;' \
  'proxy_pass_header Location;' \
  'proxy_pass_header Set-Cookie;' \
  'proxy_pass_header X-CSRF-Token;' \
  'add_header Cache-Control "no-store" always;' \
  'add_header Pragma "no-cache" always;'; do
  grep -Fq "$required" <<<"$login_block" \
    || { echo "missing login rate-limit contract: $required" >&2; exit 1; }
done
grep -Fq 'limit_req_zone $binary_remote_addr zone=auth_login:10m rate=5r/m;' "$conf"
if grep -Fq 'limit_req' <<<"$login_block" && \
   [ "$(grep -Fc 'limit_req zone=auth_login' "$conf")" -ne 1 ]; then
  echo 'login rate-limit must be the only auth request limiter' >&2
  exit 1
fi

auth_block="$(awk -v start="$auth_line" -v end="$fallback_line" \
  'NR >= start && NR < end { print }' "$conf")"
if grep -Fq 'limit_req' <<<"$auth_block"; then
  echo 'callback/logout/session auth proxy must not be rate-limited' >&2
  exit 1
fi
for required in \
  'proxy_pass http://api-server:8080;' \
  'proxy_set_header Host $host;' \
  'proxy_set_header X-Real-IP $remote_addr;' \
  'proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;' \
  'proxy_set_header X-Forwarded-Host $host;' \
  'proxy_set_header X-Forwarded-Proto $scheme;' \
  'proxy_set_header Connection "";' \
  'proxy_pass_header Location;' \
  'proxy_pass_header Set-Cookie;' \
  'proxy_pass_header X-CSRF-Token;' \
  'add_header Cache-Control "no-store" always;' \
  'add_header Pragma "no-cache" always;' \
  'add_header Strict-Transport-Security "max-age=31536000" always;' \
  'add_header X-Content-Type-Options nosniff always;' \
  'add_header X-Frame-Options DENY always;'; do
  grep -Fq "$required" <<<"$auth_block"
done

# The API's auth responses carry these headers; the edge must not suppress
# them while proxying login redirects, callback cookies, or CSRF responses.
grep -Fq 'proxy_http_version 1.1;' <<<"$auth_block"
grep -Fq 'proxy_read_timeout 60s;' <<<"$auth_block"

# The public callback and the confidential client secret belong to the API
# process. Keep this deployment contract next to the edge route check so a
# future proxy edit cannot silently strand Auth0 callbacks or expose a secret
# through plaintext environment configuration.
grep -Fq \
  'AUTH0_REDIRECT_URI: ${AUTH0_REDIRECT_URI:-https://app.lagrange.local/auth/callback}' \
  "$compose"
grep -Fq 'AUTH0_CLIENT_SECRET_FILE: /run/secrets/auth0_client_secret' "$compose"
grep -Fq 'source: api_auth0_client_secret' "$compose"
grep -Fq 'target: auth0_client_secret' "$compose"
grep -Fxq 'AUTH0_REDIRECT_URI=https://app.lagrange.local/auth/callback' "$env_example"

echo 'AUTH0_DEPLOYMENT_STATIC_CHECK: PASS'

echo 'NGINX_AUTH_ROUTE_STATIC_CHECK: PASS'
