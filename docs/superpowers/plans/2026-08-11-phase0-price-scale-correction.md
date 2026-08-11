# Phase 0 Price Scale Correction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish the corrected Phase 0 prices as immutable dataset version 2 while preserving the intended scale-4 simulation results and preventing another Decimal/raw boundary error.

**Architecture:** Source fixtures remain integer KRW. The materializer writes logical `Decimal` values to Parquet version-2 partitions, and the catalog/golden consumers perform an explicit exact conversion to raw scale-4 or scale-8 integers. Rust Paper/factor consumers share one active Phase 0 contract, and every dependent golden/provenance identity is regenerated as v2.

**Tech Stack:** Python 3.12, `decimal.Decimal`, PyArrow 25, NautilusTrader/uv, Rust/Cargo, JSON golden manifests, GitHub Actions.

---

## File structure

- `tests/golden/phase0/synth_data.py` — authoritative Phase 0 values, version IDs, and exact Decimal/raw conversion.
- `tests/golden/phase0/phase0_dataset.py` — Parquet schemas and versioned partition writer.
- `nt/custom-data/catalog_builder.py` — Parquet logical Decimal to event raw-integer boundary.
- `tests/golden/phase0/runner.py` and `tests/golden/robustness/runner.py` — scale-4 simulation consumers and golden emitters.
- `scripts/ci/prepare_phase0.py` — CI materialization and dataset-version evidence.
- `crates/job-queue/src/phase0.rs` — shared Rust active Phase 0 dataset ID and curated partition version.
- Phase 0/robustness `golden.json`, outputs, summaries, `golden-set.json`, and manifests — approved v2 evidence.
- Focused Python/Rust tests — logical price, raw conversion, version routing, deterministic golden verification.
- `docs/STATUS.md` — records v1 as defective and v2 as the active corrected baseline.

### Task 0: Create the isolated implementation worktree

**Files:**
- No source files.

- [ ] **Step 1: Create a feature worktree from the approved design commit**

Use `superpowers:using-git-worktrees`, verify that `.worktrees` is ignored,
and run from the primary checkout:

```powershell
git check-ignore -q .worktrees
if ($LASTEXITCODE -ne 0) { throw ".worktrees must be ignored before use" }
git worktree add .worktrees/phase0-price-scale-v2 -b fix/phase0-price-scale-v2 main
git -C .worktrees/phase0-price-scale-v2 status --short
```

Expected: the worktree is created at the current clean `main`, the feature
branch is checked out, and status prints nothing. Run Tasks 1–6 inside this
worktree.

### Task 1: Correct the logical Decimal and raw-integer boundary

**Files:**
- Modify: `tests/golden/phase0/synth_data.py:35-50,215-264`
- Modify: `tests/golden/phase0/phase0_dataset.py:1-88`
- Modify: `tests/golden/phase0/runner.py:88-103,140-150`
- Modify: `tests/golden/robustness/runner.py:103-121`
- Modify: `nt/custom-data/catalog_builder.py:92-105,169-214`
- Modify: `nt/custom-data/tests/curated_helpers.py:1-170`
- Modify: `nt/custom-data/tests/test_catalog_builder.py:30-55`
- Modify: `scripts/ci/prepare_phase0.py:16-57`
- Modify: `scripts/ci/test_prepare_phase0.py:1-72`
- Modify: `tests/golden/phase0/test_phase0_gate.py:25-205`
- Modify: `nt/backtest-worker/tests/test_worker.py:34-84`

- [ ] **Step 1: Write the failing Parquet and conversion tests**

In `scripts/ci/test_prepare_phase0.py`, import `date`, `Decimal`, PyArrow, and
PyArrow compute, target the first 069500 partition, and add the exact
logical-value assertions:

