#!/usr/bin/env bash
# Closed daily release mode from the already parsed protected dotenv file.
# Internal container paths and the verified hash are never user configuration.
DAILY_RELEASE_MODE=disabled
DAILY_RELEASE_ERROR=

daily_release_fail() {
  DAILY_RELEASE_ERROR=$1
  return 1
}

daily_release_parse_mode() {
  local expected key
  DAILY_RELEASE_ERROR=
  DAILY_RELEASE_MODE=disabled
  expected=${DOTENV_VALUES[OWNER_BETA_DAILY_MODE]-disabled}
  case "$expected" in
    disabled|owner_only) ;;
    *) daily_release_fail 'owner_beta_daily_mode_invalid'; return 1 ;;
  esac
  if [[ -v OWNER_BETA_DAILY_MODE ]] &&
     [ "${OWNER_BETA_DAILY_MODE-}" != "$expected" ]; then
    daily_release_fail 'owner_beta_daily_shell_override_mismatch'; return 1
  fi
  for key in OWNER_BETA_DAILY_MODE_FILE PRICE_BETA_DAILY_MODE \
    PRICE_BETA_DAILY_CONFIG_FILE PRICE_BETA_DAILY_CONFIG_SHA256 \
    LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256; do
    if dotenv_has "$key" || [[ -v "$key" ]]; then
      daily_release_fail 'owner_beta_daily_internal_or_file_input_forbidden'; return 1
    fi
  done
  if [ "$expected" = owner_only ]; then
    [ "${DOTENV_VALUES[OWNER_BETA_ACCESS_MODE]-disabled}" = owner_only ] || {
      daily_release_fail 'owner_beta_daily_requires_owner_only'; return 1;
    }
    [ "${DOTENV_VALUES[OWNER_BETA_PRICE_INPUT_MODE]-disabled}" = sealed_v1 ] || {
      daily_release_fail 'owner_beta_daily_requires_sealed_price'; return 1;
    }
    [ "${DOTENV_VALUES[OWNER_BETA_PAPER_MODE]-disabled}" = disabled ] || {
      daily_release_fail 'owner_beta_daily_requires_paper_disabled'; return 1;
    }
  fi
  DAILY_RELEASE_MODE=$expected
}
