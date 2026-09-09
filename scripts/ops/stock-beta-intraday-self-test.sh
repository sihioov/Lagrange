#!/usr/bin/env bash
# Provider-free mutation self-test for the WP6-A artifact/schema checker.
# Every mutation is made below a throw-away mktemp directory; no operational
# file, provider, credential, clock, Docker, database, or network is touched.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
static=$root/scripts/ops/stock-beta-intraday-static-check.sh
artifact_rel=configs/market-hours/krx-intraday-session-windows-v1.json
schema_rel=configs/market-hours/krx-intraday-session-windows-v1.schema.json
static_rel=scripts/ops/stock-beta-intraday-static-check.sh
env_example_rel=deploy/compose/.env.example
provision_rel=scripts/ops/provision-linux.sh
validator_rel=scripts/ops/validate-production-config.sh
self_test_rel=scripts/ops/stock-beta-intraday-self-test.sh

# The validator is intentionally root-only because production metadata is
# protected. Run the complete fixture suite in one simulated-root process;
# never touch the real root, sudo, accounts, Docker, services, or host paths.
if [ "$(id -u)" -ne 0 ]; then
  if [ "${LAGRANGE_B1_ROOT_FIXTURE_CHILD:-0}" = 1 ]; then
    echo 'TEST_ENVIRONMENT_ERROR: intraday root fixture did not obtain root identity' >&2
    exit 1
  fi
  script_path=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/$(basename "${BASH_SOURCE[0]}")
  if unshare -Ur true >/dev/null 2>&1; then
    exec unshare -Ur env LAGRANGE_B1_ROOT_FIXTURE_CHILD=1 \
      bash "$script_path" "$@"
  fi
  if command -v fakeroot >/dev/null 2>&1; then
    exec fakeroot env LAGRANGE_B1_ROOT_FIXTURE_CHILD=1 \
      bash "$script_path" "$@"
  fi
  echo 'TEST_ENVIRONMENT_ERROR: intraday root fixture requires user namespaces or fakeroot' >&2
  exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/stock-beta-intraday-self-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

die() {
  echo "STOCK_BETA_INTRADAY_SELF_TEST: $*" >&2
  exit 1
}

[ -f "$static" ] || die 'STATIC_CHECK_MISSING'
[ ! -L "$static" ] || die 'STATIC_CHECK_SYMLINK'
command -v python3 >/dev/null 2>&1 || die 'PYTHON3_MISSING'
bash -n "$static" || die 'STATIC_CHECK_SYNTAX_INVALID'

copy_fixture() {
  local fixture=$1
  mkdir -p "$fixture/configs/market-hours" "$fixture/scripts/ops" \
    "$fixture/deploy/compose"
  cp -- "$root/$artifact_rel" "$fixture/$artifact_rel"
  cp -- "$root/$schema_rel" "$fixture/$schema_rel"
  cp -- "$root/$static_rel" "$fixture/$static_rel"
  cp -- "$root/$env_example_rel" "$fixture/$env_example_rel"
  cp -- "$root/$provision_rel" "$fixture/$provision_rel"
  cp -- "$root/$validator_rel" "$fixture/$validator_rel"
  cp -- "$root/$self_test_rel" "$fixture/$self_test_rel"
}

rewrite_json() {
  local path=$1
  local mutation=$2
  python3 - "$path" "$mutation" <<'PY'
import json
import pathlib
import sys


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate fixture key")
        result[key] = value
    return result


path = pathlib.Path(sys.argv[1])
mutation = sys.argv[2]
document = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)

if mutation == "nonempty":
    document["entries"].append(
        {
            "date": "2026-09-09",
            "disposition": "REGULAR",
            "open_local": "09:00:00",
            "close_local": "15:30:00",
            "evidence_url": "https://global.krx.co.kr/fixture",
            "evidence_retrieved_at": "2026-09-09T00:00:00Z",
            "evidence_sha256": "sha256:" + ("0" * 64),
        }
    )
elif mutation == "extra":
    document["fixture_extra"] = True
elif mutation == "missing":
    del document["timezone"]
elif mutation == "bool":
    document["schema_version"] = True
elif mutation == "wrong_type":
    document["entries"] = {}
elif mutation == "schema_open":
    document["additionalProperties"] = True
elif mutation == "schema_old_url_pattern":
    document["$defs"]["entry"]["properties"]["evidence_url"]["pattern"] = (
        r"^https://global\.krx\.co\.kr/[^\s]+$"
    )
elif mutation == "schema_missing_closed":
    document["$defs"]["entry"]["allOf"] = [
        clause
        for clause in document["$defs"]["entry"]["allOf"]
        if clause["if"]["properties"]["disposition"]["const"] != "CLOSED"
    ]
elif mutation == "schema_missing_date_format":
    del document["$defs"]["entry"]["properties"]["date"]["format"]
elif mutation == "schema_bool_const":
    document["properties"]["schema_version"]["const"] = True
else:
    raise ValueError("unknown fixture mutation")

path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
PY
}

