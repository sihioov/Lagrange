# R1 workspace coordinator corrections — 2026-10-12

Source changes on the dirty candidate based on `ac0a08364788e6c2f234cf85025a8d975f362d31`.
Compilation and dynamic acceptance remain pending. No production, provider or database operation ran for these edits.

## F2: immutable calendar fixture and freshness regression

The [follow-up specification](2026-10-12-r1-workspace-followup.md) mistakenly said to retain final unavailable/replay assertions. The current test ends after the first real-EOD freshness assertion. The implementation worker correctly left this family unchanged. The coordinator resolves this by preserving both existing freshness assertions and adding explicit replay coverage.

Only `crates/api-server/tests/risk_snapshot_seam.rs` changes:

- Prepare the Jan31 verified Raw bundle before the Feb3 unavailable bundle. Assert the calendar files have equal hashes, sizes, bytes and facts.
- Assemble a sink input that reuses the actual Jan31 Calendar PublicationFile, storage path, CalendarEvidence and facts. Keep the unavailable bundle's other files and its own batch/date/retrieval identity. Both original Raw roots remain alive. This assembled sink fixture is explicitly distinguished from the original `from_raw` output; no Raw bytes or manifests are edited.
- Publish unavailable evidence first and retain `DataFreshness::Unknown`; publish the earlier real EOD and retain `Age(259_260)`.
- A third independently committed, unassembled unavailable batch has a fresh batch ID and conflicting calendar evidence. Require typed Conflict and unchanged publication/history/projection counts. The fresh batch reaches calendar identity validation without first colliding with an existing batch replay. Positive nonzero counts prevent vacuous empty-count coverage.
- Replay the assembled unavailable publication after the real EOD. Require AlreadyPublished, unchanged counts and unchanged real-EOD freshness.

The added replay and conflict assertions are coordinator-specified follow-up coverage, not pre-existing assertions. No provider scope, sink validation, provenance identifier or timestamp is rewritten. Other Raw fixture callers remain unchanged. Validate the complete `risk_snapshot_seam` and `publication_sink` targets in the disposable QA slot.

## U1: explicit roundtrip parser contract

The coordinator accepts the [diagnostic recommendation](2026-10-12-r1-factor-canonical-diagnostic.md). Exact-HEAD default/unified feature experiments and byte/bit receipts prove that implicit serde_json feature selection changes canonical metrics and hashes. Direct and Value-mediated synthetic compatibility checks show the roundtrip parser accepts both recorded old/new snapshots unchanged; the default parser rejects the roundtrip-produced snapshot.

Enable only `float_roundtrip` on factor-engine's existing serde_json dependency, with the same version. Preserve all production math, serde normalization, validation, schema/factor versions and stored data. Update the synthetic computation golden together with this explicit dependency contract. The complete workspace already enabled this parser through another dependency; isolated binary/package builds now select it consistently.

Freeze the exact 4,178-byte synthetic default-parser snapshot from the diagnostic in `crates/factor-engine/tests/fixtures/owner_equity_v2_legacy_default.json`. Its full-file SHA-256 is `bcd6c1a7c35df5465a7892c7c3f9d65fe27efc82f6cf2e17518e5df467d44c33`; its old self-hash remains `2e3f4bb3a0a369a682e045a6c0375396d2c0292532df3a0479f04c28205b91bf`. The new legacy readback regression pins both, checks byte-identical direct and Value-mediated decoding, and retains strict verification. Extend current roundtrip coverage to the two-row golden as well as the original one-row case. Both fixtures reject changed hashes, one-bit numeric changes and structural tampering even when the latter is rehashed.

This is measured synthetic compatibility, not proof for real historic payloads, PostgreSQL JSONB roundtrips or every platform. The current V2 DB path stores headers and individual row JSON, not the full self-hashed candidate; no DB rehash/migration is introduced. No committed historical full-candidate fixture was found. Production acceptance remains separate.

## Source integrity and validation

