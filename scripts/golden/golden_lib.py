"""golden_lib - canonical hashing, GoldenManifest, and field-level diff core.

Part of the Todo 6 reproducibility / golden-manifest / evidence harness.

Guarantees
----------
- Canonical JSON bytes: key-order independent, compact, ASCII-escaped UTF-8, so
  byte-identical hashes are produced for logically equal documents regardless of
  file whitespace or key insertion order.
- Parquet hashing: metadata-stripped canonical projection (schema names/types +
  row-major values) so the hash is stable across pyarrow rewrites that only
  change file metadata; value changes still change the hash.
- GoldenManifest carries the documented version dimensions
  data/strategy/engine/code/config/seed/timezone and is byte-deterministic
  (no timestamps, no absolute paths).
- field_diff reports leaf-path diffs (dict keys via '.', list indexes via '[i]').
- verify recomputes hashes and reports field-level diffs against embedded
  approved content; embedded content is never secrets/proprietary data, only
  synthetic fixtures and approved synthetic outputs.

Secrets: this module never reads credential paths and its evidence writer emits
only hashes, versions, paths, and diff lines - never file contents.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ALGO = "sha256"
HASH_PREFIX = f"{ALGO}:"
GENERATOR = "scripts/golden/golden.py"
VERSION_DIMENSIONS = ("data", "strategy", "engine", "code", "config", "seed", "timezone")
# "code" is intentionally excluded here: it is resolved from git (or pinned
# via --code-override) and injected into the manifest, never config-supplied.
CONFIG_VERSION_DIMENSIONS = ("data", "strategy", "engine", "config", "seed", "timezone")

_PARQUET_MAGIC = b"PAR1"


class GoldenManifestError(Exception):
    """Raised for unreadable inputs or invalid manifests."""


@dataclass(frozen=True)
class Diff:
    """A single field-level difference between two JSON documents."""

    path: str
    kind: str  # "changed" | "added" | "removed"
    old: Any = None
    new: Any = None


@dataclass
class EntryResult:
    """Verification outcome for one manifest entry (fixture or artifact)."""

    category: str
    path: str
    ok: bool
    expected_sha256: str
    actual_sha256: str
    diffs: list[Diff] = field(default_factory=list)
    note: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "category": self.category,
            "path": self.path,
            "ok": self.ok,
            "expected_sha256": self.expected_sha256,
            "actual_sha256": self.actual_sha256,
            "diffs": [render_diff(d) for d in self.diffs],
            "note": self.note,
        }


@dataclass
class VerifyReport:
    golden_id: str
    artifacts: list[EntryResult]
    fixtures: list[EntryResult]

    @property
    def ok(self) -> bool:
        return all(e.ok for e in self.artifacts) and all(e.ok for e in self.fixtures)

    def to_dict(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "golden_id": self.golden_id,
            "artifacts": [e.to_dict() for e in self.artifacts],
            "fixtures": [e.to_dict() for e in self.fixtures],
        }


# --------------------------------------------------------------------------- #
# Canonical serialization / hashing
# --------------------------------------------------------------------------- #

def canonical_json_bytes(obj: Any) -> bytes:
    """Deterministic canonical JSON bytes: sorted keys, compact, ASCII-escaped."""
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("utf-8")


def hash_bytes(data: bytes) -> str:
    """Content hash string, e.g. 'sha256:<64 hex chars>'."""
    digest = hashlib.sha256(data).hexdigest()
    return f"{HASH_PREFIX}{digest}"


def load_json_file(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise GoldenManifestError(f"cannot read JSON {path}: {exc}") from exc


def _try_parquet_bytes(path: Path) -> tuple[bytes | None, str | None]:
    """Return (canonical_bytes, None) for a readable parquet file, else (None, note)."""
    try:
        import pyarrow.parquet as pq  # type: ignore[import-not-found]

        table = pq.read_table(path, memory_map=True)
    except ImportError:
        return None, "pyarrow unavailable; raw-bytes hash used"
    except Exception as exc:  # corrupt / truncated parquet
        return None, f"detected parquet magic but read failed ({exc}); raw-bytes hash used"

    # Canonical projection: schema (names + types) + row-major values. Strips
    # file/row-group metadata, created_by, key-value metadata, and sort columns,
    # so logically identical data hashes identically across rewrites.
    schema: list[dict[str, str]] = [
        {"name": field_.name, "type": str(field_.type)} for field_ in table.schema
    ]
    rows: list[list[Any]] = []
    for batch in table.to_batches():
        for row_idx in range(batch.num_rows):
            rows.append([_canonical_scalar(batch.column(i)[row_idx].as_py()) for i in range(batch.num_columns)])
    projection = {"schema": schema, "rows": rows}
    return canonical_json_bytes(projection), None


def _canonical_scalar(value: Any) -> Any:
    """Normalize a parquet scalar to a byte-stable canonical form."""
    if value is None:
        return None
    if isinstance(value, bool):
        return value
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            raise GoldenManifestError(f"non-finite float in parquet data: {value!r}")
        return value  # repr-based shortest form is deterministic for equal values
    if isinstance(value, str):
        return value
    if isinstance(value, bytes):
        return {"__binary__": base64.b64encode(value).decode("ascii")}
    if isinstance(value, (list, tuple)):
        return [_canonical_scalar(v) for v in value]
    if isinstance(value, dict):
        return {k: _canonical_scalar(v) for k, v in value.items()}
    # Timestamps/decimals surface as Python datetime/Decimal after as_py().
    if hasattr(value, "isoformat"):  # datetime/date/time
        return value.isoformat()
    if hasattr(value, "__float__") and hasattr(value, "as_tuple"):  # Decimal
        return str(value)
    return str(value)


def hash_file(path: Path) -> tuple[str, dict[str, str]]:
    """Content-addressed hash for a file.

    Returns (hash_string, meta) where meta["kind"] is one of
    "json" | "parquet" | "binary" describing how the bytes were hashed.
    """
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise GoldenManifestError(f"cannot read {path}: {exc}") from exc

    # JSON: canonical (whitespace/key-order independent).
    try:
        obj = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        obj = None
    if obj is not None:
        return hash_bytes(canonical_json_bytes(obj)), {"kind": "json"}

    # Parquet: canonical projection when readable; raw bytes otherwise.
    if raw.startswith(_PARQUET_MAGIC) and raw.endswith(_PARQUET_MAGIC):
        canonical, note = _try_parquet_bytes(path)
        if canonical is not None:
            return hash_bytes(canonical), {"kind": "parquet"}
        return hash_bytes(raw), {"kind": "binary", "note": note or "unreadable parquet"}

    return hash_bytes(raw), {"kind": "binary"}


# --------------------------------------------------------------------------- #
# Field-level diff
# --------------------------------------------------------------------------- #

def _render_value(value: Any) -> str:
    if isinstance(value, str):
        rendered = value
    else:
        rendered = json.dumps(value, sort_keys=True, ensure_ascii=True)
    if len(rendered) > 80:
        rendered = rendered[:77] + "..."
    return rendered


def render_diff(diff: Diff) -> str:
    if diff.kind == "changed":
        return f"{diff.path}: {_render_value(diff.old)} -> {_render_value(diff.new)}"
    if diff.kind == "added":
        return f"+ {diff.path}: {_render_value(diff.new)}"
    if diff.kind == "removed":
        return f"- {diff.path}: {_render_value(diff.old)}"
    return f"{diff.path}: {diff.kind}"


def field_diff(old: Any, new: Any, path: str = "") -> list[Diff]:
    """Recursive leaf-path diff between two JSON documents.

    Dict keys join with '.', list indexes with '[i]'. Deterministic ordering:
    dicts iterate the sorted union of keys; lists iterate indexes ascending.
    """
    if isinstance(old, dict) and isinstance(new, dict):
        diffs: list[Diff] = []
        for key in sorted(set(old) | set(new)):
            child = f"{path}.{key}" if path else key
            if key not in old:
                diffs.append(Diff(child, "added", None, new[key]))
            elif key not in new:
                diffs.append(Diff(child, "removed", old[key], None))
            else:
                diffs.extend(field_diff(old[key], new[key], child))
        return diffs

    if isinstance(old, list) and isinstance(new, list):
        diffs = []
        for idx in range(max(len(old), len(new))):
            child = f"{path}[{idx}]"
            if idx >= len(old):
                diffs.append(Diff(child, "added", None, new[idx]))
            elif idx >= len(new):
                diffs.append(Diff(child, "removed", old[idx], None))
            else:
                diffs.extend(field_diff(old[idx], new[idx], child))
        return diffs

    if old != new:
        return [Diff(path, "changed", old, new)]
    return []


# --------------------------------------------------------------------------- #
# GoldenManifest construction / serialization
# --------------------------------------------------------------------------- #

def resolve_code(base_dir: Path, revision: str = "HEAD") -> dict[str, str]:
    """Resolve a validated commit and its actual tree from a Git worktree."""
    try:
        commit = subprocess.run(
            ["git", "rev-parse", "--verify", f"{revision}^{{commit}}"],
            cwd=base_dir,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        tree = subprocess.run(
            ["git", "rev-parse", "--verify", f"{commit}^{{tree}}"],
            cwd=base_dir, capture_output=True, text=True, check=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise GoldenManifestError(
            f"cannot resolve git code version {revision!r} from {base_dir}"
        ) from exc
    return {"commit": commit, "tree": tree}


def manifest_from_config(
    config: dict[str, Any], base_dir: Path, code: dict[str, str], output_dir: Path | None = None
) -> dict[str, Any]:
    """Build a GoldenManifest dict from a generation config (see golden.json).

    Entry paths are stored relative to `output_dir` (the manifest's own
    location) so the manifest stays self-contained wherever it is written.
    """
    output_dir = output_dir or base_dir
    versions: dict[str, Any] = {}
    for dimension in CONFIG_VERSION_DIMENSIONS:
        if dimension not in config["versions"]:
            raise GoldenManifestError(f"golden config missing version dimension: {dimension}")
        versions[dimension] = config["versions"][dimension]
    versions["code"] = {"commit": code["commit"], "tree": code["tree"]}
    versions["config"] = {
        **config["versions"]["config"],
        "hash": hash_bytes(canonical_json_bytes(config)),
    }

    manifest: dict[str, Any] = {
        "manifest_version": str(config.get("manifest_version", "1")),
        "golden_id": config["golden_id"],
        "generator": GENERATOR,
        "hash_algorithm": ALGO,
        "versions": versions,
        "fixtures": [_build_entry(e, base_dir, output_dir, section="fixtures") for e in config["fixtures"]],
        "artifacts": [_build_entry(e, base_dir, output_dir, section="artifacts") for e in config["artifacts"]],
    }
    return manifest


def _build_entry(entry: dict[str, Any], base_dir: Path, output_dir: Path, section: str) -> dict[str, Any]:
    path = (base_dir / entry["path"]).resolve()
    sha256, meta = hash_file(path)
    record: dict[str, Any] = {
        "category": entry["category"],
        "path": _relative_posix(path, output_dir),
        "sha256": sha256,
        "kind": meta["kind"],
    }
    if section == "artifacts":
        record["schema_version"] = int(entry.get("schema_version", 1))
    if meta["kind"] == "json" and entry.get("embed", True) is not False:
        record["content"] = load_json_file(path)
    return record


def _relative_posix(path: Path, base: Path) -> str:
    try:
        return os.path.relpath(path.resolve(), base.resolve()).replace("\\", "/")
    except ValueError as exc:
        raise GoldenManifestError(
            f"cannot express {path} relative to manifest location {base} "
            "(different drives/roots); write the manifest inside the config's tree"
        ) from exc


def serialize_manifest(manifest: dict[str, Any]) -> bytes:
    """Byte-deterministic manifest serialization (no timestamps / absolute paths)."""
    return canonical_json_bytes(manifest)


def load_manifest(path: Path) -> dict[str, Any]:
    obj = load_json_file(path)
    if not isinstance(obj, dict):
        raise GoldenManifestError(f"manifest {path} is not a JSON object")
    for section in ("fixtures", "artifacts"):
        if section not in obj:
            raise GoldenManifestError(f"manifest {path} missing section '{section}'")
    return obj


# --------------------------------------------------------------------------- #
# Verification
# --------------------------------------------------------------------------- #

def _verify_section(manifest: dict[str, Any], base_dir: Path, section: str) -> list[EntryResult]:
    results: list[EntryResult] = []
    for entry in manifest[section]:
        path = (base_dir / entry["path"]).resolve()
        try:
            actual_sha256, meta = hash_file(path)
        except GoldenManifestError:
            results.append(EntryResult(
                category=entry["category"], path=entry["path"], ok=False,
                expected_sha256=entry["sha256"], actual_sha256="",
                note="file missing or unreadable",
            ))
            continue
        ok = actual_sha256 == entry["sha256"]
        diffs: list[Diff] = []
        note: str | None = None
        if not ok:
            content = entry.get("content")
            if content is not None and meta["kind"] == "json":
                try:
                    diffs = field_diff(content, load_json_file(path))
                except GoldenManifestError as exc:
                    note = f"content diff unavailable: {exc}"
            else:
                note = "hash mismatch; content not embedded (binary/parquet) - file-level only"
        results.append(EntryResult(
            category=entry["category"], path=entry["path"], ok=ok,
            expected_sha256=entry["sha256"], actual_sha256=actual_sha256,
            diffs=diffs, note=note,
        ))
    return results


def verify_manifest(manifest: dict[str, Any], base_dir: Path) -> VerifyReport:
    return VerifyReport(
        golden_id=str(manifest["golden_id"]),
        artifacts=_verify_section(manifest, base_dir, "artifacts"),
        fixtures=_verify_section(manifest, base_dir, "fixtures"),
    )


def render_report(report: VerifyReport) -> str:
    """Human-readable verification report (used by the CLI)."""
    lines: list[str] = []
    versions = {}
    lines.append(f"GOLDEN VERIFY  {report.golden_id}")

    def section(title: str, results: list[EntryResult]) -> None:
        lines.append(f"{title} ({len(results)})")
        for entry in results:
            if entry.ok:
                lines.append(f"  [PASS] {entry.category:15s} {entry.path}  {entry.actual_sha256}")
            else:
                lines.append(f"  [FAIL] {entry.category:15s} {entry.path}")
                lines.append(f"          expected: {entry.expected_sha256}")
                lines.append(f"          actual:   {entry.actual_sha256}")
                for diff in entry.diffs:
                    lines.append(f"          {render_diff(diff)}")
                if entry.note:
                    lines.append(f"          note: {entry.note}")

    section("ARTIFACTS", report.artifacts)
    section("FIXTURES", report.fixtures)
    failed_artifacts = sum(1 for e in report.artifacts if not e.ok)
    failed_fixtures = sum(1 for e in report.fixtures if not e.ok)
    verdict = "PASS" if report.ok else "FAIL"
    lines.append(
        f"VERDICT: {verdict} "
        f"({failed_artifacts}/{len(report.artifacts)} artifacts, "
        f"{failed_fixtures}/{len(report.fixtures)} fixtures failed)"
    )
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------- #
# Evidence writer (sanitized by construction)
# --------------------------------------------------------------------------- #

def write_evidence(manifest: dict[str, Any], report: VerifyReport, out_path: Path) -> str:
    """Write a sanitized evidence text: hashes, versions, paths, diff lines only.

    Never embeds artifact/fixture content, credentials, or proprietary data.
    """
    versions = manifest["versions"]
    lines = [
        "LAGRANGE STATION - GOLDEN MANIFEST EVIDENCE",
        "=" * 44,
        f"golden_id: {manifest['golden_id']}",
        f"manifest_version: {manifest['manifest_version']}",
        f"hash_algorithm: {manifest['hash_algorithm']}",
        f"generator: {manifest['generator']}",
        "",
        "versions:",
        f"  data:     {versions['data']}",
        f"  strategy: {versions['strategy']}",
        f"  engine:   {versions['engine']}",
        f"  code:     commit={versions['code']['commit']} tree={versions['code']['tree']}",
        f"  config:   {versions['config']}",
        f"  seed:     {versions['seed']}",
        f"  timezone: {versions['timezone']}",
        "",
        f"artifacts ({len(report.artifacts)}):",
    ]
    for entry in report.artifacts:
        status = "OK  " if entry.ok else "FAIL"
        lines.append(f"  [{status}] {entry.category:15s} {entry.path}  {entry.actual_sha256}")
        for diff in entry.diffs:
            lines.append(f"          {render_diff(diff)}")
        if entry.note:
            lines.append(f"          note: {entry.note}")
    lines.append(f"fixtures ({len(report.fixtures)}):")
    for entry in report.fixtures:
        status = "OK  " if entry.ok else "FAIL"
        lines.append(f"  [{status}] {entry.category:20s} {entry.path}  {entry.actual_sha256}")
        for diff in entry.diffs:
            lines.append(f"          {render_diff(diff)}")
        if entry.note:
            lines.append(f"          note: {entry.note}")
    verdict = "PASS" if report.ok else "FAIL"
    lines.append("")
    lines.append(f"VERDICT: {verdict}")
    lines.append(
        "redaction: this evidence is sanitized by construction; it contains only "
        "hashes, versions, paths, and diff lines - no secrets or proprietary data."
    )
    text = "\n".join(lines) + "\n"
    out_path.write_text(text, encoding="utf-8")
    return text
