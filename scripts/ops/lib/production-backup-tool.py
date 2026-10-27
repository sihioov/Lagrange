#!/usr/bin/env python3
"""Install and verify an immutable backup tool independently of application images.

The package contains only tracked operations, database verifier and configuration
sources. It never switches current, changes a release, reads a secret, calls
Docker, or starts a unit. Existing or partial installations are never replaced.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import io
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tarfile

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "backup_tool_files", Path(__file__).with_name("owner-beta-daily-release.py"))
assert SPEC and SPEC.loader
files = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(files)

MANIFEST = ".production-backup-tool.json"
SCHEMA = "production-backup-tool-v1"
MAX_FILE = 4 * 1024 * 1024
MAX_PACKAGE = 32 * 1024 * 1024
MAX_MEMBERS = 1000
UNITS = tuple("deploy/systemd/lagrange-production-backup" + suffix
              for suffix in (".service", ".timer", "-verify.service", "-verify.timer"))
REQUIRED = frozenset((
    "scripts/ops/lib/production-backup-tool.py",
    "scripts/ops/lib/owner-beta-daily-release.py",
    "scripts/ops/lib/owner-beta-daily-backup.py",
    "scripts/ops/lib/owner-beta-daily-backup-files.py",
    "scripts/ops/lib/owner-beta-daily-operations.py",
    "scripts/ops/lib/owner-beta-daily-runtime.py",
    "scripts/ops/lib/owner-beta-daily-base.py",
    "scripts/ops/lib/owner-beta-simulation-release.py",
    "scripts/ops/lib/owner-beta-simulation-runtime.py",
    "scripts/ops/lib/owner-beta-simulation-operations.py",
    "scripts/ops/lib/service-status-policy.py",
    "scripts/ops/run-production-backup.sh",
    "scripts/ops/install-production-backup.sh",
    "scripts/ops/transition-production-backup.py",
    "deploy/db/daily-backup-verify.sh",
    "deploy/db/owner-beta-daily-backup-audit.sql", *UNITS))


def require(value):
    if not value:
        raise ValueError("BACKUP_TOOL_INVALID")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def allowed(name):
    if not isinstance(name, str):
        return False
    path = PurePosixPath(name)
    return (str(path) == name and not path.is_absolute()
            and not any(part in {"", ".", ".."} for part in path.parts)
            and (name.startswith(("scripts/ops/", "deploy/db/", "configs/"))
                 or name in UNITS))


def read_manifest(tool, application_commit, *, install_root=Path("/opt/lagrange"),
                  expected_hash=None):
    tool, install_root = Path(tool), Path(install_root)
    require(files.is_commit(tool.name) and files.is_commit(application_commit)
            and tool == install_root / "maintenance" / tool.name)
    raw = files.read_root_file(str(tool / MANIFEST), gid=0, mode=0o600,
                               maximum=MAX_FILE)
    require(expected_hash is None or digest(raw) == expected_hash)
    value = files.json_bytes(raw, MAX_FILE)
    require(isinstance(value, dict) and set(value) == {
        "schema", "tool_commit", "application_commit", "members"})
    require(value["schema"] == SCHEMA and value["tool_commit"] == tool.name
            and value["application_commit"] == application_commit)
    members = value["members"]
    require(isinstance(members, dict) and REQUIRED <= members.keys()
            and len(members) <= MAX_MEMBERS)
    total = 0
    for name, member in members.items():
        require(allowed(name) and isinstance(member, dict)
                and set(member) == {"sha256", "bytes", "mode"}
                and files.is_hash(member["sha256"])
                and type(member["bytes"]) is int and 0 <= member["bytes"] <= MAX_FILE
                and type(member["mode"]) is int and member["mode"] in {0o644, 0o755})
        data = files.read_root_file(str(tool / name), gid=0, mode=member["mode"],
                                   maximum=MAX_FILE)
        require(len(data) == member["bytes"] and digest(data) == member["sha256"])
        total += len(data)
        require(total <= MAX_PACKAGE)
    return digest(raw)


def read_git_package(repository, commit):
    require(files.is_commit(commit))
    env = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C",
           "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
           "GIT_NO_REPLACE_OBJECTS": "1"}
    result = subprocess.run(
        ["/usr/bin/git", "-c", "safe.directory=" + str(repository),
         "-C", str(repository), "archive", "--format=tar", commit,
         "scripts/ops", "deploy/db", "configs", *UNITS],
        env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=30, check=False)
    require(result.returncode == 0 and len(result.stdout) <= MAX_PACKAGE)
    result_members = {}
    with tarfile.open(fileobj=io.BytesIO(result.stdout), mode="r:") as archive:
        for entry in archive:
            if entry.isdir():
                continue
            require(entry.isfile() and allowed(entry.name)
                    and entry.name not in result_members and entry.size <= MAX_FILE
                    and len(result_members) < MAX_MEMBERS)
            stream = archive.extractfile(entry)
            require(stream is not None)
            data = stream.read(MAX_FILE + 1)
            require(len(data) == entry.size)
            result_members[entry.name] = (data, 0o755 if entry.mode & 0o111 else 0o644)
    require(REQUIRED <= result_members.keys())
    return result_members


def publish(members, commit, application_commit, *, install_root=Path("/opt/lagrange")):
    require(files.is_commit(commit) and files.is_commit(application_commit)
            and REQUIRED <= members.keys() and len(members) <= MAX_MEMBERS)
    # Validate everything before creating the destination. No archive extraction
    # primitive, symlink, hardlink, working-tree byte or secret is copied.
    metadata, total = {}, 0
    for name, (data, mode) in members.items():
        require(allowed(name) and type(data) is bytes and len(data) <= MAX_FILE
                and mode in {0o644, 0o755})
        total += len(data)
        require(total <= MAX_PACKAGE)
        metadata[name] = {"sha256": digest(data), "bytes": len(data), "mode": mode}
    tool = Path(install_root) / "maintenance" / commit
    with files.root_directory(str(install_root)) as parent:
        try:
            os.mkdir("maintenance", 0o755, dir_fd=parent)
            os.fsync(parent)
        except FileExistsError:
            pass
    with files.root_directory(str(tool.parent)) as parent:
        os.mkdir(commit, 0o755, dir_fd=parent)  # exclusive; partials are retained
        os.fsync(parent)
    directories = sorted({str(PurePosixPath(n).parent) for n in members})
    for directory in directories:
        cursor = tool
        for part in PurePosixPath(directory).parts:
            with files.root_directory(str(cursor)) as parent:
                try:
                    os.mkdir(part, 0o755, dir_fd=parent)
                    os.fsync(parent)
                except FileExistsError:
                    pass
            cursor /= part
    for name, (data, mode) in members.items():
        path = tool / name
        with files.root_directory(str(path.parent)) as parent:
            files.create_file(parent, path.name, data, 0, 0, mode)
    value = {"schema": SCHEMA, "tool_commit": commit,
             "application_commit": application_commit, "members": metadata}
    with files.root_directory(str(tool)) as parent:
        files.create_file(parent, MANIFEST, files.canonical(value), 0, 0, 0o600)
    return read_manifest(tool, application_commit, install_root=install_root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--commit", required=True)
    parser.add_argument("--application-commit", required=True)
    args = parser.parse_args()
    try:
        require(files.is_commit(args.commit) and files.is_commit(args.application_commit))
        if not args.apply:
            print("BACKUP_TOOL: PLAN immutable maintenance package; application unchanged")
            return 0
        files.require_root()
        os.umask(0o077)
        members = read_git_package(Path(__file__).absolute().parents[3], args.commit)
        result = publish(members, args.commit, args.application_commit)
        print("BACKUP_TOOL: INSTALLED " + result)
        return 0
    except Exception:
        print("BACKUP_TOOL: FAILED_PARTIALS_RETAINED", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
