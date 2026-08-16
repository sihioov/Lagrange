#!/usr/bin/env bash
# Static migration safety audit for the integration lane. It fails closed when
# a migration loses the guards exercised by the runtime workflow; it never
# modifies migration SQL.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
down="$root/migrations/0039_auth_audit_outbox.down.sql"
identity="$root/migrations/0040_identity_provisioning.up.sql"
audit_worker="$root/apps/api-server/auth/src/postgres.rs"

die() {
  echo "PG_VALIDATION_MIGRATION_AUDIT: FAIL: $*" >&2
  exit 1
}

[ -f "$down" ] || die '0039 down migration is missing'
[ -f "$identity" ] || die '0040 up migration is missing'
[ -f "$audit_worker" ] || die 'auth audit worker source is missing'

# A rollback must not silently destroy an undelivered audit obligation.
grep -Fq 'auth_audit_outbox' "$down" || die '0039 down does not mention auth_audit_outbox'
grep -Eq 'delivered_at[[:space:]]+IS[[:space:]]+NULL|undelivered|pending' "$down" \
  || die '0039 down lacks an undelivered-row guard'
grep -Fq 'RAISE EXCEPTION' "$down" \
  || die '0039 down lacks a fail-closed rollback exception'

# The serving actor must be checked before a SECURITY DEFINER identity function
# overwrites its transaction-local GUC. The dynamic identity-boundary.sql test
# is the DB proof; these bounded body checks keep the static lane fail-closed.
create_body=$(sed -n '/CREATE FUNCTION public.create_invitation/,/ALTER FUNCTION public.create_invitation/p' "$identity")
claim_body=$(sed -n '/CREATE FUNCTION public.claim_invitation/,/ALTER FUNCTION public.claim_invitation/p' "$identity")
for function_name in create_invitation claim_invitation; do
  body=$create_body
  [ "$function_name" = claim_invitation ] && body=$claim_body
  if ! grep -Fq "current_setting('app.actor_user_id'" <<<"$body" \
     && ! grep -Fq 'consume_identity_actor_capability' <<<"$body"; then
    die "0040 ${function_name} has no actor binding check"
  fi
done

# A successful empty poll must not erase a prior delivery failure. The worker
# source must preserve consecutive-failure state across an empty successful
# poll; runtime readiness tests supplement this static assertion.
grep -Fq 'delivered == 0' "$audit_worker" \
  || die 'auth audit worker has no empty-poll branch to protect readiness state'
grep -Fq 'fn next_consecutive_failures' "$audit_worker" \
  || die 'auth audit worker has no explicit consecutive-failure transition'
grep -Fq $'else {\n        current\n    }' "$audit_worker" \
  || die 'auth audit worker does not retain failures on an empty successful poll'
if grep -Fq 'if failed == 0 {' "$audit_worker" \
   && grep -Fq 'consecutive_failures = 0;' "$audit_worker"; then
  die 'auth audit worker resets consecutive failures on an empty successful poll'
fi

echo 'PG_VALIDATION_MIGRATION_AUDIT: PASS'
