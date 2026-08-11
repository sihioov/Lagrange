# Phase 0 Price Scale Correction Design

**Date:** 2026-08-11
**Status:** Approved for implementation
**Scope:** Correct the Phase 0 curated price representation and publish it as a new immutable dataset version.

## 1. Problem

The Phase 0 source fixtures express OHLC prices as integer KRW. For example,
`069500.KRX` opens at `10,150` KRW on 2020-01-20. The synthetic curated-data
generator currently multiplies that value by `10,000` before passing it to a
PyArrow `decimal128(18, 4)` column. PyArrow then applies the decimal scale,
causing readers to observe `101,500,000.0000` KRW instead of `10,150.0000`
KRW.

The existing Python golden runners accidentally depend on the oversized
decimal value as if it were an unscaled scale-4 integer. Ratio-based strategy
signals therefore remained plausible, while absolute-price consumers such as
Paper execution exposed the defect.

The adjusted-bars fixture repeats the same representation error for its
scale-8 adjustment factor: the logical factor `1.00000000` is supplied as the
pre-scaled integer `100_000_000`, so PyArrow stores `100000000.00000000`.
Correcting OHLC without correcting this companion Decimal boundary would
leave the adjusted-price path internally inconsistent.

The curated store contract states that corrections must never replace an
existing `version={v}` partition. The correction must therefore be published
as Phase 0 dataset version 2 rather than silently changing version 1.

## 2. Goals

- Store Phase 0 OHLC values as their actual KRW amounts in
  `decimal128(18, 4)` columns.
- Preserve deterministic scale-4 integer arithmetic inside golden simulation
  by converting decimal KRW to a raw scale-4 integer at the consumer boundary.
- Publish the corrected data, manifests, provenance, and dependent golden
  baselines under version-2 identities.
- Make the 10,000x regression directly observable in tests rather than relying
  only on generated-file hashes.
- Keep long workspace and deployment verification on GitHub Actions; run only
  focused feedback tests locally.

## 3. Non-goals

- Extending the 260-session synthetic history.
- Adding or changing trading strategies.
- Changing the source fixture prices, seed, session calendar, fees, slippage,
  or portfolio policy.
- Supporting both dataset versions in one runtime selection UI. Version 1
  remains historical evidence in Git; version 2 becomes the active Phase 0
  fixture.

## 4. Considered approaches

### A. Versioned data correction — selected

Create `kr-etf-daily-phase0-v2`, materialize it under `version=2`, and update
the active consumers and dependent golden identities. This honors curated-data
immutability and makes provenance unambiguous.

### B. Rewrite version 1 in place — rejected

This is a smaller patch, but the same dataset and partition identifiers would
refer to different bytes and prices over time. Historical runs could no longer
be reproduced reliably.

### C. Compensate in each reader — rejected

Dividing Phase 0 prices by 10,000 in Paper and other consumers would retain
invalid data at rest, spread fixture-specific logic across production readers,
and allow new consumers to repeat the bug.

## 5. Data contract

### 5.1 Source and curated representations

- Source fixture: integer KRW, e.g. `10150`.
- Curated logical value: decimal KRW, e.g. `Decimal("10150.0000")`.
- Parquet type: `decimal128(18, 4)`.
- Golden simulation raw value: scale-4 integer, e.g. `101_500_000`.
- Curated adjustment factor: decimal value `Decimal("1.00000000")`.
- Catalog adjustment-factor raw value: scale-8 integer `100_000_000`.

The generator will construct explicit four-decimal `Decimal` values and will
not pre-multiply them. A named exact conversion helper will convert curated
decimal KRW to a scale-4 integer only where the Phase 0 and robustness runners
need integer arithmetic. The helper will reject non-finite values and values
that cannot be represented exactly at scale 4.

The catalog builder will likewise convert Parquet decimal values to their
exact scale-4 or scale-8 raw integers instead of casting away their fractional
scale. This conversion is the only place where the event stream receives raw
integer prices and factors.

### 5.2 Version identities

- Dataset ID: `kr-etf-daily-phase0-v2`.
- Data/generator version: `2.0.0`.
- Curated partition: `version=2`.
- Phase 0 golden/config/provenance identities that encode `v1` will be bumped
  to `v2`.
- Robustness golden/config/provenance identities derived from the same Phase 0
  data will also be bumped to `v2`.

All runtime fixtures, result-model fixtures, resolver expectations, and test
contracts that name the active Phase 0 dataset must consistently reference
version 2. Historical version-1 content is not copied into the active data
tree and is not mutated.

## 6. Processing flow

1. Generate the same deterministic integer-KRW source bars.
2. Convert each OHLC value to an explicit four-decimal KRW value.
3. Write Parquet beneath the immutable `version=2` partitions.
4. Read curated decimals in Phase 0 and robustness runners.
5. Convert decimals to exact scale-4 integers at the simulation boundary.
6. Regenerate dependent outputs, manifests, content hashes, and provenance.

Signals and execution economics should remain semantically unchanged because
the runners regain the same intended scale-4 integers explicitly. Dataset,
config, provenance, and manifest hashes will change. Any other changed artifact
must be reviewed as a potential unintended behavioral regression.

## 7. Validation and error handling

Focused regression tests will assert:

- `069500.KRX` 2020-01-20 open reads from Parquet as exactly `10150.0000`.
- It does not read as `101500000.0000`.
- All OHLC values are positive and retain `low <= open/close <= high`.
- The prepared dataset contains 3 instruments, 260 sessions each, and 780
  total rows in version-2 partitions.
- Decimal-to-raw conversion maps `10150.0000` to `101_500_000` exactly and
  rejects excess precision.
- The adjusted-bars factor reads as `1.00000000` in Parquet and reaches the
  event catalog as raw scale-8 `100_000_000`.
- Phase 0 next-session-open fill assertions still compare the intended raw
  scale-4 values.
- No active manifests or provenance records retain the version-1 dataset ID.
- Golden manifest verification succeeds after regeneration.

Generation or loading will fail loudly on mixed version-1/version-2 active
paths, unexpected row counts, invalid decimals, or stale hashes.

## 8. Verification strategy

Local verification is limited to fast, focused tests for the generator,
materializer, Phase 0 gate, robustness gate, and affected Rust consumers.
Generated artifacts will be reviewed for semantic changes, especially fills,
fees, equity, and metrics.

The committed branch will then use the existing GitHub Actions push workflow
for full workspace tests and the research smoke workflow where applicable.
There is no scheduled nightly workflow. A CI failure must be fixed from its
specific logs rather than rerunning the full suite locally by default.

## 9. Rollout

The correction lands atomically: generator, partition version, active dataset
identities, regenerated goldens, consumer constants, and tests change in the
same branch. Partial compatibility with an active v1 dataset is intentionally
not supported. Documentation will record that v1 contained the price-scale
defect and v2 is the corrected baseline.
