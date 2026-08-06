"""Worker outcome -> job-queue lifecycle mapping (plan Todo 20).

The queue consumer (Todo 21 / the Compose golden job) drives:

    claim = queue.claim_next(worker_id)
    outcome = Worker().run(request_from_claim, output_dir, status_path)
    if kind == "success": queue.settle_success(&claim)
    else: queue.settle_failure(&claim, error_class, code, message)

Following the Todo 19 conventions: only `transient` failures retry; integrity
violations (bad data, ledger mismatch, isolation breaks) never do. The run id
is derived deterministically from the job so a retried job re-publishes the
same run id - the idempotency anchor the `ManifestWriter` relies on.
"""
from __future__ import annotations

import uuid
from typing import Any

# DNS namespace: the same fixed namespace used by the Rust publisher's run-id
# derivation so both sides agree.
RUN_NAMESPACE = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")

#: Worker states that map to non-retryable (`integrity`) settle failures.
_INTEGRITY_FAILURE_KINDS = frozenset({
    "readonly_mount_violation",
    "network_disabled",
    "guard_error",
})


def run_id_for_job(job_id: str | None, fingerprint: str) -> str:
    """Deterministic run id: uuid5(DNS, job_id) when a job exists, else
    uuid5(DNS, fingerprint). Re-running the same job derives the same id."""
    key = job_id if job_id else fingerprint
    return str(uuid.uuid5(RUN_NAMESPACE, key))


def classify_outcome(outcome: Any) -> tuple[str, str | None, str, str]:
    """(settle_kind, error_class, code, message) for the job queue.

    - `("success", None, "succeeded", ...)` -> settle_success
    - `("failure", "transient", code, msg)` -> settle_failure(Transient, ...) (retries)
    - `("failure", "integrity", code, msg)` -> settle_failure(Integrity, ...) (never retries)
    """
    state = outcome.state
    if state == "SUCCEEDED":
        return ("success", None, "succeeded", "backtest completed and validated")
    if state == "REJECTED":
        error = outcome.error or {}
        return (
            "failure",
            "integrity",
            error.get("kind", "integrity_failed"),
            error.get("detail", "result failed integrity validation"),
        )
    if state == "TERMINATED":
        error = outcome.error or {}
        return (
            "failure",
            "transient",
            "worker_terminated",
            error.get("detail", "worker terminated before completion"),
        )
    error = outcome.error or {}
    kind = error.get("kind", "child_failed")
    if kind in _INTEGRITY_FAILURE_KINDS:
        return ("failure", "integrity", kind, error.get("detail", "isolation violation"))
    return ("failure", "transient", "child_failed", error.get("detail", "child failed"))
