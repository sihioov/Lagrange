"""Credential-free runtime contract tests.

These tests deliberately never construct a KIS client. They prove the
container has useful liveness/reconciliation behaviour without turning a
simulator into a claim that real trading is available.
"""
from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from live_node.runtime import (
    GateState,
    LiveNodeRuntime,
    RuntimeConfig,
    check_liveness,
    _write_status,
    read_gate_file,
    read_status,
)
from live_node.isolation import AccountLock
from live_node.lifecycle import NodeState


def status_path(root: Path, account: str = "acct-runtime") -> Path:
    return root / f"live-node-{account}.status.json"


def wait_for_cycle(path: Path) -> dict[str, object]:
    for _ in range(200):
        payload = read_status(path)
        if payload is not None and int(payload.get("cycle", 0)) >= 1:
            return payload
        time.sleep(0.005)
    pytest.fail(f"runtime did not publish a reconciliation cycle: {path}")


def test_dry_run_reconciles_but_never_claims_trade_readiness(tmp_path):
    stop = threading.Event()
    runtime = LiveNodeRuntime(
        RuntimeConfig("acct-runtime", tmp_path, interval_seconds=0.01)
    )
    thread = threading.Thread(target=runtime.run, args=(stop,), daemon=True)
    thread.start()

    cycle = wait_for_cycle(status_path(tmp_path))
    assert cycle["state"] == "READY"
    assert cycle["healthy"] is True
    assert cycle["simulation_ready"] is True
    assert cycle["trade_ready"] is False
    assert cycle["ready"] is False
    assert cycle["refusal"] == "LIVE_DRY_RUN"
    assert cycle["reconciliation_green"] is True
    assert cycle["risk_green"] is True
    assert cycle["metrics"]["orders_submitted_total"] == 0
    assert (tmp_path / "live-node-acct-runtime.lock").exists()

    stop.set()
    thread.join(timeout=2)
    assert not thread.is_alive()
    assert not (tmp_path / "live-node-acct-runtime.lock").exists()
    assert read_status(status_path(tmp_path))["state"] == "STOPPED"


def test_blocked_risk_gate_never_reaches_simulation_ready(tmp_path):
    seen: list[dict[str, object]] = []
    runtime = LiveNodeRuntime(
        RuntimeConfig(
            "acct-runtime",
            tmp_path,
            risk_green=False,
            interval_seconds=0.01,
            run_once=True,
        )
    )
    runtime.run(on_status=seen.append)
    cycle = next(payload for payload in seen if payload["cycle"] == 1)
    assert cycle["state"] == "RECONCILING"
    assert cycle["risk_green"] is False
    assert cycle["simulation_ready"] is False
    assert cycle["trade_ready"] is False
    assert cycle["refusal"] == "LIVE_RISK_GATE_BLOCKED"
    assert cycle["metrics"]["orders_submitted_total"] == 0


def test_invalid_gate_file_fails_closed(tmp_path):
    gate_file = tmp_path / "gates.json"
    gate_file.write_text('{"risk_green": true}', encoding="utf-8")
    seen: list[dict[str, object]] = []
    runtime = LiveNodeRuntime(
        RuntimeConfig(
            "acct-runtime",
            tmp_path,
            gate_file=gate_file,
            interval_seconds=0.01,
            run_once=True,
        )
    )
    runtime.run(on_status=seen.append)
    cycle = next(payload for payload in seen if payload["cycle"] == 1)
    assert cycle["gate_error"] == "LIVE_GATE_FILE_INVALID"
    assert cycle["risk_green"] is False
    assert cycle["reconciliation_green"] is False
    assert cycle["metrics"]["kill_switch_state"] == 1


def test_a_gate_file_cannot_clear_a_configured_kill_switch(tmp_path):
    gate_file = tmp_path / "gates.json"
    gate_file.write_text(
        json.dumps(
            {
                "kill_switch_engaged": False,
                "risk_green": True,
                "reconciliation_green": True,
                "data_fresh": True,
            }
        ),
        encoding="utf-8",
    )
    seen: list[dict[str, object]] = []
    runtime = LiveNodeRuntime(
        RuntimeConfig(
            "acct-runtime",
            tmp_path,
            kill_switch_engaged=True,
            gate_file=gate_file,
            interval_seconds=0.01,
            run_once=True,
        )
    )
    runtime.run(on_status=seen.append)
    cycle = next(payload for payload in seen if payload["cycle"] == 1)
    assert cycle["kill_switch_engaged"] is True
    assert cycle["refusal"] == "LIVE_KILL_SWITCH_ENGAGED"
    assert cycle["simulation_ready"] is False


def test_kill_switch_clear_requires_a_new_reconciliation(tmp_path):
    runtime = LiveNodeRuntime(RuntimeConfig("acct-runtime", tmp_path))
    snapshot = runtime.simulator.reconcile(1)
    runtime.node.to(NodeState.RECONCILING)
    runtime._advance_state(GateState.safe_default(), snapshot)
    assert runtime.node.state.value == "READY"

    runtime._advance_state(
        GateState(
            kill_switch_engaged=True,
            risk_green=True,
            reconciliation_green=True,
            data_fresh=True,
        ),
        snapshot,
    )
    assert runtime.node.state.value == "DEGRADED"
    runtime._advance_state(GateState.safe_default(), snapshot)
    assert runtime.node.state.value == "RECONCILING"


