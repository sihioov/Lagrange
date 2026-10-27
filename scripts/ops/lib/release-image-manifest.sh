#!/usr/bin/env bash
# Shared strict V2/V3 manifest primitives for the single-host owner-beta release.
#
# This library is Bash/GNU/Linux only.  It deliberately models Docker's local
# immutable image ID (`sha256:<64 lowercase hex>`) rather than a registry
# RepoDigest: the owner-beta release is single-host and single-platform.

RELEASE_IMAGE_MANIFEST_V2_FORMAT='LAGRANGE_RELEASE_MANIFEST_V2'
RELEASE_IMAGE_MANIFEST_V3_FORMAT='LAGRANGE_RELEASE_MANIFEST_V3'
RELEASE_IMAGE_MANIFEST_FORMAT=$RELEASE_IMAGE_MANIFEST_V3_FORMAT
RELEASE_IMAGE_SERVICES_V2=(
  db-role-bootstrap
  db-migrate
  api-server
  web
  research-worker
  recommendation-runner
  candidate-runner
  owner-beta-runner
  owner-equity-v2-runner
  nt-backtest-worker-1
  nt-backtest-worker-2
  paper-scheduler
)
RELEASE_IMAGE_SERVICES_V3=(
  db-role-bootstrap
  db-migrate
  api-server
  web
  research-worker
  recommendation-runner
  candidate-runner
  owner-beta-runner
  owner-equity-v2-runner
  nt-backtest-worker-1
  nt-backtest-worker-2
  paper-scheduler
  owner-beta-simulation-runner
)
# The current writer/build set is V3. V2 remains a separate fixed historical
# reader set so changing the default cannot widen an old manifest's contract.
RELEASE_IMAGE_SERVICES=("${RELEASE_IMAGE_SERVICES_V3[@]}")

# Populated only by release_image_manifest_load or by the build helper before
# release_image_manifest_write. Callers must never supplement either fixed set
# from Compose or user input.
declare -Ag RELEASE_IMAGE_MANIFEST_REFS=()
declare -Ag RELEASE_IMAGE_MANIFEST_IDS=()
declare -Ag RELEASE_IMAGE_MANIFEST_REVISIONS=()
RELEASE_IMAGE_MANIFEST_LOADED_FORMAT=
RELEASE_IMAGE_MANIFEST_LOADED_COMMIT=
RELEASE_IMAGE_MANIFEST_LOADED_SERVICES=()
RELEASE_IMAGE_MANIFEST_ERROR=

release_image_manifest_fail() {
  RELEASE_IMAGE_MANIFEST_ERROR=$1
  return 1
}

release_image_manifest_reset() {
  RELEASE_IMAGE_MANIFEST_REFS=()
  RELEASE_IMAGE_MANIFEST_IDS=()
  RELEASE_IMAGE_MANIFEST_REVISIONS=()
  RELEASE_IMAGE_MANIFEST_LOADED_FORMAT=
  RELEASE_IMAGE_MANIFEST_LOADED_COMMIT=
  RELEASE_IMAGE_MANIFEST_LOADED_SERVICES=()
  RELEASE_IMAGE_MANIFEST_ERROR=
}

release_image_manifest_is_commit() {
  [[ "$1" =~ ^[0-9a-f]{40}$ ]] &&
    [ "$1" != 0000000000000000000000000000000000000000 ]
}

release_image_manifest_is_image_id() {
  [[ "$1" =~ ^sha256:[0-9a-f]{64}$ ]]
}

release_image_manifest_service_is_allowed() {
  local candidate=$1 service
  for service in "${RELEASE_IMAGE_SERVICES_V3[@]}"; do
    [ "$candidate" = "$service" ] && return 0
  done
  return 1
}

release_image_manifest_service_is_in_list() {
  local candidate=$1 service
  shift
  for service in "$@"; do
    [ "$candidate" = "$service" ] && return 0
  done
  return 1
}

# Runtime aliases are fixed and share one canonical build/manifest entry.
release_image_manifest_canonical_service() {
  local service=$1
  case "$service" in
    owner-beta-daily-producer|owner-beta-daily-admitter)
      printf '%s' owner-beta-runner
      ;;
    *)
      release_image_manifest_service_is_allowed "$service" || {
        release_image_manifest_fail 'runtime service has no canonical image'
        return 1
      }
      printf '%s' "$service"
      ;;
  esac
}

