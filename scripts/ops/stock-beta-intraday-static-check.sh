#!/usr/bin/env bash
# Provider-free static check for the WP6-A session-window artifact/schema.
# This is intentionally not a general JSON-Schema validator: it checks the
# exact checked-in contract and records which runtime semantics remain outside
# ordinary offline JSON-Schema validation.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
artifact=$root/configs/market-hours/krx-intraday-session-windows-v1.json
schema=$root/configs/market-hours/krx-intraday-session-windows-v1.schema.json
env_example=$root/deploy/compose/.env.example
provision=$root/scripts/ops/provision-linux.sh
validator=$root/scripts/ops/validate-production-config.sh
self_test=$root/scripts/ops/stock-beta-intraday-self-test.sh

die() {
  echo "STOCK_BETA_INTRADAY_STATIC: $*" >&2
  exit 1
}

command -v python3 >/dev/null 2>&1 || die 'PYTHON3_MISSING'
for path in "$artifact" "$schema" "$env_example" "$provision" "$validator" "$self_test"; do
  [ -f "$path" ] || die 'REQUIRED_FILE_MISSING'
  [ ! -L "$path" ] || die 'REQUIRED_FILE_SYMLINK'
done

for script in "$provision" "$validator" "$self_test"; do
  bash -n "$script" || die "SHELL_SYNTAX_INVALID:$script"
done

for expected in \
  'OWNER_INTRADAY_QUOTES_MODE=off' \
  'KIS_READ_COORDINATION_MODE=legacy' \
  'KIS_READ_CREDENTIAL_GENERATION=' \
  'LAGRANGE_RUNTIME_STATE_DIR=' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SHA256='; do
  [ "$(grep -Fxc -- "$expected" "$env_example")" -eq 1 ] ||
    die "ENV_DEFAULT_MISSING:$expected"
done
grep -Fq 'explicit absolute host root' "$env_example" ||
  die 'ENV_HOST_ROOT_COMMENT_MISSING'
grep -Fq 'fixed kis-read-coordination leaf' "$env_example" ||
  die 'ENV_COORDINATION_LEAF_COMMENT_MISSING'
grep -Fq '/run/lagrange/kis-read-coordination' "$env_example" ||
  die 'ENV_CONTAINER_PATH_COMMENT_MISSING'
grep -Fq 'separately approved' "$env_example" ||
  die 'ENV_ACTIVATION_COMMENT_MISSING'
if grep -Eq 'KIS_APP_KEY=|KIS_APP_SECRET=|CANO|ACNT_PRDT_CD|KIS_ACCOUNT_REF|owner_only' \
  "$env_example"; then
  die 'ENV_EXAMPLE_CONTAINS_FORBIDDEN_OR_ENABLED_VALUE'
fi

for expected in \
  'runtime_state_root=${LAGRANGE_RUNTIME_STATE_DIR-}' \
  'coordination_leaf_name=kis-read-coordination' \
  'coordination_mode=${KIS_READ_COORDINATION_MODE-legacy}' \
  'safe_runtime_state_path' \
  'reject_runtime_state_overlap' \
  'shared_required coordination requires explicit LAGRANGE_RUNTIME_STATE_DIR' \
  'check_coordination_tree' \
  'ensure_coordination_dir' \
  'check_coordination_dir "$runtime_state_root" 0 "$worker_gid" 750' \
  'check_coordination_dir "$coordination_leaf" "$worker_uid" "$worker_gid" 700'; do
  grep -Fq -- "$expected" "$provision" || die "PROVISION_HOOK_MISSING:$expected"
done
if grep -Eq 'runtime_state_root=\$\{LAGRANGE_RUNTIME_STATE_DIR:-' "$provision"; then
  die 'PROVISION_RUNTIME_STATE_FALLBACK_PRESENT'
fi