```python
from datetime import date
from decimal import Decimal
import pyarrow as pa
import pyarrow.compute as pc

bars = pq.read_table(
    root / "curated/curated/bars/market=kr/symbol=069500.KRX/"
    "year=2020/version=2/bars.parquet"
)
first = bars.filter(
    pc.equal(bars["trading_date"], pa.scalar(date(2020, 1, 20)))
).to_pylist()[0]
self.assertEqual(first["open"], Decimal("10150.0000"))
self.assertNotEqual(first["open"], Decimal("101500000.0000"))
self.assertLessEqual(first["low"], min(first["open"], first["close"]))
self.assertGreaterEqual(first["high"], max(first["open"], first["close"]))

for path in paths:
    for row in pq.read_table(path).to_pylist():
        self.assertGreater(row["open"], 0)
        self.assertGreater(row["close"], 0)
        self.assertLessEqual(row["low"], min(row["open"], row["close"]))
        self.assertGreaterEqual(row["high"], max(row["open"], row["close"]))

adjusted = pq.read_table(
    root / "curated/curated/bars/market=kr/symbol=069500.KRX/"
    "year=2020/version=2/adjusted_bars.parquet"
)
self.assertEqual(adjusted["adjustment_factor"][0].as_py(), Decimal("1.00000000"))
```

Also assert the preparation summary:

```python
self.assertEqual(summary["dataset_version"], "kr-etf-daily-phase0-v2")
self.assertEqual(summary["curated_version"], 2)
self.assertFalse(any(root.rglob("version=1")))
```

In `tests/golden/phase0/test_phase0_gate.py`, add the exact raw conversion
contract:

```python
from decimal import Decimal

def test_decimal_krw_converts_to_exact_raw4() -> None:
    synth = _load_synth()
    assert synth.decimal_to_raw4(Decimal("10150.0000")) == 101_500_000
    with pytest.raises(ValueError, match="scale 4"):
        synth.decimal_to_raw4(Decimal("10150.00001"))
    with pytest.raises(ValueError, match="finite"):
        synth.decimal_to_raw4(Decimal("NaN"))
```

Change `nt/custom-data/tests/curated_helpers.py` test input values from
pre-scaled integers to logical Decimals:

```python
from decimal import Decimal

PRICE_QUANTUM = Decimal("0.0001")
FACTOR_ONE = Decimal("1.00000000")

def _price(value: int) -> Decimal:
    return Decimal(value).quantize(PRICE_QUANTUM)
```

Use `_price(bar["open"])` for each OHLC field and `FACTOR_ONE` for
`adjustment_factor`. Keep the event assertions in
`test_catalog_builder.py` at `101_500_000`, `103_800_000`, and
`100_000_000`; the corrected reader must recreate those raw integers.

Add an explicit mixed-version rejection test to
`nt/custom-data/tests/test_catalog_builder.py`:

```python
def test_builder_rejects_mixed_curated_versions(builder, curated_root, tmp_path):
    import shutil

    version1 = (
        curated_root / "curated/bars/market=kr/symbol=069500.KRX/"
        "year=2020/version=1"
    )
    version2 = version1.parent / "version=2"
    shutil.copytree(version1, version2)
    with pytest.raises(Exception, match="mixed curated versions"):
        builder.build_catalog(curated_root, tmp_path / "catalog")
```

- [ ] **Step 2: Run the focused tests and capture RED**

Run:

```powershell
uv run --with pyarrow==25.0.0 python -m unittest scripts.ci.test_prepare_phase0 -v
uv run --project nt pytest nt/custom-data/tests/test_catalog_builder.py tests/golden/phase0/test_phase0_gate.py::test_decimal_krw_converts_to_exact_raw4 -q
```

Expected: the preparation test cannot find `version=2`; the conversion test
reports that `decimal_to_raw4` is missing; the catalog round-trip returns
`10150` instead of `101500000` after the logical test fixture is corrected.

- [ ] **Step 3: Implement the authoritative Decimal helpers and v2 constants**

In `tests/golden/phase0/synth_data.py`, use:

```python
from decimal import Decimal

GENERATOR_VERSION = "2.0.0"
DATA_VERSION = "kr-etf-daily-phase0-v2"
CURATED_VERSION = 2
PRICE_SCALE = 4
PRICE_FACTOR = 10 ** PRICE_SCALE
PRICE_QUANTUM = Decimal("0.0001")

def krw_decimal(value: int) -> Decimal:
    return Decimal(value).quantize(PRICE_QUANTUM)

def decimal_to_raw4(value: Decimal) -> int:
    if not value.is_finite():
        raise ValueError("price must be finite at scale 4")
    scaled = value * PRICE_FACTOR
    if scaled != scaled.to_integral_value():
        raise ValueError(f"price {value} cannot be represented exactly at scale 4")
    return int(scaled)
```

