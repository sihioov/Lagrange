"""The backtest worker supervisor (plan Todo 20).

One job = one temp run directory + one isolated child process (ADR-005):
temp dir -> spawn the NT child under CPU/memory/time limits with the dataset
mounted read-only and the network disabled -> the child writes raw results ->
normalize into the common model -> integrity validation -> Parquet artifacts +
manifest -> cleanup. Structured status/provenance is written to the caller's
`status_path`; the run directory is removed even on failure.

Publication rule: the DB manifest is written ONLY after
[`IntegrityGate::publish`] succeeds; a rejected run produces no manifest, so
nothing is ever published after an integrity failure.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .guard import ReadOnlyMountError
from .isolation import IsolationLimits, ProcessIsolation, Termination, interpreter_path, venv_site_packages
from .normalizer import IntegrityGate, Normalizer, NormalizerError, write_parquet_artifacts
from .publish import build_manifest
from .raw import RawResult

DEFAULT_LIMITS = {
    "memory_bytes": 2 * 1024 * 1024 * 1024,
    "cpu_seconds": 1800,
    "wall_seconds": 3600,
    "active_processes": 1,
    "network_disabled": True,
}


@dataclass
class RunOutcome:
    """Structured status + provenance of one worker run."""

    run_id: str
    state: str
    exit_code: int | None
    isolation: dict[str, Any] = field(default_factory=dict)
    timing: dict[str, Any] = field(default_factory=dict)
    process: dict[str, Any] = field(default_factory=dict)
    artifacts: list[dict[str, Any]] = field(default_factory=list)
    provenance: dict[str, Any] = field(default_factory=dict)
    warnings: list[Any] = field(default_factory=list)
    error: dict[str, Any] | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "run_id": self.run_id,
            "state": self.state,
            "exit_code": self.exit_code,
            "isolation": self.isolation,
            "timing": self.timing,
            "process": self.process,
            "artifacts": self.artifacts,
            "provenance": self.provenance,
            "warnings": self.warnings,
            "error": self.error,
        }


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _snapshot_tree(root: Path) -> dict[str, tuple[int, int]]:
    """(relative path -> (size, mtime_ns)) for every file under `root`."""
    snapshot: dict[str, tuple[int, int]] = {}
    for path in sorted(root.rglob("*")):
        if path.is_file():
            stat = path.stat()
            snapshot[str(path.relative_to(root))] = (stat.st_size, stat.st_mtime_ns)
    return snapshot


def _verify_tree(root: Path, before: dict[str, tuple[int, int]]) -> str | None:
    after = _snapshot_tree(root)
    if set(before) != set(after):
        missing = set(before) - set(after)
        added = set(after) - set(before)
        return f"read-only mount changed: missing={sorted(missing)[:3]} added={sorted(added)[:3]}"
    for name, entry in before.items():
        if after[name] != entry:
            return f"read-only mount changed: {name} size/mtime differs"
    return None


def _child_env(request: dict[str, Any], run_dir: Path) -> dict[str, str]:
    nt_root = Path(__file__).resolve().parents[2]
    paths = [str(nt_root), str(nt_root / "strategies"), str(nt_root / "backtest-worker")]
    site = venv_site_packages()
    if site:
        paths.insert(0, site)
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(paths)
    env["LAGRANGE_STATUS_FILE"] = str(run_dir / "status.json")
    env["LAGRANGE_CONTROL_FILE"] = str(run_dir / "stop")
    return env


class Worker:
    """Supervisor: runs one isolated backtest and normalizes its results."""

    def __init__(self, scratch: Path | None = None, keep_run_dir: bool = False) -> None:
        self.scratch = Path(scratch) if scratch else None
        if self.scratch is not None:
            self.scratch.mkdir(parents=True, exist_ok=True)
        self.keep_run_dir = keep_run_dir

    def run(self, request: dict[str, Any], output_dir: Path, status_path: Path) -> RunOutcome:
        output_dir = Path(output_dir)
        output_dir.mkdir(parents=True, exist_ok=True)
        started = time.time()
        run_id = str(request.get("run_id") or uuid.uuid4())
        dataset_path = Path(request["dataset_path"])
        limits = IsolationLimits(**{**DEFAULT_LIMITS, **request.get("limits", {})})
        readonly_mounts = tuple(request.get("readonly_mounts", [])) + (str(dataset_path),)
        limits = IsolationLimits(
            memory_bytes=limits.memory_bytes,
            cpu_seconds=limits.cpu_seconds,
            wall_seconds=limits.wall_seconds,
            active_processes=limits.active_processes,
            network_disabled=limits.network_disabled,
            readonly_mounts=readonly_mounts,
        )

        run_dir = Path(tempfile.mkdtemp(prefix="lagrange-run-", dir=str(self.scratch) if self.scratch else None))
        iso = ProcessIsolation(limits)
        child_pid: int | None = None
        try:
            request_path = run_dir / "request.json"
            request_path.write_text(json.dumps(request, sort_keys=True), encoding="utf-8")
            mount_before = _snapshot_tree(dataset_path)

            env = _child_env(request, run_dir)
            log_path = run_dir / "child.log"
            with log_path.open("w", encoding="utf-8") as log:
                proc = iso.spawn(
                    [interpreter_path(), "-m", "backtest_worker.simulate", "--request", str(request_path)],
                    cwd=run_dir,
                    env=env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                )
            child_pid = proc.pid

            exit_code: int | None = None
            termination: Termination | None = None
            if limits.wall_seconds is not None:
                try:
                    exit_code = iso.wait(proc, timeout=limits.wall_seconds)
                except subprocess.TimeoutExpired:
                    termination = iso.terminate(proc, grace_seconds=8)
                    exit_code = termination.exit_code
            else:
                exit_code = proc.wait()

            child_status = _read_child_status(run_dir)
            mount_violation = _verify_tree(dataset_path, mount_before)

            if child_status.get("state") == "TERMINATED":
                outcome = RunOutcome(run_id=run_id, state="TERMINATED", exit_code=exit_code,
                                     error={"kind": "terminated", "detail": child_status.get("signal")})
            elif child_status.get("state") == "FAILED":
                outcome = RunOutcome(run_id=run_id, state="FAILED", exit_code=exit_code, error=child_status.get("error"))
            elif mount_violation:
                outcome = RunOutcome(run_id=run_id, state="FAILED", exit_code=exit_code,
                                     error={"kind": "readonly_mount_violation", "detail": mount_violation})
            elif child_status.get("state") != "SUCCEEDED":
                outcome = RunOutcome(run_id=run_id, state="FAILED", exit_code=exit_code,
                                     error={"kind": "child_status", "detail": child_status.get("state")})
            else:
                outcome = self._normalize_and_materialize(request, run_dir, output_dir, run_id, exit_code)
        except NormalizerError as error:
            outcome = RunOutcome(run_id=run_id, state="REJECTED", exit_code=1,
                                 error={"kind": type(error).__name__, "detail": str(error)})
        except ReadOnlyMountError as error:
            outcome = RunOutcome(run_id=run_id, state="FAILED", exit_code=1,
                                 error={"kind": "readonly_mount_violation", "detail": str(error)})
        except Exception as error:  # noqa: BLE001 - structured reporting
            outcome = RunOutcome(run_id=run_id, state="FAILED", exit_code=1,
                                 error={"kind": type(error).__name__, "detail": str(error)})
        finally:
            outcome.exit_code = outcome.exit_code or 0 if outcome.state == "SUCCEEDED" else outcome.exit_code
            outcome.timing = {"started_at": _now(), "finished_at": _now(), "wall_seconds": round(time.time() - started, 3)}
            outcome.process = {"supervisor_pid": os.getpid(), "child_pid": child_pid}
            outcome.isolation = {
                "backend": iso.backend,
                "memory_bytes": limits.memory_bytes,
                "cpu_seconds": limits.cpu_seconds,
                "wall_seconds": limits.wall_seconds,
                "active_processes": limits.active_processes,
                "network_disabled": limits.network_disabled,
                "readonly_mounts": list(limits.readonly_mounts),
            }
            status_path = Path(status_path)
            status_path.parent.mkdir(parents=True, exist_ok=True)
            status_path.write_text(json.dumps(outcome.to_dict(), indent=2, sort_keys=True) + "\n", encoding="utf-8")
            iso.close()
            if not self.keep_run_dir:
                shutil.rmtree(run_dir, ignore_errors=True)
        return outcome

    def _normalize_and_materialize(self, request, run_dir, output_dir, run_id, exit_code) -> RunOutcome:
        raw = RawResult.load(run_dir / "raw")
        result = Normalizer().normalize(raw, validate=True)
        gate = IntegrityGate()
        gate.validate(result)
        gate.publish()

        artifacts = write_parquet_artifacts(result, output_dir)
        (output_dir / "result.json").write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        manifest = build_manifest(
            result,
            run_id=run_id,
            owner_user_id=request.get("owner_user_id"),
            job_id=request.get("job_id"),
            run_dir=output_dir,
            status="SUCCEEDED",
            artifacts=artifacts,
        )
        (output_dir / "manifest.json").write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        return RunOutcome(
            run_id=run_id,
            state="SUCCEEDED",
            exit_code=exit_code,
            artifacts=list(artifacts.values()),
            provenance=result["provenance"],
            warnings=list(result["warnings"]),
        )


def _read_child_status(run_dir: Path) -> dict[str, Any]:
    status_path = run_dir / "status.json"
    if not status_path.exists():
        return {"state": "NO_STATUS"}
    try:
        return json.loads(status_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return {"state": "CORRUPT_STATUS"}
