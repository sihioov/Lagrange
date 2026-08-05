"""Wave-gate evidence test: an unapproved golden delta fails the gate.

Todo 6/14 contract: the committed golden manifest pins the approved
recommendation/order/fill/equity/fee/provenance artifacts.  Any unapproved
delta (e.g. a hand-edited fill price) must fail `golden.py verify` with a
field-level diff naming the exact mutated path - never a silent pass.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPTS_GOLDEN = REPO_ROOT / "scripts" / "golden"
PHASE0_DIR = Path(__file__).resolve().parent
MANIFEST = PHASE0_DIR / "manifest.json"
OUTPUTS_DIR = PHASE0_DIR / "outputs"

ARTIFACTS = [
    "recommendation.json",
    "orders.json",
    "fills.json",
    "equity.json",
    "fees.json",
    "metrics.json",
    "provenance.json",
]


def _materialize(tmp: Path) -> Path:
    """Copy manifest + outputs + fixtures preserving the committed layout."""
    tree = tmp / "golden" / "phase0"
    (tree / "outputs").mkdir(parents=True)
    shutil.copy2(MANIFEST, tree / "manifest.json")
    for name in ARTIFACTS:
        shutil.copy2(OUTPUTS_DIR / name, tree / "outputs" / name)
    shutil.copytree(REPO_ROOT / "tests" / "fixtures", tmp / "fixtures", dirs_exist_ok=True)
    return tree


def test_unapproved_fill_delta_fails_with_exact_diff(tmp_path: Path) -> None:
    tree = _materialize(tmp_path)
    fills_path = tree / "outputs" / "fills.json"
    fills = json.loads(fills_path.read_text(encoding="utf-8"))
    original = fills["fills"][0]["price_raw"]
    fills["fills"][0]["price_raw"] = original + 1  # unapproved delta
    fills_path.write_text(json.dumps(fills, indent=2, sort_keys=True), encoding="utf-8")

    verify = subprocess.run(
        [sys.executable, str(SCRIPTS_GOLDEN / "golden.py"), "verify", str(tree / "manifest.json")],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
        cwd=str(REPO_ROOT),
    )
    assert verify.returncode != 0, "unapproved golden delta must fail the gate"
    # The diff must name the exact mutated path with old -> new values.
    assert f"fills[0].price_raw: {original} -> {original + 1}" in verify.stdout, verify.stdout
    assert "VERDICT: FAIL" in verify.stdout
