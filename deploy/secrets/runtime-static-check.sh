#!/usr/bin/env bash
# Static contract check for service-specific native-Linux secret delivery.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
compose="$root/deploy/compose/compose.yml"
provision="$root/deploy/secrets/provision-runtime-secrets.sh"
production_validator="$root/scripts/ops/validate-production-config.sh"
secret_gitignore="$root/deploy/secrets/.gitignore"
env_example="$root/deploy/compose/.env.example"
secrets_readme="$root/deploy/secrets/README.md"
db_dockerfile="$root/deploy/db/Dockerfile"
bootstrap_roles="$root/deploy/db/bootstrap-roles.sh"
paper_wrapper="$root/deploy/runtime/paper-runner-entrypoint"

die() {
  echo "secrets-runtime-static-check: $*" >&2
  exit 1
}

[ -x "$provision" ] || die "provisioner must be executable"
[ "$(stat -c '%a' "$provision")" = 755 ] || die "provisioner must have mode 0755"
[ ! -L "$provision" ] || die "provisioner must not be a symlink"
grep -Fxq '!provision-runtime-secrets.sh' "$secret_gitignore" \
  || die 'provisioner must be visible to Git while real secrets stay ignored'
if git check-ignore -q "$provision"; then
  die 'provisioner is unexpectedly ignored by Git'
fi
bash -n "$provision" || die "provisioner has shell syntax errors"
grep -Fq -- '--scope infrastructure|serving-prereqs|backfill|range-raw|range-raw-recovery|release' "$provision" \
  || die 'provisioner scope contract is absent'
grep -Fq 'scope=$scope' "$provision" \
  || die 'provisioner must report the selected scope'
grep -Fq 'serving-prereqs)' "$provision" \
  || die 'provisioner serving-prereqs scope branch is missing'
grep -Fq 'preflight_source' "$provision" \
  || die 'provisioner source preflight is missing'
grep -Fq 'for spec in "${copy_specs[@]}"' "$provision" \
  || die 'provisioner must preflight and copy the selected inventory'
grep -Fq 'before creating the runtime' "$provision" \
  || die 'provisioner must preflight before writes'
[ -f "$db_dockerfile" ] || die 'missing database one-shot Dockerfile'
[ -f "$bootstrap_roles" ] || die 'missing database role bootstrap script'
[ -f "$paper_wrapper" ] || die 'missing Paper runtime wrapper'
bash -n "$paper_wrapper" || die 'Paper runtime wrapper has shell syntax errors'
bash -n "$bootstrap_roles" || die 'database role bootstrap has shell syntax errors'
grep -Fq 'DB_DAILY_INPUT_ADMITTER_PASSWORD_FILE' "$bootstrap_roles" \
  || die 'bootstrap must accept the admitter credential by file path'
if grep -Fq 'DB_DAILY_INPUT_ADMITTER_PASSWORD=' "$bootstrap_roles"; then
  die 'bootstrap must not accept the admitter credential as an environment value'
fi
grep -Fq 'BEGIN;' "$bootstrap_roles" && grep -Fq 'COMMIT;' "$bootstrap_roles" \
  || die 'role changes and grants must be transactionally bounded'
grep -Fq 'NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE' "$bootstrap_roles" \
  || die 'admitter role attributes must remain direct-login and restricted'
grep -Fq 'NOREPLICATION NOBYPASSRLS' "$bootstrap_roles" \
  || die 'admitter role must not replicate or bypass row-level security'
grep -Fq 'WHERE member = admitter_oid OR roleid = admitter_oid' "$bootstrap_roles" \
  || die 'bootstrap must reject incoming and outgoing admitter memberships'
grep -Fq 'WITH RECURSIVE membership_closure' "$bootstrap_roles" \
  || die 'bootstrap must inspect transitive app/worker membership paths'
grep -Fq 'UNION' "$bootstrap_roles" \
  || die 'bootstrap membership closure must deduplicate cycles'
if grep -Fq 'UNION ALL' "$bootstrap_roles"; then
  die 'bootstrap membership closure must terminate on cycles'
fi
grep -Fq "has_database_privilege('daily_input_admitter'" "$bootstrap_roles" \
  || die 'bootstrap must preflight and verify effective database privileges'
grep -Fq "has_schema_privilege('daily_input_admitter'::name, ns.oid, 'CREATE')" "$bootstrap_roles" \
  || die 'bootstrap must reject effective schema CREATE privileges'
if grep -Eq 'REVOKE[^;]*daily_input_admitter' "$bootstrap_roles"; then
  die 'bootstrap must not silently revoke an unexpected admitter grant'
fi
preflight_line=$(grep -nF 'DO \$role_preflight\$' "$bootstrap_roles" | cut -d: -f1)
role_loop_line=$(grep -nF 'for role in "${role_names[@]}"; do' "$bootstrap_roles" | cut -d: -f1)
[ -n "$preflight_line" ] && [ -n "$role_loop_line" ] \
  || die 'bootstrap role preflight/mutation order is missing'
