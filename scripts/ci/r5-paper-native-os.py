#!/usr/bin/env python3
"""Run the eight Root-only, non-database R5 Paper native OS tests on hosted CI."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
import uuid


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/ci/r5-paper-native-os.py"
TARGET_NAME = "job_queue"
PACKAGE_NAME = "job-queue"
LIB_SOURCE = ROOT / "crates/job-queue/src/lib.rs"
FIXTURE_PARENT = Path("/run/r5-paper-checker-integration")
TESTS = (
    "owner_beta::paper::operations::protected::tests::actual_native_protected_original_rejects_replace_link_symlink_and_mode_change",
    "owner_beta::paper::operations::protected::tests::actual_native_dispatch_custody_is_durable_before_dispatch_and_survives_unknown",
    "owner_beta::paper::operations::protected::bound::tests::actual_bound_fixed_index_prevents_retry_and_rejects_missing_or_replaced_originals",
    "owner_beta::paper::operations::protected::bound::tests::actual_bound_publication_fault_boundaries_never_reconstruct_or_resubmit",
    "owner_beta::paper::operations::protected::chain::tests::retained_paper_state_checks_identity_floor_missing_mismatch_and_name_replacement",
    "owner_beta::paper::operations::protected::chain::tests::retained_post_commit_result_loss_or_same_byte_replacement_preserves_unknown_identity",
    "owner_beta::paper::operations::protected::bound::tests::actual_recover_custody_rejects_original_replacement_and_preserves_unknown_dispatch",
    "owner_beta::paper::operations::protected::archive_sandbox::tests::sandbox_kernel_child",
)
ROOT_FAILURE_ERROR_CODES = frozenset({
    "PATH_NOT_ABSOLUTE", "PATH_COMPONENT_MISSING", "PATH_SYMLINK_REJECTED",
    "PATH_CANONICAL_MISMATCH", "PATH_OUTSIDE_ALLOWED_ROOT",
    "ROOT_DRIVER_PIN_INVALID", "CANDIDATE_SHA_MISMATCH", "RUNNER_SOURCE_IDENTITY_DRIFT",
    "NATIVE_SOURCE_CHECKPOINT_MISMATCH", "TEST_ARTIFACT_IDENTITY_DRIFT",
    "TEST_ARTIFACT_METADATA_INVALID", "TEST_ARTIFACT_NOT_ELF", "TEST_ARTIFACT_HASH_MISMATCH",
    "OWNED_COMMAND_START_FAILED", "INTERRUPTED", "CHILD_OUTPUT_LIMIT_EXCEEDED",
    "OWNED_COMMAND_TIMEOUT", "NATIVE_TEST_TIMEOUT", "NAMED_TEST_RESULT_INVALID",
    "RUN_DIRECTORY_INVALID", "FIXTURE_PARENT_PREEXISTED", "FIXTURE_PARENT_METADATA_INVALID",
    "FIXTURE_OWNERSHIP_MISSING", "FIXTURE_PARENT_IDENTITY_DRIFT", "FIXTURE_NONCE_METADATA_INVALID",
    "FIXTURE_NONCE_MISMATCH", "FIXTURE_UNOWNED_ENTRY", "FIXTURE_CHILD_METADATA_INVALID",
    "FIXTURE_CHILD_NODE_INVALID",
})
ROOT_FAILURE_CLEANUP_CODES = frozenset({
    "NOT_CONFIRMED", "PENDING", "PASS", "FIXTURE_OWNERSHIP_MISSING",
    "FIXTURE_PARENT_IDENTITY_DRIFT", "FIXTURE_PARENT_METADATA_INVALID",
    "FIXTURE_NONCE_METADATA_INVALID", "FIXTURE_NONCE_MISMATCH", "FIXTURE_UNOWNED_ENTRY",
    "FIXTURE_CHILD_METADATA_INVALID", "FIXTURE_CHILD_NODE_INVALID",
})
SHA_RE = re.compile(r"[0-9a-f]{40}\Z", re.ASCII)
HASH_RE = re.compile(r"[0-9a-f]{64}\Z", re.ASCII)
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\Z", re.ASCII)
MAX_BUILD_LOG = 16 * 1024 * 1024
MAX_CHILD_LOG = 1024 * 1024
BUILD_TIMEOUT = 30 * 60
INVENTORY_TIMEOUT = 30
TEST_TIMEOUT = 120
ROOT_HELPER_TIMEOUT = 360
GRACE_SECONDS = 8
EVIDENCE_LEAF = "r5-paper-native-os"
ROOT_MODE_ARG = "__root_driver_v1"
PROTECTED_SOURCE_SHA256 = "4fd1ea0f92323d73d82e19b0edb2c40ae42e3722fab9a244938d52ba0d2dd87e"
# Internal, source-pinned adapters; public CLI arguments cannot select a profile.
SOURCE_CHECKPOINT_VALIDATOR = None
ROOT_IDENTITY_ENCODER = None
ROOT_IDENTITY_DECODER = None
FIXTURE_FACTORY = None
STRICT_NAMED_RESULT = False
ROOT_UNEXPECTED_ERROR = None
RESULT_SCHEMA = "r5-paper-native-os-result-v1"
REPORT_SCHEMA = "r5-paper-native-os-report-v1"
REPORT_NOT_RUN = ("actual_native_source_join_requires_post_anchor_arm",
                  "actual_native_source_join_rejects_pre_reset_arm_despite_later_db_writes",
                  "actual_native_readback_invalid_current_daily_off_expired_without_worker",
                  "actual_native_absent_or_unproven_readback_is_commit_unknown")
REPORT_SCOPE = "eight ignored hosted Root OS-only native tests including an owned kernel-filter child; no PostgreSQL, Source, image, Origin, or production authority"


def verify_source_checkpoint() -> None:
    if SOURCE_CHECKPOINT_VALIDATOR is not None:
        SOURCE_CHECKPOINT_VALIDATOR()
        return
    if sha256_file(ROOT / "crates/job-queue/src/owner_beta/paper/operations/protected.rs") != PROTECTED_SOURCE_SHA256:
        raise RunnerError("NATIVE_SOURCE_CHECKPOINT_MISMATCH")


def fixture_child_name(name: str) -> bool:
    return any(name.startswith(prefix) and UUID_RE.fullmatch(name[len(prefix):]) is not None
               for prefix in ("native-readback-", "native-action-", "native-chain-state-", "native-chain-result-"))


class RunnerError(Exception):
    """An error with a bounded, non-sensitive public code."""

    def __init__(self, code: str, status: int = 1):
        super().__init__(code)
        self.code = code
        self.status = status if 1 <= status <= 255 else 1


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        while block := os.read(fd, 1024 * 1024):
            digest.update(block)
    finally:
        os.close(fd)
    return digest.hexdigest()


def _no_symlink_path(path: Path, *, start: Path | None = None) -> Path:
    """Return a canonical existing path after rejecting symlinks in its suffix."""
    if not path.is_absolute():
        raise RunnerError("PATH_NOT_ABSOLUTE")
    path = Path(os.path.abspath(path))
    parts = path.parts
    current = Path(parts[0])
    for part in parts[1:]:
        current = current / part
        try:
            mode = os.lstat(current).st_mode
        except OSError as exc:
            raise RunnerError("PATH_COMPONENT_MISSING") from exc
        if stat.S_ISLNK(mode):
            raise RunnerError("PATH_SYMLINK_REJECTED")
    resolved = path.resolve(strict=True)
    if resolved != path:
        raise RunnerError("PATH_CANONICAL_MISMATCH")
    if start is not None and start not in resolved.parents:
        raise RunnerError("PATH_OUTSIDE_ALLOWED_ROOT")
    return resolved


def _git_output(argv: list[str], cwd: Path) -> str:
    with tempfile.TemporaryDirectory() as temp:
        base = Path(temp)
        try:
            code, stdout, _ = run_logged(
                argv, cwd=cwd,
                env={"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C",
                     "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"},
                stdout_path=base / "stdout.log", stderr_path=base / "stderr.log",
                timeout=15, max_bytes=64 * 1024)
        except RunnerError as exc:
            raise RunnerError("GIT_IDENTITY_FAILED") from exc
        if code != 0:
            raise RunnerError("GIT_IDENTITY_FAILED")
        return stdout.strip()


def verify_candidate(candidate: Path, expected_sha: str) -> None:
    if SHA_RE.fullmatch(expected_sha) is None:
        raise RunnerError("GITHUB_SHA_INVALID")
    candidate = _no_symlink_path(candidate)
    if candidate != ROOT:
        raise RunnerError("CANDIDATE_PATH_MISMATCH")
    actual = _git_output(["git", "-c", f"safe.directory={candidate}", "-C", str(candidate), "rev-parse", "HEAD"], candidate)
    if actual != expected_sha:
        raise RunnerError("CANDIDATE_SHA_MISMATCH")
    dirty = _git_output(["git", "-c", f"safe.directory={candidate}", "-C", str(candidate),
                         "status", "--porcelain", "--untracked-files=all"], candidate)
    if dirty:
        raise RunnerError("CANDIDATE_WORKTREE_NOT_CLEAN")
    verify_source_checkpoint()


def _git_head_without_process(candidate: Path) -> str:
    git_entry = candidate / ".git"
    try:
        gst = os.lstat(git_entry)
    except OSError as exc:
        raise RunnerError("GIT_IDENTITY_FAILED") from exc
    if stat.S_ISDIR(gst.st_mode):
        git_dir = git_entry
    elif stat.S_ISREG(gst.st_mode):
        content = git_entry.read_text(encoding="utf-8").strip()
        match = re.fullmatch(r"gitdir: (.+)", content, re.ASCII)
        if match is None:
            raise RunnerError("GIT_IDENTITY_FAILED")
        git_dir = Path(match.group(1))
        if not git_dir.is_absolute():
            git_dir = candidate / git_dir
    else:
        raise RunnerError("GIT_IDENTITY_FAILED")
    git_dir = _no_symlink_path(git_dir)
    head_path = git_dir / "HEAD"
    head_path = _no_symlink_path(head_path)
    try:
        head = head_path.read_text(encoding="ascii").strip()
    except OSError as exc:
        raise RunnerError("GIT_IDENTITY_FAILED") from exc
    if SHA_RE.fullmatch(head):
        return head
    ref_match = re.fullmatch(r"ref: (refs/[A-Za-z0-9._/-]+)", head, re.ASCII)
    if ref_match is None or ".." in ref_match.group(1).split("/"):
        raise RunnerError("GIT_IDENTITY_FAILED")
    ref = ref_match.group(1)
    common_dir = git_dir
    commondir_path = git_dir / "commondir"
    try:
        cst = os.lstat(commondir_path)
    except FileNotFoundError:
        pass
    except OSError as exc:
        raise RunnerError("GIT_IDENTITY_FAILED") from exc
    else:
        if not stat.S_ISREG(cst.st_mode):
            raise RunnerError("GIT_IDENTITY_FAILED")
        common_raw = Path(commondir_path.read_text(encoding="utf-8").strip())
        common_dir = common_raw if common_raw.is_absolute() else git_dir / common_raw
        common_dir = _no_symlink_path(common_dir)
    loose = common_dir / ref
    try:
        lst = os.lstat(loose)
        if not stat.S_ISREG(lst.st_mode):
            raise RunnerError("GIT_IDENTITY_FAILED")
        value = loose.read_text(encoding="ascii").strip()
    except FileNotFoundError:
        value = ""
        try:
            packed_refs = common_dir / "packed-refs"
            packed_st = os.lstat(packed_refs)
            if not stat.S_ISREG(packed_st.st_mode):
                raise RunnerError("GIT_IDENTITY_FAILED")
            for line in packed_refs.read_text(encoding="ascii").splitlines():
                match = re.fullmatch(r"([0-9a-f]{40}) " + re.escape(ref), line, re.ASCII)
                if match:
                    if value:
                        raise RunnerError("GIT_IDENTITY_FAILED")
                    value = match.group(1)
        except OSError as exc:
            raise RunnerError("GIT_IDENTITY_FAILED") from exc
    if SHA_RE.fullmatch(value) is None:
        raise RunnerError("GIT_IDENTITY_FAILED")
    return value


def verify_candidate_after_handoff(candidate: Path, expected_sha: str, script_hash: str) -> None:
    if SHA_RE.fullmatch(expected_sha) is None or HASH_RE.fullmatch(script_hash) is None:
        raise RunnerError("ROOT_DRIVER_PIN_INVALID")
    if _no_symlink_path(candidate) != ROOT or _git_head_without_process(candidate) != expected_sha:
        raise RunnerError("CANDIDATE_SHA_MISMATCH")
    if sha256_file(SCRIPT) != script_hash:
        raise RunnerError("RUNNER_SOURCE_IDENTITY_DRIFT")
    verify_source_checkpoint()


def hosted_paths(environ: dict[str, str]) -> tuple[Path, Path, str]:
    if environ.get("GITHUB_ACTIONS") != "true":
        raise RunnerError("GITHUB_ACTIONS_REQUIRED")
    sha = environ.get("GITHUB_SHA", "")
    if SHA_RE.fullmatch(sha) is None:
        raise RunnerError("GITHUB_SHA_INVALID")
    workspace_raw, temp_raw, target_raw = (environ.get(k, "") for k in
                                            ("GITHUB_WORKSPACE", "RUNNER_TEMP", "CARGO_TARGET_DIR"))
    if not workspace_raw or not temp_raw or not target_raw:
        raise RunnerError("HOSTED_PATHS_REQUIRED")
    workspace = _no_symlink_path(Path(workspace_raw))
    if workspace != ROOT:
        raise RunnerError("WORKSPACE_MISMATCH")
    temp = _no_symlink_path(Path(temp_raw))
    temp_st = os.stat(temp, follow_symlinks=False)
    if not stat.S_ISDIR(temp_st.st_mode) or temp_st.st_uid != os.geteuid() or temp_st.st_mode & 0o022:
        raise RunnerError("RUNNER_TEMP_NOT_PRIVATE")
    target = _no_symlink_path(Path(target_raw), start=temp)
    target_st = os.stat(target, follow_symlinks=False)
    if not stat.S_ISDIR(target_st.st_mode) or target_st.st_uid != os.geteuid():
        raise RunnerError("CARGO_TARGET_DIR_INVALID")
    verify_candidate(workspace, sha)
    return temp, target, sha


def parse_compiler_artifact(lines: list[str], target_dir: Path) -> Path:
    records: list[str] = []
    for line in lines:
        try:
            message = json.loads(line)
        except (json.JSONDecodeError, TypeError):
            continue
        if not isinstance(message, dict) or message.get("reason") != "compiler-artifact":
            continue
        target = message.get("target") or {}
        profile = message.get("profile") or {}
        if (target.get("name") == TARGET_NAME and target.get("kind") == ["lib"]
                and target.get("src_path") == str(LIB_SOURCE) and profile.get("test") is True):
            executable = message.get("executable")
            if isinstance(executable, str):
                records.append(executable)
            else:
                raise RunnerError("TEST_ARTIFACT_PATH_MISSING")
    if len(records) != 1:
        raise RunnerError("TEST_ARTIFACT_MISSING_OR_AMBIGUOUS")
    path = _no_symlink_path(Path(records[0]), start=target_dir)
    st = os.stat(path, follow_symlinks=False)
    if not stat.S_ISREG(st.st_mode) or st.st_nlink != 1 or st.st_mode & 0o022 or st.st_uid != os.geteuid():
        raise RunnerError("TEST_ARTIFACT_METADATA_INVALID")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        if os.read(fd, 4) != b"\x7fELF":
            raise RunnerError("TEST_ARTIFACT_NOT_ELF")
    finally:
        os.close(fd)
    return path


def file_identity(fd: int) -> dict[str, int]:
    st = os.fstat(fd)
    return {"device": st.st_dev, "inode": st.st_ino, "size": st.st_size,
            "mode": stat.S_IMODE(st.st_mode), "uid": st.st_uid, "gid": st.st_gid,
            "links": st.st_nlink}


def stat_identity(st: os.stat_result) -> dict[str, int]:
    return {"device": st.st_dev, "inode": st.st_ino, "size": st.st_size,
            "mode": stat.S_IMODE(st.st_mode), "uid": st.st_uid, "gid": st.st_gid,
            "links": st.st_nlink}


def open_pinned_elf(path: Path, target_dir: Path, expected_hash: str,
                    expected_identity: dict[str, int] | None = None) -> tuple[int, dict[str, int]]:
    canonical = _no_symlink_path(path, start=target_dir)
    fd = os.open(canonical, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        identity = file_identity(fd)
        if expected_identity is not None and identity != expected_identity:
            raise RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
        if not stat.S_ISREG(os.fstat(fd).st_mode) or identity["links"] != 1 or identity["mode"] & 0o022:
            raise RunnerError("TEST_ARTIFACT_METADATA_INVALID")
        if os.pread(fd, 4, 0) != b"\x7fELF":
            raise RunnerError("TEST_ARTIFACT_NOT_ELF")
        digest = hashlib.sha256()
        offset = 0
        while block := os.pread(fd, 1024 * 1024, offset):
            digest.update(block)
            offset += len(block)
        if digest.hexdigest() != expected_hash:
            raise RunnerError("TEST_ARTIFACT_HASH_MISMATCH")
        if stat_identity(os.stat(canonical, follow_symlinks=False)) != identity:
            raise RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
        return fd, identity
    except BaseException:
        os.close(fd)
        raise


def parse_inventory(output: str) -> dict[str, int]:
    counts = {name: 0 for name in TESTS}
    for line in output.splitlines():
        match = re.fullmatch(r"(.+): test", line.strip(), re.ASCII)
        if match and match.group(1) in counts:
            counts[match.group(1)] += 1
    if any(value != 1 for value in counts.values()):
        raise RunnerError("TEST_INVENTORY_MISSING_OR_DUPLICATE")
    return counts


def validate_named_result(name: str, output: str, exit_code: int) -> None:
    escaped = re.escape(name)
    named = re.findall(rf"^test {escaped}\s+\.\.\. ok$", output, re.MULTILINE)
    summaries = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;",
                           output, re.MULTILINE)
    if exit_code != 0 or named != [f"test {name} ... ok"] or summaries != [("1", "0", "0")]:
        raise RunnerError("NAMED_TEST_RESULT_INVALID", exit_code if exit_code else 1)
    if STRICT_NAMED_RESULT:
        named_lines = [line for line in output.splitlines()
                       if line.startswith("test ") and not line.startswith("test result:")]
        result_lines = [line for line in output.splitlines() if line.startswith("test result:")]
        if named_lines != [f"test {name} ... ok"] or len(result_lines) != 1:
            raise RunnerError("NAMED_TEST_RESULT_INVALID")
        if re.fullmatch(r"test result: ok\. 1 passed; 0 failed; 0 ignored;"
                        r"(?: 0 measured; [0-9]+ filtered out; finished in [0-9.]+s)?",
                        result_lines[0], re.ASCII) is None:
            raise RunnerError("NAMED_TEST_RESULT_INVALID")


def _private_log(path: Path) -> int:
    return os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)


def run_logged(argv: list[str], *, cwd: Path, env: dict[str, str], stdout_path: Path,
               stderr_path: Path, timeout: int, max_bytes: int = MAX_BUILD_LOG,
               pass_fds: tuple[int, ...] = (), terminate_grace: int = GRACE_SECONDS,
               on_started: Callable[[subprocess.Popen[bytes]], None] | None = None) -> tuple[int, str, str]:
    out_fd = _private_log(stdout_path)
    err_fd = _private_log(stderr_path)
    process: subprocess.Popen[bytes] | None = None
    selector = selectors.DefaultSelector()
    sizes = {"stdout": 0, "stderr": 0}
    outputs = {"stdout": out_fd, "stderr": err_fd}
    termination: str | None = None
    terminated_at: float | None = None
    killed = False

    def stop(reason: str) -> None:
        nonlocal termination, terminated_at
        if termination is not None:
            return
        termination = reason
        terminated_at = time.monotonic()
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

    try:
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, close_fds=True,
                                   pass_fds=pass_fds, start_new_session=True)
        if on_started is not None:
            on_started(process)
        assert process.stdout is not None and process.stderr is not None
        os.set_blocking(process.stdout.fileno(), False)
        os.set_blocking(process.stderr.fileno(), False)
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        deadline = time.monotonic() + timeout
        while selector.get_map():
            now = time.monotonic()
            if termination is None and now >= deadline:
                stop("timeout")
            if termination is not None and terminated_at is not None and not killed and now - terminated_at >= terminate_grace:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                killed = True
            try:
                ready = selector.select(0.1)
            except KeyboardInterrupt:
                stop("interrupted")
                continue
            for key, _ in ready:
                stream = key.fileobj
                channel = key.data
                try:
                    chunk = os.read(stream.fileno(), 65536)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(stream)
                    stream.close()
                    continue
                remaining = max(0, max_bytes - sizes[channel])
                if remaining:
                    view = memoryview(chunk[:remaining])
                    while view:
                        written = os.write(outputs[channel], view)
                        view = view[written:]
                    sizes[channel] += min(len(chunk), remaining)
                if len(chunk) > remaining:
                    stop("output_limit")
        if termination is None:
            # EOF does not imply exit: wait only for the remaining original
            # budget before treating the still-owned child as timed out.
            try:
                code = process.wait(timeout=max(0.0, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                stop("timeout")
                _terminate_owned(process, grace=terminate_grace)
                code = process.wait()
        else:
            if process.poll() is None:
                _terminate_owned(process, grace=terminate_grace)
            code = process.wait()
    except OSError as exc:
        raise RunnerError("OWNED_COMMAND_START_FAILED") from exc
    except KeyboardInterrupt:
        if process is not None:
            _terminate_owned(process, grace=terminate_grace)
        raise RunnerError("INTERRUPTED", 130) from None
    finally:
        selector.close()
        if process is not None:
            for pipe in (process.stdout, process.stderr):
                if pipe is not None and not pipe.closed:
                    pipe.close()
            if process.poll() is None:
                _terminate_owned(process, grace=terminate_grace)
            else:
                process.wait()
        os.close(out_fd)
        os.close(err_fd)
    if termination == "output_limit":
        raise RunnerError("CHILD_OUTPUT_LIMIT_EXCEEDED")
    if termination == "timeout":
        raise RunnerError("OWNED_COMMAND_TIMEOUT", 124)
    if termination == "interrupted":
        raise RunnerError("INTERRUPTED", 130)
    return code, stdout_path.read_text(encoding="utf-8", errors="replace"), \
        stderr_path.read_text(encoding="utf-8", errors="replace")


def _terminate_owned(process: subprocess.Popen[bytes], *, grace: int = GRACE_SECONDS) -> None:
    if process.poll() is not None:
        process.wait()
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=grace)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def _no_acl(fd: int) -> bool:
    for attr in ("system.posix_acl_access", "system.posix_acl_default"):
        try:
            os.getxattr(fd, attr)
        except OSError as exc:
            if exc.errno in (getattr(__import__("errno"), "ENODATA"), getattr(__import__("errno"), "ENOTSUP")):
                continue
            return False
        else:
            return False
    return True


class OwnedFixture:
    """Create/remove the single fixed Root fixture parent using retained identity."""

    def __init__(self, parent: Path = FIXTURE_PARENT, owner_uid: int = 0, owner_gid: int = 0,
                 nonce: str | None = None):
        self.parent = parent
        self.owner_uid = owner_uid
        self.owner_gid = owner_gid
        self.fd: int | None = None
        self.identity: tuple[int, int] | None = None
        self.nonce = nonce or str(uuid.uuid4())

    def before_test(self) -> None:
        """Default native fixture behavior is unchanged."""

    def after_test(self) -> None:
        """Additional source-pinned profiles can independently check cleanup."""

    def create(self) -> None:
        if self.parent == FIXTURE_PARENT:
            run_st = os.lstat(Path("/run"))
            if not stat.S_ISDIR(run_st.st_mode) or run_st.st_uid != 0:
                raise RunnerError("RUN_DIRECTORY_INVALID")
        created_identity: tuple[int, int] | None = None
        try:
            os.lstat(self.parent)
        except FileNotFoundError:
            try:
                os.mkdir(self.parent, 0o700)
                made = os.lstat(self.parent)
                created_identity = (made.st_dev, made.st_ino)
            except FileExistsError as exc:
                raise RunnerError("FIXTURE_PARENT_PREEXISTED") from exc
        else:
            raise RunnerError("FIXTURE_PARENT_PREEXISTED")
        try:
            self.fd = os.open(self.parent, os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0))
            st = os.fstat(self.fd)
            self.identity = (st.st_dev, st.st_ino)
            if st.st_uid != self.owner_uid or st.st_gid != self.owner_gid or stat.S_IMODE(st.st_mode) != 0o700 or not _no_acl(self.fd):
                raise RunnerError("FIXTURE_PARENT_METADATA_INVALID")
            nfd = os.open(".owner-nonce", os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
                          0o400, dir_fd=self.fd)
            try:
                os.write(nfd, (self.nonce + "\n").encode("ascii"))
                os.fsync(nfd)
            finally:
                os.close(nfd)
            os.fsync(self.fd)
        except BaseException:
            if self.fd is not None:
                try:
                    self._verify_parent_name()
                    for entry in os.listdir(self.fd):
                        st = os.stat(entry, dir_fd=self.fd, follow_symlinks=False)
                        if entry == ".owner-nonce" and stat.S_ISREG(st.st_mode) and st.st_uid == self.owner_uid:
                            os.unlink(entry, dir_fd=self.fd)
                    if not os.listdir(self.fd):
                        os.close(self.fd)
                        self.fd = None
                        os.rmdir(self.parent)
                except (OSError, RunnerError):
                    pass
            elif created_identity is not None:
                try:
                    named = os.lstat(self.parent)
                    if ((named.st_dev, named.st_ino) == created_identity and stat.S_ISDIR(named.st_mode)
                            and named.st_uid == self.owner_uid and stat.S_IMODE(named.st_mode) == 0o700
                            and not os.listdir(self.parent)):
                        os.rmdir(self.parent)
                except OSError:
                    pass
            raise

    def _verify(self) -> None:
        if self.fd is None or self.identity is None:
            raise RunnerError("FIXTURE_OWNERSHIP_MISSING")
        st = os.fstat(self.fd)
        try:
            named = os.lstat(self.parent)
        except OSError as exc:
            raise RunnerError("FIXTURE_PARENT_IDENTITY_DRIFT") from exc
        if (st.st_dev, st.st_ino) != self.identity or (named.st_dev, named.st_ino) != self.identity:
            raise RunnerError("FIXTURE_PARENT_IDENTITY_DRIFT")
        if (not stat.S_ISDIR(named.st_mode) or named.st_uid != self.owner_uid or named.st_gid != self.owner_gid
                or stat.S_IMODE(named.st_mode) != 0o700 or not _no_acl(self.fd)):
            raise RunnerError("FIXTURE_PARENT_METADATA_INVALID")
        nfd = os.open(".owner-nonce", os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0), dir_fd=self.fd)
        try:
            nst = os.fstat(nfd)
            if (not stat.S_ISREG(nst.st_mode) or nst.st_uid != self.owner_uid or nst.st_gid != self.owner_gid
                    or nst.st_nlink != 1 or nst.st_size != 37 or stat.S_IMODE(nst.st_mode) != 0o400):
                raise RunnerError("FIXTURE_NONCE_METADATA_INVALID")
            if os.read(nfd, 128) != (self.nonce + "\n").encode("ascii"):
                raise RunnerError("FIXTURE_NONCE_MISMATCH")
        finally:
            os.close(nfd)

    def cleanup(self) -> None:
        self._verify()
        assert self.fd is not None
        names = os.listdir(self.fd)
        for name in names:
            if name == ".owner-nonce":
                continue
            if not fixture_child_name(name):
                raise RunnerError("FIXTURE_UNOWNED_ENTRY")
            _remove_owned_tree(self.fd, name, self.owner_uid)
        self._verify()
        os.unlink(".owner-nonce", dir_fd=self.fd)
        os.fsync(self.fd)
        self._verify_parent_name()
        os.close(self.fd)
        self.fd = None
        os.rmdir(self.parent)
        self.identity = None

    def _verify_parent_name(self) -> None:
        assert self.identity is not None
        st = os.lstat(self.parent)
        if (st.st_dev, st.st_ino) != self.identity or stat.S_ISLNK(st.st_mode):
            raise RunnerError("FIXTURE_PARENT_IDENTITY_DRIFT")


def _remove_owned_tree(parent_fd: int, name: str, owner_uid: int) -> None:
    fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0), dir_fd=parent_fd)
    try:
        st = os.fstat(fd)
        if not stat.S_ISDIR(st.st_mode) or st.st_uid != owner_uid or stat.S_IMODE(st.st_mode) != 0o700:
            raise RunnerError("FIXTURE_CHILD_METADATA_INVALID")
        for child in os.listdir(fd):
            cst = os.stat(child, dir_fd=fd, follow_symlinks=False)
            if stat.S_ISDIR(cst.st_mode):
                _remove_owned_tree(fd, child, owner_uid)
            elif stat.S_ISREG(cst.st_mode) and cst.st_uid == owner_uid:
                os.unlink(child, dir_fd=fd)
            else:
                raise RunnerError("FIXTURE_CHILD_NODE_INVALID")
        os.fsync(fd)
    finally:
        os.close(fd)
    os.rmdir(name, dir_fd=parent_fd)


def _root_driver(args: list[str]) -> int:
    outcomes: list[dict[str, str]] = []
    child: subprocess.Popen[bytes] | None = None
    interrupted = False

    def on_signal(signum: int, _frame: object) -> None:
        nonlocal interrupted
        interrupted = True
        if child is not None and child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

    prior = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGTERM, signal.SIGINT)}
    if len(args) != 9:
        raise RunnerError("ROOT_DRIVER_ARGUMENTS_INVALID")
    nonce, candidate_text, sha, target_text, elf_text, elf_hash, identity_text, evidence_text, script_hash = args
    if (UUID_RE.fullmatch(nonce) is None or SHA_RE.fullmatch(sha) is None
            or HASH_RE.fullmatch(elf_hash) is None or HASH_RE.fullmatch(script_hash) is None):
        raise RunnerError("ROOT_DRIVER_PIN_INVALID")
    try:
        expected_identity = (json.loads(identity_text, object_pairs_hook=_unique_root_result_object)
                             if STRICT_NAMED_RESULT else json.loads(identity_text))
        if ROOT_IDENTITY_DECODER is not None:
            expected_identity = ROOT_IDENTITY_DECODER(expected_identity)
        if not isinstance(expected_identity, dict) or set(expected_identity) != {
            "device", "inode", "size", "mode", "uid", "gid", "links"
        } or any(type(v) is not int or v < 0 for v in expected_identity.values()):
            raise ValueError
    except (json.JSONDecodeError, ValueError, TypeError):
        raise RunnerError("ROOT_DRIVER_PIN_INVALID") from None
    candidate, target, elf, evidence = map(Path, (candidate_text, target_text, elf_text, evidence_text))
    if os.geteuid() != 0 or os.environ.get("SUDO_UID", "") != str(expected_identity["uid"]):
        raise RunnerError("ROOT_REQUIRED")
    root_env = os.environ
    if set(root_env) - {"LANG", "LC_ALL", "TZ", "SUDO_UID", "PATH"}:
        raise RunnerError("ROOT_ENVIRONMENT_NOT_CLEARED")
    if (root_env.get("LANG"), root_env.get("LC_ALL"), root_env.get("TZ")) != ("C", "C", "UTC"):
        raise RunnerError("ROOT_LOCALE_INVALID")
    verify_candidate_after_handoff(candidate, sha, script_hash)
    temp = evidence.parent
    _no_symlink_path(temp)
    temp_st = os.stat(temp, follow_symlinks=False)
    if not stat.S_ISDIR(temp_st.st_mode) or temp_st.st_uid != expected_identity["uid"] or temp_st.st_mode & 0o022:
        raise RunnerError("RUNNER_TEMP_IDENTITY_INVALID")
    evidence = _no_symlink_path(evidence, start=temp)
    if evidence != temp / EVIDENCE_LEAF:
        raise RunnerError("EVIDENCE_PATH_OUTSIDE_RUNNER_TEMP")
    est = os.stat(evidence, follow_symlinks=False)
    if not stat.S_ISDIR(est.st_mode) or est.st_uid != expected_identity["uid"] or stat.S_IMODE(est.st_mode) != 0o700:
        raise RunnerError("EVIDENCE_DIRECTORY_INVALID")
    target = _no_symlink_path(target, start=temp)
    target_st = os.stat(target, follow_symlinks=False)
    if not stat.S_ISDIR(target_st.st_mode) or target_st.st_uid != expected_identity["uid"] or target_st.st_mode & 0o022:
        raise RunnerError("CARGO_TARGET_IDENTITY_INVALID")
    elf = _no_symlink_path(elf, start=target)
    fixture = OwnedFixture(owner_uid=0, nonce=nonce) if FIXTURE_FACTORY is None else FIXTURE_FACTORY()
    final_error: str | None = None
    cleanup = "NOT_CONFIRMED"
    try:
        if interrupted:
            raise RunnerError("INTERRUPTED", 130)
        fixture.create()
        cleanup = "PENDING"
        if interrupted:
            raise RunnerError("INTERRUPTED", 130)
        for index, name in enumerate(TESTS, start=1):
            if interrupted:
                raise RunnerError("INTERRUPTED", 130)
            verify_candidate_after_handoff(candidate, sha, script_hash)
            fixture.before_test()
            fd, identity = open_pinned_elf(elf, target, elf_hash, expected_identity)
            if identity != expected_identity:
                os.close(fd)
                raise RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
            out_path = evidence / f"root-test-{index}.stdout.log"
            err_path = evidence / f"root-test-{index}.stderr.log"
            def retain_child(process: subprocess.Popen[bytes]) -> None:
                nonlocal child
                child = process
            try:
                result, output, _ = run_logged(
                    [f"/proc/self/fd/{fd}", "--exact", "--ignored", "--test-threads=1", name],
                    cwd=candidate, env={"LANG": "C", "LC_ALL": "C", "TZ": "UTC"},
                    stdout_path=out_path, stderr_path=err_path, timeout=TEST_TIMEOUT,
                    max_bytes=MAX_CHILD_LOG, pass_fds=(fd,), on_started=retain_child)
                os.chown(out_path, expected_identity["uid"], 0)
                os.chown(err_path, expected_identity["uid"], 0)
                if interrupted:
                    raise RunnerError("INTERRUPTED", 130)
                fixture.after_test()
                validate_named_result(name, output, result)
                outcomes.append({"name": name, "status": "PASS"})
            except RunnerError as exc:
                if exc.code == "OWNED_COMMAND_TIMEOUT":
                    raise RunnerError("NATIVE_TEST_TIMEOUT", 124) from exc
                raise
            finally:
                if child is not None and child.poll() is None:
                    _terminate_owned(child)
                if child is not None:
                    child.wait()
                child = None
                for log_path in (out_path, err_path):
                    try:
                        os.chown(log_path, expected_identity["uid"], 0)
                    except FileNotFoundError:
                        pass
                os.close(fd)
        verify_candidate_after_handoff(candidate, sha, script_hash)
        open_fd, current = open_pinned_elf(elf, target, elf_hash, expected_identity)
        os.close(open_fd)
        if current != expected_identity:
            raise RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
    except RunnerError as exc:
        final_error = exc.code
        if exc.code == "INTERRUPTED":
            outcomes.append({"name": "INTERRUPTED", "status": "FAIL"})
    except BaseException:
        if ROOT_UNEXPECTED_ERROR is None:
            raise
        final_error = ROOT_UNEXPECTED_ERROR
    finally:
        if child is not None:
            _terminate_owned(child)
            child.wait()
        if fixture.fd is not None:
            try:
                fixture.cleanup()
                cleanup = "PASS"
            except RunnerError as exc:
                cleanup = exc.code
                final_error = final_error or exc.code
        for sig, handler in prior.items():
            signal.signal(sig, handler)
    result_obj = {"schema": RESULT_SCHEMA, "source_sha": sha,
                  "elf_sha256": elf_hash, "tests": outcomes, "cleanup": cleanup,
                  "status": "PASS" if final_error is None and len(outcomes) == len(TESTS) else "FAIL",
                  "error": final_error}
    sys.stdout.write(json.dumps(result_obj, sort_keys=True, separators=(",", ":")) + "\n")
    sys.stdout.flush()
    return 0 if result_obj["status"] == "PASS" else 1


def write_private(path: Path, data: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    try:
        with os.fdopen(fd, "wb", closefd=False) as stream:
            stream.write(data)
            stream.flush()
            os.fsync(fd)
    finally:
        os.close(fd)


def create_evidence(temp: Path) -> Path:
    base = temp / EVIDENCE_LEAF
    try:
        os.mkdir(base, 0o700)
    except FileExistsError as exc:
        raise RunnerError("EVIDENCE_DIRECTORY_PREEXISTED") from exc
    st = os.stat(base, follow_symlinks=False)
    if st.st_uid != os.geteuid() or stat.S_IMODE(st.st_mode) != 0o700:
        raise RunnerError("EVIDENCE_DIRECTORY_INVALID")
    return base


def verify_evidence(evidence: Path, temp: Path, owner_uid: int) -> None:
    temp_path = _no_symlink_path(temp)
    evidence_path = _no_symlink_path(evidence)
    if evidence_path != temp_path / EVIDENCE_LEAF:
        raise RunnerError("EVIDENCE_PATH_OUTSIDE_RUNNER_TEMP")
    st = os.stat(evidence, follow_symlinks=False)
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != owner_uid or stat.S_IMODE(st.st_mode) != 0o700:
        raise RunnerError("EVIDENCE_DIRECTORY_INVALID")


def _root_result(output: str, source_sha: str, elf_hash: str) -> dict[str, object]:
    try:
        value = json.loads(output, object_pairs_hook=_unique_root_result_object) if STRICT_NAMED_RESULT else json.loads(output)
    except (json.JSONDecodeError, ValueError):
        raise RunnerError("ROOT_RESULT_MALFORMED") from None
    if not isinstance(value, dict) or set(value) != {"schema", "source_sha", "elf_sha256", "tests", "cleanup", "status", "error"}:
        raise RunnerError("ROOT_RESULT_SCHEMA_INVALID")
    if value["schema"] != RESULT_SCHEMA or value["source_sha"] != source_sha or value["elf_sha256"] != elf_hash:
        raise RunnerError("ROOT_RESULT_IDENTITY_INVALID")
    tests = value["tests"]
    if not isinstance(tests, list) or any(not isinstance(x, dict) or set(x) != {"name", "status"} for x in tests):
        raise RunnerError("ROOT_RESULT_TESTS_INVALID")
    if value["status"] != "PASS" or value["cleanup"] != "PASS" or value["error"] is not None:
        raise RunnerError("ROOT_RESULT_FAILED")
    if tests != [{"name": name, "status": "PASS"} for name in TESTS]:
        raise RunnerError("ROOT_RESULT_TESTS_INVALID")
    return value


def _unique_root_result_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError
        value[key] = item
    return value


def _unknown_root_failure() -> dict[str, object]:
    return {"root_error": "UNKNOWN", "root_result_status": "UNKNOWN",
            "tests": [], "cleanup": "NOT_CONFIRMED"}


def _root_failure_projection(output: str, source_sha: str, elf_hash: str) -> dict[str, object]:
    """Project only a fully validated failed Root result into closed public fields."""
    if (type(output) is not str or type(source_sha) is not str or SHA_RE.fullmatch(source_sha) is None
            or type(elf_hash) is not str or HASH_RE.fullmatch(elf_hash) is None):
        return _unknown_root_failure()
    try:
        value = json.loads(output, object_pairs_hook=_unique_root_result_object)
    except (json.JSONDecodeError, ValueError, TypeError, RecursionError):
        return _unknown_root_failure()
    if (type(value) is not dict
            or set(value) != {"schema", "source_sha", "elf_sha256", "tests", "cleanup", "status", "error"}
            or type(value["schema"]) is not str or value["schema"] != RESULT_SCHEMA
            or type(value["source_sha"]) is not str or value["source_sha"] != source_sha
            or type(value["elf_sha256"]) is not str or value["elf_sha256"] != elf_hash
            or type(value["status"]) is not str or value["status"] != "FAIL"
            or type(value["cleanup"]) is not str or value["cleanup"] not in ROOT_FAILURE_CLEANUP_CODES
            or type(value["error"]) is not str or value["error"] not in ROOT_FAILURE_ERROR_CODES):
        return _unknown_root_failure()

    tests = value["tests"]
    if type(tests) is not list:
        return _unknown_root_failure()
    error = value["error"]
    if error == "INTERRUPTED":
        if not tests or tests[-1] != {"name": "INTERRUPTED", "status": "FAIL"}:
            return _unknown_root_failure()
        passed = tests[:-1]
        if len(passed) >= len(TESTS):
            return _unknown_root_failure()
    else:
        passed = tests
    expected = [{"name": name, "status": "PASS"} for name in TESTS[:len(passed)]]
    if passed != expected:
        return _unknown_root_failure()
    safe_tests = [dict(item) for item in passed]
    if error == "INTERRUPTED":
        safe_tests.append({"name": "INTERRUPTED", "status": "FAIL"})
    return {"root_error": error, "root_result_status": "FAIL",
            "tests": safe_tests, "cleanup": value["cleanup"]}


def run_hosted(environ: dict[str, str] | None = None) -> dict[str, object]:
    env0 = dict(os.environ if environ is None else environ)
    temp, target, source_sha = hosted_paths(env0)
    evidence = create_evidence(temp)
    status = "FAIL"
    artifact_hash: str | None = None
    root_results: dict[str, object] | None = None
    root_failure: dict[str, object] | None = None
    error = "UNEXPECTED_FAILURE"
    try:
        script_hash = sha256_file(SCRIPT)
        cargo_env = dict(os.environ)
        cargo_env["CARGO_TARGET_DIR"] = str(target)
        cargo_env["CARGO_BUILD_JOBS"] = "2"
        code, stdout, _stderr = run_logged(
            ["cargo", "test", "--locked", "--package", PACKAGE_NAME, "--lib", "--no-run", "--message-format=json"],
            cwd=ROOT, env=cargo_env, stdout_path=evidence / "compile.stdout.log",
            stderr_path=evidence / "compile.stderr.log", timeout=BUILD_TIMEOUT)
        if code != 0:
            raise RunnerError("CARGO_COMPILE_FAILED", code)
        elf = parse_compiler_artifact(stdout.splitlines(), target)
        artifact_hash = sha256_file(elf)
        elf_fd, identity = open_pinned_elf(elf, target, artifact_hash)
        try:
            inventory_cmd = [f"/proc/self/fd/{elf_fd}", "--list", "--ignored"]
            code, inventory, _ = run_logged(
                inventory_cmd, cwd=ROOT, env={"LANG": "C", "LC_ALL": "C", "TZ": "UTC"},
                stdout_path=evidence / "inventory.stdout.log", stderr_path=evidence / "inventory.stderr.log",
                timeout=INVENTORY_TIMEOUT, max_bytes=MAX_CHILD_LOG, pass_fds=(elf_fd,))
            if code != 0:
                raise RunnerError("TEST_INVENTORY_COMMAND_FAILED", code)
            parse_inventory(inventory)
            again, now = open_pinned_elf(elf, target, artifact_hash, identity)
            os.close(again)
            if now != identity:
                raise RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
        finally:
            os.close(elf_fd)
        sudo = Path("/usr/bin/sudo")
        if not sudo.is_file() or not os.access(sudo, os.X_OK):
            raise RunnerError("NONINTERACTIVE_SUDO_UNAVAILABLE")
        nonce = str(uuid.uuid4())
        uid = str(os.geteuid())
        handoff_identity = identity if ROOT_IDENTITY_ENCODER is None else ROOT_IDENTITY_ENCODER(identity)
        driver_args = [ROOT_MODE_ARG, nonce, str(ROOT), source_sha, str(target), str(elf), artifact_hash,
                       json.dumps(handoff_identity, sort_keys=True, separators=(",", ":")), str(evidence), script_hash]
        root_env = {"LANG": "C", "LC_ALL": "C", "TZ": "UTC", "SUDO_UID": uid}
        root_out = evidence / "root-driver.stdout.log"
        root_err = evidence / "root-driver.stderr.log"
        code, raw, _ = run_logged(
            [str(sudo), "-n", "--", "/usr/bin/env", "-i", "LANG=C", "LC_ALL=C", "TZ=UTC",
             f"SUDO_UID={uid}", "/usr/bin/python3", str(SCRIPT), *driver_args],
            cwd=ROOT, env={"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C", "TZ": "UTC"},
            stdout_path=root_out, stderr_path=root_err, timeout=ROOT_HELPER_TIMEOUT,
            max_bytes=MAX_CHILD_LOG, terminate_grace=60)
        if code != 0:
            root_failure = _root_failure_projection(raw, source_sha, artifact_hash)
            root_results = {"tests": root_failure["tests"], "cleanup": root_failure["cleanup"]}
            raise RunnerError("ROOT_OS_TESTS_FAILED", code)
        root_results = _root_result(raw, source_sha, artifact_hash)
        verify_candidate(ROOT, source_sha)
        fd, current = open_pinned_elf(elf, target, artifact_hash, identity)
        os.close(fd)
        if current != identity:
            raise RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
        verify_evidence(evidence, temp, os.geteuid())
        status, error = "PASS", "NONE"
    except RunnerError as exc:
        error = exc.code
    report = {"schema": REPORT_SCHEMA, "status": status, "error": error,
              "source_sha": source_sha, "elf_sha256": artifact_hash,
              "tests": root_results.get("tests", []) if isinstance(root_results, dict) else [],
              "cleanup": root_results.get("cleanup", "NOT_CONFIRMED") if isinstance(root_results, dict) else "NOT_CONFIRMED",
              "not_run": list(REPORT_NOT_RUN), "scope": REPORT_SCOPE}
    if root_failure is not None:
        report["root_error"] = root_failure["root_error"]
        report["root_result_status"] = root_failure["root_result_status"]
    write_private(evidence / "report.json", (json.dumps(report, sort_keys=True, indent=2) + "\n").encode())
    return {**report, "evidence": str(evidence)}


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    try:
        if argv and argv[0] == ROOT_MODE_ARG:
            return _root_driver(argv[1:])
        if argv:
            raise RunnerError("NO_PUBLIC_ARGUMENTS_ACCEPTED", 2)
        result = run_hosted()
        public_result = {k: result[k] for k in ("status", "error", "source_sha", "tests", "cleanup", "not_run")}
        if "root_error" in result:
            root_error = result["root_error"]
            root_status = result.get("root_result_status")
            if not (root_status == "FAIL" and type(root_error) is str
                    and root_error in ROOT_FAILURE_ERROR_CODES) and not (
                        root_status == "UNKNOWN" and root_error == "UNKNOWN"):
                root_error, root_status = "UNKNOWN", "UNKNOWN"
            public_result["root_error"] = root_error
            public_result["root_result_status"] = root_status
        print(json.dumps(public_result, sort_keys=True, separators=(",", ":")))
        return 0 if result["status"] == "PASS" else 1
    except RunnerError as exc:
        print(json.dumps({"status": "FAIL", "error": exc.code}, separators=(",", ":")))
        return exc.status


if __name__ == "__main__":
    raise SystemExit(main())
