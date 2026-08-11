# GitHub Actions CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add free public-repository GitHub Actions verification that generates the ignored Phase 0 dataset for PR tests, runs PostgreSQL-backed workspace gates, and runs the full Compose smoke once per `main` push without any scheduled execution.

**Architecture:** A small dependency-light Python materializer shares the existing Phase 0 Parquet writer with the golden runner. A Python contract suite validates both the generated dataset and the semantic/security shape of two SHA-pinned workflows. Rust formatting, Clippy, workspace tests, and Compose smoke run in separate ephemeral `ubuntu-latest` jobs to stay within the 14 GB standard-runner disk.

**Tech Stack:** GitHub Actions YAML, Python 3.12, PyYAML 6.0.2, PyArrow 25.0.0, Rust 1.97.1/Cargo, Docker Compose, PostgreSQL 18.4.

---

### Task 1: Add RED workflow-contract tests

**Files:**
- Create: `scripts/ci/test_ci_contract.py`
- Test: `scripts/ci/test_ci_contract.py`

- [ ] **Step 1: Create a contract test that requires both workflows**

Create a `unittest` module that loads YAML with `yaml.BaseLoader`, asserts
`.github/workflows/ci.yml` and `.github/workflows/research-smoke.yml` exist,
and uses helpers with these exact contracts:

```python
from __future__ import annotations

import re
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"
SHA_ACTION = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


def load_workflow(name: str) -> tuple[Path, dict]:
    path = WORKFLOWS / name
    if not path.is_file():
        raise AssertionError(f"missing workflow: {path.relative_to(ROOT)}")
    parsed = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
    if not isinstance(parsed, dict):
        raise AssertionError(f"workflow is not a mapping: {name}")
    return path, parsed


def action_uses(workflow: dict) -> list[str]:
    return [
        step["uses"]
        for job in workflow.get("jobs", {}).values()
        for step in job.get("steps", [])
        if "uses" in step
    ]
```

The tests must assert:

```python
class WorkflowContractTests(unittest.TestCase):
    def test_triggers_have_no_schedule(self) -> None:
        _, ci = load_workflow("ci.yml")
        _, smoke = load_workflow("research-smoke.yml")
        self.assertEqual(set(ci["on"]), {"pull_request", "push", "workflow_dispatch"})
        self.assertEqual(ci["on"]["pull_request"]["branches"], ["main"])
        self.assertEqual(ci["on"]["push"]["branches"], ["main"])
        self.assertEqual(set(smoke["on"]), {"push", "workflow_dispatch"})
        self.assertEqual(smoke["on"]["push"]["branches"], ["main"])

    def test_actions_permissions_timeouts_and_storage_policy(self) -> None:
        for name in ("ci.yml", "research-smoke.yml"):
            path, workflow = load_workflow(name)
            self.assertEqual(workflow["permissions"], {"contents": "read"})
            for use in action_uses(workflow):
                self.assertRegex(use, SHA_ACTION)
            for job in workflow["jobs"].values():
                self.assertEqual(job["runs-on"], "ubuntu-latest")
                self.assertIn("timeout-minutes", job)
                for step in job.get("steps", []):
                    if step.get("uses", "").startswith("actions/checkout@"):
                        self.assertEqual(step.get("with", {}).get("persist-credentials"), "false")
            text = path.read_text(encoding="utf-8").lower()
            self.assertNotIn("actions/upload-artifact", text)
            self.assertNotIn("actions/cache", text)
            self.assertNotIn("target/", text)

    def test_ci_runs_required_gates_and_disposable_data(self) -> None:
        path, ci = load_workflow("ci.yml")
        self.assertEqual(set(ci["jobs"]), {"policy", "format", "clippy", "workspace-tests", "required"})
        text = path.read_text(encoding="utf-8")
        self.assertIn("scripts/ci/prepare_phase0.py --root data/phase0", text)
        self.assertIn("deploy/qa/qa-db.compose.yml up -d --wait", text)
        self.assertIn("cargo test --workspace --locked --no-fail-fast", text)
        self.assertIn("cargo clippy --workspace --all-targets --all-features --locked -- -D warnings", text)
        self.assertIn("cargo fmt --all -- --check", text)
        self.assertIn("CARGO_INCREMENTAL: '0'", text)
        qa_compose = (ROOT / "deploy" / "qa" / "qa-db.compose.yml").read_text(encoding="utf-8")
        self.assertRegex(qa_compose, r"image:\s+postgres@sha256:[0-9a-f]{64}")

    def test_smoke_runs_only_the_existing_functional_script(self) -> None:
        path, smoke = load_workflow("research-smoke.yml")
        self.assertEqual(set(smoke["jobs"]), {"research-smoke"})
        text = path.read_text(encoding="utf-8")
        self.assertIn("bash scripts/qa/research-worker-smoke.sh", text)
        self.assertNotIn("--static-only", text)
```

