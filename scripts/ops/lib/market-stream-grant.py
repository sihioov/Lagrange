#!/usr/bin/python3
"""Fail-closed installer for one coordinator-reviewed market-stream grant."""

from __future__ import annotations

import datetime as dt
import hashlib
import hmac
import json
import os
import pathlib
import re
import signal
import stat
import subprocess
import sys
import tempfile
import unicodedata
import uuid
from dataclasses import dataclass
from typing import Any, Callable, Mapping, Sequence

MAX_APPROVAL_BYTES = 16 * 1024
MAX_SQL_BYTES = 64 * 1024
EXPECTED_SCOPE = "owner-market-stream-grant"
EXPECTED_TR_ID = "H0STCNT0"
EXPECTED_WIRE_VERSION = "kis-h0stcnt0-20260914-v1"
EXPECTED_IDENTITY_SHA256 = "0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79"
DOCKER = "/usr/bin/docker"
DOCKER_HOST = "unix:///var/run/docker.sock"
COMPOSE_PROJECT = "lagrange-station"
CONTAINER_LABEL = "io.lagrange.market-stream.grant-operation"
CONTAINER_TIMEOUT_SECONDS = 90
DOCKER_ACTION_TIMEOUT_SECONDS = 10
CONTAINER_PSQL_SCRIPT = (
    'PGPASSWORD=$(cat "$DB_PASSWORD_FILE") || exit 1; export PGPASSWORD; '
    "export PGCONNECT_TIMEOUT=5; "
    'exec psql -X --no-password -v ON_ERROR_STOP=1 -P pager=off -qAt '
    '-h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d "$DB_NAME" "$@"'
)
APPROVAL_KEYS = frozenset(
    {
        "schema_version",
        "scope",
        "grant_id",
        "grant_revision",
        "credential_slot_id",
        "credential_generation",
        "owner_user_id",
        "entitlement_id",
        "entitlement_reference",
        "entitlement_document_sha256",
        "tr_id",
        "wire_version",
        "network_contract_sha256",
        "identity_list_sha256",
        "effective_from",
        "effective_until",
        "activation_commit",
    }
)
UUID_FIELDS = (
    "grant_id",
    "grant_revision",
    "credential_slot_id",
    "owner_user_id",
    "entitlement_id",
)
DIGEST_FIELDS = ("entitlement_document_sha256", "network_contract_sha256")
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
COMMIT40 = re.compile(r"[0-9a-f]{40}\Z")
GENERATION = re.compile(r"[1-9][0-9]{0,19}\Z")
ISO_DATE = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}\Z")
U64_MAX = 18446744073709551615
SUCCESS = frozenset({"PLAN", "APPLIED", "ALREADY_APPLIED", "REVOKED", "ALREADY_REVOKED"})
FAILURES = frozenset(
    {
        "INVALID_INPUT",
        "APPROVAL_REJECTED",
        "INSTALLED_CONTEXT_REJECTED",
        "IMAGE_REJECTED",
        "DB_OUTCOME_UNKNOWN",
        "CLEANUP_FAILED",
        "INTERNAL_ERROR",
    }
)


class GrantFailure(Exception):
    """Carries only one finite, nonsecret status code."""

    def __init__(self, code: str):
        self.code = code if code in FAILURES else "INTERNAL_ERROR"
        super().__init__(self.code)


@dataclass(frozen=True)
class Request:
    mode: str
    expected_commit: str | None = None
    approval_path: str | None = None
    approval_sha256: str | None = None
    grant_id: str | None = None


@dataclass(frozen=True)
class InstalledContext:
    root: pathlib.Path
    release_dir: pathlib.Path
    commit: str
    wrapper: pathlib.Path
    helper: pathlib.Path
    sql_file: pathlib.Path
    compose_file: pathlib.Path
    env_file: pathlib.Path
    manifest_file: pathlib.Path
    slot_id: str
    credential_generation: str
    configured_grant_id: str
    contract_sha256: str
    db_migrate_image_id: str
    db_migrate_revision: str


def _fail(code: str) -> None:
    raise GrantFailure(code)


def valid_commit(value: object) -> bool:
    return type(value) is str and COMMIT40.fullmatch(value) is not None and value != "0" * 40


def valid_sha256(value: object) -> bool:
    return type(value) is str and HEX64.fullmatch(value) is not None


def valid_uuid(value: object) -> bool:
    if type(value) is not str:
        return False
    try:
        parsed = uuid.UUID(value)
    except (ValueError, AttributeError, TypeError):
        return False
    return parsed.int != 0 and str(parsed) == value


def _reject_json_constant(_: str) -> None:
    _fail("APPROVAL_REJECTED")


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            _fail("APPROVAL_REJECTED")
        value[key] = item
    return value


