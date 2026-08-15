"""The credential-free Live runtime.

The repository's KIS adapter is deliberately still an external-only boundary:
this module never imports it, opens a socket, reads a secret, or submits an
order.  It gives the owner-profile container a real process to operate while
that boundary is unavailable.  The simulator owns one account lock, performs a
deterministic reconciliation on every cycle, and exposes the same safety gates
that a future broker-backed loop must satisfy.

There are two different readiness answers in the status document:

``simulation_ready``
    The local loop is alive and its deterministic account snapshot is green.
``trade_ready`` / ``ready``
    A real order could be submitted.  This is always false in dry-run mode,
    even after the simulator reconciles successfully.

That distinction matters operationally.  A dry-run process should stay up so
its lock, health, and reconciliation machinery are exercised, but it must
never be mistaken for a KIS-connected trading process.
"""
from __future__ import annotations

import hashlib
import json
import math
import os
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Mapping

from .health import metrics, report
from .isolation import ACCOUNT_ID_RE, AccountLock, _pid_alive
from .lifecycle import NodeLifecycle, NodeState

_GATE_KEYS = ("kill_switch_engaged", "risk_green", "reconciliation_green", "data_fresh")

# A node may spend a little longer than its nominal interval in reconciliation,
# but a fixed upper multiple keeps an old heartbeat from being mistaken for a
# live process forever.  Readiness remains a separate, gate-aware answer.
LIVENESS_MAX_INTERVAL_MULTIPLE = 3.0


def check_liveness(
    lock: AccountLock,
    status_path: Path,
    interval_seconds: float,
    *,
    now: float | None = None,
) -> tuple[bool, str]:
    """Verify the process owner and a bounded, recent reconciliation heartbeat."""

    if interval_seconds <= 0 or not math.isfinite(interval_seconds):
        return False, "LIVE_NODE_INTERVAL_INVALID"
    holder = lock.read_holder()
    if holder is None or holder.pid is None:
        return False, "LIVE_NODE_NOT_RUNNING"
    if not _pid_alive(holder.pid):
        return False, "LIVE_NODE_OWNER_DEAD"
    status = read_status(status_path)
    if status is None:
        return False, "LIVE_NODE_STATUS_UNAVAILABLE"
    if status.get("pid") != holder.pid:
        return False, "LIVE_NODE_STATUS_OWNER_MISMATCH"
    heartbeat = status.get("last_reconciliation_at")
    if not isinstance(heartbeat, (int, float)) or isinstance(heartbeat, bool):
        return False, "LIVE_NODE_RECONCILIATION_MISSING"
    if not math.isfinite(float(heartbeat)):
        return False, "LIVE_NODE_RECONCILIATION_INVALID"
    observed_at = time.time() if now is None else now
    age = observed_at - float(heartbeat)
    if age < 0 or age > interval_seconds * LIVENESS_MAX_INTERVAL_MULTIPLE:
        return False, "LIVE_NODE_RECONCILIATION_STALE"
    return True, "process_alive"


@dataclass(frozen=True)
class GateState:
    """The inputs which can block a cycle.

    ``source_error`` is intentionally descriptive but never includes file
    contents.  A malformed or unreadable control file is fail-closed: the
    kill switch is considered engaged and all positive gates are red.
    """

    kill_switch_engaged: bool
    risk_green: bool
    reconciliation_green: bool
    data_fresh: bool
    source_error: str | None = None

    @classmethod
    def safe_default(cls) -> "GateState":
        # The deterministic simulator has no broker and therefore has no
        # external risk source to consult. Its own fixed snapshot is green,
        # while the execution-enabled flag still prevents live submission.
        return cls(
            kill_switch_engaged=False,
            risk_green=True,
            reconciliation_green=True,
            data_fresh=True,
        )

    @classmethod
    def blocked(cls, reason: str) -> "GateState":
        return cls(
            kill_switch_engaged=True,
            risk_green=False,
            reconciliation_green=False,
            data_fresh=False,
            source_error=reason,
        )