duplicate_artifact_key() {
  local path=$1
  python3 - "$path" <<'PY'
import pathlib
import sys


path = pathlib.Path(sys.argv[1])
raw = path.read_text(encoding="utf-8")
needle = '  "exchange": "KRX",\n'
if raw.count(needle) != 1:
    raise SystemExit("fixture duplicate target is not unique")
path.write_text(raw.replace(needle, needle + needle, 1), encoding="utf-8")
PY
}

run_url_pattern_tests() {
  local pattern_file=$1
  python3 - "$pattern_file" <<'PY'
import json
import pathlib
import re
import sys


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise SystemExit("URL_PATTERN_DUPLICATE_SCHEMA_KEY")
        result[key] = value
    return result


schema = json.loads(
    pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"),
    object_pairs_hook=unique_object,
)
pattern = schema["$defs"]["entry"]["properties"]["evidence_url"]["pattern"]
prefix = "https://global.krx.co.kr/"
path = "fixture"
case_count = 0


def check(label, value, expected):
    global case_count
    case_count += 1
    actual = re.search(pattern, value) is not None
    if actual is not expected:
        raise SystemExit(f"URL_PATTERN_CASE_FAILED_{label}")


# Deliberately use re.search, never fullmatch: this exercises the same
# unanchored pattern semantics that exposed the old terminal-LF defect.
check("valid", prefix + path, True)
for code in list(range(32)) + [127]:
    character = chr(code)
    check(f"control_{code}_interior", prefix + "fi" + character + "xture", False)
    check(f"control_{code}_suffix", prefix + path + character, False)
check("trailing_lf", prefix + path + "\n", False)
check("trailing_cr", prefix + path + "\r", False)

if case_count != 69:
    raise SystemExit("URL_PATTERN_CASE_COUNT_INVALID")
print(case_count)
PY
}

pass_count=0
expected_failure_count=0
actual_nonzero_count=0
regex_case_count=$(run_url_pattern_tests "$root/$schema_rel") || die 'URL_PATTERN_TEST_FAILED'
[ "$regex_case_count" -eq 69 ] || die "URL_PATTERN_COUNT_INVALID cases=$regex_case_count"
printf 'STOCK_BETA_INTRADAY_SELF_TEST: regex_cases=%s regex_passed=%s\n' \
  "$regex_case_count" "$regex_case_count"

expect_pass() {
  local name=$1
  local fixture=$2
  local output status
  if output=$(bash "$fixture/$static_rel" 2>&1); then
    status=0
  else
    status=$?
  fi
  [ "$status" -eq 0 ] || {
    printf '%s\n' "$output" >&2
    die "CASE_FAILED name=$name exit=$status expected=0"
  }
  grep -Fq 'STOCK_BETA_INTRADAY_STATIC: PASS' <<<"$output" ||
    die "CASE_PASS_MARKER_MISSING name=$name"
  pass_count=$((pass_count + 1))
  printf 'STOCK_BETA_INTRADAY_SELF_TEST: case=%s exit=%s result=PASS\n' "$name" "$status"
}

expect_failure() {
  local name=$1
  local fixture=$2
  local output status
  if output=$(bash "$fixture/$static_rel" 2>&1); then
    status=0
  else
    status=$?
  fi
  [ "$status" -ne 0 ] || {
    printf '%s\n' "$output" >&2
    die "CASE_FAILED name=$name exit=$status expected=nonzero"
  }
  grep -Fq 'STOCK_BETA_INTRADAY_STATIC:' <<<"$output" ||
    die "CASE_UNTYPED_FAILURE name=$name"
  if grep -Fq 'Traceback' <<<"$output"; then
    die "CASE_UNSAFE_FAILURE_OUTPUT name=$name"
  fi
  expected_failure_count=$((expected_failure_count + 1))
  actual_nonzero_count=$((actual_nonzero_count + 1))
  printf 'STOCK_BETA_INTRADAY_SELF_TEST: case=%s exit=%s result=EXPECTED_FAILURE\n' \
    "$name" "$status"
}

baseline=$tmp/baseline
copy_fixture "$baseline"
expect_pass baseline "$baseline"

fixture=$tmp/nonempty
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" nonempty
expect_failure nonempty-artifact "$fixture"

fixture=$tmp/extra
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" extra
expect_failure extra-artifact-key "$fixture"

fixture=$tmp/missing
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" missing
expect_failure missing-artifact-key "$fixture"

fixture=$tmp/bool
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" bool
expect_failure boolean-schema-version "$fixture"

fixture=$tmp/wrong-type
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" wrong_type
expect_failure wrong-artifact-type "$fixture"

fixture=$tmp/duplicate
copy_fixture "$fixture"
duplicate_artifact_key "$fixture/$artifact_rel"
expect_failure duplicate-artifact-key "$fixture"

fixture=$tmp/schema-open
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_open
expect_failure schema-open-top-level "$fixture"

fixture=$tmp/schema-old-url-pattern
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_old_url_pattern
expect_failure schema-old-weak-url-pattern "$fixture"

fixture=$tmp/schema-missing-conditional
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_missing_closed
expect_failure schema-missing-closed-constraint "$fixture"

fixture=$tmp/schema-missing-format
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_missing_date_format
expect_failure schema-missing-date-format "$fixture"

