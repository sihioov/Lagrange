#!/usr/bin/env python3
"""Provider-free proof in a disposable root-owned Linux mount namespace."""
import contextlib
import errno
import fcntl
import importlib.util
import os
from pathlib import Path
import select
import hashlib
import subprocess
import sys
import tempfile
import types
from unittest import mock


def require(condition, label):
    if not condition:
        raise RuntimeError(label)


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


@contextlib.contextmanager
def readonly(original, target):
    subprocess.run(["mount", "--bind", str(original), str(target)], check=True, timeout=10)
    try:
        subprocess.run(["mount", "-o", "remount,bind,ro", str(target)], check=True, timeout=10)
        yield
    finally:
        subprocess.run(["umount", str(target)], check=True, timeout=10)


WRITER = """
import fcntl, os, sys
with open(sys.argv[1], 'r+b', buffering=0) as lock:
    ino = os.fstat(lock.fileno())
    try:
        fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
    except BlockingIOError:
        print(ino.st_dev, ino.st_ino, 'BLOCKED', flush=True)
    else:
        print(ino.st_dev, ino.st_ino, 'HELD', flush=True)
        sys.stdin.buffer.read(1)
"""


@contextlib.contextmanager
def writer(path, expected):
    proc = subprocess.Popen([sys.executable, "-I", "-S", "-B", "-c", WRITER, str(path)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    try:
        require(select.select([proc.stdout], [], [], 10)[0], "WRITER_RESPONSE_TIMEOUT")
        parts = proc.stdout.readline().split()
        original = path.stat()
        require(parts == [str(original.st_dev).encode(), str(original.st_ino).encode(), expected],
                "WRITER_LOCK_STATE")
        yield
    finally:
        proc.communicate(input=b"", timeout=10)
        require(proc.returncode == 0, "WRITER_EXIT")


def refused(action, errors, label):
    try:
        action()
    except errors:
        return
    raise RuntimeError(label)


def run():
    here = Path(__file__).absolute().parent
    root = here.parents[1]
    units = root / "deploy/systemd"
    unit = (units / "lagrange-production-backup.service").read_text()
    dropin = (units / "lagrange-production-backup.nightly-simulation.conf.in").read_text()
    require("ProtectSystem=strict\n" in unit
            and "ReadOnlyPaths=/etc/lagrange /var/lib/lagrange/data\n" in dropin
            and "ReadWritePaths=/opt/lagrange /run /var/lib/lagrange/backup-state/nightly-simulation\n" in dropin,
            "SCHEDULED_HARDENING")
    ops = module("nightly_sandbox_ops", here / "lib/owner-beta-paper-operations.py")
    paper = module("nightly_sandbox_backup", here / "lib/owner-beta-paper-backup.py")
    daily = module("nightly_sandbox_daily", here / "lib/owner-beta-daily-backup.py")
    source = ops._load_local("paper_stop_source", "owner-beta-paper-evidence.py")
    with tempfile.TemporaryDirectory(prefix="lagrange-nightly-sandbox.", dir="/var/lib") as tmp:
        fixture = Path(tmp)
        originals, serving = fixture / "originals", fixture / "serving"
        control = originals / ".control"
        control.mkdir(mode=0o700, parents=True)
        serving.mkdir(mode=0o700)
        workspace = fixture / "state" / "2026-11-25"
        workspace.mkdir(mode=0o700, parents=True)
        archive_lock = control / source.ARCHIVE_LOCK_LEAF
        collector_lock = control / "lock"
        for path in (archive_lock, collector_lock):
            path.touch(mode=0o600)
        with readonly(originals, serving):
            mounted_lock = serving / ".control" / archive_lock.name
            try:
                os.close(os.open(mounted_lock, os.O_RDWR | os.O_NOFOLLOW))
            except OSError as exc:
                require(exc.errno == errno.EROFS, "SERVING_MOUNT_WRONG_REFUSAL")
            else:
                raise RuntimeError("SERVING_MOUNT_WRITABLE")
            with ops._readonly_source_archive_lock(source, serving) as fd:
                info, original = os.fstat(fd), archive_lock.stat()
                require((info.st_dev, info.st_ino) == (original.st_dev, original.st_ino)
                        and fcntl.fcntl(fd, fcntl.F_GETFL) & os.O_ACCMODE == os.O_RDONLY,
                        "ARCHIVE_INODE_OR_ACCESS_MODE")
                with writer(archive_lock, b"HELD"):
                    refused(lambda: fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB),
                            BlockingIOError, "WRITER_AND_BACKUP_BOTH_HELD")
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with writer(archive_lock, b"BLOCKED"):
                    pass
            with writer(archive_lock, b"HELD"):
                pass

            observer = object.__new__(ops._HostStopObserver)
            observer.deadline = ops._ArchiveDeadline()
            observer.locks = []
            try:
                fd = observer.acquire_lock(str(serving / ".control" / collector_lock.name), 0, 0, 0o600,
                                           read_only=True)
                require(fcntl.fcntl(fd, fcntl.F_GETFL) & os.O_ACCMODE == os.O_RDONLY
                        and os.path.samestat(os.fstat(fd), collector_lock.stat()), "DORMANT_LOCK_INODE")
                with writer(collector_lock, b"BLOCKED"):
                    pass
            finally:
                while observer.locks:
                    _, manager = observer.locks.pop()
                    manager.__exit__(None, None, None)

            def read_archive_lock():
                with ops._readonly_source_archive_lock(source, serving):
                    pass

            os.chown(archive_lock, 10001, 0)
            refused(read_archive_lock, source.Rejected, "FOREIGN_LOCK_ACCEPTED")
            os.chown(archive_lock, 0, 0)
            renamed = archive_lock.with_name("retained")
            with mock.patch.object(fcntl, "flock", side_effect=OSError(errno.EOPNOTSUPP, "fixture")) as lock:
                refused(lambda: observer.acquire_lock(str(serving / ".control/lock"), 0, 0, 0o600, read_only=True),
                        (OSError, source.Rejected), "UNSUPPORTED_LOCK_ACCEPTED")
                require(lock.call_count == 1, "UNSUPPORTED_LOCK_RETRIED")
            while observer.locks:
                _, manager = observer.locks.pop()
                manager.__exit__(None, None, None)

            def replace_archive():
                with ops._readonly_source_archive_lock(source, serving) as fd:
                    require(os.path.samestat(os.fstat(fd), archive_lock.stat()), "PREVIOUS_LOCK_IDENTITY")
                    archive_lock.rename(renamed)
                    archive_lock.touch(mode=0o600)
            refused(replace_archive, ops.PaperOperationsError, "RENAMED_LOCK_REUSED")
            archive_lock.unlink()
            archive_lock.symlink_to(renamed)
            refused(read_archive_lock, source.Rejected, "SYMLINK_LOCK_ACCEPTED")

        paper._nightly_disk_space(daily.install, str(workspace), 1)
        config = fixture / "backup.conf"
        config_bytes = b"SYNTHETIC_CONFIG_ONLY\n"
        config.write_bytes(config_bytes)
        config.chmod(0o600)
        args = types.SimpleNamespace(mode="apply", nightly_state_date=workspace.name, config_file=str(config),
            expected_db_first_session="2026-11-26", expected_config_first_session="2026-11-26")
        app = types.SimpleNamespace(commit="1" * 40, release=str(fixture))
        sequencer = fixture / "scripts/ops/preupgrade-simulation-backup.py"
        sequencer.parent.mkdir(parents=True, mode=0o700)
        sequencer_bytes = b"SYNTHETIC_SEQUENCER_ONLY\n"
        sequencer.write_bytes(sequencer_bytes)
        tool = {"DAILY_BACKUP_TOOL_COMMIT": "2" * 40}
        claim = {"schema": "lagrange-nightly-simulation-backup-claim-v1", "date": workspace.name,
                 "sequencer_sha256": "sha256:" + hashlib.sha256(sequencer_bytes).hexdigest(),
                 "started_at_utc": "2026-11-24T17:30:00.000000Z",
                 "config": {"schema": "lagrange-nightly-simulation-backup-v2", "backup_profile": "daily-simulation",
                    "application_commit": app.commit, "tool_commit": tool["DAILY_BACKUP_TOOL_COMMIT"],
                    "config_file": str(config), "config_sha256": "sha256:" + hashlib.sha256(config_bytes).hexdigest(),
                    "expected_db_first_session": args.expected_db_first_session,
                    "expected_config_first_session": args.expected_config_first_session}}
        claim_file = workspace / "claim.json"
        claim_file.write_bytes(daily.files.canonical(claim) + b"\n")
        claim_file.chmod(0o600)
        with mock.patch.object(daily, "NIGHTLY_STATE_ROOT", str(workspace.parent)):
            require(daily.configured_nightly_workspace(args, app, tool) == str(workspace), "CLAIM_PATH")
            refused(lambda: daily.configured_nightly_workspace(
                        args, types.SimpleNamespace(commit="3" * 40, release=app.release), tool),
                    daily.runtime.RuntimeErrorCode, "FOREIGN_CLAIM_ACCEPTED")
            sequencer.write_bytes(b"REPLACED\n")
            refused(lambda: daily.configured_nightly_workspace(args, app, tool),
                    daily.runtime.RuntimeErrorCode, "FOREIGN_SEQUENCER_ACCEPTED")
            sequencer.write_bytes(sequencer_bytes)
            args.nightly_state_date = None
            refused(lambda: daily.configured_nightly_workspace(args, app, tool),
                    daily.runtime.RuntimeErrorCode, "UNCLAIMED_APPLY_ACCEPTED")
            args.mode = "preflight"
            require(daily.configured_nightly_workspace(args, app, tool) == str(workspace.parent), "PREFLIGHT_STATE_ROOT")
        disk_view = fixture / "disk-view"
        disk_view.mkdir(mode=0o700)
        with readonly(workspace, disk_view):
            refused(lambda: paper._nightly_disk_space(daily.install, str(disk_view), 1),
                    paper.PaperBackupError, "READONLY_WORKSPACE_ACCEPTED")
        ops_for_space = types.SimpleNamespace(_STOP_ATTEMPTS={}, _ArchiveDeadline=ops._ArchiveDeadline,
                                             journal=ops.journal)
        # The authority check is covered by the nightly owner. Exercise the
        # actual descriptor-local scratch construction and cleanup here.
        class FixtureContext:
            def revalidate(self):
                pass

        with mock.patch.object(paper, "_operations", return_value=ops_for_space):
            space = paper._CaptureSpace(FixtureContext(), workspace=str(workspace))
        with space:
            require(space.root.parent == workspace and Path(space.tree["path"]).parent == workspace,
                    "SCRATCH_OUTSIDE_CLAIM")
            require(all(path.stat().st_mode & 0o777 == 0o700 for path in (space.root, Path(space.tree["path"]))),
                    "SCRATCH_MODE")
        require(list(workspace.iterdir()) == [claim_file], "SCRATCH_CLEANUP")
    print("CONFIGURED_PAPER_NIGHTLY_SANDBOX: PASS")


if __name__ == "__main__":
    run()
