#!/usr/bin/env bash
# Shared strict V2 manifest primitives for the single-host owner-beta release.
#
# This library is Bash/GNU/Linux only.  It deliberately models Docker's local
# immutable image ID (`sha256:<64 lowercase hex>`) rather than a registry
# RepoDigest: the owner-beta release is single-host and single-platform.

RELEASE_IMAGE_MANIFEST_FORMAT='LAGRANGE_RELEASE_MANIFEST_V2'
RELEASE_IMAGE_SERVICES=(
  db-role-bootstrap
  db-migrate
  api-server
  web
  research-worker
  recommendation-runner
  candidate-runner
  owner-beta-runner
  nt-backtest-worker-1
  nt-backtest-worker-2
  paper-scheduler
)

# Populated only by release_image_manifest_load or by the build helper before
# release_image_manifest_write.  The service list above is the complete,
# canonical order; callers must never supplement it from Compose or user input.
declare -Ag RELEASE_IMAGE_MANIFEST_REFS=()
declare -Ag RELEASE_IMAGE_MANIFEST_IDS=()
declare -Ag RELEASE_IMAGE_MANIFEST_REVISIONS=()
RELEASE_IMAGE_MANIFEST_ERROR=

release_image_manifest_fail() {
  RELEASE_IMAGE_MANIFEST_ERROR=$1
  return 1
}

release_image_manifest_reset() {
  RELEASE_IMAGE_MANIFEST_REFS=()
  RELEASE_IMAGE_MANIFEST_IDS=()
  RELEASE_IMAGE_MANIFEST_REVISIONS=()
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
  for service in "${RELEASE_IMAGE_SERVICES[@]}"; do
    [ "$candidate" = "$service" ] && return 0
  done
  return 1
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

# Parse exactly the canonical V2 document:
#
#   LAGRANGE_RELEASE_MANIFEST_V2
#   commit|<40 lowercase hex>
#   image|<service>|<configured exact commit tag>|<local image_id>|<revision>
#
# It admits no unknown, duplicate, missing, out-of-order, or malformed records.
release_image_manifest_load() {
  local path=$1 expected_commit=$2 expected_lines line_number service expected_service
  local expected_ref last_byte
  local -a lines=() fields=()

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
  expected_lines=$((2 + ${#RELEASE_IMAGE_SERVICES[@]}))
  [ "${#lines[@]}" -eq "$expected_lines" ] || {
    release_image_manifest_fail 'manifest record count is not canonical'
    return 1
  }
  [ "${lines[0]}" = "$RELEASE_IMAGE_MANIFEST_FORMAT" ] || {
    release_image_manifest_fail 'manifest format is unsupported'
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

  for ((line_number = 0; line_number < ${#RELEASE_IMAGE_SERVICES[@]}; line_number++)); do
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
    expected_service=${RELEASE_IMAGE_SERVICES[$line_number]}
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
    RELEASE_IMAGE_MANIFEST_REFS["$service"]=${fields[2]}
    RELEASE_IMAGE_MANIFEST_IDS["$service"]=${fields[3]}
    RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=${fields[4]}
  done
}

release_image_manifest_write() {
  local path=$1 commit=$2 service expected_ref ref image_id revision
  release_image_manifest_is_commit "$commit" || {
    release_image_manifest_fail 'manifest write commit is invalid'
    return 1
  }
  for service in "${RELEASE_IMAGE_SERVICES[@]}"; do
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
    printf '%s\n' "$RELEASE_IMAGE_MANIFEST_FORMAT"
    printf 'commit|%s\n' "$commit"
    for service in "${RELEASE_IMAGE_SERVICES[@]}"; do
      printf 'image|%s|%s|%s|%s\n' \
        "$service" "${RELEASE_IMAGE_MANIFEST_REFS[$service]}" \
        "${RELEASE_IMAGE_MANIFEST_IDS[$service]}" \
        "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]}"
    done
  } >"$path"
}

# Values are only the fixed whitelist and parser-validated SHA-256 image IDs,
# so the generated YAML cannot be influenced by a service/name/value injection.
release_image_manifest_write_compose_override() {
  local path=$1 service image_id revision
  for service in "${RELEASE_IMAGE_SERVICES[@]}"; do
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
    for service in "${RELEASE_IMAGE_SERVICES[@]}"; do
      printf '  %s:\n' "$service"
      printf '    image: %s\n' "${RELEASE_IMAGE_MANIFEST_IDS[$service]}"
      # Docker Compose's documented !reset tag removes the source build map.
      # `up --no-build` and the absence of the opt-in `run --build` add defense
      # in depth; Compose v5 no longer exposes `run --no-build`.
      printf '    build: !reset null\n'
    done
  } >"$path"
}
