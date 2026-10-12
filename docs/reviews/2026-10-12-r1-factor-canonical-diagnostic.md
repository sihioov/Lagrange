# R1 U1 canonical float diagnostic — 2026-10-12

**Cause proved:** exact-HEAD factor-engine canonicalization depends on serde_json's `float_roundtrip` feature. The historic golden is stable under the default parser; the simulated workspace parser preserves the computed float bits and produces the exact hash seen in the candidate failure. This is a latent package determinism defect exposed by dependency feature unification, not an introduced Rust lint patch regression. No production policy or source change was implemented.

Recommendation for coordinator decision: make roundtrip float decoding an explicit factor-engine dependency contract, retain strict verification, and correct the golden only together with that contract and a legacy synthetic readback regression. Compatibility evidence below supports this direction, with limits; it does not constitute overall R1 or production acceptance.

## Identity, isolation and resources

- Baseline: `ac0a08364788e6c2f234cf85025a8d975f362d31`, package boundary only. Every Cargo invocation ran from `/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/factor-diagnostic-ryvs3zjg/archive`, extracted from `git archive` of that exact HEAD. No shared-worktree build/test ran.
- Private evidence directory, abbreviated **D** below: `/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/factor-diagnostic-ryvs3zjg`. `head.tar` SHA-256: `835c5e944c17f3ca6a10ef4598efa10c532d6921f708c439d68d2426fb1c6d56`.
- Before archiving, 180 source/manifest/lock inputs were hashed against HEAD. Factor source, factor manifest, root manifest and lock match exactly; 11 pre-existing dirty kis-client/market-data inputs differ and were **not** used. The complete list/hashes are in `D/identity.json`, SHA-256 `7e3e1d7ad26db3e969d322eb8d1812e5fba0ced7263065c347b2746e33d9c3d2`.

| Input | HEAD/current SHA-256 |
| --- | --- |
| crates/factor-engine/src/owner_equity_v2.rs | `8b66e4ad73bd20774f38ea3f172d7d5a2f8aae2d9706b2851524098fe6c5456a` |
| crates/factor-engine/Cargo.toml | `d2f4b5fbe3ee5bc6d74cd5e524becb3f07caa36225899a48871f35dce2f4a4b6` |
| Cargo.toml | `cd6895e63076449191b96adcebac53050311ee2e7a1dd055cf623e386abd1f7b` |
| Cargo.lock | `d3528661c8ad9d15d4cc7b6d4f4f91e530c851770b8a79751c2c2a680645783e` |

- Initial resource check: 8,768 MiB RAM available, 1 MiB swap free; no Cargo/rustc/rustdoc appeared in the process-name check. Each run monitored MemAvailable and `oom_kill`, bounded to 600 seconds and the task's 30-minute deadline. No timeout, resource termination, OOM increment, permission retry or overlapping Cargo graph occurred. Final check: 9,080 MiB RAM available, 4 MiB swap free; no compiler remained. Compilation/test elapsed times are retained per receipt; the archive diagnostic/end-identity checks finished about 10.7 minutes after allocation.
- Safe child environment contained only actual HOME/RUSTUP_HOME/toolchain PATH plus the granted absolute CARGO_HOME/CARGO_TARGET_DIR, offline=true, jobs=2, incremental=0, test threads=2, color=never, private absolute `D/tmp`, and two synthetic diagnostic directory/label variables. No inherited DB/provider/credential variable reached Cargo. Rust toolchain pin is 1.97.1. Actual Cargo executable was `/home/l1nnx/.local/bin/cargo`, a symlink to the rustup Cargo shim.
- Cache writes were limited to the explicitly granted existing Cargo paths. No install/update/fetch, DB, Docker, npm, browser, provider or production action occurred. Source analysis and all archived test data used synthetic fixtures only.

## Executed command matrix

Every row is the exact argument suffix after `/home/l1nnx/.local/bin/cargo test -p factor-engine --locked`; cwd is the archive above. **G** = `owner_equity_v2::tests::golden_canonical_snapshot_hash`; **E** = `owner_equity_v2::tests::r1_canonical_export`; **R** = `owner_equity_v2::tests::r1_canonical_cross_read`; **U** = `serde_json/float_roundtrip,serde_json/raw_value`. Expand these literal aliases to recover exact argv; `D/run-inventory.json` also records each complete argv/environment-name list/elapsed time/features/exit/counts/log digest (SHA-256 `29cb0429dc477aefd510313172c4927dda7d1cc04188037f4e85ffa08f0c35cf`). Logs are `D/<label>.log`; complete per-run receipts are `D/<label>.json`.

