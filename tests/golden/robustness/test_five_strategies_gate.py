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

import golden_lib as gl  # noqa: E402

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
GOLDEN_ID = "kr-etf-five-strategies-v2"
DATA_ID = "kr-etf-daily-phase0-v2"
GENERATOR_VERSION = "2.0.0"
CONFIG_ID = "golden-config-five-strategies-v2"
CODE_COMMIT = "9f319ca55679a801402da92df23a8c49291da645"
CODE_TREE = "fcd39ad2aa99804bc9354eb8812171e335e4b3d1"
GOLDEN_JSON = DIR / "golden.json"
GOLDEN_SET = DIR / "golden-set.json"
MANIFEST = DIR / "manifest.json"


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run_runner(strategy: str, out_dir: Path, timeout: int = 300) -> subprocess.CompletedProcess:
    committed = (DIR / "strategies" / strategy / "outputs" / "provenance.json")
    code_commit = "unknown"
    if committed.exists():
        code_commit = json.loads(committed.read_text(encoding="utf-8"))["code_commit"]
    cmd = [
        sys.executable, str(RUNNER), "--strategy-id", strategy, "--out-dir", str(out_dir),
        "--code-commit", code_commit,
    ]
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


def _load_committed_evidence() -> tuple[dict, dict, dict, dict[str, dict]]:
    golden = json.loads(GOLDEN_JSON.read_text(encoding="utf-8"))
    golden_set = json.loads(GOLDEN_SET.read_text(encoding="utf-8"))
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    provenance = {
        strategy: json.loads(
            (DIR / "strategies" / strategy / "outputs" / "provenance.json").read_text(
                encoding="utf-8"
            )
        )
        for strategy in STRATEGIES
    }
    return golden, golden_set, manifest, provenance


def _assert_robustness_evidence_consistent(
    golden: dict,
    golden_set: dict,
    manifest: dict,
    provenance: dict[str, dict],
    repo_root: Path = REPO_ROOT,
) -> None:
    """Cross-check the independently committed robustness evidence."""
    assert golden["golden_id"] == GOLDEN_ID, "golden identity is not the approved v2 ID"
    assert golden_set["golden_id"] == GOLDEN_ID, "golden-set identity mismatch"
    assert manifest["golden_id"] == GOLDEN_ID, "manifest golden identity mismatch"

    expected_data = {"id": DATA_ID, "version": GENERATOR_VERSION, "source": "synthetic"}
    assert golden["versions"]["data"] == expected_data, "golden data identity mismatch"
    assert golden_set["versions"]["data"] == expected_data, "golden-set data identity mismatch"
    assert manifest["versions"]["data"] == expected_data, "manifest data identity mismatch"

    assert golden["versions"]["config"]["id"] == CONFIG_ID, "golden config identity mismatch"
    assert golden_set["versions"]["config"]["id"] == CONFIG_ID, "golden-set config identity mismatch"
    assert manifest["versions"]["config"]["id"] == CONFIG_ID, "manifest config identity mismatch"
    expected_config_hash = gl.hash_bytes(gl.canonical_json_bytes(golden))
    assert (
        manifest["versions"]["config"]["hash"] == expected_config_hash
    ), "config hash does not match canonical golden config"

    code = manifest["versions"]["code"]
    assert code["commit"] == CODE_COMMIT, "manifest code commit is not the approved pin"
    assert code["tree"] == CODE_TREE, "manifest code tree is not the approved pin"
    for strategy, artifact in provenance.items():
        assert artifact["dataset_version"] == DATA_ID, f"{strategy} dataset_version mismatch"
        assert (
            artifact["data_generator_version"] == GENERATOR_VERSION
        ), f"{strategy} data_generator_version mismatch"
        assert artifact["code_commit"] == CODE_COMMIT, f"{strategy} code_commit mismatch"

    resolved_commit = subprocess.run(
        ["git", "rev-parse", "--verify", f"{CODE_COMMIT}^{{commit}}"],
        capture_output=True,
        text=True,
        cwd=repo_root,
    )
    assert resolved_commit.returncode == 0, f"manifest code commit does not exist: {CODE_COMMIT}"
    assert resolved_commit.stdout.strip() == CODE_COMMIT, "manifest code commit is not canonical"
    resolved_tree = subprocess.run(
        ["git", "rev-parse", "--verify", f"{CODE_COMMIT}^{{tree}}"],
        capture_output=True,
        text=True,
        cwd=repo_root,
    )
    assert resolved_tree.returncode == 0, f"cannot resolve code tree for commit: {CODE_COMMIT}"
    assert resolved_tree.stdout.strip() == CODE_TREE, "manifest code tree does not match the commit"


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