Replace all four `int(bar[field]) * 10_000` expressions in
`generate_curated_rows()` with `krw_decimal(int(bar[field]))`.

In `tests/golden/phase0/phase0_dataset.py`, make the partition version
explicit and store the logical identity adjustment factor:

```python
from decimal import Decimal

def materialize_curated_zone(
    rows: list[dict], curated_root: Path, *, version: int
) -> None:
    bars_table = _bars_table(rows)
    adj_table = _adjusted_table([
        {
            **row,
            "adjustment_kind": "split",
            "adjustment_factor": Decimal("1.00000000"),
            "adjustment_events": "[]",
        }
        for row in rows
    ])
    seen: set[tuple[str, str]] = set()
    for row in rows:
        iid = row["instrument_id"]
        year = row["trading_date"][:4]
        if (iid, year) in seen:
            continue
        seen.add((iid, year))
        mask = pa.array([
            candidate["instrument_id"] == iid
            and candidate["trading_date"][:4] == year
            for candidate in rows
        ])
        part = (
            curated_root / "curated" / "bars" / "market=kr"
            / f"symbol={iid}" / f"year={year}" / f"version={version}"
        )
        part.mkdir(parents=True, exist_ok=True)
        pq.write_table(bars_table.filter(mask), part / "bars.parquet")
        pq.write_table(adj_table.filter(mask), part / "adjusted_bars.parquet")
```

Pass `version=synth_data.CURATED_VERSION` from `runner.py`,
`scripts/ci/prepare_phase0.py`, and `nt/backtest-worker/tests/test_worker.py`.
In the CI preparation validator, replace its literal partition glob with:

```python
partition = f"version={synth_data.CURATED_VERSION}"
paths = sorted(root.glob(
    "curated/curated/bars/market=kr/"
    f"symbol=*/year=*/{partition}/bars.parquet"
))
```

The worker test must call the module actually imported by the runner:

```python
runner.phase0_dataset.materialize_curated_zone(
    rows, data_root / "curated", version=synth.CURATED_VERSION
)
```

The preparation summary must return the two version fields:

```python
return {
    "root": str(root.relative_to(ROOT)),
    "dataset_version": synth_data.DATA_VERSION,
    "curated_version": synth_data.CURATED_VERSION,
    "sessions": counts,
    "total_bars": sum(counts.values()),
}
```

- [ ] **Step 4: Implement exact catalog and runner conversions**

Replace `catalog_builder._fixed_to_int` with a scale-aware exact conversion:

```python
def _fixed_to_int(table: pa.Table, column: str, scale: int) -> list[int]:
    raw: list[int] = []
    factor = 10 ** scale
    for value in table.column(column).to_pylist():
        if value is None or not value.is_finite():
            raise CatalogBuilderError(f"curated column {column!r} contains an invalid decimal")
        scaled = value * factor
        if scaled != scaled.to_integral_value():
            raise CatalogBuilderError(
                f"curated column {column!r} value {value} exceeds scale {scale}"
            )
        raw.append(int(scaled))
    return raw
```

Call it with scale 4 for OHLC and scale 8 for `adjustment_factor`:

```python
opens = _fixed_to_int(table, "open", 4)
highs = _fixed_to_int(table, "high", 4)
lows = _fixed_to_int(table, "low", 4)
closes = _fixed_to_int(table, "close", 4)
factors = _fixed_to_int(adj, "adjustment_factor", 8)
```

At the start of `_read_curated_rows`, reject a mixed active store before
reading any row:

```python
bars_paths = sorted(bars_dir.rglob("bars.parquet"))
versions = {path.parent.name for path in bars_paths}
if len(versions) > 1:
    raise CatalogBuilderError(
        f"mixed curated versions are not allowed: {sorted(versions)}"
    )
```

