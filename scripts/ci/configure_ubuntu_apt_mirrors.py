"""Prefer configured canonical Ubuntu mirrors on ephemeral GitHub Linux runners."""

from __future__ import annotations

import errno
import os
import platform
import secrets
import stat
import sys
from pathlib import Path

MAX_CONFIG_BYTES = 16 * 1024
MIRROR_FILE = Path("/etc/apt/apt-mirrors.txt")
AZURE = "http://azure.archive.ubuntu.com/ubuntu/"
ARCHIVE = "https://archive.ubuntu.com/ubuntu/"
SECURITY = "https://security.ubuntu.com/ubuntu/"
PRIORITIES = {ARCHIVE: 1, SECURITY: 2, AZURE: 3}


class MirrorConfigError(Exception):
    """A bounded, non-sensitive configuration refusal."""


def transform_mirror_config(text: str) -> tuple[str, str]:
    """Return (text, status), changing only priorities for the exact known mirrors."""
    if len(text) > MAX_CONFIG_BYTES:
        raise MirrorConfigError("CONFIG_TOO_LARGE")
    try:
        encoded = text.encode("utf-8", errors="strict")
    except UnicodeEncodeError as exc:
        raise MirrorConfigError("INVALID_UTF8") from exc
    transformed, status = _transform_bytes(encoded)
    return transformed.decode("utf-8"), status


def _transform_bytes(raw: bytes) -> tuple[bytes, str]:
    if len(raw) > MAX_CONFIG_BYTES:
        raise MirrorConfigError("CONFIG_TOO_LARGE")
    try:
        text = raw.decode("utf-8", errors="strict")
    except UnicodeDecodeError as exc:
        raise MirrorConfigError("INVALID_UTF8") from exc

    lines = text.splitlines(keepends=True)
    azure_present = False
    for line in lines:
        body, _ = _split_ending(line)
        if body.lstrip().startswith("#") or not body.strip():
            continue
        candidate = body.lstrip()
        if candidate == AZURE or (
            candidate.startswith(AZURE)
            and len(candidate) > len(AZURE)
            and candidate[len(AZURE)] in "\t "
        ):
            azure_present = True

    if not azure_present:
        return raw, "UNCHANGED"

    # With the Azure source present, require the complete known runner mirror set and
    # reject unrecognized records instead of guessing how they should be prioritized.
    records: dict[str, list[tuple[int, str, str]]] = {uri: [] for uri in PRIORITIES}
    for index, line in enumerate(lines):
        body, _ = _split_ending(line)
        if body.lstrip().startswith("#") or not body.strip():
            continue
        fields = body.split("\t")
        if len(fields) != 2 or fields[0] not in PRIORITIES:
            raise MirrorConfigError("UNKNOWN_RECORD")
        value = fields[1][len("priority:"):] if fields[1].startswith("priority:") else ""
        if not value.isascii() or not value.isdecimal():
            raise MirrorConfigError("MALFORMED_PRIORITY")
        _, ending = _split_ending(line)
        records[fields[0]].append((index, value, ending))

    for uri in PRIORITIES:
        if len(records[uri]) != 1:
            raise MirrorConfigError("MISSING_OR_DUPLICATE_MIRROR")

    output = list(lines)
    changed = False
    for uri, desired in PRIORITIES.items():
        index, current, ending = records[uri][0]
        if current.lstrip("0") != str(desired):
            output[index] = f"{uri}\tpriority:{desired}{ending}"
            changed = True
    return ("".join(output).encode("utf-8"), "UPDATED" if changed else "UNCHANGED")


def _split_ending(line: str) -> tuple[str, str]:
    if line.endswith("\r\n"):
        return line[:-2], "\r\n"
    if line.endswith("\n") or line.endswith("\r"):
        return line[:-1], line[-1:]
    return line, ""