def _canonical_json(payload: Mapping[str, Any]) -> bytes:
    try:
        return (
            json.dumps(
                payload,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
                allow_nan=False,
            ).encode("utf-8")
            + b"\n"
        )
    except (TypeError, ValueError, UnicodeError):
        _fail("APPROVAL_REJECTED")


def validate_approval_payload(payload: object) -> dict[str, Any]:
    if type(payload) is not dict or set(payload) != APPROVAL_KEYS:
        _fail("APPROVAL_REJECTED")
    if type(payload["schema_version"]) is not int or payload["schema_version"] != 1:
        _fail("APPROVAL_REJECTED")
    if type(payload["scope"]) is not str or payload["scope"] != EXPECTED_SCOPE:
        _fail("APPROVAL_REJECTED")
    for key in APPROVAL_KEYS - {"schema_version"}:
        if type(payload[key]) is not str:
            _fail("APPROVAL_REJECTED")
    for key in UUID_FIELDS:
        if not valid_uuid(payload[key]):
            _fail("APPROVAL_REJECTED")
    generation = payload["credential_generation"]
    if (
        GENERATION.fullmatch(generation) is None
        or int(generation) > U64_MAX
    ):
        _fail("APPROVAL_REJECTED")
    if payload["tr_id"] != EXPECTED_TR_ID or payload["wire_version"] != EXPECTED_WIRE_VERSION:
        _fail("APPROVAL_REJECTED")
    if payload["identity_list_sha256"] != EXPECTED_IDENTITY_SHA256:
        _fail("APPROVAL_REJECTED")
    for key in DIGEST_FIELDS:
        if not valid_sha256(payload[key]):
            _fail("APPROVAL_REJECTED")
    if not valid_commit(payload["activation_commit"]):
        _fail("APPROVAL_REJECTED")
    reference = payload["entitlement_reference"]
    if (
        not reference
        or len(reference) > 512
        or reference.strip() == ""
        or any(unicodedata.category(char) == "Cc" for char in reference)
    ):
        _fail("APPROVAL_REJECTED")
    start = payload["effective_from"]
    end = payload["effective_until"]
    if ISO_DATE.fullmatch(start) is None or ISO_DATE.fullmatch(end) is None:
        _fail("APPROVAL_REJECTED")
    try:
        start_date = dt.date.fromisoformat(start)
        end_date = dt.date.fromisoformat(end)
    except ValueError:
        _fail("APPROVAL_REJECTED")
    if start_date.isoformat() != start or end_date.isoformat() != end or end_date < start_date:
        _fail("APPROVAL_REJECTED")
    return payload


def parse_approval_bytes(raw: bytes, expected_sha256: str) -> dict[str, Any]:
    if type(raw) is not bytes or len(raw) > MAX_APPROVAL_BYTES or not valid_sha256(expected_sha256):
        _fail("APPROVAL_REJECTED")
    try:
        payload = json.loads(
            raw.decode("utf-8", errors="strict"),
            object_pairs_hook=_unique_object,
            parse_float=lambda _: _fail("APPROVAL_REJECTED"),
            parse_constant=_reject_json_constant,
        )
    except GrantFailure:
        raise
    except (UnicodeError, json.JSONDecodeError, ValueError, TypeError):
        _fail("APPROVAL_REJECTED")
    validated = validate_approval_payload(payload)
    if raw != _canonical_json(validated):
        _fail("APPROVAL_REJECTED")
    actual = hashlib.sha256(raw).hexdigest()
    if not hmac.compare_digest(actual, expected_sha256):
        _fail("APPROVAL_REJECTED")
    return validated


def trusted_directory_chain(directory: os.PathLike[str] | str) -> pathlib.Path:
    text = os.path.abspath(os.fspath(directory))
    if "\x00" in text:
        _fail("APPROVAL_REJECTED")
    path = pathlib.Path(text)
    current = pathlib.Path("/")
    try:
        for component in path.parts[1:]:
            current = current / component
            metadata = os.lstat(current)
            if (
                not stat.S_ISDIR(metadata.st_mode)
                or metadata.st_uid != 0
                or stat.S_IMODE(metadata.st_mode) & 0o022
            ):
                _fail("APPROVAL_REJECTED")
    except (OSError, GrantFailure):
        _fail("APPROVAL_REJECTED")
    return path


def _trusted_release_file(
    path: os.PathLike[str] | str,
    *,
    exact_mode: int | None = None,
    max_bytes: int | None = None,
) -> os.stat_result:
    file_path = pathlib.Path(path)
    try:
        trusted_directory_chain(file_path.parent)
        metadata = os.lstat(file_path)
    except (OSError, GrantFailure):
        _fail("INSTALLED_CONTEXT_REJECTED")
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != 0
        or metadata.st_gid != 0
        or stat.S_IMODE(metadata.st_mode) & 0o022
        or (exact_mode is not None and stat.S_IMODE(metadata.st_mode) != exact_mode)
        or (max_bytes is not None and metadata.st_size > max_bytes)
    ):
        _fail("INSTALLED_CONTEXT_REJECTED")
    return metadata


