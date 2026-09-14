#!/usr/bin/env python3
"""Install the bounded operational KRX intraday session-window artifact.

The command has no provider, database, Docker, or environment-file inputs. It
validates the complete source document before any target mutation, publishes a
content-addressed immutable window, and replaces the activation descriptor
only after that window is durable.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import stat
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping
from urllib.parse import urlsplit


MAX_WINDOW_BYTES = 1_048_576
MAX_ACTIVATION_BYTES = 4096
SCHEMA_VERSION = 1
EXCHANGE = "KRX"
TIMEZONE = "Asia/Seoul"
ACTIVATION_NAME = "activation.json"
WINDOW_NAME_RE = re.compile(r"^windows-[0-9a-f]{64}\.json$")
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
TIME_RE = re.compile(r"^(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d$")
RFC3339_RE = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}"
    r"(?:\.\d{1,6})?(?:Z|[+-]\d{2}:\d{2})$"
)
KST = dt.timezone(dt.timedelta(hours=9))

DIRECTORY_UID = 0
DIRECTORY_GID = 10001
DIRECTORY_MODE = 0o750
FILE_UID = 0
FILE_GID = 10001
FILE_MODE = 0o640

WINDOW_KEYS = frozenset(
    {"schema_version", "exchange", "timezone", "entries"}
)
ENTRY_KEYS = frozenset(
    {
        "date",
        "disposition",
        "open_local",
        "close_local",
        "evidence_url",
        "evidence_retrieved_at",
        "evidence_sha256",
    }
)
ACTIVATION_KEYS = frozenset({"schema_version", "window_sha256"})


class InstallerError(Exception):
    """A fixed outward error code; no path, content, or provider detail."""

    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


@dataclass(frozen=True)
class PreparedWindow:
    raw: bytes
    window_sha256: str
    requested_date: dt.date


@dataclass(frozen=True)
class FileMetadata:
    is_directory: bool
    is_symlink: bool
    is_regular: bool
    uid: int
    gid: int
    mode: int
    link_count: int
    size: int


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise InstallerError("JSON_INVALID")
        result[key] = value
    return result


def _reject_nonfinite(value: str) -> None:
    del value
    raise InstallerError("JSON_INVALID")


def _parse_json(raw: bytes, maximum: int) -> Any:
    if not isinstance(raw, bytes) or not raw or len(raw) > maximum:
        raise InstallerError("JSON_INVALID")
    try:
        text = raw.decode("utf-8")
        return json.loads(
            text,
            object_pairs_hook=_reject_duplicate_keys,
            parse_constant=_reject_nonfinite,
        )
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise InstallerError("JSON_INVALID") from None


def _require_object(value: Any, code: str = "JSON_INVALID") -> dict[str, Any]:
    if type(value) is not dict:
        raise InstallerError(code)
    return value


def _require_exact_keys(value: Mapping[str, Any], expected: frozenset[str]) -> None:
    if frozenset(value) != expected:
        raise InstallerError("SCHEMA_INVALID")


def _require_string(value: Any) -> str:
    if type(value) is not str:
        raise InstallerError("SCHEMA_INVALID")
    return value


def _require_version(value: Any) -> None:
    if type(value) is not int or value != SCHEMA_VERSION:
        raise InstallerError("SCHEMA_INVALID")


def _parse_requested_date(value: str) -> dt.date:
    if type(value) is not str or not DATE_RE.fullmatch(value):
        raise InstallerError("DATE_INVALID")
    try:
        return dt.date.fromisoformat(value)
    except ValueError:
        raise InstallerError("DATE_INVALID") from None


def _parse_entry_date(value: Any) -> dt.date:
    value = _require_string(value)
    if not DATE_RE.fullmatch(value):
        raise InstallerError("SCHEMA_INVALID")
    try:
        return dt.date.fromisoformat(value)
    except ValueError:
        raise InstallerError("SCHEMA_INVALID") from None


def _parse_time(value: Any) -> dt.time:
    value = _require_string(value)
    if not TIME_RE.fullmatch(value):
        raise InstallerError("SCHEMA_INVALID")
    hour, minute, second = (int(part) for part in value.split(":"))
    return dt.time(hour, minute, second)


def _parse_evidence_timestamp(value: Any, now: dt.datetime, entry_date: dt.date) -> None:
    value = _require_string(value)
    if not RFC3339_RE.fullmatch(value):
        raise InstallerError("EVIDENCE_INVALID")
    normalized = f"{value[:-1]}+00:00" if value.endswith("Z") else value
    try:
        observed = dt.datetime.fromisoformat(normalized)
    except ValueError:
        raise InstallerError("EVIDENCE_INVALID") from None
    if observed.tzinfo is None or observed.utcoffset() is None:
        raise InstallerError("EVIDENCE_INVALID")
    if observed > now or observed.astimezone(KST).date() != entry_date:
        raise InstallerError("EVIDENCE_INVALID")


def _validate_evidence_url(value: Any) -> None:
    value = _require_string(value)
    if not value.startswith("https://global.krx.co.kr/") or any(
        ord(char) < 0x20 for char in value
    ):
        raise InstallerError("EVIDENCE_INVALID")
    try:
        parsed = urlsplit(value)
        port = parsed.port
    except ValueError:
        raise InstallerError("EVIDENCE_INVALID") from None
    if (
        parsed.scheme != "https"
        or parsed.hostname != "global.krx.co.kr"
        or port is not None
        or parsed.username is not None
        or parsed.password is not None
        or not parsed.path.startswith("/")
    ):
        raise InstallerError("EVIDENCE_INVALID")


def _validate_hash(value: Any) -> str:
    value = _require_string(value)
    if not HASH_RE.fullmatch(value):
        raise InstallerError("HASH_INVALID")
    return value


def parse_activation(raw: bytes) -> str:
    """Parse the closed activation schema and return its canonical hash."""

    value = _require_object(_parse_json(raw, MAX_ACTIVATION_BYTES))
    _require_exact_keys(value, ACTIVATION_KEYS)
    _require_version(value["schema_version"])
    return _validate_hash(value["window_sha256"])


def window_filename(window_sha256: str) -> str:
    """Derive the only acceptable operational window filename."""

    window_sha256 = _validate_hash(window_sha256)
    filename = f"windows-{window_sha256[7:]}.json"
    if not WINDOW_NAME_RE.fullmatch(filename):
        raise InstallerError("HASH_INVALID")
    return filename


def parse_window_document(
    raw: bytes,
    requested_date: str,
    now: dt.datetime,
) -> PreparedWindow:
    """Validate the existing window contract without changing its bytes."""

    requested = _parse_requested_date(requested_date)
    if now.tzinfo is None or now.utcoffset() is None:
        raise InstallerError("CLOCK_INVALID")
    if now.astimezone(KST).date() != requested:
        raise InstallerError("DATE_NOT_CURRENT")
    return _parse_window_document_with_clock(raw, requested, now)


def parse_window_document_at(
    raw: bytes,
    requested_date: str,
    now: dt.datetime,
) -> PreparedWindow:
    """Alias making the injected-clock seam explicit to offline tests."""

    return parse_window_document(raw, requested_date, now)


def _parse_window_document_with_clock(
    raw: bytes,
    requested: dt.date,
    now: dt.datetime,
) -> PreparedWindow:
    value = _require_object(_parse_json(raw, MAX_WINDOW_BYTES))
    _require_exact_keys(value, WINDOW_KEYS)
    _require_version(value["schema_version"])
    if value["exchange"] != EXCHANGE or value["timezone"] != TIMEZONE:
        raise InstallerError("SCHEMA_INVALID")
    if type(value["entries"]) is not list or not value["entries"]:
        raise InstallerError("SCHEMA_INVALID")

    previous_date: dt.date | None = None
    requested_entry = False
    for wire_entry in value["entries"]:
        entry = _require_object(wire_entry)
        _require_exact_keys(entry, ENTRY_KEYS)
        entry_date = _parse_entry_date(entry["date"])
        if previous_date is not None and entry_date <= previous_date:
            raise InstallerError("SCHEMA_INVALID")
        previous_date = entry_date
        requested_entry = requested_entry or entry_date == requested
        disposition = _require_string(entry["disposition"])
        open_local = entry["open_local"]
        close_local = entry["close_local"]
        if disposition == "CLOSED":
            if open_local is not None or close_local is not None:
                raise InstallerError("SCHEMA_INVALID")
        elif disposition == "REGULAR":
            if open_local != "09:00:00" or close_local != "15:30:00":
                raise InstallerError("SCHEMA_INVALID")
        elif disposition == "SPECIAL":
            if open_local is None or close_local is None:
                raise InstallerError("SCHEMA_INVALID")
            if _parse_time(open_local) >= _parse_time(close_local):
                raise InstallerError("SCHEMA_INVALID")
        else:
            raise InstallerError("SCHEMA_INVALID")
        _validate_evidence_url(entry["evidence_url"])
        _parse_evidence_timestamp(entry["evidence_retrieved_at"], now, entry_date)
        _validate_hash(entry["evidence_sha256"])
    if not requested_entry:
        raise InstallerError("DATE_ENTRY_MISSING")
    return PreparedWindow(raw, f"sha256:{hashlib.sha256(raw).hexdigest()}", requested)


def validate_directory_metadata(metadata: FileMetadata) -> None:
    if (
        metadata.is_symlink
        or not metadata.is_directory
        or metadata.uid != DIRECTORY_UID
        or metadata.gid != DIRECTORY_GID
        or metadata.mode != DIRECTORY_MODE
    ):
        raise InstallerError("ROOT_METADATA_INVALID")


def validate_file_metadata(metadata: FileMetadata) -> None:
    if (
        metadata.is_symlink
        or metadata.is_directory
        or not metadata.is_regular
        or metadata.uid != FILE_UID
        or metadata.gid != FILE_GID
        or metadata.mode != FILE_MODE
        or metadata.link_count != 1
    ):
        raise InstallerError("FILE_METADATA_INVALID")


def _metadata_from_stat(value: os.stat_result) -> FileMetadata:
    return FileMetadata(
        is_directory=stat.S_ISDIR(value.st_mode),
        is_symlink=stat.S_ISLNK(value.st_mode),
        is_regular=stat.S_ISREG(value.st_mode),
        uid=value.st_uid,
        gid=value.st_gid,
        mode=stat.S_IMODE(value.st_mode),
        link_count=value.st_nlink,
        size=value.st_size,
    )


def _safe_path(value: str, code: str) -> str:
    if type(value) is not str or not value or not os.path.isabs(value):
        raise InstallerError(code)
    if "\x00" in value or value != os.path.normpath(value):
        raise InstallerError(code)
    if value != "/" and value.endswith("/"):
        raise InstallerError(code)
    parts = value.split("/")[1:]
    if any(not part or part in {".", ".."} for part in parts):
        raise InstallerError(code)
    return value


def _overlaps(left: str, right: str) -> bool:
    left = left.rstrip("/") or "/"
    right = right.rstrip("/") or "/"
    return left == right or left.startswith(f"{right}/") or right.startswith(f"{left}/")


def validate_operational_paths(input_path: str | None, root_path: str) -> tuple[str | None, str]:
    input_path = _safe_path(input_path, "INPUT_PATH_INVALID") if input_path is not None else None
    root_path = _safe_path(root_path, "ROOT_PATH_INVALID")
    if root_path in {
        "/",
        "/etc",
        "/opt",
        "/var",
        "/var/lib",
        "/usr",
        "/usr/local",
        "/tmp",
        "/home",
        "/run",
        "/data",
    }:
        raise InstallerError("ROOT_PATH_BROAD")
    if input_path is not None and _overlaps(input_path, root_path):
        raise InstallerError("PATH_OVERLAP")
    return input_path, root_path


def _lstat(path: str, *, dir_fd: int | None = None) -> os.stat_result | None:
    try:
        return os.stat(path, dir_fd=dir_fd, follow_symlinks=False)
    except FileNotFoundError:
        return None
    except OSError:
        raise InstallerError("PATH_UNREADABLE") from None


def _validate_nonsymlink_ancestors(path: str, missing_code: str, unsafe_code: str) -> None:
    current = Path("/")
    for component in Path(path).parts[1:]:
        current /= component
        metadata = _lstat(str(current))
        if metadata is None:
            raise InstallerError(missing_code)
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise InstallerError(unsafe_code)


def _validate_existing_ancestors(path: str) -> None:
    _validate_nonsymlink_ancestors(path, "ROOT_PARENT_MISSING", "ROOT_PATH_UNSAFE")
    current = Path("/")
    for component in Path(path).parts[1:]:
        current /= component
        metadata = _lstat(str(current))
        if metadata is None or metadata.st_uid != 0 or stat.S_IMODE(metadata.st_mode) & 0o022:
            raise InstallerError("ROOT_PATH_UNSAFE")


def _validate_root_path(root_path: str, *, allow_create: bool) -> bool:
    parent = str(Path(root_path).parent)
    _validate_existing_ancestors(parent)
    parent_metadata = _lstat(parent)
    if parent_metadata is None:
        raise InstallerError("ROOT_PARENT_MISSING")
    validate_directory_metadata(_metadata_from_stat(parent_metadata))
    root_metadata = _lstat(root_path)
    if root_metadata is None:
        if allow_create:
            return True
        raise InstallerError("ROOT_MISSING")
    validate_directory_metadata(_metadata_from_stat(root_metadata))
    return False


def _open_directory(path: str) -> int:
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    try:
        return os.open(path, flags)
    except FileNotFoundError:
        raise InstallerError("ROOT_MISSING") from None
    except OSError:
        raise InstallerError("ROOT_PATH_UNSAFE") from None


def _read_fd(fd: int, maximum: int) -> bytes:
    before = os.fstat(fd)
    before_metadata = _metadata_from_stat(before)
    validate_file_metadata(before_metadata)
    if before.st_size < 0 or before.st_size > maximum:
        raise InstallerError("FILE_TOO_LARGE")
    chunks: list[bytes] = []
    total = 0
    while total <= maximum:
        chunk = os.read(fd, min(65536, maximum + 1 - total))
        if not chunk:
            break
        chunks.append(chunk)
        total += len(chunk)
        if total > maximum:
            raise InstallerError("FILE_TOO_LARGE")
    after = os.fstat(fd)
    after_metadata = _metadata_from_stat(after)
    validate_file_metadata(after_metadata)
    if (
        before.st_dev != after.st_dev
        or before.st_ino != after.st_ino
        or before.st_size != after.st_size
        or total != before.st_size
    ):
        raise InstallerError("FILE_CHANGED")
    return b"".join(chunks)


def _read_named_file(directory_fd: int, name: str, maximum: int) -> bytes | None:
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK
    try:
        fd = os.open(name, flags, dir_fd=directory_fd)
    except FileNotFoundError:
        return None
    except OSError:
        raise InstallerError("FILE_METADATA_INVALID") from None
    try:
        return _read_fd(fd, maximum)
    finally:
        os.close(fd)


def _read_input_file(path: str) -> bytes:
    _validate_nonsymlink_ancestors(
        str(Path(path).parent), "INPUT_PARENT_MISSING", "INPUT_PATH_UNSAFE"
    )
    metadata = _lstat(path)
    if metadata is None:
        raise InstallerError("INPUT_MISSING")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise InstallerError("INPUT_PATH_UNSAFE")
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK
    try:
        fd = os.open(path, flags)
    except OSError:
        raise InstallerError("INPUT_UNREADABLE") from None
    try:
        return _read_fd_for_input(fd)
    finally:
        os.close(fd)


def _read_fd_for_input(fd: int) -> bytes:
    before = os.fstat(fd)
    if not stat.S_ISREG(before.st_mode) or before.st_size < 0 or before.st_size > MAX_WINDOW_BYTES:
        raise InstallerError("INPUT_INVALID")
    chunks: list[bytes] = []
    total = 0
    while total <= MAX_WINDOW_BYTES:
        chunk = os.read(fd, min(65536, MAX_WINDOW_BYTES + 1 - total))
        if not chunk:
            break
        chunks.append(chunk)
        total += len(chunk)
        if total > MAX_WINDOW_BYTES:
            raise InstallerError("INPUT_TOO_LARGE")
    after = os.fstat(fd)
    if (
        not stat.S_ISREG(after.st_mode)
        or before.st_mode != after.st_mode
        or before.st_uid != after.st_uid
        or before.st_gid != after.st_gid
        or before.st_dev != after.st_dev
        or before.st_ino != after.st_ino
        or before.st_size != after.st_size
        or total != before.st_size
    ):
        raise InstallerError("INPUT_CHANGED")
    return b"".join(chunks)


def _current_now() -> dt.datetime:
    return dt.datetime.now(dt.timezone.utc)


def _validate_input(path: str, requested_date: str, now: dt.datetime) -> PreparedWindow:
    raw = _read_input_file(path)
    return parse_window_document_at(raw, requested_date, now)


def _ensure_root(root_path: str) -> tuple[int, int | None]:
    create = _validate_root_path(root_path, allow_create=True)
    parent_fd: int | None = None
    if create:
        parent_fd = _open_directory(str(Path(root_path).parent))
        created = False
        try:
            try:
                os.mkdir(Path(root_path).name, DIRECTORY_MODE, dir_fd=parent_fd)
                created = True
            except FileExistsError:
                pass
            fd = _open_directory(root_path)
            try:
                if created:
                    os.fchown(fd, DIRECTORY_UID, DIRECTORY_GID)
                    os.fchmod(fd, DIRECTORY_MODE)
                validate_directory_metadata(_metadata_from_stat(os.fstat(fd)))
            except Exception:
                os.close(fd)
                raise
            os.fsync(parent_fd)
            return fd, parent_fd
        except InstallerError:
            raise
        except OSError:
            raise InstallerError("ROOT_CREATE_FAILED") from None
    fd = _open_directory(root_path)
    try:
        validate_directory_metadata(_metadata_from_stat(os.fstat(fd)))
    except Exception:
        os.close(fd)
        raise
    return fd, parent_fd


def _write_staged(directory_fd: int, name: str, raw: bytes) -> None:
    stage_name = f".{name}.tmp-{os.getpid()}"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    try:
        fd = os.open(stage_name, flags, FILE_MODE, dir_fd=directory_fd)
    except OSError:
        raise InstallerError("WRITE_FAILED") from None
    try:
        os.fchown(fd, FILE_UID, FILE_GID)
        os.fchmod(fd, FILE_MODE)
        offset = 0
        while offset < len(raw):
            offset += os.write(fd, raw[offset:])
        os.fsync(fd)
        validate_file_metadata(_metadata_from_stat(os.fstat(fd)))
    except InstallerError:
        try:
            os.close(fd)
        finally:
            try:
                os.unlink(stage_name, dir_fd=directory_fd)
            except OSError:
                pass
        raise
    except OSError:
        try:
            os.close(fd)
        finally:
            try:
                os.unlink(stage_name, dir_fd=directory_fd)
            except OSError:
                pass
        raise InstallerError("WRITE_FAILED") from None
    else:
        os.close(fd)

    try:
        os.link(
            stage_name,
            name,
            src_dir_fd=directory_fd,
            dst_dir_fd=directory_fd,
            follow_symlinks=False,
        )
    except FileExistsError:
        try:
            existing = _read_named_file(directory_fd, name, MAX_WINDOW_BYTES)
            if existing != raw:
                raise InstallerError("IMMUTABLE_MISMATCH")
        finally:
            try:
                os.unlink(stage_name, dir_fd=directory_fd)
            except OSError:
                pass
        return
    except OSError:
        try:
            os.unlink(stage_name, dir_fd=directory_fd)
        except OSError:
            pass
        raise InstallerError("WRITE_FAILED") from None
    try:
        os.unlink(stage_name, dir_fd=directory_fd)
        os.fsync(directory_fd)
    except OSError:
        raise InstallerError("WRITE_FAILED") from None


def _replace_activation(directory_fd: int, window_sha256: str) -> None:
    stage_name = f".{ACTIVATION_NAME}.tmp-{os.getpid()}"
    raw = json.dumps(
        {"schema_version": SCHEMA_VERSION, "window_sha256": window_sha256},
        separators=(",", ":"),
        ensure_ascii=True,
    ).encode("ascii")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    try:
        fd = os.open(stage_name, flags, FILE_MODE, dir_fd=directory_fd)
    except OSError:
        raise InstallerError("WRITE_FAILED") from None
    try:
        os.fchown(fd, FILE_UID, FILE_GID)
        os.fchmod(fd, FILE_MODE)
        offset = 0
        while offset < len(raw):
            offset += os.write(fd, raw[offset:])
        os.fsync(fd)
        validate_file_metadata(_metadata_from_stat(os.fstat(fd)))
    except (OSError, InstallerError):
        os.close(fd)
        try:
            os.unlink(stage_name, dir_fd=directory_fd)
        except OSError:
            pass
        raise InstallerError("WRITE_FAILED") from None
    else:
        os.close(fd)
    try:
        os.replace(stage_name, ACTIVATION_NAME, src_dir_fd=directory_fd, dst_dir_fd=directory_fd)
        os.fsync(directory_fd)
    except OSError:
        try:
            os.unlink(stage_name, dir_fd=directory_fd)
        except OSError:
            pass
        raise InstallerError("WRITE_FAILED") from None


def apply(prepared: PreparedWindow, root_path: str) -> None:
    if os.geteuid() != 0:
        raise InstallerError("ROOT_REQUIRED")
    directory_fd, parent_fd = _ensure_root(root_path)
    try:
        existing_activation = _read_named_file(
            directory_fd, ACTIVATION_NAME, MAX_ACTIVATION_BYTES
        )
        if existing_activation is not None:
            parse_activation(existing_activation)
        target_name = window_filename(prepared.window_sha256)
        existing_window = _read_named_file(directory_fd, target_name, MAX_WINDOW_BYTES)
        if existing_window is not None and existing_window != prepared.raw:
            raise InstallerError("IMMUTABLE_MISMATCH")
        if existing_window is None:
            _write_staged(directory_fd, target_name, prepared.raw)
        _replace_activation(directory_fd, prepared.window_sha256)
        if parent_fd is not None:
            os.fsync(parent_fd)
    except InstallerError:
        raise
    except OSError:
        raise InstallerError("WRITE_FAILED") from None
    finally:
        os.close(directory_fd)
        if parent_fd is not None:
            os.close(parent_fd)


def check(root_path: str, requested_date: str, now: dt.datetime) -> None:
    _validate_root_path(root_path, allow_create=False)
    directory_fd = _open_directory(root_path)
    try:
        activation_raw = _read_named_file(directory_fd, ACTIVATION_NAME, MAX_ACTIVATION_BYTES)
        if activation_raw is None:
            raise InstallerError("ACTIVATION_MISSING")
        window_sha256 = parse_activation(activation_raw)
        target_name = window_filename(window_sha256)
        window_raw = _read_named_file(directory_fd, target_name, MAX_WINDOW_BYTES)
        if window_raw is None:
            raise InstallerError("WINDOW_MISSING")
        actual = f"sha256:{hashlib.sha256(window_raw).hexdigest()}"
        if actual != window_sha256:
            raise InstallerError("WINDOW_HASH_MISMATCH")
        parse_window_document_at(window_raw, requested_date, now)
    finally:
        os.close(directory_fd)


class _ArgumentParser(argparse.ArgumentParser):
    def error(self, _message: str) -> None:
        raise InstallerError("ARGUMENTS_INVALID")


def _parser() -> argparse.ArgumentParser:
    parser = _ArgumentParser(
        description="Install or check the bounded operational intraday session-window artifact."
    )
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--check", action="store_true")
    modes.add_argument("--apply", action="store_true")
    parser.add_argument("--input", dest="input_path")
    parser.add_argument("--root", dest="root_path", required=True)
    parser.add_argument("--date", dest="requested_date", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    try:
        args = _parser().parse_args(argv)
        input_path, root_path = validate_operational_paths(args.input_path, args.root_path)
        now = _current_now()
        requested = _parse_requested_date(args.requested_date)
        if now.astimezone(KST).date() != requested:
            raise InstallerError("DATE_NOT_CURRENT")
        if args.apply:
            if input_path is None:
                raise InstallerError("INPUT_REQUIRED")
            prepared = _validate_input(input_path, args.requested_date, now)
            apply(prepared, root_path)
            print("INTRADAY_SESSION_WINDOW_APPLY: PASS")
        else:
            if input_path is not None:
                _validate_input(input_path, args.requested_date, now)
            check(root_path, args.requested_date, now)
            print("INTRADAY_SESSION_WINDOW_CHECK: PASS")
        return 0
    except InstallerError as error:
        print(f"INTRADAY_SESSION_WINDOW_{error.code}", file=sys.stderr)
        return 2
    except (OSError, ValueError, TypeError):
        print("INTRADAY_SESSION_WINDOW_INTERNAL", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