def _directory_fd(path: Path) -> int:
    raw_path = os.fspath(path)
    if not os.path.isabs(raw_path) or ".." in raw_path.split("/"):
        raise MirrorConfigError("UNSAFE_PATH")
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_CLOEXEC", 0)
    nofollow = getattr(os, "O_NOFOLLOW", 0)
    current = os.open("/", flags | nofollow)
    try:
        for component in path.parts[1:]:
            if component in ("", ".", ".."):
                raise MirrorConfigError("UNSAFE_PATH")
            child = os.open(component, flags | nofollow, dir_fd=current)
            os.close(current)
            current = child
        return current
    except BaseException:
        os.close(current)
        raise


def _identity(info: os.stat_result) -> tuple[int, int]:
    return info.st_dev, info.st_ino


def _version(info: os.stat_result) -> tuple[int, int, int, int, int, int]:
    return (
        info.st_dev,
        info.st_ino,
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
        stat.S_IMODE(info.st_mode),
    )


def _validate_parent(info: os.stat_result) -> None:
    if (
        not stat.S_ISDIR(info.st_mode)
        or info.st_uid != os.geteuid()
        or info.st_gid != os.getegid()
        or info.st_mode & (stat.S_IWGRP | stat.S_IWOTH)
    ):
        raise MirrorConfigError("UNSAFE_PARENT")


def _validate_file(info: os.stat_result) -> None:
    if (
        not stat.S_ISREG(info.st_mode)
        or info.st_nlink != 1
        or info.st_uid != os.geteuid()
        or info.st_gid != os.getegid()
        or info.st_mode & (stat.S_IWGRP | stat.S_IWOTH)
    ):
        raise MirrorConfigError("UNSAFE_FILE")


def configure_mirror_file(path: Path) -> str:
    """Safely rewrite one existing file; this path-taking API is for tests only."""
    if path.name in ("", ".", "..") or "/" in path.name:
        raise MirrorConfigError("UNSAFE_PATH")

    parent_fd = _directory_fd(path.parent)
    source_fd: int | None = None
    temp_fd: int | None = None
    temp_name: str | None = None
    temp_identity: tuple[int, int] | None = None
    renamed = False
    try:
        parent_info = os.fstat(parent_fd)
        _validate_parent(parent_info)
        parent_identity = _identity(parent_info)
        try:
            pre_open_info = os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
        except FileNotFoundError:
            return "NOOP_MISSING"
        _validate_file(pre_open_info)
        open_flags = (
            os.O_RDONLY
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0)
            | getattr(os, "O_NONBLOCK", 0)
        )
        try:
            source_fd = os.open(path.name, open_flags, dir_fd=parent_fd)
        except FileNotFoundError:
            return "NOOP_MISSING"
        source_info = os.fstat(source_fd)
        _validate_file(source_info)
        if _version(source_info) != _version(pre_open_info):
            raise MirrorConfigError("SOURCE_REPLACED")
        named_info = os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
        if _identity(named_info) != _identity(source_info):
            raise MirrorConfigError("SOURCE_REPLACED")
        if source_info.st_size > MAX_CONFIG_BYTES:
            raise MirrorConfigError("CONFIG_TOO_LARGE")

        chunks: list[bytes] = []
        total = 0
        while total <= MAX_CONFIG_BYTES:
            chunk = os.read(source_fd, min(4096, MAX_CONFIG_BYTES + 1 - total))
            if not chunk:
                break
            chunks.append(chunk)
            total += len(chunk)
        if total > MAX_CONFIG_BYTES:
            raise MirrorConfigError("CONFIG_TOO_LARGE")
        raw = b"".join(chunks)
        if total != source_info.st_size or _version(os.fstat(source_fd)) != _version(source_info):
            raise MirrorConfigError("SOURCE_CHANGED")
        transformed, status = _transform_bytes(raw)
        if status == "UNCHANGED":
            _recheck_target(path, parent_fd, parent_identity, source_info, source_fd)
            return status

        for _ in range(8):
            candidate = f".apt-mirrors-{secrets.token_hex(8)}.tmp"
            try:
                temp_fd = os.open(
                    candidate,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL
                    | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
                    0o600,
                    dir_fd=parent_fd,
                )
                temp_name = candidate
                break
            except FileExistsError:
                continue
        if temp_fd is None or temp_name is None:
            raise MirrorConfigError("TEMP_CREATE_FAILED")
        temp_info = os.fstat(temp_fd)
        temp_identity = _identity(temp_info)
        os.fchmod(temp_fd, stat.S_IMODE(source_info.st_mode))
        view = memoryview(transformed)
        while view:
            written = os.write(temp_fd, view)
            if written <= 0:
                raise MirrorConfigError("TEMP_WRITE_FAILED")
            view = view[written:]
        os.fsync(temp_fd)

        _recheck_target(path, parent_fd, parent_identity, source_info, source_fd)
        current_parent = os.fstat(parent_fd)
        _validate_parent(current_parent)
        if _identity(current_parent) != parent_identity:
            raise MirrorConfigError("PARENT_REPLACED")
        temp_named = os.stat(temp_name, dir_fd=parent_fd, follow_symlinks=False)
        if _identity(temp_named) != temp_identity:
            raise MirrorConfigError("TEMP_REPLACED")
        os.rename(temp_name, path.name, src_dir_fd=parent_fd, dst_dir_fd=parent_fd)
        renamed = True
        os.fsync(parent_fd)
        return status
    except OSError as exc:
        if exc.errno in (errno.ELOOP, errno.ENOTDIR, errno.EACCES, errno.EPERM):
            raise MirrorConfigError("UNSAFE_PATH") from exc
        raise MirrorConfigError("IO_FAILURE") from exc
    finally:
        if temp_name is not None and not renamed:
            try:
                current_temp = os.stat(temp_name, dir_fd=parent_fd, follow_symlinks=False)
                if temp_identity is not None and _identity(current_temp) == temp_identity:
                    os.unlink(temp_name, dir_fd=parent_fd)
            except FileNotFoundError:
                pass
            except OSError:
                pass
        for fd in (temp_fd, source_fd, parent_fd):
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass


def _recheck_target(
    path: Path,
    parent_fd: int,
    parent_identity: tuple[int, int],
    source_info: os.stat_result,
    source_fd: int,
) -> None:
    held_parent = os.fstat(parent_fd)
    _validate_parent(held_parent)
    if _identity(held_parent) != parent_identity:
        raise MirrorConfigError("PARENT_REPLACED")
    try:
        named_parent_fd = _directory_fd(path.parent)
    except OSError as exc:
        raise MirrorConfigError("PARENT_REPLACED") from exc
    try:
        named_parent = os.fstat(named_parent_fd)
        _validate_parent(named_parent)
        if _identity(named_parent) != parent_identity:
            raise MirrorConfigError("PARENT_REPLACED")
    finally:
        os.close(named_parent_fd)
    held_file = os.fstat(source_fd)
    _validate_file(held_file)
    if _version(held_file) != _version(source_info):
        raise MirrorConfigError("SOURCE_CHANGED")
    try:
        named_file = os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
    except FileNotFoundError as exc:
        raise MirrorConfigError("SOURCE_REPLACED") from exc
    _validate_file(named_file)
    if _version(named_file) != _version(source_info):
        raise MirrorConfigError("SOURCE_REPLACED")


def main() -> int:
    if len(sys.argv) != 1:
        print("REFUSED")
        return 2
    if (
        sys.platform != "linux"
        or platform.machine() != "x86_64"
        or os.geteuid() != 0
        or os.environ.get("GITHUB_ACTIONS") != "true"
        or os.environ.get("RUNNER_OS") != "Linux"
    ):
        print("REFUSED")
        return 2
    try:
        result = configure_mirror_file(MIRROR_FILE)
    except (MirrorConfigError, OSError):
        print("REFUSED")
        return 1
    print(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
