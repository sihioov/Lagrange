#!/usr/bin/env python3
"""Private, role-owned copies of the one fixed V3 historical base.

No service, DB, provider, permission change to the source, or alternate source.
The caller holds the release lock with the daily cohort stopped. Publication is
exclusive and atomic; an existing partial/different view is never repaired.
Rust still independently verifies its original owner/mode/content/approval gates.
"""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import os
from pathlib import Path
import sys
import uuid

sys.dont_write_bytecode = True


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).parent / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


install = load("daily_base_install", "owner-beta-daily-release.py")
files = load("daily_base_files", "owner-beta-daily-backup-files.py")
require = files.require

# Exact approved V3 registry pins, not caller-selected artifacts or identities.
# configs/evidence/kis-historical-price-only-v3-approved-artifacts.json
CANDIDATE = "b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d"
MANIFEST = "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6"
BARS = "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe"
BARS_SIZE = 7672914
DIRECTORY = "historical-base"
ROLES = {"producer": 10002, "admitter": 10003}
LEAVES = {"manifest.json", "bars.ndjson"}


def digest(value):
    return "sha256:" + hashlib.sha256(value).hexdigest()


def validate_bytes(values):
    manifest = install.json_bytes(values["manifest.json"], maximum=2 * 1024 * 1024)
    require(isinstance(manifest, dict) and list(manifest)[-1:] == ["manifest_sha256"],
            "DAILY_BASE_MANIFEST")
    unsigned = {k: v for k, v in manifest.items() if k != "manifest_sha256"}
    require(install.canonical(manifest) + b"\n" == values["manifest.json"] and
            manifest["manifest_sha256"] == MANIFEST == digest(install.canonical(unsigned)) and
            manifest.get("candidate_content_sha256") == "sha256:" + CANDIDATE and
            manifest.get("bars", {}).get("sha256") == BARS and
            manifest["bars"].get("size_bytes") == BARS_SIZE and
            len(values["bars.ndjson"]) == BARS_SIZE and digest(values["bars.ndjson"]) == BARS,
            "DAILY_BASE_BYTES")


@contextlib.contextmanager
def artifact(root, uid):
    """Hold the exact tree and leaf identities through the caller's use."""
    require(set(os.listdir(root)) == {CANDIDATE}, "DAILY_BASE_MEMBERS")
    with install.child_directory(root, CANDIDATE, uid, uid, 0o700) as leaf:
        require(set(os.listdir(leaf)) == LEAVES, "DAILY_BASE_MEMBERS")
        before = {name: files.identity(os.stat(name, dir_fd=leaf, follow_symlinks=False))
                  for name in LEAVES}
        values = {name: install.read_file(leaf, name, uid, uid, 0o600,
                    2 * 1024 * 1024 if name == "manifest.json" else BARS_SIZE)
                  for name in LEAVES}
        validate_bytes(values)
        yield values
        require(set(os.listdir(root)) == {CANDIDATE} and set(os.listdir(leaf)) == LEAVES,
                "DAILY_BASE_CHANGED")
        for name in LEAVES:
            require(files.identity(os.stat(name, dir_fd=leaf, follow_symlinks=False)) == before[name],
                    "DAILY_BASE_CHANGED")
            require(install.read_file(leaf, name, uid, uid, 0o600,
                        2 * 1024 * 1024 if name == "manifest.json" else BARS_SIZE) == values[name],
                    "DAILY_BASE_CHANGED")


@contextlib.contextmanager
def source_artifact(path):
    # Reuse the explicit legacy host-parent exception from backup admission.
    # It applies only to this base parent, never to daily state or a destination.
    parent = Path(path).parent.stat(follow_symlinks=False)
    boundary = (parent.st_uid, 10001, 0o750) if parent.st_uid != 0 else None
    root = files.Root("base", str(path), 10001, 10001, 0o750, parent=boundary)
    with files.open_root(root) as fd:
        # The original root can also retain V2/history. Read only the fixed V3
        # leaf; unrelated siblings are neither copied nor altered.
        with install.child_directory(fd, CANDIDATE, 10001, 10001, 0o700) as leaf:
            require(set(os.listdir(leaf)) == LEAVES, "DAILY_BASE_MEMBERS")
            before = {name: files.identity(os.stat(name, dir_fd=leaf, follow_symlinks=False))
                      for name in LEAVES}
            values = {name: install.read_file(leaf, name, 10001, 10001, 0o600,
                        2 * 1024 * 1024 if name == "manifest.json" else BARS_SIZE)
                      for name in LEAVES}
            validate_bytes(values)
            yield values
            require(set(os.listdir(leaf)) == LEAVES, "DAILY_BASE_CHANGED")
            for name in LEAVES:
                require(files.identity(os.stat(name, dir_fd=leaf, follow_symlinks=False)) == before[name],
                        "DAILY_BASE_CHANGED")
                require(install.read_file(leaf, name, 10001, 10001, 0o600,
                            2 * 1024 * 1024 if name == "manifest.json" else BARS_SIZE) == values[name],
                        "DAILY_BASE_CHANGED")


