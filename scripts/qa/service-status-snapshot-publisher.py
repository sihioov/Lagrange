#!/usr/bin/env python3
"""Publish synthetic host snapshots only inside the named WP-5 /tmp tree."""

from __future__ import annotations

import argparse
import copy
import json
import os
import re
import shutil
import stat
import subprocess
import tempfile
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_PATH = REPO_ROOT / "tests/fixtures/service-status/base-input.json"
COMMIT = "0123456789abcdef0123456789abcdef01234567"
PROJECT = "lagrange-service-status-wp5"
ROOT_OWNER = 0
API_GROUP = 10001
ROOT_NAME = "host-snapshot"
SNAPSHOT_NAME = "snapshot.json"
TEMP_ROOT_RE = re.compile(r"^lagrange-service-status-wp5-[A-Za-z0-9][A-Za-z0-9._-]*$")
TIMESTAMP_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T.*(?:Z|[+-]\d{2}:\d{2})$")
QA_IMAGE = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"

# Only this disposable actual directory is mounted writable. The image is already
# cached; its shell performs root filesystem operations, without sudo or a network.
ROOT_OPERATIONS = r'''
set -eu
action="$1"; input="$2"; caller="$3"
finish() {
  code=$?
  printf 'publisher_original_exit=%s memory_peak_bytes=%s memory_events=%s\n' "$code" "$(cat /sys/fs/cgroup/memory.peak)" "$(tr '\n' ',' </sys/fs/cgroup/memory.events)" >&2
  awk '$1=="anon" || $1=="file" || $1=="shmem" {printf "publisher_memory_stat_%s_bytes=%s\n",$1,$2}' /sys/fs/cgroup/memory.stat >&2
}
trap finish EXIT
printf 'publisher_live_caps memory_max_bytes=%s swap_max_bytes=%s cpu_weight=%s\n' "$(cat /sys/fs/cgroup/memory.max)" "$(cat /sys/fs/cgroup/memory.swap.max)" "$(cat /sys/fs/cgroup/cpu.weight)" >&2
test "$(cat /sys/fs/cgroup/memory.max)" = 134217728
test "$(cat /sys/fs/cgroup/memory.swap.max)" = 0
test "$(cat /sys/fs/cgroup/cpu.weight)" = 10
test "$(stat -c '%u:%a' /qa)" = "$caller:700"
case "$action" in
  create|replace-policy)
    test -f "/qa/$input"; test ! -L "/qa/$input"
    test "$(stat -c '%u:%a:%h' "/qa/$input")" = "$caller:600:1"
    test "$(stat -c '%s' "/qa/$input")" -le 8388608
    if test "$action" = create; then
      test ! -e /qa/host-snapshot; test ! -e /qa/handoff
      mkdir -m 0750 /qa/host-snapshot; chown 0:10001 /qa/host-snapshot
      mkdir -m 0770 /qa/handoff; chown 0:10001 /qa/handoff
      destination=/qa/host-snapshot/snapshot.json
    else
      test ! -L /qa/host-snapshot
      test "$(stat -c '%u:%g:%a' /qa/host-snapshot)" = 0:10001:750
      test -f /qa/host-snapshot/snapshot.json; test ! -L /qa/host-snapshot/snapshot.json
      test "$(stat -c '%u:%g:%a:%h' /qa/host-snapshot/snapshot.json)" = 0:10001:640:1
      destination="/qa/host-snapshot/.replacement-$input"
    fi
    (set -C; umask 077; cat "/qa/$input" >"$destination")
    chown 0:10001 "$destination"; chmod 0640 "$destination"; sync -f "$destination"
    if test "$action" = replace-policy; then mv -T "$destination" /qa/host-snapshot/snapshot.json; fi
    sync -f /qa/host-snapshot
    ;;
  read-snapshot)
    test ! -L /qa/host-snapshot; test ! -L /qa/host-snapshot/snapshot.json
    test "$(stat -c '%u:%g:%a' /qa/host-snapshot)" = 0:10001:750
    test "$(stat -c '%u:%g:%a:%h' /qa/host-snapshot/snapshot.json)" = 0:10001:640:1
    stat -c '%u:%g:%a:%h:%i' /qa/host-snapshot/snapshot.json
    cat /qa/host-snapshot/snapshot.json
    ;;
  copy-session|copy-audit)
    test ! -L /qa/handoff
    test "$(stat -c '%u:%g:%a' /qa/handoff)" = 0:10001:770
    if test "$action" = copy-session; then file=owner-session.json; else file=audit-proof.json; fi
    test -f "/qa/handoff/$file"; test ! -L "/qa/handoff/$file"
    test "$(stat -c '%u:%h' "/qa/handoff/$file")" = 10001:1
    cat "/qa/handoff/$file"
    ;;
  cleanup)
    for directory in /qa/host-snapshot /qa/handoff; do
      if test -e "$directory"; then
        test -d "$directory"; test ! -L "$directory"
        test "$(stat -c '%u:%g' "$directory")" = 0:10001
        rm -r -- "$directory"
      fi
    done
    ;;
  *) exit 2 ;;
esac
'''


