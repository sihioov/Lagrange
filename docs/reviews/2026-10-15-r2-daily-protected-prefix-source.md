# R2 I1b-S5b — first protected prefix and in-memory candidate disposition

2026-10-15; R2-04/09. Direct coordinator inspection, not an independent review.
Prepared outside the frozen S5a candidate, then registered after full terminal
recovery. Executable evidence for this successor remains pending. The [bounded contract](2026-10-15-r2-daily-protected-prefix-contract.md)
defines the first historical continuation; later admitted daily parents remain work.

The only production constructor for ProtectedHistoricalPrefix borrows the existing
opaque ApprovedHistoricalPriceOnlyArtifact. It checks the embedded v3 registry
identity, all five pins against the structurally valid policy, and the approved
XKRX2452-session/26972-row complete sorted ETF11 grid. V2 and arbitrary caller bars
cannot create it. Approval, historical arithmetic, calendar bytes and D4 code are
unchanged. The policy hash binds context but is not runtime policy activation.

Reconstruction requires the exact previous frontier/policy and re-resolves every
discovered bar original against captured snapshots and identical FileEvidence.
It validates all rows using exact decimal strings/JSON integer tokens. Fractional
JSON numbers are rejected for arithmetic, independently of the unchanged legacy
normalizer's numeric document encoding. Scaled integer comparison checks overflow;
there is no float fallback or rounding. All raw OHLCV and optional trading-value
overlaps are compared; absent/zero differ. An in-range date absent from the protected
calendar conflicts. Another requested date's target observation must agree too.

Only exact11 selected target rows enter the addition; raw/adjusted fields are
identical in both numeric representation and scale. Historical fields are retained
unchanged. The NDJSON encoder uses the existing historical field order and LF,
instrument/date ordering and bounded writes. Session/row/byte caps reject rather
than truncate. Results retain the source and approved-base borrows, with private
construction and read-only access. No durable frontier, filesystem or DB authority.

Candidate construction checks every normalized descriptor against actual retained
document bytes and requires the complete unique descriptor set. It uses existing
D4 evidence/proof/draft/manifest canonical encoders; the producer commit is explicit.
Full base sessions plus the first next session, all five pins, policy, evidence,
bars and normalization hashes/counts are connected. First parent is null. There is
no caller-supplied manifest, clock, output UUID, admission hash or write capability.

21 private definitions are WRITTEN_UNEXECUTED:17 price/prefix/encoding and4 document
binding cases. They cover exact decimals/overflow/OHLC/volume/date/optional values,
every overlap field, protected session gaps, future-request target contradictions,
fixed policy/context, sorted complete grids, raw/adjusted preservation, literal
NDJSON/terminal-LF byte limit, duplicate rows and descriptor corruption/substitution/
omission/extra/order. The small private prefix is not an approved v3 capability.
Scoped standalone rustfmt2024 is the only local Rust tool used. No compiler, local
test/build/helper/parser, provider, DB, runtime action or delegation occurred.

Limitations: the real full approved-base constructor and complete snapshot-to-first-
candidate integration are not executed by these private fixtures. Full128MiB
allocation behavior is not claimed; a smaller exact byte seam exercises the same
writer. Non-Linux/native ACL behavior, protected admitted daily-parent readback,
sealed source frontier refresh/persistence, immutable artifact storage/admission/
supply, I3–I7/integrated independent review, runtime/Owner/three days and final C_R2
remain open. No source or executable acceptance is claimed before hosted recovery.

S5a-C01 changes only two positive fixture timestamps to the required nine-digit
fraction. The entire production observation region is byte-identical.285 of287
previous pins remain identical; only observation test literals and module registrations
change. New history.rs/candidate.rs have no dependency, workflow, SQL, provider
request or existing approval implementation changes. All carried regression names
are retained. The previous failed run and supplement remain immutable.
