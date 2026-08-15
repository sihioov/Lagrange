"""CLI entry for the live node (plan Todo 41).

    python -m live_node start  --account <id> [--lock-dir DIR]
    python -m live_node status --account <id> [--lock-dir DIR]
    python -m live_node plan-startup --account <id> --input state.json

The runbooks in `docs/runbooks/` drive these, and their machine assertions read
the JSON on stdout. Every subcommand therefore prints ONE json object and
nothing else, so a runbook can assert on a field rather than grepping prose
that a later edit would break.

Exit codes are part of the contract, because a runbook's `set -e` reads them:

    0  the node is ready, or the command answered successfully
    1  the command failed
    2  the node is running but NOT ready (blocked, degraded, killed)

2 is distinct from 1 on purpose. "Blocked" is the system working exactly as
designed -- it is what a kill switch and a red reconciliation are supposed to
produce -- and a runbook that could not tell it apart from a crash would
escalate every safe refusal as an outage.
"""
from __future__ import annotations

import argparse
import json
import signal
import sys
import threading
from pathlib import Path

from .health import metrics, report
from .isolation import AccountLock, NodeAlreadyRunning
from .lifecycle import NodeLifecycle, NodeState, NodeStatus
from .runtime import (
    GateState,
    LiveNodeRuntime,
    RuntimeConfig,
    check_liveness,
    read_gate_file,
    read_status,
)
from .startup import plan_startup

EXIT_OK = 0
EXIT_FAILED = 1
EXIT_NOT_READY = 2


def _emit(payload: dict[str, object]) -> None:
    """One JSON object on stdout, nothing else."""
    json.dump(payload, sys.stdout, indent=None, sort_keys=True)
    sys.stdout.write("\n")


def _default_lock_dir() -> Path:
    return Path.cwd() / ".live-node"


def _default_status_path(lock_dir: Path, account: str) -> Path:
    return lock_dir / f"live-node-{account}.status.json"


def _status_path(args: argparse.Namespace) -> Path:
    return args.status_file or _default_status_path(args.lock_dir, args.account)


def _exit_for_payload(payload: dict[str, object]) -> int:
    return EXIT_OK if bool(payload.get("ready")) else EXIT_NOT_READY


def _bool_field(payload: dict[str, object], key: str, default: bool = False) -> bool:
    value = payload.get(key, default)
    return value if isinstance(value, bool) else default


def _effective_gates(defaults: GateState, path: Path | None) -> GateState:
    """Apply an optional gate file as additional, restrictive input."""

    observed = read_gate_file(path, defaults)
    if observed.source_error is not None:
        return observed
    return GateState(
        kill_switch_engaged=defaults.kill_switch_engaged or observed.kill_switch_engaged,
        risk_green=defaults.risk_green and observed.risk_green,
        reconciliation_green=defaults.reconciliation_green
        and observed.reconciliation_green,
        data_fresh=defaults.data_fresh and observed.data_fresh,
    )


