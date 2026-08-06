"""Red tests (Todo 20): worker outcome -> job-queue lifecycle mapping.

The queue consumer (Todo 21 / the Compose golden job) drives
claim -> Worker.run -> settle_success | settle_failure. These tests pin the
deterministic run id (the manifest idempotency anchor) and the settle
classification of every worker outcome, following the Todo 19 queue
conventions (only transient failures retry; integrity failures never do).
"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from backtest_worker.job_lifecycle import classify_outcome, run_id_for_job


class _Outcome:
    def __init__(self, state, error=None):
        self.state = state
        self.error = error or {}


def test_run_id_is_deterministic_for_a_job():
    first = run_id_for_job("123e4567-e89b-12d3-a456-426614174002", "fingerprint")
    second = run_id_for_job("123e4567-e89b-12d3-a456-426614174002", "fingerprint")
    assert first == second, "the same job must derive the same run id"
    assert first == run_id_for_job("123e4567-e89b-12d3-a456-426614174002", "other-fingerprint")
    assert first != run_id_for_job("123e4567-e89b-12d3-a456-426614174099", "fingerprint")
    assert first != run_id_for_job(None, "fingerprint")
    import uuid

    uuid.UUID(first)  # valid uuid


def test_succeeded_maps_to_settle_success():
    assert classify_outcome(_Outcome("SUCCEEDED"))[0] == "success"


def test_integrity_rejection_maps_to_non_retryable_failure():
    kind, cls, code, message = classify_outcome(
        _Outcome("REJECTED", {"kind": "LedgerMismatchError", "detail": "cash != equity"})
    )
    assert kind == "failure"
    assert cls == "integrity", "integrity failures must never retry"
    assert code == "LedgerMismatchError"
    assert "cash != equity" in message


def test_terminated_maps_to_retryable_failure():
    kind, cls, code, _ = classify_outcome(
        _Outcome("TERMINATED", {"kind": "terminated", "detail": "SIGTERM"})
    )
    assert kind == "failure"
    assert cls == "transient", "a terminated worker must be retryable"


def test_child_failure_is_retryable_but_isolation_violations_are_not():
    _, cls, code, _ = classify_outcome(_Outcome("FAILED", {"kind": "child_status", "detail": "x"}))
    assert cls == "transient"
    assert code == "child_failed"
    _, cls, code, _ = classify_outcome(
        _Outcome("FAILED", {"kind": "readonly_mount_violation", "detail": "dataset changed"})
    )
    assert cls == "integrity", "a read-only-mount violation must not retry"
    assert code == "readonly_mount_violation"