fixture=$tmp/schema-bool
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_bool_const
expect_failure schema-boolean-const "$fixture"

[ "$pass_count" -eq 1 ] || die "COUNT_MISMATCH passes=$pass_count expected=1"
[ "$expected_failure_count" -eq 11 ] ||
  die "COUNT_MISMATCH checker_mutations=$expected_failure_count expected=11"
[ "$actual_nonzero_count" -eq 11 ] ||
  die "COUNT_MISMATCH checker_nonzero=$actual_nonzero_count expected=11"

rewrite_text() {
  local path=$1 old=$2 new=$3
  python3 - "$path" "$old" "$new" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
old = sys.argv[2]
new = sys.argv[3]
text = path.read_text(encoding="utf-8")
count = text.count(old)
if count != 1:
    raise SystemExit(f"TEXT_REWRITE_TARGET_COUNT_{count}")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
PY
}

b1_static_mutations=0
expect_static_failure_b1() {
  local name=$1 fixture=$2 output status
  if output=$(bash "$fixture/$static_rel" 2>&1); then
    status=0
  else
    status=$?
  fi
  [ "$status" -ne 0 ] || {
    printf '%s\n' "$output" >&2
    die "B1_STATIC_MUTATION_PASSED name=$name"
  }
  grep -Fq 'STOCK_BETA_INTRADAY_STATIC:' <<<"$output" ||
    die "B1_STATIC_MUTATION_UNTYPED name=$name"
  b1_static_mutations=$((b1_static_mutations + 1))
  printf 'STOCK_BETA_INTRADAY_SELF_TEST: b1_static_case=%s exit=%s result=EXPECTED_FAILURE\n' \
    "$name" "$status"
}

fixture=$tmp/static-env-enabled
copy_fixture "$fixture"
rewrite_text "$fixture/$env_example_rel" 'OWNER_INTRADAY_QUOTES_MODE=off' \
  'OWNER_INTRADAY_QUOTES_MODE=owner_only'
expect_static_failure_b1 env-default-enabled "$fixture"

fixture=$tmp/static-alias-hook-missing
copy_fixture "$fixture"
python3 - "$fixture/$validator_rel" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = "    OWNER_INTRADAY_QUOTES_MODE_FILE"
if text.count(needle) != 1:
    raise SystemExit("ALIAS_HOOK_TARGET_COUNT")
path.write_text(text.replace(needle, "    # alias removed", 1), encoding="utf-8")
PY
expect_static_failure_b1 validator-alias-hook-missing "$fixture"

fixture=$tmp/static-provision-hook-missing
copy_fixture "$fixture"
rewrite_text "$fixture/$provision_rel" \
  '  check_coordination_dir "$coordination_leaf" "$worker_uid" "$worker_gid" 700 kis-read-coordination' \
  '  :'
expect_static_failure_b1 provision-leaf-check-missing "$fixture"

# The following fixture uses the real production validator and provisioner.
# Only account/NSS commands are narrow stubs; every path argument is inside
# this mktemp tree and any unexpected account mutation fails the test.
b1=$tmp/b1
fake_bin=$b1/fake-bin
account_log=$b1/account-commands.log
mkdir -p "$fake_bin" "$b1"

cat >"$fake_bin/id" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  '-u') printf '0\n' ;;
  '-u lagrange') printf '10002\n' ;;
  '-G lagrange') printf '10002 10001\n' ;;
  *) echo 'unexpected fixture id query' >&2; exit 97 ;;
esac
SH
cat >"$fake_bin/getent" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  'group') printf 'lagrange:x:10002:\nlagrange-data:x:10001:\n' ;;
  'group lagrange') printf 'lagrange:x:10002:\n' ;;
  'group lagrange-data') printf 'lagrange-data:x:10001:\n' ;;
  'passwd') printf 'lagrange:x:10002:10002::/nonexistent:/usr/sbin/nologin\n' ;;
  'passwd lagrange') printf 'lagrange:x:10002:10002::/nonexistent:/usr/sbin/nologin\n' ;;
  *) echo 'unexpected fixture getent query' >&2; exit 97 ;;
esac
SH
cat >"$fake_bin/install" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
map_owner() {
  case "$1" in
    root) printf '0' ;;
    lagrange) printf '10002' ;;
    10001|10002) printf '%s' "$1" ;;
    *) echo 'unexpected fixture install owner' >&2; exit 97 ;;
  esac
}
map_group() {
  case "$1" in
    root) printf '0' ;;
    lagrange) printf '10002' ;;
    lagrange-data) printf '10001' ;;
    10001|10002) printf '%s' "$1" ;;
    *) echo 'unexpected fixture install group' >&2; exit 97 ;;
  esac
}
args=()
targets=()
owner=
group=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--owner) [ "$#" -ge 2 ] || exit 97; owner=$(map_owner "$2"); shift 2 ;;
    -g|--group) [ "$#" -ge 2 ] || exit 97; group=$(map_group "$2"); shift 2 ;;
    --owner=*) owner=$(map_owner "${1#--owner=}"); shift ;;
    --group=*) group=$(map_group "${1#--group=}"); shift ;;
    --)
      args+=(--)
      shift
      while [ "$#" -gt 0 ]; do targets+=("$1"); args+=("$1"); shift; done
      ;;
    *) args+=("$1"); shift ;;
  esac