if ! (( preflight_line < role_loop_line )); then
  die 'existing role preflight must run before legacy role mutation'
fi
grep -Fq 'validate_health_state' "$paper_wrapper" \
  || die 'Paper runtime wrapper must validate loop progress in healthcheck'
grep -Fq 'cycle_deadline_at' "$paper_wrapper" \
  || die 'Paper runtime wrapper must honor bounded cycle health state'
grep -Fxq 'USER 999:999' "$db_dockerfile" \
  || die 'database one-shot image must remain non-root UID/GID 999:999'
grep -Fq 'LAGRANGE_RUNTIME_SECRET_DIR' "$compose" \
  || die 'Compose must use the runtime secret directory'
if awk '/^[[:space:]]+file:/ && $0 !~ /LAGRANGE_RUNTIME_SECRET_DIR/ {bad=1} END {exit bad}' "$compose"; then
  :
else
  die 'Compose must not mount operator source files directly'
fi

for uid in 101 999 10001; do
  grep -Fq "uid: \"$uid\"" "$compose" \
    || die "Compose is missing an explicit UID contract for $uid"
  grep -Fq "gid: \"$uid\"" "$compose" \
    || die "Compose is missing an explicit GID contract for $uid"
done
grep -Fq 'mode: 0440' "$compose" || die 'Compose is missing 0440 secret mounts'
grep -Fq 'mode: 0400' "$compose" || die 'database one-shots must use 0400 mounts'
grep -Fq 'provision-runtime-secrets.sh' "$compose" \
  || die 'Compose documentation must reference the provisioner'
grep -Fq 'deploy/secrets/provision-runtime-secrets.sh' "$env_example" \
  || die 'Compose env documentation must reference the provisioner'
grep -Fq '${LAGRANGE_SECRET_SOURCE_DIR}/db_daily_input_admitter_password' "$env_example" \
  || die 'Compose env documentation must identify the admitter source credential path'
grep -Fq 'deploy/secrets/provision-runtime-secrets.sh' "$secrets_readme" \
  || die 'secret documentation must reference the provisioner'
for service in reverse-proxy api-server db-role-bootstrap db-migrate postgres \
  research-schema-check research-worker recommendation-runner candidate-runner \
  owner-beta-runner research-range-raw nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler; do
  grep -Fq "/$service/" "$compose" \
    || die "Compose is missing service-specific runtime path for $service"
done

service_block() {
  local service=$1
  awk -v service="$service" '
    $0 == "  " service ":" { in_service=1; print; next }
    in_service && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
    in_service { print }
  ' "$compose"
}

# Docker file-backed secrets are source files on native Linux, so verify both
# sides of the ownership contract. The image, provisioner, and long syntax
# mounts must agree on the non-root UID/GID and the stricter one-shot mode.
for service in db-role-bootstrap db-migrate; do
  grep -Eq "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]].*[[:space:]]999[[:space:]]+999[[:space:]]+0400[[:space:]]+yes$" "$provision" \
    || die "provisioner must install $service secrets as 999:999 mode 0400"
  expected_count=1
  [ "$service" = db-role-bootstrap ] && expected_count=8
  provision_count=$(grep -Ec "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]]" "$provision" || true)
  [ "$provision_count" -eq "$expected_count" ] \
    || die "$service provisioner entry count changed (expected $expected_count)"
  block=$(service_block "$service")
  [ -n "$block" ] || die "missing Compose service block: $service"
  if grep -Eq 'uid: "0"|gid: "0"' <<<"$block"; then
    die "$service must not mount root-owned secrets"
  fi
  uid_count=$(grep -Ec '^[[:space:]]+uid: "999"$' <<<"$block" || true)
  gid_count=$(grep -Ec '^[[:space:]]+gid: "999"$' <<<"$block" || true)
  mode_count=$(grep -Ec '^[[:space:]]+mode: 0400$' <<<"$block" || true)
  [ "$uid_count" -eq "$expected_count" ] \
    || die "$service must mount $expected_count secrets with uid 999"
  [ "$gid_count" -eq "$expected_count" ] \
    || die "$service must mount $expected_count secrets with gid 999"
  [ "$mode_count" -eq "$expected_count" ] \
    || die "$service must mount $expected_count secrets with mode 0400"
done