def approval_file_stat_is_trusted(metadata: os.stat_result, max_bytes: int = MAX_APPROVAL_BYTES) -> bool:
    return (
        stat.S_ISREG(metadata.st_mode)
        and metadata.st_uid == 0
        and metadata.st_gid == 0
        and stat.S_IMODE(metadata.st_mode) == 0o600
        and metadata.st_nlink == 1
        and 0 <= metadata.st_size <= max_bytes
    )


def installed_current_target_matches(target: str, commit: str) -> bool:
    return valid_commit(commit) and target == "releases/" + commit


def context_snapshot_is_valid(
    snapshot: Sequence[str], expected_commit: str, *, require_binding: bool = True
) -> bool:
    if (
        len(snapshot) != 7
        or not valid_commit(expected_commit)
        or any(type(value) is not str for value in snapshot)
    ):
        return False
    parsed_commit, slot_id, generation, grant_id, contract_sha256, image_id, image_revision = snapshot
    release_matches = (
        parsed_commit == expected_commit
        and re.fullmatch(r"sha256:[0-9a-f]{64}", image_id) is not None
        and image_revision == expected_commit
    )
    # Revocation addresses an explicit grant, including from an installed off
    # release without WS settings. It still requires the exact installed image.
    return release_matches and (
        not require_binding or (
            valid_uuid(slot_id)
            and GENERATION.fullmatch(generation) is not None
            and int(generation) <= U64_MAX
            and valid_uuid(grant_id)
            and valid_sha256(contract_sha256)
        )
    )


def read_trusted_approval_file(
    path_text: str,
    *,
    open_fn: Callable[..., int] | None = None,
    fstat_fn: Callable[[int], os.stat_result] | None = None,
    lstat_fn: Callable[[os.PathLike[str] | str], os.stat_result] | None = None,
    ancestor_guard: Callable[[os.PathLike[str] | str], pathlib.Path] | None = None,
    metadata_guard: Callable[[os.stat_result], bool] | None = None,
) -> bytes:
    if type(path_text) is not str or not path_text or "\x00" in path_text:
        _fail("APPROVAL_REJECTED")
    open_fn = os.open if open_fn is None else open_fn
    fstat_fn = os.fstat if fstat_fn is None else fstat_fn
    lstat_fn = os.lstat if lstat_fn is None else lstat_fn
    ancestor_guard = trusted_directory_chain if ancestor_guard is None else ancestor_guard
    metadata_guard = approval_file_stat_is_trusted if metadata_guard is None else metadata_guard
    path = pathlib.Path(os.path.abspath(path_text))
    try:
        ancestor_guard(path.parent)
        before = lstat_fn(path)
    except (OSError, GrantFailure):
        _fail("APPROVAL_REJECTED")
    if not metadata_guard(before) or not hasattr(os, "O_NOFOLLOW"):
        _fail("APPROVAL_REJECTED")
    try:
        descriptor = open_fn(
            path,
            os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_CLOEXEC", 0) | os.O_NOFOLLOW,
        )
    except (OSError, TypeError):
        _fail("APPROVAL_REJECTED")
    try:
        after = fstat_fn(descriptor)
        if (
            not metadata_guard(after)
            or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino)
            or after.st_size > MAX_APPROVAL_BYTES
        ):
            _fail("APPROVAL_REJECTED")
        chunks: list[bytes] = []
        total = 0
        while True:
            block = os.read(descriptor, min(4096, MAX_APPROVAL_BYTES + 1 - total))
            if not block:
                break
            chunks.append(block)
            total += len(block)
            if total > MAX_APPROVAL_BYTES:
                _fail("APPROVAL_REJECTED")
        if total != after.st_size:
            _fail("APPROVAL_REJECTED")
        return b"".join(chunks)
    except GrantFailure:
        raise
    except OSError:
        _fail("APPROVAL_REJECTED")
    finally:
        os.close(descriptor)