- [ ] **Step 2: Run the test and observe the expected RED**

Run:

```bash
uv run --with PyYAML==6.0.2 python -m unittest scripts.ci.test_ci_contract -v
```

Expected: FAIL with `missing workflow: .github/workflows/ci.yml`. Fix import or
test mistakes until the only failure cause is the absent production workflow.

- [ ] **Step 3: Commit the RED contract test**

```bash
git add scripts/ci/test_ci_contract.py
git commit -m "test(ci): define GitHub Actions contract"
```

### Task 2: Add RED Phase 0 preparation tests

**Files:**
- Create: `scripts/ci/test_prepare_phase0.py`
- Test: `scripts/ci/test_prepare_phase0.py`

- [ ] **Step 1: Write subprocess tests for materialization and path safety**

Create tests that run the wished-for command in a temporary directory under
the repository so the production path-containment rule is exercised:

```python
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "prepare_phase0.py"
EXPECTED = {"069500.KRX": 260, "229200.KRX": 260, "114260.KRX": 260}


def run_prepare(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(root)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


class PreparePhase0Tests(unittest.TestCase):
    def test_materializes_the_exact_phase0_partition_contract(self) -> None:
        with tempfile.TemporaryDirectory(prefix=".phase0-test-", dir=ROOT) as tmp:
            root = Path(tmp) / "phase0"
            proc = run_prepare(root)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            summary = json.loads(proc.stdout)
            self.assertEqual(summary["sessions"], EXPECTED)
            self.assertEqual(summary["total_bars"], 780)
            paths = sorted(root.glob("curated/curated/bars/market=kr/symbol=*/year=*/version=1/bars.parquet"))
            counts: dict[str, int] = {}
            for path in paths:
                table = pq.read_table(path, columns=["instrument_id"])
                for value in table.column("instrument_id").to_pylist():
                    counts[value] = counts.get(value, 0) + 1
            self.assertEqual(counts, EXPECTED)
            self.assertEqual(len(paths), 6)

    def test_refuses_a_nonempty_destination(self) -> None:
        with tempfile.TemporaryDirectory(prefix=".phase0-test-", dir=ROOT) as tmp:
            root = Path(tmp) / "phase0"
            root.mkdir()
            (root / "stale").write_text("stale", encoding="utf-8")
            proc = run_prepare(root)
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("destination must be absent or empty", proc.stderr)

    def test_refuses_a_destination_outside_the_repository(self) -> None:
        with tempfile.TemporaryDirectory(prefix="phase0-outside-") as tmp:
            proc = run_prepare(Path(tmp) / "phase0")
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("destination must be inside repository", proc.stderr)
```

- [ ] **Step 2: Run the tests and observe the expected RED**

Run:

```bash
uv run --with pyarrow==25.0.0 python -m unittest scripts.ci.test_prepare_phase0 -v
```

Expected: the materialization test FAILS because
`scripts/ci/prepare_phase0.py` does not exist. The two rejection tests must not
be treated as GREEN until the production command exists; their expected error
messages are part of the later GREEN evidence.

- [ ] **Step 3: Commit the RED data contract**

