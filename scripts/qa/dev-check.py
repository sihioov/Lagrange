#!/usr/bin/env python3
"""Thin, receipt-aware dispatcher for the repository's existing checks."""

from __future__ import annotations

import argparse
import datetime as dt
import errno
import fnmatch
import hashlib
import json
import os
import platform
import re
import shutil
import socket
import stat
import subprocess
import sys
import time
import tomllib
import uuid
from pathlib import Path, PurePosixPath
from typing import Any

try:
    import fcntl
except ImportError:  # Windows remains available for the non-heavy CLI stages.
    fcntl = None  # type: ignore[assignment]


ROOT = Path(__file__).resolve().parents[2]
SCHEMA_VERSION = 1
REUSE_ALLOWLIST = {
    "cargo-format": "formatter",
    "python-syntax": "python-syntax",
    "qa-python-syntax": "python-syntax",
    "qa-shell-syntax": "shell-syntax",
    "manifest-pins": "manifest-pins",
}
SECRET_FILE_NAMES = {
    "id_rsa",
    "id_ed25519",
    "credentials",
    "credentials.json",
    "secrets.json",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
}
SECRET_SUFFIXES = {".pem", ".p12", ".pfx", ".key", ".token", ".env", ".envrc", ".secret", ".credentials"}
GENERATED_PARTS = {".git", "target", "node_modules", ".venv"}
FINGERPRINT_ENV = {
    "PATH",
    "RUSTUP_TOOLCHAIN",
    "RUSTUP_AUTO_INSTALL",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_INCREMENTAL",
    "CARGO_BUILD_JOBS",
    "PYTHONPATH",
    "NODE_OPTIONS",
    "NPM_CONFIG_REGISTRY",
    "UV_INDEX_URL",
    "PLAYWRIGHT_BROWSERS_PATH",
    "CI",
}
DIAGNOSTIC_MAX_BYTES = 16 * 1024
SECRET_ENV_NAME = re.compile(r"(?:KEY|SECRET|TOKEN|PASSWORD|DATABASE_URL|ACCOUNT|AUTH|COOKIE)", re.I)


class InputError(Exception):
    """An invalid request or unresolved repository input."""


class HeavySlotBusy(InputError):
    """Another local heavy command currently owns the shared advisory lock."""


HEAVY_CHECK_IDS = {
    "cargo-clippy-workspace",
    "cargo-test-workspace",
    "python-pytest",
    "web-test",
    "web-build",
    "web-e2e",
}


def validate_private_lock_directory(raw_path: str | Path) -> Path:
    path = Path(raw_path).expanduser()
    if not path.is_absolute() or ".." in path.parts:
        raise InputError("heavy lock directory must be an absolute path without parent traversal")
    current = Path(path.anchor)
    try:
        for part in path.parts[1:]:
            current = current / part
            info = current.lstat()
            if stat.S_ISLNK(info.st_mode):
                raise InputError("heavy lock directory path contains a symlink")
        info = path.lstat()
    except OSError as exc:
        if exc.errno == errno.ENOENT:
            raise InputError("heavy lock directory does not exist; create an owner-private mode-0700 directory") from exc
        raise InputError("cannot inspect the heavy lock directory") from exc
    if not stat.S_ISDIR(info.st_mode):
        raise InputError("heavy lock location is not a directory")
    if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise InputError("heavy lock directory must be owned by the current user with mode 0700")
    return path


def heavy_lock_directory(explicit: str | Path | None = None) -> Path:
    if not sys.platform.startswith("linux") or fcntl is None:
        raise InputError("local focused heavy checks require a Linux advisory lock; use the hosted integration plan")
    raw = str(explicit) if explicit is not None else os.environ.get("XDG_RUNTIME_DIR", "")
    if not raw:
        raise InputError("local focused heavy checks require XDG_RUNTIME_DIR or --heavy-lock-dir")
    directory = validate_private_lock_directory(raw)
    if directory == ROOT or ROOT in directory.parents:
        raise InputError("heavy lock directory must be shared outside repository worktrees")
    return directory


class HeavySlot:
    """Cross-worktree, user-wide advisory lock for one local heavy child."""

    def __init__(self, directory: str | Path | None = None) -> None:
        if os.name != "posix":
            raise InputError("local focused heavy checks require a Linux advisory lock; use the hosted integration plan")
        self.directory = heavy_lock_directory(directory)
        self.path = self.directory / "lagrange-dev-check-heavy.lock"
        self.descriptor: int | None = None

    def __enter__(self) -> "HeavySlot":
        flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
        created = False
        try:
            try:
                self.descriptor = os.open(self.path, flags | os.O_EXCL, 0o600)
                created = True
            except FileExistsError:
                self.descriptor = os.open(self.path, flags)
            info = os.fstat(self.descriptor)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
                raise InputError("heavy lock file must be a regular file owned by the current user")
            if created:
                os.fchmod(self.descriptor, 0o600)
            elif stat.S_IMODE(info.st_mode) != 0o600:
                raise InputError("existing heavy lock file must have mode 0600")
            try:
                fcntl.flock(self.descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError as exc:
                if exc.errno in {errno.EACCES, errno.EAGAIN}:
                    raise HeavySlotBusy("another local heavy command holds the shared user lock; no child was started") from exc
                raise InputError("could not acquire the shared local heavy lock") from exc
            return self
        except Exception:
            self.close()
            raise

    def close(self) -> None:
        if self.descriptor is not None:
            try:
                fcntl.flock(self.descriptor, fcntl.LOCK_UN)
            except OSError:
                pass
            os.close(self.descriptor)
            self.descriptor = None

    def __exit__(self, exc_type: Any, exc: Any, traceback: Any) -> None:
        self.close()


def run_child_with_heavy_slot(
    argv: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    lock_directory: str | Path | None = None,
) -> subprocess.CompletedProcess[str]:
    with HeavySlot(lock_directory) as slot:
        # The owned heavy command waits for its own full lifetime. Do not
        # timeout-kill only a harness parent and release the slot underneath
        # descendants it may have started.
        child = subprocess.Popen(
            argv,
            cwd=cwd,
            text=True,
            encoding="utf-8",
            errors="replace",
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            pass_fds=(slot.descriptor,),
        )
        interrupted = False
        while True:
            try:
                stdout, stderr = child.communicate()
                break
            except KeyboardInterrupt:
                # Reap the owned child before releasing the slot. Inherited
                # lock ownership also lasts through direct-child exit if the
                # CLI is terminated abruptly; harnesses own their descendants.
                interrupted = True
        if interrupted:
            raise KeyboardInterrupt
        return subprocess.CompletedProcess(argv, child.returncode, stdout, stderr)


def local_heavy_execution(environment: str) -> bool:
    # An environment label cannot disable the local lock on a developer host.
    return environment == "local" or os.environ.get("GITHUB_ACTIONS") != "true"


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds").replace(
        "+00:00", "Z"
    )


def config_path(root: Path = ROOT) -> Path:
    return root / "scripts" / "qa" / "check-profiles.json"


def dispatcher_path(root: Path = ROOT) -> Path:
    return root / "scripts" / "qa" / "dev-check.py"