# Every selected runtime inventory entry is declared once in the provisioner
# copy-spec functions and must remain aligned with validator runtime_specs. The
# serving-prereqs list intentionally includes the staged owner-beta admitter
# credential and other non-KIS serving copies but no KIS source;
# backup_encryption_key is source-only and is preflighted without a runtime
# destination. The staged admitter copy does not activate a Compose service.
for expected in \
  'db-role-bootstrap postgres_password postgres_password 999 999 0400 yes' \
  'db-role-bootstrap db_migration_owner_password db_migration_owner_password 999 999 0400 yes' \
  'db-role-bootstrap db_app_password db_app_password 999 999 0400 yes' \
  'db-role-bootstrap db_worker_password db_worker_password 999 999 0400 yes' \
  'db-role-bootstrap db_audit_password db_audit_password 999 999 0400 yes' \
  'db-role-bootstrap db_research_password db_research_password 999 999 0400 yes' \
  'db-role-bootstrap db_admin_password db_admin_password 999 999 0400 yes' \
  'db-role-bootstrap db_daily_input_admitter_password db_daily_input_admitter_password 999 999 0400 yes' \
  'db-migrate db_migration_owner_password db_migration_owner_password 999 999 0400 yes' \
  'postgres postgres_password postgres_password 999 999 0440 yes' \
  'research-schema-check postgres_password postgres_password 999 999 0440 yes' \
  'reverse-proxy lagrange_tls_cert tls/lagrange.crt 101 101 0440 no' \
  'reverse-proxy lagrange_tls_key tls/lagrange.key 101 101 0440 no' \
  'owner-beta-daily-admitter db_daily_input_admitter_password db_daily_input_admitter_password 10003 10003 0440 yes' \
  'api-server db_app_password db_app_password 10001 10001 0440 yes' \
  'api-server db_admin_password db_admin_password 10001 10001 0440 yes' \
  'api-server db_audit_password db_audit_password 10001 10001 0440 yes' \
  'api-server cursor_secret cursor_secret 10001 10001 0440 yes' \
  'api-server session_secret session_secret 10001 10001 0440 yes' \
  'api-server csrf_secret csrf_secret 10001 10001 0440 yes' \
  'api-server auth0_client_secret auth0_client_secret 10001 10001 0440 yes' \
  'research-worker db_research_password db_research_password 10001 10001 0440 yes' \
  'recommendation-runner db_worker_password db_worker_password 10001 10001 0440 yes' \
  'candidate-runner db_worker_password db_worker_password 10001 10001 0440 yes' \
  'owner-beta-runner db_worker_password db_worker_password 10001 10001 0440 yes' \
  'nt-backtest-worker-1 db_worker_password db_worker_password 10001 10001 0440 yes' \
  'nt-backtest-worker-2 db_worker_password db_worker_password 10001 10001 0440 yes' \
  'paper-scheduler db_app_password db_app_password 10001 10001 0440 yes' \
  'paper-scheduler db_worker_password db_worker_password 10001 10001 0440 yes' \
  'paper-scheduler db_admin_password db_admin_password 10001 10001 0440 yes' \
  'paper-scheduler db_audit_password db_audit_password 10001 10001 0440 yes' \
  'research-worker kis_app_key kis_app_key 10001 10001 0440 yes' \
  'research-worker kis_app_secret kis_app_secret 10001 10001 0440 yes' \
  'research-range-raw kis_app_key kis_app_key 10001 10001 0440 yes' \
  'research-range-raw kis_app_secret kis_app_secret 10001 10001 0440 yes'; do
  read -r service target source uid gid mode single_line <<<"$expected"
  grep -Eq "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]]+$target[[:space:]]+$source[[:space:]]+$uid[[:space:]]+$gid[[:space:]]+$mode[[:space:]]+$single_line$" "$provision" \
    || die "runtime provisioner inventory missing: $expected"
done

# Parse the selected release copy map and validator's release runtime_specs as
# inert source text. Never source, eval, or execute either production script to
# derive these inventories. Unknown or ambiguous shell mapping syntax fails
# closed so a new conditional/call cannot silently bypass the parity check.
command -v python3 >/dev/null 2>&1 || die 'release inventory parity requires the existing python3 runtime'
python3 - "$provision" "$production_validator" <<'PY' || die 'release provisioner and validator inventories must match by path, UID, GID, and numeric mode'
import re
import sys


class InventoryError(Exception):
    pass


COMPONENT_PATTERN = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]*", re.ASCII)
ASCII_DECIMAL_PATTERN = re.compile(r"[0-9]+", re.ASCII)
OCTAL_MODE_PATTERN = re.compile(r"[0-7]{3,4}", re.ASCII)


def require_component(value, source_label, field_name):
    if COMPONENT_PATTERN.fullmatch(value) is None:
        raise InventoryError(f"invalid {source_label} {field_name} literal")


def split_ascii_fields(value):
    stripped = value.strip(" \t")
    return re.split(r"[ \t]+", stripped) if stripped else []


def require_source_path(value, source_label):
    components = value.split("/")
    if not components or any(
        component in {".", ".."} or COMPONENT_PATTERN.fullmatch(component) is None
        for component in components
    ):
        raise InventoryError(f"invalid {source_label} source path literal")