done
/usr/bin/install "${args[@]}"
if [ -n "$owner" ] || [ -n "$group" ]; then
  [ -n "$owner" ] || owner=0
  [ -n "$group" ] || group=0
  for target in "${targets[@]}"; do
    /usr/bin/chown --no-dereference "$owner:$group" -- "$target" || true
  done
fi
SH
for account_command in groupadd useradd usermod; do
  cat >"$fake_bin/$account_command" <<SH
#!/usr/bin/env bash
set -euo pipefail
printf '%s %s\\n' '$account_command' "\$*" >>'$account_log'
echo 'unexpected fixture account mutation' >&2
exit 97
SH
done
cat >"$fake_bin/chown" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
/usr/bin/chown "$@" 2>/dev/null || true
SH
chmod 0755 "$fake_bin/id" "$fake_bin/getent" "$fake_bin/install" \
  "$fake_bin/groupadd" "$fake_bin/useradd" "$fake_bin/usermod" "$fake_bin/chown"
PATH="$fake_bin:$PATH"

prov_base=$b1/provision
prov_config=$prov_base/etc
prov_deploy=$prov_base/opt
prov_data=$prov_base/data
prov_artifacts=$prov_base/artifacts
prov_secrets=$prov_config/secrets
prov_runtime_secrets=$b1/runtime-secrets
prov_state=$b1/runtime-state
provision_output() {
  local coordination=$1 state_root=$2 action=$3 output=$4
  if env \
    "PATH=$fake_bin:$PATH" \
    "LAGRANGE_CONFIG_ROOT=$prov_config" \
    "LAGRANGE_DEPLOY_ROOT=$prov_deploy" \
    "LAGRANGE_DATA_ROOT=$prov_data" \
    "LAGRANGE_ARTIFACTS_DIR=$prov_artifacts" \
    "LAGRANGE_HOST_SECRET_ROOT=$prov_secrets" \
    "LAGRANGE_RUNTIME_SECRET_DIR=$prov_runtime_secrets" \
    "LAGRANGE_RUNTIME_STATE_DIR=$state_root" \
    "KIS_READ_COORDINATION_MODE=$coordination" \
    bash "$root/$provision_rel" "$action" >"$output" 2>&1; then
    return 0
  else
    local status=$?
    sed -n '1,120p' "$output" >&2
    return "$status"
  fi
}

old_provision_output=$b1/provision-old.out
provision_output legacy '' --dry-run "$old_provision_output" ||
  die 'B1_DEFAULT_PROVISION_FAILED'
[ ! -e "$prov_base" ] || die 'B1_DEFAULT_PROVISION_CREATED_PATHS'
grep -Fq 'coordination=disabled' "$old_provision_output" ||
  die 'B1_DEFAULT_PROVISION_MARKER_MISSING'

if provision_output shared_required '' --dry-run "$b1/provision-shared-missing.out"; then
  die 'B1_SHARED_PROVISION_WITHOUT_ROOT_PASSED'
fi
grep -Fq 'shared_required coordination requires explicit LAGRANGE_RUNTIME_STATE_DIR' \
  "$b1/provision-shared-missing.out" || die 'B1_SHARED_PROVISION_ERROR_MISSING'

provision_output shared_required "$prov_state" --dry-run "$b1/provision-dry-run.out" ||
  die 'B1_EXPLICIT_PROVISION_DRY_RUN_FAILED'
[ ! -e "$prov_state" ] || die 'B1_DRY_RUN_CREATED_COORDINATION_TREE'
grep -Fq 'coordination-leaf=' "$b1/provision-dry-run.out" ||
  die 'B1_DRY_RUN_COORDINATION_PLAN_MISSING'

provision_output shared_required "$prov_state" --apply "$b1/provision-apply-one.out" ||
  die 'B1_SIMULATED_APPLY_FAILED'
coordination_leaf=$prov_state/kis-read-coordination
[ "$(stat -c '%u:%g:%a' -- "$prov_state")" = '0:10001:750' ] ||
  die 'B1_PARENT_METADATA_INVALID_AFTER_APPLY'
[ "$(stat -c '%u:%g:%a' -- "$coordination_leaf")" = '10001:10001:700' ] ||
  die 'B1_LEAF_METADATA_INVALID_AFTER_APPLY'
[ -z "$(find "$coordination_leaf" -mindepth 1 -maxdepth 1 -print -quit)" ] ||
  die 'B1_APPLY_CREATED_RUNTIME_FILES'

sentinel_value='B1_SENTINEL_DO_NOT_PRINT'
printf '%s' "$sentinel_value" >"$coordination_leaf/state-v1.json"
printf '%s' lock-fixture >"$coordination_leaf/coordination.lock"
printf '%s' temp-fixture >"$coordination_leaf/.state-v1.tmp.001"
chown 10001:10001 "$coordination_leaf/state-v1.json" \
  "$coordination_leaf/coordination.lock" "$coordination_leaf/.state-v1.tmp.001"