| Label | Exact suffix | Exit | Passed/failed/ignored | Log SHA-256 |
| --- | --- | ---: | --- | --- |
| default-golden | `--lib G -- --exact` | 0 | 1/0/0 | `b57dbdc0a9a8521f5a4ff5fafce16099626566e0f1d4a6bca4c4f828c1bc845d` |
| unified-golden | `--lib --features U G -- --exact` | 101 | 0/1/0 | `1a5fb4b126c3ff7d8277e86c9a10d0a1286370b5b85597a0b738271a41fdc3f9` |
| default-lib | `--lib` | 0 | 28/0/0 | `a1b54478747835f5e74280a472de4a929bb51131ed79b262469b8c2bea7d0987` |
| unified-lib | `--lib --features U` | 101 | 27/1/0 | `d091c30da76d06f1457ea18519752ff7a1e24219b7762b62de03935c31252a5b` |
| float-only-golden | `--lib --features serde_json/float_roundtrip G -- --exact` | 101 | 0/1/0 | `84d4c04b9916e27102278ca4d57ed779167f9a7060436c92cafbc085df48b503` |
| raw-only-golden | `--lib --features serde_json/raw_value G -- --exact` | 0 | 1/0/0 | `0a9b7649e525ceb6bc2351c941733969a9ac4408c4f036c5077a2352e124e03b` |
| default-v1 | `--test fixed_stock_price_beta` | 0 | 11/0/0 | `e266397fbc8573380a0a3deb01b4b5117f503366b22ad324f2b29f6368996aa1` |
| unified-v1 | `--test fixed_stock_price_beta --features U` | 0 | 11/0/0 | `9a70244166994d9ff50aa74f209424e943ad6022300fbe20e5af84840a77580c` |
| default-capture | `--lib E -- --exact` | 0 | 1/0/0 | `f30aea0c5e767c8c4c2ee1266f61726f4bc8bbf7cfe6866ffb5c007012073770` |
| unified-capture | `--lib --features U E -- --exact` | 0 | 1/0/0 | `9987772c718ca27d621244aa9a5b0462ce75196936fdab736964c1f0dbfac348` |
| default-reader | `--lib R -- --exact` | 0 | 1/0/0 | `555f9ae857bcb82eb4d13ad5ae358605f0e7078be101dcf82ee7ae13aff79677` |
| unified-reader | `--lib --features U R -- --exact` | 0 | 1/0/0 | `51d6dfa6232724cd4e73913d1bdd4a36aae41c36432999af4a8edd4442c29748` |

Rows 1–8 used untouched exact-HEAD source. Rows 9–12 used only the allowed archived test instrumentation. Reader-test exit 0 means the observations were successfully captured; the matrix below deliberately contains a rejected cross-feature snapshot. No original test/golden was changed or suppressed. The three exit-101 results all identify the same original golden and exact workspace actual hash, not three distinct defects.

Actual Cargo fingerprints confirm default = serde_json **default/std**, unified simulation = **default/float_roundtrip/raw_value/std**, float-only = **default/float_roundtrip/std**, raw-only = **default/raw_value/std**. Factor-engine has no own enabled features in these runs. Original golden fingerprints are factor-engine `7b403ace642f5a67` / `29a7fb47d83af502`, linked to serde_json `914a381e80857d79` / `14e3d38a3352cba2`; `D/golden-feature-evidence.json` pins the dependency fingerprint values. This simulates the recorded workspace serde_json features; **it is not a new full-workspace run**.

serde_json is locked to 1.0.151. Its cached `.crate` SHA-256 `c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14` matches Cargo.lock; Cargo.toml, de.rs, ser.rs and number.rs match that cached archive byte-for-byte (`D/serde-source-identity.json`). The optional .cargo-checksum.json was absent, so verification used the pinned cached crate instead; no source/cache was changed.

## Exact byte/bit proof

Baseline compute at owner_equity_v2.rs:299–306 serializes computed metrics, reparses them, recomputes scores, then computes the self-hash; 327–333 hashes the serialized struct with content_sha256 cleared. serde_json's default and float_roundtrip paths differ at its de.rs:623–674. The initial metric/scoring JSON bytes are **identical** in the two feature selections. Only two of the 28 recorded row-float bit values change during default parsing:

| Synthetic row/field | Before parse / roundtrip parser | After default parse | Change |
| --- | --- | --- | --- |
| 000071.KRX volatility_60 | `3f8dbb1b711d9311` = 0.014517034902107125 | `3f8dbb1b711d9310` = 0.014517034902107123 | −1 ULP |
| 000070.KRX return_20 | `3fb99999999999a0` = 0.10000000000000009 | `3fb999999999999f` = 0.10000000000000007 | −1 ULP |

Roundtrip parsing changes **no captured bits or bytes**. Reassigning scores after parse changes no bytes in either selection for this fixture; row order, rank, score, other metrics, admission/source pins, warnings and schema/factor identifiers stay equal. Thus neither changed math nor provenance explains the hash difference. Raw_value alone retains the default hash; float_roundtrip alone causes the unified hash, isolating the causal feature.