for expected in \
  'guard_new_config_shell_overrides' \
  'reject_new_config_file_aliases' \
  'OWNER_INTRADAY_QUOTES_MODE_FILE' \
  'KIS_READ_COORDINATION_MODE_FILE' \
  'KIS_READ_CREDENTIAL_GENERATION_FILE' \
  'LAGRANGE_RUNTIME_STATE_DIR_FILE' \
  'OWNER_INTRADAY_SESSION_WINDOWS_SHA256_FILE' \
  'intraday_quotes_mode=off' \
  'coordination_mode=legacy' \
  '18446744073709551615' \
  'generation_length' \
  'coordination_path_safe' \
  'backfill|range-raw|release' \
  'state-v1.json requires coordination.lock' \
  "'0:10001:750'" \
  "'10001:10001:700'" \
  "'10001:10001:600:1'" \
  'overlaps a protected tree'; do
  grep -Fq -- "$expected" "$validator" || die "VALIDATOR_HOOK_MISSING:$expected"
done
if grep -Eiq 'cat[[:space:]].*(coordination|state-v1)|sha(256|sum).*state-v1' "$validator"; then
  die 'VALIDATOR_READS_COORDINATION_CONTENT'
fi

for expected in \
  'LAGRANGE_B1_ROOT_FIXTURE_CHILD' \
  'fakeroot' \
  'validate-production-config.sh' \
  'provision-linux.sh' \
  'coordination.lock' \
  'sentinel' \
  'idempotent'; do
  grep -Fq -- "$expected" "$self_test" || die "SELF_TEST_HOOK_MISSING:$expected"
done

python3 - "$artifact" "$schema" <<'PY'
import json
import pathlib
import sys


class ContractViolation(Exception):
    pass


class DuplicateKey(Exception):
    pass


class InvalidConstant(Exception):
    pass


ARTIFACT_KEYS = ("schema_version", "exchange", "timezone", "entries")
ENTRY_KEYS = (
    "date",
    "disposition",
    "open_local",
    "close_local",
    "evidence_url",
    "evidence_retrieved_at",
    "evidence_sha256",
)
DISPOSITIONS = ("REGULAR", "SPECIAL", "CLOSED")
DATE_PATTERN = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"
TIME_PATTERN = r"^(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]$"
URL_PATTERN = r"^https://global\.krx\.co\.kr/[^\s\x00-\x1f\x7f]+(?![\s\S])"
UTC_DATETIME_PATTERN = (
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T"
    r"(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]"
    r"(?:\.[0-9]{1,9})?Z$"
)
SHA256_PATTERN = r"^sha256:[0-9a-f]{64}$"
SCHEMA_URI = "https://json-schema.org/draft/2020-12/schema"
SCHEMA_ID = (
    "https://lagrange.local/schemas/market-hours/"
    "krx-intraday-session-windows-v1.schema.json"
)


def violation(code):
    raise ContractViolation(code)


def reject_constant(_value):
    raise InvalidConstant


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKey
        result[key] = value
    return result


def load_json(path, label):
    try:
        raw = pathlib.Path(path).read_bytes()
        text = raw.decode("utf-8")
        return json.loads(
            text,
            object_pairs_hook=unique_object,
            parse_constant=reject_constant,
        )
    except DuplicateKey:
        violation(f"{label}_DUPLICATE_KEY")
    except (InvalidConstant, UnicodeDecodeError, json.JSONDecodeError, OSError):
        violation(f"{label}_JSON_INVALID")


def exact_keys(value, expected, code):
    if type(value) is not dict or set(value) != set(expected) or len(value) != len(expected):
        violation(code)


def exact_strings(value, expected, code):
    if (
        type(value) is not list
        or len(value) != len(expected)
        or any(type(item) is not str for item in value)
    ):
        violation(code)
    if len(set(value)) != len(value) or set(value) != set(expected):
        violation(code)


def exact_string_list(value, expected, code):
    if type(value) is not list or value != list(expected):
        violation(code)


def exact_int(value, expected, code):
    if type(value) is not int or value != expected:
        violation(code)


