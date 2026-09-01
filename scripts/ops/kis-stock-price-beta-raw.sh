#!/usr/bin/env bash
# Root-only operator wrapper for the fixed-30 KIS daily-bars Raw one-shot.
#
# Plan is local and deliberately avoids production env, secret, Docker, data
# root, and network access. Preflight/execute validate the immutable source and
# installed env before Compose; execute never manages a daemon lifecycle.
set -euo pipefail

script_dir=$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
root=$(cd -P "$script_dir/../.." && pwd -P)
source "$script_dir/lib/dotenv.sh"

compose_file=$root/deploy/compose/compose.yml
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
start_date=2025-08-04
end_date=2026-08-28
entitlement_file=$root/configs/data-rights/kis.entitlement.json
entitlement_id=ent_kis_personal_owner_20260821
entitlement_provider=kis
entitlement_reference=repo://docs/decisions/0005-kis-personal-use-entitlement.md
entitlement_file_sha256=56bc018f748e2a1cfa78c4b94c18adccb2e0afd6a2d66fea4ecd3654db56b36e
start_seen=0
end_seen=0
mode=plan
mode_seen=0
env_file_seen=0
compose_service=research-stock-price-beta-raw
compose_profile=stock-price-beta-raw
confirmation=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS
prepared_image=${KIS_STOCK_PRICE_BETA_IMAGE_PREPARED:-0}
prepared_release_root=${KIS_STOCK_PRICE_BETA_PREPARED_RELEASE_ROOT:-}

usage() {
  cat <<'EOF'
Usage: scripts/ops/kis-stock-price-beta-raw.sh
       [--start YYYY-MM-DD] [--end YYYY-MM-DD]
       [--env-file ABSOLUTE_PATH]
       [--plan|--preflight|--execute]

The default --plan prints the fixed KIS daily-bars request shape only. It does
not read the production env or secrets, invoke Docker, write data, or use a
network. --preflight is root-only and validates the installed immutable env,
exact source HEAD, fixed universe identity, entitlement binding, and Compose
config without building or starting a container. --execute additionally
requires KIS_STOCK_PRICE_BETA_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS,
builds and provenance-checks the dedicated image, then runs exactly one
profile-gated no-deps Raw one-shot. It never stops or starts a daemon.
EOF
}

die() {
  printf 'kis-stock-price-beta-raw: %s\n' "$*" >&2
  exit 1
}

