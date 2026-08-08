"""Todo 41: the CLI the runbooks drive.

Exit codes are the contract, because a runbook's `set -e` reads them. The one
that matters is 2: "running but not ready" is the system working exactly as
designed -- a kill switch and a red reconciliation are SUPPOSED to produce it
-- and a runbook that could not tell it apart from a crash would escalate
every safe refusal as an outage.
"""
from __future__ import annotations

import json

from live_node.__main__ import EXIT_FAILED, EXIT_NOT_READY, EXIT_OK, main


def run(capsys, argv):
    code = main(argv)
    out = capsys.readouterr().out.strip()
    return code, json.loads(out) if out else None


def test_start_claims_the_account_but_never_reports_ready(capsys, tmp_path):
    # A node that started is a node that must reconcile. `start` deliberately
    # cannot skip that, so anything scripting a start cannot mistake "the
    # process came up" for "the account may trade".
    code, body = run(
        capsys, ["--lock-dir", str(tmp_path), "start", "--account", "acct-1"]
    )
    assert code == EXIT_NOT_READY
    assert body["started"] is True
    assert body["ready"] is False
    assert body["state"] == "RECONCILING"
    assert body["healthy"] is True


def test_start_refuses_when_another_node_holds_the_account(capsys, tmp_path):
    run(capsys, ["--lock-dir", str(tmp_path), "start", "--account", "acct-1"])
    code, body = run(
        capsys, ["--lock-dir", str(tmp_path), "start", "--account", "acct-1"]
    )
    assert code == EXIT_FAILED, "a refused start is a failure, not a safe refusal"
    assert body["started"] is False
    assert body["error"] == "NODE_ALREADY_RUNNING"


def test_status_distinguishes_no_node_from_a_running_one(capsys, tmp_path):
    code, body = run(
        capsys, ["--lock-dir", str(tmp_path), "status", "--account", "acct-1"]
    )
    assert code == EXIT_NOT_READY
    assert body["running"] is False
    # STARTING, not STOPPED: STOPPED would claim a node ran and finished.
    assert body["state"] == "STARTING"

    run(capsys, ["--lock-dir", str(tmp_path), "start", "--account", "acct-1"])
    _, body = run(
        capsys, ["--lock-dir", str(tmp_path), "status", "--account", "acct-1"]
    )
    assert body["running"] is True
    assert body["held_by_pid"] is not None


def test_a_killed_node_exits_two_not_one(capsys, tmp_path):
    # The distinction the runbooks depend on. Engaged is healthy-but-not-ready,
    # which is a safe refusal rather than a fault.
    code, body = run(
        capsys,
        [
            "--lock-dir",
            str(tmp_path),
            "status",
            "--account",
            "acct-1",
            "--kill-switch-engaged",
            "--reconciliation-green",
        ],
    )
    assert code == EXIT_NOT_READY
    assert body["healthy"] is True
    assert body["refusal"] == "LIVE_KILL_SWITCH_ENGAGED"
    assert body["metrics"]["kill_switch_state"] == 1


def test_status_emits_every_documented_metric(capsys, tmp_path):
    _, body = run(
        capsys, ["--lock-dir", str(tmp_path), "status", "--account", "acct-1"]
    )
    for name in (
        "orders_submitted_total",
        "orders_rejected_total",
        "unknown_order_states",
        "reconciliation_mismatches",
        "stale_data_blocks",
        "kill_switch_state",
    ):
        # An absent metric is indistinguishable from a failed scrape; an
        # explicit zero is not.
        assert name in body["metrics"], name


def test_plan_startup_shows_what_a_restart_would_sweep(capsys, tmp_path):
    # The step whose absence would let a crashed node resume trading against
    # books it has not re-checked, shown BEFORE it happens.
    state = tmp_path / "state.json"
    state.write_text(
        json.dumps(
            {
                "blocking_mismatch_kinds": [],
                "fills_to_apply": ["E-1"],
                "intent_states": {"oi-1": "SUBMITTING", "oi-2": "FILLED"},
                "lookups_required": [],
            }
        ),
        encoding="utf-8",
    )
    code, body = run(
        capsys,
        [
            "--lock-dir",
            str(tmp_path),
            "plan-startup",
            "--account",
            "acct-1",
            "--input",
            str(state),
        ],
    )
    assert body["to_sweep"] == ["oi-1"]
    assert body["fills_to_apply"] == ["E-1"]
    assert body["outcome"] == "READY"
    assert code == EXIT_OK


def test_plan_startup_blocks_and_says_why(capsys, tmp_path):
    state = tmp_path / "state.json"
    state.write_text(
        json.dumps(
            {
                "blocking_mismatch_kinds": ["POSITION"],
                "fills_to_apply": [],
                "intent_states": {},
                "lookups_required": [],
            }
        ),
        encoding="utf-8",
    )
    code, body = run(
        capsys,
        [
            "--lock-dir",
            str(tmp_path),
            "plan-startup",
            "--account",
            "acct-1",
            "--input",
            str(state),
        ],
    )
    assert code == EXIT_NOT_READY
    assert body["may_trade"] is False
    assert body["blocking_reasons"] == ["POSITION"]


def test_every_command_prints_exactly_one_json_object(capsys, tmp_path):
    # Runbook assertions read a field rather than grepping prose that a later
    # edit would break, so stray output would break them.
    for argv in (
        ["--lock-dir", str(tmp_path), "start", "--account", "acct-1"],
        ["--lock-dir", str(tmp_path), "status", "--account", "acct-1"],
    ):
        main(argv)
        out = capsys.readouterr().out
        assert out.count("\n") == 1, out
        json.loads(out)