def exact_string(value, expected, code):
    if type(value) is not str or value != expected:
        violation(code)


def exact_bool(value, expected, code):
    if type(value) is not bool or value is not expected:
        violation(code)


def check_artifact(document):
    exact_keys(document, ARTIFACT_KEYS, "ARTIFACT_TOP_LEVEL_KEYS_INVALID")
    exact_int(document["schema_version"], 1, "ARTIFACT_SCHEMA_VERSION_INVALID")
    exact_string(document["exchange"], "KRX", "ARTIFACT_EXCHANGE_INVALID")
    exact_string(document["timezone"], "Asia/Seoul", "ARTIFACT_TIMEZONE_INVALID")
    if type(document["entries"]) is not list:
        violation("ARTIFACT_ENTRIES_TYPE_INVALID")
    if document["entries"]:
        violation("ARTIFACT_ENTRIES_NOT_EMPTY")


def check_schema(document):
    exact_keys(
        document,
        (
            "$schema",
            "$id",
            "title",
            "description",
            "type",
            "additionalProperties",
            "required",
            "properties",
            "$defs",
        ),
        "SCHEMA_TOP_LEVEL_KEYS_INVALID",
    )
    exact_string(document["$schema"], SCHEMA_URI, "SCHEMA_DRAFT_INVALID")
    exact_string(document["$id"], SCHEMA_ID, "SCHEMA_ID_INVALID")
    if type(document["title"]) is not str or not document["title"]:
        violation("SCHEMA_TITLE_INVALID")
    if type(document["description"]) is not str:
        violation("SCHEMA_DESCRIPTION_INVALID")
    for marker in (
        "positive SPECIAL interval",
        "sorted unique dates",
        "actual whole-file SHA-256",
        "same KST civil date",
        "not in the future",
        "does not claim",
    ):
        if marker not in document["description"]:
            violation("SCHEMA_RUNTIME_LIMITS_UNDOCUMENTED")
    exact_string(document["type"], "object", "SCHEMA_ROOT_TYPE_INVALID")
    exact_bool(document["additionalProperties"], False, "SCHEMA_ROOT_NOT_CLOSED")
    exact_strings(document["required"], ARTIFACT_KEYS, "SCHEMA_REQUIRED_KEYS_INVALID")

    properties = document["properties"]
    exact_keys(properties, ARTIFACT_KEYS, "SCHEMA_ROOT_PROPERTIES_INVALID")
    exact_keys(properties["schema_version"], ("const",), "SCHEMA_VERSION_RULE_INVALID")
    exact_int(properties["schema_version"]["const"], 1, "SCHEMA_VERSION_RULE_INVALID")
    exact_keys(properties["exchange"], ("const",), "SCHEMA_EXCHANGE_RULE_INVALID")
    exact_string(properties["exchange"]["const"], "KRX", "SCHEMA_EXCHANGE_RULE_INVALID")
    exact_keys(properties["timezone"], ("const",), "SCHEMA_TIMEZONE_RULE_INVALID")
    exact_string(
        properties["timezone"]["const"], "Asia/Seoul", "SCHEMA_TIMEZONE_RULE_INVALID"
    )

    entries_rule = properties["entries"]
    exact_keys(
        entries_rule,
        ("type", "description", "items"),
        "SCHEMA_ENTRIES_RULE_INVALID",
    )
    exact_string(entries_rule["type"], "array", "SCHEMA_ENTRIES_RULE_INVALID")
    if type(entries_rule["description"]) is not str or not entries_rule["description"]:
        violation("SCHEMA_ENTRIES_DESCRIPTION_INVALID")
    exact_keys(entries_rule["items"], ("$ref",), "SCHEMA_ENTRIES_ITEMS_INVALID")
    exact_string(entries_rule["items"]["$ref"], "#/$defs/entry", "SCHEMA_ENTRIES_ITEMS_INVALID")

    defs = document["$defs"]
    exact_keys(defs, ("entry",), "SCHEMA_DEFS_INVALID")
    entry = defs["entry"]
    exact_keys(
        entry,
        ("type", "description", "additionalProperties", "required", "properties", "allOf"),
        "SCHEMA_ENTRY_RULE_INVALID",
    )
    exact_string(entry["type"], "object", "SCHEMA_ENTRY_TYPE_INVALID")
    if type(entry["description"]) is not str or not entry["description"]:
        violation("SCHEMA_ENTRY_DESCRIPTION_INVALID")
    exact_bool(entry["additionalProperties"], False, "SCHEMA_ENTRY_NOT_CLOSED")
    exact_strings(entry["required"], ENTRY_KEYS, "SCHEMA_ENTRY_REQUIRED_KEYS_INVALID")

    entry_properties = entry["properties"]
    exact_keys(entry_properties, ENTRY_KEYS, "SCHEMA_ENTRY_PROPERTIES_INVALID")
    check_field(
        entry_properties["date"],
        ("type", "format", "pattern"),
        "SCHEMA_DATE_RULE_INVALID",
    )
    exact_string(entry_properties["date"]["type"], "string", "SCHEMA_DATE_RULE_INVALID")
    exact_string(entry_properties["date"]["format"], "date", "SCHEMA_DATE_RULE_INVALID")
    exact_string(entry_properties["date"]["pattern"], DATE_PATTERN, "SCHEMA_DATE_RULE_INVALID")

    check_field(entry_properties["disposition"], ("enum",), "SCHEMA_DISPOSITION_RULE_INVALID")
    exact_string_list(
        entry_properties["disposition"]["enum"], DISPOSITIONS, "SCHEMA_DISPOSITION_RULE_INVALID"
    )

    for field in ("open_local", "close_local"):
        check_field(
            entry_properties[field],
            ("type", "pattern"),
            "SCHEMA_LOCAL_TIME_RULE_INVALID",
        )
        exact_string_list(
            entry_properties[field]["type"], ("string", "null"), "SCHEMA_LOCAL_TIME_RULE_INVALID"
        )
        exact_string(
            entry_properties[field]["pattern"], TIME_PATTERN, "SCHEMA_LOCAL_TIME_RULE_INVALID"
        )

    check_field(
        entry_properties["evidence_url"],
        ("type", "pattern"),
        "SCHEMA_EVIDENCE_URL_RULE_INVALID",
    )
    exact_string(entry_properties["evidence_url"]["type"], "string", "SCHEMA_EVIDENCE_URL_RULE_INVALID")
    exact_string(
        entry_properties["evidence_url"]["pattern"], URL_PATTERN, "SCHEMA_EVIDENCE_URL_RULE_INVALID"
    )

    check_field(
        entry_properties["evidence_retrieved_at"],
        ("type", "format", "pattern"),
        "SCHEMA_EVIDENCE_DATETIME_RULE_INVALID",
    )
    exact_string(
        entry_properties["evidence_retrieved_at"]["type"],
        "string",
        "SCHEMA_EVIDENCE_DATETIME_RULE_INVALID",
    )
    exact_string(
        entry_properties["evidence_retrieved_at"]["format"],
        "date-time",
        "SCHEMA_EVIDENCE_DATETIME_RULE_INVALID",
    )
    exact_string(
        entry_properties["evidence_retrieved_at"]["pattern"],
        UTC_DATETIME_PATTERN,
        "SCHEMA_EVIDENCE_DATETIME_RULE_INVALID",
    )

    check_field(
        entry_properties["evidence_sha256"],
        ("type", "pattern"),
        "SCHEMA_EVIDENCE_HASH_RULE_INVALID",
    )
    exact_string(
        entry_properties["evidence_sha256"]["type"],
        "string",
        "SCHEMA_EVIDENCE_HASH_RULE_INVALID",
    )
    exact_string(
        entry_properties["evidence_sha256"]["pattern"],
        SHA256_PATTERN,
        "SCHEMA_EVIDENCE_HASH_RULE_INVALID",
    )

    check_conditionals(entry["allOf"], entry_properties)