def _persisted_status(
    account: str,
    persisted: dict[str, object],
    args: argparse.Namespace,
) -> tuple[dict[str, object], NodeStatus] | None:
    """Re-evaluate persisted state against current restrictive controls.

    A status file is an observation from the loop, not authority to clear a
    newly engaged kill switch.  Rebuilding the pure lifecycle status lets the
    CLI answer a truthful readiness question while the next loop cycle catches
    up and publishes the same decision.
    """

    try:
        state = NodeState(str(persisted["state"]))
    except (KeyError, TypeError, ValueError):
        # Corrupt/unknown state is not safe to report as running or ready.
        return None

    metrics_payload = persisted.get("metrics")
    counters = dict(metrics_payload) if isinstance(metrics_payload, dict) else {}
    kill_from_status = _bool_field(
        persisted,
        "kill_switch_engaged",
        counters.get("kill_switch_state") == 1,
    )
    gates = GateState(
        kill_switch_engaged=kill_from_status or args.kill_switch_engaged,
        risk_green=_bool_field(persisted, "risk_green"),
        reconciliation_green=_bool_field(persisted, "reconciliation_green"),
        data_fresh=_bool_field(persisted, "data_fresh"),
    )
    if args.risk_gate_blocked:
        gates = GateState(
            kill_switch_engaged=gates.kill_switch_engaged,
            risk_green=False,
            reconciliation_green=gates.reconciliation_green,
            data_fresh=gates.data_fresh,
        )
    if args.reconciliation_blocked:
        gates = GateState(
            kill_switch_engaged=gates.kill_switch_engaged,
            risk_green=gates.risk_green,
            reconciliation_green=False,
            data_fresh=gates.data_fresh,
        )
    if args.data_stale:
        gates = GateState(
            kill_switch_engaged=gates.kill_switch_engaged,
            risk_green=gates.risk_green,
            reconciliation_green=gates.reconciliation_green,
            data_fresh=False,
        )
    gates = _effective_gates(gates, args.gate_file)

    degraded = persisted.get("degraded_reasons", ())
    degraded_reasons = (
        tuple(item for item in degraded if isinstance(item, str))
        if isinstance(degraded, (list, tuple))
        else ()
    )
    lifecycle_status = NodeStatus(
        state=state,
        account_id=account,
        kill_switch_engaged=gates.kill_switch_engaged,
        reconciliation_green=gates.reconciliation_green,
        data_fresh=gates.data_fresh,
        risk_green=gates.risk_green,
        execution_enabled=_bool_field(
            persisted, "execution_enabled", not _bool_field(persisted, "dry_run", True)
        )
        and not args.execution_disabled,
        degraded_reasons=degraded_reasons,
    )
    health = report(lifecycle_status).to_dict()
    payload = {
        **persisted,
        **health,
        "account_id": account,
        "kill_switch_engaged": gates.kill_switch_engaged,
        "risk_green": gates.risk_green,
        "reconciliation_green": gates.reconciliation_green,
        "data_fresh": gates.data_fresh,
        "metrics": metrics(lifecycle_status, counters),
    }
    return payload, lifecycle_status


def cmd_start(args: argparse.Namespace) -> int:
    """Claims the account and reports the state the node starts in.

    Does NOT reach READY. A node that started is a node that must reconcile,
    and this command deliberately cannot skip that: it leaves the node in
    RECONCILING and exits 2, so anything scripting a start cannot mistake
    "the process came up" for "the account may trade".
    """
    lock = AccountLock(args.lock_dir, args.account)
    try:
        held = lock.acquire()
    except NodeAlreadyRunning as exc:
        _emit(
            {
                "account_id": args.account,
                "error": "NODE_ALREADY_RUNNING",
                "held_by_pid": exc.holder.pid,
                "message": str(exc),
                "started": False,
            }
        )
        return EXIT_FAILED

    node = NodeLifecycle(args.account)
    node.to(NodeState.RECONCILING)
    status = node.status(
        kill_switch_engaged=args.kill_switch_engaged,
        reconciliation_green=False,
        data_fresh=True,
    )
    payload = report(status).to_dict()
    payload["started"] = True
    payload["pid"] = held.pid
    payload["lock_path"] = str(lock.path)
    _emit(payload)
    # Started, not ready. The distinction is the point.
    return EXIT_OK if status.may_submit() else EXIT_NOT_READY