def parse_metadata(uid, gid, mode, source_label, single_line=None):
    if ASCII_DECIMAL_PATTERN.fullmatch(uid) is None:
        raise InventoryError(f"invalid {source_label} UID literal")
    if ASCII_DECIMAL_PATTERN.fullmatch(gid) is None:
        raise InventoryError(f"invalid {source_label} GID literal")
    if OCTAL_MODE_PATTERN.fullmatch(mode) is None:
        raise InventoryError(f"invalid {source_label} mode literal")
    if single_line is not None and single_line not in {"yes", "no"}:
        raise InventoryError(f"invalid {source_label} single_line literal")
    return int(uid), int(gid), int(mode, 8)


def parse_copy(line, source_label):
    fields = split_ascii_fields(line)
    if len(fields) != 8 or fields[0] != "add_copy":
        raise InventoryError(f"unrecognized {source_label} add_copy mapping")
    _, service, target, source, uid, gid, mode, single_line = fields
    require_component(service, source_label, "service")
    require_component(target, source_label, "target")
    require_source_path(source, source_label)
    metadata = parse_metadata(uid, gid, mode, source_label, single_line)
    return f"{service}/{target}", metadata


def add_unique(destination, item, source_label):
    path, metadata = item
    if path in destination:
        raise InventoryError(f"ambiguous duplicate {source_label} path")
    destination[path] = metadata


def parse_copy_function(source, function_name):
    matches = list(
        re.finditer(
            rf"(?ms)^{re.escape(function_name)}\(\) \{{\n(.*?)^\}}$",
            source,
        )
    )
    if len(matches) != 1:
        raise InventoryError(f"missing or ambiguous {function_name} definition")
    inventory = {}
    for raw_line in matches[0].group(1).splitlines():
        line = raw_line.strip(" \t")
        if not line or line.startswith("#"):
            continue
        add_unique(inventory, parse_copy(line, function_name), function_name)
    return inventory


def parse_release_provisioner(path):
    source = open(path, encoding="utf-8").read()
    serving = parse_copy_function(source, "add_serving_copies")
    infrastructure = parse_copy_function(source, "add_infrastructure_copies")
    branches = list(re.finditer(r"(?ms)^  release\)\n(.*?)^    ;;$", source))
    if len(branches) != 1:
        raise InventoryError("missing or ambiguous release scope branch")

    inventory = {}
    calls = {"add_serving_copies": 0, "add_infrastructure_copies": 0}
    extras = 0
    for raw_line in branches[0].group(1).splitlines():
        line = raw_line.strip(" \t")
        if not line or line.startswith("#"):
            continue
        if line in calls:
            calls[line] += 1
            continue
        if line == "add_extra_source backup_encryption_key yes":
            extras += 1
            continue
        if line.startswith("add_copy "):
            add_unique(inventory, parse_copy(line, "release scope"), "release scope")
            continue
        raise InventoryError("unrecognized release scope mapping syntax")
    if calls != {"add_serving_copies": 1, "add_infrastructure_copies": 1} or extras != 1:
        raise InventoryError("ambiguous release scope function/source selection")
    for selected in (serving, infrastructure):
        for path, metadata in selected.items():
            add_unique(inventory, (path, metadata), "selected release")
    if not inventory:
        raise InventoryError("selected release inventory is empty")
    return inventory


def parse_validator_release(path):
    source = open(path, encoding="utf-8").read()
    matches = list(
        re.finditer(
            r"(?ms)^else\n  runtime_specs=\(\n(.*?)\n  \)\nfi\nfor spec in \"\$\{runtime_specs\[@\]\}\"; do",
            source,
        )
    )
    if len(matches) != 1:
        raise InventoryError("missing or ambiguous validator release runtime_specs branch")
    inventory = {}
    for raw_line in matches[0].group(1).splitlines():
        line = raw_line.strip(" \t")
        if not line or line.startswith("#"):
            continue
        for token in split_ascii_fields(line):
            add_unique(inventory, parse_validator_spec(token), "validator release")
    if not inventory:
        raise InventoryError("validator release inventory is empty")
    return inventory


def parse_validator_spec(token):
    fields = token.split(":")
    if len(fields) != 4:
        raise InventoryError("unrecognized validator release runtime_specs mapping")
    path_key, uid, gid, mode = fields
    path_components = path_key.split("/")
    if len(path_components) != 2:
        raise InventoryError("invalid validator release path literal")
    service, target = path_components
    require_component(service, "validator release", "service")
    require_component(target, "validator release", "target")
    metadata = parse_metadata(uid, gid, mode, "validator release")
    return path_key, metadata


def expect_inventory_error(label, operation):
    try:
        operation()
    except InventoryError:
        return
    raise InventoryError(f"parser regression expected rejection: {label}")