def checked_paths(root_text: str) -> tuple[Path, Path]:
    root = Path(root_text)
    task_root = root.parent.parent
    if not root.is_absolute() or task_root.parent != Path("/tmp"):
        raise ValueError("snapshot path must be /tmp/<unique-task-root>/actual/host-snapshot")
    if not TEMP_ROOT_RE.fullmatch(task_root.name):
        raise ValueError("snapshot path is outside the declared WP-5 temporary tree")
    if root.name != ROOT_NAME or root.parent.name != "actual":
        raise ValueError("only the actual/host-snapshot directory is allowed")
    if root.parent.parent.is_symlink() or root.parent.is_symlink() or root.is_symlink():
        raise ValueError("temporary snapshot path must not contain symlinks")
    caller_uid_text = os.environ.get("SUDO_UID") if os.geteuid() == 0 else str(os.geteuid())
    if caller_uid_text is None or not caller_uid_text.isdigit() or caller_uid_text == "0":
        raise PermissionError("run this confined publisher through sudo from the owning QA user")
    caller_uid = int(caller_uid_text)
    task_info = task_root.lstat()
    actual_info = root.parent.lstat()
    if (
        not stat.S_ISDIR(task_info.st_mode)
        or task_info.st_uid != caller_uid
        or stat.S_IMODE(task_info.st_mode) != 0o700
        or not stat.S_ISDIR(actual_info.st_mode)
        or actual_info.st_uid != caller_uid
        or stat.S_IMODE(actual_info.st_mode) != 0o700
    ):
        raise PermissionError("temporary task and actual directories must be owner-only QA directories")
    return root, task_root


def root_operation(root: Path, action: str, prepared: Path | None = None) -> bytes:
    """Run a fixed operation in a capped cached container, then preserve its receipt."""
    name = f"lagrange-service-status-wp5-publisher-{os.getpid()}-{time.time_ns()}"
    log = root.parent / "logs" / "publisher-resources.log"
    command = [
        "docker", "run", "--pull=never", "--name", name, "--network", "none",
        "--memory", "128m", "--memory-swap", "128m", "--cpu-shares", "48",
        "--cpus", "0.5", "--read-only", "--user", "0:0", "--cap-drop", "ALL",
        "--cap-add", "CHOWN", "--cap-add", "DAC_OVERRIDE",
        "--security-opt", "no-new-privileges",
        "--mount", f"type=bind,src={root.parent},dst=/qa",
        "--entrypoint", "/bin/sh", "--interactive", QA_IMAGE,
        "-s", "--", action, prepared.name if prepared is not None else "unused", str(os.geteuid()),
    ]
    with log.open("a", encoding="utf-8") as receipt:
        receipt.write(f"publisher_action={action} container={name} time_utc={render_time(datetime.now(timezone.utc))}\n")
        receipt.flush()
        try:
            result = subprocess.run(command, input=ROOT_OPERATIONS.encode(), stdout=subprocess.PIPE,
                                    stderr=receipt, timeout=30, check=False)
            inspected = subprocess.run(
                ["docker", "inspect", "--format", "{{.State.OOMKilled}} {{.State.ExitCode}} {{.State.StartedAt}} {{.State.FinishedAt}}", name],
                stdout=subprocess.PIPE, stderr=receipt, check=True,
            ).stdout.decode().strip()
            receipt.write(f"publisher_container_final={inspected}\n")
            receipt.flush()
            if result.returncode != 0 or not inspected.startswith("false 0 "):
                raise RuntimeError(f"confined publisher {action} failed; original exit={result.returncode}; see {log}")
            return result.stdout
        finally:
            subprocess.run(["docker", "stop", "--time", "5", name], stdout=subprocess.DEVNULL,
                           stderr=receipt, timeout=10, check=False)
            subprocess.run(["docker", "rm", name], stdout=subprocess.DEVNULL,
                           stderr=receipt, timeout=10, check=True)