def test_gate_symlink_fails_closed(tmp_path):
    target = tmp_path / "gates.json"
    target.write_text(
        '{"kill_switch_engaged":false,"risk_green":true,'
        '"reconciliation_green":true,"data_fresh":true}',
        encoding="utf-8",
    )
    link = tmp_path / "gates-link.json"
    try:
        link.symlink_to(target)
    except (NotImplementedError, OSError):
        pytest.skip("symlinks unavailable")
    gates = read_gate_file(link, GateState.safe_default())
    assert gates.source_error == "LIVE_GATE_FILE_INVALID"
    assert gates.kill_switch_engaged is True


def test_status_is_owner_only_and_published_as_a_complete_document(tmp_path):
    path = tmp_path / "status.json"
    _write_status(path, {"state": "READY", "ready": False})
    assert stat.S_IMODE(path.stat().st_mode) == 0o600
    assert read_status(path) == {"ready": False, "state": "READY"}


def test_liveness_requires_lock_owner_and_recent_reconciliation(tmp_path):
    account = "acct-runtime"
    lock = AccountLock(tmp_path, account)
    with lock as holder:
        path = status_path(tmp_path, account)
        _write_status(
            path,
            {"pid": holder.pid, "last_reconciliation_at": 90.0, "ready": False},
        )
        assert check_liveness(lock, path, 10.0, now=100.0) == (True, "process_alive")
        assert check_liveness(lock, path, 10.0, now=121.0) == (
            False,
            "LIVE_NODE_RECONCILIATION_STALE",
        )
        _write_status(path, {"pid": holder.pid + 1, "last_reconciliation_at": 100.0})
        assert check_liveness(lock, path, 10.0, now=100.0) == (
            False,
            "LIVE_NODE_STATUS_OWNER_MISMATCH",
        )


def test_liveness_does_not_require_trade_readiness(tmp_path):
    account = "acct-runtime"
    lock = AccountLock(tmp_path, account)
    with lock as holder:
        path = status_path(tmp_path, account)
        _write_status(
            path,
            {
                "pid": holder.pid,
                "last_reconciliation_at": 100.0,
                "ready": False,
                "dry_run": True,
            },
        )
        assert check_liveness(lock, path, 30.0, now=100.0)[0] is True


def test_direct_live_execution_request_is_fail_closed(tmp_path):
    with pytest.raises(ValueError, match="reviewed KIS runtime"):
        RuntimeConfig("acct-runtime", tmp_path, execution_enabled=True)


def test_sigterm_stops_runtime_and_releases_account_lock(tmp_path):
    root = Path(__file__).resolve().parents[1]
    status = status_path(tmp_path)
    environment = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "PYTHONPATH": str(root),
        "PYTHONUNBUFFERED": "1",
    }
    process = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "live_node",
            "--lock-dir",
            str(tmp_path),
            "run",
            "--account",
            "acct-runtime",
            "--interval-seconds",
            "1",
            "--status-file",
            str(status),
        ],
        cwd=root,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        wait_for_cycle(status)
        process.terminate()
        assert process.wait(timeout=2) == 0
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=2)
    assert not (tmp_path / "live-node-acct-runtime.lock").exists()
    final = read_status(status)
    assert final["state"] == "STOPPED"
    output = process.stdout.read() if process.stdout else ""
    assert json.loads(output)["state"] == "STOPPED"


def test_entrypoint_exposes_live_health_and_truthful_dry_run_readiness(tmp_path):
    package_root = Path(__file__).resolve().parents[1]
    entrypoint = package_root / "runtime" / "live-node-entrypoint"
    status = status_path(tmp_path)
    environment = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "PYTHONPATH": str(package_root),
        "PYTHONUNBUFFERED": "1",
        "LIVE_NODE_MODE": "enabled",
        "LIVE_NODE_DRY_RUN": "1",
        "LIVE_NODE_ACCOUNT_ID": "acct-entrypoint",
        "LIVE_NODE_LOCK_DIR": str(tmp_path),
        "LIVE_NODE_STATUS_FILE": str(status),
        "LIVE_NODE_RECONCILIATION_INTERVAL_SECONDS": "1",
        "LIVE_NODE_PYTHON": sys.executable,
    }
    process = subprocess.Popen(
        [str(entrypoint), "run"],
        cwd=package_root.parent.parent,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        wait_for_cycle(status)
        health = subprocess.run(
            [str(entrypoint), "healthcheck"],
            cwd=package_root.parent.parent,
            env=environment,
            check=True,
            capture_output=True,
            text=True,
        )
        assert json.loads(health.stdout)["status"] == "healthy"
        readiness = subprocess.run(
            [str(entrypoint), "readiness"],
            cwd=package_root.parent.parent,
            env=environment,
            capture_output=True,
            text=True,
        )
        assert readiness.returncode == 1
        body = json.loads(readiness.stdout)
        assert body["ready"] is False
        assert body["simulation_ready"] is True
        assert body["trade_ready"] is False
    finally:
        process.terminate()
        process.wait(timeout=2)
    assert not (tmp_path / "live-node-acct-entrypoint.lock").exists()
    assert read_status(status)["state"] == "STOPPED"