def run_parser_regressions():
    disallowed_tokens = [
        r"${service}",
        r"$service",
        r"$(id)",
        "`id`",
        '"literal"',
        "'literal'",
        r"literal\name",
        "*",
        "n?me",
        "[abc]",
        "{a,b}",
        "~user",
        "safe;id",
        "safe|id",
        "safe&",
        "<(id)",
        "safe>out",
    ]
    for token_index, token in enumerate(disallowed_tokens):
        for field_index in range(3):
            fields = ["service", "target", "source"]
            fields[field_index] = token
            line = "add_copy " + " ".join(fields + ["10003", "10003", "0440", "yes"])
            position = ("service", "target", "source")[field_index]
            expect_inventory_error(
                f"parse_copy {position} token #{token_index}",
                lambda line=line: parse_copy(line, "parser regression"),
            )
        for field_index in range(2):
            path_components = ["service", "target"]
            path_components[field_index] = token
            spec = f"{'/'.join(path_components)}:10003:10003:440"
            position = ("service", "target")[field_index]
            expect_inventory_error(
                f"parse_validator_spec {position} token #{token_index}",
                lambda spec=spec: parse_validator_spec(spec),
            )

    invalid_sources = [
        "/absolute",
        "../parent",
        "tls/../parent",
        "tls//file",
        "tls/",
        "./tls/file",
        "tls/./file",
    ]
    for source_index, source in enumerate(invalid_sources):
        line = f"add_copy service target {source} 10003 10003 0440 yes"
        expect_inventory_error(
            f"parse_copy relative source #{source_index}",
            lambda line=line: parse_copy(line, "parser regression"),
        )

    valid_copy = "add_copy service target source 10003 10003 0440 yes"
    expect_inventory_error(
        "parse_copy missing field",
        lambda: parse_copy("add_copy service target source 10003 10003 0440", "parser regression"),
    )
    expect_inventory_error(
        "parse_copy extra field",
        lambda: parse_copy(valid_copy + " extra", "parser regression"),
    )
    expect_inventory_error(
        "parse_copy non-ASCII field separator",
        lambda: parse_copy("add_copy service target\u00a0source 10003 10003 0440 yes", "parser regression"),
    )
    for field_name, value in (("UID", "${uid}"), ("GID", "$gid"), ("UID", "١"), ("GID", "١")):
        fields = ["service", "target", "source", "10003", "10003", "0440", "yes"]
        fields[3 if field_name == "UID" else 4] = value
        line = "add_copy " + " ".join(fields)
        expect_inventory_error(
            f"parse_copy {field_name} numeric token",
            lambda line=line: parse_copy(line, "parser regression"),
        )
    for mode in ("0o440", "888", "44", "04400"):
        line = f"add_copy service target source 10003 10003 {mode} yes"
        expect_inventory_error(
            "parse_copy unsupported mode",
            lambda line=line: parse_copy(line, "parser regression"),
        )
    for single_line in ("YES", "true", "1"):
        line = f"add_copy service target source 10003 10003 0440 {single_line}"
        expect_inventory_error(
            "parse_copy unsupported single_line spelling",
            lambda line=line: parse_copy(line, "parser regression"),
        )

    valid_spec = "service/target:10003:10003:440"
    expect_inventory_error(
        "parse_validator_spec missing field",
        lambda: parse_validator_spec("service/target:10003:10003"),
    )
    expect_inventory_error(
        "parse_validator_spec extra field",
        lambda: parse_validator_spec(valid_spec + ":extra"),
    )
    for field_name, value in (("UID", "${uid}"), ("GID", "$gid"), ("UID", "١"), ("GID", "١")):
        fields = ["service/target", "10003", "10003", "440"]
        fields[1 if field_name == "UID" else 2] = value
        spec = ":".join(fields)
        expect_inventory_error(
            f"parse_validator_spec {field_name} numeric token",
            lambda spec=spec: parse_validator_spec(spec),
        )
    for mode in ("0o440", "888", "44", "04400"):
        spec = f"service/target:10003:10003:{mode}"
        expect_inventory_error(
            "parse_validator_spec unsupported mode",
            lambda spec=spec: parse_validator_spec(spec),
        )

    positive_copies = (
        (
            "add_copy api_server target-1 cursor_secret 10001 10001 0440 yes",
            ("api_server/target-1", (10001, 10001, 0o440)),
        ),
        (
            "add_copy owner-beta_daily-admitter lagrange.crt tls/lagrange.crt 10003 10003 0440 no",
            ("owner-beta_daily-admitter/lagrange.crt", (10003, 10003, 0o440)),
        ),
        (
            "add_copy owner-beta_daily-admitter lagrange_key tls/lagrange.key 10003 10003 0400 yes",
            ("owner-beta_daily-admitter/lagrange_key", (10003, 10003, 0o400)),
        ),
    )
    for positive_index, (line, expected) in enumerate(positive_copies):
        if parse_copy(line, "parser regression") != expected:
            raise InventoryError(f"parse_copy positive regression #{positive_index} changed tuple")

    positive_specs = (
        (
            "owner-beta_daily-admitter/db_daily_input_admitter_password:10003:10003:440",
            ("owner-beta_daily-admitter/db_daily_input_admitter_password", (10003, 10003, 0o440)),
        ),
        ("api_server/cert.1:101:101:0440", ("api_server/cert.1", (101, 101, 0o440))),
    )
    for positive_index, (spec, expected) in enumerate(positive_specs):
        if parse_validator_spec(spec) != expected:
            raise InventoryError(f"parse_validator_spec positive regression #{positive_index} changed tuple")


