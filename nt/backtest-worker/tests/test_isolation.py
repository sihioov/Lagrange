"""Red tests (Todo 20): one-node-per-process worker isolation.

Covers the OS-level mechanisms the worker supervisor applies to the NT child:
CPU/memory/time limits, one-node-per-process (no grandchild processes), the
child runtime guard (network disabled, read-only dataset/catalog mounts), and
graceful termination with structured status output.

Platform notes: Windows enforces memory/CPU/active-process limits via a Job
Object (probed green on this host); POSIX uses resource rlimits for
memory/CPU plus a process group for termination. One-node-per-process has no
unprivileged POSIX equivalent (cgroups/containers cover it in the Compose
leg), so the grandchild test skips on POSIX with that reason. Network and
read-only-mount guards are portable (pure-python runtime patches).
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import pytest

from helpers import IS_WINDOWS, child_env, interpreter

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from backtest_worker.isolation import IsolationLimits, ProcessIsolation  # noqa: E402


def _spawn(script: str, limits: dict, env_extra: dict | None = None, cwd=None):
    iso = ProcessIsolation(IsolationLimits(**limits))
    env = child_env(**(env_extra or {}))
    proc = iso.spawn(
        [interpreter(), "-c", script],
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return iso, proc


def _read_result(proc) -> dict | None:
    out, _ = proc.communicate()
    for line in (out or "").splitlines():
        if line.startswith("RESULT:"):
            return json.loads(line[len("RESULT:"):])
    return None


def test_memory_limit_kills_the_child():
    script = "import time; buf=[bytearray(1024*1024) for _ in range(200)]; print('DONE'); time.sleep(30)"
    iso, proc = _spawn(script, {"memory_bytes": 32 * 1024 * 1024, "wall_seconds": 60})
    t0 = time.time()
    try:
        exit_code = iso.wait(proc, timeout=60)
    finally:
        iso.close()
    assert exit_code != 0, "child exceeding the memory limit must be killed"
    assert time.time() - t0 < 20, "memory kill must be fast, not a wall-clock timeout"
    iso.close()


def test_cpu_time_limit_kills_a_busy_child():
    script = (
        "import time; t=time.time(); x=0\n"
        "while time.time()-t < 30: x = x*1.0000001 + 1\n"
        "print('DONE')"
    )
    iso, proc = _spawn(script, {"cpu_seconds": 1.5, "wall_seconds": 90})
    t0 = time.time()
    try:
        exit_code = iso.wait(proc, timeout=90)
    finally:
        iso.close()
    assert exit_code != 0, "child exceeding the CPU limit must be killed"
    assert time.time() - t0 < 40, "CPU kill must be fast, not a wall-clock timeout"


def test_wall_clock_deadline_terminates_gracefully():
    script = (
        "from backtest_worker import guard\n"
        "guard.install()\n"
        "import time\n"
        "print('SLEEPING', flush=True); time.sleep(60); print('DONE')"
    )
    status_file = Path(tempfile.mkdtemp(prefix="status-")) / "wall-status.json"
    control_file = Path(tempfile.mkdtemp(prefix="control-")) / "stop"
    env = {"LAGRANGE_STATUS_FILE": str(status_file), "LAGRANGE_CONTROL_FILE": str(control_file)}
    iso, proc = _spawn(script, {"wall_seconds": 3}, env_extra=env)
    t0 = time.time()
    try:
        with pytest.raises(subprocess.TimeoutExpired):
            iso.wait(proc, timeout=3)
        term = iso.terminate(proc, grace_seconds=8)
    finally:
        iso.close()
    assert term.graceful, f"expected graceful termination, got {term}"
    assert time.time() - t0 < 15, "graceful termination must not wait the full sleep"
    assert status_file.exists(), "the child must write a structured status file"
    status = json.loads(status_file.read_text(encoding="utf-8"))
    assert status.get("state") == "TERMINATED", f"unexpected status: {status}"


@pytest.mark.skipif(not IS_WINDOWS, reason="one-node-per-process needs a Windows Job Object; POSIX cgroups/containers cover it in the Compose leg")
def test_one_node_per_process_blocks_grandchildren():
    script = (
        "import subprocess, sys\n"
        "try:\n"
        "    subprocess.run([sys.executable, '-c', 'pass'], check=True, timeout=10)\n"
        "    print('RESULT:' + __import__('json').dumps({'spawn': 'ok'}))\n"
        "except Exception as exc:\n"
        "    print('RESULT:' + __import__('json').dumps({'spawn': type(exc).__name__, 'detail': str(exc)}))\n"
    )
    iso, proc = _spawn(script, {"active_processes": 1, "wall_seconds": 60})
    try:
        result = _read_result(proc)
    finally:
        iso.close()
    assert result is not None, "child must report the spawn attempt"
    assert result.get("spawn") != "ok", "grandchild spawn must be denied inside the isolated node"


def test_network_disabled_blocks_external_but_allows_loopback():
    script = (
        "from backtest_worker import guard\n"
        "guard.install()\n"
        "import socket, json\n"
        "out = {}\n"
        "try:\n"
        "    socket.create_connection(('8.8.8.8', 443), timeout=1)\n"
        "    out['external'] = 'connected'\n"
        "except Exception as exc:\n"
        "    out['external'] = type(exc).__name__\n"
        "try:\n"
        "    socket.create_connection(('127.0.0.1', 1), timeout=0.5)\n"
        "    out['loopback'] = 'connected'\n"
        "except Exception as exc:\n"
        "    out['loopback'] = type(exc).__name__\n"
        "print('RESULT:' + json.dumps(out))"
    )
    iso, proc = _spawn(script, {"network_disabled": True})
    try:
        result = _read_result(proc)
    finally:
        iso.close()
    assert result is not None
    assert result["external"] == "NetworkDisabledError", f"external connect must be blocked, got {result}"
    assert result["loopback"] != "NetworkDisabledError", "loopback must stay allowed"


def test_readonly_mount_rejects_writes_in_child():
    dataset = tempfile.mkdtemp(prefix="ro-dataset-")
    probe = Path(dataset) / "bars.parquet"
    probe.write_text("immutable", encoding="utf-8")
    script = (
        "from backtest_worker import guard\n"
        "guard.install()\n"
        "import json\n"
        "try:\n"
        "    open(%r, 'w').write('tampered')\n"
        "    out = {'write': 'ok'}\n"
        "except Exception as exc:\n"
        "    out = {'write': type(exc).__name__, 'detail': str(exc)}\n"
        "print('RESULT:' + json.dumps(out))" % str(probe)
    )
    env = {"LAGRANGE_RO_MOUNTS": dataset}
    iso, proc = _spawn(script, {}, env_extra=env)
    try:
        result = _read_result(proc)
    finally:
        iso.close()
    assert result is not None
    assert result["write"] == "ReadOnlyMountError", f"write into a read-only mount must be rejected, got {result}"
    assert probe.read_text(encoding="utf-8") == "immutable", "the mounted file must stay untouched"


def test_graceful_termination_writes_structured_status():
    status_file = Path(tempfile.mkdtemp(prefix="status-")) / "child-status.json"
    control_file = Path(tempfile.mkdtemp(prefix="control-")) / "stop"
    script = (
        "from backtest_worker import guard\n"
        "guard.install()\n"
        "import time\n"
        "time.sleep(60)\n"
        "print('DONE')"
    )
    env = {"LAGRANGE_STATUS_FILE": str(status_file), "LAGRANGE_CONTROL_FILE": str(control_file)}
    iso, proc = _spawn(script, {"wall_seconds": 30}, env_extra=env)
    try:
        time.sleep(1.0)
        term = iso.terminate(proc, grace_seconds=8)
    finally:
        iso.close()
    assert term.graceful, f"expected graceful termination, got {term}"
    assert status_file.exists(), "the child must write a structured status file on termination"
    status = json.loads(status_file.read_text(encoding="utf-8"))
    assert status.get("state") == "TERMINATED", f"unexpected status: {status}"
    assert status.get("signal") == "CONTROL_FILE", f"unexpected signal: {status}"
    assert isinstance(status.get("pid"), int)


@pytest.mark.skipif(IS_WINDOWS, reason="real SIGTERM delivery is POSIX; Windows uses the control-file watchdog (console CTRL events are unreliable under redirected stdio)")
def test_sigterm_writes_structured_status():
    status_file = Path(tempfile.mkdtemp(prefix="status-")) / "sigterm-status.json"
    script = (
        "from backtest_worker import guard\n"
        "guard.install()\n"
        "import time\n"
        "time.sleep(60)\n"
        "print('DONE')"
    )
    env = {"LAGRANGE_STATUS_FILE": str(status_file)}
    iso, proc = _spawn(script, {"wall_seconds": 30}, env_extra=env)
    try:
        time.sleep(1.0)
        term = iso.terminate(proc, grace_seconds=8)
    finally:
        iso.close()
    assert term.graceful, f"expected graceful SIGTERM termination, got {term}"
    status = json.loads(status_file.read_text(encoding="utf-8"))
    assert status.get("state") == "TERMINATED", f"unexpected status: {status}"
    assert status.get("signal") == "SIGTERM", f"unexpected signal: {status}"
