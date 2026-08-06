"""Building the DB manifest for a normalized result (plan Todo 20).

The manifest maps 1:1 to the T3 backtest-result tables (migration 0006):
`backtest_runs` (the `run` section), `backtest_metrics` (`metrics`),
`backtest_warnings` (`warnings`), and `result_artifacts` (`artifacts`). The
Rust `result-model` crate (`manifest.rs`) defines the matching serde types and
the sqlx writer; this module is the worker's Python-side producer. Nothing is
written to the database here - the publisher (Rust) consumes this JSON.
"""
from __future__ import annotations

import re
from typing import Any

_SHA256_HEX = re.compile(r"^[0-9a-f]{64}$")


class ManifestError(ValueError):
    """The normalized result cannot be represented as a DB manifest."""


def _config_sha256(config_hash: str) -> str:
    hex_hash = config_hash.removeprefix("sha256:").lower()
    if not _SHA256_HEX.match(hex_hash):
        raise ManifestError(f"config_hash {config_hash!r} is not a 64-char sha256 hex")
    return hex_hash


def build_manifest(
    result: dict[str, Any],
    *,
    run_id: str,
    owner_user_id: str,
    job_id: str | None,
    run_dir,
    status: str,
    artifacts: dict[str, dict] | None = None,
    artifact_root: str = "",
) -> dict[str, Any]:
    provenance = result["provenance"]
    if artifacts is None:
        artifacts = write_parquet_artifacts(result, run_dir)
    return {
        "run": {
            "id": run_id,
            "owner_user_id": owner_user_id,
            "job_id": job_id,
            "strategy_id": provenance["strategy_id"],
            "strategy_version": provenance["strategy_version"],
            "dataset_version": provenance["dataset_version"],
            "engine": provenance["engine"],
            "engine_version": provenance["engine_version"],
            "config_sha256": _config_sha256(provenance["config_hash"]),
            "code_commit": provenance["code_commit"],
            "random_seed": provenance["random_seed"],
            "timezone": provenance["timezone"],
            "status": status,
            "summary_json": result["summary"],
        },
        "metrics": {key: float(value) for key, value in result["metrics"].items()},
        "warnings": list(result["warnings"]),
        "artifacts": [
            {
                "artifact_type": entry["artifact_type"],
                "parquet_path": f"{artifact_root}{entry['parquet_path']}",
                "row_count": entry["row_count"],
                "sha256": entry["sha256"],
                "size_bytes": entry["size_bytes"],
                "summary_json": {},
            }
            for entry in artifacts.values()
        ],
    }


def write_parquet_artifacts(result: dict[str, Any], run_dir) -> dict[str, dict]:
    from .normalizer import write_parquet_artifacts as _write

    return _write(result, run_dir)
