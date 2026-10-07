#!/usr/bin/env bash
# Fixed Compose overlay selection for credentialed KIS read paths.
#
# Callers must load the production dotenv file with the non-evaluating parser
# before calling kis_read_compose_configure. This library never sources an env
# file, evaluates a value, trusts an ambient mode, or selects a caller-supplied
# overlay path. It only returns fixed -f arguments derived from source_root.

declare -ga KIS_READ_COMPOSE_FILE_ARGS=()
declare -g KIS_READ_COMPOSE_OWNER_INTRADAY_QUOTES_MODE=
declare -g KIS_READ_COMPOSE_COORDINATION_MODE=
declare -g KIS_READ_COMPOSE_SESSION_WINDOWS_SOURCE=
declare -g KIS_READ_COMPOSE_QUOTE_TRANSPORT=
declare -g KIS_READ_COMPOSE_ERROR=

kis_read_compose_fail() {
  KIS_READ_COMPOSE_ERROR=$1
  return 1
}

kis_read_compose_require_file() {
  local path=$1 label=$2
  [ -f "$path" ] && [ ! -L "$path" ] ||
    kis_read_compose_fail "$label missing or symlinked: $path"
}

# Pure metadata validation shared with the root-side config validator. These
# values select reviewed source wiring; they are never provider credentials or
# evidence that an operator has approved an activation.
kis_read_compose_validate_market_stream() {
  local key expected alias value origin authority port pool
  local quotes_mode=${DOTENV_VALUES[OWNER_INTRADAY_QUOTES_MODE]-off}
  local transport=${DOTENV_VALUES[OWNER_INTRADAY_QUOTE_TRANSPORT]-rest}
  local uuid_pattern='^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
  local origin_pattern='^https://([a-z0-9]([a-z0-9.-]*[a-z0-9])?|\[[0-9a-f:]+\])(:[1-9][0-9]{0,4})?$'

  KIS_READ_COMPOSE_QUOTE_TRANSPORT=
  for key in OWNER_INTRADAY_QUOTE_TRANSPORT OWNER_MARKET_STREAM_ORIGIN \
    KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID KIS_MARKET_STREAM_GRANT_ID \
    KIS_MARKET_STREAM_CONTRACT_SHA256; do
    expected=${DOTENV_VALUES[$key]-}
    if [ "$key" = OWNER_INTRADAY_QUOTE_TRANSPORT ] && ! dotenv_has "$key"; then
      expected=rest
    fi
    if [[ -v "$key" ]] && [ "${!key-}" != "$expected" ]; then
      kis_read_compose_fail "${key,,}_shell_override_mismatch" || return 1
    fi
    alias=${key}_FILE
    if dotenv_has "$alias" || [[ -v "$alias" ]]; then
      kis_read_compose_fail "${alias,,}_forbidden" || return 1
    fi
  done
  case "$transport" in
    rest|market_ws) ;;
    *) kis_read_compose_fail 'owner_intraday_quote_transport_invalid' || return 1 ;;
  esac
  if [ "$quotes_mode" = owner_only ] &&
     { ! dotenv_has OWNER_INTRADAY_QUOTE_TRANSPORT || [ "$transport" != market_ws ]; }; then
    kis_read_compose_fail 'owner_intraday_quotes_requires_explicit_market_ws' || return 1
  fi
  KIS_READ_COMPOSE_QUOTE_TRANSPORT=$transport
  [ "$quotes_mode" = owner_only ] && [ "$transport" = market_ws ] || return 0

  for key in KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID KIS_MARKET_STREAM_GRANT_ID; do
    value=${DOTENV_VALUES[$key]-}
    if [[ ! "$value" =~ $uuid_pattern ]] ||
       [ "$value" = 00000000-0000-0000-0000-000000000000 ]; then
      kis_read_compose_fail "${key,,}_invalid" || return 1
    fi
  done
  [[ ${DOTENV_VALUES[KIS_MARKET_STREAM_CONTRACT_SHA256]-} =~ ^[0-9a-f]{64}$ ]] ||
    { kis_read_compose_fail 'kis_market_stream_contract_sha256_invalid'; return 1; }
  origin=${DOTENV_VALUES[OWNER_MARKET_STREAM_ORIGIN]-}
  if [ "${#origin}" -gt 256 ] || [[ ! "$origin" =~ $origin_pattern ]]; then
    kis_read_compose_fail 'owner_market_stream_origin_invalid' || return 1
  fi
  authority=${origin#https://}
  if [[ "$authority" =~ :([0-9]+)$ ]]; then
    port=${BASH_REMATCH[1]}
    [ "$((10#$port))" -le 65535 ] ||
      { kis_read_compose_fail 'owner_market_stream_origin_invalid'; return 1; }
  fi

  # Twenty dedicated LISTEN connections must leave room for actor reads.
  # Reject an explicit undersized value rather than silently changing it.
  pool=${DOTENV_VALUES[DB_APP_MAX_CONNECTIONS]-32}
  if [[ ! "$pool" =~ ^[1-9][0-9]{0,9}$ ]] ||
     [ "$pool" -lt 24 ] || [ "$pool" -gt 4294967295 ]; then
    kis_read_compose_fail 'market_stream_app_pool_too_small_or_invalid' || return 1
  fi
  if [[ -v DB_APP_MAX_CONNECTIONS ]] && [ "$DB_APP_MAX_CONNECTIONS" != "$pool" ]; then
    kis_read_compose_fail 'db_app_max_connections_shell_override_mismatch' || return 1
  fi
  if dotenv_has DB_APP_MAX_CONNECTIONS_FILE || [[ -v DB_APP_MAX_CONNECTIONS_FILE ]]; then
    kis_read_compose_fail 'db_app_max_connections_file_forbidden' || return 1
  fi
}