blocked() {
  printf 'BLOCKED_EXTERNAL: %s\n' "$*" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --start)
      [ "$#" -ge 2 ] || die '--start needs YYYY-MM-DD'
      [ "$start_seen" -eq 0 ] || die '--start must not be repeated'
      start_date=$2
      start_seen=1
      shift 2
      ;;
    --end)
      [ "$#" -ge 2 ] || die '--end needs YYYY-MM-DD'
      [ "$end_seen" -eq 0 ] || die '--end must not be repeated'
      end_date=$2
      end_seen=1
      shift 2
      ;;
    --env-file)
      [ "$#" -ge 2 ] || die '--env-file needs an absolute path'
      [ "$env_file_seen" -eq 0 ] || die '--env-file must not be repeated'
      env_file=$2
      env_file_seen=1
      shift 2
      ;;
    --plan|--preflight|--execute)
      [ "$mode_seen" -eq 0 ] || die 'choose exactly one mode'
      mode=${1#--}
      mode_seen=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

[[ "$start_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --start date'
[[ "$end_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die 'invalid --end date'
python3 - "$start_date" "$end_date" <<'PY'
import datetime as dt
import sys

try:
    start = dt.date.fromisoformat(sys.argv[1])
    end = dt.date.fromisoformat(sys.argv[2])
except ValueError as exc:
    raise SystemExit(f"invalid calendar date: {exc}")
if end < start:
    raise SystemExit("--end precedes --start")
PY
[ "$start_date" = 2025-08-04 ] || die 'fixed capture range requires --start 2025-08-04'
[ "$end_date" = 2026-08-28 ] || die 'fixed capture range requires --end 2026-08-28'

print_plan() {
  printf 'KIS_STOCK_PRICE_BETA_RAW_PLAN mode=plan\n'
  printf '  range=%s..%s universe=kr-stock-price-beta-v1 symbols=30\n' "$start_date" "$end_date"
  printf '  interval=D windows=window-01:2026-04-21..2026-08-28,window-02:2025-12-12..2026-04-20,window-03:2025-08-04..2025-12-11 planned_gets_before_retries=90\n'
  printf '  request=GET /uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice tr_id=FHKST03010100 FID_ORG_ADJ_PRC=1 continuation=blank single_page=yes\n'
  printf '  provider=KIS read-only market-data network=range-raw-egress\n'
  printf '  entitlement=%s provider=%s reference=%s file_sha256=%s coverage=krx_eod_bars/usr_owner\n' "$entitlement_id" "$entitlement_provider" "$entitlement_reference" "$entitlement_file_sha256"
  printf '  service=%s profile=%s raw_mount=/data/raw no_curated_db_artifact_mounts=yes\n' "$compose_service" "$compose_profile"
  printf 'PLAN_ONLY: no Docker, secret, production-env, data-root write, or network action made\n'
}

# Plan must be usable without access to protected production files. Every
# validation and external command below is intentionally after this return.
if [ "$mode" = plan ]; then
  print_plan
  exit 0
fi

[ "$(id -u)" -eq 0 ] ||
  blocked "--$mode must run as root to inspect installed production paths"

safe_absolute_file() {
  local path=$1 label=$2 probe=$1
  case "$path" in
    /*) ;;
    *) blocked "$label must be an absolute path" ;;
  esac
  case "$path" in
    *$'\n'*|*$'\r'*|*/../*|*/..|../*)
      blocked "$label has an unsafe path shape"
      ;;
  esac
  while [ "$probe" != / ]; do
    [ ! -L "$probe" ] || blocked "$label must not traverse a symlink"
    probe=${probe%/*}
    [ -n "$probe" ] || probe=/
  done
  if [ -e "$path" ]; then
    [ -f "$path" ] && [ ! -L "$path" ] || blocked "$label must be a regular file"
  fi
}

safe_absolute_file "$env_file" env-file
[ -f "$env_file" ] && [ ! -L "$env_file" ] ||
  blocked 'installed immutable env file is missing'
env_metadata=$(stat -c '%u:%g:%a' -- "$env_file") ||
  blocked 'installed immutable env file metadata is unreadable'
[ "$env_metadata" = 0:0:600 ] ||
  blocked 'installed immutable env file must be root:root mode 0600'
[ -f "$compose_file" ] && [ ! -L "$compose_file" ] ||
  die 'Compose file is missing or unsafe'

if ! dotenv_load "$env_file"; then
  echo 'INVALID_CONFIG: production env file is malformed' >&2
  printf '  - %s\n' "${DOTENV_ERRORS[@]}" >&2
  exit 1
fi
if ! dotenv_validate_shell_overrides; then
  echo 'INVALID_CONFIG: shell overrides do not match production env file' >&2
  printf '  - %s\n' "${DOTENV_SHELL_ERRORS[@]}" >&2
  exit 1
fi

commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] ||
  blocked 'LAGRANGE_CODE_COMMIT must be exactly 40 lowercase hexadecimal characters'
[ "$commit" != 0000000000000000000000000000000000000000 ] ||
  die 'LAGRANGE_CODE_COMMIT must not be all zeroes'
case "$prepared_image" in
  0)
    head=$(git -c "safe.directory=$root" -C "$root" rev-parse --verify 'HEAD^{commit}' 2>/dev/null) ||
      die 'repository HEAD is unavailable'
    [ "$head" = "$commit" ] ||
      blocked 'LAGRANGE_CODE_COMMIT does not match repository HEAD'
    worktree_status=$(git -c "safe.directory=$root" -C "$root" \
      status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
      die 'cannot inspect build root worktree status'
    [ -z "$worktree_status" ] ||
      blocked 'build root worktree must be clean'
    ;;
  1)
    [ "$prepared_release_root" = "$root" ] ||
      blocked 'prepared-image run must bind to its installed release root'
    case "$root" in
      */releases/"$commit") ;;
      *) blocked 'prepared-image run requires a commit-named installed release root' ;;
    esac
    release_owner=$(stat -c '%u' -- "$root") || blocked 'installed release root ownership is unreadable'
    [ "$release_owner" = 0 ] || blocked 'installed release root must be root-owned'
    ;;
  *) blocked 'KIS_STOCK_PRICE_BETA_IMAGE_PREPARED must be exactly 0 or 1' ;;
esac

entitlement_reference=$(dotenv_effective_get RESEARCH_ENTITLEMENT_REFERENCE)
[ -n "$entitlement_reference" ] ||
  blocked 'RESEARCH_ENTITLEMENT_REFERENCE must be present in the installed env'
entitlement_hash=$(dotenv_effective_get RESEARCH_ENTITLEMENT_SHA256)
[[ "$entitlement_hash" =~ ^(sha256:)?[0-9a-f]{64}$ ]] ||
  blocked 'RESEARCH_ENTITLEMENT_SHA256 must be 64 lowercase hex, optionally prefixed sha256:'

command -v sha256sum >/dev/null 2>&1 || blocked 'sha256sum is required for entitlement identity validation'
safe_absolute_file "$entitlement_file" entitlement-file
[ -f "$entitlement_file" ] && [ ! -L "$entitlement_file" ] ||
  blocked 'checked-in entitlement file is missing or unsafe'
checked_entitlement_sha=$(sha256sum -- "$entitlement_file" | awk '{print $1}') ||
  blocked 'checked-in entitlement hash cannot be computed'
[ "$checked_entitlement_sha" = "$entitlement_file_sha256" ] ||
  blocked 'checked-in entitlement bytes do not match the reviewed contract hash'
python3 - "$entitlement_file" "$entitlement_reference" "$entitlement_hash" <<'PY'
import hashlib
import json
import sys

path, supplied_reference, supplied_hash = sys.argv[1:]
try:
    with open(path, "rb") as stream:
        raw = stream.read()
    document = json.loads(raw.decode("utf-8"))
except (OSError, UnicodeDecodeError, ValueError):
    raise SystemExit("checked-in entitlement is not valid JSON")
expected_hash = "56bc018f748e2a1cfa78c4b94c18adccb2e0afd6a2d66fea4ecd3654db56b36e"
expected_reference = "repo://docs/decisions/0005-kis-personal-use-entitlement.md"
provided_hash = supplied_hash[7:] if supplied_hash.startswith("sha256:") else supplied_hash
contract = document.get("contract_document", {})
if (
    hashlib.sha256(raw).hexdigest() != expected_hash
    or provided_hash != expected_hash
    or supplied_reference != expected_reference
    or document.get("schema_version") != 1
    or document.get("provider") != "kis"
    or document.get("entitlement_id") != "ent_kis_personal_owner_20260821"
    or document.get("lifecycle") != "ACTIVE"
    or "krx_eod_bars" not in document.get("covered_datasets", [])
    or "usr_owner" not in document.get("covered_users", [])
    or contract.get("document_reference") != expected_reference
):
    raise SystemExit("entitlement pin or coverage does not match the reviewed contract")
PY

command -v python3 >/dev/null 2>&1 || blocked 'python3 is required for calendar validation'
universe_file="$root/configs/universes/kr-stock-price-beta-v1.json"
safe_absolute_file "$universe_file" universe
[ -f "$universe_file" ] && [ ! -L "$universe_file" ] ||
  blocked 'fixed-stock universe file is missing or unsafe'
universe_sha=$(sha256sum -- "$universe_file" | awk '{print $1}') ||
  blocked 'fixed-stock universe hash cannot be computed'
[ "$universe_sha" = 2a0d55143df0274fcfa357f2824ed752e2969469f93254ed7dfa64766a00dde1 ] ||
  blocked 'fixed-stock universe bytes do not match the reviewed contract hash'
python3 - "$universe_file" <<'PY'
import json
import sys

expected = [
    "005930", "000660", "373220", "207940", "005380", "000270", "105560", "055550",
    "068270", "035420", "035720", "005490", "051910", "006400", "012330", "028260",
    "012450", "329180", "034020", "015760", "017670", "030200", "066570", "009150",
    "096770", "036570", "090430", "011200", "003490", "000810",
]
try:
    with open(sys.argv[1], encoding="utf-8") as stream:
        document = json.load(stream)
except (OSError, ValueError):
    raise SystemExit("fixed-stock universe is not valid JSON")
ids = [item.get("id") for item in document.get("instruments", [])]
if (
    document.get("schema_id") != "kr-stock-price-beta-universe"
    or document.get("schema_version") != 1
    or document.get("universe_id") != "kr-stock-price-beta-v1"
    or document.get("audience") != "OWNER_ONLY"
    or document.get("capability") != "PRICE_VOLUME_RESEARCH_ONLY"
    or document.get("vendor_snapshot") is not True
    or document.get("strict_pit") is not False
    or document.get("market") != "KR"
    or document.get("venue") != "KRX"
    or document.get("asset_class") != "EQUITY"
    or document.get("instrument_count") != len(expected)
    or ids != [f"{symbol}.KRX" for symbol in expected]
):
    raise SystemExit("fixed-stock universe does not match the reviewed 30-ID contract")
PY
command -v docker >/dev/null 2>&1 || blocked 'docker is not installed'
docker compose version >/dev/null 2>&1 || blocked 'Docker Compose v2 is unavailable'
compose_range_raw_batch_id=$(dotenv_effective_get RANGE_RAW_BATCH_ID)
[ -n "$compose_range_raw_batch_id" ] || compose_range_raw_batch_id=compose-config-disabled

compose() {
  LAGRANGE_CODE_COMMIT="$commit" \
  RANGE_RAW_BATCH_ID="$compose_range_raw_batch_id" \
    docker compose --profile "$compose_profile" --env-file "$env_file" \
    --file "$compose_file" "$@"
}

compose config --quiet || die 'Compose interpolation/config validation failed'

if [ "$mode" = preflight ]; then
  echo 'KIS_STOCK_PRICE_BETA_RAW_PREFLIGHT: PASS (no build, KIS call, or container lifecycle)'
  exit 0
fi

[ "${KIS_STOCK_PRICE_BETA_CONFIRM:-}" = "$confirmation" ] ||
  blocked 'set KIS_STOCK_PRICE_BETA_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS for execute'
case "$prepared_image" in
  0|1) ;;
  *) blocked 'KIS_STOCK_PRICE_BETA_IMAGE_PREPARED must be exactly 0 or 1' ;;
esac

running_services=$(compose ps --status running --services 2>/dev/null) ||
  blocked 'cannot inspect running Compose services'
if grep -Fxq research-worker <<<"$running_services"; then
  blocked 'ordinary research-worker daemon is running; stop it through a separate operator protection workflow'
fi
if grep -Fxq "$compose_service" <<<"$running_services"; then
  blocked 'another fixed-stock Raw one-shot is already running'
fi

if [ "$prepared_image" = 0 ]; then
  compose build --pull=false "$compose_service" ||
    die 'fixed-stock Raw image build failed'
fi

image="lagrange-station-research-stock-price-beta-raw:$commit"
docker image inspect "$image" >/dev/null 2>&1 ||
  die 'fixed-stock Raw image was not produced for the requested commit'
revision=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' \
  "$image" 2>/dev/null) || die 'cannot inspect fixed-stock Raw image OCI revision'
[ "$revision" = "$commit" ] ||
  die 'fixed-stock Raw image OCI revision does not match LAGRANGE_CODE_COMMIT'
image_commit=$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' \
  "$image" 2>/dev/null | awk -F= '$1 == "LAGRANGE_CODE_COMMIT" { print substr($0, index($0, "=") + 1); exit }')
[ "$image_commit" = "$commit" ] ||
  die 'fixed-stock Raw image ENV LAGRANGE_CODE_COMMIT does not match the requested commit'

# The acknowledgement is passed as a Compose run override rather than stored
# in the production env file. It is exact by design and is not a credential.
compose run --rm --no-deps -e "KIS_STOCK_PRICE_BETA_CONFIRM=$confirmation" "$compose_service" \
  --start "$start_date" --end "$end_date" --execute ||
  die 'fixed-stock Raw one-shot failed'
echo 'KIS_STOCK_PRICE_BETA_RAW: PASS (fixed 30-stock Raw capture completed)'