def open_repo_file(root: Path, relative: str):
    parts = PurePosixPath(relative).parts
    if PurePosixPath(relative).is_absolute() or not parts or any(part in {"", ".", ".."} for part in parts):
        raise OSError(errno.EINVAL, "unsafe repository-relative path")
    directory_fd: int | None = None
    file_fd: int | None = None
    try:
        if os.open in getattr(os, "supports_dir_fd", set()) and hasattr(os, "O_NOFOLLOW"):
            directory_flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | os.O_NOFOLLOW
            directory_fd = os.open(root, directory_flags)
            for part in parts[:-1]:
                next_fd = os.open(part, directory_flags, dir_fd=directory_fd)
                os.close(directory_fd)
                directory_fd = next_fd
            file_fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory_fd)
        else:
            current = root
            for index, part in enumerate(parts):
                current = current / part
                info = current.lstat()
                if stat.S_ISLNK(info.st_mode):
                    raise OSError(errno.ELOOP, "symlink in repository-relative path")
                if index < len(parts) - 1 and not stat.S_ISDIR(info.st_mode):
                    raise OSError(errno.ENOTDIR, "non-directory path component")
            file_fd = os.open(current, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        if not stat.S_ISREG(os.fstat(file_fd).st_mode):
            raise OSError(errno.EINVAL, "repository input is not a regular file")
        stream = os.fdopen(file_fd, "rb")
        file_fd = None
        return stream
    finally:
        if file_fd is not None:
            os.close(file_fd)
        if directory_fd is not None:
            os.close(directory_fd)


def load_config(root: Path = ROOT) -> dict[str, Any]:
    path = config_path(root)
    try:
        with open_repo_file(root, "scripts/qa/check-profiles.json") as stream:
            data = json.loads(stream.read().decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise InputError(f"cannot read profile configuration: {path.relative_to(root)}") from exc
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        raise InputError("unsupported or malformed profile configuration")
    profiles = data.get("profiles")
    checks = data.get("checks")
    if not isinstance(profiles, dict) or not isinstance(checks, dict):
        raise InputError("profile configuration is missing profiles or checks")
    for profile_name, profile in profiles.items():
        if not isinstance(profile, dict):
            raise InputError(f"malformed profile: {profile_name}")
        for stage in ("fast", "focused", "integration"):
            ids = profile.get(stage)
            if not isinstance(ids, list) or any(check_id not in checks for check_id in ids):
                raise InputError(f"malformed {profile_name}/{stage} check mapping")
    for check_id, check in checks.items():
        if not isinstance(check, dict) or not isinstance(check.get("argv"), list):
            raise InputError(f"malformed check definition: {check_id}")
        for required in ("cwd", "inputs", "reuse"):
            if required not in check:
                raise InputError(f"check {check_id} is missing {required}")
    return data


def git(root: Path, args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=root,
            text=True,
            encoding="utf-8",
            errors="replace",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=15,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise InputError("Git could not inspect the repository") from exc
    if check and result.returncode != 0:
        detail = result.stderr.strip().splitlines()
        raise InputError(detail[0] if detail else "Git inspection failed")
    return result


def resolve_commit(root: Path, ref: str) -> str:
    if not ref or "\0" in ref:
        raise InputError("base ref is empty or invalid")
    result = git(root, ["rev-parse", "--verify", "--end-of-options", f"{ref}^{{commit}}"], check=False)
    if result.returncode != 0 or not result.stdout.strip():
        raise InputError(f"base ref does not resolve to a commit: {ref}")
    return result.stdout.strip().splitlines()[0]


def parse_name_status_z(output: str) -> list[tuple[str, str]]:
    parts = output.split("\0")
    result: list[tuple[str, str]] = []
    index = 0
    while index < len(parts):
        status = parts[index]
        index += 1
        if not status:
            continue
        if status.startswith(("R", "C")):
            if index + 1 >= len(parts):
                raise InputError("Git returned a truncated rename record")
            old_path, new_path = parts[index], parts[index + 1]
            index += 2
            result.extend(((old_path, status), (new_path, status)))
        else:
            if index >= len(parts):
                raise InputError("Git returned a truncated path record")
            result.append((parts[index], status))
            index += 1
    return result


def sensitive_path(path: str) -> bool:
    parts = PurePosixPath(path).parts
    if any(part.lower() in {".aws", ".ssh", "secrets", "secret", "secret-store", "credentials", "private-keys"} for part in parts):
        return True
    for part in parts:
        lower = part.lower()
        if lower.startswith(".env"):
            return True
        if lower.endswith(("_secret.json", "_secret.yaml", "_secret.yml", "_credentials.json", "_credentials.toml")):
            return True
        if lower in SECRET_FILE_NAMES or Path(lower).suffix in SECRET_SUFFIXES:
            return True
    return False


def generated_path(path: str) -> bool:
    return any(part in GENERATED_PARTS for part in PurePosixPath(path).parts)


def matches(path: str, pattern: str) -> bool:
    if pattern == "@changed":
        return True
    if "/" not in pattern and not pattern.startswith("."):
        if "/" in path:
            return False
    escaped = re.escape(pattern)
    escaped = escaped.replace(r"\*\*/", r"(?:.*/)?")
    escaped = escaped.replace(r"\*\*", r".*")
    escaped = escaped.replace(r"\*", r"[^/]*")
    escaped = escaped.replace(r"\?", r"[^/]")
    return re.fullmatch(escaped, path) is not None


def collect_changes(root: Path, base: str) -> dict[str, Any]:
    base_commit = resolve_commit(root, base)
    head = git(root, ["rev-parse", "--verify", "HEAD"]).stdout.strip()
    statuses: dict[str, set[str]] = {}
    commands = (
        ["diff", "--name-status", "-z", "--find-renames", base_commit, head],
        ["diff", "--cached", "--name-status", "-z", "--find-renames", "HEAD"],
        ["diff", "--name-status", "-z", "--find-renames", "HEAD"],
    )
    for args in commands:
        parsed = parse_name_status_z(git(root, args).stdout)
        for path, status_value in parsed:
            if path and not generated_path(path):
                statuses.setdefault(path, set()).add(status_value)
    untracked = git(root, ["ls-files", "--others", "--exclude-standard", "-z"]).stdout
    untracked_paths: list[str] = []
    for path in untracked.split("\0"):
        if path and not generated_path(path):
            statuses.setdefault(path, set()).add("??")
            if not sensitive_path(path):
                untracked_paths.append(path)

    sensitive_count = sum(1 for path in statuses if sensitive_path(path))
    visible = [
        {"path": path, "status": ",".join(sorted(statuses[path]))}
        for path in sorted(statuses)
        if not sensitive_path(path)
    ]
    all_path_output = git(root, ["ls-files", "-z"]).stdout
    all_paths = {path for path in all_path_output.split("\0") if path and not generated_path(path)}
    all_paths.update(
        path
        for path in git(root, ["ls-files", "--others", "--exclude-standard", "-z"]).stdout.split("\0")
        if path and not generated_path(path)
    )
    all_paths.update(statuses)
    return {
        "base": base,
        "base_commit": base_commit,
        "head": head,
        "changes": visible,
        "changed_paths": [entry["path"] for entry in visible],
        "untracked_paths": sorted(untracked_paths),
        "all_paths": sorted(path for path in all_paths if not sensitive_path(path)),
        "sensitive_changes_count": sensitive_count,
        "statuses": {path: sorted(values) for path, values in statuses.items() if not sensitive_path(path)},
    }


def select_profiles(config: dict[str, Any], changes: dict[str, Any]) -> dict[str, Any]:
    paths = changes["changed_paths"]
    common = [
        path
        for path in paths
        if any(matches(path, pattern) for pattern in config.get("common_patterns", []))
    ]
    unclassified: list[str] = []
    selected: set[str] = set()
    for path in paths:
        path_profiles: set[str] = set()
        for rule in config.get("path_rules", []):
            if any(matches(path, pattern) for pattern in rule.get("patterns", [])):
                path_profiles.update(rule.get("profiles", []))
        if path_profiles:
            selected.update(path_profiles)
        else:
            unclassified.append(path)
    widened = bool(common or unclassified or changes["sensitive_changes_count"])
    if widened:
        selected = {"all"}
    return {
        "profiles": sorted(selected),
        "unclassified_paths": unclassified,
        "common_change_paths": common,
        "widened_to_all": widened,
        "requires_full_integration": widened,
    }


def changed_for_check(check: dict[str, Any], changes: dict[str, Any], root: Path) -> list[str]:
    patterns = check.get("changed_patterns", [])
    matches_changed = [
        path for path in changes["changed_paths"] if any(matches(path, pattern) for pattern in patterns)
    ]
    if matches_changed:
        return sorted(set(matches_changed))
    # Explicit profile execution remains useful when no file in that scope changed:
    # check the existing source files for the profile instead of silently doing nothing.
    input_patterns = [p for p in check.get("inputs", []) if p != "@changed"]
    fallback = [
        path
        for path in changes["all_paths"]
        if any(matches(path, pattern) for pattern in input_patterns)
        and (not patterns or any(matches(path, pattern) for pattern in patterns))
    ]
    return sorted(set(fallback))


def expand_argv(
    check_id: str,
    check: dict[str, Any],
    changes: dict[str, Any],
    root: Path,
    package: str | None = None,
) -> list[str]:
    raw_argv = check.get("package_argv") if package is not None else check.get("argv")
    if not isinstance(raw_argv, list):
        raise InputError(f"check {check_id} does not support package selection")
    files = changed_for_check(check, changes, root)
    python_files = [path for path in files if path.endswith(".py") and (root / path).is_file()]
    shell_files = [path for path in files if path.endswith(".sh") and (root / path).is_file()]
    command: list[str] = []
    for token in raw_argv:
        if token == "{base_commit}":
            command.append(changes["base_commit"])
        elif token == "{changed_paths}":
            visible = [f":(literal){path}" for path in changes["changed_paths"]]
            command.extend(visible if visible else [":(literal)__dev_check_no_visible_paths__"])
        elif token == "{untracked_paths}":
            command.extend(changes.get("untracked_paths", []))
        elif token == "{changed_python}":
            command.extend(python_files)
        elif token == "{changed_shell}":
            command.extend(shell_files)
        elif token == "{package}":
            if package is None:
                raise InputError(f"check {check_id} needs a package name")
            command.append(package)
        else:
            command.append(str(token))
    return command


def resolved_cwd(root: Path, check: dict[str, Any]) -> Path:
    raw = Path(check["cwd"])
    cwd = (root / raw).resolve()
    try:
        cwd.relative_to(root.resolve())
    except ValueError as exc:
        raise InputError(f"check cwd escapes the repository: {check['cwd']}") from exc
    if not cwd.is_dir():
        raise InputError(f"check cwd is missing: {check['cwd']}")
    return cwd


def describe_check(
    check_id: str,
    check: dict[str, Any],
    changes: dict[str, Any],
    root: Path,
    package: str | None = None,
) -> dict[str, Any]:
    argv = expand_argv(check_id, check, changes, root, package)
    return {
        "check_id": check_id,
        "label": check.get("label", check_id),
        "argv": argv,
        "cwd": str(resolved_cwd(root, check)),
        "inputs": list(check.get("inputs", [])),
        "reuse": bool(check.get("reuse", False)) and REUSE_ALLOWLIST.get(check_id) == check.get("reuse_kind"),
        "timeout_seconds": int(check.get("timeout_seconds", 600)),
    }


def workspace_packages(root: Path) -> set[str]:
    try:
        manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise InputError("cannot read Cargo workspace members") from exc
    members = manifest.get("workspace", {}).get("members", [])
    packages: set[str] = set()
    for pattern in members:
        for manifest_path in root.glob(f"{pattern}/Cargo.toml"):
            try:
                source = manifest_path.read_text(encoding="utf-8")
            except OSError as exc:
                raise InputError(f"cannot read Cargo package manifest: {manifest_path.relative_to(root)}") from exc
            package_table = extract_toml_table(source, "package")
            if package_table is None:
                raise InputError(f"Cargo workspace manifest has no [package] table: {manifest_path.relative_to(root)}")
            try:
                package = tomllib.loads("[package]\n" + package_table).get("package", {})
            except tomllib.TOMLDecodeError as exc:
                raise InputError(f"cannot parse Cargo package table: {manifest_path.relative_to(root)}") from exc
            name = package.get("name")
            if not isinstance(name, str) or not name:
                raise InputError(f"Cargo package manifest has no package name: {manifest_path.relative_to(root)}")
            packages.add(name)
    return packages


def extract_toml_table(source: str, wanted: str) -> str | None:
    """Return one explicit TOML table without parsing unrelated TOML 1.1 values."""
    current: str | None = None
    captured: list[str] = []
    quote: str | None = None
    triple = False
    square_depth = 0
    curly_depth = 0
    lines = source.splitlines(keepends=True)
    for line in lines:
        plain_line = line.rstrip("\r\n")
        is_header = quote is None and square_depth == 0 and curly_depth == 0
        if is_header:
            array_match = re.fullmatch(r"\s*\[\[([^\[\]]+)\]\]\s*(?:#.*)?", plain_line)
            table_match = re.fullmatch(r"\s*\[([^\[\]]+)\]\s*(?:#.*)?", plain_line)
            match = array_match or table_match
            if match:
                if current == wanted:
                    return "".join(captured)
                current = match.group(1).strip()
                captured = []
                continue
        if current == wanted:
            captured.append(line)
        index = 0
        while index < len(plain_line):
            char = plain_line[index]
            if quote is not None:
                if triple and plain_line.startswith(quote * 3, index):
                    quote = None
                    triple = False
                    index += 3
                    continue
                if not triple and quote == '"' and char == "\\":
                    index += 2
                    continue
                if not triple and char == quote:
                    quote = None
                index += 1
                continue
            if char == "#":
                break
            if char in {'"', "'"}:
                if plain_line.startswith(char * 3, index):
                    quote = char
                    triple = True
                    index += 3
                    continue
                quote = char
                triple = False
            elif char == "[":
                square_depth += 1
            elif char == "]":
                square_depth = max(0, square_depth - 1)
            elif char == "{":
                curly_depth += 1
            elif char == "}":
                curly_depth = max(0, curly_depth - 1)
            index += 1
    if current == wanted:
        return "".join(captured)
    return None


def validate_package(profile: str, stage: str, package: str | None, root: Path) -> None:
    if package is None:
        return
    if profile != "rust" or stage != "focused":
        raise InputError("--package is supported only for --profile rust --stage focused")
    if package not in workspace_packages(root):
        raise InputError(f"not a Cargo workspace package: {package}")


def compare_versions(found: str, requirement: dict[str, Any]) -> str | None:
    def parts(version: str) -> tuple[int, ...]:
        numbers = re.findall(r"\d+", version)
        return tuple(int(part) for part in numbers) or (0,)

    exact = requirement.get("exact")
    if exact and found != exact:
        return f"needs {exact}, found {found}"
    minimum = requirement.get("minimum")
    if minimum and parts(found) < parts(str(minimum)):
        return f"needs at least {minimum}, found {found}"
    maximum = requirement.get("maximum_exclusive")
    if maximum and parts(found) >= parts(str(maximum)):
        return f"needs below {maximum}, found {found}"
    return None


def tool_version(name: str, spec: dict[str, Any], timeout: float = 8.0) -> dict[str, Any]:
    if spec.get("presence_only"):
        executable = shutil.which(name)
        if executable is None:
            return {"name": name, "ok": False, "error": f"missing tool: {name}", "path": None}
        return {"name": name, "ok": True, "path": executable, "version": None}
    argv = [str(part) for part in spec["version_argv"]]
    executable = shutil.which(argv[0])
    if executable is None:
        return {"name": name, "ok": False, "error": f"missing tool: {name}", "path": None}
    env = os.environ.copy()
    env.update({str(k): str(v) for k, v in spec.get("environment", {}).items()})
    if Path(argv[0]).name in {"cargo", "rustc", "rustfmt"}:
        env["RUSTUP_AUTO_INSTALL"] = "0"
    try:
        result = subprocess.run(
            argv,
            cwd=spec.get("cwd", ROOT),
            env=env,
            text=True,
            encoding="utf-8",
            errors="replace",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return {"name": name, "ok": False, "error": f"could not read {name} version", "path": executable}
    combined = (result.stdout + "\n" + result.stderr).strip()
    match = re.search(spec.get("version_regex", r"^(.+)$"), combined, re.MULTILINE)
    if result.returncode != 0 or match is None:
        return {"name": name, "ok": False, "error": f"could not verify {name} version", "path": executable}
    found = match.group(1)
    mismatch = compare_versions(found, spec)
    if mismatch:
        return {
            "name": name,
            "ok": False,
            "error": f"wrong version for {name}: {mismatch}",
            "path": executable,
            "version": found,
        }
    return {"name": name, "ok": True, "path": executable, "version": found}


def portable_repo_path(root: Path, raw_path: str) -> Path:
    if raw_path == "nt/.venv/bin/python" and os.name == "nt":
        return root / "nt" / ".venv" / "Scripts" / "python.exe"
    return root / raw_path


def check_python_packages(root: Path, requirements: list[dict[str, Any]]) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for item in requirements:
        raw_python = str(item.get("path", "python3"))
        if raw_python == "test-python":
            # Rust Phase0 generators use PYTHON, defaulting to `python`.
            python = shutil.which(os.environ.get("PYTHON") or "python")
        elif raw_python == "python3":
            python = shutil.which("python3")
        else:
            python_path = portable_repo_path(root, raw_python)
            python = str(python_path) if python_path.is_file() else None
        if not python:
            results.append({"ok": False, "error": f"missing Python environment: {raw_python}"})
            continue
        expected_python = item.get("python_version")
        if expected_python:
            try:
                version_result = subprocess.run(
                    [python, "-B", "-c", "import sys; print('.'.join(map(str,sys.version_info[:2])))"],
                    cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                    timeout=10, check=False,
                )
            except (OSError, subprocess.TimeoutExpired):
                version_result = None
            found_python = version_result.stdout.strip() if version_result and version_result.returncode == 0 else None
            results.append({
                "ok": found_python == expected_python, "python": raw_python,
                "package": "Python interpreter", "expected": expected_python,
                "found": found_python, "error": None if found_python == expected_python else "wrong test Python interpreter version",
            })
        for package, expected in item.get("packages", {}).items():
            script = (
                "import importlib.metadata as m,sys; "
                "print(m.version(sys.argv[1]))"
            )
            try:
                result = subprocess.run(
                    [python, "-B", "-c", script, package],
                    cwd=root,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    timeout=10,
                    check=False,
                )
            except (OSError, subprocess.TimeoutExpired):
                result = None
            found = result.stdout.strip() if result and result.returncode == 0 else None
            okay = bool(found) and (found == expected if "." in expected else found.split(".", 1)[0] == expected)
            results.append(
                {
                    "ok": okay,
                    "python": raw_python,
                    "package": package,
                    "expected": expected,
                    "found": found,
                    "error": None if okay else f"missing or wrong package version: {package}",
                }
            )
    return results


def browser_available(browser: str) -> bool:
    if browser != "chromium":
        return False
    browser_root_raw = os.environ.get("PLAYWRIGHT_BROWSERS_PATH")
    browser_root = Path(browser_root_raw).expanduser() if browser_root_raw else Path.home() / ".cache" / "ms-playwright"
    if not browser_root.is_dir() or browser_root.is_symlink():
        return False
    candidates = (
        "chromium-*/chrome-linux/chrome",
        "chromium-*/chrome-linux64/chrome",
        "chromium-*/chrome-win/chrome.exe",
        "chromium-*/chrome-mac/Chromium.app/Contents/MacOS/Chromium",
        "chromium_headless_shell-*/chrome-linux/headless_shell",
        "chromium_headless_shell-*/chrome-linux64/headless_shell",
    )
    return any(any(browser_root.glob(pattern)) for pattern in candidates)


def check_port(name: str, port: int, bind: Any = None) -> dict[str, Any]:
    if not 1 <= port <= 65535:
        return {"name": name, "port": port, "ok": False, "errno": "INVALID_PORT"}
    binder = bind or socket.socket
    sock: socket.socket | None = None
    try:
        sock = binder(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", port))
        return {"name": name, "port": port, "ok": True, "errno": None}
    except OSError as exc:
        error_name = getattr(exc, "errno", None)
        label = errno_name(error_name)
        return {"name": name, "port": port, "ok": False, "errno": label}
    finally:
        if sock is not None:
            sock.close()


def errno_name(value: int | None) -> str:
    import errno

    return errno.errorcode.get(value, f"ERRNO_{value}" if value is not None else "UNKNOWN_ERRNO")


def safe_output_parent(root: Path) -> list[str]:
    errors: list[str] = []
    uid = os.getuid() if hasattr(os, "getuid") else None
    groups = set(os.getgroups()) if hasattr(os, "getgroups") else set()
    if hasattr(os, "getgid"):
        groups.add(os.getgid())
    output = root / "target" / "dev-check"
    components = [root, root / "target", output]
    for index, path in enumerate(components):
        if not path.exists() and not path.is_symlink():
            continue
        try:
            info = path.lstat()
        except OSError:
            errors.append(f"cannot inspect output path: {path.relative_to(root)}")
            continue
        rel = "." if path == root else str(path.relative_to(root))
        if stat.S_ISLNK(info.st_mode):
            errors.append(f"output path is a symlink: {rel}")
            continue
        if not stat.S_ISDIR(info.st_mode):
            errors.append(f"output path is not a directory: {rel}")
            continue
        if uid is not None and info.st_uid != uid:
            errors.append(f"output path has a different owner: {rel}")
        mode = stat.S_IMODE(info.st_mode)
        if index == 2 and mode & 0o077:
            errors.append(f"receipt parent must be private (mode 0700): {rel}")
        elif mode & 0o002 or (mode & 0o020 and info.st_gid not in groups):
            errors.append(f"output parent has unsafe write permissions: {rel}")
        if index < 2 and (mode & 0o300) != 0o300:
            errors.append(f"output parent is not owner-writable/searchable: {rel}")
        if index == 2 and (mode & 0o300) != 0o300:
            errors.append(f"receipt parent is not owner-writable/searchable: {rel}")
    existing_parent = output if output.is_dir() else output.parent if output.parent.is_dir() else root
    if not os.access(existing_parent, os.W_OK | os.X_OK):
        errors.append(f"output parent is not writable: {existing_parent.relative_to(root)}")
    return errors


def normalize_ports(items: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_name: dict[str, dict[str, Any]] = {}
    invalid: list[dict[str, Any]] = []
    for item in items:
        name = str(item["name"])
        mode = str(item.get("mode", "available"))
        raw = os.environ.get(name) if name.isupper() else None
        value: Any = raw if raw not in (None, "") else item.get("default")
        try:
            port = int(value)
        except (TypeError, ValueError):
            invalid.append({"name": name, "names": [name], "port": value, "invalid": True})
            continue
        normalized = {"name": name, "names": [name], "port": port, "mode": mode}
        previous = by_name.get(name)
        if previous is None:
            by_name[name] = normalized
        elif previous["port"] != port or previous["mode"] != mode:
            invalid.append({
                "name": name,
                "names": [name],
                "port": port,
                "invalid": True,
                "reason": f"conflicting port requirements for {name}",
            })

    by_port: dict[int, list[dict[str, Any]]] = {}
    for item in by_name.values():
        by_port.setdefault(item["port"], []).append(item)
    for port, endpoints in by_port.items():
        if len(endpoints) > 1:
            names = sorted(endpoint["name"] for endpoint in endpoints)
            reason = f"configured endpoint port collision: {' and '.join(names)} both use {port}"
            for endpoint in endpoints:
                endpoint["collision"] = reason
    return [*by_name.values(), *invalid]


def selected_ids(config: dict[str, Any], profile: str, stage: str, package: str | None = None) -> list[str]:
    if stage == "integration":
        return []
    profile_data = config["profiles"][profile]
    check_ids = list(profile_data[stage])
    if package is not None:
        check_ids = [
            "cargo-clippy-workspace" if check_id == "cargo-clippy-workspace" else check_id
            for check_id in check_ids
        ]
    return list(dict.fromkeys(check_ids))


def requirement_context(
    config: dict[str, Any],
    profile: str,
    stage: str,
    check_ids: list[str],
    package: str | None = None,
) -> dict[str, Any]:
    requirements: dict[str, Any] = {
        "tools": {"git"},
        "paths": [],
        "required_env": set(),
        "ports": [],
        "docker_daemon": False,
        "docker_compose": False,
        "python_packages": [],
        "browsers": set(),
        "pin_manifest": False,
        "prepare": {},
        "database_url_exact": None,
    }
    if stage == "integration":
        req_profile = "all" if profile == "docs" else profile
        req = config.get("integration_requirements", {}).get(req_profile, {})
        requirements["tools"].update(req.get("tools", []))
        requirements["paths"].extend(req.get("paths", []))
        requirements["required_env"].update(req.get("required_env", []))
        requirements["ports"].extend(req.get("ports", []))
        requirements["docker_daemon"] = bool(req.get("docker_daemon"))
        requirements["docker_compose"] = bool(req.get("docker_compose"))
        requirements["python_packages"].extend(req.get("python_packages", []))
        requirements["browsers"].update(req.get("browsers", []))
        requirements["database_url_exact"] = req.get("database_url_exact")
    for check_id in check_ids:
        check = config["checks"][check_id]
        requirements["tools"].update(check.get("tools", []))
        requirements["paths"].extend(check.get("required_paths", []))
        requirements["required_env"].update(check.get("required_env", []))
        requirements["ports"].extend(check.get("ports", []))
        requirements["python_packages"].extend(check.get("python_packages", []))
        requirements["browsers"].update(check.get("required_browsers", []))
        requirements["pin_manifest"] = requirements["pin_manifest"] or bool(check.get("pin_manifest"))
        requirements["docker_daemon"] = requirements["docker_daemon"] or bool(check.get("docker_daemon"))
        if check.get("prepare_if_missing"):
            for path in check.get("required_paths", []):
                requirements["prepare"][path] = check["prepare_if_missing"]
        requirements["prepare"].update(check.get("prepare_by_path", {}))
    if profile == "rust" and stage == "focused" and package is not None and "cargo-test-workspace" in check_ids:
        # A deliberately small, source-backed map for package test suites with
        # DB and/or Phase0 prerequisites. Unlisted pure packages need neither.
        package_requirements = config.get("rust_package_prerequisites", {}).get(package, {})
        requirements["tools"].update(package_requirements.get("tools", []))
        requirements["paths"].extend(package_requirements.get("paths", []))
        requirements["required_env"].update(package_requirements.get("required_env", []))
        requirements["ports"].extend(package_requirements.get("ports", []))
        requirements["docker_daemon"] = requirements["docker_daemon"] or bool(package_requirements.get("docker_daemon"))
        requirements["docker_compose"] = requirements["docker_compose"] or bool(package_requirements.get("docker_compose"))
        requirements["python_packages"].extend(package_requirements.get("python_packages", []))
        requirements["prepare"].update(package_requirements.get("prepare", {}))
        requirements["database_url_exact"] = package_requirements.get("database_url_exact")
    requirements["tools"] = sorted(requirements["tools"])
    if requirements["pin_manifest"]:
        requirements["tools"] = sorted(set(requirements["tools"]) | {"bash", "grep", "head", "sed", "tr"})
    requirements["paths"] = list(dict.fromkeys(requirements["paths"]))
    requirements["required_env"] = sorted(requirements["required_env"])
    requirements["ports"] = normalize_ports(requirements["ports"])
    requirements["python_packages"] = list(requirements["python_packages"])
    requirements["browsers"] = sorted(requirements["browsers"])
    return requirements


def loopback_database_ok() -> bool:
    raw = os.environ.get("DATABASE_URL", "")
    expected_port = os.environ.get("LAGRANGE_QA_DB_PORT", "")
    # Parse only non-secret routing fields; never include the URL in a diagnostic.
    try:
        from urllib.parse import urlsplit

        parsed = urlsplit(raw)
        port = parsed.port
        return (
            parsed.scheme in {"postgres", "postgresql"}
            and parsed.hostname in {"127.0.0.1", "localhost", "::1"}
            and port is not None
            and str(port) == expected_port
        )
    except ValueError:
        return False


def preflight_profile(
    root: Path,
    config: dict[str, Any],
    profile: str,
    stage: str,
    environment: str = "local",
    package: str | None = None,
    *,
    port_bind: Any = None,
    check_ids_override: list[str] | None = None,
    heavy_lock_dir: str | Path | None = None,
) -> dict[str, Any]:
    selected = selected_ids(config, profile, stage, package)
    check_ids = list(check_ids_override) if check_ids_override is not None else selected
    if any(check_id not in selected for check_id in check_ids):
        raise InputError("preflight check selection must belong to the selected profile and stage")
    requirements = requirement_context(config, profile, stage, check_ids, package)
    errors: list[str] = []
    observations: list[dict[str, Any]] = []
    tool_facts: dict[str, Any] = {}

    if (
        local_heavy_execution(environment)
        and stage == "focused"
        and set(check_ids) & HEAVY_CHECK_IDS
    ):
        try:
            heavy_lock_directory(heavy_lock_dir)
        except InputError as exc:
            errors.append(str(exc))
            observations.append({"kind": "heavy-slot", "status": "BLOCKED", "message": str(exc)})
        else:
            observations.append({"kind": "heavy-slot", "status": "READY", "scope": "user-wide"})

    for tool_name in requirements["tools"]:
        spec = config.get("tools", {}).get(tool_name)
        if not isinstance(spec, dict):
            errors.append(f"tool requirement is not configured: {tool_name}")
            observations.append({"kind": "tool", "name": tool_name, "status": "INVALID"})
            continue
        result = tool_version(tool_name, {**spec, "cwd": root})
        tool_facts[tool_name] = {key: result.get(key) for key in ("path", "version", "ok")}
        observations.append({"kind": "tool", **result})
        if not result["ok"]:
            errors.append(result["error"])

    missing_env = [name for name in requirements["required_env"] if not os.environ.get(name)]
    if missing_env:
        errors.append("missing environment variables: " + ", ".join(missing_env))
    for name in requirements["required_env"]:
        observations.append({"kind": "environment", "name": name, "present": name not in missing_env})
    if requirements["database_url_exact"] and os.environ.get("DATABASE_URL") != requirements["database_url_exact"]:
        errors.append("DATABASE_URL must match the fixed intraday disposable QA contract")
        observations.append({"kind": "environment", "name": "DATABASE_URL", "present": "DATABASE_URL" not in missing_env, "valid": False})
    if (
        "DATABASE_URL" in requirements["required_env"]
        and "LAGRANGE_QA_DB_PORT" in requirements["required_env"]
        and "DATABASE_URL" not in missing_env
        and "LAGRANGE_QA_DB_PORT" not in missing_env
        and not loopback_database_ok()
    ):
        errors.append("DATABASE_URL must point to the disposable loopback port in LAGRANGE_QA_DB_PORT")
        observations.append({"kind": "environment", "name": "DATABASE_URL", "present": True, "valid": False})

    for raw_path in requirements["paths"]:
        path = portable_repo_path(root, raw_path)
        exists = path.exists()
        observations.append({"kind": "path", "path": raw_path, "present": exists})
        if not exists:
            prep = requirements["prepare"].get(raw_path)
            detail = f"missing prerequisite path: {raw_path}"
            if prep:
                detail += f"; prepare with: {prep}"
            errors.append(detail)

    for item in requirements["python_packages"]:
        for result in check_python_packages(root, [item]):
            observations.append({"kind": "python-package", **result})
            if not result.get("ok"):
                package_name = result.get("package", "environment")
                prepare = next(
                    (
                        check.get("prepare_if_missing")
                        for check in config["checks"].values()
                        if check.get("prepare_if_missing") and item in check.get("python_packages", [])
                    ),
                    None,
                )
                detail = result.get("error") or f"missing package: {package_name}"
                if prepare:
                    detail += f"; prepare with: {prepare}"
                errors.append(detail)

    for browser in requirements["browsers"]:
        present = browser_available(browser)
        observations.append({"kind": "browser", "name": browser, "present": present})
        if not present:
            errors.append("missing Playwright Chromium; prepare with: npx playwright install --with-deps chromium (cwd apps/web)")

    for port_item in requirements["ports"]:
        if port_item.get("collision"):
            errors.append(port_item["collision"])
            observations.append({"kind": "port", **port_item, "ok": False, "errno": "ENDPOINT_PORT_COLLISION"})
            continue
        if port_item.get("invalid"):
            errors.append(port_item.get("reason", f"invalid port for {port_item['name']}"))
            observations.append({"kind": "port", **port_item, "ok": False, "errno": "INVALID_PORT"})
            continue
        if port_item.get("mode") in {"configured-endpoint", "reachable-endpoint"}:
            okay = (
                "DATABASE_URL" not in missing_env
                and "LAGRANGE_QA_DB_PORT" not in missing_env
                and loopback_database_ok()
                and os.environ.get("LAGRANGE_QA_DB_PORT") == str(port_item["port"])
                and (not requirements["database_url_exact"] or os.environ.get("DATABASE_URL") == requirements["database_url_exact"])
            )
            result = {
                "name": "/".join(port_item["names"]),
                "port": port_item["port"],
                "ok": okay,
                "errno": None if okay else "ENDPOINT_NOT_CONFIGURED",
                "mode": port_item["mode"],
            }
            if okay and port_item["mode"] == "reachable-endpoint":
                from urllib.parse import urlsplit

                try:
                    with socket.create_connection(
                        (urlsplit(os.environ["DATABASE_URL"]).hostname, port_item["port"]),
                        timeout=1,
                    ):
                        pass
                except OSError as exc:
                    result["ok"] = False
                    result["errno"] = errno_name(exc.errno)
            observations.append({"kind": "port", **result})
            if not result["ok"]:
                errors.append(f"database endpoint port {result['port']} is not configured as the disposable loopback endpoint")
                if result["errno"] != "ENDPOINT_NOT_CONFIGURED":
                    errors[-1] = f"disposable database endpoint port {result['port']} is unreachable: observed errno {result['errno']}"
            continue
        result = check_port("/".join(port_item["names"]), port_item["port"], bind=port_bind)
        observations.append({"kind": "port", **result})
        if not result["ok"]:
            if result["errno"] == "EADDRINUSE":
                errors.append(f"port {result['port']} unavailable: observed errno EADDRINUSE")
            else:
                errors.append(f"port {result['port']} bind denied or failed: observed errno {result['errno']}")

    if requirements["docker_daemon"] and "docker" in requirements["tools"]:
        docker = shutil.which("docker")
        if docker:
            try:
                result = subprocess.run(
                    ["docker", "info", "--format", "{{.ServerVersion}}"],
                    cwd=root,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    timeout=8,
                    check=False,
                )
            except (OSError, subprocess.TimeoutExpired):
                result = None
            okay = bool(result and result.returncode == 0 and result.stdout.strip())
            observations.append({"kind": "docker-daemon", "available": okay})
            if not okay:
                exit_code = result.returncode if result is not None else "unavailable"
                errors.append(f"Docker daemon unavailable (docker info result {exit_code})")
            elif requirements.get("docker_compose"):
                try:
                    compose_result = subprocess.run(
                        ["docker", "compose", "version"],
                        cwd=root,
                        text=True,
                        encoding="utf-8",
                        errors="replace",
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        timeout=8,
                        check=False,
                    )
                except (OSError, subprocess.TimeoutExpired):
                    compose_result = None
                compose_ok = bool(compose_result and compose_result.returncode == 0)
                observations.append({"kind": "docker-compose", "available": compose_ok})
                if not compose_ok:
                    code = compose_result.returncode if compose_result is not None else "unavailable"
                    errors.append(f"Docker Compose plugin unavailable (docker compose version result {code})")

    output_errors = safe_output_parent(root)
    for error in output_errors:
        observations.append({"kind": "output-path", "status": "BLOCKED", "message": error})
        errors.append(error)
    observations.append({"kind": "output-path", "status": "SAFE", "path": "target/dev-check"} if not output_errors else {"kind": "output-path", "status": "BLOCKED", "path": "target/dev-check"})

    if requirements["pin_manifest"]:
        pin_script = root / "scripts" / "check-pins.sh"
        if not pin_script.is_file():
            errors.append("missing scripts/check-pins.sh for manifest-only pin validation")
            observations.append({"kind": "pin-manifest", "status": "MISSING"})
        else:
            try:
                result = subprocess.run(
                    ["bash", str(pin_script), "--manifest-only"],
                    cwd=root,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    timeout=20,
                    check=False,
                )
            except (OSError, subprocess.TimeoutExpired):
                result = None
            okay = bool(result and result.returncode == 0)
            observations.append({"kind": "pin-manifest", "status": "PASS" if okay else "FAILED"})
            if not okay:
                status = result.returncode if result is not None else "unavailable"
                errors.append(f"check-pins.sh --manifest-only failed (exit {status})")

    return {
        "status": "READY" if not errors else "BLOCKED",
        "profile": profile,
        "stage": stage,
        "environment": environment,
        "checks_not_run": True,
        "errors": errors,
        "observations": observations,
        "tool_facts": tool_facts,
        "check_ids": check_ids,
    }


def input_paths_for_check(
    check: dict[str, Any], changes: dict[str, Any]
) -> list[str]:
    patterns = check.get("inputs", [])
    selected: set[str] = set()
    for pattern in patterns:
        if pattern == "@changed":
            selected.update(changes["changed_paths"])
            continue
        for path in changes["all_paths"]:
            if matches(path, pattern):
                selected.add(path)
        # Keep missing/deleted path names as tombstones when they are in scope.
        for path in changes["changed_paths"]:
            if matches(path, pattern):
                selected.add(path)
    return sorted(path for path in selected if not sensitive_path(path) and not generated_path(path))


def safe_environment_facts(names: list[str]) -> dict[str, Any]:
    facts: dict[str, Any] = {}
    path_or_url_names = {
        "PATH",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "PYTHONPATH",
        "NPM_CONFIG_REGISTRY",
        "UV_INDEX_URL",
        "PLAYWRIGHT_BROWSERS_PATH",
        "NODE_OPTIONS",
    }
    for name in sorted(set(names)):
        value = os.environ.get(name)
        if SECRET_ENV_NAME.search(name):
            facts[name] = {"present": value is not None, "value": "<redacted>"}
        else:
            fact = {
                "present": value is not None,
                "sha256": hashlib.sha256(value.encode("utf-8")).hexdigest() if value is not None else None,
            }
            if value is not None and name not in path_or_url_names:
                fact["value"] = value
            facts[name] = fact
    return facts


def safe_tool_facts(check: dict[str, Any], tool_facts: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for name in sorted(set(check.get("tools", []))):
        fact = tool_facts.get(name)
        if isinstance(fact, dict):
            result[name] = {key: fact.get(key) for key in ("path", "version", "ok")}
    return result


def safe_fingerprint_environment(
    check: dict[str, Any],
    argv: list[str] | None = None,
    *,
    environment: str = "local",
    heavy: bool = False,
) -> dict[str, Any]:
    names = sorted(FINGERPRINT_ENV | set(check.get("fingerprint_env", [])))
    facts = safe_environment_facts(names)
    if argv and Path(argv[0]).name in {"cargo", "rustc", "rustfmt"}:
        facts["RUSTUP_AUTO_INSTALL"] = {
            "present": True,
            "value": "0",
            "sha256": hashlib.sha256(b"0").hexdigest(),
            "dispatcher_override": True,
        }
    if argv and is_cargo_formatter(argv):
        facts["RUSTFMT"] = {"present": False, "sha256": None, "dispatcher_override": True}
    if heavy and argv and Path(argv[0]).name == "cargo":
        facts["CARGO_BUILD_JOBS"] = {
            "present": True,
            "value": "2",
            "sha256": hashlib.sha256(b"2").hexdigest(),
            "dispatcher_override": True,
        }
    return facts


def is_cargo_formatter(argv: list[str]) -> bool:
    arguments = argv[1:]
    if arguments and arguments[0].startswith("+"):
        arguments = arguments[1:]
    return bool(argv and Path(argv[0]).name == "cargo" and arguments and arguments[0] == "fmt")


def command_environment(argv: list[str], *, environment: str = "local", heavy: bool = False) -> dict[str, str]:
    env = os.environ.copy()
    if argv and Path(argv[0]).name in {"cargo", "rustc", "rustfmt"}:
        env["RUSTUP_AUTO_INSTALL"] = "0"
    if is_cargo_formatter(argv):
        # cargo-fmt otherwise accepts an arbitrary executable instead of the probed toolchain.
        env.pop("RUSTFMT", None)
    if heavy and argv and Path(argv[0]).name == "cargo":
        env["CARGO_BUILD_JOBS"] = "2"
    return env


def _output_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return str(value)


def safe_diagnostic(
    root: Path,
    *,
    stdout: Any = "",
    stderr: Any = "",
    message: str | None = None,
) -> tuple[bytes, bool]:
    sections: list[str] = []
    if message:
        sections.append(message)
    stderr_text = _output_text(stderr)
    stdout_text = _output_text(stdout)
    if stderr_text.strip():
        sections.append("stderr:\n" + stderr_text.rstrip())
    if stdout_text.strip():
        sections.append("stdout:\n" + stdout_text.rstrip())
    text = "\n\n".join(sections) or "No command diagnostic output was produced."

    for name, value in os.environ.items():
        if SECRET_ENV_NAME.search(name) and value:
            text = text.replace(value, "[REDACTED]")
    text = re.sub(
        r"(?i)\b(api[_ -]?key|secret|token|password|authorization|cookie)\b(\s*[:=]\s*)([^\s,;]+)",
        r"\1\2[REDACTED]",
        text,
    )
    text = re.sub(
        r"(?i)(?:[a-z]:)?(?:[^\s\"'<>]*[\\/])*data[\\/](?:phase0|raw|curated)(?:[\\/][^\s\"'<>]*)?",
        "[provider-data-path]",
        text,
    )
    for relative in ("data/phase0", "data/raw", "data/curated"):
        text = text.replace(str(root / relative), "[provider-data-path]")
        text = text.replace(relative, "[provider-data-path]")
    text = re.sub(
        r"(?i)(?:[a-z]:)?(?:[^\s\"'<>]*[\\/])?(?:\.aws|\.ssh|secrets?|credentials?|private-keys)(?:[\\/][^\s\"'<>]*)?",
        "[private-path]",
        text,
    )
    encoded = text.encode("utf-8", errors="replace")
    if len(encoded) <= DIAGNOSTIC_MAX_BYTES:
        return encoded, False
    marker = b"\n[diagnostic truncated]\n"
    clipped = encoded[: DIAGNOSTIC_MAX_BYTES - len(marker)]
    return clipped + marker, True


def file_fingerprint(root: Path, paths: list[str]) -> tuple[list[dict[str, Any]], bool]:
    result: list[dict[str, Any]] = []
    coverage_safe = True
    for relative in paths:
        posix = PurePosixPath(relative)
        if posix.is_absolute() or not posix.parts or any(part in {"", ".", ".."} for part in posix.parts):
            result.append({"path": relative, "state": "ambiguous-path"})
            coverage_safe = False
            continue
        path = root.joinpath(*posix.parts)
        current = root
        missing = False
        ambiguous: str | None = None
        info: os.stat_result | None = None
        for index, part in enumerate(posix.parts):
            current = current / part
            try:
                component_info = current.lstat()
            except FileNotFoundError:
                missing = True
                break
            except OSError as exc:
                ambiguous = f"unreadable:{errno_name(exc.errno)}"
                break
            if stat.S_ISLNK(component_info.st_mode):
                ambiguous = "ambiguous-symlink"
                break
            if index < len(posix.parts) - 1 and not stat.S_ISDIR(component_info.st_mode):
                ambiguous = "non-directory-ancestor"
                break
            if index == len(posix.parts) - 1:
                info = component_info
        if missing:
            # Deleted inputs are stable tombstones and do not make coverage ambiguous.
            result.append({"path": relative, "state": "missing"})
            continue
        if ambiguous is not None:
            result.append({"path": relative, "state": ambiguous})
            coverage_safe = False
            continue
        if info is None or not stat.S_ISREG(info.st_mode):
            result.append({"path": relative, "state": "non-regular"})
            coverage_safe = False
            continue
        digest = hashlib.sha256()
        descriptor: int | None = None
        file_size = info.st_size
        try:
            if os.open in getattr(os, "supports_dir_fd", set()) and hasattr(os, "O_NOFOLLOW"):
                directory_flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | os.O_NOFOLLOW
                descriptor = os.open(root, directory_flags)
                for part in posix.parts[:-1]:
                    next_descriptor = os.open(part, directory_flags, dir_fd=descriptor)
                    os.close(descriptor)
                    descriptor = next_descriptor
                file_descriptor = os.open(posix.parts[-1], os.O_RDONLY | os.O_NOFOLLOW, dir_fd=descriptor)
            else:
                file_descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
            opened_info = os.fstat(file_descriptor)
            if not stat.S_ISREG(opened_info.st_mode):
                os.close(file_descriptor)
                result.append({"path": relative, "state": "non-regular"})
                coverage_safe = False
                continue
            file_size = opened_info.st_size
            with os.fdopen(file_descriptor, "rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
        except OSError as exc:
            if exc.errno == errno.ELOOP:
                state = "ambiguous-symlink"
            elif exc.errno == errno.ENOTDIR:
                state = "non-directory-ancestor"
            elif exc.errno == errno.ENOENT:
                result.append({"path": relative, "state": "missing"})
                continue
            else:
                state = f"unreadable:{errno_name(exc.errno)}"
            result.append({"path": relative, "state": state})
            coverage_safe = False
        finally:
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
        if not result or result[-1].get("path") != relative:
            result.append({"path": relative, "state": "file", "sha256": digest.hexdigest(), "size": file_size})
        elif result[-1].get("state") == "file":
            result[-1] = {"path": relative, "state": "file", "sha256": digest.hexdigest(), "size": file_size}
    return result, coverage_safe


def fingerprint(
    root: Path,
    check_id: str,
    check: dict[str, Any],
    argv: list[str],
    cwd: Path,
    changes: dict[str, Any],
    tool_facts: dict[str, Any],
    environment: str = "local",
    heavy: bool = False,
) -> tuple[str, list[str], bool]:
    paths = input_paths_for_check(check, changes)
    dispatcher_records, dispatcher_safe = file_fingerprint(root, ["scripts/qa/dev-check.py"])
    profiles_records, profiles_safe = file_fingerprint(root, ["scripts/qa/check-profiles.json"])
    dispatcher_hash = dispatcher_records[0].get("sha256", dispatcher_records[0]["state"])
    profiles_hash = profiles_records[0].get("sha256", profiles_records[0]["state"])
    file_records, coverage_safe = file_fingerprint(root, paths)
    coverage_safe = coverage_safe and dispatcher_safe and profiles_safe
    data = {
        "schema_version": SCHEMA_VERSION,
        "dispatcher_hash": dispatcher_hash,
        "profiles_hash": profiles_hash,
        "check_id": check_id,
        "reuse_kind": check.get("reuse_kind"),
        "argv": argv,
        "cwd": str(cwd),
        "files": file_records,
        "dispatcher_state": dispatcher_records[0],
        "profiles_state": profiles_records[0],
        "input_coverage_safe": coverage_safe,
        "tools": {name: tool_facts.get(name) for name in check.get("tools", [])},
        "environment_name": environment,
        "environment": safe_fingerprint_environment(
            check, argv, environment=environment, heavy=heavy
        ),
        "platform": {"system": platform.system(), "release": platform.release(), "machine": platform.machine()},
    }
    encoded = json.dumps(data, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest(), paths, coverage_safe


def secure_private_dir(path: Path, *, must_be_private: bool = True) -> bool:
    try:
        info = path.lstat()
    except OSError:
        return False
    if not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode):
        return False
    if hasattr(os, "getuid") and info.st_uid != os.getuid():
        return False
    mode = stat.S_IMODE(info.st_mode)
    return not (mode & 0o077) if must_be_private else not bool(mode & 0o002)


def secure_receipt_file(path: Path) -> bool:
    try:
        info = path.lstat()
    except OSError:
        return False
    if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode) or info.st_size > 1024 * 1024:
        return False
    if hasattr(os, "getuid") and info.st_uid != os.getuid():
        return False
    return not bool(stat.S_IMODE(info.st_mode) & 0o077)


def make_run_dir(root: Path, run_id: str | None = None) -> tuple[str, Path]:
    parent = root / "target" / "dev-check"
    parent_errors = safe_output_parent(root)
    if parent_errors:
        raise InputError(parent_errors[0])
    target = root / "target"
    if not target.exists():
        try:
            target.mkdir(mode=0o700)
        except OSError as exc:
            raise InputError("cannot create target output parent") from exc
    if not parent.exists():
        try:
            parent.mkdir(mode=0o700)
        except OSError as exc:
            raise InputError("cannot create private receipt parent") from exc
        if not secure_private_dir(parent):
            raise InputError("new receipt parent is not private")
    chosen_id = run_id or (dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:12])
    if not re.fullmatch(r"[A-Za-z0-9TzZ-]{8,80}", chosen_id):
        raise InputError("run id contains unsupported characters")
    run_dir = parent / chosen_id
    try:
        run_dir.mkdir(mode=0o700)
    except FileExistsError as exc:
        raise InputError("receipt run directory already exists") from exc
    except OSError as exc:
        raise InputError("cannot create private receipt run directory") from exc
    if not secure_private_dir(run_dir):
        raise InputError("new receipt run directory is not private")
    return chosen_id, run_dir


def write_private_json(path: Path, data: dict[str, Any]) -> None:
    temp_path = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    fd = os.open(temp_path, flags, 0o600)
    try:
        if hasattr(os, "fchmod"):
            os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(data, stream, sort_keys=True, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        if path.exists() or path.is_symlink():
            raise InputError(f"receipt path already exists: {path.name}")
        os.replace(temp_path, path)
    except BaseException:
        try:
            temp_path.unlink()
        except OSError:
            pass
        raise


def write_private_bytes(path: Path, data: bytes) -> None:
    temp_path = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    fd = os.open(temp_path, flags, 0o600)
    try:
        if hasattr(os, "fchmod"):
            os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        if path.exists() or path.is_symlink():
            raise InputError(f"receipt path already exists: {path.name}")
        os.replace(temp_path, path)
    except BaseException:
        try:
            temp_path.unlink()
        except OSError:
            pass
        raise


def receipt_valid(
    receipt: Any,
    *,
    check_id: str,
    expected_fingerprint: str,
    run_manifest: Any,
    expected_tool_facts: dict[str, Any] | None = None,
    expected_environment_facts: dict[str, Any] | None = None,
) -> bool:
    if not isinstance(receipt, dict) or not isinstance(run_manifest, dict):
        return False
    if receipt.get("schema_version") != SCHEMA_VERSION or receipt.get("check_id") != check_id:
        return False
    if receipt.get("input_fingerprint") != expected_fingerprint or not re.fullmatch(
        r"[0-9a-f]{64}", str(receipt.get("input_fingerprint", ""))
    ):
        return False
    if receipt.get("verdict") != "PASS" or receipt.get("original_exit_code") != 0:
        return False
    if receipt.get("reuse_eligible") is not True or receipt.get("timed_out") or receipt.get("inputs_stable") is not True:
        return False
    if receipt.get("runtime_dependent") is not False or receipt.get("partial") is not False:
        return False
    if not isinstance(receipt.get("tool_facts"), dict) or not isinstance(receipt.get("environment_facts"), dict):
        return False
    if expected_tool_facts is not None and receipt["tool_facts"] != expected_tool_facts:
        return False
    if expected_environment_facts is not None and receipt["environment_facts"] != expected_environment_facts:
        return False
    if not isinstance(receipt.get("target_head"), str) or not re.fullmatch(r"[0-9a-f]{40,64}", receipt["target_head"]):
        return False
    if not isinstance(receipt.get("argv"), list) or not isinstance(receipt.get("cwd"), str):
        return False
    if any(not isinstance(argument, str) for argument in receipt["argv"]):
        return False
    if not isinstance(receipt.get("started_at"), str) or not isinstance(receipt.get("completed_at"), str):
        return False
    if not isinstance(receipt.get("duration_ms"), int) or receipt["duration_ms"] < 0:
        return False
    evidence = receipt.get("evidence_paths")
    if not isinstance(evidence, list) or len(evidence) > 41:
        return False
    if any(not isinstance(path, str) or Path(path).is_absolute() or ".." in PurePosixPath(path).parts for path in evidence):
        return False
    if receipt.get("reuse_kind") != REUSE_ALLOWLIST.get(check_id):
        return False
    if run_manifest.get("schema_version") != SCHEMA_VERSION or run_manifest.get("complete") is not True:
        return False
    if run_manifest.get("status") != "PASS" or run_manifest.get("run_id") != receipt.get("run_id"):
        return False
    manifest_checks = run_manifest.get("results")
    if not isinstance(manifest_checks, list) or not any(
        isinstance(item, dict) and item.get("check_id") == check_id and item.get("verdict") == "PASS"
        for item in manifest_checks
    ):
        return False
    if run_manifest.get("target_head") != receipt.get("target_head"):
        return False
    return True


def find_reusable_receipt(
    root: Path,
    check_id: str,
    expected_fingerprint: str,
    *,
    expected_tool_facts: dict[str, Any] | None = None,
    expected_environment_facts: dict[str, Any] | None = None,
) -> dict[str, Any] | None:
    parent = root / "target" / "dev-check"
    if not secure_private_dir(parent):
        return None
    candidates: list[tuple[str, dict[str, Any], Path]] = []
    try:
        directories = list(parent.iterdir())
    except OSError:
        return None
    for run_dir in directories:
        if not secure_private_dir(run_dir):
            continue
        receipt_path = run_dir / f"{check_id}.json"
        manifest_path = run_dir / "run.json"
        if not secure_receipt_file(receipt_path) or not secure_receipt_file(manifest_path):
            continue
        try:
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            run_manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if receipt_valid(
            receipt,
            check_id=check_id,
            expected_fingerprint=expected_fingerprint,
            run_manifest=run_manifest,
            expected_tool_facts=expected_tool_facts,
            expected_environment_facts=expected_environment_facts,
        ):
            candidates.append((str(receipt.get("completed_at", "")), receipt, receipt_path))
    if not candidates:
        return None
    _, receipt, receipt_path = max(candidates, key=lambda item: item[0])
    return {"receipt": receipt, "path": receipt_path}


def execute_check(
    root: Path,
    check_id: str,
    check: dict[str, Any],
    argv: list[str],
    cwd: Path,
    changes: dict[str, Any],
    tool_facts: dict[str, Any],
    *,
    reuse: bool,
    environment: str = "local",
    heavy_lock_dir: str | Path | None = None,
) -> dict[str, Any]:
    heavy = local_heavy_execution(environment) and check_id in HEAVY_CHECK_IDS
    started = utc_now()
    monotonic_start = time.monotonic()
    before_fingerprint, paths, coverage_safe = fingerprint(
        root, check_id, check, argv, cwd, changes, tool_facts, environment, heavy
    )
    receipt_tool_facts = safe_tool_facts(check, tool_facts)
    receipt_environment_facts = safe_fingerprint_environment(
        check, argv, environment=environment, heavy=heavy
    )
    evidence = paths[:40]
    if len(paths) > 40:
        evidence.append(f"… {len(paths) - 40} additional input paths omitted")
    if not coverage_safe:
        file_records, _ = file_fingerprint(root, paths)
        unsafe = [f"{item['path']} ({item['state']})" for item in file_records if item.get("state") not in {"file", "missing"}]
        diagnostic, truncated = safe_diagnostic(
            root,
            message="Input coverage is ambiguous; selected command was not started: " + ", ".join(unsafe[:40]),
        )
        return {
            "check_id": check_id,
            "label": check.get("label", check_id),
            "verdict": "FAILED_INPUT_COVERAGE",
            "original_exit_code": None,
            "input_fingerprint": before_fingerprint,
            "inputs_stable": False,
            "started_at": started,
            "completed_at": utc_now(),
            "duration_ms": round((time.monotonic() - monotonic_start) * 1000),
            "argv": argv,
            "cwd": str(cwd),
            "evidence_paths": evidence,
            "original_receipt": None,
            "original_target_head": None,
            "runtime_dependent": bool(check.get("runtime_dependent", False)),
            "timed_out": False,
            "start_error": None,
            "reuse_eligible": False,
            "paths": paths,
            "tool_facts": receipt_tool_facts,
            "environment_facts": receipt_environment_facts,
            "diagnostic_bytes": diagnostic,
            "diagnostic_truncated": truncated,
        }
    allowed_reuse = (
        reuse
        and bool(check.get("reuse"))
        and REUSE_ALLOWLIST.get(check_id) == check.get("reuse_kind")
        and not bool(check.get("runtime_dependent", False))
    )
    if allowed_reuse:
        prior = find_reusable_receipt(
            root,
            check_id,
            before_fingerprint,
            expected_tool_facts=receipt_tool_facts,
            expected_environment_facts=receipt_environment_facts,
        )
        if prior is not None:
            after_fingerprint, _, after_coverage_safe = fingerprint(
                root,
                check_id,
                check,
                argv,
                cwd,
                refresh_path_universe(root, changes),
                tool_facts,
                environment,
                heavy,
            )
            if after_fingerprint == before_fingerprint and after_coverage_safe:
                receipt = prior["receipt"]
                return {
                    "check_id": check_id,
                    "label": check.get("label", check_id),
                    "verdict": "REUSED",
                    "original_exit_code": 0,
                    "input_fingerprint": before_fingerprint,
                    "inputs_stable": True,
                    "started_at": started,
                    "completed_at": utc_now(),
                    "duration_ms": round((time.monotonic() - monotonic_start) * 1000),
                    "argv": argv,
                    "cwd": str(cwd),
                    "evidence_paths": [str(prior["path"].relative_to(root))],
                    "original_receipt": str(prior["path"].relative_to(root)),
                    "original_target_head": receipt["target_head"],
                    "runtime_dependent": False,
                    "timed_out": False,
                    "reuse_eligible": False,
                    "paths": paths,
                    "tool_facts": receipt_tool_facts,
                    "environment_facts": receipt_environment_facts,
                }

    timed_out = False
    start_error: str | None = None
    heavy_slot_blocked = False
    heavy_slot_message: str | None = None
    stdout = ""
    stderr = ""
    original_exit: int | None
    try:
        effective_env = command_environment(argv, environment=environment, heavy=heavy)
        if heavy:
            result = run_child_with_heavy_slot(
                argv,
                cwd=cwd,
                env=effective_env,
                lock_directory=heavy_lock_dir,
            )
        else:
            result = subprocess.run(
                argv,
                cwd=cwd,
                text=True,
                encoding="utf-8",
                errors="replace",
                env=effective_env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=float(check.get("timeout_seconds", 600)),
                check=False,
            )
        original_exit = result.returncode
        stdout = result.stdout
        stderr = result.stderr
    except InputError as exc:
        original_exit = None
        heavy_slot_blocked = True
        heavy_slot_message = str(exc)
    except subprocess.TimeoutExpired as exc:
        original_exit = None
        timed_out = True
        stdout = _output_text(exc.stdout)
        stderr = _output_text(exc.stderr)
    except OSError as exc:
        original_exit = None
        start_error = errno_name(exc.errno)
    current_changes = refresh_path_universe(root, changes)
    after_fingerprint, _, after_coverage_safe = fingerprint(
        root, check_id, check, argv, cwd, current_changes, tool_facts, environment, heavy
    )
    inputs_stable = after_fingerprint == before_fingerprint and after_coverage_safe
    if start_error:
        verdict = "FAILED_START"
    elif heavy_slot_blocked:
        verdict = "BLOCKED_HEAVY_SLOT"
    elif timed_out:
        verdict = "FAILED_TIMEOUT"
    elif not inputs_stable:
        verdict = "FAILED_INPUT_CHANGED"
    elif original_exit != 0:
        verdict = "FAILED"
    else:
        verdict = "PASS"
    diagnostic_bytes: bytes | None = None
    diagnostic_truncated = False
    if verdict not in {"PASS"}:
        if start_error:
            message = f"Command could not be started (errno {start_error})."
        elif heavy_slot_blocked:
            message = heavy_slot_message or "shared local heavy slot is unavailable; no child was started"
        elif timed_out:
            message = f"Command timed out after {check.get('timeout_seconds', 600)} seconds."
        elif not inputs_stable:
            message = "Selected input coverage changed while the command was running."
        else:
            message = f"Command exited with status {original_exit}."
        diagnostic_bytes, diagnostic_truncated = safe_diagnostic(
            root, stdout=stdout, stderr=stderr, message=message
        )
    return {
        "check_id": check_id,
        "label": check.get("label", check_id),
        "verdict": verdict,
        "original_exit_code": original_exit,
        "input_fingerprint": before_fingerprint,
        "inputs_stable": inputs_stable,
        "started_at": started,
        "completed_at": utc_now(),
        "duration_ms": round((time.monotonic() - monotonic_start) * 1000),
        "argv": argv,
        "cwd": str(cwd),
        "evidence_paths": evidence,
        "original_receipt": None,
        "original_target_head": None,
        "runtime_dependent": bool(check.get("runtime_dependent", False)),
        "timed_out": timed_out,
        "heavy_slot_blocked": heavy_slot_blocked,
        "start_error": start_error,
        "reuse_eligible": False,
        "paths": paths,
        "tool_facts": receipt_tool_facts,
        "environment_facts": receipt_environment_facts,
        "diagnostic_bytes": diagnostic_bytes,
        "diagnostic_truncated": diagnostic_truncated,
    }


def refresh_path_universe(root: Path, changes: dict[str, Any]) -> dict[str, Any]:
    tracked = git(root, ["ls-files", "-z"]).stdout
    untracked = git(root, ["ls-files", "--others", "--exclude-standard", "-z"]).stdout
    paths = set(changes["all_paths"])
    paths.update(path for path in tracked.split("\0") if path and not generated_path(path))
    paths.update(path for path in untracked.split("\0") if path and not generated_path(path))
    changed = set(changes["changed_paths"])
    changed.update(path for path in untracked.split("\0") if path and not generated_path(path) and not sensitive_path(path))
    return {**changes, "all_paths": sorted(path for path in paths if not sensitive_path(path)), "changed_paths": sorted(changed)}


def persist_run(
    root: Path,
    run_id: str,
    run_dir: Path,
    profile: str,
    stage: str,
    environment: str,
    head: str,
    results: list[dict[str, Any]],
    selected: list[str],
    complete: bool,
) -> None:
    overall_pass = complete and len(results) == len(selected) and all(
        result["verdict"] in {"PASS", "REUSED"} for result in results
    )
    has_skips = any(result["verdict"] == "SKIPPED_NO_APPLICABLE_INPUT" for result in results)
    status = "PASS" if overall_pass else "PASS_WITH_SKIPS" if complete and has_skips else "FAILED" if complete else "PARTIAL"
    for result in results:
        diagnostic_metadata = None
        diagnostic_bytes = result.pop("diagnostic_bytes", None)
        if isinstance(diagnostic_bytes, bytes):
            diagnostic_path = run_dir / f"{result['check_id']}.diagnostic.log"
            write_private_bytes(diagnostic_path, diagnostic_bytes)
            diagnostic_metadata = {
                "path": str(diagnostic_path.relative_to(root)),
                "sha256": hashlib.sha256(diagnostic_bytes).hexdigest(),
                "size_bytes": len(diagnostic_bytes),
                "truncated": bool(result.pop("diagnostic_truncated", False)),
            }
            result["diagnostic"] = diagnostic_metadata
        receipt = {
            "schema_version": SCHEMA_VERSION,
            "run_id": run_id,
            "check_id": result["check_id"],
            "target_head": head,
            "input_fingerprint": result["input_fingerprint"],
            "argv": result["argv"],
            "cwd": result["cwd"],
            "environment": environment,
            "started_at": result["started_at"],
            "completed_at": result["completed_at"],
            "duration_ms": result["duration_ms"],
            "original_exit_code": result["original_exit_code"],
            "verdict": result["verdict"],
            "inputs_stable": result["inputs_stable"],
            "timed_out": result["timed_out"],
            "runtime_dependent": result["runtime_dependent"],
            "partial": not complete,
            "reuse_eligible": bool(
                overall_pass
                and result["verdict"] == "PASS"
                and REUSE_ALLOWLIST.get(result["check_id"]) is not None
                and result["inputs_stable"]
                and not result["timed_out"]
                and not result["runtime_dependent"]
            ),
            "reuse_kind": REUSE_ALLOWLIST.get(result["check_id"]),
            "evidence_paths": result["evidence_paths"],
            "tool_facts": result.get("tool_facts", {}),
            "environment_facts": result.get("environment_facts", {}),
            "diagnostic": diagnostic_metadata,
            "original_receipt": result.get("original_receipt"),
            "original_target_head": result.get("original_target_head"),
            "start_error": result.get("start_error"),
        }
        write_private_json(run_dir / f"{result['check_id']}.json", receipt)
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "profile": profile,
        "stage": stage,
        "environment": environment,
        "target_head": head,
        "started_at": results[0]["started_at"] if results else utc_now(),
        "completed_at": utc_now(),
        "complete": complete,
        "status": status,
        "selected_checks": selected,
        "results": [
            {
                "check_id": item["check_id"],
                "verdict": item["verdict"],
                "original_exit_code": item["original_exit_code"],
                "original_receipt": item.get("original_receipt"),
            }
            for item in results
        ],
    }
    # The manifest is written last. A crash before it leaves untrusted partial receipts.
    write_private_json(run_dir / "run.json", manifest)


def render_preflight(report: dict[str, Any]) -> str:
    lines = [
        f"PREFLIGHT {report['status']} — {report['profile']}/{report['stage']} ({report['environment']})",
        "No tests or application checks were run by preflight.",
    ]
    for observation in report["observations"]:
        kind = observation.get("kind")
        if kind == "tool":
            if observation.get("ok"):
                version = observation.get("version")
                lines.append(f"  tool {observation['name']}: {version if version else 'available'}")
            else:
                lines.append(f"  tool {observation['name']}: {observation.get('error', 'unavailable')}")
        elif kind == "environment":
            lines.append(f"  env {observation['name']}: {'present' if observation.get('present') else 'missing'}")
        elif kind == "port":
            state = (
                "reachable disposable endpoint" if observation.get("mode") == "reachable-endpoint"
                else "configured endpoint" if observation.get("mode") == "configured-endpoint"
                else "available for startup"
            ) if observation.get("ok") else f"blocked ({observation.get('errno')})"
            lines.append(f"  port {observation['port']}: {state}")
        elif kind == "docker-daemon":
            lines.append(f"  Docker daemon: {'available' if observation.get('available') else 'unavailable'}")
        elif kind == "docker-compose":
            lines.append(f"  Docker Compose plugin: {'available' if observation.get('available') else 'unavailable'}")
        elif kind in {"path", "browser", "pin-manifest", "python-package", "output-path"}:
            if observation.get("status") == "SAFE":
                lines.append("  receipt path: target/dev-check is private")
            elif observation.get("status") == "BLOCKED":
                lines.append(f"  receipt path: {observation.get('message', 'unsafe')}")
            elif observation.get("kind") == "path":
                lines.append(f"  path {observation['path']}: {'present' if observation['present'] else 'missing'}")
            elif observation.get("kind") == "browser":
                lines.append(f"  browser {observation['name']}: {'present' if observation['present'] else 'missing'}")
            elif observation.get("kind") == "pin-manifest":
                lines.append(f"  check-pins --manifest-only: {observation['status']}")
            elif observation.get("kind") == "python-package":
                package = observation.get("package", "Python environment")
                state = f"{observation.get('found')}" if observation.get("ok") else "missing or wrong version"
                lines.append(f"  Python package {package}: {state}")
        elif kind == "heavy-slot":
            lines.append(
                "  local heavy slot: ready (shared user-wide advisory lock)"
                if observation.get("status") == "READY"
                else f"  local heavy slot: {observation.get('message', 'blocked')}"
            )
    if report["errors"]:
        lines.append("Preflight blockers:")
        lines.extend(f"  - {error}" for error in report["errors"])
    else:
        lines.append("Preflight pass is not a test pass.")
    return "\n".join(lines)


def render_plan(plan: dict[str, Any]) -> str:
    lines = [f"Plan base: {plan['base']} ({plan['base_commit']})", f"Target HEAD: {plan['head']}"]
    if plan["changes"]:
        lines.append("Changed paths:")
        lines.extend(f"  {item['status']:>7}  {item['path']}" for item in plan["changes"])
    else:
        lines.append("Changed paths: none")
    if plan["sensitive_changes_count"]:
        lines.append(f"Sensitive path changes excluded from output/fingerprints: {plan['sensitive_changes_count']}")
    if plan["unclassified_paths"]:
        lines.append("Unclassified paths (coverage widened):")
        lines.extend(f"  {path}" for path in plan["unclassified_paths"])
    if not plan["profiles"]:
        lines.append("No profile selected; no paths changed. Final hosted CI still applies to confirmed commits.")
    else:
        lines.append("Selected profiles:")
        for profile in plan["profiles"]:
            lines.append(f"  {profile}")
            for stage in ("fast", "focused"):
                if stage == "focused" and profile == "rust":
                    lines.append("    focused: requires an explicit local --package; full workspace commands are hosted integration")
                    continue
                if stage == "focused" and profile == "all":
                    lines.append("    focused: hosted integration only; broad local focused execution is blocked")
                    continue
                for check in plan["checks"][profile][stage]:
                    lines.append(f"    {stage}: {check['check_id']} — {json.dumps(check['argv'], ensure_ascii=False)} (cwd {check['cwd']}, reuse={str(check['reuse']).lower()})")
            for job in plan["integration_plans"].get(profile, []):
                lines.append(f"    integration plan: hosted job {job['job']}")
                for command in job["commands"]:
                    lines.append(f"      command: {command}")
                for prerequisite in job["prerequisites"]:
                    lines.append(f"      requires: {prerequisite}")
    lines.append(
        "Full integration required: " + ("yes" if plan["requires_full_integration"] else "no; required hosted CI still applies")
    )
    return "\n".join(lines)


def build_plan(root: Path, config: dict[str, Any], base: str = "HEAD") -> dict[str, Any]:
    changes = collect_changes(root, base)
    selection = select_profiles(config, changes)
    checks: dict[str, Any] = {}
    for profile in selection["profiles"]:
        checks[profile] = {}
        for stage in ("fast", "focused"):
            if stage == "focused" and profile in {"rust", "all"}:
                checks[profile][stage] = []
                continue
            check_ids = selected_ids(config, profile, stage)
            checks[profile][stage] = [
                describe_check(check_id, config["checks"][check_id], changes, root)
                for check_id in check_ids
            ]
    integration_plans = dict(config.get("integration_plans", {}))
    if "docs" in selection["profiles"]:
        integration_plans["docs"] = config.get("integration_plans", {}).get("all", [])
    return {
        "schema_version": SCHEMA_VERSION,
        "base": base,
        "base_commit": changes["base_commit"],
        "head": changes["head"],
        "changes": changes["changes"],
        "changed_paths": changes["changed_paths"],
        "sensitive_changes_count": changes["sensitive_changes_count"],
        "unclassified_paths": selection["unclassified_paths"],
        "common_change_paths": selection["common_change_paths"],
        "profiles": selection["profiles"],
        "widened_to_all": selection["widened_to_all"],
        "requires_full_integration": selection["requires_full_integration"],
        "checks": checks,
        "integration_plans": integration_plans,
        "local_focused_policy": {
            "rust": "explicit-package-required; workspace integration is hosted-only",
            "all": "hosted-integration-only",
        },
    }


def run_profile(
    root: Path,
    config: dict[str, Any],
    profile: str,
    stage: str,
    base: str = "HEAD",
    environment: str = "local",
    package: str | None = None,
    reuse: bool = False,
    heavy_lock_dir: str | Path | None = None,
) -> tuple[int, dict[str, Any]]:
    if stage == "integration" or (profile == "all" and stage == "focused"):
        raise InputError("broad integration execution is hosted plan-only")
    if profile == "rust" and stage == "focused" and package is None:
        raise InputError("Rust focused execution requires an explicit --package")
    validate_package(profile, stage, package, root)
    changes = collect_changes(root, base)
    check_ids = selected_ids(config, profile, stage, package)
    preflight = preflight_profile(
        root, config, profile, stage, environment, package, heavy_lock_dir=heavy_lock_dir
    )
    if preflight["status"] != "READY":
        return 2, {"status": "BLOCKED", "preflight": preflight, "results": [], "checks_not_run": True}
    run_id, run_dir = make_run_dir(root)
    results: list[dict[str, Any]] = []
    all_complete = True
    for check_id in check_ids:
        check = config["checks"][check_id]
        argv = expand_argv(check_id, check, changes, root, package)
        template_argv = check.get("argv", [])
        dynamic_input = any(token in {"{changed_python}", "{changed_shell}"} for token in template_argv)
        applicable = changed_for_check(check, changes, root)
        applicable_dynamic_files = any(
            token == "{changed_python}"
            and any(path.endswith(".py") and (root / path).is_file() for path in applicable)
            or token == "{changed_shell}"
            and any(path.endswith(".sh") and (root / path).is_file() for path in applicable)
            for token in template_argv
        )
        if dynamic_input and not applicable_dynamic_files:
            # The check has no applicable source files. This is a transparent no-op,
            # not a fabricated PASS, and it cannot supply a reusable receipt.
            result = {
                "check_id": check_id,
                "label": check.get("label", check_id),
                "verdict": "SKIPPED_NO_APPLICABLE_INPUT",
                "original_exit_code": None,
                "input_fingerprint": "",
                "inputs_stable": True,
                "started_at": utc_now(),
                "completed_at": utc_now(),
                "duration_ms": 0,
                "argv": argv,
                "cwd": str(resolved_cwd(root, check)),
                "evidence_paths": [],
                "original_receipt": None,
                "original_target_head": None,
                "runtime_dependent": True,
                "timed_out": False,
                "reuse_eligible": False,
                "paths": [],
            }
        else:
            result = execute_check(
                root,
                check_id,
                check,
                argv,
                resolved_cwd(root, check),
                changes,
                preflight["tool_facts"],
                reuse=reuse and stage != "integration",
                environment=environment,
                heavy_lock_dir=heavy_lock_dir,
            )
        results.append(result)
        print(f"{result['verdict']}: {check_id} — {result['label']}")
        if result["verdict"] not in {"PASS", "REUSED", "SKIPPED_NO_APPLICABLE_INPUT"}:
            all_complete = False
            break
    complete = all_complete and len(results) == len(check_ids)
    head = git(root, ["rev-parse", "--verify", "HEAD"]).stdout.strip()
    persist_run(root, run_id, run_dir, profile, stage, environment, head, results, check_ids, complete)
    for result in results:
        diagnostic = result.get("diagnostic")
        if isinstance(diagnostic, dict):
            print(
                f"Failure diagnostic for {result['check_id']}: {diagnostic['path']} "
                f"(sha256 {diagnostic['sha256']}; truncated={str(diagnostic['truncated']).lower()})"
            )
    has_skips = any(result["verdict"] == "SKIPPED_NO_APPLICABLE_INPUT" for result in results)
    slot_blocked = any(item["verdict"] == "BLOCKED_HEAVY_SLOT" for item in results)
    report = {
        "status": "BLOCKED" if slot_blocked else "PASS_WITH_SKIPS" if complete and has_skips else "PASS" if complete else "FAILED",
        "preflight": preflight,
        "run_id": run_id,
        "receipt_dir": str(run_dir.relative_to(root)),
        "checks_not_run": False,
        "results": results,
        "first_failure": next((item for item in results if item["verdict"] not in {"PASS", "REUSED", "SKIPPED_NO_APPLICABLE_INPUT"}), None),
    }
    return (2 if slot_blocked else 0 if complete else 1), report


def render_integration_plan(config: dict[str, Any], profile: str) -> str:
    plan_profile = "all" if profile == "docs" else profile
    lines = [
        f"INTEGRATION PLAN ONLY — {profile}",
        "No local integration command was started. A plan is not an integration pass.",
    ]
    for job in config.get("integration_plans", {}).get(plan_profile, []):
        lines.append(f"Hosted job: {job['job']}")
        for command in job.get("commands", []):
            lines.append(f"  command: {command}")
        for prerequisite in job.get("prerequisites", []):
            lines.append(f"  prerequisite: {prerequisite}")
    if not config.get("integration_plans", {}).get(plan_profile):
        lines.append("No profile-specific integration command; the repository's required hosted CI still applies.")
    if plan_profile != "all":
        lines.append(
            "Final hosted CI still requires policy, format, clippy, web, workspace-tests, "
            "python-tests, postgres-integration-validation, and the required aggregate."
        )
    return "\n".join(lines)


def build_parser(config: dict[str, Any]) -> argparse.ArgumentParser:
    profiles = sorted(config["profiles"])
    stages = ("fast", "focused", "integration")
    shared = (
        "Defaults: --base is HEAD; --environment is local. Explicit --profile selects that profile's "
        "checks for the requested stage, independent of the caller's current directory; the repository "
        "root is resolved from this script. Integration reports existing hosted commands and prerequisites "
        "without executing local integration."
    )
    parser = argparse.ArgumentParser(
        description="Plan, preflight, and dispatch existing repository checks.",
        epilog=shared,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    commands = parser.add_subparsers(dest="command", required=True)

    plan = commands.add_parser("plan", description=shared, formatter_class=argparse.RawDescriptionHelpFormatter)
    plan.add_argument("--base", default="HEAD", help="base commit or ref (default: HEAD; no fetch)")
    plan.add_argument("--format", choices=("text", "json"), default="text")

    preflight = commands.add_parser("preflight", description=shared, formatter_class=argparse.RawDescriptionHelpFormatter)
    preflight.add_argument("--profile", required=True, choices=profiles)
    preflight.add_argument("--stage", required=True, choices=stages)
    preflight.add_argument("--base", default="HEAD", help="base commit or ref (default: HEAD; no fetch)")
    preflight.add_argument("--environment", choices=("local", "ci"), default="local", help="environment label (default: local)")
    preflight.add_argument("--package", help="actual Cargo workspace member (Rust focused stage only)")
    preflight.add_argument("--check-id", action="append", choices=sorted(config["checks"]), help="narrow this preflight to one selected profile check; repeatable")
    preflight.add_argument("--heavy-lock-dir", help="owner-private mode-0700 directory for the shared local heavy slot")

    run = commands.add_parser("run", description=shared, formatter_class=argparse.RawDescriptionHelpFormatter)
    run.add_argument("--profile", required=True, choices=profiles)
    run.add_argument("--stage", required=True, choices=stages)
    run.add_argument("--base", default="HEAD", help="base commit or ref (default: HEAD; no fetch)")
    run.add_argument("--environment", choices=("local", "ci"), default="local", help="environment label (default: local)")
    run.add_argument("--package", help="actual Cargo workspace member (Rust focused stage only)")
    run.add_argument("--heavy-lock-dir", help="owner-private mode-0700 directory for the shared local heavy slot")
    run.add_argument("--reuse", action="store_true", help="reuse only eligible deterministic receipts")
    return parser


def main(argv: list[str] | None = None) -> int:
    try:
        config = load_config(ROOT)
        parser = build_parser(config)
        args = parser.parse_args(argv)
        if args.command == "plan":
            plan = build_plan(ROOT, config, args.base)
            print(json.dumps(plan, ensure_ascii=False, sort_keys=True, indent=2) if args.format == "json" else render_plan(plan))
            return 0
        if args.profile == "all" and args.stage == "focused" and args.command in {"run", "preflight"}:
            print("BLOCKED: broad local focused execution includes hosted integration work; no local check was started.")
            print(render_integration_plan(config, "all"))
            return 2
        if (
            args.profile == "rust"
            and args.stage == "focused"
            and args.package is None
            and (args.command == "run" or (args.command == "preflight" and not args.check_id))
        ):
            print("BLOCKED: Rust focused local execution requires an explicit --package <workspace-package>.")
            print(render_integration_plan(config, "rust"))
            return 2
        validate_package(args.profile, args.stage, args.package, ROOT)
        if args.command == "preflight":
            resolve_commit(ROOT, args.base)
            if args.check_id and args.stage == "integration":
                raise InputError("--check-id cannot narrow an integration preflight")
            allowed_checks = selected_ids(config, args.profile, args.stage, args.package)
            if args.check_id and any(check_id not in allowed_checks for check_id in args.check_id):
                raise InputError("preflight check selection must belong to the selected profile and stage")
            report = preflight_profile(
                ROOT,
                config,
                args.profile,
                args.stage,
                args.environment,
                args.package,
                check_ids_override=args.check_id,
                heavy_lock_dir=args.heavy_lock_dir,
            )
            print(render_preflight(report))
            return 0 if report["status"] == "READY" else 2
        if args.stage == "integration":
            print(render_integration_plan(config, args.profile))
            return 0
        if args.stage == "focused":
            print(f"Running prerequisite fast stage for {args.profile} before focused checks.")
            fast_status, fast_report = run_profile(
                ROOT,
                config,
                args.profile,
                "fast",
                args.base,
                args.environment,
                None,
                args.reuse,
                args.heavy_lock_dir,
            )
            if fast_report.get("status") == "BLOCKED":
                print(render_preflight(fast_report["preflight"]))
                print("Focused checks were not started because the fast stage preflight is blocked.")
                return fast_status
            print(f"Fast stage receipt directory: {fast_report['receipt_dir']}")
            if fast_report["first_failure"]:
                failure = fast_report["first_failure"]
                print(f"First failure preserved: {failure['check_id']} ({failure['verdict']}, exit {failure['original_exit_code']})")
            if fast_status != 0:
                print("Focused checks were not started because the fast stage failed.")
                return fast_status
            print("Prerequisite fast checks passed.")
        status, report = run_profile(
            ROOT,
            config,
            args.profile,
            args.stage,
            args.base,
            args.environment,
            args.package,
            args.reuse,
            args.heavy_lock_dir,
        )
        if report.get("status") == "BLOCKED":
            print(render_preflight(report["preflight"]))
            print("No selected check command was started.")
            return status
        print(f"Run receipt directory: {report['receipt_dir']}")
        if report["first_failure"]:
            failure = report["first_failure"]
            print(f"First failure preserved: {failure['check_id']} ({failure['verdict']}, exit {failure['original_exit_code']})")
        elif report["status"] == "PASS_WITH_SKIPS":
            print("Selected checks completed with explicit no-applicable-input skips; no PASS was recorded for skipped checks.")
        else:
            print("All selected checks completed. Reused results retain their original receipt and tested revision.")
        return status
    except InputError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