def parse_cli(argv: Sequence[str]) -> Request:
    args = list(argv)
    if not args or args == ["--plan"]:
        return Request("plan")
    if args[0] != "--apply" or "--plan" in args:
        _fail("INVALID_INPUT")
    values: dict[str, str] = {}
    flags: set[str] = set()
    index = 1
    while index < len(args):
        option = args[index]
        if option == "--revoke":
            if option in flags:
                _fail("INVALID_INPUT")
            flags.add(option)
            index += 1
        elif option in {
            "--expected-commit",
            "--approval-input",
            "--approval-sha256",
            "--grant-id",
        }:
            if option in values or index + 1 >= len(args):
                _fail("INVALID_INPUT")
            values[option] = args[index + 1]
            index += 2
        else:
            _fail("INVALID_INPUT")
    commit = values.get("--expected-commit")
    if not valid_commit(commit):
        _fail("INVALID_INPUT")
    if "--revoke" in flags:
        if set(values) != {"--expected-commit", "--grant-id"} or not valid_uuid(values["--grant-id"]):
            _fail("INVALID_INPUT")
        return Request("revoke", commit, grant_id=values["--grant-id"])
    if set(values) != {"--expected-commit", "--approval-input", "--approval-sha256"}:
        _fail("INVALID_INPUT")
    if not values["--approval-input"] or not valid_sha256(values["--approval-sha256"]):
        _fail("INVALID_INPUT")
    return Request(
        "install",
        commit,
        approval_path=values["--approval-input"],
        approval_sha256=values["--approval-sha256"],
    )


def inherited_overrides_are_absent(
    expected_commit: str, environment: Mapping[str, str] | None = None
) -> bool:
    environment = os.environ if environment is None else environment
    for key, value in environment.items():
        if key.startswith(("COMPOSE_", "DOCKER_")):
            return False
        if key in {
            "LAGRANGE_COMPOSE_FILE",
            "LAGRANGE_ENV_FILE",
            "LAGRANGE_RELEASE_ROOT",
            "POSTGRES_DB",
            "RANGE_RAW_BATCH_ID",
        }:
            return False
        if key == "LAGRANGE_CODE_COMMIT" and value != expected_commit:
            return False
    return True