chmod 0600 "$coordination_leaf/state-v1.json" \
  "$coordination_leaf/coordination.lock" "$coordination_leaf/.state-v1.tmp.001"
state_inode_before=$(stat -c '%i' -- "$coordination_leaf/state-v1.json")
lock_inode_before=$(stat -c '%i' -- "$coordination_leaf/coordination.lock")
provision_output shared_required "$prov_state" --apply "$b1/provision-apply-two.out" ||
  die 'B1_SIMULATED_IDEMPOTENT_APPLY_FAILED'
[ "$(stat -c '%i' -- "$coordination_leaf/state-v1.json")" = "$state_inode_before" ] ||
  die 'B1_IDEMPOTENT_APPLY_CHANGED_STATE_INODE'
[ "$(stat -c '%i' -- "$coordination_leaf/coordination.lock")" = "$lock_inode_before" ] ||
  die 'B1_IDEMPOTENT_APPLY_CHANGED_LOCK_INODE'
grep -Fq "$sentinel_value" "$coordination_leaf/state-v1.json" ||
  die 'B1_IDEMPOTENT_APPLY_CHANGED_SENTINEL'
if grep -Fq "$sentinel_value" "$b1/provision-apply-two.out"; then
  die 'B1_PROVISION_SENTINEL_LEAK_APPLY'
fi
provision_output shared_required "$prov_state" --preflight "$b1/provision-preflight.out" ||
  die 'B1_SIMULATED_PREFLIGHT_FAILED'
if grep -Fq "$sentinel_value" "$b1/provision-preflight.out"; then
  die 'B1_PROVISION_SENTINEL_LEAK_PREFLIGHT'
fi
[ ! -s "$account_log" ] || die 'B1_ACCOUNT_COMMAND_ESCAPED_FIXTURE'

mkdir -p "$b1/path-real" "$b1/path-link-parent"
ln -s "$b1/path-real" "$b1/path-link-parent/link"
if provision_output legacy "$b1/path-link-parent/link/state" --dry-run "$b1/provision-symlink.out"; then
  die 'B1_PROVISION_SYMLINK_PATH_PASSED'
fi
grep -Fq 'must not traverse a symlink' "$b1/provision-symlink.out" ||
  die 'B1_PROVISION_SYMLINK_ERROR_MISSING'
if provision_output legacy "$prov_data/raw" --dry-run "$b1/provision-overlap.out"; then
  die 'B1_PROVISION_OVERLAP_PATH_PASSED'
fi
grep -Fq 'overlaps an existing protected tree' "$b1/provision-overlap.out" ||
  die 'B1_PROVISION_OVERLAP_ERROR_MISSING'

validator=$root/$validator_rel
validator_source=$b1/validator-source
validator_runtime=$b1/validator-runtime
validator_data=$b1/validator-data
mkdir -p "$validator_source" "$validator_runtime/research-range-raw" "$validator_data"
printf '%s' fixture-kis-key >"$validator_source/kis_app_key"
printf '%s' fixture-kis-secret >"$validator_source/kis_app_secret"
chmod 0400 "$validator_source/kis_app_key" "$validator_source/kis_app_secret"
printf '%s' fixture-runtime-key >"$validator_runtime/research-range-raw/kis_app_key"
printf '%s' fixture-runtime-secret >"$validator_runtime/research-range-raw/kis_app_secret"
chown 10001:10001 "$validator_runtime/research-range-raw/kis_app_key" \
  "$validator_runtime/research-range-raw/kis_app_secret"
chmod 0440 "$validator_runtime/research-range-raw/kis_app_key" \
  "$validator_runtime/research-range-raw/kis_app_secret"

validator_commit=0000000000000000000000000000000000000000
valid_window_hash=sha256:0000000000000000000000000000000000000000000000000000000000000000
write_range_env() {
  local path=$1 include_new=$2 coordination=$3 intraday=$4 generation=$5 state_root=$6 window_hash=$7
  {
    printf 'LAGRANGE_DATA_DIR=%s\n' "$validator_data"
    printf 'LAGRANGE_SECRET_SOURCE_DIR=%s\n' "$validator_source"
    printf 'LAGRANGE_RUNTIME_SECRET_DIR=%s\n' "$validator_runtime"
    printf 'RESEARCH_APP_ENV=production\n'
    printf 'RESEARCH_FETCH_MODE=credentialed\n'
    printf 'RESEARCH_ENTITLEMENT_REFERENCE=fixture-entitlement\n'
    printf 'RESEARCH_CANDIDATE_ENABLED=false\n'
    if [ "$include_new" = yes ]; then
      printf 'OWNER_INTRADAY_QUOTES_MODE=%s\n' "$intraday"
      printf 'KIS_READ_COORDINATION_MODE=%s\n' "$coordination"
      printf 'KIS_READ_CREDENTIAL_GENERATION=%s\n' "$generation"
      printf 'LAGRANGE_RUNTIME_STATE_DIR=%s\n' "$state_root"
      printf 'OWNER_INTRADAY_SESSION_WINDOWS_SHA256=%s\n' "$window_hash"
    fi
  } >"$path"
  chmod 0600 "$path"
}