def container_publish(command: str, root_text: str) -> None:
    root, _ = checked_paths(root_text)
    if command == "cleanup":
        root_operation(root, command)
        return
    if command in ("copy-session", "copy-audit"):
        payload = root_operation(root, command)
        json.loads(payload)
        destination = root.parent / ("owner-session.json" if command == "copy-session" else "audit-proof.json")
        fd = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
        return
    if command == "create":
        source = json.loads(FIXTURE_PATH.read_text(encoding="utf-8"))
        snapshot = copy.deepcopy(source["host_read"]["snapshot"])
        generated = parse_time(snapshot["generated_at"])
        if generated is None:
            raise ValueError("frozen fixture lacks generated_at")
        snapshot = shift_timestamps(snapshot, datetime.now(timezone.utc) - timedelta(seconds=5) - generated)
        set_snapshot_metadata(snapshot)
        before_inode = None
        times = timestamp_projection(snapshot)
    else:
        metadata, content = root_operation(root, "read-snapshot").split(b"\n", 1)
        before_inode = metadata.decode().split(":")[-1]
        snapshot = json.loads(content)
        if snapshot["policy"]["release_commit"] != COMMIT or (
            snapshot["policy"]["profile"], snapshot["policy"]["research_mode"]
        ) != ("owner_daily", "daily"):
            raise ValueError("only the approved same-commit daily snapshot may be replaced")
        times = timestamp_projection(snapshot)
        snapshot["policy"]["profile"] = "owner_daemon"
        snapshot["policy"]["research_mode"] = "daemon"
    fd, text = tempfile.mkstemp(prefix=".snapshot-prepared-", dir=root.parent)
    prepared = Path(text)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(serialize(snapshot)); stream.flush(); os.fsync(stream.fileno())
        root_operation(root, command, prepared)
        metadata, content = root_operation(root, "read-snapshot").split(b"\n", 1)
        observed = json.loads(content)
        if observed != snapshot or timestamp_projection(observed) != times:
            raise ValueError("root snapshot bytes or original timestamps changed")
        if before_inode is not None and metadata.decode().split(":")[-1] == before_inode:
            raise ValueError("policy replacement did not change the directory entry inode")
        proof = {
            "phase": command, "metadata": metadata.decode(), "previous_inode": before_inode,
            "release_commit": observed["policy"]["release_commit"],
            "policy": observed["policy"]["research_mode"], "source_timestamps": times,
        }
        (root.parent / f"snapshot-{command}-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
    finally:
        prepared.unlink(missing_ok=True)


def require_root() -> None:
    if os.geteuid() != 0:
        raise PermissionError("the disposable fixture publisher requires the exact root-owned temporary-tree permission")


def parse_time(value: str) -> datetime | None:
    if not TIMESTAMP_RE.fullmatch(value):
        return None
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    parsed = datetime.fromisoformat(normalized)
    if parsed.tzinfo is None:
        raise ValueError("snapshot timestamps must include an offset")
    return parsed.astimezone(timezone.utc)


def render_time(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")


def shift_timestamps(value: object, delta: timedelta) -> object:
    if isinstance(value, dict):
        return {key: shift_timestamps(child, delta) for key, child in value.items()}
    if isinstance(value, list):
        return [shift_timestamps(child, delta) for child in value]
    if isinstance(value, str):
        parsed = parse_time(value)
        return render_time(parsed + delta) if parsed is not None else value
    return value


def timestamp_projection(value: object, prefix: str = "") -> dict[str, str]:
    found: dict[str, str] = {}
    if isinstance(value, dict):
        for key, child in value.items():
            found.update(timestamp_projection(child, f"{prefix}/{key}"))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found.update(timestamp_projection(child, f"{prefix}/{index}"))
    elif isinstance(value, str) and parse_time(value) is not None:
        found[prefix] = value
    return found


def serialize(snapshot: dict[str, object]) -> bytes:
    return (json.dumps(snapshot, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")


def set_snapshot_metadata(snapshot: dict[str, object]) -> None:
    policy = snapshot["policy"]
    if not isinstance(policy, dict):
        raise ValueError("fixture policy shape changed")
    policy["release_commit"] = COMMIT
    policy["compose_project"] = PROJECT
    policy["profile"] = "owner_daily"
    policy["research_mode"] = "daily"
    policy["owner_access"] = "owner_only"
    policy["quote_transport"] = "market_ws"


def create(root_text: str) -> None:
    require_root()
    root, _ = checked_paths(root_text)
    if not root.parent.is_dir():
        raise ValueError("the declared QA actual directory must exist first")
    source = json.loads(FIXTURE_PATH.read_text(encoding="utf-8"))
    snapshot = copy.deepcopy(source["host_read"]["snapshot"])
    original_generated = parse_time(snapshot["generated_at"])
    if original_generated is None:
        raise ValueError("frozen fixture lacks generated_at")
    target_generated = datetime.now(timezone.utc) - timedelta(seconds=5)
    snapshot = shift_timestamps(snapshot, target_generated - original_generated)
    set_snapshot_metadata(snapshot)

    root.mkdir(mode=0o750)
    os.chown(root, ROOT_OWNER, API_GROUP)
    os.chmod(root, 0o750)
    target = root / SNAPSHOT_NAME
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o640)
    try:
        os.fchown(fd, ROOT_OWNER, API_GROUP)
        os.fchmod(fd, 0o640)
        with os.fdopen(fd, "wb", closefd=False) as stream:
            stream.write(serialize(snapshot))
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(fd)
    dir_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        os.fsync(dir_fd)
    finally:
        os.close(dir_fd)


def replace_policy(root_text: str) -> None:
    require_root()
    root, _ = checked_paths(root_text)
    target = root / SNAPSHOT_NAME
    info = target.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != ROOT_OWNER or info.st_gid != API_GROUP:
        raise ValueError("only the root-owned regular snapshot may be replaced")
    if stat.S_IMODE(info.st_mode) != 0o640:
        raise ValueError("snapshot mode changed from the fixed 0640 contract")
    before = json.loads(target.read_text(encoding="utf-8"))
    if before["policy"]["release_commit"] != COMMIT:
        raise ValueError("snapshot release commit differs from the harness release")
    if (before["policy"]["profile"], before["policy"]["research_mode"]) != ("owner_daily", "daily"):
        raise ValueError("only the approved initial daily snapshot may be replaced")
    timestamps_before = timestamp_projection(before)
    after = copy.deepcopy(before)
    after["policy"]["profile"] = "owner_daemon"
    after["policy"]["research_mode"] = "daemon"
    if timestamp_projection(after) != timestamps_before:
        raise ValueError("source timestamps changed while preparing the atomic policy replacement")

    fd, temporary = tempfile.mkstemp(prefix=".snapshot-replacement-", dir=root)
    temp_path = Path(temporary)
    try:
        os.fchown(fd, ROOT_OWNER, API_GROUP)
        os.fchmod(fd, 0o640)
        with os.fdopen(fd, "wb", closefd=False) as stream:
            stream.write(serialize(after))
            stream.flush()
            os.fsync(stream.fileno())
        os.close(fd)
        os.replace(temp_path, target)
        dir_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        try:
            os.fsync(dir_fd)
        finally:
            os.close(dir_fd)
    finally:
        try:
            os.close(fd)
        except OSError:
            pass
        if temp_path.exists():
            temp_path.unlink()

    replaced = json.loads(target.read_text(encoding="utf-8"))
    replaced_info = target.stat()
    if (
        timestamp_projection(replaced) != timestamps_before
        or replaced["policy"]["release_commit"] != COMMIT
        or (replaced["policy"]["profile"], replaced["policy"]["research_mode"])
        != ("owner_daemon", "daemon")
        or (replaced_info.st_uid, replaced_info.st_gid, stat.S_IMODE(replaced_info.st_mode))
        != (ROOT_OWNER, API_GROUP, 0o640)
    ):
        raise ValueError("atomic replacement did not preserve the accepted file contract")


def cleanup(root_text: str) -> None:
    require_root()
    root, task_root = checked_paths(root_text)
    if root.exists():
        shutil.rmtree(root)
    for name in ("handoff",):
        handoff = task_root / "actual" / name
        if handoff.exists():
            if handoff.is_symlink():
                raise ValueError("handoff directory must not be a symlink")
            shutil.rmtree(handoff)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("create", "replace-policy", "cleanup", "copy-session", "copy-audit"))
    parser.add_argument("root")
    args = parser.parse_args()
    if os.geteuid() != 0:
        container_publish(args.command, args.root)
    elif args.command == "create":
        create(args.root)
    elif args.command == "replace-policy":
        replace_policy(args.root)
    elif args.command == "cleanup":
        cleanup(args.root)
    else:
        raise PermissionError("copy handoffs through the owning QA user")


if __name__ == "__main__":
    main()