| Producer | Self-hash-cleared byte SHA-256 (= content hash without prefix) | Full 4,178-byte snapshot file SHA-256 |
| --- | --- | --- |
| default parser | `2e3f4bb3a0a369a682e045a6c0375396d2c0292532df3a0479f04c28205b91bf` | `bcd6c1a7c35df5465a7892c7c3f9d65fe27efc82f6cf2e17518e5df467d44c33` |
| roundtrip parser | `b4b55daefb3fad4830b2dff9b75081a52ecd469de69a0dd2643a0f8b6b3846e5` | `7fcb5612515d94aff5fcf4f30eea6342f7f807229eb655245fd04d0285f67a3d` |

`D/bit-comparison.json` SHA-256 `321dbcf18090d6a0f32045b260eb9e6ee7342df7866ad8e69f668af513f78143` records exact bit/value comparisons. Captured files are `<producer>.{before-parse,after-parse,after-scoring}.{cleared,bits}.json` and `<producer>.snapshot.json`; the two producers are default-capture/unified-capture. These are new synthetic test outputs, never production records. No serialized body is copied into this report.

The historic golden is a default-parser result; it is **not feature-independent authority**. Full exact-HEAD lib runs demonstrate existing roundtrip/permutation/future-isolation/rank/pin checks pass in both selections, with the sole unified failure being the golden. The 11 existing fixed-stock V1 tests, including canonical JSON roundtrip, structural tamper, recomputed snapshot/manifest tamper and descriptor-safe storage checks, pass in each selection. That is same-selection evidence, not V1 cross-reader compatibility proof.

## Synthetic persisted-byte compatibility

Each generated snapshot was read under both parsers using (a) direct from_slice to the typed snapshot and (b) from_slice to Value, then from_value to the typed snapshot. Both routes gave the same result:

| Stored synthetic producer | Default reader | Roundtrip reader |
| --- | --- | --- |
| default | verify succeeds; serialized bytes unchanged | verify succeeds; serialized bytes unchanged |
| roundtrip | **SnapshotHashMismatch**; bytes change; recomputed hash becomes `2e3f4bb3…91bf` | verify succeeds; serialized bytes unchanged |

No reader repairs, rewrites, rehashes or ignores a stored hash. Hash, one-bit numeric and structural tampering were separately rejected by the archived export diagnostic under both selections. Cross-read receipts are `D/default-reader.cross-read.json` SHA-256 `45b35dac651b3fe88f9aa1b0c828e4e151c1188675072271d93495f6a329f58e` and `D/unified-reader.cross-read.json` SHA-256 `3bb611c1b62083e1f80de60705d878b9f4e45b608526ca48abef1d16939e1701`.

This proves forward readability of **this** historic default-produced canonical fixture by a roundtrip reader, and incompatibility of the reverse direction. It does not prove compatibility for every historic payload, another architecture/libm/compiler version, or actual PostgreSQL jsonb formatting/SQLx decoding. No existing committed Owner V2 full self-hashed canonical JSON fixture was found by tracked fixture/golden JSON searches; only generated Rust fixtures and the golden literal were located. No installed artifact or DB was inspected.

## Persisted consumers and consequences

- Factor verification at owner_equity_v2.rs:342–346 checks structure and exact self-hash, while 386–412 checks scores/ranks from decoded metric values. Those guards stay mandatory.
- Production wrapper at factor-engine owner_equity_v2.rs:430–442 feeds job-queue owner_equity_v2.rs:861. Worker publication at 1142–1184 stores a header plus **individual row JSON values**, not the complete self-hashed candidate/content_sha256. Row admission/lineage and publication transaction checks remain in force.
- API repository owner_equity_v2.rs:451–480 decodes `signals_json` through from_value and checks instrument/generation/rank. It does **not** read/reverify a full Owner V2 canonical candidate hash. ADR-0008:95–115 likewise specifies immutable header/rows and the active-ready universe hash. This narrows the earlier U1 hypothesis about persisted full-candidate verification; do not invent a DB rehash/migration requirement.
- A float feature pin makes newly computed Owner V2 metric bits/hashes independent of whether an unrelated crate enables this serde_json feature. New computation may differ from a historic default-only build by the demonstrated last bits/hash. Old stored rows must remain immutable; no rehash/rewriting or fallback acceptance is proposed. Public full-candidate readers using the old parser can reject new snapshots as demonstrated, even though the current DB path does not store that aggregate hash.
- Cargo dependency features unify across selected packages. A factor-engine pin can also change other serde_json consumers in smaller binary selections; the current complete workspace already enables float_roundtrip through jsonschema. V1 same-selection tests passed, but no universal cross-feature compatibility claim is warranted. Candidate full-workspace/default/all-feature/mixed matrix validation is still required after any adopted repair.