validator_cases=0
validator_passes=0
validator_failures=0
validator_expect() {
  local name=$1 expected=$2 env_file=$3 output status marker
  shift 3
  output=$b1/validator-$name.out
  if env LAGRANGE_ENV_FILE="$env_file" LAGRANGE_CODE_COMMIT="$validator_commit" "$@" \
       bash "$validator" --scope range-raw >"$output" 2>&1; then
    status=0
  else
    status=$?
  fi
  [ "$status" -eq "$expected" ] || {
    sed -n '1,80p' "$output" >&2
    die "B1_VALIDATOR_CASE_STATUS name=$name got=$status expected=$expected"
  }
  if [ "$expected" -eq 0 ]; then
    grep -Fq 'PRODUCTION_CONFIG: PASS' "$output" ||
      die "B1_VALIDATOR_PASS_MARKER_MISSING name=$name"
  else
    marker=INVALID_CONFIG:
    [ "$expected" -eq 2 ] && marker=BLOCKED_EXTERNAL:
    grep -Fq "$marker" "$output" || die "B1_VALIDATOR_FAILURE_MARKER_MISSING name=$name"
  fi
  if grep -Fq "$sentinel_value" "$output"; then
    die "B1_VALIDATOR_SENTINEL_LEAK name=$name"
  fi
  validator_cases=$((validator_cases + 1))
  if [ "$expected" -eq 0 ]; then validator_passes=$((validator_passes + 1));
  else validator_failures=$((validator_failures + 1)); fi
  printf 'STOCK_BETA_INTRADAY_SELF_TEST: validator_case=%s exit=%s result=%s\n' \
    "$name" "$status" "$([ "$expected" -eq 0 ] && echo PASS || echo EXPECTED_FAILURE)"
}

old_env=$b1/validator-old.env
write_range_env "$old_env" no '' '' '' '' '' ''
validator_expect defaults-absent 0 "$old_env"

off_env=$b1/validator-off.env
write_range_env "$off_env" yes legacy off not-a-number '' '' not-a-hash
validator_expect explicit-off-ignores-hash 0 "$off_env"

valid_env=$b1/validator-valid.env
write_range_env "$valid_env" yes shared_required owner_only 18446744073709551615 \
  "$prov_state" "$valid_window_hash"
validator_expect valid-shared-owner-only 0 "$valid_env"

missing_state_env=$b1/validator-missing-state.env
write_range_env "$missing_state_env" yes shared_required off 17 "$b1/missing-state" ''
validator_expect shared-missing-filesystem 2 "$missing_state_env"
[ ! -e "$b1/missing-state" ] || die 'B1_VALIDATOR_CREATED_MISSING_STATE'