@dataclass(frozen=True)
class SimulatorSnapshot:
    """Stable account state returned by every simulator reconciliation."""

    cycle: int
    cash: str = "0.00"
    positions: tuple[tuple[str, str], ...] = ()
    open_orders: tuple[str, ...] = ()
    mismatches: tuple[str, ...] = ()

    @property
    def green(self) -> bool:
        return not self.mismatches

    @property
    def state_hash(self) -> str:
        canonical = json.dumps(
            {
                "cash": self.cash,
                "open_orders": list(self.open_orders),
                "positions": [list(item) for item in self.positions],
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
        return hashlib.sha256(canonical).hexdigest()

    def to_dict(self) -> dict[str, object]:
        return {
            "cash": self.cash,
            "cycle": self.cycle,
            "mismatches": list(self.mismatches),
            "open_orders": list(self.open_orders),
            "positions": {symbol: quantity for symbol, quantity in self.positions},
            "state_hash": self.state_hash,
        }


class DeterministicSimulator:
    """A no-network broker/account model used only by the dry-run runtime."""

    def reconcile(self, cycle: int) -> SimulatorSnapshot:
        # Keep this deliberately boring and deterministic. No strategy signal
        # is generated and no order is ever submitted; the loop exercises
        # startup/reconciliation/liveness contracts only.
        return SimulatorSnapshot(cycle=cycle)


@dataclass(frozen=True)
class RuntimeConfig:
    account_id: str
    lock_dir: Path
    status_path: Path | None = None
    interval_seconds: float = 30.0
    kill_switch_engaged: bool = False
    risk_green: bool = True
    reconciliation_green: bool = True
    data_fresh: bool = True
    execution_enabled: bool = False
    gate_file: Path | None = None
    run_once: bool = False

    def __post_init__(self) -> None:
        if not self.account_id or not ACCOUNT_ID_RE.fullmatch(self.account_id):
            raise ValueError("account_id contains unsupported characters")
        if self.interval_seconds <= 0:
            raise ValueError("interval_seconds must be positive")
        if self.execution_enabled:
            # There is intentionally no broker-backed implementation in this
            # package. Refuse even a direct Python caller that tries to bypass
            # the shell boundary; only the future reviewed KIS runtime may
            # ever set this bit.
            raise ValueError("live execution is unavailable without a reviewed KIS runtime")

    @property
    def resolved_status_path(self) -> Path:
        return self.status_path or (self.lock_dir / f"live-node-{self.account_id}.status.json")


def _gate_value(value: object, key: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{key} must be a JSON boolean")
    return value


def read_gate_file(path: Path | None, defaults: GateState) -> GateState:
    """Read the optional operator control document without leaking contents."""

    if path is None:
        return defaults
    try:
        if path.is_symlink():
            return GateState.blocked("LIVE_GATE_FILE_INVALID")
        raw = path.read_text(encoding="utf-8")
        document = json.loads(raw)
        if not isinstance(document, dict):
            raise ValueError("gate document must be an object")
        # Requiring all fields avoids treating an accidentally truncated file
        # as an all-clear. Values are booleans, never secret-bearing strings.
        values = {key: _gate_value(document[key], key) for key in _GATE_KEYS}
    except FileNotFoundError:
        return GateState.blocked("LIVE_GATE_FILE_UNAVAILABLE")
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError, ValueError):
        return GateState.blocked("LIVE_GATE_FILE_INVALID")
    return GateState(**values)


def read_status(path: Path) -> dict[str, object] | None:
    """Read the last atomic status document, treating corruption as absent."""

    try:
        raw = path.read_text(encoding="utf-8")
        value = json.loads(raw)
    except (FileNotFoundError, OSError, UnicodeError, json.JSONDecodeError):
        return None
    return value if isinstance(value, dict) else None


def _write_status(path: Path, payload: Mapping[str, object]) -> None:
    """Publish status atomically and with owner-only permissions."""

    parent = path.parent
    if parent.is_symlink():
        raise RuntimeError("LIVE_NODE_STATUS_FILE parent must not be a symlink")
    parent.mkdir(parents=True, exist_ok=True)
    fd, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=parent)
    temporary = Path(temporary_name)
    try:
        os.chmod(temporary, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(dict(payload), handle, separators=(",", ":"), sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


class LiveNodeRuntime:
    """One-account, graceful, deterministic dry-run process."""

    def __init__(
        self,
        config: RuntimeConfig,
        *,
        simulator: DeterministicSimulator | None = None,
        clock: Callable[[], float] = time.time,
    ) -> None:
        self.config = config
        self.simulator = simulator or DeterministicSimulator()
        self.clock = clock
        self.node = NodeLifecycle(config.account_id)
        self.cycle = 0
        self.counters = {
            "orders_submitted_total": 0,
            "orders_rejected_total": 0,
            "unknown_order_states": 0,
            "reconciliation_mismatches": 0,
            "stale_data_blocks": 0,
            "kill_switch_state": 0,
        }
        self._last_reconciliation: SimulatorSnapshot | None = None
        self._last_reconciliation_at: float | None = None
        self._started_at: float | None = None

    def _defaults(self) -> GateState:
        return GateState(
            kill_switch_engaged=self.config.kill_switch_engaged,
            risk_green=self.config.risk_green,
            reconciliation_green=self.config.reconciliation_green,
            data_fresh=self.config.data_fresh,
        )

    def _gates(self) -> GateState:
        defaults = self._defaults()
        observed = read_gate_file(self.config.gate_file, defaults)
        if observed.source_error is not None:
            return observed

        # A gate file is an additional source of restrictions, never an
        # override that can clear a safer command-line/configuration value.
        # In particular, a stale or compromised file must not disengage a
        # configured kill switch or turn a red freshness/risk gate green.
        return GateState(
            kill_switch_engaged=defaults.kill_switch_engaged
            or observed.kill_switch_engaged,
            risk_green=defaults.risk_green and observed.risk_green,
            reconciliation_green=defaults.reconciliation_green
            and observed.reconciliation_green,
            data_fresh=defaults.data_fresh and observed.data_fresh,
        )

    def _advance_state(self, gates: GateState, snapshot: SimulatorSnapshot) -> None:
        reconciliation_green = gates.reconciliation_green and snapshot.green
        all_green = (
            not gates.kill_switch_engaged
            and gates.risk_green
            and reconciliation_green
            and gates.data_fresh
        )
        if self.node.state is NodeState.READY and not all_green:
            reasons = []
            if gates.kill_switch_engaged:
                reasons.append("kill switch engaged")
            if not gates.risk_green:
                reasons.append("risk gate blocked")
            if not reconciliation_green:
                reasons.append("reconciliation not green")
            if not gates.data_fresh:
                reasons.append("data stale")
            self.node.degrade(", ".join(reasons) or "runtime gate blocked")
        elif self.node.state is NodeState.DEGRADED and all_green:
            # A degraded process must earn READY through another explicit
            # reconciliation; never jump directly from DEGRADED to READY.
            self.node.to(NodeState.RECONCILING)
        elif self.node.state is NodeState.RECONCILING and all_green:
            self.node.to(NodeState.READY)

    def _payload(self, gates: GateState) -> dict[str, object]:
        snapshot = self._last_reconciliation
        reconciliation_green = bool(snapshot and snapshot.green and gates.reconciliation_green)
        status = self.node.status(
            kill_switch_engaged=gates.kill_switch_engaged,
            reconciliation_green=reconciliation_green,
            data_fresh=gates.data_fresh,
            risk_green=gates.risk_green,
            execution_enabled=self.config.execution_enabled,
        )
        health = report(status).to_dict()
        trade_ready = bool(health["ready"])
        simulation_ready = (
            self.node.state is NodeState.READY
            and not gates.kill_switch_engaged
            and gates.risk_green
            and reconciliation_green
            and gates.data_fresh
        )
        payload: dict[str, object] = {
            **health,
            "account_id": self.config.account_id,
            "pid": os.getpid(),
            "cycle": self.cycle,
            "dry_run": not self.config.execution_enabled,
            "execution_mode": "live" if self.config.execution_enabled else "dry-run",
            "live_execution": self.config.execution_enabled,
            "kill_switch_engaged": gates.kill_switch_engaged,
            "simulation_ready": simulation_ready,
            "trade_ready": trade_ready,
            "started_at": self._started_at,
            "last_reconciliation_at": self._last_reconciliation_at,
            "reconciliation": snapshot.to_dict() if snapshot else None,
            "risk_green": gates.risk_green,
            "reconciliation_green": reconciliation_green,
            "data_fresh": gates.data_fresh,
            "gate_error": gates.source_error,
            "metrics": metrics(status, self.counters),
        }
        return payload

    def _publish(self, gates: GateState) -> dict[str, object]:
        payload = self._payload(gates)
        _write_status(self.config.resolved_status_path, payload)
        return payload

    def _stop(self, gates: GateState) -> dict[str, object]:
        if self.node.state not in (NodeState.STOPPING, NodeState.STOPPED):
            self.node.to(NodeState.STOPPING)
            self._publish(gates)
            self.node.to(NodeState.STOPPED)
        return self._publish(gates)

    def run(
        self,
        stop_event: threading.Event | None = None,
        *,
        on_status: Callable[[dict[str, object]], None] | None = None,
    ) -> dict[str, object]:
        """Run until stopped, or one reconciliation when ``run_once`` is set."""

        stop_event = stop_event or threading.Event()
        lock = AccountLock(self.config.lock_dir, self.config.account_id)
        with lock as holder:
            self._started_at = self.clock()
            self.node.to(NodeState.RECONCILING)
            initial = self._publish(self._gates())
            if on_status:
                on_status(initial)
            while not stop_event.is_set():
                self.cycle += 1
                try:
                    snapshot = self.simulator.reconcile(self.cycle)
                except Exception:  # pragma: no cover - defensive boundary
                    self.node.degrade("simulator reconciliation failed")
                    gates = GateState.blocked("LIVE_RECONCILIATION_FAILED")
                    self._last_reconciliation = SimulatorSnapshot(
                        cycle=self.cycle, mismatches=("SIMULATOR_FAILURE",)
                    )
                    payload = self._publish(gates)
                    if on_status:
                        on_status(payload)
                    if self.config.run_once:
                        break
                    if stop_event.wait(self.config.interval_seconds):
                        break
                    continue

                self._last_reconciliation = snapshot
                self._last_reconciliation_at = self.clock()
                gates = self._gates()
                self.counters["reconciliation_mismatches"] = len(snapshot.mismatches)
                if not gates.data_fresh:
                    self.counters["stale_data_blocks"] += 1
                self.counters["kill_switch_state"] = int(gates.kill_switch_engaged)
                self._advance_state(gates, snapshot)
                payload = self._publish(gates)
                if on_status:
                    on_status(payload)
                if self.config.run_once:
                    break
                if stop_event.wait(self.config.interval_seconds):
                    break

            final = self._stop(self._gates())
            if on_status:
                on_status(final)
            # Include the holder in the returned value for callers that want
            # to correlate a status event without writing it to disk.
            final["pid"] = holder.pid
            return final