release_image_manifest_ref_for() {
  local service=$1 commit=$2
  release_image_manifest_service_is_allowed "$service" || return 1
  release_image_manifest_is_commit "$commit" || return 1
  printf 'lagrange-station-%s:%s' "$service" "$commit"
}

release_image_manifest_require_absolute_path() {
  local path=$1 label=$2
  [ -n "$path" ] || {
    release_image_manifest_fail "$label must not be empty"
    return 1
  }
  case "$path" in
    /*) ;;
    *)
      release_image_manifest_fail "$label must be absolute"
      return 1
      ;;
  esac
  case "$path" in
    /) ;;
    */)
      release_image_manifest_fail "$label must not have a trailing slash"
      return 1
      ;;
  esac
  case "$path" in
    *$'\n'*|*$'\r'*|*'//'*)
      release_image_manifest_fail "$label is not a canonical absolute path"
      return 1
      ;;
    */../*|*/..|*/./*|*/.)
      release_image_manifest_fail "$label must not contain dot path components"
      return 1
      ;;
  esac
}

release_image_manifest_trusted_directory() {
  local directory=$1 label=$2 component current metadata uid mode mode_value
  local -a components=()
  release_image_manifest_require_absolute_path "$directory" "$label" || return 1
  [ "$directory" != / ] || return 0

  current=
  IFS=/ read -r -a components <<<"${directory#/}"
  for component in "${components[@]}"; do
    [ -n "$component" ] || {
      release_image_manifest_fail "$label has an empty path component"
      return 1
    }
    current="${current}/${component}"
    [ -d "$current" ] && [ ! -L "$current" ] || {
      release_image_manifest_fail "$label has a missing or symlinked directory component"
      return 1
    }
    metadata=$(stat -c '%u:%a' -- "$current") || {
      release_image_manifest_fail "$label directory metadata cannot be inspected"
      return 1
    }
    uid=${metadata%%:*}
    mode=${metadata#*:}
    [ "$uid" = 0 ] || {
      release_image_manifest_fail "$label directory is not root-owned"
      return 1
    }
    mode_value=$((8#$mode))
    (( (mode_value & 0022) == 0 )) || {
      release_image_manifest_fail "$label directory is group/other writable"
      return 1
    }
  done
}

# A trusted external input must be root:root 0600 and every directory in its
# path must be root-owned, non-symlinked, and non-group/other-writable.  The
# deployer validates this input before copying it once into root-owned staging;
# it subsequently validates that installed copy again before activation.
release_image_manifest_trusted_file() {
  local path=$1 label=$2 metadata
  release_image_manifest_require_absolute_path "$path" "$label" || return 1
  [ -f "$path" ] && [ ! -L "$path" ] || {
    release_image_manifest_fail "$label must be a regular non-symlink file"
    return 1
  }
  [ -r "$path" ] || {
    release_image_manifest_fail "$label is not readable"
    return 1
  }
  metadata=$(stat -c '%u:%g:%a' -- "$path") || {
    release_image_manifest_fail "$label metadata cannot be inspected"
    return 1
  }
  [ "$metadata" = 0:0:600 ] || {
    release_image_manifest_fail "$label must be root:root mode 0600"
    return 1
  }
  release_image_manifest_trusted_directory "$(dirname -- "$path")" "$label"
}

release_image_manifest_is_ascii_text() {
  # The grammar is deliberately ASCII-only.  This detects NUL/control/high-bit
  # bytes before Bash's line reader could normalize or discard them.
  LC_ALL=C tr -d '\12\40-\176' <"$1" | cmp -s - /dev/null
}

# Parse exactly one of the two canonical documents:
#
#   LAGRANGE_RELEASE_MANIFEST_V2
#   commit|<40 lowercase hex>
#   image|<service>|<configured exact commit tag>|<local image_id>|<revision>
#
# V3 uses the same grammar with the V3 header and one final simulation-runner
# record. Neither format admits unknown, duplicate, missing, out-of-order, or
# malformed records.
release_image_manifest_load() {
  local path=$1 expected_commit=$2 expected_lines line_number service expected_service
  local expected_ref last_byte
  local -a lines=() fields=()
  local -a expected_services=()
  local -A parsed_refs=() parsed_ids=() parsed_revisions=()

  release_image_manifest_reset
  release_image_manifest_is_commit "$expected_commit" || {
    release_image_manifest_fail 'expected release commit is invalid'
    return 1
  }
  [ -f "$path" ] && [ ! -L "$path" ] || {
    release_image_manifest_fail 'manifest must be a regular non-symlink file'
    return 1
  }
  [ -r "$path" ] || {
    release_image_manifest_fail 'manifest is not readable'
    return 1
  }
  release_image_manifest_is_ascii_text "$path" || {
    release_image_manifest_fail 'manifest contains non-ASCII/control bytes'
    return 1
  }
  [ -s "$path" ] || {
    release_image_manifest_fail 'manifest is empty'
    return 1
  }
  last_byte=$(tail -c 1 -- "$path" | od -An -tx1 | tr -d '[:space:]') || {
    release_image_manifest_fail 'manifest final byte cannot be inspected'
    return 1
  }
  [ "$last_byte" = 0a ] || {
    release_image_manifest_fail 'manifest must end with one newline'
    return 1
  }
  mapfile -t lines <"$path" || {
    release_image_manifest_fail 'manifest cannot be read'
    return 1
  }
  case "${lines[0]}" in
    "$RELEASE_IMAGE_MANIFEST_V2_FORMAT")
      expected_services=("${RELEASE_IMAGE_SERVICES_V2[@]}")
      ;;
    "$RELEASE_IMAGE_MANIFEST_V3_FORMAT")
      expected_services=("${RELEASE_IMAGE_SERVICES_V3[@]}")
      ;;
    *)
      release_image_manifest_fail 'manifest format is unsupported'
      return 1
      ;;
  esac
  expected_lines=$((2 + ${#expected_services[@]}))
  [ "${#lines[@]}" -eq "$expected_lines" ] || {
    release_image_manifest_fail 'manifest record count is not canonical'
    return 1
  }

  case "${lines[1]}" in
    *'|')
      release_image_manifest_fail 'manifest commit record has a trailing separator'
      return 1
      ;;
  esac
  IFS='|' read -r -a fields <<<"${lines[1]}"
  [ "${#fields[@]}" -eq 2 ] && [ "${fields[0]}" = commit ] &&
    release_image_manifest_is_commit "${fields[1]:-}" || {
      release_image_manifest_fail 'manifest commit record is malformed'
      return 1
    }
  [ "${fields[1]}" = "$expected_commit" ] || {
    release_image_manifest_fail 'manifest commit disagrees with expected source commit'
    return 1
  }

  for ((line_number = 0; line_number < ${#expected_services[@]}; line_number++)); do
    fields=()
    case "${lines[$((line_number + 2))]}" in
      *'|')
        release_image_manifest_fail 'manifest image record has a trailing separator'
        return 1
        ;;
    esac
    IFS='|' read -r -a fields <<<"${lines[$((line_number + 2))]}"
    [ "${#fields[@]}" -eq 5 ] && [ "${fields[0]:-}" = image ] || {
      release_image_manifest_fail 'manifest image record has the wrong shape'
      return 1
    }
    service=${fields[1]:-}
    expected_service=${expected_services[$line_number]}
    [ "$service" = "$expected_service" ] || {
      release_image_manifest_fail 'manifest service records are not the canonical whitelist/order'
      return 1
    }
    expected_ref=$(release_image_manifest_ref_for "$service" "$expected_commit") || {
      release_image_manifest_fail 'manifest service has no configured image reference'
      return 1
    }
    [ "${fields[2]:-}" = "$expected_ref" ] || {
      release_image_manifest_fail 'manifest configured image reference disagrees with Compose'
      return 1
    }
    release_image_manifest_is_image_id "${fields[3]:-}" || {
      release_image_manifest_fail 'manifest image_id is not an exact local Docker image ID'
      return 1
    }
    release_image_manifest_is_commit "${fields[4]:-}" &&
      [ "${fields[4]}" = "$expected_commit" ] || {
        release_image_manifest_fail 'manifest image revision disagrees with expected source commit'
        return 1
      }
    parsed_refs["$service"]=${fields[2]}
    parsed_ids["$service"]=${fields[3]}
    parsed_revisions["$service"]=${fields[4]}
  done

  # Publish all parsed data only after every record passes validation.
  RELEASE_IMAGE_MANIFEST_REFS=()
  RELEASE_IMAGE_MANIFEST_IDS=()
  RELEASE_IMAGE_MANIFEST_REVISIONS=()
  for service in "${expected_services[@]}"; do
    RELEASE_IMAGE_MANIFEST_REFS["$service"]=${parsed_refs[$service]}
    RELEASE_IMAGE_MANIFEST_IDS["$service"]=${parsed_ids[$service]}
    RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=${parsed_revisions[$service]}
  done
  RELEASE_IMAGE_MANIFEST_LOADED_FORMAT=${lines[0]}
  RELEASE_IMAGE_MANIFEST_LOADED_COMMIT=$expected_commit
  RELEASE_IMAGE_MANIFEST_LOADED_SERVICES=("${expected_services[@]}")
}

release_image_manifest_map_has_exact_current_services() {
  local -n values=$1
  local service
  [ "${#values[@]}" -eq "${#RELEASE_IMAGE_SERVICES_V3[@]}" ] || return 1
  for service in "${!values[@]}"; do
    release_image_manifest_service_is_allowed "$service" || return 1
  done
  for service in "${RELEASE_IMAGE_SERVICES_V3[@]}"; do
    [ "${values[$service]+present}" = present ] || return 1
  done
}

release_image_manifest_write() {
  local path=$1 commit=$2 service expected_ref ref image_id revision
  release_image_manifest_is_commit "$commit" || {
    release_image_manifest_fail 'manifest write commit is invalid'
    return 1
  }
  release_image_manifest_map_has_exact_current_services RELEASE_IMAGE_MANIFEST_REFS &&
    release_image_manifest_map_has_exact_current_services RELEASE_IMAGE_MANIFEST_IDS &&
    release_image_manifest_map_has_exact_current_services RELEASE_IMAGE_MANIFEST_REVISIONS || {
      release_image_manifest_fail 'manifest write maps must contain exactly the current service set'
      return 1
    }
  for service in "${RELEASE_IMAGE_SERVICES_V3[@]}"; do
    expected_ref=$(release_image_manifest_ref_for "$service" "$commit") || {
      release_image_manifest_fail 'manifest write service is invalid'
      return 1
    }
    ref=${RELEASE_IMAGE_MANIFEST_REFS[$service]:-}
    image_id=${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}
    revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}
    [ "$ref" = "$expected_ref" ] || {
      release_image_manifest_fail 'manifest write configured reference is invalid'
      return 1
    }
    release_image_manifest_is_image_id "$image_id" || {
      release_image_manifest_fail 'manifest write image_id is invalid'
      return 1
    }
    release_image_manifest_is_commit "$revision" && [ "$revision" = "$commit" ] || {
      release_image_manifest_fail 'manifest write revision is invalid'
      return 1
    }
  done
  {
    printf '%s\n' "$RELEASE_IMAGE_MANIFEST_V3_FORMAT"
    printf 'commit|%s\n' "$commit"
    for service in "${RELEASE_IMAGE_SERVICES_V3[@]}"; do
      printf 'image|%s|%s|%s|%s\n' \
        "$service" "${RELEASE_IMAGE_MANIFEST_REFS[$service]}" \
        "${RELEASE_IMAGE_MANIFEST_IDS[$service]}" \
        "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]}"
    done
  } >"$path"
}

release_image_manifest_loaded_state_is_valid() {
  local -a expected_services=()
  local -n refs=RELEASE_IMAGE_MANIFEST_REFS
  local -n ids=RELEASE_IMAGE_MANIFEST_IDS
  local -n revisions=RELEASE_IMAGE_MANIFEST_REVISIONS
  local service expected_ref service_index

  case "$RELEASE_IMAGE_MANIFEST_LOADED_FORMAT" in
    "$RELEASE_IMAGE_MANIFEST_V2_FORMAT")
      expected_services=("${RELEASE_IMAGE_SERVICES_V2[@]}")
      ;;
    "$RELEASE_IMAGE_MANIFEST_V3_FORMAT")
      expected_services=("${RELEASE_IMAGE_SERVICES_V3[@]}")
      ;;
    *) return 1 ;;
  esac
  release_image_manifest_is_commit "$RELEASE_IMAGE_MANIFEST_LOADED_COMMIT" || return 1
  [ "${#RELEASE_IMAGE_MANIFEST_LOADED_SERVICES[@]}" -eq "${#expected_services[@]}" ] || return 1
  for ((service_index = 0; service_index < ${#expected_services[@]}; service_index++)); do
    [ "${RELEASE_IMAGE_MANIFEST_LOADED_SERVICES[$service_index]}" = "${expected_services[$service_index]}" ] || return 1
  done
  [ "${#refs[@]}" -eq "${#expected_services[@]}" ] &&
    [ "${#ids[@]}" -eq "${#expected_services[@]}" ] &&
    [ "${#revisions[@]}" -eq "${#expected_services[@]}" ] || return 1
  for service in "${!refs[@]}"; do
    release_image_manifest_service_is_in_list "$service" "${expected_services[@]}" || return 1
  done
  for service in "${!ids[@]}"; do
    release_image_manifest_service_is_in_list "$service" "${expected_services[@]}" || return 1
  done
  for service in "${!revisions[@]}"; do
    release_image_manifest_service_is_in_list "$service" "${expected_services[@]}" || return 1
  done
  for service in "${expected_services[@]}"; do
    [ "${refs[$service]+present}" = present ] &&
      [ "${ids[$service]+present}" = present ] &&
      [ "${revisions[$service]+present}" = present ] || return 1
    expected_ref=$(release_image_manifest_ref_for "$service" "$RELEASE_IMAGE_MANIFEST_LOADED_COMMIT") || return 1
    [ "${refs[$service]}" = "$expected_ref" ] || return 1
    release_image_manifest_is_image_id "${ids[$service]}" || return 1
    [ "${revisions[$service]}" = "$RELEASE_IMAGE_MANIFEST_LOADED_COMMIT" ] || return 1
  done
}

# Values are only the fixed whitelist and parser-validated SHA-256 image IDs,
# so the generated YAML cannot be influenced by a service/name/value injection.
release_image_manifest_write_compose_override() {
  local path=$1 daily_mode=${2:-disabled} service canonical image_id revision
  local -a runtime_services=()
  release_image_manifest_loaded_state_is_valid || {
    release_image_manifest_fail 'Compose override requires an intact successful manifest load'
    return 1
  }
  runtime_services=("${RELEASE_IMAGE_MANIFEST_LOADED_SERVICES[@]}")
  case "$daily_mode" in
    disabled) ;;
    owner_only)
      runtime_services+=(owner-beta-daily-producer owner-beta-daily-admitter)
      ;;
    *) release_image_manifest_fail 'daily Compose mode is invalid'; return 1 ;;
  esac
  for service in "${RELEASE_IMAGE_MANIFEST_LOADED_SERVICES[@]}"; do
    image_id=${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}
    revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}
    release_image_manifest_is_image_id "$image_id" || {
      release_image_manifest_fail 'Compose override image_id is invalid'
      return 1
    }
    release_image_manifest_is_commit "$revision" || {
      release_image_manifest_fail 'Compose override revision is invalid'
      return 1
    }
  done
  {
    printf 'services:\n'
    for service in "${runtime_services[@]}"; do
      canonical=$(release_image_manifest_canonical_service "$service") || return 1
      printf '  %s:\n' "$service"
      printf '    image: %s\n' "${RELEASE_IMAGE_MANIFEST_IDS[$canonical]}"
      # Docker Compose's documented !reset tag removes the source build map.
      # `up --no-build` and the absence of the opt-in `run --build` add defense
      # in depth; Compose v5 no longer exposes `run --no-build`.
      printf '    build: !reset null\n'
    done
  } >"$path"
}
