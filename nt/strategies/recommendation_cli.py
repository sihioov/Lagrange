"""Closed-set, file-only target generator boundary for recommendations.

The parent process chooses data, universe, and provenance.  This child only
validates that sealed request, calls one shipped generator, and atomically
publishes canonical JSON.  Stdout is deliberately not a result channel.
"""

from __future__ import annotations

import argparse
import importlib
import json
import math
import os
import re
import stat
import tempfile
import uuid
from datetime import date
from pathlib import Path
from typing import Any

from strategies._common import TargetError, compute_portfolio_snapshot_id


GENERATORS = {
    "buy_and_hold": "strategies.buy_and_hold.target",
    "trend_following": "strategies.trend_following.target",
    "relative_momentum": "strategies.relative_momentum.target",
    "dual_momentum": "strategies.dual_momentum.target",
    "inverse_volatility": "strategies.inverse_volatility.target",
}

SHIPPED_VERSION = "1.0.0"
MAX_REQUEST_BYTES = 1024 * 1024
MAX_RESULT_BYTES = 1024 * 1024
MAX_STATUS_BYTES = 16 * 1024
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
PLAIN_HASH_RE = re.compile(r"^[0-9a-f]{64}$")
FACTOR_RE = re.compile(r"^[a-z0-9_]{1,64}$")
CANONICAL_UNIVERSE = [
    "069500.KRX",
    "102110.KRX",
    "229200.KRX",
    "143850.KRX",
    "133690.KRX",
    "195930.KRX",
    "192090.KRX",
    "148070.KRX",
    "114260.KRX",
    "153130.KRX",
    "132030.KRX",
]


class CliError(Exception):
    def __init__(self, code: str, summary: str):
        super().__init__(summary)
        self.code = code
        self.summary = summary


def _exact_object(value: Any, keys: set[str], label: str) -> dict[str, Any]:
    if type(value) is not dict or set(value) != keys:
        raise CliError("INVALID_REQUEST", f"{label} does not match the required schema")
    return value


def _string(value: Any, label: str) -> str:
    if type(value) is not str or not value or value.strip() != value:
        raise CliError("INVALID_REQUEST", f"{label} must be a non-empty string")
    return value


def _finite(value: Any, label: str) -> float:
    if type(value) not in (int, float) or not math.isfinite(value):
        raise CliError("INVALID_REQUEST", f"{label} must be a finite number")
    return float(value)


def _validate_request(value: Any) -> dict[str, Any]:
    request = _exact_object(
        value,
        {
            "strategy_id",
            "strategy_version",
            "parameters",
            "as_of",
            "universe",
            "factors",
            "provenance",
        },
        "request",
    )
    strategy_id = _string(request["strategy_id"], "strategy_id")
    if strategy_id not in GENERATORS:
        raise CliError("UNKNOWN_STRATEGY", "strategy is not shipped by this build")
    version = _string(request["strategy_version"], "strategy_version")
    if version != SHIPPED_VERSION:
        raise CliError("UNSUPPORTED_VERSION", "strategy version is not shipped by this build")
    if type(request["parameters"]) is not dict:
        raise CliError("INVALID_REQUEST", "parameters must be an object")

    as_of = _string(request["as_of"], "as_of")
    try:
        parsed_date = date.fromisoformat(as_of)
    except ValueError as error:
        raise CliError("INVALID_REQUEST", "as_of must be a canonical ISO date") from error
    if len(as_of) != 10 or parsed_date.isoformat() != as_of:
        raise CliError("INVALID_REQUEST", "as_of must be a canonical ISO date")

    universe = request["universe"]
    if type(universe) is not list or universe != CANONICAL_UNIVERSE:
        raise CliError("INVALID_REQUEST", "universe is not the canonical fixed snapshot")
    if any(type(member) is not str for member in universe):
        raise CliError("INVALID_REQUEST", "universe members must be strings")

    factors = request["factors"]
    if type(factors) is not dict or set(factors) != set(CANONICAL_UNIVERSE):
        raise CliError("INVALID_REQUEST", "factor instruments do not match the fixed universe")
    for member in CANONICAL_UNIVERSE:
        member_factors = factors[member]
        if type(member_factors) is not dict:
            raise CliError("INVALID_REQUEST", "instrument factors must be objects")
        for factor_id, raw in member_factors.items():
            if type(factor_id) is not str or not FACTOR_RE.fullmatch(factor_id):
                raise CliError("INVALID_REQUEST", "factor id is invalid")
            if raw is not None:
                _finite(raw, "factor value")

    provenance = _exact_object(
        request["provenance"],
        {
            "dataset_version_id",
            "dataset_id",
            "dataset_version",
            "curated_version",
            "dataset_manifest_sha256",
            "universe_snapshot_id",
            "factor_snapshot_hash",
        },
        "provenance",
    )
    dataset_version_id = _string(
        provenance["dataset_version_id"], "provenance.dataset_version_id"
    )
    try:
        if str(uuid.UUID(dataset_version_id)) != dataset_version_id:
            raise ValueError
    except ValueError as error:
        raise CliError(
            "INVALID_REQUEST", "provenance.dataset_version_id must be a canonical UUID"
        ) from error
    _string(provenance["dataset_id"], "provenance.dataset_id")
    _string(provenance["dataset_version"], "provenance.dataset_version")
    if type(provenance["curated_version"]) is not int or provenance["curated_version"] <= 0:
        raise CliError("INVALID_REQUEST", "provenance.curated_version must be positive")
    manifest_hash = _string(
        provenance["dataset_manifest_sha256"],
        "provenance.dataset_manifest_sha256",
    )
    if not PLAIN_HASH_RE.fullmatch(manifest_hash):
        raise CliError(
            "INVALID_REQUEST",
            "provenance.dataset_manifest_sha256 is not a canonical hash",
        )
    for key in ("universe_snapshot_id", "factor_snapshot_hash"):
        value = _string(provenance[key], f"provenance.{key}")
        if not HASH_RE.fullmatch(value):
            raise CliError("INVALID_REQUEST", f"provenance.{key} is not a canonical hash")
    return request


