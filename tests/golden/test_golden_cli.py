"""CLI-level tests for scripts/golden/golden.py (generate / verify / hash / evidence).

Runs the real CLI in subprocesses against a hermetic golden tree; the committed
manifest at tests/golden/manifest.json is verified by
test_committed_manifest_verifies (guarded until the manifest exists).
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

from conftest import GOLDEN_PY

REPO_ROOT = Path(__file__).resolve().parents[2]


def _run_cli(golden_tree: Path, *args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
    cmd = [sys.executable, str(GOLDEN_PY), *args]
    return subprocess.run(cmd, capture_output=True, text=True, cwd=cwd or golden_tree)


def _generate(tmp_manifest: Path, golden_tree: Path) -> subprocess.CompletedProcess:
    config = golden_tree / "golden" / "golden.json"
    return _run_cli(golden_tree, "generate", str(config), "-o", str(tmp_manifest),
                    "--code-override", "0" * 40)


# --------------------------------------------------------------------------- #
# hash subcommand
# --------------------------------------------------------------------------- #

def test_cli_hash_json(golden_tree: Path) -> None:
    bars = golden_tree / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"
    proc = _run_cli(golden_tree, "hash", str(bars))
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.strip().startswith("sha256:")
    assert "bars.json" in proc.stdout


def test_cli_hash_same_for_key_order_variants(golden_tree: Path, tmp_path: Path) -> None:
    src = golden_tree / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"
    data = json.loads(src.read_text(encoding="utf-8"))
    reordered = tmp_path / "reordered.json"
    reordered.write_text(json.dumps(dict(reversed(list(data.items())))), encoding="utf-8")
    h1 = _run_cli(golden_tree, "hash", str(src)).stdout.strip()
    h2 = _run_cli(golden_tree, "hash", str(reordered)).stdout.strip()
    assert h1 == h2


# --------------------------------------------------------------------------- #
# generate subcommand: deterministic output
# --------------------------------------------------------------------------- #

def test_cli_generate_deterministic_across_two_runs(golden_tree: Path, tmp_path: Path) -> None:
    m1 = tmp_path / "m1.json"
    m2 = tmp_path / "m2.json"
    p1 = _generate(m1, golden_tree)
    p2 = _generate(m2, golden_tree)
    assert p1.returncode == 0, p1.stderr
    assert p2.returncode == 0, p2.stderr
    assert m1.read_bytes() == m2.read_bytes()
    manifest = json.loads(m1.read_text(encoding="utf-8"))
    assert manifest["golden_id"] == "kr-etf-2020-01-31-test"
    assert set(manifest["versions"]) == {"data", "strategy", "engine", "code", "config", "seed", "timezone"}


# --------------------------------------------------------------------------- #
# verify subcommand: exit codes + field-level diffs
# --------------------------------------------------------------------------- #

def test_cli_verify_ok_on_unchanged(golden_tree: Path, tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    assert _generate(manifest, golden_tree).returncode == 0
    proc = _run_cli(golden_tree, "verify", str(manifest))
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "VERDICT: PASS" in proc.stdout


def test_cli_verify_fails_with_field_diff_after_fill_price_mutation(golden_tree: Path, tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    assert _generate(manifest, golden_tree).returncode == 0

    fills = golden_tree / "golden" / "outputs" / "2020-01-31" / "fills.json"
    data = json.loads(fills.read_text(encoding="utf-8"))
    data["fills"][0]["price"] = "10400.00"
    fills.write_text(json.dumps(data), encoding="utf-8")

    proc = _run_cli(golden_tree, "verify", str(manifest))
    assert proc.returncode != 0, "verify must exit nonzero on drift"
    out = proc.stdout
    assert "VERDICT: FAIL" in out
    assert "fills[0].price" in out
    assert "10300.00 -> 10400.00" in out
    assert "fill" in out  # category named


def test_cli_verify_fails_on_fixture_bars_close_mutation(golden_tree: Path, tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    assert _generate(manifest, golden_tree).returncode == 0

    bars = golden_tree / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"
    data = json.loads(bars.read_text(encoding="utf-8"))
    data["bars"][0]["close"] = 1
    bars.write_text(json.dumps(data), encoding="utf-8")

    proc = _run_cli(golden_tree, "verify", str(manifest))
    assert proc.returncode != 0
    assert "bars[0].close" in proc.stdout


def test_cli_verify_writes_machine_report(golden_tree: Path, tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    report = tmp_path / "report.json"
    assert _generate(manifest, golden_tree).returncode == 0
    proc = _run_cli(golden_tree, "verify", str(manifest), "--report", str(report))
    assert proc.returncode == 0
    rep = json.loads(report.read_text(encoding="utf-8"))
    assert rep["ok"] is True
    assert rep["golden_id"] == "kr-etf-2020-01-31-test"
    categories = {a["category"] for a in rep["artifacts"]}
    assert categories == {"recommendation", "order", "fill", "equity", "fee", "metric", "provenance"}
    assert all(a["ok"] for a in rep["artifacts"])


# --------------------------------------------------------------------------- #
# evidence subcommand
# --------------------------------------------------------------------------- #

def test_cli_evidence_writes_sanitized_report(golden_tree: Path, tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    evidence = tmp_path / "evidence.txt"
    assert _generate(manifest, golden_tree).returncode == 0
    proc = _run_cli(golden_tree, "evidence", str(manifest), "-o", str(evidence))
    assert proc.returncode == 0, proc.stderr
    text = evidence.read_text(encoding="utf-8")
    low = text.lower()
    for marker in ("secret", "password", "token", "api_key", "begin private key"):
        assert marker not in low, f"evidence leaked {marker}"
    assert "kr-etf-2020-01-31-test" in text


# --------------------------------------------------------------------------- #
# Committed manifest (runs once tests/golden/manifest.json is generated)
# --------------------------------------------------------------------------- #

def test_committed_manifest_verifies() -> None:
    manifest = REPO_ROOT / "tests" / "golden" / "manifest.json"
    if not manifest.exists():
        pytest_skip = __import__("pytest").skip
        pytest_skip("committed manifest not generated yet (Todo 6 increment 4)")
    proc = subprocess.run(
        [sys.executable, str(GOLDEN_PY), "verify", str(manifest)],
        capture_output=True, text=True, cwd=REPO_ROOT,
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "VERDICT: PASS" in proc.stdout
