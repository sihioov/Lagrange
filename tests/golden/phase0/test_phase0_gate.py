"""Todo 14 RED gate tests: Phase 0 NautilusTrader next-open golden gate.

The five gates below are the acceptance contract for the Phase 0 proof
(plan Todo 14, docs requirements AT-02/AT-03, design ADR-004/005/007):

1. `test_two_fresh_process_runs_produce_identical_hashes` - the SAME config
   run twice, each in its own fresh Python process, produces byte-identical
   recommendation/order/fill/equity/fee/provenance artifacts and the golden
   manifest verify passes (AT-03: identical outputs on identical input).
2. `test_at02_fill_price_is_next_raw_open_plus_slippage` - every fill price
   equals the NEXT KRX session raw open plus the configured slippage (buy) /
   minus it (sell), at the open instant; no signal-day close and no
   execution-day high/low/close is ever used (AT-02).
3. `test_future_field_probe_fails_the_gate` - a deliberate probe that touches
   `SessionOpenEvent.high/low/close` fails the gate run with a typed
   violation (future-field barrier holds at runtime).
4. `test_same_process_second_run_is_rejected` - two golden runs in one
   process are rejected (ADR-005 per-job process isolation).
5. `test_version_drift_fails_the_golden_gate` - an altered engine/strategy
   version produces a provenance hash mismatch and the manifest verify fails
   with a field-level diff (stale-state gate).
6. `test_committed_phase0_manifest_verifies` - the committed golden manifest
   + committed outputs verify with exit 0 (Todo 6 pattern; evidence gate).
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import subprocess
import sys
from decimal import Decimal
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPTS_GOLDEN = REPO_ROOT / "scripts" / "golden"
PHASE0_DIR = Path(__file__).resolve().parent
RUNNER = PHASE0_DIR / "runner.py"
GOLDEN_JSON = PHASE0_DIR / "golden.json"
MANIFEST = PHASE0_DIR / "manifest.json"
OUTPUTS_DIR = PHASE0_DIR / "outputs"
FIXTURE_BARS = REPO_ROOT / "tests" / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"

if str(SCRIPTS_GOLDEN) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_GOLDEN))

ARTIFACTS = [
    "recommendation.json",
    "orders.json",
    "fills.json",
    "equity.json",
    "fees.json",
    "metrics.json",
    "provenance.json",
]

GOLDEN_ID = "kr-etf-phase0-next-open-v1"


# --------------------------------------------------------------------------- #
# helpers
# --------------------------------------------------------------------------- #

def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run_runner(out_dir: Path, *extra_args: str, timeout: int = 600) -> subprocess.CompletedProcess:
    """Run the isolated phase0 runner in a FRESH process.

    The run is hermetic: `--code-commit` is pinned to the committed golden
    provenance so a run reproduces the approved artifacts byte-for-byte
    (same design as golden.py `--code-override`).
    """
    committed = json.loads((OUTPUTS_DIR / "provenance.json").read_text(encoding="utf-8"))
    cmd = [
        sys.executable, str(RUNNER), "--out-dir", str(out_dir),
        "--code-commit", committed["code_commit"], *extra_args,
    ]
    return subprocess.run(
        cmd, capture_output=True, text=True, encoding="utf-8", errors="replace",
        cwd=str(REPO_ROOT), timeout=timeout,
    )


def artifact_hashes(out_dir: Path) -> dict[str, str]:
    return {name: _sha256_bytes((out_dir / name).read_bytes()) for name in ARTIFACTS}


def load_artifact(out_dir: Path, name: str) -> dict:
    return json.loads((out_dir / name).read_text(encoding="utf-8"))


def verify_manifest(manifest: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPTS_GOLDEN / "golden.py"), "verify", str(manifest)],
        capture_output=True, text=True, cwd=str(REPO_ROOT),
    )


def materialize_golden_tree(tmp: Path) -> Path:
    """Copy manifest + outputs + fixtures into `tmp` preserving relative paths.

    The committed manifest references outputs/ and ../fixtures/... relative to
    its own location; drifted-run tests must verify against a COPY so the
    committed golden outputs are never mutated by failure probes.
    """
    tree = tmp / "golden" / "phase0"
    (tree / "outputs").mkdir(parents=True)
    shutil.copy2(MANIFEST, tree / "manifest.json")
    for name in ARTIFACTS:
        shutil.copy2(OUTPUTS_DIR / name, tree / "outputs" / name)
    fixtures_src = REPO_ROOT / "tests" / "fixtures"
    shutil.copytree(fixtures_src, tmp / "fixtures", dirs_exist_ok=True)
    return tree


def _load_synth() -> object:
    spec = importlib.util.spec_from_file_location("phase0_synth", PHASE0_DIR / "synth_data.py")
    synth = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(synth)
    return synth


def _expected_open_by_session() -> dict[tuple[str, str], int]:
    """Independent AT-02 expectation: raw opens from the synthetic generator."""
    return {(bar["instrument"], bar["date"]): bar["open"] for bar in _load_synth().generate_bars()}


def test_decimal_krw_converts_to_exact_raw4() -> None:
    synth = _load_synth()
    assert synth.decimal_to_raw4(Decimal("10150.0000")) == 101_500_000
    with pytest.raises(ValueError, match="scale 4"):
        synth.decimal_to_raw4(Decimal("10150.00001"))
    with pytest.raises(ValueError, match="finite"):
        synth.decimal_to_raw4(Decimal("NaN"))


# --------------------------------------------------------------------------- #
# gate 1: deterministic repeatability across fresh processes
# --------------------------------------------------------------------------- #

def test_two_fresh_process_runs_produce_identical_hashes(tmp_path: Path) -> None:
    out_a = tmp_path / "run_a"
    out_b = tmp_path / "run_b"
    proc_a = run_runner(out_a)
    assert proc_a.returncode == 0, proc_a.stdout + proc_a.stderr
    proc_b = run_runner(out_b)
    assert proc_b.returncode == 0, proc_b.stdout + proc_b.stderr

    summary_a = json.loads((out_a / "summary.json").read_text(encoding="utf-8"))
    summary_b = json.loads((out_b / "summary.json").read_text(encoding="utf-8"))
    # Each run must have been its own fresh process (ADR-005 isolation proof).
    assert summary_a["process"]["pid"] != summary_b["process"]["pid"]

    hashes_a = artifact_hashes(out_a)
    hashes_b = artifact_hashes(out_b)
    # SHOW the equality, never claim it: every artifact hash must match.
    for name in ARTIFACTS:
        assert hashes_a[name] == hashes_b[name], (
            f"artifact {name} differs between fresh-process runs:\n"
            f"  run_a {hashes_a[name]}\n  run_b {hashes_b[name]}"
        )
    assert summary_a["artifact_hashes"] == hashes_a
    assert summary_b["artifact_hashes"] == hashes_b

    # The committed golden outputs are regenerated from run_a (deterministic)
    # and the committed manifest must verify (Todo 6 golden gate).
    for name in ARTIFACTS:
        shutil.copy2(out_a / name, OUTPUTS_DIR / name)
    verify = verify_manifest(MANIFEST)
    assert verify.returncode == 0, verify.stdout + verify.stderr
    assert "VERDICT: PASS" in verify.stdout


# --------------------------------------------------------------------------- #
# gate 2: AT-02 fill price == next KRX session raw open +/- configured slippage
# --------------------------------------------------------------------------- #

def test_at02_fill_price_is_next_raw_open_plus_slippage(tmp_path: Path) -> None:
    out = tmp_path / "run_at02"
    proc = run_runner(out)
    assert proc.returncode == 0, proc.stdout + proc.stderr

    fills = load_artifact(out, "fills.json")
    slippage_bps = int(fills["fill_model"]["slippage_bps"])
    assert slippage_bps > 0, "phase0 run must configure non-zero slippage (non-vacuous AT-02)"

    opens = _expected_open_by_session()
    for fill in fills["fills"]:
        instrument = fill["instrument"]
        date = fill["date"]
        assert fill["source"] == "NEXT_SESSION_OPEN"
        assert (instrument, date) in opens, f"fill references unknown session {instrument} {date}"
        raw_open = opens[(instrument, date)]  # integer KRW at scale 0 (fixture contract)
        side = fill["side"]
        # AT-02: buy fills at raw open + slippage; sell fills at raw open - slippage.
        # The quantization is the generator contract (single source of truth).
        expected = _load_synth().slipped_open_raw(raw_open * 10_000, side, slippage_bps)
        assert fill["price_raw"] == expected, (
            f"AT-02 violation: {side} {instrument} on {date}: "
            f"fill {fill['price_raw']} != next raw open {raw_open} +/- {slippage_bps}bps "
            f"(expected raw {expected})"
        )
        # Fill must occur at the open instant, and must never touch the
        # signal-day close or the execution-day high/low/close.
        assert fill["never_uses"] == ["signal_day_close", "execution_day_high", "execution_day_low", "execution_day_close"]
        assert fill["barrier_held"], f"future-field barrier violated for {instrument} {date}"

    # Every order from orders.json is represented in fills (no phantom orders).
    orders = load_artifact(out, "orders.json")
    filled_order_ids = {f["order_id"] for f in fills["fills"]}
    for order in orders["orders"]:
        assert order["state"] == "FILLED"
        assert order["order_id"] in filled_order_ids


# --------------------------------------------------------------------------- #
# gate 3: deliberate future-field probe fails the gate
# --------------------------------------------------------------------------- #

def test_future_field_probe_fails_the_gate(tmp_path: Path) -> None:
    out = tmp_path / "run_probe"
    proc = run_runner(out, "--probe-future-fields")
    assert proc.returncode != 0, (
        "future-field probe must fail the gate but exited 0; "
        + proc.stdout + proc.stderr
    )
    combined = proc.stdout + proc.stderr
    assert "FUTURE_FIELD_VIOLATION" in combined, combined


# --------------------------------------------------------------------------- #
# gate 4: same-process second run is rejected (ADR-005)
# --------------------------------------------------------------------------- #

def test_same_process_second_run_is_rejected() -> None:
    spec = importlib.util.spec_from_file_location("phase0_runner", RUNNER)
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    # First call in a fresh process context: allowed.
    runner.assert_fresh_process()
    # Second call in the SAME process: must be rejected.
    with pytest.raises(runner.Phase0IsolationError):
        runner.assert_fresh_process()


# --------------------------------------------------------------------------- #
# gate 5: version drift -> golden hash mismatch -> gate failure
# --------------------------------------------------------------------------- #

def test_strategy_version_drift_fails_golden_gate(tmp_path: Path) -> None:
    tree = materialize_golden_tree(tmp_path)
    proc = run_runner(tree / "outputs", "--strategy-version", "9.9.9-drift")
    assert proc.returncode == 0, proc.stdout + proc.stderr
    provenance = load_artifact(tree / "outputs", "provenance.json")
    assert provenance["strategy_version"] == "9.9.9-drift"
    verify = verify_manifest(tree / "manifest.json")
    assert verify.returncode != 0, "drifted strategy version must fail the gate"
    assert "strategy_version: 1.0.0 -> 9.9.9-drift" in verify.stdout, verify.stdout


def test_engine_version_drift_fails_golden_gate(tmp_path: Path) -> None:
    tree = materialize_golden_tree(tmp_path)
    proc = run_runner(tree / "outputs", "--engine-version", "9.9.9-drift")
    assert proc.returncode == 0, proc.stdout + proc.stderr
    provenance = load_artifact(tree / "outputs", "provenance.json")
    assert provenance["engine_version"] == "9.9.9-drift"
    verify = verify_manifest(tree / "manifest.json")
    assert verify.returncode != 0, "drifted engine version must fail the gate"
    assert "engine_version: 1.231.0 -> 9.9.9-drift" in verify.stdout, verify.stdout


# --------------------------------------------------------------------------- #
# gate 6: committed golden manifest verifies (evidence gate)
# --------------------------------------------------------------------------- #

def test_committed_phase0_manifest_verifies() -> None:
    assert MANIFEST.exists(), "committed phase0 manifest missing"
    assert OUTPUTS_DIR.exists(), "committed phase0 outputs missing"
    verify = verify_manifest(MANIFEST)
    assert verify.returncode == 0, verify.stdout + verify.stderr
    assert "VERDICT: PASS" in verify.stdout
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    assert manifest["golden_id"] == GOLDEN_ID
    assert manifest["versions"]["engine"]["version"] == "1.231.0"
    assert manifest["versions"]["timezone"] == "Asia/Seoul"
