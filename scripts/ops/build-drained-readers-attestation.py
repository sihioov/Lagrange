#!/usr/bin/env python3
"""Build the v1 drained-readers build attestation from observed Docker state.

Observe-only: this tool only runs `docker inspect`.  It never stops, starts,
removes or updates a container.  The inspect template is read from the build
layout gate itself (the `docker_format` literal) so the recorded strings are
the exact strings every build gate later re-compares.  The result is
serialized in the validator's canonical form, written to a private temporary
file, accepted by the real bash validator
`rbl_drained_readers_attestation_binding`, and only then linked into place.
Creating a record for a production build remains a separate Owner decision.

Errors print one value-free code on stderr and nothing else.
"""
from __future__ import annotations

import argparse
import ast
import json
import os
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable

HELPER = Path(__file__).resolve().parent / "lib" / "release-build-layout.sh"
PROJECT = "lagrange-station"
SCOPE = "image-build-only"
FORMAT_V1 = "lagrange-build-drained-readers-attestation-v1"
# Sorted, as the validator requires.  research-worker exits 2, the others 0.
READERS_V1 = (
    ("lagrange-station-owner-equity-v2-runner-1", 0),
    ("lagrange-station-research-worker-1", 2),
)
MAX_LIFETIME_SECONDS = 12 * 60 * 60
HEALTH = ("healthy", "unhealthy", "starting", "absent")
PREFIX = "DRAINED_READERS_ATTESTATION"


class AttestationError(Exception):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def _template() -> str:
    """Return the exact inspect template of the build layout gate."""
    try:
        text = HELPER.read_text(encoding="utf-8")
        body = text.split("release_build_layout_gate() {", 1)[1].split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        for node in ast.walk(ast.parse(body)):
            if (isinstance(node, ast.Assign) and len(node.targets) == 1
                    and isinstance(node.targets[0], ast.Name) and node.targets[0].id == "docker_format"):
                value = ast.literal_eval(node.value)
                if isinstance(value, str):
                    return value
    except (OSError, IndexError, SyntaxError, ValueError, UnicodeError):
        pass
    raise AttestationError("ATTESTATION_DOCKER")


def _default_runner(argv: tuple[str, ...], **kwargs: Any) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(argv, timeout=kwargs.get("timeout"), stdout=kwargs.get("stdout"),
                          stderr=kwargs.get("stderr"), check=False, close_fds=True,
                          env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C", "TZ": "UTC"})


def _inspect_one(name: str, expected_exit: int, runner: Callable[..., Any], template: str) -> dict[str, Any]:
    argv = ("docker", "inspect", "--type", "container", "--format", template, name)
    try:
        result = runner(argv, timeout=10.0, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    except (OSError, subprocess.SubprocessError) as error:
        raise AttestationError("ATTESTATION_DOCKER") from error
    raw = getattr(result, "stdout", None)
    if getattr(result, "returncode", 1) != 0 or not isinstance(raw, bytes) or len(raw) > 4096:
        raise AttestationError("ATTESTATION_DOCKER")
    try:
        text = raw.decode("utf-8")
    except UnicodeError as error:
        raise AttestationError("ATTESTATION_DOCKER") from error
    if not text.endswith("\n") or text.count("\n") != 1:
        raise AttestationError("ATTESTATION_DOCKER")
    fields = text[:-1].split("\t")
    if len(fields) != 14:
        raise AttestationError("ATTESTATION_DOCKER")
    (ident, running, restarting, oom, health, restarts, project, image,
     exit_code, status, paused, dead, started, finished) = fields
    if (not re.fullmatch(r"[0-9a-f]{64}", ident) or not re.fullmatch(r"sha256:[0-9a-f]{64}", image)
            or not re.fullmatch(r"[0-9]{1,9}", restarts) or not re.fullmatch(r"-?[0-9]{1,4}", exit_code)
            or health not in HEALTH
            or any(flag not in ("true", "false") for flag in (running, restarting, oom, paused, dead))):
        raise AttestationError("ATTESTATION_DOCKER")
    if (status != "exited" or project != PROJECT
            or (running, restarting, oom, paused, dead) != ("false",) * 5
            or exit_code != str(expected_exit)):
        raise AttestationError("ATTESTATION_READER_STATE")
    return {
        "container_id": ident, "container_name": name, "dead": False, "exit_code": int(exit_code),
        "finished_at_utc": finished, "health_status": health, "image_id": image, "oom_killed": False,
        "paused": False, "restarting": False, "restart_count": int(restarts), "running": False,
        "started_at_utc": started, "status": status,
    }


def observe(runner: Callable[..., Any], template: str) -> list[dict[str, Any]]:
    return [_inspect_one(name, code, runner, template) for name, code in READERS_V1]


def _timestamp(ns: int) -> str:
    seconds, fraction = divmod(ns, 1_000_000_000)
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(seconds)) + ".%09dZ" % fraction


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")


def build_document(commit: str, observed_ns: int, lifetime: int, containers: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "containers": containers,
        "expires_at_utc": _timestamp(observed_ns + lifetime * 1_000_000_000),
        "format": FORMAT_V1,
        "observed_at_utc": _timestamp(observed_ns),
        "project": PROJECT,
        "scope": SCOPE,
        "target_commit": commit,
    }


def _check_output(path: str) -> tuple[str, str]:
    if (not path.startswith("/") or path.startswith("//") or any(ord(ch) < 32 or ord(ch) == 127 for ch in path)
            or os.path.normpath(path) != path or path.endswith("/")):
        raise AttestationError("ATTESTATION_OUTPUT_PATH")
    parent, base = os.path.split(path)
    if not base or base in (".", "..") or os.path.realpath(parent) != parent:
        raise AttestationError("ATTESTATION_OUTPUT_PATH")
    return parent, base


def _check_directory(parent: str) -> None:
    try:
        info = os.lstat(parent)
    except OSError as error:
        raise AttestationError("ATTESTATION_OUTPUT_PATH") from error
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid()
            or stat.S_IMODE(info.st_mode) & 0o022):
        raise AttestationError("ATTESTATION_OUTPUT_PATH")


