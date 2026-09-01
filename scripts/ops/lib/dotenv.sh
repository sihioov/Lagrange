#!/usr/bin/env bash
# Small, non-evaluating dotenv parser shared by the production operator gates.
#
# Compose gives exported shell variables precedence over --env-file values.
# These helpers make that precedence explicit and fail closed when a shell
# override would change the effective production configuration.  Values are
# never sourced or evaluated as shell code.

declare -gA DOTENV_VALUES=()
declare -ga DOTENV_ERRORS=()
declare -ga DOTENV_SHELL_ERRORS=()

dotenv_load() {
  local file=$1 line key value
  DOTENV_VALUES=()
  DOTENV_ERRORS=()
  [ -f "$file" ] || {
    DOTENV_ERRORS+=("missing dotenv file: $file")
    return 1
  }

  while IFS= read -r line || [ -n "$line" ]; do
    line=${line%$'\r'}
    [ -z "$line" ] && continue
    case "$line" in
      \#*) continue ;;
    esac
    [[ "$line" == *=* ]] || {
      DOTENV_ERRORS+=("invalid dotenv line (missing '=')")
      continue
    }
    key=${line%%=*}
    value=${line#*=}
    [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || {
      DOTENV_ERRORS+=("invalid dotenv key")
      continue
    }
    # The production env contract is strict literal key=value. Compose's
    # env-file reader would otherwise reinterpret these values: `$VAR` and
    # `${VAR:-fallback}` interpolate from the shell, quotes enable escape and
    # quote processing, and an unquoted ` #` starts an inline comment. Reject
    # all such syntax before any value can reach Compose. Current production
    # literals (including https:// URLs) contain none of these characters.
    if [[ "$value" == *'$'* || "$value" == *\\* || "$value" == *\"* ||
          "$value" == *"'"* || "$value" =~ [[:space:]]# ]]; then
      DOTENV_ERRORS+=("dotenv value for $key uses Compose interpolation, quote, escape, or inline-comment syntax")
      continue
    fi
    [ -z "${DOTENV_VALUES[$key]+set}" ] || {
      DOTENV_ERRORS+=("duplicate dotenv key: $key")
      continue
    }
    DOTENV_VALUES[$key]=$value
  done <"$file"

  [ "${#DOTENV_ERRORS[@]}" -eq 0 ]
}

dotenv_get() {
  printf '%s' "${DOTENV_VALUES[$1]-}"
}

dotenv_has() {
  [ -n "${DOTENV_VALUES[$1]+set}" ]
}

# Compare every key represented by the env file. This deliberately covers
# paths, profiles, fetch mode, security settings, and all Compose interpolation
# values rather than maintaining a second, drifting allowlist. The additional
# names below catch a dangerous shell-only value even when an operator forgot
# to put that key in the env file (notably COMPOSE_PROFILES=live). The one
# exception is LAGRANGE_CODE_COMMIT: CI may provide it explicitly when the env
# file leaves it empty, but a non-empty env-file commit remains authoritative.
DOTENV_COMPOSE_SECURITY_KEYS=(
  LAGRANGE_DATA_DIR LAGRANGE_ARTIFACTS_DIR LAGRANGE_RUNTIME_SECRET_DIR
  LAGRANGE_SECRET_SOURCE_DIR LAGRANGE_PGDATA_VOLUME POSTGRES_USER POSTGRES_DB
  API_APP_ENV API_INTERNAL_URL RESEARCH_APP_ENV RESEARCH_FETCH_MODE
  RESEARCH_RUN_AT_KST RESEARCH_MAX_PUBLICATION_AGE_SECS
  RESEARCH_ATTEMPT_TIMEOUT_SECS RESEARCH_CANDIDATE_ENABLED
  RESEARCH_ENTITLEMENT_REFERENCE RECOMMENDATION_APP_ENV CANDIDATE_APP_ENV
  PAPER_APP_ENV PAPER_HEALTH_MAX_AGE_SECS PAPER_OPERATION_TIMEOUT_MS
  PAPER_CYCLE_TIMEOUT_MS PAPER_SHUTDOWN_GRACE_MS BACKTEST_MIN_FREE_BYTES
  BACKTEST_MAX_QUEUED_BACKTESTS BACKTEST_RECONCILE_GRACE_SECS
  BACKTEST_RECONCILE_INTERVAL_SECS AUTH0_DOMAIN AUTH0_CLIENT_ID
  AUTH0_REDIRECT_URI RUST_LOG RECOMMENDATION_DATASET_VERSION_ID
  RECOMMENDATION_DATASET_ID RECOMMENDATION_DATASET_VERSION
  RECOMMENDATION_CURATED_VERSION RECOMMENDATION_DATASET_MANIFEST_SHA256
  OWNER_BETA_ACCESS_MODE OWNER_BETA_ACCESS_MODE_FILE
  OWNER_BETA_EQUITY_SIGNALS_MODE OWNER_BETA_EQUITY_SIGNALS_MODE_FILE
  OWNER_BETA_PAPER_MODE OWNER_BETA_PAPER_MODE_FILE
  OWNER_EQUITY_V2_RUNTIME_MODE
  COMPOSE_PROFILES LIVE_NODE_MODE LIVE_NODE_DRY_RUN CANDIDATE_HEALTH_MAX_AGE_SECS
  CANDIDATE_POLL_MS CANDIDATE_SCHEDULE_POLL_SECS CANDIDATE_SWEEP_MS
  CANDIDATE_HEARTBEAT_MS CANDIDATE_LEASE_MS CANDIDATE_BACKOFF_MS
)

dotenv_validate_shell_overrides() {
  local key shell_value file_value
  DOTENV_SHELL_ERRORS=()
  local -A checked=()
  for key in "${!DOTENV_VALUES[@]}" "${DOTENV_COMPOSE_SECURITY_KEYS[@]}"; do
    [ -z "${checked[$key]+set}" ] || continue
    checked[$key]=1
    if [[ -v "$key" ]]; then
      shell_value=${!key-}
      file_value=${DOTENV_VALUES[$key]-}
      [ "$shell_value" = "$file_value" ] ||
        DOTENV_SHELL_ERRORS+=("shell override for $key does not exactly match env-file value")
    fi
  done

  if [[ -v LAGRANGE_CODE_COMMIT ]]; then
    shell_value=${LAGRANGE_CODE_COMMIT-}
    file_value=${DOTENV_VALUES[LAGRANGE_CODE_COMMIT]-}
    if [ -n "$file_value" ] && [ "$shell_value" != "$file_value" ]; then
      DOTENV_SHELL_ERRORS+=("shell override for LAGRANGE_CODE_COMMIT does not exactly match env-file value")
    fi
  fi
  [ "${#DOTENV_SHELL_ERRORS[@]}" -eq 0 ]
}

# Return the effective value after the explicitly documented commit exception.
# Every other Compose/security value is the env-file value once the override
# check above has passed.
dotenv_effective_get() {
  local key=$1
  if [ "$key" = LAGRANGE_CODE_COMMIT ] && [[ -v LAGRANGE_CODE_COMMIT ]] &&
     [ -z "${DOTENV_VALUES[LAGRANGE_CODE_COMMIT]-}" ]; then
    printf '%s' "${LAGRANGE_CODE_COMMIT-}"
  else
    dotenv_get "$key"
  fi
}