def check_field(value, expected_keys, code):
    exact_keys(value, expected_keys, code)


def check_conditionals(all_of, entry_properties):
    if type(all_of) is not list or len(all_of) != 3:
        violation("SCHEMA_DISPOSITION_CONDITIONALS_INVALID")

    seen = set()
    for clause in all_of:
        exact_keys(clause, ("if", "then"), "SCHEMA_DISPOSITION_CONDITIONALS_INVALID")
        condition = clause["if"]
        exact_keys(condition, ("properties", "required"), "SCHEMA_DISPOSITION_CONDITIONALS_INVALID")
        exact_string_list(
            condition["required"], ("disposition",), "SCHEMA_DISPOSITION_CONDITIONALS_INVALID"
        )
        exact_keys(
            condition["properties"], ("disposition",), "SCHEMA_DISPOSITION_CONDITIONALS_INVALID"
        )
        disposition_rule = condition["properties"]["disposition"]
        exact_keys(disposition_rule, ("const",), "SCHEMA_DISPOSITION_CONDITIONALS_INVALID")
        disposition = disposition_rule["const"]
        if disposition not in DISPOSITIONS or disposition in seen:
            violation("SCHEMA_DISPOSITION_CONDITIONALS_INVALID")
        seen.add(disposition)

        then = clause["then"]
        exact_keys(then, ("properties",), "SCHEMA_DISPOSITION_CONDITIONALS_INVALID")
        then_properties = then["properties"]
        exact_keys(
            then_properties,
            ("open_local", "close_local"),
            "SCHEMA_DISPOSITION_CONDITIONALS_INVALID",
        )
        if disposition == "CLOSED":
            for field in ("open_local", "close_local"):
                exact_keys(
                    then_properties[field], ("const",), "SCHEMA_CLOSED_CONSTRAINT_INVALID"
                )
                if then_properties[field]["const"] is not None:
                    violation("SCHEMA_CLOSED_CONSTRAINT_INVALID")
        elif disposition == "REGULAR":
            expected = {"open_local": "09:00:00", "close_local": "15:30:00"}
            for field, value in expected.items():
                exact_keys(
                    then_properties[field], ("const",), "SCHEMA_REGULAR_CONSTRAINT_INVALID"
                )
                exact_string(
                    then_properties[field]["const"], value, "SCHEMA_REGULAR_CONSTRAINT_INVALID"
                )
        else:
            for field in ("open_local", "close_local"):
                exact_keys(
                    then_properties[field], ("type", "pattern"), "SCHEMA_SPECIAL_CONSTRAINT_INVALID"
                )
                exact_string(
                    then_properties[field]["type"], "string", "SCHEMA_SPECIAL_CONSTRAINT_INVALID"
                )
                exact_string(
                    then_properties[field]["pattern"], TIME_PATTERN, "SCHEMA_SPECIAL_CONSTRAINT_INVALID"
                )

    if seen != set(DISPOSITIONS):
        violation("SCHEMA_DISPOSITION_CONDITIONALS_INVALID")


def main():
    if len(sys.argv) != 3:
        violation("ARGUMENTS_INVALID")
    artifact = load_json(sys.argv[1], "ARTIFACT")
    schema = load_json(sys.argv[2], "SCHEMA")
    check_artifact(artifact)
    check_schema(schema)


try:
    main()
except ContractViolation as error:
    print(f"STOCK_BETA_INTRADAY_STATIC: {error}", file=sys.stderr)
    sys.exit(1)
except (KeyError, IndexError, TypeError, ValueError):
    print("STOCK_BETA_INTRADAY_STATIC: CONTRACT_UNCHECKABLE", file=sys.stderr)
    sys.exit(1)
PY

echo 'STOCK_BETA_INTRADAY_STATIC: PASS artifact=empty schema=closed conditionals=checked runtime_semantics=not_claimed'