kis_read_compose_reject_shell_mode_overrides() {
  local key expected shell_value
  for key in OWNER_INTRADAY_QUOTES_MODE KIS_READ_COORDINATION_MODE \
    OWNER_INTRADAY_SESSION_WINDOWS_SOURCE; do
    case "$key" in
      OWNER_INTRADAY_QUOTES_MODE) expected=$1 ;;
      KIS_READ_COORDINATION_MODE) expected=$2 ;;
      OWNER_INTRADAY_SESSION_WINDOWS_SOURCE) expected=$3 ;;
    esac
    if [[ -v "$key" ]]; then
      shell_value=${!key-}
      [ "$shell_value" = "$expected" ] ||
        kis_read_compose_fail "${key,,}_shell_override_mismatch"
      [ -z "$KIS_READ_COMPOSE_ERROR" ] || return 1
    fi
  done
}

kis_read_compose_configure() {
  local source_root=$1
  local quotes_mode coordination_mode session_windows_source
  local intraday_overlay operational_overlay market_stream_overlay

  KIS_READ_COMPOSE_FILE_ARGS=()
  KIS_READ_COMPOSE_OWNER_INTRADAY_QUOTES_MODE=
  KIS_READ_COMPOSE_COORDINATION_MODE=
  KIS_READ_COMPOSE_SESSION_WINDOWS_SOURCE=
  KIS_READ_COMPOSE_QUOTE_TRANSPORT=
  KIS_READ_COMPOSE_ERROR=

  case "$source_root" in
    /*) ;;
    *) kis_read_compose_fail 'source root must be absolute' || return 1 ;;
  esac

  if dotenv_has OWNER_INTRADAY_QUOTES_MODE; then
    quotes_mode=$(dotenv_get OWNER_INTRADAY_QUOTES_MODE)
  else
    quotes_mode=off
  fi
  if dotenv_has KIS_READ_COORDINATION_MODE; then
    coordination_mode=$(dotenv_get KIS_READ_COORDINATION_MODE)
  else
    coordination_mode=legacy
  fi
  if dotenv_has OWNER_INTRADAY_SESSION_WINDOWS_SOURCE; then
    session_windows_source=$(dotenv_get OWNER_INTRADAY_SESSION_WINDOWS_SOURCE)
  else
    session_windows_source=release_v1
  fi

  case "$quotes_mode" in
    off|owner_only) ;;
    *)
      kis_read_compose_fail 'owner_intraday_quotes_mode_invalid' || return 1
      ;;
  esac
  case "$coordination_mode" in
    legacy|shared_required) ;;
    *)
      kis_read_compose_fail 'kis_read_coordination_mode_invalid' || return 1
      ;;
  esac
  case "$session_windows_source" in
    release_v1|operational_v1) ;;
    *)
      kis_read_compose_fail 'owner_intraday_session_windows_source_invalid' || return 1
      ;;
  esac
  kis_read_compose_validate_market_stream || return 1

  if [ "$quotes_mode" = owner_only ] && [ "$coordination_mode" != shared_required ]; then
    kis_read_compose_fail 'owner_intraday_quotes_requires_shared' || return 1
  fi
  if [ "$session_windows_source" = operational_v1 ] &&
     [ "$coordination_mode" = legacy ]; then
    kis_read_compose_fail 'operational_session_windows_requires_shared' || return 1
  fi
  kis_read_compose_reject_shell_mode_overrides \
    "$quotes_mode" "$coordination_mode" "$session_windows_source" || return 1

  # The legacy feature-off defaults intentionally take this branch without
  # looking for either overlay. Shared coordination is explicit, even when
  # owner intraday production is off, so every reader gets the same fixed
  # coordinator wiring for that configured mode.
  if [ "$coordination_mode" = shared_required ]; then
    intraday_overlay=$source_root/deploy/compose/compose.intraday.yml
    kis_read_compose_require_file "$intraday_overlay" 'intraday overlay' || return 1
    KIS_READ_COMPOSE_FILE_ARGS+=(-f "$intraday_overlay")
  fi

  if [ "$session_windows_source" = operational_v1 ] &&
     [ "$quotes_mode" = owner_only ]; then
    operational_overlay=$source_root/deploy/compose/compose.intraday-operational.yml
    kis_read_compose_require_file "$operational_overlay" 'operational overlay' || return 1
    KIS_READ_COMPOSE_FILE_ARGS+=(-f "$operational_overlay")
  fi

  if [ "$quotes_mode" = owner_only ] &&
     [ "$KIS_READ_COMPOSE_QUOTE_TRANSPORT" = market_ws ]; then
    market_stream_overlay=$source_root/deploy/compose/compose.market-stream.yml
    kis_read_compose_require_file "$market_stream_overlay" 'market stream overlay' || return 1
    KIS_READ_COMPOSE_FILE_ARGS+=(-f "$market_stream_overlay")
  fi

  KIS_READ_COMPOSE_OWNER_INTRADAY_QUOTES_MODE=$quotes_mode
  KIS_READ_COMPOSE_COORDINATION_MODE=$coordination_mode
  KIS_READ_COMPOSE_SESSION_WINDOWS_SOURCE=$session_windows_source
}