```bash
git add scripts/ci/test_prepare_phase0.py
git commit -m "test(ci): define Phase 0 preparation contract"
```

### Task 3: Implement the Phase 0 preparation boundary

**Files:**
- Create: `tests/golden/phase0/phase0_dataset.py`
- Create: `scripts/ci/prepare_phase0.py`
- Modify: `tests/golden/phase0/runner.py:31-173,228-230`
- Test: `scripts/ci/test_prepare_phase0.py`
- Test: `tests/golden/phase0/test_phase0_gate.py`

- [ ] **Step 1: Extract the existing Parquet materializer without behavior changes**

Move `_bars_table`, `_adjusted_table`, and `materialize_curated_zone` from
`runner.py` into `phase0_dataset.py`. The new module imports `date`, `Path`,
`pyarrow as pa`, and `pyarrow.parquet as pq`; the bodies remain byte-for-byte
equivalent. In `runner.py`, replace the removed imports and functions with:

```python
import phase0_dataset  # noqa: E402
import synth_data  # noqa: E402
```

and replace the call with:

```python
phase0_dataset.materialize_curated_zone(rows, curated_root)
```

- [ ] **Step 2: Implement the safe command**

Create `scripts/ci/prepare_phase0.py` with this entrypoint and validation flow:

```python
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parents[2]
PHASE0 = ROOT / "tests" / "golden" / "phase0"
sys.path.insert(0, str(PHASE0))

import phase0_dataset  # noqa: E402
import synth_data  # noqa: E402

EXPECTED = {instrument: synth_data.SESSIONS for instrument in synth_data.INSTRUMENTS}


def destination(value: str) -> Path:
    path = Path(value)
    resolved = (ROOT / path).resolve() if not path.is_absolute() else path.resolve()
    try:
        relative = resolved.relative_to(ROOT)
    except ValueError as exc:
        raise ValueError("destination must be inside repository") from exc
    if relative == Path("."):
        raise ValueError("destination must not be repository root")
    return resolved


def prepare(root: Path) -> dict[str, object]:
    if root.exists() and any(root.iterdir()):
        raise ValueError("destination must be absent or empty")
    root.mkdir(parents=True, exist_ok=True)
    rows = synth_data.generate_curated_rows()
    phase0_dataset.materialize_curated_zone(rows, root / "curated")
    counts: dict[str, int] = {}
    paths = sorted(root.glob("curated/curated/bars/market=kr/symbol=*/year=*/version=1/bars.parquet"))
    for path in paths:
        table = pq.read_table(path, columns=["instrument_id"])
        for value in table.column("instrument_id").to_pylist():
            counts[value] = counts.get(value, 0) + 1
    if counts != EXPECTED or len(paths) != 6:
        raise RuntimeError(f"Phase 0 validation failed: partitions={len(paths)}, sessions={counts}")
    return {"root": str(root.relative_to(ROOT)), "sessions": counts, "total_bars": sum(counts.values())}


def main() -> int:
    parser = argparse.ArgumentParser(description="materialize deterministic Phase 0 Parquet data")
    parser.add_argument("--root", required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(prepare(destination(args.root)), sort_keys=True))
        return 0
    except Exception as exc:
        print(f"PREPARE_PHASE0_ERROR: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 3: Run preparation tests GREEN**

Run:

```bash
uv run --with pyarrow==25.0.0 python -m unittest scripts.ci.test_prepare_phase0 -v
```

Expected: 3 tests PASS, including exact 780-row evidence and both typed refusal
messages.

- [ ] **Step 4: Prove the golden runner still consumes the shared materializer**

Run:

```bash
uv run --project nt pytest tests/golden/phase0/test_phase0_gate.py -q
```

Expected: all Phase 0 gate tests PASS.

- [ ] **Step 5: Commit the implementation**

```bash
git add scripts/ci/prepare_phase0.py tests/golden/phase0/phase0_dataset.py tests/golden/phase0/runner.py
git commit -m "feat(ci): materialize Phase 0 test data"
```

### Task 4: Implement the PR and main CI workflows

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/research-smoke.yml`
- Test: `scripts/ci/test_ci_contract.py`