def _shared_parser_snapshot(
    *,
    dotenv_library: pathlib.Path,
    manifest_library: pathlib.Path,
    env_file: pathlib.Path,
    manifest_file: pathlib.Path,
    expected_commit: str,
    run_fn: Callable[..., subprocess.CompletedProcess[bytes]] | None = None,
) -> tuple[str, str, str, str, str, str, str]:
    run_fn = subprocess.run if run_fn is None else run_fn
    fixed_bash = r"""
set -euo pipefail
source "$1"
source "$2"
dotenv_load "$3"
dotenv_validate_shell_overrides
commit=$(dotenv_get LAGRANGE_CODE_COMMIT)
[ "$commit" = "$5" ]
release_image_manifest_load "$4" "$commit"
printf '%s\n' \
  "$commit" \
  "$(dotenv_get KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID)" \
  "$(dotenv_get KIS_READ_CREDENTIAL_GENERATION)" \
  "$(dotenv_get KIS_MARKET_STREAM_GRANT_ID)" \
  "$(dotenv_get KIS_MARKET_STREAM_CONTRACT_SHA256)" \
  "${RELEASE_IMAGE_MANIFEST_IDS[db-migrate]}" \
  "${RELEASE_IMAGE_MANIFEST_REVISIONS[db-migrate]}"
"""
    argv = [
        "/usr/bin/env",
        "-i",
        "PATH=/usr/bin:/bin",
        "LANG=C",
        "LC_ALL=C",
        "HOME=/nonexistent",
        "LAGRANGE_CODE_COMMIT=" + expected_commit,
        "RANGE_RAW_BATCH_ID=compose-config-disabled",
        "/usr/bin/bash",
        "--noprofile",
        "--norc",
        "-c",
        fixed_bash,
        "market-stream-grant-context",
        str(dotenv_library),
        str(manifest_library),
        str(env_file),
        str(manifest_file),
        expected_commit,
    ]
    try:
        completed = run_fn(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"},
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        _fail("INSTALLED_CONTEXT_REJECTED")
    if completed.returncode != 0 or completed.stderr:
        _fail("INSTALLED_CONTEXT_REJECTED")
    if not completed.stdout.endswith(b"\n") or b"\0" in completed.stdout:
        _fail("INSTALLED_CONTEXT_REJECTED")
    fields = completed.stdout[:-1].split(b"\n")
    if len(fields) != 7:
        _fail("INSTALLED_CONTEXT_REJECTED")
    try:
        return tuple(item.decode("ascii", errors="strict") for item in fields)  # type: ignore[return-value]
    except UnicodeError:
        _fail("INSTALLED_CONTEXT_REJECTED")


def verify_installed_context(
    helper_file: os.PathLike[str] | str,
    expected_commit: str,
    *,
    require_binding: bool = True,
    run_fn: Callable[..., subprocess.CompletedProcess[bytes]] | None = None,
) -> InstalledContext:
    if os.getuid() != 0 or os.geteuid() != 0 or not valid_commit(expected_commit):
        _fail("INSTALLED_CONTEXT_REJECTED")
    helper = pathlib.Path(os.path.abspath(os.fspath(helper_file)))
    try:
        if pathlib.Path(os.path.realpath(helper, strict=True)) != helper:
            _fail("INSTALLED_CONTEXT_REJECTED")
        match = re.fullmatch(
            r"(.+)/releases/([0-9a-f]{40})/scripts/ops/lib/market-stream-grant\.py",
            helper.as_posix(),
        )
        if match is None:
            _fail("INSTALLED_CONTEXT_REJECTED")
        root = pathlib.Path(match.group(1))
        commit = match.group(2)
        if commit != expected_commit or not valid_commit(commit):
            _fail("INSTALLED_CONTEXT_REJECTED")
        trusted_directory_chain(root)
        release_dir = root / "releases" / commit
        trusted_directory_chain(release_dir / "scripts" / "ops" / "lib")
        trusted_directory_chain(release_dir / "deploy" / "compose")
        current = root / "current"
        link_stat = os.lstat(current)
        if (
            not stat.S_ISLNK(link_stat.st_mode)
            or link_stat.st_uid != 0
            or not installed_current_target_matches(os.readlink(current), commit)
        ):
            _fail("INSTALLED_CONTEXT_REJECTED")
    except (OSError, GrantFailure):
        _fail("INSTALLED_CONTEXT_REJECTED")

    wrapper = release_dir / "scripts" / "ops" / "install-owner-market-stream-grant.sh"
    helper_path = release_dir / "scripts" / "ops" / "lib" / "market-stream-grant.py"
    sql_file = release_dir / "scripts" / "ops" / "lib" / "market-stream-grant.sql"
    db_library = release_dir / "scripts" / "ops" / "lib" / "db.sh"
    dotenv_library = release_dir / "scripts" / "ops" / "lib" / "dotenv.sh"
    manifest_library = release_dir / "scripts" / "ops" / "lib" / "release-image-manifest.sh"
    compose_file = release_dir / "deploy" / "compose" / "compose.yml"
    env_file = release_dir / "deploy" / "compose" / ".env"
    manifest_file = release_dir / ".lagrange-release-manifest"

    for path in (
        wrapper,
        helper_path,
        sql_file,
        db_library,
        dotenv_library,
        manifest_library,
        compose_file,
    ):
        _trusted_release_file(path)
    _trusted_release_file(env_file, exact_mode=0o600)
    _trusted_release_file(manifest_file, exact_mode=0o600)
    if not inherited_overrides_are_absent(expected_commit):
        _fail("INSTALLED_CONTEXT_REJECTED")

    parsed = _shared_parser_snapshot(
        dotenv_library=dotenv_library,
        manifest_library=manifest_library,
        env_file=env_file,
        manifest_file=manifest_file,
        expected_commit=commit,
        run_fn=run_fn,
    )
    (
        parsed_commit,
        slot_id,
        credential_generation,
        grant_id,
        contract_sha256,
        image_id,
        image_revision,
    ) = parsed
    if not context_snapshot_is_valid(parsed, commit, require_binding=require_binding):
        _fail("INSTALLED_CONTEXT_REJECTED")
    return InstalledContext(
        root,
        release_dir,
        commit,
        wrapper,
        helper_path,
        sql_file,
        compose_file,
        env_file,
        manifest_file,
        slot_id,
        credential_generation,
        grant_id,
        contract_sha256,
        image_id,
        image_revision,
    )


def approval_matches_installed(
    payload: Mapping[str, Any], context: InstalledContext, expected_commit: str
) -> bool:
    return (
        payload.get("activation_commit") == expected_commit == context.commit
        and payload.get("credential_slot_id") == context.slot_id
        and payload.get("credential_generation") == context.credential_generation
        and payload.get("grant_id") == context.configured_grant_id
        and payload.get("network_contract_sha256") == context.contract_sha256
    )


def _docker_env(commit: str) -> dict[str, str]:
    return {
        "PATH": "/usr/bin:/bin",
        "LANG": "C",
        "LC_ALL": "C",
        "HOME": "/nonexistent",
        "DOCKER_CONFIG": "/nonexistent",
        "LAGRANGE_CODE_COMMIT": commit,
        "RANGE_RAW_BATCH_ID": "compose-config-disabled",
    }


def image_inspect_argv(context: InstalledContext) -> list[str]:
    return [
        DOCKER,
        "--host",
        DOCKER_HOST,
        "image",
        "inspect",
        "--format",
        '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}',
        context.db_migrate_image_id,
    ]


def render_compose_override(image_id: str) -> bytes:
    if re.fullmatch(r"sha256:[0-9a-f]{64}", image_id) is None:
        _fail("IMAGE_REJECTED")
    return (
        "services:\n"
        "  db-migrate:\n"
        f'    image: "{image_id}"\n'
        "    build: !reset null\n"
        "    pull_policy: never\n"
        "    healthcheck:\n"
        "      disable: true\n"
    ).encode("ascii")


def build_compose_argv(
    context: InstalledContext,
    override_file: pathlib.Path,
    operation_id: str,
    mode: str,
    payload_json: str,
    grant_id: str,
) -> tuple[list[str], str]:
    if mode not in {"install", "revoke"} or re.fullmatch(r"[0-9a-f]{32}", operation_id) is None:
        _fail("INVALID_INPUT")
    if not valid_uuid(grant_id):
        _fail("INVALID_INPUT")
    name = "market-stream-grant-" + operation_id
    argv = [
        DOCKER,
        "--host",
        DOCKER_HOST,
        "compose",
        "--project-name",
        COMPOSE_PROJECT,
        "--env-file",
        str(context.env_file),
        "-f",
        str(context.compose_file),
        "-f",
        str(override_file),
        "run",
        "--rm",
        "--no-deps",
        "-T",
        "--pull",
        "never",
        "--name",
        name,
        "--label",
        CONTAINER_LABEL + "=" + operation_id,
        "--entrypoint",
        "/bin/sh",
        "db-migrate",
        "-ec",
        CONTAINER_PSQL_SCRIPT,
        "market-stream-grant-psql",
        "-v",
        "grant_payload=" + payload_json,
        "-v",
        "grant_mode=" + mode,
        "-v",
        "grant_id=" + grant_id,
    ]
    return argv, name


def _docker_capture(
    argv: Sequence[str],
    commit: str,
    *,
    run_fn: Callable[..., subprocess.CompletedProcess[bytes]] | None = None,
    timeout: int = DOCKER_ACTION_TIMEOUT_SECONDS,
) -> subprocess.CompletedProcess[bytes]:
    run_fn = subprocess.run if run_fn is None else run_fn
    try:
        return run_fn(
            list(argv),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=_docker_env(commit),
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        _fail("DB_OUTCOME_UNKNOWN")


def verify_local_image(
    context: InstalledContext,
    *,
    run_fn: Callable[..., subprocess.CompletedProcess[bytes]] | None = None,
) -> bool:
    result = _docker_capture(image_inspect_argv(context), context.commit, run_fn=run_fn)
    if not image_inspection_matches(
        context, result.returncode, result.stdout, result.stderr
    ):
        _fail("IMAGE_REJECTED")
    return True


def image_inspection_matches(
    context: InstalledContext, returncode: int, stdout: bytes, stderr: bytes
) -> bool:
    expected = (context.db_migrate_image_id + "|" + context.commit + "\n").encode("ascii")
    return returncode == 0 and stderr == b"" and stdout == expected


def _write_private_override(image_id: str) -> tuple[pathlib.Path, pathlib.Path]:
    contents = render_compose_override(image_id)
    directory: pathlib.Path | None = None
    path: pathlib.Path | None = None
    try:
        directory = pathlib.Path(tempfile.mkdtemp(prefix="market-stream-grant-", dir="/tmp"))
        os.chmod(directory, 0o700)
        directory_stat = os.lstat(directory)
        if (
            not stat.S_ISDIR(directory_stat.st_mode)
            or directory_stat.st_uid != 0
            or stat.S_IMODE(directory_stat.st_mode) != 0o700
        ):
            _fail("CLEANUP_FAILED")
        path = directory / "compose.override.yml"
        descriptor = os.open(
            path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        try:
            view = memoryview(contents)
            while view:
                count = os.write(descriptor, view)
                view = view[count:]
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        metadata = os.lstat(path)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != 0
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_nlink != 1
        ):
            _fail("CLEANUP_FAILED")
        return directory, path
    except GrantFailure:
        if directory is not None:
            _remove_private_override(directory, path)
        raise
    except OSError:
        if directory is not None:
            _remove_private_override(directory, path)
        _fail("CLEANUP_FAILED")


def _remove_private_override(
    directory: pathlib.Path, path: pathlib.Path | None
) -> bool:
    try:
        if path is not None:
            metadata = os.lstat(path)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
                return False
            os.unlink(path)
        directory_metadata = os.lstat(directory)
        if (
            not stat.S_ISDIR(directory_metadata.st_mode)
            or directory_metadata.st_uid != 0
            or stat.S_IMODE(directory_metadata.st_mode) != 0o700
        ):
            return False
        os.rmdir(directory)
        return True
    except OSError:
        return False


def _terminate_owned_process_group(
    process: Any,
    *,
    killpg_fn: Callable[[int, int], None] | None = None,
) -> bool:
    killpg_fn = os.killpg if killpg_fn is None else killpg_fn
    process_id = getattr(process, "pid", None)
    if type(process_id) is not int or process_id <= 0:
        return False
    try:
        killpg_fn(process_id, signal.SIGTERM)
    except ProcessLookupError:
        pass
    except OSError:
        return False
    try:
        process.communicate(timeout=5)
        return True
    except subprocess.TimeoutExpired:
        try:
            killpg_fn(process_id, signal.SIGKILL)
            process.communicate(timeout=5)
            return True
        except (OSError, subprocess.TimeoutExpired):
            return False
    except OSError:
        return False


def _container_ids(
    context: InstalledContext,
    filter_value: str,
    *,
    run_fn: Callable[..., subprocess.CompletedProcess[bytes]] | None = None,
) -> list[str]:
    listed = _docker_capture(
        [DOCKER, "--host", DOCKER_HOST, "ps", "-aq", "--no-trunc", "--filter", filter_value],
        context.commit,
        run_fn=run_fn,
    )
    if listed.returncode != 0 or listed.stderr:
        _fail("CLEANUP_FAILED")
    if listed.stdout == b"":
        return []
    if re.fullmatch(rb"[0-9a-f]{64}\n", listed.stdout) is None:
        _fail("CLEANUP_FAILED")
    return [listed.stdout[:-1].decode("ascii")]


def cleanup_exact_container(
    context: InstalledContext,
    operation_id: str,
    container_name: str,
    *,
    run_fn: Callable[..., subprocess.CompletedProcess[bytes]] | None = None,
) -> bool:
    if (
        re.fullmatch(r"[0-9a-f]{32}", operation_id) is None
        or container_name != "market-stream-grant-" + operation_id
    ):
        return False
    name_filter = "name=^/" + container_name + "$"
    try:
        container_ids = _container_ids(context, name_filter, run_fn=run_fn)
    except GrantFailure:
        return False
    if not container_ids:
        return True
    container_id = container_ids[0]
    inspect_argv = [
        DOCKER,
        "--host",
        DOCKER_HOST,
        "container",
        "inspect",
        "--format",
        (
            '{{.Id}}|{{.Image}}|'
            '{{index .Config.Labels "io.lagrange.market-stream.grant-operation"}}|'
            "{{.Name}}|{{.State.Running}}"
        ),
        container_id,
    ]
    try:
        inspected = _docker_capture(
            inspect_argv, context.commit, run_fn=run_fn, timeout=DOCKER_ACTION_TIMEOUT_SECONDS
        )
    except GrantFailure:
        return False
    if inspected.returncode != 0 or inspected.stderr:
        return False
    try:
        if not inspected.stdout.endswith(b"\n"):
            return False
        fields = inspected.stdout[:-1].decode("ascii", errors="strict").split("|")
    except UnicodeError:
        return False
    if (
        len(fields) != 5
        or fields[0] != container_id
        or fields[1] != context.db_migrate_image_id
        or fields[2] != operation_id
        or fields[3] != "/" + container_name
        or fields[4] not in {"true", "false"}
    ):
        return False
    remove_argv = [
        DOCKER,
        "--host",
        DOCKER_HOST,
        "container",
        "rm",
        "--force",
        container_id,
    ]
    try:
        removed = _docker_capture(
            remove_argv, context.commit, run_fn=run_fn, timeout=DOCKER_ACTION_TIMEOUT_SECONDS
        )
    except GrantFailure:
        return False
    if removed.returncode != 0 or removed.stderr or removed.stdout != (container_id + "\n").encode("ascii"):
        return False
    try:
        id_absent = not _container_ids(context, "id=" + container_id, run_fn=run_fn)
        name_absent = not _container_ids(context, name_filter, run_fn=run_fn)
        return id_absent and name_absent
    except GrantFailure:
        return False


def run_db_operation(
    context: InstalledContext,
    operation_id: str,
    mode: str,
    payload_json: str,
    grant_id: str,
    sql_bytes: bytes,
    *,
    run_fn: Callable[..., subprocess.CompletedProcess[bytes]] | None = None,
    popen_factory: Callable[..., Any] | None = None,
    killpg_fn: Callable[[int, int], None] | None = None,
) -> str:
    if (
        re.fullmatch(r"[0-9a-f]{32}", operation_id) is None
        or mode not in {"install", "revoke"}
        or not valid_uuid(grant_id)
        or type(sql_bytes) is not bytes
        or len(sql_bytes) > MAX_SQL_BYTES
    ):
        _fail("INVALID_INPUT")
    verify_local_image(context, run_fn=run_fn)
    directory, override_path = _write_private_override(context.db_migrate_image_id)
    result = "DB_OUTCOME_UNKNOWN"
    process = None
    attempted = False
    process_cleanup_ok = True
    container_name = "market-stream-grant-" + operation_id
    try:
        command, container_name = build_compose_argv(
            context, override_path, operation_id, mode, payload_json, grant_id
        )
        popen_factory = subprocess.Popen if popen_factory is None else popen_factory
        attempted = True
        process = popen_factory(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=_docker_env(context.commit),
            start_new_session=True,
        )
        try:
            stdout, stderr = process.communicate(
                input=sql_bytes, timeout=CONTAINER_TIMEOUT_SECONDS
            )
        except subprocess.TimeoutExpired:
            process_cleanup_ok = _terminate_owned_process_group(process, killpg_fn=killpg_fn)
        else:
            if process.returncode == 0 and stderr == b"":
                expected_statuses = (
                    {b"APPLIED\n", b"ALREADY_APPLIED\n"} if mode == "install"
                    else {b"REVOKED\n", b"ALREADY_REVOKED\n"}
                )
                if stdout in expected_statuses:
                    result = stdout.decode("ascii").strip()
    except (GrantFailure, OSError, ValueError):
        result = "DB_OUTCOME_UNKNOWN"
        if process is not None and process.returncode is None:
            process_cleanup_ok = _terminate_owned_process_group(process, killpg_fn=killpg_fn)
    finally:
        # Compose --rm is not cleanup evidence. Reconcile every attempted run,
        # including success, command failure, timeout and transport exceptions.
        if attempted:
            if process is not None and process.returncode is None and process_cleanup_ok:
                process_cleanup_ok = _terminate_owned_process_group(process, killpg_fn=killpg_fn)
            container_cleanup_ok = cleanup_exact_container(
                context, operation_id, container_name, run_fn=run_fn
            )
            if not process_cleanup_ok or not container_cleanup_ok:
                result = "CLEANUP_FAILED"
        if not _remove_private_override(directory, override_path):
            result = "CLEANUP_FAILED"
    return result


def _read_trusted_sql(context: InstalledContext) -> bytes:
    metadata = _trusted_release_file(context.sql_file, max_bytes=MAX_SQL_BYTES)
    if not hasattr(os, "O_NOFOLLOW"):
        _fail("INSTALLED_CONTEXT_REJECTED")
    try:
        descriptor = os.open(
            context.sql_file,
            os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_CLOEXEC", 0) | os.O_NOFOLLOW,
        )
    except OSError:
        _fail("INSTALLED_CONTEXT_REJECTED")
    try:
        opened = os.fstat(descriptor)
        if (
            not stat.S_ISREG(opened.st_mode)
            or opened.st_uid != 0
            or opened.st_gid != 0
            or stat.S_IMODE(opened.st_mode) & 0o022
            or opened.st_dev != metadata.st_dev
            or opened.st_ino != metadata.st_ino
            or opened.st_size > MAX_SQL_BYTES
        ):
            _fail("INSTALLED_CONTEXT_REJECTED")
        chunks: list[bytes] = []
        total = 0
        while True:
            block = os.read(descriptor, min(8192, MAX_SQL_BYTES + 1 - total))
            if not block:
                break
            chunks.append(block)
            total += len(block)
            if total > MAX_SQL_BYTES:
                _fail("INSTALLED_CONTEXT_REJECTED")
        if total != opened.st_size:
            _fail("INSTALLED_CONTEXT_REJECTED")
        return b"".join(chunks)
    except GrantFailure:
        raise
    except OSError:
        _fail("INSTALLED_CONTEXT_REJECTED")
    finally:
        os.close(descriptor)


def _new_operation_id() -> str:
    return uuid.uuid4().hex


def main(
    argv: Sequence[str] | None = None,
    *,
    output: Any = None,
    operation_id_factory: Callable[[], str] | None = None,
) -> str:
    output = sys.stdout if output is None else output
    operation_id_factory = _new_operation_id if operation_id_factory is None else operation_id_factory
    operation_id = operation_id_factory()
    status = "INTERNAL_ERROR"
    try:
        request = parse_cli(sys.argv[1:] if argv is None else argv)
        if request.mode == "plan":
            status = "PLAN"
        else:
            context = verify_installed_context(
                __file__, request.expected_commit or "", require_binding=request.mode == "install"
            )
            if request.mode == "install":
                raw = read_trusted_approval_file(
                    request.approval_path or ""
                )
                payload = parse_approval_bytes(raw, request.approval_sha256 or "")
                if not approval_matches_installed(
                    payload, context, request.expected_commit or ""
                ):
                    _fail("APPROVAL_REJECTED")
                payload_json = raw[:-1].decode("utf-8", errors="strict")
                grant_id = payload["grant_id"]
            else:
                payload_json = "{}"
                grant_id = request.grant_id or ""
            sql_bytes = _read_trusted_sql(context)
            status = run_db_operation(
                context, operation_id, request.mode, payload_json, grant_id, sql_bytes
            )
            if status not in SUCCESS | FAILURES:
                status = "INTERNAL_ERROR"
    except GrantFailure as failure:
        status = failure.code
    except Exception:
        status = "INTERNAL_ERROR"
    if status not in SUCCESS | FAILURES:
        status = "INTERNAL_ERROR"
    output.write(
        "MARKET_STREAM_GRANT: " + status + " operation_id=" + operation_id + "\n"
    )
    output.flush()
    return status


if __name__ == "__main__":
    result = main()
    raise SystemExit(0 if result in SUCCESS else 1)
