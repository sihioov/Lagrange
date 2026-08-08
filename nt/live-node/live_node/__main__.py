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
import sys
from pathlib import Path

from .health import metrics, report
from .isolation import AccountLock, NodeAlreadyRunning
from .lifecycle import NodeLifecycle, NodeState
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

    node = NodeLifecycle(args.account)
    if holder is None:
        # Nothing holds the account: there is no node. STARTING is the honest
        # reading -- not STOPPED, which would claim a node ran and finished.
        state = NodeState.STARTING
    else:
        node.to(NodeState.RECONCILING)
        state = NodeState.RECONCILING

    status = node.status(
        kill_switch_engaged=args.kill_switch_engaged,
        reconciliation_green=args.reconciliation_green,
        data_fresh=not args.data_stale,
    )
    payload = report(status).to_dict()
    payload["running"] = holder is not None
    payload["held_by_pid"] = holder.pid if holder else None
    payload["metrics"] = metrics(status, {})
    payload["state"] = state.value
    _emit(payload)
    return EXIT_OK if status.may_submit() else EXIT_NOT_READY


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
    status.add_argument("--data-stale", action="store_true")
    status.set_defaults(func=cmd_status)

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
