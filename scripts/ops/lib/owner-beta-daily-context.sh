#!/usr/bin/env bash
# Private non-secret context channel for the installed daily release controller.
# Never source dotenv values; never emit the full environment or a credential.
set -euo pipefail
fail() { printf '%s\n' 'DAILY_CONTEXT_INVALID' >&2; exit 1; }
[ "$#" -eq 3 ] || fail
release=$1 commit=$2 operation=$3
[ "$(id -u)" = 0 ] || fail
case "$operation" in prepare|enable|disable|quiescent) ;; *) fail ;; esac
source "$release/scripts/ops/lib/dotenv.sh"
source "$release/scripts/ops/lib/release-image-manifest.sh"
release_image_manifest_is_commit "$commit" || fail
release_image_manifest_trusted_directory "$release" daily-release || fail
envfile=$release/deploy/compose/.env
release_image_manifest_trusted_file "$envfile" daily-env || fail
dotenv_load "$envfile" || fail
[ "${DOTENV_VALUES[LAGRANGE_CODE_COMMIT]-}" = "$commit" ] || fail
# Protected configuration decides values. The Python caller separately rejects
# relevant ambient overrides before activation; shutdown never depends on Auth0.
if [ "$operation" = enable ] || [ "$operation" = prepare ]; then
    source "$release/scripts/ops/lib/owner-beta-daily-release.sh"
    daily_release_parse_mode || fail
fi
for key in LAGRANGE_CODE_COMMIT LAGRANGE_DATA_DIR LAGRANGE_ARTIFACTS_DIR \
    LAGRANGE_RUNTIME_SECRET_DIR POSTGRES_DB MAX_JOBS_PER_OWNER \
    PRICE_BETA_LEASE_SECS PRICE_BETA_BACKOFF_SECS OWNER_BETA_DAILY_MODE \
    OWNER_BETA_ACCESS_MODE OWNER_BETA_PRICE_INPUT_MODE OWNER_BETA_PAPER_MODE; do
    printf '%s\0%s\0' "$key" "${DOTENV_VALUES[$key]-}"
done
if [ "$operation" = enable ]; then
    source "$release/scripts/ops/lib/kis-read-compose.sh"
    kis_read_compose_configure "$release" || fail
    for item in "${KIS_READ_COMPOSE_FILE_ARGS[@]}"; do
        printf '%s\0%s\0' COMPOSE_ARG "$item"
    done
fi