run_parser_regressions()


def inventory_parity(provisioner, validator):
    return provisioner == validator


try:
    provisioner_release = parse_release_provisioner(sys.argv[1])
    validator_release = parse_validator_release(sys.argv[2])
    if not inventory_parity(provisioner_release, validator_release):
        raise InventoryError("selected release path/UID/GID/numeric mode inventories differ")

    protected_path = "db-role-bootstrap/db_daily_input_admitter_password"
    if protected_path not in provisioner_release:
        raise InventoryError("selected release omits the admitter bootstrap credential")
    expected_uid, expected_gid, expected_mode = provisioner_release[protected_path]

    without_bootstrap = dict(provisioner_release)
    del without_bootstrap[protected_path]
    wrong_uid = dict(provisioner_release)
    wrong_uid[protected_path] = (expected_uid + 1, expected_gid, expected_mode)
    wrong_mode = dict(provisioner_release)
    wrong_mode[protected_path] = (expected_uid, expected_gid, expected_mode ^ 0o040)
    for variant in (without_bootstrap, wrong_uid, wrong_mode):
        if inventory_parity(variant, validator_release):
            raise InventoryError("in-memory release inventory mutation was not rejected")
except (OSError, UnicodeError, InventoryError, IndexError) as error:
    print(f"secrets-runtime-static-check: release inventory parity failed: {error}", file=sys.stderr)
    raise SystemExit(1)
PY

[ "$(grep -Ec '^[[:space:]]+add_copy[[:space:]]+owner-beta-daily-admitter[[:space:]]+db_daily_input_admitter_password[[:space:]]+db_daily_input_admitter_password[[:space:]]+10003[[:space:]]+10003[[:space:]]+0440[[:space:]]+yes$' "$provision" || true)" -eq 1 ] \
  || die 'admitter runtime secret must be staged once as 10003:10003 mode 0440'
if grep -Eq '^[[:space:]]+add_copy[[:space:]]+(api-server|research-worker|recommendation-runner|candidate-runner|owner-beta-runner|owner-beta-daily-producer|nt-backtest-worker-[12]|paper-scheduler|research-range-raw)[[:space:]].*db_daily_input_admitter_password' "$provision"; then
  die 'admitter database credential must not be copied to an existing API, worker, producer, or raw service'
fi
grep -Fq 'add_extra_source backup_encryption_key yes' "$provision" \
  || die 'serving-prereqs/release backup source preflight is missing'
grep -Fq 'DB_DAILY_INPUT_ADMITTER_PASSWORD_FILE: /run/secrets/db_daily_input_admitter_password' "$compose" \
  || die 'Compose bootstrap must receive only the admitter password-file path'
grep -Fq 'bootstrap_db_daily_input_admitter_password:' "$compose" \
  || die 'Compose bootstrap secret source is missing'
grep -Fq 'file: ${LAGRANGE_RUNTIME_SECRET_DIR:-../secrets/runtime}/db-role-bootstrap/db_daily_input_admitter_password' "$compose" \
  || die 'Compose bootstrap secret source must resolve to the staged runtime copy'
serving_branch=$(awk '
  /^  serving-prereqs\)/ { in_scope=1; next }
  in_scope && /^  backfill\)/ { exit }
  in_scope { print }
' "$provision")
if grep -Eq 'kis_app_key|kis_app_secret' <<<"$serving_branch"; then
  die 'serving-prereqs must not add KIS runtime copies'
fi
recovery_branch=$(awk '
  /^  range-raw-recovery\)/ { in_scope=1; next }
  in_scope && /^  release\)/ { exit }
  in_scope { print }
' "$provision")
grep -Fq 'range-raw-recovery)' "$provision" \
  || die 'range-raw-recovery provisioner scope is missing'
if grep -Eq 'add_copy|kis_app_key|kis_app_secret' <<<"$recovery_branch"; then
  die 'range-raw-recovery must not install runtime or KIS secret copies'
fi
raw_branch=$(awk '
  /^  range-raw\)/ { in_scope=1; next }
  in_scope && /^  range-raw-recovery\)/ { exit }
  in_scope { print }
' "$provision")
if grep -Eq 'db_daily_input_admitter_password|db_(migration_owner|app|worker|audit|research|admin)_password|postgres_password' <<<"$raw_branch"; then
  die 'range-raw must remain free of database credentials'
fi
if grep -Eq '^  owner-beta-daily-admitter:' "$compose"; then
  die 'I0b must not add the deferred admitter runtime service'