## Minimal proposed policy scope — coordinator decision required

Prefer explicitly enabling `float_roundtrip` on factor-engine's existing serde_json dependency, **without adding raw_value**, changing parser/math code, weakening verification, changing schema/factor versions or implementing a legacy fallback. This selects the parser that preserves the original computed bits and accepts both demonstrated old/new byte streams. Retaining the old implicit approximation would require an explicit separate canonical parsing/quantization design to withstand feature unification; deleting the roundtrip normalization alone would not fix decoding by old readers.

If adopted, the bounded future implementation allowlist is:

1. `/data/worktrees/3puw275b/orange-dugong/crates/factor-engine/Cargo.toml`, existing serde_json dependency at line 17: declare `features = ["float_roundtrip"]` with version unchanged. No lock update or production API/feature surface addition is needed.
2. `/data/worktrees/3puw275b/orange-dugong/crates/factor-engine/src/owner_equity_v2.rs`, test section around 1115–1137 only: update the golden to the now-proved `b4b55dae…846e5` **as part of the adopted feature contract**, add legacy fixture byte-identical read/self-verify assertions and multi-row current-byte roundtrip/tamper checks. Keep the original math, serde normalization, strict hash/pin/rank guards and all existing assertions/test names. Do not merge archive instrumentation into production.
3. New `/data/worktrees/3puw275b/orange-dugong/crates/factor-engine/tests/fixtures/owner_equity_v2_legacy_default.json`: freeze exactly `D/default-capture.snapshot.json` (full-file digest above), explicitly labeled synthetic exact-HEAD/default-produced fixture. Preserve its original `2e3f4bb3…` self-hash and assert its exact byte digest; never regenerate its expectation through the same computation under test.

Meaningful verification after adoption: full factor lib default and direct-dependency unified selections must agree on the new golden; legacy readback must retain old bytes/hash and reject hash/numeric/structure tampering; existing permutation, future-row, rank/pin and V1 tests must remain. Then reserved workers run the complete candidate workspace and strict feature matrix. No production/schema/DB change is justified by this diagnostic alone. Coordinator decides whether the measured legacy coverage suffices for this release or requests additional operator-provided historical canonical fixtures; absence must remain visible, not be papered over by a hash update.

No architectural edge changed. Proposed manifest feature selection keeps the same dependency; proposed tests occur below production definitions, so existing diagram evidence into consumer imports is unchanged. No diagram/ROADMAP/other-report edit was made.

## Archive patch, final state and limitations

Allowed instrumentation added test-only capture calls at archived lines 304/308/311, helper functions at 452–484 and diagnostic tests at 1175–1225. Original source and instrumented copy are `D/owner_equity_v2.rs.original` / `D/owner_equity_v2.rs.instrumented`; the latter SHA-256 is `5732e05c5b310c384810281dbd1301e3b78e40465f66c3267d755ffcdca1e97a`. Exact retained `D/archive-instrumentation.patch` SHA-256 `b17ec5daa5b79e6bd1153da3acaba4bb390e2bf2c4e61d07952903cfa7f9d4aa` contains all changes. The original golden remained unchanged in every run. Instrumentation used the existing two-symbol fixture, recorded no other input and contained no provider/DB call.

After execution, the archived source was restored byte-for-byte; all **1,771** regular files in the archive were compared to head.tar with zero differences. Real factor source/manifest also still match their starting hashes. `D/end-snapshot.json` records this check; no worktree implementation or unrelated concurrent edit was overwritten. All 12 owned Cargo children completed, none timed out, and final compiler/process checks found no owned work remaining. Private evidence is retained.

Changed tracked file: this new report only, lines 1–112. Actual checks: exact-HEAD archive/hash comparison; resource/process checks; 12 serial package-only Cargo commands above; fingerprint/cache-source verification; archived byte/bit/cross-reader/tamper capture; targeted source/fixture/ADR reads. Deviations: none from the authorized execution boundary; float-only/raw-only controls and V1 checks supplied focused causal/compatibility evidence. No full workspace, current-candidate source build or baseline DB execution was attempted.

Not found: committed Owner V2 full canonical persisted fixture; optional checksum metadata (resolved via the cached crate). Not verified: real historic payload/DB compatibility, V1 cross-reader compatibility, other platforms/toolchains, candidate integration and production behavior. Remaining decision: adopt the proposed explicit roundtrip policy and measured compatibility envelope. Root cause is closed; no further diagnostic decomposition is necessary unless broader historic compatibility proof is requested. **HEAVY_SLOT_RELEASED — actual idle, no owned process.**