def cmd_status(args: argparse.Namespace) -> int:
    """Reports health, readiness, and the §15.2 metric set."""
    lock = AccountLock(args.lock_dir, args.account)
    holder = lock.read_holder()

    # A running runtime publishes an atomic, authoritative snapshot. Reading
    # it avoids reconstructing READY/DEGRADED from a lock file and lets the
    # operator see the simulator-vs-trade readiness distinction. A snapshot
    # from another pid is ignored; the lock remains the authority for who owns
    # the account.
    persisted = read_status(_status_path(args))
    if (
        holder is not None
        and persisted is not None
        and persisted.get("pid") == holder.pid
    ):
        evaluated = _persisted_status(args.account, persisted, args)
        if evaluated is not None:
            payload, _status = evaluated
            payload["running"] = True
            payload["held_by_pid"] = holder.pid
            _emit(payload)
            return _exit_for_payload(payload)
    if holder is None and persisted is not None and persisted.get("state") == NodeState.STOPPED.value:
        evaluated = _persisted_status(args.account, persisted, args)
        if evaluated is not None:
            payload, _status = evaluated
            payload["running"] = False
            payload["held_by_pid"] = None
            _emit(payload)
            return _exit_for_payload(payload)

    node = NodeLifecycle(args.account)
    if holder is None:
        # Nothing holds the account: there is no node. STARTING is the honest
        # reading -- not STOPPED, which would claim a node ran and finished.
        state = NodeState.STARTING
    else:
        node.to(NodeState.RECONCILING)
        state = NodeState.RECONCILING

    gates = _effective_gates(
        GateState(
            kill_switch_engaged=args.kill_switch_engaged,
            risk_green=not args.risk_gate_blocked,
            reconciliation_green=args.reconciliation_green
            and not args.reconciliation_blocked,
            data_fresh=not args.data_stale,
        ),
        args.gate_file,
    )
    status = node.status(
        kill_switch_engaged=gates.kill_switch_engaged,
        reconciliation_green=gates.reconciliation_green,
        data_fresh=gates.data_fresh,
        risk_green=gates.risk_green,
        execution_enabled=not args.execution_disabled,
    )
    payload = report(status).to_dict()
    payload["running"] = holder is not None
    payload["held_by_pid"] = holder.pid if holder else None
    payload["metrics"] = metrics(status, {})
    payload["state"] = state.value
    payload["kill_switch_engaged"] = gates.kill_switch_engaged
    payload["risk_green"] = gates.risk_green
    payload["reconciliation_green"] = gates.reconciliation_green
    payload["data_fresh"] = gates.data_fresh
    payload["gate_error"] = gates.source_error
    _emit(payload)
    return _exit_for_payload(payload)


def cmd_healthcheck(args: argparse.Namespace) -> int:
    """Check process ownership and the recent reconciliation heartbeat only."""

    lock = AccountLock(args.lock_dir, args.account)
    healthy, reason = check_liveness(
        lock,
        _status_path(args),
        args.interval_seconds,
    )
    holder = lock.read_holder()
    _emit(
        {
            "account_id": args.account,
            "healthy": healthy,
            "liveness": healthy,
            "status": "healthy" if healthy else "unhealthy",
            "reason": reason,
            "held_by_pid": holder.pid if holder and holder.pid is not None else None,
        }
    )
    return EXIT_OK if healthy else EXIT_FAILED


def cmd_run(args: argparse.Namespace) -> int:
    """Runs the deterministic simulator until SIGTERM/SIGINT or ``--once``."""

    stop_event = threading.Event()

    def request_stop(_signum: int, _frame: object) -> None:
        # The event lets the loop publish STOPPING/STOPPED and release the
        # account lock before the process exits. A shell trap alone can only
        # kill the child between writes.
        stop_event.set()

    previous: dict[int, object] = {}
    try:
        signals = [signal.SIGINT, signal.SIGTERM]
        if hasattr(signal, "SIGHUP"):
            signals.append(signal.SIGHUP)
        for signum in signals:
            previous[signum] = signal.signal(signum, request_stop)
        config = RuntimeConfig(
            account_id=args.account,
            lock_dir=args.lock_dir,
            status_path=args.status_file,
            interval_seconds=args.interval_seconds,
            kill_switch_engaged=args.kill_switch_engaged,
            risk_green=not args.risk_gate_blocked,
            reconciliation_green=not args.reconciliation_blocked,
            data_fresh=not args.data_stale,
            execution_enabled=args.execution_enabled,
            gate_file=args.gate_file,
            run_once=args.run_once,
        )
        final = LiveNodeRuntime(config).run(stop_event)
    except NodeAlreadyRunning as exc:
        _emit(
            {
                "account_id": args.account,
                "error": "NODE_ALREADY_RUNNING",
                "held_by_pid": exc.holder.pid,
                "started": False,
                "message": str(exc),
            }
        )
        return EXIT_FAILED
    except (OSError, RuntimeError, ValueError):
        # Keep runtime failures machine-readable and avoid echoing a path or
        # control-file contents into logs. The detailed failure remains local
        # to the process supervisor's exit status.
        _emit(
            {
                "account_id": args.account,
                "error": "LIVE_RUNTIME_FAILED",
                "started": False,
            }
        )
        return EXIT_FAILED
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
    _emit(final)
    return EXIT_OK