fi

grep -Fq 'reject_dotdot()' "$provision" \
  || die 'provisioner must reject .. path aliases'
grep -Fq 'check_path "$source_dir" source-directory' "$provision" \
  || die 'provisioner must fence source directory ancestors'
grep -Fq 'check_path "$runtime_dir" runtime-directory' "$provision" \
  || die 'provisioner must fence runtime directory ancestors'
grep -Fq 'check_path "$input"' "$provision" \
  || die 'provisioner must fence source-file ancestors'
grep -Fq 'check_path "$output_dir"' "$provision" \
  || die 'provisioner must fence runtime service-directory ancestors'
grep -Fq 'check_path "$output"' "$provision" \
  || die 'provisioner must fence runtime secret-file ancestors'
grep -Fq 'must not traverse a symlink' "$provision" \
  || die 'provisioner must reject symlinked ancestors'
grep -Fq "stat -c '%a'" "$provision" \
  || die 'provisioner must validate source file modes before writes'
grep -Fq 'crypto_placeholder_pattern' "$provision" \
  || die 'provisioner crypto placeholder contract is missing'
grep -Fq "grep -Eq '^[0-9a-f]{64}$'" "$provision" \
  || die 'provisioner crypto lowercase-hex contract is missing'
grep -Fq 'crypto source secrets must be distinct' "$provision" \
  || die 'provisioner crypto distinctness contract is missing'
grep -Fq "\\r'" "$provision" \
  || die 'provisioner must reject CR-containing credential secrets'
grep -Fq 'numeric_chown()' "$provision" \
  || die 'provisioner must use an explicit numeric ownership helper'
grep -Fq 'chown --no-dereference -- "$uid:$gid"' "$provision" \
  || die 'provisioner must assign Docker-only UIDs with numeric chown'
if grep -Eq '^[[:space:]]*install([[:space:]]|$)' "$provision"; then
  die 'provisioner must not use install -o/-g for Docker-only numeric UIDs'
fi
grep -Fq 'mktemp -- "$output_dir/.lagrange-secret.' "$provision" \
  || die 'provisioner must stage runtime files before replacement'
grep -Fq 'mv -T -- "$staged" "$output"' "$provision" \
  || die 'provisioner must atomically rename staged runtime files'

# Exercise the path fence without requiring real root or any secret copy. A
# fake id(1) satisfies the early root guard; every fixture exits before mkdir/
# install, so this remains a no-root static test of symlink and `..` rejection.
fixture=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-secret-path-test.XXXXXX")
trap 'rm -rf -- "$fixture"' EXIT
fake_bin="$fixture/bin"
mkdir -p "$fake_bin"
printf '%s\n' '#!/usr/bin/env bash' 'if [ "$1" = -u ]; then echo 0; else exec /usr/bin/id "$@"; fi' >"$fake_bin/id"
chmod 0755 "$fake_bin/id"
mkdir -p "$fixture/source-real" "$fixture/runtime-real" "$fixture/source" "$fixture/runtime"
ln -s "$fixture/source-real" "$fixture/source-link"
ln -s "$fixture/runtime-real" "$fixture/runtime-link"

expect_path_rejection() {
  local label=$1 output=$2
  shift 2
  if PATH="$fake_bin:$PATH" "$@" >"$output" 2>&1; then
    die "provisioner accepted unsafe $label path"
  fi
  grep -Eq "must not (traverse a symlink|contain '\.\.')" "$output" \
    || die "provisioner did not report unsafe $label path"
}

expect_path_rejection source-ancestor "$fixture/source-ancestor.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source-link/child" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill
expect_path_rejection runtime-ancestor "$fixture/runtime-ancestor.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime-link/child" \
      bash "$provision" --scope backfill
expect_path_rejection dotdot-alias "$fixture/dotdot.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source/../source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

printf 'fixture-secret' >"$fixture/source/postgres_password.real"
ln -s "$fixture/source/postgres_password.real" "$fixture/source/postgres_password"
expect_path_rejection input-ancestor "$fixture/input.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

rm -f "$fixture/source/postgres_password"
printf 'fixture-secret' >"$fixture/source/postgres_password"
chmod 0600 "$fixture/source/postgres_password"
ln -s "$fixture/runtime-real" "$fixture/runtime/db-role-bootstrap"
expect_path_rejection output-directory "$fixture/output-dir.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

rm -f "$fixture/runtime/db-role-bootstrap"
mkdir -p "$fixture/runtime/db-role-bootstrap"
ln -s "$fixture/runtime-real/postgres_password" \
  "$fixture/runtime/db-role-bootstrap/postgres_password"
expect_path_rejection output-file "$fixture/output-file.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

