"""Lagrange Station live node (plan Todo 41).

One process, one account, under the Phase 3 Compose profile. The node never
starts ready: it must reconcile green before it may place anything, and a
crash brings it back in exactly that state rather than resuming.

The safety decisions live in pure modules so they are testable without a
broker and reproducible from a log after an incident:

* :mod:`live_node.lifecycle` -- node states and the submit admission rule
* :mod:`live_node.isolation` -- one node per account, enforced by the OS
* :mod:`live_node.cancel_policy` -- what happens to working orders on engage
* :mod:`live_node.health` -- health vs readiness, which are not the same
"""
from __future__ import annotations

from .cancel_policy import CancelPlan, CancelPolicy, OrderDisposition, WorkingOrder, plan
from .health import HealthReport, METRIC_NAMES, metrics, report
from .isolation import AccountLock, LockInfo, NodeAlreadyRunning
from .lifecycle import (
    IllegalTransition,
    NodeLifecycle,
    NodeState,
    NodeStatus,
    Reason,
    resume_after_crash,
)

__all__ = [
    "AccountLock",
    "CancelPlan",
    "CancelPolicy",
    "HealthReport",
    "IllegalTransition",
    "LockInfo",
    "METRIC_NAMES",
    "NodeAlreadyRunning",
    "NodeLifecycle",
    "NodeState",
    "NodeStatus",
    "OrderDisposition",
    "Reason",
    "WorkingOrder",
    "metrics",
    "plan",
    "report",
    "resume_after_crash",
]