def _exists(path: str) -> bool:
    try:
        os.lstat(path)
    except FileNotFoundError:
        return False
    except OSError as error:
        raise AttestationError("ATTESTATION_OUTPUT_PATH") from error
    return True


def _write_private(directory: str, prefix: str, data: bytes) -> str:
    for _ in range(8):
        tmp = os.path.join(directory, "%s.tmp.%s" % (prefix, secrets.token_hex(8)))
        try:
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | getattr(os, "O_CLOEXEC", 0), 0o600)
        except FileExistsError:
            continue
        except OSError as error:
            raise AttestationError("ATTESTATION_OUTPUT_PATH") from error
        try:
            os.fchmod(fd, 0o600)
            view = memoryview(data)
            while view:
                view = view[os.write(fd, view):]
            os.fsync(fd)
        except OSError as error:
            os.close(fd)
            _unlink(tmp)
            raise AttestationError("ATTESTATION_OUTPUT_PATH") from error
        os.close(fd)
        return tmp
    raise AttestationError("ATTESTATION_OUTPUT_PATH")


def _unlink(path: str) -> None:
    try:
        os.unlink(path)
    except OSError:
        pass


def validate_with_real_validator(path: str, commit: str) -> None:
    try:
        result = subprocess.run(
            ["bash", "-c", 'source "$1"; rbl_drained_readers_attestation_binding "$2" "$3"',
             "attestation", str(HELPER), path, commit],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False, timeout=30,
            env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C", "TZ": "UTC"})
    except (OSError, subprocess.SubprocessError) as error:
        raise AttestationError("ATTESTATION_VALIDATOR") from error
    if result.returncode != 0:
        raise AttestationError("ATTESTATION_VALIDATOR")


def run(mode: str, commit: str, output: str, lifetime: int, fmt: str,
        runner: Callable[..., Any] = _default_runner, clock: Callable[[], int] = time.time_ns,
        out: Any = None) -> int:
    out = out or sys.stdout
    if fmt == "v2":
        raise AttestationError("ATTESTATION_V2_UNSPECIFIED")
    if (fmt != "v1" or not re.fullmatch(r"[0-9a-f]{40}", commit) or set(commit) == {"0"}
            or not 1 <= lifetime <= MAX_LIFETIME_SECONDS):
        raise AttestationError("ATTESTATION_ARGUMENTS")
    parent, base = _check_output(output)
    if mode == "plan":
        print("%s: PLAN format=v1 readers=%d" % (PREFIX, len(READERS_V1)), file=out)
        return 0
    _check_directory(parent)
    if _exists(output):
        raise AttestationError("ATTESTATION_OUTPUT_EXISTS")
    template = _template()
    first = observe(runner, template)
    document = build_document(commit, clock(), lifetime, first)
    data = canonical(document)
    if mode == "preflight":
        scratch = os.path.realpath(tempfile.mkdtemp(prefix="drained-readers-preflight-"))
        try:
            tmp = _write_private(scratch, "preflight", data)
            validate_with_real_validator(tmp, commit)
            if observe(runner, template) != first:
                raise AttestationError("ATTESTATION_READER_CHANGED")
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
        print("%s: PREFLIGHT format=v1 readers=%d" % (PREFIX, len(READERS_V1)), file=out)
        for item in first:
            print("reader=%s exit_code=%d status=%s" % (item["container_name"], item["exit_code"], item["status"]), file=out)
        return 0
    tmp = _write_private(parent, base, data)
    try:
        validate_with_real_validator(tmp, commit)
        if observe(runner, template) != first:
            raise AttestationError("ATTESTATION_READER_CHANGED")
        try:
            os.link(tmp, output)
        except FileExistsError as error:
            raise AttestationError("ATTESTATION_OUTPUT_EXISTS") from error
        except OSError as error:
            raise AttestationError("ATTESTATION_OUTPUT_PATH") from error
    finally:
        _unlink(tmp)
    try:
        fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    except OSError:
        pass
    print("%s: APPLIED format=v1 readers=%d" % (PREFIX, len(READERS_V1)), file=out)
    return 0


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:  # value-free
        print("ATTESTATION_ARGUMENTS", file=sys.stderr)
        raise SystemExit(2)


def _cli(argv: list[str] | None = None, runner: Callable[..., Any] = _default_runner) -> int:
    parser = _Parser(description=__doc__, add_help=True)
    modes = parser.add_mutually_exclusive_group(required=True)
    for mode in ("plan", "preflight", "apply"):
        modes.add_argument("--" + mode, dest="mode", action="store_const", const=mode)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--lifetime-seconds", required=True, type=int)
    parser.add_argument("--format", choices=("v1", "v2"), default="v1")
    args = parser.parse_args(argv)
    try:
        return run(args.mode, args.commit, args.output, args.lifetime_seconds, args.format, runner=runner)
    except AttestationError as error:
        print(error.code, file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(_cli())