# A serving-prereqs crypto source with 63 hex characters must fail before any
# runtime directory is created. The fake id(1) keeps this focused test rootless.
crypto_source="$fixture/crypto-source"
crypto_runtime="$fixture/crypto-runtime"
malformed_cursor=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde
mkdir -p "$crypto_source/tls"
for name in postgres_password db_migration_owner_password db_app_password \
  db_worker_password db_audit_password db_research_password db_admin_password \
  db_daily_input_admitter_password \
  session_secret csrf_secret cursor_secret auth0_client_secret backup_encryption_key; do
  value=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
  [ "$name" = cursor_secret ] && value=$malformed_cursor
  printf '%s' "$value" >"$crypto_source/$name"
  chmod 0600 "$crypto_source/$name"
done
printf '%s\n' fixture-certificate >"$crypto_source/tls/lagrange.crt"
printf '%s\n' fixture-private-key >"$crypto_source/tls/lagrange.key"
chmod 0600 "$crypto_source/tls/lagrange.crt" "$crypto_source/tls/lagrange.key"
if PATH="$fake_bin:$PATH" env \
   LAGRANGE_SECRET_SOURCE_DIR="$crypto_source" \
   LAGRANGE_RUNTIME_SECRET_DIR="$crypto_runtime" \
   bash "$provision" --scope serving-prereqs \
   >"$fixture/crypto-shape.out" 2>&1; then
  die 'provisioner accepted malformed cursor crypto source'
fi
grep -Fq 'crypto source cursor_secret must contain exactly 64 lowercase hex characters' \
  "$fixture/crypto-shape.out" \
  || die 'provisioner did not report malformed cursor shape'
[ ! -e "$crypto_runtime" ] || die 'malformed crypto source left partial runtime writes'
if grep -Fq "$malformed_cursor" "$fixture/crypto-shape.out"; then
  die 'provisioner leaked a crypto fixture value'
fi

# Exercise the numeric ownership path with fakeroot.  UID/GID 10001 is
# deliberately a Docker-only identity on production hosts, so no NSS user is
# required for this test.  fakeroot lets the root guard and stat assertions run
# without mutating host ownership; the provisioner still receives numeric
# chown arguments and the resulting metadata is checked for every UID class.
if command -v fakeroot >/dev/null 2>&1; then
  numeric_source="$fixture/numeric-source"
  numeric_runtime="$fixture/numeric-runtime"
  mkdir -p "$numeric_source/tls"
  for name in postgres_password db_migration_owner_password db_app_password \
    db_worker_password db_audit_password db_research_password db_admin_password \
    db_daily_input_admitter_password \
    auth0_client_secret; do
    printf 'fixture-%s' "$name" >"$numeric_source/$name"
    chmod 0600 "$numeric_source/$name"
  done
  printf '%064d' 1 >"$numeric_source/session_secret"
  printf '%064d' 2 >"$numeric_source/csrf_secret"
  printf '%064d' 3 >"$numeric_source/cursor_secret"
  printf '%064d' 4 >"$numeric_source/backup_encryption_key"
  chmod 0600 "$numeric_source/session_secret" "$numeric_source/csrf_secret" \
    "$numeric_source/cursor_secret" "$numeric_source/backup_encryption_key"
  printf '%s\n' fixture-certificate >"$numeric_source/tls/lagrange.crt"
  printf '%s\n' fixture-private-key >"$numeric_source/tls/lagrange.key"
  chmod 0600 "$numeric_source/tls/lagrange.crt" "$numeric_source/tls/lagrange.key"
  if ! PATH="$fake_bin:$PATH" fakeroot bash -c '
    set -e
    source_dir=$2
    runtime_dir=$3
    LAGRANGE_SECRET_SOURCE_DIR="$source_dir" \
      LAGRANGE_RUNTIME_SECRET_DIR="$runtime_dir" \
      bash "$1" --scope serving-prereqs >/dev/null
    LAGRANGE_SECRET_SOURCE_DIR="$source_dir" \
      LAGRANGE_RUNTIME_SECRET_DIR="$runtime_dir" \
      bash "$1" --scope serving-prereqs >/dev/null
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/api-server/session_secret")" = 10001:10001:440 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/reverse-proxy/lagrange_tls_cert")" = 101:101:440 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/db-role-bootstrap/postgres_password")" = 999:999:400 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/owner-beta-daily-admitter/db_daily_input_admitter_password")" = 10003:10003:440 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/owner-beta-daily-admitter")" = 10003:10003:750 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/db-role-bootstrap/db_daily_input_admitter_password")" = 999:999:400 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/api-server")" = 10001:10001:750 ]
  ' _ "$provision" "$numeric_source" "$numeric_runtime" \
    >"$fixture/numeric-ownership.out" 2>&1; then
    cat "$fixture/numeric-ownership.out" >&2
    die 'numeric ownership fixture did not converge to expected metadata'
  fi
else
  echo 'secrets-runtime-static-check: fakeroot unavailable; numeric stat fixture skipped' >&2
fi

echo 'SECRETS_RUNTIME_STATIC: PASS'