Change the subsequent loop header from
`for bars_path in sorted(bars_dir.rglob("bars.parquet")):` to
`for bars_path in bars_paths:`; its body then uses the scale-aware conversions
shown above.

Keep the existing nonempty-destination preparation test: it prevents a v1
tree from being reused as the destination of v2 generation.

In `phase0/runner.py` use:

```python
open_raw4 = synth_data.decimal_to_raw4(row["open"])
```

In `robustness/runner.py`, build the session schedule with:

```python
day[row["instrument_id"]] = {
    "open": synth_data.decimal_to_raw4(row["open"]),
    "close": synth_data.decimal_to_raw4(row["close"]),
}
```

- [ ] **Step 5: Run the corrected boundary tests**

Run:

```powershell
uv run --with pyarrow==25.0.0 python -m unittest scripts.ci.test_prepare_phase0 -v
uv run --project nt pytest nt/custom-data/tests/test_catalog_builder.py tests/golden/phase0/test_phase0_gate.py::test_decimal_krw_converts_to_exact_raw4 tests/golden/phase0/test_phase0_gate.py::test_at02_fill_price_is_next_raw_open_plus_slippage -q
```

Expected: preparation reports 780 bars in version 2; Parquet open is
`10150.0000`; catalog open raw is `101500000`; all focused tests pass.

- [ ] **Step 6: Commit the scale boundary and v2 materializer**

```powershell
git add tests/golden/phase0/synth_data.py tests/golden/phase0/phase0_dataset.py tests/golden/phase0/runner.py tests/golden/robustness/runner.py nt/custom-data/catalog_builder.py nt/custom-data/tests/curated_helpers.py nt/custom-data/tests/test_catalog_builder.py scripts/ci/prepare_phase0.py scripts/ci/test_prepare_phase0.py tests/golden/phase0/test_phase0_gate.py nt/backtest-worker/tests/test_worker.py
git diff --cached --check
git commit -m "fix(phase0): correct curated decimal price scale"
```

### Task 2: Route active Rust and request consumers to Phase 0 v2

**Files:**
- Create: `crates/job-queue/src/phase0.rs`
- Modify: `crates/job-queue/src/lib.rs`
- Modify: `crates/job-queue/src/paper_execution.rs:70-80`
- Modify: `crates/job-queue/src/paper_valuation.rs:15-28`
- Modify: `crates/job-queue/src/factor_series.rs:320-375`
- Modify: `crates/job-queue/src/resolver.rs:28-40`
- Modify: `crates/job-queue/tests/backtest_runner.rs:140-155`
- Modify: `crates/api-server/tests/paper_execution_seam.rs:42-125,210-225`
- Modify: `crates/api-server/tests/paper_runner.rs:70-88`
- Modify: `crates/api-server/tests/paper_valuation.rs:42-60`
- Modify: `crates/result-model/tests/manifest_db.rs:270-288`
- Modify: `crates/result-model/tests/fixtures/manifest.json`
- Modify: `crates/result-model/tests/fixtures/result.json`
- Modify: `nt/backtest-worker/tests/test_worker.py:70-88`

- [ ] **Step 1: Add a failing shared-contract test**

Create `crates/job-queue/src/phase0.rs` first with only the test so it fails to
compile on missing constants:

```rust
#[cfg(test)]
mod tests {
    use super::{CURATED_VERSION, DATASET_ID};

    #[test]
    fn active_phase0_contract_is_the_corrected_v2_dataset() {
        assert_eq!(DATASET_ID, "kr-etf-daily-phase0-v2");
        assert_eq!(CURATED_VERSION, 2);
    }
}
```

Expose the module from `lib.rs` with `pub mod phase0;` and run:

```powershell
cargo test -p job-queue phase0::tests::active_phase0_contract_is_the_corrected_v2_dataset --locked
```

Expected: compile failure because `DATASET_ID` and `CURATED_VERSION` do not
exist.

- [ ] **Step 2: Add the shared contract and replace production literals**

Add above the test in `phase0.rs`:

```rust
//! Active deterministic Phase 0 dataset contract shared by Paper consumers.

/// Corrected immutable Phase 0 dataset identity.
pub const DATASET_ID: &str = "kr-etf-daily-phase0-v2";
/// Corrected curated partition version.
pub const CURATED_VERSION: u32 = 2;
```

Import `crate::phase0::CURATED_VERSION` in `paper_execution.rs` and
`paper_valuation.rs`, remove their private `VERSION` constants, and pass
`CURATED_VERSION` to `CurateStore::bars_path`. In the Phase 0 fixture tests in
`factor_series.rs`, replace both `version=1` path components and the builder
version argument with `CURATED_VERSION`/`format!("version={CURATED_VERSION}")`.

- [ ] **Step 3: Update every active request/test identity**

Make these exact mechanical replacements:

```text
kr-etf-daily-phase0-v1 -> kr-etf-daily-phase0-v2
```

in:

```text
crates/job-queue/tests/backtest_runner.rs
crates/api-server/tests/paper_execution_seam.rs
crates/result-model/tests/manifest_db.rs
crates/result-model/tests/fixtures/manifest.json
crates/result-model/tests/fixtures/result.json
nt/backtest-worker/tests/test_worker.py
```

Use `job_queue::phase0::CURATED_VERSION` for the temporary curated stores in
`paper_execution_seam.rs`, `paper_runner.rs`, and `paper_valuation.rs`. Replace
the obsolete paper seam explanation with:

```rust
/// The repository fixture is kept tiny so the seam remains fast. Its prices
/// are the same integer-KRW source values as corrected Phase 0 v2, written
/// through the production CurateStore schema at the active partition version.
```

Update `resolver.rs` to say that extending the data span would create another
immutable dataset version after v2; do not claim that v1 is still active.

- [ ] **Step 4: Run focused Rust and worker checks**

Run:

```powershell
cargo test -p job-queue phase0::tests::active_phase0_contract_is_the_corrected_v2_dataset --locked
cargo check -p job-queue -p api-server -p result-model --all-targets --locked
uv run --project nt pytest nt/backtest-worker/tests/test_worker.py -q
```

Expected: all commands pass and the worker result records
`kr-etf-daily-phase0-v2`.

- [ ] **Step 5: Commit active v2 routing**

```powershell
git add crates/job-queue/src/phase0.rs crates/job-queue/src/lib.rs crates/job-queue/src/paper_execution.rs crates/job-queue/src/paper_valuation.rs crates/job-queue/src/factor_series.rs crates/job-queue/src/resolver.rs crates/job-queue/tests/backtest_runner.rs crates/api-server/tests/paper_execution_seam.rs crates/api-server/tests/paper_runner.rs crates/api-server/tests/paper_valuation.rs crates/result-model/tests/manifest_db.rs crates/result-model/tests/fixtures/manifest.json crates/result-model/tests/fixtures/result.json nt/backtest-worker/tests/test_worker.py
git diff --cached --check
git commit -m "fix(phase0): route consumers to dataset v2"
```

### Task 3: Regenerate and approve the Phase 0 v2 golden

**Files:**
- Modify: `tests/golden/phase0/golden.json`
- Modify: `tests/golden/phase0/test_phase0_gate.py`
- Regenerate: `tests/golden/phase0/outputs/*.json`
- Regenerate: `tests/golden/phase0/manifest.json`

- [ ] **Step 1: Change the Phase 0 golden identities**

Use these exact values in `golden.json` and the test constant:

```json
{
  "golden_id": "kr-etf-phase0-next-open-v2",
  "versions": {
    "data": {"id": "kr-etf-daily-phase0-v2", "version": "2.0.0", "source": "synthetic"},
    "config": {"id": "golden-config-phase0-v2"}
  }
}
```

```python
GOLDEN_ID = "kr-etf-phase0-next-open-v2"
```

- [ ] **Step 2: Confirm the old committed golden identity is RED**

Run:

```powershell
uv run --project nt pytest tests/golden/phase0/test_phase0_gate.py::test_committed_phase0_manifest_verifies -q
```

Expected: failure because the committed manifest still reports
`kr-etf-phase0-next-open-v1` while the approved test contract requires v2.

