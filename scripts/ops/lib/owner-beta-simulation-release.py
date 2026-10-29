#!/usr/bin/env python3
"""Validate and stage the protected owner-beta simulation reader binding.

This preparation library never starts a service, opens a database, or accesses
the configured historical artifact root. Runtime activation is a separate
release concern.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import stat
import sys
import unicodedata
import uuid
from functools import wraps


CONFIG_LIMIT = 16 * 1024
MANIFEST_LIMIT = 64 * 1024
RECEIPT_LIMIT = 64 * 1024
COMPLETE_BYTES = 65
CONFIG_DIRECTORY = ".owner-beta-simulation"
CONFIG_NAME = "config.json"
MANIFEST_NAME = ".lagrange-release-manifest"
FLOOR_NAME = ".owner-beta-simulation-reader-floor"
READER_PATH = "configs/evidence/owner-beta-simulation-reader-v1.json"
READER_BYTES = (
    b'{"schema":"owner-beta-simulation-reader-v1",'
    b'"required_migration_versions":[63,64],'
    b'"input_versions":["owner-beta-price-only-simulation-job-v1",'
    b'"owner-beta-price-only-simulation-job-v2"],'
    b'"result_versions":["owner-beta-price-only-result-v1",'
    b'"owner-beta-price-only-result-v2"],'
    b'"preserve_simulation_history":true}'
)
FLOOR_BYTES = (
    b'{"schema":"owner-beta-simulation-reader-floor-v1",'
    b'"reader":"owner-beta-simulation-reader-v1"}'
)

SIMULATION_SCHEMA = "owner-beta-simulation-process-v1"
MANIFEST_V3_FORMAT = "LAGRANGE_RELEASE_MANIFEST_V3"
BUILD_ORIGIN = "official-release-v1"
PACKAGE = "job-queue"
BIN = "owner-beta-simulation-runner"
TARGET = "x86_64-unknown-linux-musl"
TOOLCHAIN = "1.97.1"
PROFILE = "release"
PLATFORM = "linux/amd64"
RECEIPT_FORMAT = "lagrange-rust-artifact-v1"

_DAILY_MODULE_NAME = "_lagrange_owner_beta_daily_release_wp5c1"
_POLICY_MODULE_NAME = "_lagrange_service_status_policy_wp5c1"
_SHA256_RE = re.compile(r"sha256:[0-9a-f]{64}\Z")
_HEX64_RE = re.compile(r"[0-9a-f]{64}\Z")
_COMMIT_RE = re.compile(r"[0-9a-f]{40}\Z")
_CONFIG_KEYS = frozenset({
    "schema", "code_commit", "manifest_format", "manifest_sha256", "image_id",
    "receipt_sha256", "binary_sha256", "source_input_sha256", "recipe_sha256",
    "build_origin", "package", "bin", "target", "profile", "toolchain", "owner",
    "entitlement_reference", "entitlement_sha256", "historical_root", "reader_uid",
    "reader_gid", "paths", "limits",
})
_PATHS = {
    "config": "/run/owner-beta-simulation/config.json",
    "artifact_root": "/data/owner-beta-artifacts",
    "receipt": "/usr/share/lagrange/owner-beta-simulation/artifact.json",
    "complete": "/usr/share/lagrange/owner-beta-simulation/COMPLETE",
    "executable": "/usr/local/bin/owner-beta-simulation-runner",
    "worker_id": "owner-beta-simulation-runner",
}
_LIMITS = {
    "memory_bytes": (1, 2**63 - 1),
    "nano_cpus": (1, 2**63 - 1),
    "health_tmpfs_bytes": (1, 2**63 - 1),
    "pids_limit": (1, 2**31 - 1),
    "startup_timeout_seconds": (1, 2**31 - 1),
    "health_max_age_seconds": (1, 2**31 - 1),
    "stop_timeout_seconds": (11, 2**31 - 1),
}
_RECEIPT_KEYS = frozenset({
    "binary_mode", "binary_sha256", "bin", "cache_key", "cargo_success",
    "compile_env", "features", "format", "h_sha256", "host_triple", "input_sha256",
    "package", "platform", "profile", "recipe_sha256", "source_commit",
})


class SimulationInstallError(Exception):
    """A fixed, non-sensitive error code for the public boundary."""

    def __init__(self, code: str = "SIMULATION_INSTALL_INVALID"):
        super().__init__(code)
        self.code = code


def _reject(code: str = "SIMULATION_INSTALL_INVALID") -> None:
    raise SimulationInstallError(code)


def _daily():
    loaded = sys.modules.get(_DAILY_MODULE_NAME)
    if loaded is not None:
        return loaded
    path = os.path.join(os.path.dirname(__file__), "owner-beta-daily-release.py")
    spec = importlib.util.spec_from_file_location(_DAILY_MODULE_NAME, path)
    if spec is None or spec.loader is None:
        _reject("SIMULATION_INSTALL_DEPENDENCY")
    module = importlib.util.module_from_spec(spec)
    sys.modules[_DAILY_MODULE_NAME] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        sys.modules.pop(_DAILY_MODULE_NAME, None)
        _reject("SIMULATION_INSTALL_DEPENDENCY")
    return module


def _policy():
    loaded = sys.modules.get(_POLICY_MODULE_NAME)
    if loaded is not None:
        return loaded
    path = os.path.join(os.path.dirname(__file__), "service-status-policy.py")
    spec = importlib.util.spec_from_file_location(_POLICY_MODULE_NAME, path)
    if spec is None or spec.loader is None:
        _reject("SIMULATION_INSTALL_DEPENDENCY")
    module = importlib.util.module_from_spec(spec)
    sys.modules[_POLICY_MODULE_NAME] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        sys.modules.pop(_POLICY_MODULE_NAME, None)
        _reject("SIMULATION_INSTALL_DEPENDENCY")
    return module


def _boundary(function):
    @wraps(function)
    def wrapped(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except SimulationInstallError:
            raise
        except OSError as error:
            code = "SIMULATION_INSTALL_IO"
            if isinstance(error, FileNotFoundError):
                code = "SIMULATION_INSTALL_MISSING"
            elif isinstance(error, FileExistsError):
                code = "SIMULATION_INSTALL_EXISTS"
            raise SimulationInstallError(code) from None
        except Exception as error:
            daily = sys.modules.get(_DAILY_MODULE_NAME)
            daily_error = getattr(daily, "DailyInstallError", None) if daily else None
            if daily_error is not None and isinstance(error, daily_error):
                raw = error.args[0] if error.args else ""
                suffix = raw.removeprefix("DAILY_INSTALL_") if type(raw) is str else ""
                allowed = {
                    "PLATFORM", "PATH", "FILE", "SIZE", "CHANGED", "ACL", "POLICY",
                    "STATE", "BUSY", "WRITE", "MANIFEST", "PERMISSION", "INVALID",
                }
                code = "SIMULATION_INSTALL_" + suffix if suffix in allowed else "SIMULATION_INSTALL_INVALID"
                raise SimulationInstallError(code) from None
            raise SimulationInstallError("SIMULATION_INSTALL_INVALID") from None
    return wrapped


def _is_commit(value) -> bool:
    return (type(value) is str and _COMMIT_RE.fullmatch(value) is not None
            and value != "0" * 40)


def _is_sha256(value) -> bool:
    return type(value) is str and _SHA256_RE.fullmatch(value) is not None


def _is_hex64(value) -> bool:
    return type(value) is str and _HEX64_RE.fullmatch(value) is not None


def _canonical(value) -> bytes:
    try:
        return json.dumps(value, ensure_ascii=False, separators=(",", ":"),
                          allow_nan=False).encode("utf-8")
    except (TypeError, ValueError, UnicodeError, RecursionError):
        _reject()


def _unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            _reject()
        result[key] = value
    return result


def _bad_constant(_value):
    _reject()


def _valid_strings(value):
    if type(value) is str:
        if any(0xD800 <= ord(character) <= 0xDFFF for character in value):
            _reject()
    elif type(value) is list:
        for item in value:
            _valid_strings(item)
    elif type(value) is dict:
        for key, item in value.items():
            if type(key) is not str:
                _reject()
            _valid_strings(key)
            _valid_strings(item)


def _json_object(data: bytes, maximum: int):
    if type(data) is not bytes or len(data) > maximum:
        _reject("SIMULATION_INSTALL_SIZE")
    try:
        value = json.loads(data.decode("utf-8", "strict"), object_pairs_hook=_unique_pairs,
                           parse_constant=_bad_constant)
    except SimulationInstallError:
        raise
    except (UnicodeError, ValueError, RecursionError):
        _reject()
    if type(value) is not dict:
        _reject()
    _valid_strings(value)
    return value


def _canonical_host_path(value: str) -> bool:
    if type(value) is not str or not value.startswith("/") or value == "/" or value.endswith("/"):
        return False
    if any(unicodedata.category(character) == "Cc"
           or 0xD800 <= ord(character) <= 0xDFFF for character in value):
        return False
    parts = value[1:].split("/")
    return all(part not in ("", ".", "..") for part in parts)


def _validate_config_value(value, commit: str):
    if type(value) is not dict or set(value) != _CONFIG_KEYS or not _is_commit(commit):
        _reject()
    fixed = {
        "schema": SIMULATION_SCHEMA,
        "manifest_format": MANIFEST_V3_FORMAT,
        "build_origin": BUILD_ORIGIN,
        "package": PACKAGE,
        "bin": BIN,
        "target": TARGET,
        "profile": PROFILE,
        "toolchain": TOOLCHAIN,
    }
    for key, expected in fixed.items():
        if type(value.get(key)) is not str or value[key] != expected:
            _reject()
    if not _is_commit(value.get("code_commit")) or value["code_commit"] != commit:
        _reject()
    for key in ("manifest_sha256", "image_id", "receipt_sha256", "binary_sha256",
                "source_input_sha256", "recipe_sha256", "entitlement_sha256"):
        if not _is_sha256(value.get(key)):
            _reject()
    owner = value.get("owner")
    if type(owner) is not str:
        _reject()
    try:
        parsed_owner = uuid.UUID(owner)
    except (ValueError, AttributeError):
        _reject()
    if parsed_owner.int == 0 or str(parsed_owner) != owner:
        _reject()
    reference = value.get("entitlement_reference")
    if (type(reference) is not str or not reference or reference.strip() != reference
            or len(reference) > 512 or len(reference.encode("utf-8")) > 4096
            or any(unicodedata.category(character) == "Cc" for character in reference)):
        _reject()
    if not _canonical_host_path(value.get("historical_root")):
        _reject()
    if type(value.get("reader_uid")) is not int or value["reader_uid"] != 10001:
        _reject()
    if type(value.get("reader_gid")) is not int or value["reader_gid"] != 10001:
        _reject()
    paths = value.get("paths")
    if type(paths) is not dict or set(paths) != set(_PATHS) or paths != _PATHS:
        _reject()
    limits = value.get("limits")
    if type(limits) is not dict or set(limits) != set(_LIMITS):
        _reject()
    for key, (minimum, maximum) in _LIMITS.items():
        number = limits.get(key)
        if type(number) is not int or not minimum <= number <= maximum:
            _reject()
    return value


def _validate_config(data: bytes, commit: str):
    if type(data) is not bytes or len(data) > CONFIG_LIMIT:
        _reject("SIMULATION_INSTALL_SIZE")
    return _validate_config_value(_json_object(data, CONFIG_LIMIT), commit)


def _manifest_services(manifest_format: str):
    policy = _policy()
    if manifest_format == policy.MANIFEST_V2_FORMAT:
        return tuple(policy.MANIFEST_V2_SERVICES)
    if manifest_format == policy.MANIFEST_V3_FORMAT:
        return tuple(policy.MANIFEST_V3_SERVICES)
    _reject()


def _validate_manifest(data: bytes, commit: str):
    if type(data) is not bytes or len(data) > MANIFEST_LIMIT or not _is_commit(commit):
        _reject("SIMULATION_INSTALL_SIZE")
    try:
        text = data.decode("ascii", "strict")
    except UnicodeDecodeError:
        _reject()
    if not text.endswith("\n") or "\r" in text or "\x00" in text:
        _reject()
    lines = text[:-1].split("\n")
    if not lines:
        _reject()
    manifest_format = lines[0]
    services = _manifest_services(manifest_format)
    if len(lines) != len(services) + 2 or lines[1] != "commit|" + commit:
        _reject()
    images = {}
    for index, service in enumerate(services, start=2):
        fields = lines[index].split("|")
        if len(fields) != 5 or fields[0] != "image" or fields[1] != service:
            _reject()
        expected_ref = f"lagrange-station-{service}:{commit}"
        if (fields[2] != expected_ref or not _is_sha256(fields[3])
                or fields[4] != commit):
            _reject()
        images[service] = fields[3]
    return {
        "format": manifest_format,
        "commit": commit,
        "sha256": "sha256:" + hashlib.sha256(data).hexdigest(),
        "services": services,
        "image_ids": images,
    }


def _revalidate_config(config):
    if type(config) is not dict:
        _reject()
    commit = config.get("code_commit")
    if not _is_commit(commit):
        _reject()
    encoded = _canonical(config)
    return _validate_config(encoded, commit)


def _expected_compile_env(source_commit: str, input_sha256: str, recipe_sha256: str):
    return {
        "CARGO_BUILD_JOBS": "2",
        "CARGO_TARGET_DIR": "/cargo-target",
        "RUSTFLAGS": "<unset>",
        "LAGRANGE_CODE_COMMIT": source_commit,
        "R4_SIMULATION_BUILD_GIT_COMMIT": source_commit,
        "R4_SIMULATION_BUILD_TOOLCHAIN": TOOLCHAIN,
        "R4_SIMULATION_BUILD_TARGET": TARGET,
        "R4_SIMULATION_BUILD_PROFILE": PROFILE,
        "R4_SIMULATION_BUILD_ORIGIN": BUILD_ORIGIN,
        "R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256": input_sha256,
        "R4_SIMULATION_BUILD_RECIPE_SHA256": recipe_sha256,
    }


def _validate_receipt(data: bytes, complete: bytes, config: dict):
    validated_config = _revalidate_config(config)
    receipt = _json_object(data, RECEIPT_LIMIT)
    if set(receipt) != _RECEIPT_KEYS:
        _reject()
    for key in ("binary_mode", "binary_sha256", "bin", "cache_key", "format", "h_sha256",
                "host_triple", "input_sha256", "package", "platform", "profile",
                "recipe_sha256", "source_commit"):
        if type(receipt.get(key)) is not str:
            _reject()
    if (type(receipt.get("cargo_success")) is not bool
            or type(receipt.get("features")) is not list
            or any(type(item) is not str for item in receipt["features"])
            or type(receipt.get("compile_env")) is not dict
            or any(type(key) is not str or type(item) is not str
                   for key, item in receipt["compile_env"].items())):
        _reject()
    if (receipt["binary_mode"] != "0755" or receipt["bin"] != BIN
            or receipt["cargo_success"] is not True or receipt["features"] != []
            or receipt["format"] != RECEIPT_FORMAT or receipt["host_triple"] != TARGET
            or receipt["package"] != PACKAGE or receipt["platform"] != PLATFORM
            or receipt["profile"] != PROFILE):
        _reject()
    if not _is_commit(receipt["source_commit"]):
        _reject()
    for key in ("binary_sha256", "cache_key", "h_sha256", "input_sha256", "recipe_sha256"):
        if not _is_hex64(receipt[key]):
            _reject()
    config_input = validated_config["source_input_sha256"][len("sha256:"):]
    config_recipe = validated_config["recipe_sha256"][len("sha256:"):]
    expected_env = _expected_compile_env(receipt["source_commit"], config_input, config_recipe)
    if (receipt["source_commit"] != validated_config["code_commit"]
            or receipt["binary_sha256"] != validated_config["binary_sha256"][len("sha256:"):]
            or receipt["input_sha256"] != config_input
            or receipt["recipe_sha256"] != config_recipe
            or receipt["compile_env"] != expected_env):
        _reject()
    receipt_digest = hashlib.sha256(data).hexdigest()
    if (type(complete) is not bytes or len(complete) != COMPLETE_BYTES
            or complete != receipt_digest.encode("ascii") + b"\n"
            or validated_config["receipt_sha256"] != "sha256:" + receipt_digest):
        _reject()
    return receipt


def _assert_manifest_binding(config, manifest):
    if (manifest["format"] != MANIFEST_V3_FORMAT
            or config["manifest_format"] != MANIFEST_V3_FORMAT
            or config["manifest_sha256"] != manifest["sha256"]
            or config["image_id"] != manifest["image_ids"].get(BIN)):
        _reject()


def _validate_binding(config_data, manifest_data, receipt_data, complete_data, commit):
    config = _validate_config(config_data, commit)
    manifest = _validate_manifest(manifest_data, commit)
    _assert_manifest_binding(config, manifest)
    _validate_receipt(receipt_data, complete_data, config)
    return config


@_boundary
def validate_config(data: bytes, commit: str):
    """Validate closed config bytes and return the parsed config mapping."""
    return _validate_config(data, commit)


@_boundary
def validate_manifest(data: bytes, commit: str):
    """Validate exact V2/V3 manifest bytes and return the parsed binding."""
    return _validate_manifest(data, commit)


@_boundary
def validate_receipt(data: bytes, complete: bytes, config: dict):
    """Validate official receipt bytes after revalidating the supplied config."""
    return _validate_receipt(data, complete, config)


@_boundary
def validate_binding(config_data, manifest_data, receipt_data, complete_data, commit):
    """Validate config, manifest, receipt and COMPLETE as one exact binding."""
    return _validate_binding(config_data, manifest_data, receipt_data, complete_data, commit)


@_boundary
def stage_config(stage, source, receipt_source, complete_source, commit):
    """Stage original config bytes after all four binding inputs validate."""
    if (not all(_canonical_host_path(path) for path in
                (stage, source, receipt_source, complete_source)) or not _is_commit(commit)):
        _reject("SIMULATION_INSTALL_PATH")
    daily = _daily()
    daily.require_root()
    config_data = daily.read_root_file(source, gid=0, mode=0o600, maximum=CONFIG_LIMIT)
    receipt_data = daily.read_root_file(receipt_source, gid=0, mode=0o444,
                                        maximum=RECEIPT_LIMIT)
    complete_data = daily.read_root_file(complete_source, gid=0, mode=0o444,
                                         maximum=COMPLETE_BYTES)
    with daily.root_directory(stage) as parent:
        manifest_data = daily.read_file(parent, MANIFEST_NAME, uid=0, gid=0,
                                        mode=0o600, maximum=MANIFEST_LIMIT)
        config = _validate_binding(config_data, manifest_data, receipt_data, complete_data, commit)
        os.mkdir(CONFIG_DIRECTORY, 0o700, dir_fd=parent)
        os.fsync(parent)
        directory_fd = os.open(CONFIG_DIRECTORY, daily.DIR_FLAGS, dir_fd=parent)
        try:
            daily.directory_info(directory_fd, uid=0, gid=0, mode=0o700)
            daily.create_file(directory_fd, CONFIG_NAME, config_data, 0, 10001, 0o440)
            os.fchown(directory_fd, 0, 10001)
            os.fchmod(directory_fd, 0o750)
            daily.directory_info(directory_fd, uid=0, gid=10001, mode=0o750)
            after = os.fstat(directory_fd)
            named = os.stat(CONFIG_DIRECTORY, dir_fd=parent, follow_symlinks=False)
            if (not stat.S_ISDIR(named.st_mode)
                    or (after.st_dev, after.st_ino) != (named.st_dev, named.st_ino)):
                _reject("SIMULATION_INSTALL_CHANGED")
            os.fsync(directory_fd)
            os.fsync(parent)
        finally:
            os.close(directory_fd)
    return config


def _validate_release_path(release, commit):
    if (not _canonical_host_path(release) or not _is_commit(commit)
            or os.path.basename(release) != commit):
        _reject("SIMULATION_INSTALL_PATH")


def _verify_config(daily, release, commit):
    _validate_release_path(release, commit)
    with daily.root_directory(release) as release_fd:
        manifest_data = daily.read_file(release_fd, MANIFEST_NAME, uid=0, gid=0,
                                        mode=0o600, maximum=MANIFEST_LIMIT)
        manifest = _validate_manifest(manifest_data, commit)
        with daily.child_directory(release_fd, CONFIG_DIRECTORY, uid=0, gid=10001,
                                   mode=0o750) as config_fd:
            if set(os.listdir(config_fd)) != {CONFIG_NAME}:
                _reject()
            config_data = daily.read_file(config_fd, CONFIG_NAME, uid=0, gid=10001,
                                          mode=0o440, maximum=CONFIG_LIMIT)
            config = _validate_config(config_data, commit)
            _assert_manifest_binding(config, manifest)
    return config, "sha256:" + hashlib.sha256(config_data).hexdigest()


@_boundary
def verify_config(release, commit: str):
    """Verify installed config and manifest bindings without opening inputs."""
    daily = _daily()
    daily.require_root()
    return _verify_config(daily, release, commit)


def _verify_reader_capability(daily, target_release):
    parts = READER_PATH.split("/")
    directory = os.path.join(target_release, *parts[:-1])
    with daily.root_directory(directory) as evidence_fd:
        data = daily.read_file(evidence_fd, parts[-1], uid=0, gid=0, mode=0o644,
                               maximum=len(READER_BYTES))
    if data != READER_BYTES:
        _reject("SIMULATION_INSTALL_POLICY")


def _read_optional_floor(daily, install_fd):
    try:
        data = daily.read_file(install_fd, FLOOR_NAME, uid=0, gid=0,
                               mode=0o600, maximum=len(FLOOR_BYTES))
    except FileNotFoundError:
        return False
    if data != FLOOR_BYTES:
        _reject()
    return True


def _config_entry_present(release_fd):
    try:
        os.stat(CONFIG_DIRECTORY, dir_fd=release_fd, follow_symlinks=False)
    except FileNotFoundError:
        return False
    return True


@_boundary
def reader_guard(install_root, target_release, commit, mode, *, history_present,
                 reserve=True):
    """Enforce the persistent reader floor for one immutable release target."""
    if (mode not in ("disabled", "owner_only") or type(history_present) is not bool
            or type(reserve) is not bool or not _is_commit(commit)
            or not _canonical_host_path(install_root)):
        _reject("SIMULATION_INSTALL_PATH")
    expected_release = install_root + "/releases/" + commit
    if target_release != expected_release or not _canonical_host_path(target_release):
        _reject("SIMULATION_INSTALL_PATH")
    daily = _daily()
    daily.require_root()
    with daily.root_directory(install_root) as install_fd:
        floor_present = _read_optional_floor(daily, install_fd)
        with daily.root_directory(target_release) as release_fd:
            manifest_data = daily.read_file(release_fd, MANIFEST_NAME, uid=0, gid=0,
                                            mode=0o600, maximum=MANIFEST_LIMIT)
            manifest = _validate_manifest(manifest_data, commit)
            config_present = _config_entry_present(release_fd)
            if config_present:
                _verify_config(daily, target_release, commit)
        requires_reader = floor_present or history_present or config_present or mode == "owner_only"
        if requires_reader:
            if manifest["format"] != MANIFEST_V3_FORMAT:
                _reject("SIMULATION_INSTALL_POLICY")
            _verify_reader_capability(daily, target_release)
            if mode == "owner_only" and not config_present:
                _reject("SIMULATION_INSTALL_POLICY")
            result = "SIMULATION_READER_REQUIRED"
        else:
            result = "LEGACY_OFF"
        if requires_reader and reserve and not floor_present:
            daily.create_file(install_fd, FLOOR_NAME, FLOOR_BYTES, 0, 0, 0o600)
        return result


class _SafeArgumentParser(argparse.ArgumentParser):
    def error(self, _message):
        self.exit(2, "SIMULATION_INSTALL_USAGE\n")


def _parser():
    parser = _SafeArgumentParser(prog="owner-beta-simulation-release.py")
    commands = parser.add_subparsers(dest="command", required=True)

    stage = commands.add_parser("stage-config")
    stage.add_argument("--stage", required=True)
    stage.add_argument("--source", required=True)
    stage.add_argument("--receipt-source", required=True)
    stage.add_argument("--complete-source", required=True)
    stage.add_argument("--commit", required=True)

    verify = commands.add_parser("verify-config")
    verify.add_argument("--release", required=True)
    verify.add_argument("--commit", required=True)

    for command in ("reader-check", "reader-guard"):
        reader = commands.add_parser(command)
        reader.add_argument("--install-root", required=True)
        reader.add_argument("--target-release", required=True)
        reader.add_argument("--commit", required=True)
        reader.add_argument("--mode", required=True)
        reader.add_argument("--history-present", action="store_true", default=False)
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        if args.command == "stage-config":
            stage_config(args.stage, args.source, args.receipt_source,
                         args.complete_source, args.commit)
            print("SIMULATION_INSTALL_OK")
        elif args.command == "verify-config":
            verify_config(args.release, args.commit)
            print("SIMULATION_INSTALL_OK")
        else:
            status = reader_guard(
                args.install_root, args.target_release, args.commit, args.mode,
                history_present=args.history_present, reserve=args.command == "reader-guard",
            )
            print(status)
        return 0
    except SimulationInstallError as error:
        print(error.code, file=sys.stderr)
        return 1
    except Exception:
        print("SIMULATION_INSTALL_INVALID", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
