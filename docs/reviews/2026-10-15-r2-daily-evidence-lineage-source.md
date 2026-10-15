# R2 I1b-S3 — lineage/calendar source disposition

2026-10-15; Wave41; R2-04/09; baseline440d4a8.
The bounded candidate implements the [S3 contract](2026-10-15-r2-daily-evidence-lineage-contract.md).
This is coordinator source inspection, not independent review or executable
acceptance. All22 new Linux definitions are **WRITTEN_UNEXECUTED**.

## Concrete source

| Boundary | Implementation and disposition |
| --- | --- |
| Cross-scope capture | [Borrowed scopes](../../crates/market-data/src/daily_price_only/verify.rs#L87) accept one or two distinct S2 snapshots with a common explicit UID. No filesystem API or lock acquisition is added. Entitlement and observation are supplied context, not new authority. |
| Original/copy chain | [Resolver](../../crates/market-data/src/daily_price_only/verify.rs#L166) performs exact lookup at every hop, checks credentialed batch/request modes and entitlement, compares bytes and metadata, orders timestamps, rejects cycles/future retrieval, and bounds links at16. Actual uncopied termination is mandatory. |
| D4 reuse | [Internal adapter](../../crates/market-data/src/daily_price_only/contract.rs#L1558) canonicalizes and validates the existing FileEvidence type with the unchanged encoder. Two existing helper visibilities are widened only within this module. No public wire format changes. |
| Provider JSON | [Bounded parser](../../crates/market-data/src/daily_price_only/verify.rs#L303) rejects duplicate decoded keys before the second value, trailing input and depth overflow. Error codes have no source/body/message payload. Byte limits are D4's existing source limit. Object sorting is independent of serde_json map-order features. |
| Calendar projection | [Projection](../../crates/market-data/src/daily_price_only/calendar.rs#L161) checks the terminal original target/request, all rows and unique dates; preserves original ordinal and full row hash; enforces SQL0060 row/span limits. Response marker M remains metadata. No follow-up request exists. |
| Historical mapper | [Pure field extraction](../../crates/market-data/src/normalize.rs#L1209) preserves generic filtering, output creation and exact error fields. The existing normalizer still filters surrounding dates at its previous point; the daily verifier independently checks all dates. |
| Deterministic documents | [Document](../../crates/market-data/src/daily_price_only/calendar.rs#L98) uses D4 FileKey commitment and A6 sorted JSON, one exact day row, no normalized Raw batch, UUID generation, time sampling or invented open/close timestamp. |
| Continuous facts | [Range proof](../../crates/market-data/src/daily_price_only/calendar.rs#L298) preflights claim counts, rejects cross-Owner/policy sets and every conflicting supplied fact, preserves sorted equivalent witnesses, and requires each requested civil date. Expected session checks the explicit16:30 KST cutoff and refuses a later source observation. |

## Written evidence and preserved inputs

[22 named fixtures](../../crates/market-data/tests/daily_price_only_evidence.rs#L250)
cover original/cross-scope copy commitments; exact reference failures; cycle and
16/17 links; every-hop mode/entitlement; copy differences; retrieval order/future;
context and exact request; missing target; every surrounding row/duplicate date;
JSON duplicates including escaped aliases and malformed second values; trailing
and32/33-depth boundary; safe errors; object output/future calendar rows;370/371
limits; nested row sorting/array order; golden source/document bytes; equivalent
witnesses/claim cap; conflicts outside requested range; weekend gaps; cutoff;
identity/range/no-trading; and read-only fingerprints with both existing locks.

The private Wave41 definition inventory also fixes37 existing normalization and
calendar-publication names. Earlier99 Rust,32 daily SQL/73 subcases,28 affected
legacy DB,6 separate chart,3 transport and83 bootstrap cases retain their prior
inventories and must be reconciled on the new candidate. Existing Linux platform
exclusions and unprinted nested helper rows/exits remain distinct.

All247 unchanged Wave40 pins match; the only prior pinned target changes are
contract.rs and module registration. Snapshot source/test, storage, migration0060
UP/DOWN and workflow are byte-identical to440d4a8. Generic normalizer changes are
the field extraction only. No Cargo dependencies or legacy tests change.

Scoped standalone rustfmt edition2024 and whitespace checks passed. No local
Cargo/compiler/Clippy/tests/Raw parser/DB/helper/validator/provider execution
occurred. Frozen inputs pin the final six source files, inventories and references.
After all-event exact-head existing-run query and remote SHA match, one hosted
attempt may verify this candidate. No old run is rerun, canceled or duplicated.

## Limits

The fixtures are synthetic committed storage, not live supplier or installed ACL
evidence. Original metadata is batch precision; no per-file timing is invented.
This slice does not verify bars/reference/action rows, seal a complete source
frontier, grant artifact admission, write SQL projection or start collection.
The parser has byte/depth limits, not a guarantee against every allocator failure.
Range proof results are civil facts, not admission or scheduling authority.
Only the later trusted pipeline can bind supplied context to DB/installer facts.

I1b remaining reconstruction/artifact/admission/supply, I3–I7, final integrated
independent review, runtime UID/ACL, installer filesystem rollback, diagrams
before main, Owner and three trading days/final C_R2 remain open. I2's accepted
dee78c4/run38696988981 evidence and all older failures remain unchanged.