- [ ] **Step 3: Regenerate outputs with a pinned implementation commit**

Run from the feature worktree:

```powershell
$pin = git rev-parse HEAD
$regen = Join-Path (Get-Location) 'data/phase0-v2-regeneration'
if (Test-Path -LiteralPath $regen) { throw "regeneration path must start absent: $regen" }
uv run --project nt python tests/golden/phase0/runner.py --out-dir tests/golden/phase0/outputs --data-root $regen --code-commit $pin
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
uv run --project nt python scripts/golden/golden.py generate tests/golden/phase0/golden.json -o tests/golden/phase0/manifest.json --code-override $pin
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$resolved = (Resolve-Path -LiteralPath $regen).Path
$expected = [System.IO.Path]::GetFullPath($regen)
if ($resolved -ne $expected -or -not $resolved.StartsWith((Join-Path (Get-Location) 'data'))) { throw "unsafe cleanup target: $resolved" }
Remove-Item -LiteralPath $resolved -Recurse -Force
```

- [ ] **Step 4: Prove execution artifacts did not change semantically**

Only provenance and summary should change. Check every execution artifact
against the pre-fix parent:

```powershell
@('recommendation.json','orders.json','fills.json','equity.json','fees.json','metrics.json') | ForEach-Object {
  $old = git show HEAD:tests/golden/phase0/outputs/$_
  $new = Get-Content -Raw tests/golden/phase0/outputs/$_
  if (($old -join "`n") + "`n" -ne $new) { throw "unexpected Phase 0 semantic delta: $_" }
}
```

Then verify:

```powershell
uv run --project nt python scripts/golden/golden.py verify tests/golden/phase0/manifest.json
uv run --project nt pytest tests/golden/phase0/test_phase0_gate.py tests/golden/phase0/test_unapproved_delta.py -q
```

Expected: `VERDICT: PASS`; both test modules pass; provenance names v2 and
generator 2.0.0.

- [ ] **Step 5: Commit the Phase 0 v2 golden evidence**

```powershell
git add tests/golden/phase0/golden.json tests/golden/phase0/test_phase0_gate.py tests/golden/phase0/outputs tests/golden/phase0/manifest.json
git diff --cached --check
git commit -m "test(phase0): approve corrected v2 golden"
```

### Task 4: Regenerate and approve the robustness v2 golden set

**Files:**
- Modify: `tests/golden/robustness/runner.py:490-520`
- Modify: `tests/golden/robustness/golden.json`
- Modify: `tests/golden/robustness/test_five_strategies_gate.py:45-62`
- Modify: `crates/result-model/tests/robustness_gate_committed.rs:25-38`
- Modify: `crates/result-model/tests/robustness_gate.rs:55-72`
- Regenerate: `tests/golden/robustness/strategies/*/outputs/*.json`
- Regenerate: `tests/golden/robustness/strategies/*/summary.json`
- Regenerate: `tests/golden/robustness/golden-set.json`
- Regenerate: `tests/golden/robustness/manifest.json`

- [ ] **Step 1: Update the robustness golden identities**

Use:

```text
kr-etf-five-strategies-v2
golden-config-five-strategies-v2
kr-etf-daily-phase0-v2
2.0.0
```

in `runner.write_golden_set`, `golden.json`, the Python `GOLDEN_ID`, and the
Rust committed-gate assertion. Update the synthetic `golden_id` used by
`crates/result-model/tests/robustness_gate.rs` to v2 as well. In
`write_golden_set`, use
`synth_data.GENERATOR_VERSION` instead of another literal data version:

```python
"data": {
    "id": synth_data.DATA_VERSION,
    "version": synth_data.GENERATOR_VERSION,
    "source": "synthetic",
},
```

- [ ] **Step 2: Confirm old robustness identity evidence is RED**

Run:

```powershell
uv run --project nt pytest tests/golden/robustness/test_five_strategies_gate.py::test_committed_golden_set_verifies -q
cargo test -p result-model --test robustness_gate_committed --locked
```

Expected: both fail because the committed golden set still reports v1 while
the approved Python and Rust contracts require v2.

