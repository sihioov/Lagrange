"""`backtest_worker` — productionized NT backtest worker (plan Todo 20).

One job = one fresh child process (ADR-005), launched under OS-level isolation:
CPU/memory/time limits, one-node-per-process, network disabled, read-only
dataset/catalog mounts. The child produces raw NT results; the supervisor
normalizes them into the common model, validates integrity, and writes
structured status/provenance output. See `worker.py` for the orchestrator.
"""

from .isolation import IsolationLimits, ProcessIsolation, Termination

__all__ = ["IsolationLimits", "ProcessIsolation", "Termination"]
