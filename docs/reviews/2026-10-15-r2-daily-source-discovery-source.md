# R2 I1b-S5a — source discovery and observation disposition

2026-10-15; R2-04/09; baseline4143051 after accepted S4 candidate302f29a.
Coordinator direct inspection, not an independent review or execution claim.
The [source boundary](2026-10-15-r2-daily-source-frontier-contract.md) preserves
D4 encoding, S3 original/calendar verification and S4 reconstruction.

Five source/test paths: new discovery.rs, observation.rs and integration fixtures;
verify.rs adds only two module-private getters; mod.rs registers the modules.
All274 other prior pins match. Existing verifier bytes are identical after
removing the two getters. Snapshot, normalizer, provider reconstruction, SQL,
workflow/dependencies and old tests remain unchanged.

Discovery enumerates both complete captured scopes, resolves actual originals,
filters the fixed ETF11 identities, and reconstructs exact11 bars/reference plus
complete action bundles. Duplicate target bar/reference deliveries, equal or
unequal, are rejected rather than silently selecting an unrepresentable D4
alternative. Every captured ETF daily bar row is validated and retained for later
protected-prefix comparison, including surrounding/other requested dates.

The calendar proof covers every civil date, requires only the target to be
TRADING and selects A2's smallest evidence hash. All equal projections remain
operational witnesses. Selected D4 bindings/documents contain no fabricated legacy
projection. The session evidence preimage is validated by the existing D4 encoder.
Each document retains bytes and descriptor; source borrows retain snapshot locks.

Observation encoding sorts full canonical FileEvidence bytes and removes only
byte-identical repeats. The exact prefix/compact array/no trailing LF and event
prefix/Owner/kind/code/range/hash/final LF are pinned with literal golden bytes.
Changed evidence unions expected and observed variants, including total source
disappearance. Replay uses exact canonical bytes. Context is validated on replay;
release, time, random ID and free-form messages never become event identity.
These are claim encoders/comparators, not source existence or trusted seal loaders.

33 definitions are WRITTEN_UNEXECUTED:16 private observation/event and17 Linux
snapshot/discovery cases. They cover byte ceilings, ordering/duplicates, metadata/
time/copy identity, invalid claims/context, disappearance/mutation/equal alternatives,
complete D4 documents, hidden price/reference alternatives, missing rows, calendar
gaps/intervening TRADING/conflicts, equal/cross-scope copied calendar evidence,
unrelated valid appends/non-ETF records, every surrounding bar, manifest order,
cutoff/bounds, action gap/noncash, read-only fingerprints and shared-lock lifetime.
No local compiler, Clippy, tests, Raw parser or project helper ran. Scoped standalone
edition2024 formatting and whitespace pass; strict lint and hosted outcomes pending.

Remaining limitations: protected v3/daily prefix reopening and overlap comparison,
protected frontier persistence, typed durable event attribution on discovery
failures, artifact/admission/supply, I3–I7/final independent review and actual runtime/
Owner/three-day evidence. The parse aggregate cap is source-inspected, not exercised
at full128MiB; non-Linux snapshot execution is not claimed. Pure source claims
cannot approve an artifact. Ralph remains active and final C_R2 remains open.