- [ ] **Step 3: Regenerate all five strategies and manifests**

Run:

```powershell
$pin = git rev-parse HEAD^
$strategies = @('buy_and_hold','trend_following','relative_momentum','dual_momentum','inverse_volatility')
foreach ($strategy in $strategies) {
  uv run --project nt python tests/golden/robustness/runner.py --strategy-id $strategy --out-dir "tests/golden/robustness/strategies/$strategy" --code-commit $pin
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
uv run --project nt python tests/golden/robustness/runner.py --strategy-id buy_and_hold --out-dir tests/golden/robustness/strategies/buy_and_hold --code-commit $pin --write-golden-set
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
uv run --project nt python scripts/golden/golden.py generate tests/golden/robustness/golden.json -o tests/golden/robustness/manifest.json --code-override $pin
```

- [ ] **Step 4: Prove strategy economics stayed unchanged and verify hashes**

For each strategy compare the six non-provenance output files to `HEAD`:

```powershell
$artifacts = @('recommendation.json','orders.json','fills.json','equity.json','fees.json','metrics.json')
foreach ($strategy in $strategies) {
  foreach ($artifact in $artifacts) {
    $path = "tests/golden/robustness/strategies/$strategy/outputs/$artifact"
    $old = git show "HEAD:$path"
    $new = Get-Content -Raw $path
    if (($old -join "`n") + "`n" -ne $new) { throw "unexpected robustness semantic delta: $strategy/$artifact" }
  }
}
```

Then run:

```powershell
uv run --project nt python scripts/golden/golden.py verify tests/golden/robustness/manifest.json
uv run --project nt pytest tests/golden/robustness/test_five_strategies_gate.py -q
cargo test -p result-model --test robustness_gate_committed --locked
```

Expected: manifest `VERDICT: PASS`, Python gate passes all five strategies,
and the Rust core gate approves all 35 artifacts.

- [ ] **Step 5: Commit robustness v2 evidence**

```powershell
git add tests/golden/robustness crates/result-model/tests/robustness_gate_committed.rs crates/result-model/tests/robustness_gate.rs
git diff --cached --check
git commit -m "test(robustness): approve phase0 v2 baselines"
```

### Task 5: Close stale v1 references and document the correction

**Files:**
- Modify: `docs/STATUS.md:120-185`

- [ ] **Step 1: Scan for stale active identities**

Run:

```powershell
rg -n "kr-etf-daily-phase0-v1|kr-etf-phase0-next-open-v1|kr-etf-five-strategies-v1|golden-config-(phase0|five-strategies)-v1" crates data-pipelines nt scripts tests .github docs/STATUS.md --glob '!target/**'
```

Expected before cleanup: only remaining stale active references are listed.
Historical design/plan documents outside this scan are intentionally not
rewritten.

- [ ] **Step 2: Update STATUS with concrete v2 evidence**

Replace the “not fixed / approval required” text with a concise record:

```markdown
**US-006 resolved:** Phase 0 v1 pre-scaled logical Decimals and made
10,150 KRW read as 101,500,000.0000. The approved v2 baseline stores
10150.0000, converts to raw scale-4 only at the catalog/simulation boundary,
uses immutable version=2 partitions, and regenerates Phase 0 plus robustness
provenance. v1 remains historical and must not be used as the active dataset.
```

Remove the price-scale item from the remaining-work list. Do not alter the
260-session limitation or strategy roadmap.

- [ ] **Step 3: Require a clean active-reference scan**

Run:

```powershell
rg -n "kr-etf-daily-phase0-v1|kr-etf-phase0-next-open-v1|kr-etf-five-strategies-v1|golden-config-(phase0|five-strategies)-v1" crates data-pipelines nt scripts tests .github docs/STATUS.md --glob '!target/**'
```

Expected: no output and exit code 1 (no matches) for the scoped active files.
References in the approved design explaining the historical v1 defect remain.

- [ ] **Step 4: Commit documentation and final identity cleanup**

```powershell
git add docs/STATUS.md
git diff --cached --check
git commit -m "docs: record phase0 v2 price correction"
```

