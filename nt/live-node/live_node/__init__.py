"""Lagrange Station live node (plan Todo 41).

One process, one account, under the Phase 3 Compose profile. The node never
starts trade-ready: it must reconcile green before the simulator may report
its local loop green, and a crash brings it back in exactly that state rather
than resuming. Dry-run mode still refuses real trade readiness permanently.

The safety decisions live in pure modules so they are testable without a
broker and reproducible from a log after an incident:

* :mod:`live_node.lifecycle` -- node states and the submit admission rule
* :mod:`live_node.isolation` -- one node per account, enforced by the OS
* :mod:`live_node.cancel_policy` -- what happens to working orders on engage
* :mod:`live_node.health` -- health vs readiness, which are not the same
* :mod:`live_node.startup` -- the one safe order: sweep, apply, reconcile, ready
"""
from __future__ import annotations

from .cancel_policy import CancelPlan, CancelPolicy, OrderDisposition, WorkingOrder, plan
from .health import HealthReport, METRIC_NAMES, metrics, report
from .isolation import AccountLock, LockInfo, NodeAlreadyRunning
from .startup import IN_FLIGHT, StartupOutcome, StartupPlan, plan_startup, sweep_targets
from .lifecycle import (
    IllegalTransition,
    NodeLifecycle,
    NodeState,
    NodeStatus,
    Reason,
    resume_after_crash,
)
from .runtime import (
    DeterministicSimulator,
    GateState,
    LiveNodeRuntime,
    RuntimeConfig,
    SimulatorSnapshot,
    read_gate_file,
    read_status,
)

__all__ = [
    "AccountLock",
    "CancelPlan",
    "CancelPolicy",
    "DeterministicSimulator",
    "GateState",
    "HealthReport",
    "IN_FLIGHT",
    "IllegalTransition",
    "LockInfo",
    "LiveNodeRuntime",
    "METRIC_NAMES",
    "NodeAlreadyRunning",
    "NodeLifecycle",
    "NodeState",
    "NodeStatus",
    "OrderDisposition",
    "Reason",
    "RuntimeConfig",
    "SimulatorSnapshot",
    "StartupOutcome",
    "StartupPlan",
    "WorkingOrder",
    "metrics",
    "plan",
    "plan_startup",
    "report",
    "read_gate_file",
    "read_status",
    "resume_after_crash",
    "sweep_targets",
]