| Source | SHA-256 after edit |
| --- | --- |
| `crates/api-server/tests/risk_snapshot_seam.rs` | `8b259ee17fdb00569353126384604ce05cf4e07bb64465780f268492f526ba9c` |
| `crates/factor-engine/Cargo.toml` | `15e4372aa3a537677d14a011ea7dc7d659e9e2f2d4d13873ee483fcd75d10dfa` |
| `crates/factor-engine/src/owner_equity_v2.rs` | `2c8efca96f822a358e7352cbbfd91c1118ab68809587a5e323eb6496bc4d92a9` |
| `crates/factor-engine/tests/fixtures/owner_equity_v2_legacy_default.json` | `bcd6c1a7c35df5465a7892c7c3f9d65fe27efc82f6cf2e17518e5df467d44c33` |

Source hashes matched the diagnostic's initial factor source/manifest hashes before editing. Cargo.lock remains `d3528661c8ad9d15d4cc7b6d4f4f91e530c851770b8a79751c2c2a680645783e`. Scoped rustfmt and whitespace checks passed; no compiler/test was run during the concurrent cold Raw validation. No source instrumentation from the private archive was copied into production.

F6 moved existing `runtime_deployment.puml` evidence; the coordinator updated the citations to state-directory creation at1738 and anchor creation at1524/1758/1765. Topology is unchanged. Both diagrams were locally rendered after the Raw worker released its slot, using the cached PlantUML image (`docker run --rm --pull=never … -tpng /data/component_architecture.puml /data/runtime_deployment.puml`), exit0. Component PNG SHA256 remains `6cde6321983f89dcacd1225eb6009c8f7e66346eb9f2cf374fc2e0b7f345e9dc`; runtime PNG is `29b7293c0e5d4ec67410774b06076ffb231fefbc917c05f4e9091c3fa652551c`. The U1 feature selection changes no dependency edge or existing production source citation.

Independent source review passed ([report](2026-10-12-r1-workspace-repair-review.md), line 130 onward); it verified all four frozen hashes and found no blocking defect. Pending: factor full-lib/default/unified/V1/legacy/tamper validation; F2 disposable DB targets; all other corrected workspace families and the final exact workspace/strict matrix. No staging or commit was performed.

## F3 follow-up: future-session recovery uses the same QA port

The corrected-environment run passed the focused factor, KIS, normalization, intraday and API selections, including F2. It stopped at `worker_price_recovery_rejects_future_sessions_before_catalog_publication`: expected Permanent, observed Retryable. The other 69 research-worker tests and all 20 publication-sink tests passed. The original failure remains in the [validation report](2026-10-12-r1-workspace-validation.md).

Source inspection found that this sixth DB-dependent worker test checks the exact QA URL on port55438 but never overrides `worker_config`'s port55432. The other five connection configurations had been updated during F3. The coordinator adds only `("DB_PORT", "55438")` to this test's existing overrides at line2369; pure configuration defaults and all assertions remain unchanged. This proves a configuration mismatch, not the historical returned error variant, which was not captured. The corrected focused execution must establish whether this resolves the observed Retryable result.

`data-pipelines/collectors/tests/research_worker.rs` now has SHA256 `a2f4bdb5e31a1b1086694c168cb4880712e9d77009d1163d81e7822b464e0c00`. Scoped rustfmt check and repository diff check exited0. The heavy slot was reassigned at11:53 UTC for the exact failed test, full research-worker target and remaining job-queue/chart/workspace/strict selections, with a fresh isolated QA project and the already validated Python/TMPDIR environment. Other passed selections are retained; no redundant broad rerun was requested before the final workspace gate.

The corrected exact test passed1/failed0/ignored0, and the complete target passed70/failed0/ignored0. The coordinator inspected both completed logs under `target/r1-runtime-fxi6ltmv/workspace-continuation-20261012-01/logs/`: `research-worker-future-session-exact.log` SHA256 `1f69f68f6db3f5313b3eb1bfaa3215ca23f3f93039fdaa19c6f18499c870c780`, and `research-worker-complete.log` SHA256 `399e4cb05a45b8a0d238fef4c703d775588a1e970ff7d38cfa6565036773f023`. This confirms the scoped port correction resolves the failed assertion while preserving the no-price-publication check. Recommendation targets subsequently passed42 and the chart target passed6, with no failures/ignored cases. Full workspace and strict results remain pending at this checkpoint.
