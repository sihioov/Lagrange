"""Todo 21 RED gate tests: five-strategy golden artifacts (§12.2).

The five baseline strategies (plan Todo 17) each get the §12.2 golden
artifact set — recommendation/orders/fills/equity/fees/metrics/provenance —
produced by a deterministic target->next-open-execution simulation
(`runner.py`), one strategy per fresh process (phase0 convention). Gates:

1. `test_five_strategies_run_deterministically` - running every strategy
   twice, each in its own fresh process, yields byte-identical artifacts
   (AT-03 style determinism across processes);
2. `test_committed_golden_set_verifies` - the committed `golden-set.json`
   (the Rust core-gate GoldenSet shape) matches the committed artifacts
   byte-for-byte;
3. `test_committed_manifest_verifies` - the Todo-6 `manifest.json` verifies
   via `scripts/golden verify` with VERDICT: PASS;
4. `test_unapproved_golden_delta_fails` - mutating one artifact in a COPY
   fails the golden-set verification with the quoted expected-vs-actual diff;
5. `test_all_strategies_have_artifacts` - every one of the five strategies
   ships all seven canonical artifacts with non-vacuous fills.
"""
from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
DIR = Path(__file__).resolve().parent
RUNNER = DIR / "runner.py"
SCRIPTS_GOLDEN = REPO_ROOT / "scripts" / "golden"

if str(SCRIPTS_GOLDEN) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_GOLDEN))

STRATEGIES = [
    "buy_and_hold",
    "trend_following",
    "relative_momentum",
    "dual_momentum",
    "inverse_volatility",
]
ARTIFACTS = [
    "recommendation",
    "orders",
    "fills",
    "equity",
    "fees",
    "metrics",
    "provenance",
]
GOLDEN_ID = "kr-etf-five-strategies-v1"
GOLDEN_SET = DIR / "golden-set.json"
MANIFEST = DIR / "manifest.json"


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run_runner(strategy: str, out_dir: Path, timeout: int = 300) -> subprocess.CompletedProcess:
    cmd = [sys.executable, str(RUNNER), "--strategy-id", strategy, "--out-dir", str(out_dir)]
    return subprocess.run(
        cmd, capture_output=True, text=True, encoding="utf-8", errors="replace",
        cwd=str(REPO_ROOT), timeout=timeout,
    )


def strategy_outputs(out_root: Path, strategy: str) -> dict[str, bytes]:
    base = out_root / strategy / "outputs"
    return {name: (base / f"{name}.json").read_bytes() for name in ARTIFACTS}


def regenerate_all(out_root: Path) -> None:
    """Runs all five strategies in fresh processes into `out_root`."""
    for strategy in STRATEGIES:
        proc = run_runner(strategy, out_root / strategy)
        assert proc.returncode == 0, f"{strategy}: {proc.stdout + proc.stderr}"
        summary = json.loads((out_root / strategy / "summary.json").read_text(encoding="utf-8"))
        assert summary["status"] == "PASS", f"{strategy}: {summary}"
        hashes = {name: _sha256_bytes((out_root / strategy / "outputs" / f"{name}.json").read_bytes())
                  for name in ARTIFACTS}
        assert summary["artifact_hashes"] == hashes, f"{strategy}: summary hashes disagree"


def verify_golden_set(golden_set_path: Path, base_dir: Path) -> list[str]:
    """Checks every artifact hash in the Rust GoldenSet shape. Returns failures."""
    manifest = json.loads(golden_set_path.read_text(encoding="utf-8"))
    failures = []
    for entry in manifest["artifacts"]:
        path = base_dir / entry["path"]
        actual = f"sha256:{_sha256_bytes(path.read_bytes())}"
        if actual != entry["sha256"]:
            failures.append(
                f"{entry['id']} at {entry['path']}: expected {entry['sha256']} got {actual}"
            )
    return failures


def verify_todo6_manifest(manifest: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPTS_GOLDEN / "golden.py"), "verify", str(manifest)],
        capture_output=True, text=True, cwd=str(REPO_ROOT),
    )


def materialize_golden_tree(tmp: Path) -> Path:
    """Copies the committed golden tree into `tmp` so failure probes never
    mutate committed artifacts."""
    tree = tmp / "golden" / "robustness"
    shutil.copytree(DIR, tree, dirs_exist_ok=True)
    return tree


def test_five_strategies_run_deterministically(tmp_path: Path) -> None:
    first = tmp_path / "first"
    second = tmp_path / "second"
    regenerate_all(first)
    regenerate_all(second)
    for strategy in STRATEGIES:
        a = strategy_outputs(first, strategy)
        b = strategy_outputs(second, strategy)
        for name in ARTIFACTS:
            assert a[name] == b[name], (
                f"{strategy}/{name} differs between fresh-process runs:\n"
                f"  first {_sha256_bytes(a[name])}\n  second {_sha256_bytes(b[name])}"
            )
        summary_a = json.loads((first / strategy / "summary.json").read_text(encoding="utf-8"))
        summary_b = json.loads((second / strategy / "summary.json").read_text(encoding="utf-8"))
        # Each run was its own fresh process.
        assert summary_a["process"]["pid"] != summary_b["process"]["pid"]
        # Non-vacuous: every strategy produces at least one fill.
        fills = json.loads(a["fills"])
        assert len(fills["fills"]) > 0, f"{strategy} produced zero fills (vacuous golden)"


def test_committed_golden_set_verifies() -> None:
    assert GOLDEN_SET.exists(), "committed golden-set.json missing"
    failures = verify_golden_set(GOLDEN_SET, DIR)
    assert not failures, "committed golden-set.json disagrees with committed artifacts:\n" + "\n".join(failures)
    manifest = json.loads(GOLDEN_SET.read_text(encoding="utf-8"))
    assert manifest["golden_id"] == GOLDEN_ID
    assert manifest["versions"]["engine"]["name"] == "lagrange-golden-sim"


def test_committed_manifest_verifies() -> None:
    assert MANIFEST.exists(), "committed manifest.json missing"
    verify = verify_todo6_manifest(MANIFEST)
    assert verify.returncode == 0, verify.stdout + verify.stderr
    assert "VERDICT: PASS" in verify.stdout


def test_unapproved_golden_delta_fails(tmp_path: Path) -> None:
    tree = materialize_golden_tree(tmp_path)
    target = tree / "strategies" / "buy_and_hold" / "outputs" / "fills.json"
    original = target.read_bytes()
    target.write_bytes(original + b" ")
    failures = verify_golden_set(tree / "golden-set.json", tree)
    assert failures, "mutated artifact must fail golden-set verification"
    assert "buy_and_hold/fills" in failures[0], failures[0]
    assert "expected" in failures[0] and "got" in failures[0], failures[0]
    # The committed tree is untouched.
    assert (DIR / "strategies" / "buy_and_hold" / "outputs" / "fills.json").read_bytes() == original


def test_all_strategies_have_artifacts() -> None:
    for strategy in STRATEGIES:
        out_dir = DIR / "strategies" / strategy / "outputs"
        for name in ARTIFACTS:
            path = out_dir / f"{name}.json"
            assert path.exists(), f"{strategy}/{name} missing"
            data = json.loads(path.read_text(encoding="utf-8"))
            assert isinstance(data, dict), f"{strategy}/{name} must be a JSON object"
        fills = json.loads((out_dir / "fills.json").read_text(encoding="utf-8"))
        assert len(fills["fills"]) > 0, f"{strategy} fills are vacuous"