def _open_request_descriptor(path: Path) -> int:
    if os.name == "nt":
        return _open_windows_request_descriptor(path)
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    flags |= getattr(os, "O_NONBLOCK", 0)
    return os.open(path, flags)


def _open_windows_request_descriptor(path: Path) -> int:
    import ctypes
    import ctypes.wintypes
    import msvcrt

    class ByHandleFileInformation(ctypes.Structure):
        _fields_ = [
            ("dwFileAttributes", ctypes.wintypes.DWORD),
            ("ftCreationTime", ctypes.wintypes.FILETIME),
            ("ftLastAccessTime", ctypes.wintypes.FILETIME),
            ("ftLastWriteTime", ctypes.wintypes.FILETIME),
            ("dwVolumeSerialNumber", ctypes.wintypes.DWORD),
            ("nFileSizeHigh", ctypes.wintypes.DWORD),
            ("nFileSizeLow", ctypes.wintypes.DWORD),
            ("nNumberOfLinks", ctypes.wintypes.DWORD),
            ("nFileIndexHigh", ctypes.wintypes.DWORD),
            ("nFileIndexLow", ctypes.wintypes.DWORD),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.argtypes = [
        ctypes.wintypes.LPCWSTR,
        ctypes.wintypes.DWORD,
        ctypes.wintypes.DWORD,
        ctypes.wintypes.LPVOID,
        ctypes.wintypes.DWORD,
        ctypes.wintypes.DWORD,
        ctypes.wintypes.HANDLE,
    ]
    kernel32.CreateFileW.restype = ctypes.wintypes.HANDLE
    kernel32.GetFileInformationByHandle.argtypes = [
        ctypes.wintypes.HANDLE,
        ctypes.POINTER(ByHandleFileInformation),
    ]
    kernel32.GetFileInformationByHandle.restype = ctypes.wintypes.BOOL
    kernel32.GetFileType.argtypes = [ctypes.wintypes.HANDLE]
    kernel32.GetFileType.restype = ctypes.wintypes.DWORD
    kernel32.CloseHandle.argtypes = [ctypes.wintypes.HANDLE]
    kernel32.CloseHandle.restype = ctypes.wintypes.BOOL

    generic_read = 0x8000_0000
    share_read_write_delete = 0x0000_0001 | 0x0000_0002 | 0x0000_0004
    open_existing = 3
    file_flag_open_reparse_point = 0x0020_0000
    invalid_handle = ctypes.c_void_p(-1).value
    handle = kernel32.CreateFileW(
        str(path),
        generic_read,
        share_read_write_delete,
        None,
        open_existing,
        file_flag_open_reparse_point,
        None,
    )
    if handle in (None, invalid_handle):
        raise ctypes.WinError(ctypes.get_last_error())

    try:
        information = ByHandleFileInformation()
        if not kernel32.GetFileInformationByHandle(handle, ctypes.byref(information)):
            raise ctypes.WinError(ctypes.get_last_error())
        file_attribute_directory = 0x0000_0010
        file_attribute_device = 0x0000_0040
        file_attribute_reparse_point = 0x0000_0400
        forbidden_attributes = (
            file_attribute_directory
            | file_attribute_device
            | file_attribute_reparse_point
        )
        file_type_disk = 0x0001
        if (
            information.dwFileAttributes & forbidden_attributes
            or kernel32.GetFileType(handle) != file_type_disk
        ):
            raise OSError("request handle is not a regular non-reparse disk file")
        descriptor = msvcrt.open_osfhandle(
            int(handle),
            os.O_RDONLY | getattr(os, "O_BINARY", 0),
        )
    except BaseException:
        kernel32.CloseHandle(handle)
        raise
    return descriptor


def _load_request(path: Path) -> dict[str, Any]:
    descriptor: int | None = None
    try:
        descriptor = _open_request_descriptor(path)
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise CliError("REQUEST_UNAVAILABLE", "request file is unavailable")
        if metadata.st_size > MAX_REQUEST_BYTES:
            raise CliError("REQUEST_TOO_LARGE", "request exceeds the size limit")
        chunks: list[bytes] = []
        remaining = MAX_REQUEST_BYTES + 1
        while remaining:
            chunk = os.read(descriptor, min(64 * 1024, remaining))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        raw = b"".join(chunks)
        if len(raw) > MAX_REQUEST_BYTES:
            raise CliError("REQUEST_TOO_LARGE", "request exceeds the size limit")
    except CliError:
        raise
    except OSError as error:
        raise CliError("REQUEST_UNAVAILABLE", "request file is unavailable") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    try:
        value = json.loads(
            raw,
            # Python accepts these JavaScript spellings even though strict JSON
            # does not. Preserve them just long enough for the typed factor
            # validator to classify them as non-finite input.
            parse_constant=lambda constant: {
                "NaN": math.nan,
                "Infinity": math.inf,
                "-Infinity": -math.inf,
            }[constant],
        )
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        raise CliError("INVALID_JSON", "request is not strict JSON") from error
    return _validate_request(value)


def _canonical_bytes(value: Any, limit: int, code: str) -> bytes:
    try:
        encoded = (
            json.dumps(
                value,
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
                allow_nan=False,
            ).encode("utf-8")
            + b"\n"
        )
    except (TypeError, ValueError) as error:
        raise CliError(code, "generator returned a non-JSON value") from error
    if len(encoded) > limit:
        raise CliError(code, "generated document exceeds the size limit")
    return encoded


def _atomic_write(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def _generate(request: dict[str, Any]) -> dict[str, Any]:
    # The module path comes only from this literal mapping.  No request field
    # is ever interpreted as a module, function, or filesystem path.
    try:
        module = importlib.import_module(GENERATORS[request["strategy_id"]])
        result = module.generate_target(
            request["parameters"],
            request["factors"],
            request["as_of"],
            request["universe"],
        )
    except TargetError as error:
        raise CliError("TARGET_GENERATION_FAILED", "target generation rejected its inputs") from error
    except Exception as error:
        raise CliError("TARGET_GENERATOR_INTERNAL", "target generator failed safely") from error
    if type(result) is not dict:
        raise CliError("INVALID_TARGET", "target generator returned the wrong root type")
    provenance = request["provenance"]
    result["dataset_version_id"] = provenance["dataset_version_id"]
    result["dataset_id"] = provenance["dataset_id"]
    result["dataset_version"] = provenance["dataset_version"]
    result["curated_version"] = provenance["curated_version"]
    result["dataset_manifest_sha256"] = provenance["dataset_manifest_sha256"]
    result["universe_snapshot_id"] = provenance["universe_snapshot_id"]
    result["factor_snapshot_hash"] = provenance["factor_snapshot_hash"]
    result["portfolio_snapshot_id"] = compute_portfolio_snapshot_id(result)
    return result


def execute(request_path: Path, result_path: Path, status_path: Path) -> None:
    result_path.unlink(missing_ok=True)
    status_path.unlink(missing_ok=True)
    request = _load_request(request_path)
    result = _generate(request)
    _atomic_write(
        result_path,
        _canonical_bytes(result, MAX_RESULT_BYTES, "RESULT_TOO_LARGE"),
    )


def _write_status(path: Path, error: CliError) -> None:
    status = {"code": error.code, "summary": error.summary[:512]}
    payload = _canonical_bytes(status, MAX_STATUS_BYTES, "STATUS_TOO_LARGE")
    _atomic_write(path, payload)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--request", required=True, type=Path)
    parser.add_argument("--result", required=True, type=Path)
    parser.add_argument("--status", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        execute(args.request, args.result, args.status)
        return 0
    except CliError as error:
        try:
            args.result.unlink(missing_ok=True)
            _write_status(args.status, error)
        except OSError:
            pass
        return 1
    except Exception:
        try:
            args.result.unlink(missing_ok=True)
            _write_status(
                args.status,
                CliError("CHILD_INTERNAL_ERROR", "target child failed safely"),
            )
        except OSError:
            pass
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