for bad_mode in '' disabled SHARED_REQUIRED ' legacy'; do
  case_name=coordination-mode-${#bad_mode}
  case_env=$b1/$case_name.env
  write_range_env "$case_env" yes "$bad_mode" off 17 '' ''
  validator_expect "$case_name" 1 "$case_env"
done
for bad_mode in '' disabled OWNER_ONLY; do
  case_name=intraday-mode-${#bad_mode}
  case_env=$b1/$case_name.env
  write_range_env "$case_env" yes legacy "$bad_mode" '' '' ''
  validator_expect "$case_name" 1 "$case_env"
done
legacy_owner_env=$b1/validator-owner-only-legacy.env
write_range_env "$legacy_owner_env" yes legacy owner_only '' '' ''
validator_expect owner-only-requires-shared 1 "$legacy_owner_env"

for bad_generation in '' 0 01 +1 ' 1' \
  18446744073709551616 184467440737095516150; do
  case_name=generation-${#bad_generation}
  case_env=$b1/$case_name.env
  write_range_env "$case_env" yes shared_required off "$bad_generation" "$prov_state" ''
  validator_expect "$case_name" 1 "$case_env"
done

missing_hash_env=$b1/validator-missing-hash.env
write_range_env "$missing_hash_env" yes shared_required owner_only 17 "$prov_state" ''
validator_expect owner-only-missing-hash 2 "$missing_hash_env"
bad_hash_env=$b1/validator-bad-hash.env
write_range_env "$bad_hash_env" yes shared_required owner_only 17 "$prov_state" sha256:BAD
validator_expect owner-only-bad-hash 1 "$bad_hash_env"

alias_file_env=$b1/validator-file-alias.env
cp -- "$valid_env" "$alias_file_env"
printf '%s\n' OWNER_INTRADAY_QUOTES_MODE_FILE=off >>"$alias_file_env"
chmod 0600 "$alias_file_env"
validator_expect file-alias-rejected 1 "$alias_file_env"
validator_expect shell-alias-rejected 1 "$valid_env" KIS_READ_COORDINATION_MODE_FILE=legacy
validator_expect shell-override-mismatch 1 "$valid_env" KIS_READ_COORDINATION_MODE=legacy
validator_expect shell-only-mode-mismatch 1 "$old_env" OWNER_INTRADAY_QUOTES_MODE=owner_only
validator_expect shell-only-root-mismatch 1 "$old_env" LAGRANGE_RUNTIME_STATE_DIR="$prov_state"

for path_case in relative/state /./state /var /; do
  case_env=$b1/validator-path-${#path_case}.env
  write_range_env "$case_env" yes legacy off '' "$path_case" ''
  validator_expect "unsafe-path-${#path_case}" 1 "$case_env"
done
overlap_data_env=$b1/validator-overlap-data.env
write_range_env "$overlap_data_env" yes legacy off '' "$validator_data/raw" ''
validator_expect overlap-raw-tree 1 "$overlap_data_env"
overlap_secret_env=$b1/validator-overlap-secret.env
write_range_env "$overlap_secret_env" yes legacy off '' "$validator_source/state" ''
validator_expect overlap-source-tree 1 "$overlap_secret_env"

mkdir -p "$b1/symlink-parent/real"
ln -s "$b1/symlink-parent/real" "$b1/symlink-parent/link"
symlink_env=$b1/validator-symlink-path.env
write_range_env "$symlink_env" yes legacy off '' "$b1/symlink-parent/link/state" ''
validator_expect unsafe-symlink-path 1 "$symlink_env"

# The sandbox may deny AF_UNIX filesystem nodes. Probe once and, when denied,
# use a clearly labelled exact-path `stat` simulation through the real
# validator; no capability is silently skipped.
socket_fixture_mode=real
socket_probe=$b1/socket-probe
if python3 - "$socket_probe" 2>/dev/null <<'PY'
import socket
import sys

sock = socket.socket(socket.AF_UNIX)
sock.bind(sys.argv[1])
sock.close()
PY
then
  rm -f -- "$socket_probe"
else
  socket_fixture_mode=narrow-simulation
fi

# Empty, lock-only, and full stores all pass metadata validation. The content
# below is never parsed by the validator; it is only a sentinel for the
# no-mutation/no-leak assertions around every invalid fixture.
find "$coordination_leaf" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
empty_env=$b1/validator-empty.env
write_range_env "$empty_env" yes shared_required off 17 "$prov_state" ''
validator_expect safe-empty-store 0 "$empty_env"
printf '%s' lock-fixture >"$coordination_leaf/coordination.lock"
chown 10001:10001 "$coordination_leaf/coordination.lock"
chmod 0600 "$coordination_leaf/coordination.lock"
validator_expect safe-lock-only-store 0 "$empty_env"
printf '%s' "$sentinel_value" >"$coordination_leaf/state-v1.json"
printf '%s' temp-fixture >"$coordination_leaf/.state-v1.tmp.001"
printf '%s' fixture-sentinel >"$coordination_leaf/.fixture-sentinel"
chown 10001:10001 "$coordination_leaf/state-v1.json" \
  "$coordination_leaf/.state-v1.tmp.001" "$coordination_leaf/.fixture-sentinel"
chmod 0600 "$coordination_leaf/state-v1.json" \
  "$coordination_leaf/.state-v1.tmp.001" "$coordination_leaf/.fixture-sentinel"
sentinel_inode=$(stat -c '%i' -- "$coordination_leaf/.fixture-sentinel")
sentinel_snapshot=$b1/sentinel.snapshot
cp -- "$coordination_leaf/.fixture-sentinel" "$sentinel_snapshot"
assert_sentinel_unchanged() {
  [ "$(stat -c '%i' -- "$coordination_leaf/.fixture-sentinel")" = "$sentinel_inode" ] ||
    die 'B1_VALIDATOR_CHANGED_SENTINEL_INODE'
  cmp -s -- "$coordination_leaf/.fixture-sentinel" "$sentinel_snapshot" ||
    die 'B1_VALIDATOR_CHANGED_SENTINEL_BYTES'
}
validator_expect safe-full-store 0 "$empty_env"
assert_sentinel_unchanged

rm -f -- "$coordination_leaf/coordination.lock"
validator_expect state-without-lock 1 "$empty_env"
assert_sentinel_unchanged
printf '%s' lock-fixture >"$coordination_leaf/coordination.lock"
chown 10001:10001 "$coordination_leaf/coordination.lock"
chmod 0600 "$coordination_leaf/coordination.lock"

chown 1000:1000 "$coordination_leaf/coordination.lock"
validator_expect wrong-lock-owner 1 "$empty_env"
assert_sentinel_unchanged
chown 10001:10001 "$coordination_leaf/coordination.lock"
chmod 0640 "$coordination_leaf/coordination.lock"
validator_expect wrong-lock-mode 1 "$empty_env"
assert_sentinel_unchanged
chmod 0600 "$coordination_leaf/coordination.lock"

rm -f -- "$coordination_leaf/coordination.lock"
mkdir -- "$coordination_leaf/coordination.lock"
validator_expect lock-directory 1 "$empty_env"
assert_sentinel_unchanged
rmdir -- "$coordination_leaf/coordination.lock"
printf '%s' lock-fixture >"$coordination_leaf/coordination.lock"
chown 10001:10001 "$coordination_leaf/coordination.lock"
chmod 0600 "$coordination_leaf/coordination.lock"

rm -f -- "$coordination_leaf/coordination.lock"
mkfifo -- "$coordination_leaf/coordination.lock"
validator_expect lock-fifo 1 "$empty_env"
assert_sentinel_unchanged
rm -f -- "$coordination_leaf/coordination.lock"
if [ "$socket_fixture_mode" = real ]; then
  python3 - "$coordination_leaf/coordination.lock" <<'PY'
import socket
import sys

sock = socket.socket(socket.AF_UNIX)
sock.bind(sys.argv[1])
sock.close()
PY
else
  cat >"$fake_bin/stat" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = -c ] && [ "${2:-}" = '%F' ] &&
   [ "${!#}" = "${B1_SOCKET_FIXTURE_PATH:?}" ]; then
  printf 'socket\n'
  exit 0
fi
exec /usr/bin/stat "$@"
SH
  chmod 0755 "$fake_bin/stat"
  export B1_SOCKET_FIXTURE_PATH="$coordination_leaf/coordination.lock"
  : >"$coordination_leaf/coordination.lock"
fi
validator_expect lock-socket 1 "$empty_env"
assert_sentinel_unchanged
# The narrow socket simulation is scoped to this one validator invocation.
# Restore the real stat implementation before the lock-only and remaining
# metadata cases so a regular coordination.lock cannot be misclassified.
if [ "$socket_fixture_mode" = narrow-simulation ]; then
  rm -f -- "$fake_bin/stat"
  unset B1_SOCKET_FIXTURE_PATH
fi
rm -f -- "$coordination_leaf/coordination.lock"
ln -s state-v1.json "$coordination_leaf/coordination.lock"
validator_expect lock-symlink 1 "$empty_env"
assert_sentinel_unchanged
rm -f -- "$coordination_leaf/coordination.lock"
printf '%s' lock-fixture >"$coordination_leaf/coordination.lock"
ln -- "$coordination_leaf/coordination.lock" "$coordination_leaf/lock-hardlink"
validator_expect lock-hardlink 1 "$empty_env"
assert_sentinel_unchanged
rm -f -- "$coordination_leaf/coordination.lock" "$coordination_leaf/lock-hardlink"
printf '%s' lock-fixture >"$coordination_leaf/coordination.lock"
chown 10001:10001 "$coordination_leaf/coordination.lock"
chmod 0600 "$coordination_leaf/coordination.lock"

chmod 0640 "$coordination_leaf/.state-v1.tmp.001"
validator_expect temp-wrong-mode 1 "$empty_env"
assert_sentinel_unchanged
chmod 0600 "$coordination_leaf/.state-v1.tmp.001"
rm -f -- "$coordination_leaf/state-v1.json"
mkdir -- "$coordination_leaf/state-v1.json"
validator_expect state-directory 1 "$empty_env"
assert_sentinel_unchanged
rmdir -- "$coordination_leaf/state-v1.json"
printf '%s' "$sentinel_value" >"$coordination_leaf/state-v1.json"
chown 10001:10001 "$coordination_leaf/state-v1.json"
chmod 0600 "$coordination_leaf/state-v1.json"

chmod 0755 "$prov_state"
validator_expect wrong-parent-mode 1 "$empty_env"
assert_sentinel_unchanged
chmod 0750 "$prov_state"
chown 1000:1000 "$prov_state"
validator_expect wrong-parent-owner 1 "$empty_env"
assert_sentinel_unchanged
chown 0:10001 "$prov_state"
chmod 0755 "$coordination_leaf"
validator_expect wrong-leaf-mode 1 "$empty_env"
assert_sentinel_unchanged
chmod 0700 "$coordination_leaf"

mv -- "$coordination_leaf" "$b1/leaf-real"
ln -s "$b1/leaf-real" "$coordination_leaf"
validator_expect leaf-symlink 1 "$empty_env"
rm -- "$coordination_leaf"
mv -- "$b1/leaf-real" "$coordination_leaf"
assert_sentinel_unchanged

mv -- "$prov_state" "$b1/state-root-real"
ln -s "$b1/state-root-real" "$prov_state"
validator_expect parent-symlink 1 "$empty_env"
rm -- "$prov_state"
mv -- "$b1/state-root-real" "$prov_state"
assert_sentinel_unchanged

validator_expect final-valid-store 0 "$empty_env"
assert_sentinel_unchanged

printf 'STOCK_BETA_INTRADAY_SELF_TEST: b1_static_mutations=%s validator_cases=%s validator_passes=%s validator_expected_failures=%s provision_fixture=PASS idempotent=PASS sentinel=PASS socket_fixture=%s\n' \
  "$b1_static_mutations" "$validator_cases" "$validator_passes" "$validator_failures" "$socket_fixture_mode"
printf 'STOCK_BETA_INTRADAY_SELF_TEST: PASS regex_cases=%s regex_passed=%s checker_mutations=%s checker_nonzero=%s total_cases=%s\n' \
  "$regex_case_count" "$regex_case_count" "$expected_failure_count" "$actual_nonzero_count" \
  "$((pass_count + expected_failure_count))"