def test_runner_writes_platform_independent_lf(tmp_path: Path) -> None:
    out_dir = tmp_path / "run"
    proc = run_runner(STRATEGIES[0], out_dir)
    assert proc.returncode == 0, proc.stdout + proc.stderr
    generated = list((out_dir / "outputs").glob("*.json")) + [out_dir / "summary.json"]
    for path in generated:
        assert b"\r\n" not in path.read_bytes(), f"{path.name} must use LF on every platform"


def test_committed_golden_set_verifies() -> None:
    assert GOLDEN_SET.exists(), "committed golden-set.json missing"
    failures = verify_golden_set(GOLDEN_SET, DIR)
    assert not failures, "committed golden-set.json disagrees with committed artifacts:\n" + "\n".join(failures)
    manifest = json.loads(GOLDEN_SET.read_text(encoding="utf-8"))
    assert manifest["golden_id"] == GOLDEN_ID
    assert manifest["versions"]["engine"]["name"] == "lagrange-golden-sim"


@pytest.mark.parametrize("document", ["golden", "golden-set", "manifest"])
def test_robustness_evidence_rejects_stale_golden_identity(document: str) -> None:
    golden, golden_set, manifest, provenance = _load_committed_evidence()
    {"golden": golden, "golden-set": golden_set, "manifest": manifest}[document][
        "golden_id"
    ] += "-stale"
    with pytest.raises(AssertionError, match="identity"):
        _assert_robustness_evidence_consistent(golden, golden_set, manifest, provenance)


def test_robustness_evidence_rejects_mutated_config_hash() -> None:
    golden, golden_set, manifest, provenance = _load_committed_evidence()
    manifest["versions"]["config"]["hash"] = "sha256:" + "0" * 64
    with pytest.raises(AssertionError, match="config hash"):
        _assert_robustness_evidence_consistent(golden, golden_set, manifest, provenance)


@pytest.mark.parametrize("field", ["dataset_version", "data_generator_version", "code_commit"])
def test_robustness_evidence_rejects_stale_provenance(field: str) -> None:
    golden, golden_set, manifest, provenance = _load_committed_evidence()
    provenance[STRATEGIES[0]][field] += "-stale"
    with pytest.raises(AssertionError, match=field):
        _assert_robustness_evidence_consistent(golden, golden_set, manifest, provenance)


def test_robustness_evidence_rejects_wrong_commit_pin() -> None:
    golden, golden_set, manifest, provenance = _load_committed_evidence()
    manifest["versions"]["code"]["commit"] = "0" * 40
    with pytest.raises(AssertionError, match="code commit"):
        _assert_robustness_evidence_consistent(golden, golden_set, manifest, provenance)


def test_robustness_evidence_rejects_wrong_tree_pin() -> None:
    golden, golden_set, manifest, provenance = _load_committed_evidence()
    manifest["versions"]["code"]["tree"] = "0" * 40
    with pytest.raises(AssertionError, match="code tree"):
        _assert_robustness_evidence_consistent(golden, golden_set, manifest, provenance)


def test_committed_manifest_verifies() -> None:
    assert MANIFEST.exists(), "committed manifest.json missing"
    verify = verify_todo6_manifest(MANIFEST)
    assert verify.returncode == 0, verify.stdout + verify.stderr
    assert "VERDICT: PASS" in verify.stdout
    golden, golden_set, manifest, provenance = _load_committed_evidence()
    _assert_robustness_evidence_consistent(golden, golden_set, manifest, provenance)


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