- [ ] **Step 1: Add the PR/main verification workflow**

Create `ci.yml` with:

```yaml
name: CI

on:
  pull_request:
    branches: [main]
  push:
    branches: [main]
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: ci-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

env:
  CARGO_INCREMENTAL: '0'
  CARGO_TERM_COLOR: always

jobs:
  policy:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0
        with:
          python-version: '3.12'
      - run: python -m pip install --disable-pip-version-check --no-cache-dir PyYAML==6.0.2
      - run: bash scripts/check-pins.sh
      - run: bash scripts/validate-foundation.sh
      - run: python -m unittest scripts.ci.test_ci_contract -v

  format:
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: cargo fmt --all -- --check

  clippy:
    runs-on: ubuntu-latest
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

  workspace-tests:
    runs-on: ubuntu-latest
    timeout-minutes: 60
    env:
      DATABASE_URL: postgres://postgres:lagrange@127.0.0.1:55432/postgres
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0
        with:
          python-version: '3.12'
      - run: python -m pip install --disable-pip-version-check --no-cache-dir pyarrow==25.0.0
      - run: python scripts/ci/prepare_phase0.py --root data/phase0
      - run: docker compose -f deploy/qa/qa-db.compose.yml up -d --wait
      - run: cargo test --workspace --locked --no-fail-fast
      - if: always()
        run: docker compose -f deploy/qa/qa-db.compose.yml down -v --remove-orphans

  required:
    if: always()
    needs: [policy, format, clippy, workspace-tests]
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - name: Require all CI jobs
        env:
          POLICY_RESULT: ${{ needs.policy.result }}
          FORMAT_RESULT: ${{ needs.format.result }}
          CLIPPY_RESULT: ${{ needs.clippy.result }}
          TEST_RESULT: ${{ needs.workspace-tests.result }}
        run: |
          test "$POLICY_RESULT" = success
          test "$FORMAT_RESULT" = success
          test "$CLIPPY_RESULT" = success
          test "$TEST_RESULT" = success
```

- [ ] **Step 2: Add the main-only Compose workflow**

Create `research-smoke.yml` with:

```yaml
name: Research worker smoke

on:
  push:
    branches: [main]
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: research-smoke-${{ github.ref }}
  cancel-in-progress: false

jobs:
  research-smoke:
    runs-on: ubuntu-latest
    timeout-minutes: 40
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: bash scripts/qa/research-worker-smoke.sh
```

- [ ] **Step 3: Run the workflow contract test GREEN**

Run:

```bash
uv run --with PyYAML==6.0.2 python -m unittest scripts.ci.test_ci_contract -v
```

Expected: all workflow trigger, pin, permission, timeout, storage, and command
contract tests PASS.

- [ ] **Step 4: Run existing pin and static smoke checks**

```bash
bash scripts/check-pins.sh
bash scripts/validate-foundation.sh
bash scripts/qa/research-worker-smoke.sh --static-only
```

Expected: all three commands exit 0.

- [ ] **Step 5: Commit the workflows**

```bash
git add .github/workflows/ci.yml .github/workflows/research-smoke.yml
git commit -m "ci: verify pull requests and main pushes"
```

### Task 5: Normalize the existing rustfmt baseline

**Files:**
- Modify: `crates/api-server/src/http/live.rs`
- Modify: `crates/api-server/src/live_order.rs`
- Modify: `crates/api-server/tests/http_backtests.rs`
- Modify: `crates/api-server/tests/paper_execution_seam.rs`
- Modify: `crates/factor-engine/tests/common/mod.rs`
- Modify: `crates/factor-engine/tests/point_in_time.rs`
- Modify: `crates/job-queue/src/paper_execution.rs`
- Modify: `crates/job-queue/src/runner.rs`
- Modify: `crates/market-data/src/curate/schema.rs`
- Modify: `crates/market-data/src/publication.rs`
- Modify: `crates/market-data/src/storage.rs`
- Modify: `crates/market-data/tests/publication.rs`
- Modify: `crates/portfolio-model/src/cost.rs`