### Task 6: Run focused local verification without duplicating CI

**Files:**
- No intended source changes.

- [ ] **Step 1: Materialize one disposable active dataset**

Run:

```powershell
$localData = Join-Path (Get-Location) 'data/phase0'
if (Test-Path -LiteralPath $localData) { throw "data/phase0 must be absent before focused verification" }
uv run --with pyarrow==25.0.0 python scripts/ci/prepare_phase0.py --root data/phase0
```

Expected JSON includes `dataset_version=kr-etf-daily-phase0-v2`,
`curated_version=2`, and `total_bars=780`.

- [ ] **Step 2: Run focused Python gates**

```powershell
uv run --with pyarrow==25.0.0 python -m unittest scripts.ci.test_prepare_phase0 -v
uv run --project nt pytest nt/custom-data/tests/test_catalog_builder.py tests/golden/phase0/test_phase0_gate.py tests/golden/phase0/test_unapproved_delta.py tests/golden/robustness/test_five_strategies_gate.py nt/backtest-worker/tests/test_worker.py -q
```

Expected: all pass.

- [ ] **Step 3: Run affected Rust checks**

```powershell
cargo test -p result-model --test robustness_gate_committed --locked
cargo test -p job-queue phase0 --locked
cargo check -p job-queue -p api-server -p result-model --all-targets --locked
cargo clippy -p job-queue -p api-server -p result-model --all-targets --all-features --locked -- -D warnings
cargo fmt -p job-queue -p api-server -p result-model -- --check
```

Expected: all pass. Do not run `cargo test --workspace` locally; the push CI
job owns that long verification.

- [ ] **Step 4: Audit generated and ignored state**

```powershell
git diff --check
git status --short
git status --short --ignored data/phase0
```

Expected before cleanup: source worktree clean after commits;
`data/phase0` appears only as ignored generated data. Then remove only the
verified generated directory:

```powershell
$resolved = (Resolve-Path -LiteralPath $localData).Path
$expected = [System.IO.Path]::GetFullPath($localData)
if ($resolved -ne $expected -or -not $resolved.StartsWith((Join-Path (Get-Location) 'data'))) { throw "unsafe cleanup target: $resolved" }
Remove-Item -LiteralPath $resolved -Recurse -Force
git status --short
```

Expected after cleanup: status prints nothing.

### Task 7: Integrate and let GitHub Actions run the long gates

**Files:**
- No source changes unless CI identifies a real defect.

- [ ] **Step 1: Fast-forward the implementation branch into local main**

From the primary checkout after all focused tests pass:

```powershell
git switch main
git merge --ff-only fix/phase0-price-scale-v2
git status --short
```

Expected: fast-forward succeeds with no conflict and the worktree is clean.

- [ ] **Step 2: Push main once**

```powershell
git push origin main
```

This single push triggers the existing `CI` and `Research worker smoke`
workflows. It does not enable or add a nightly schedule.

- [ ] **Step 3: Observe GitHub Runner results**

```powershell
gh run list --branch main --limit 10
$head = git rev-parse HEAD
$ci = gh run list --workflow ci.yml --branch main --event push --json databaseId,headSha --jq ".[] | select(.headSha == `"$head`") | .databaseId" --limit 10 | Select-Object -First 1
$smoke = gh run list --workflow research-smoke.yml --branch main --event push --json databaseId,headSha --jq ".[] | select(.headSha == `"$head`") | .databaseId" --limit 10 | Select-Object -First 1
if (-not $ci -or -not $smoke) { throw "push-triggered workflow runs were not found for $head" }
gh run watch $ci --exit-status
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
gh run watch $smoke --exit-status
```

Expected: policy, format, clippy, workspace tests, required aggregate, and
research smoke all pass on GitHub-hosted runners. If a job fails, inspect only
that job with `gh run view --log-failed`, fix forward with a focused regression
test, and push once more.

- [ ] **Step 4: Record final evidence**

```powershell
git rev-parse HEAD
git status --short
gh run list --branch main --limit 5
```

Expected: local `main` equals the pushed commit, status is clean, and the two
push-triggered workflows show successful conclusions.