def cmd_plan_startup(args: argparse.Namespace) -> int:
    """Prints the startup plan for a recorded state, without acting on it.

    The runbooks use this to show an operator what a restart WOULD do before
    it does it -- particularly which intents are about to be swept to UNKNOWN,
    since that is the step whose absence would let a crashed node resume
    trading against books it has not re-checked.
    """
    raw = json.loads(Path(args.input).read_text(encoding="utf-8"))
    plan = plan_startup(
        raw.get("intent_states", {}),
        mismatch_kinds=tuple(raw.get("blocking_mismatch_kinds", [])),
        fills_to_apply=tuple(raw.get("fills_to_apply", [])),
        lookups_required=tuple(raw.get("lookups_required", [])),
    )
    _emit(
        {
            "account_id": args.account,
            "blocking_reasons": list(plan.blocking_reasons),
            "fills_to_apply": list(plan.fills_to_apply),
            "lookups_required": list(plan.lookups_required),
            "may_trade": plan.may_trade,
            "outcome": plan.outcome.value,
            "to_sweep": list(plan.to_sweep),
        }
    )
    return EXIT_OK if plan.may_trade else EXIT_NOT_READY


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Lagrange Station Live node", prog="python -m live_node"
    )
    parser.add_argument(
        "--lock-dir",
        type=Path,
        default=_default_lock_dir(),
        help="directory holding the one-node-per-account lock",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    start = sub.add_parser("start", help="claim the account and begin reconciling")
    start.add_argument("--account", required=True)
    start.add_argument("--kill-switch-engaged", action="store_true")
    start.set_defaults(func=cmd_start)

    status = sub.add_parser("status", help="health, readiness, and metrics")
    status.add_argument("--account", required=True)
    status.add_argument("--kill-switch-engaged", action="store_true")
    status.add_argument("--reconciliation-green", action="store_true")
    status.add_argument("--reconciliation-blocked", action="store_true")
    status.add_argument("--data-stale", action="store_true")
    status.add_argument("--risk-gate-blocked", action="store_true")
    status.add_argument("--execution-disabled", action="store_true")
    status.add_argument("--gate-file", type=Path)
    status.add_argument("--status-file", type=Path)
    status.set_defaults(func=cmd_status)

    healthcheck = sub.add_parser(
        "healthcheck", help="verify owner PID and recent reconciliation liveness"
    )
    healthcheck.add_argument("--account", required=True)
    healthcheck.add_argument("--status-file", type=Path)
    healthcheck.add_argument("--interval-seconds", type=float, default=30.0)
    healthcheck.set_defaults(func=cmd_healthcheck)

    run = sub.add_parser("run", help="run the credential-free deterministic simulator")
    run.add_argument("--account", required=True)
    run.add_argument("--interval-seconds", type=float, default=30.0)
    run.add_argument("--status-file", type=Path)
    run.add_argument("--gate-file", type=Path)
    run.add_argument("--kill-switch-engaged", action="store_true")
    run.add_argument("--risk-gate-blocked", action="store_true")
    run.add_argument("--reconciliation-blocked", action="store_true")
    run.add_argument("--data-stale", action="store_true")
    run.add_argument(
        "--execution-enabled",
        action="store_true",
        help="reserved for a reviewed broker-backed runtime; simulator still sends nothing",
    )
    run.add_argument("--run-once", action="store_true", help="reconcile once, then stop")
    run.set_defaults(func=cmd_run)

    plan = sub.add_parser("plan-startup", help="show what a restart would do")
    plan.add_argument("--account", required=True)
    plan.add_argument("--input", required=True, type=Path)
    plan.set_defaults(func=cmd_plan_startup)

    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