- [ ] **Step 1: Record the current formatting RED**

Run:

```bash
cargo fmt --all -- --check
```

Expected: exit 1 with formatting-only diffs in exactly the 13 files listed
above.

- [ ] **Step 2: Apply the pinned formatter mechanically**

```bash
cargo fmt --all
```

Do not hand-edit or mix semantic changes into these hunks.

- [ ] **Step 3: Verify the formatter gate GREEN and scope**

```bash
cargo fmt --all -- --check
git diff --check
```

Expected: both exit 0. Review `git diff --stat` and verify the changes are
formatting-only in the listed files.

- [ ] **Step 4: Commit formatting separately**

```bash
git add crates/api-server crates/factor-engine crates/job-queue crates/market-data crates/portfolio-model
git commit -m "style: normalize Rust formatting baseline"
```

### Task 6: Prove the generated data fixes the clean-checkout test gap

**Files:**
- No production file changes expected.
- Test: `crates/job-queue/tests/backtest_runner.rs`

- [ ] **Step 1: Ensure the ignored dataset is absent**

```bash
test ! -e data/phase0 || rm -rf -- data/phase0
```

- [ ] **Step 2: Materialize and run the formerly failing target**

```bash
uv run --with pyarrow==25.0.0 python scripts/ci/prepare_phase0.py --root data/phase0
cargo test -p job-queue --test backtest_runner --locked -- --nocapture
```

Expected: the generator reports 780 bars and the test target passes instead of
failing on `data/phase0/curated/curated/bars` being absent.

- [ ] **Step 3: Confirm generated binaries remain ignored**

```bash
git status --short --ignored data/phase0
```

Expected: only ignored `data/phase0/` entries, with no tracked dataset change.

### Task 7: Run final verification and document the handoff

**Files:**
- Modify: `docs/STATUS.md`

- [ ] **Step 1: Record the CI trigger and local command contract**

Add a concise dated STATUS entry stating:

```markdown
- GitHub Actions CI: pull requests and pushes to `main` run policy, rustfmt,
  workspace Clippy, deterministic Phase 0 generation, disposable PostgreSQL,
  and the full Rust workspace test. Pushes to `main` additionally run the full
  research-worker Compose smoke. There is deliberately no scheduled/nightly
  trigger. Generated Phase 0 data and Rust targets remain ephemeral and are
  not uploaded or cached.
```

- [ ] **Step 2: Run fresh contract and focused verification**

```bash
uv run --with PyYAML==6.0.2 python -m unittest scripts.ci.test_ci_contract -v
uv run --with pyarrow==25.0.0 python -m unittest scripts.ci.test_prepare_phase0 -v
cargo fmt --all -- --check
cargo test -p job-queue --test backtest_runner --locked
bash scripts/qa/research-worker-smoke.sh --static-only
git diff --check
```

Expected: every command exits 0.

- [ ] **Step 3: Run the expensive local gates once before integration**

With the disposable QA database running and `DATABASE_URL` set:

```bash
docker compose -f deploy/qa/qa-db.compose.yml up -d --wait
DATABASE_URL=postgres://postgres:lagrange@127.0.0.1:55432/postgres CARGO_INCREMENTAL=0 cargo test --workspace --locked --no-fail-fast
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
docker compose -f deploy/qa/qa-db.compose.yml down -v --remove-orphans
```

Expected: workspace tests and Clippy exit 0; cleanup exits 0 even if a prior
gate failed.

- [ ] **Step 4: Commit the status update**

```bash
git add docs/STATUS.md
git commit -m "docs: record GitHub Actions verification"
```

- [ ] **Step 5: Final branch evidence**

```bash
git status --short --branch
git log --oneline --decorate -8
git diff main...HEAD --check
```

Expected: clean feature branch, all intended commits visible, and no whitespace
errors. Merge into local `main` only after this evidence is green. Pushing is a
separate remote mutation and requires available GitHub authentication.