class NativeRootView:
    """Descriptive root-owned copy; the source context remains entered."""
    def __init__(self, parent, root, values):
        self.parent, self.root, self.values = parent, root, values
        self.identity = files.identity(os.fstat(root))
        self.leaf_identity = files.identity(os.stat(CANDIDATE, dir_fd=root, follow_symlinks=False))
        with install.child_directory(root, CANDIDATE, 0, 0, 0o700) as leaf:
            self.identities = {name: files.identity(os.stat(name, dir_fd=leaf, follow_symlinks=False))
                               for name in LEAVES}
        self.revalidate()

    def revalidate(self):
        install.directory_info(self.root, 0, 0, 0o700)
        require(files.identity(os.fstat(self.root)) == self.identity
                and files.identity(os.stat("historical", dir_fd=self.parent, follow_symlinks=False)) == self.identity
                and set(os.listdir(self.root)) == {CANDIDATE}, "DAILY_BASE_CHANGED")
        require(files.identity(os.stat(CANDIDATE, dir_fd=self.root, follow_symlinks=False)) == self.leaf_identity,
                "DAILY_BASE_CHANGED")
        with install.child_directory(self.root, CANDIDATE, 0, 0, 0o700) as leaf:
            require(set(os.listdir(leaf)) == LEAVES, "DAILY_BASE_CHANGED")
            for name in LEAVES:
                require(files.identity(os.stat(name, dir_fd=leaf, follow_symlinks=False)) == self.identities[name]
                    and install.read_file(leaf, name, 0, 0, 0o600,
                        2 * 1024 * 1024 if name == "manifest.json" else BARS_SIZE) == self.values[name],
                    "DAILY_BASE_CHANGED")


@contextlib.contextmanager
def native_root_view(source, invocation_stage_fd):
    """Retain the real worker original and a nonce-stage Root view through use.

    No persistent role/layout change and no source chmod. Caller owns removal
    only after helper reaping, result import and successful final revalidation.
    """
    install.require_root()
    install.directory_info(invocation_stage_fd, 0, 0, 0o700)
    failure = None
    with source_artifact(source) as values:
        os.mkdir("historical", 0o700, dir_fd=invocation_stage_fd)
        with install.child_directory(invocation_stage_fd, "historical", 0, 0, 0o700) as root:
            os.mkdir(CANDIDATE, 0o700, dir_fd=root)
            with install.child_directory(root, CANDIDATE, 0, 0, 0o700) as leaf:
                for name in sorted(LEAVES):
                    install.create_file(leaf, name, values[name], 0, 0, 0o600)
                os.fsync(leaf)
            os.fsync(root)
            os.fsync(invocation_stage_fd)
            view = NativeRootView(invocation_stage_fd, root, values)
            try:
                yield view
            except BaseException as error:
                failure = error
            # Run both this check and source_artifact's normal after-yield check
            # even when the consumer failed; neither can manufacture success.
            try:
                view.revalidate()
            except BaseException as error:
                failure = error
    if failure is not None:
        raise failure


def verify_fd(parent, expected=None):
    with install.child_directory(parent, DIRECTORY, 0, 10001, 0o750) as views:
        require(set(os.listdir(views)) == set(ROLES), "DAILY_BASE_MEMBERS")
        with contextlib.ExitStack() as stack:
            for role, uid in ROLES.items():
                root = stack.enter_context(install.child_directory(views, role, uid, uid, 0o750))
                values = stack.enter_context(artifact(root, uid))
                require(expected is None or values == expected, "DAILY_BASE_BYTES")


def verify(daily_root):
    install.require_root()
    with install.root_directory(str(daily_root)) as parent:
        verify_fd(parent)


def provision(source, daily_root):
    install.require_root()
    with source_artifact(source) as values, install.root_directory(str(daily_root)) as parent:
        require(not any(name.startswith(".historical-base-stage-") for name in os.listdir(parent)),
                "DAILY_BASE_UNCERTAIN_STAGE")
        if DIRECTORY in os.listdir(parent):
            verify_fd(parent, values)
            return "REUSED"
        stage_name = ".historical-base-stage-" + uuid.uuid4().hex
        os.mkdir(stage_name, 0o700, dir_fd=parent)
        stage = os.open(stage_name, install.DIR_FLAGS, dir_fd=parent)
        published = False
        try:
            os.fchmod(stage, 0o700)
            install.directory_info(stage, 0, 0, 0o700)
            os.mkdir(DIRECTORY, 0o700, dir_fd=stage)
            views = os.open(DIRECTORY, install.DIR_FLAGS, dir_fd=stage)
            try:
                for role, uid in ROLES.items():
                    os.mkdir(role, 0o700, dir_fd=views)
                    root = os.open(role, install.DIR_FLAGS, dir_fd=views)
                    try:
                        os.mkdir(CANDIDATE, 0o700, dir_fd=root)
                        leaf = os.open(CANDIDATE, install.DIR_FLAGS, dir_fd=root)
                        try:
                            for name, data in values.items():
                                install.create_file(leaf, name, data, uid, uid, 0o600)
                            os.fchown(leaf, uid, uid)
                            os.fchmod(leaf, 0o700)
                            os.fsync(leaf)
                        finally:
                            os.close(leaf)
                        os.fchown(root, uid, uid)
                        os.fchmod(root, 0o750)
                        os.fsync(root)
                    finally:
                        os.close(root)
                os.fchown(views, 0, 10001)
                os.fchmod(views, 0o750)
                os.fsync(views)
            finally:
                os.close(views)
            verify_fd(stage, values)
            install.rename_no_replace(stage, DIRECTORY, parent, DIRECTORY)
            published = True
            verify_fd(parent, values)
        finally:
            os.close(stage)
            if published:
                os.rmdir(stage_name, dir_fd=parent)
                os.fsync(parent)
            # Failure retains its root-only stage. Never recursively repair or
            # remove an uncertain destination or reuse an earlier failed stage.
        return "CREATED"
