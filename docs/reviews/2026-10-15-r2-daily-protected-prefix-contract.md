# R2 I1b-S5b — protected historical prefix and exact price continuation

Prepared outside S5a candidate5a83e0d/run38728478625; that run is now fully recovered with two fixture failures. This successor
owns new history.rs/candidate.rs and their private fixtures; registration follows
full CI recovery, which is now recorded in the S5a CI report. Candidate assembly is the first in-memory artifact slice, bundled
with this prefix boundary to verify their connection in the same candidate.
The coordinator implements and reviews directly. No new worker or local execution.

The public base constructor requires the existing opaque
ApprovedHistoricalPriceOnlyArtifact. Its embedded approval-registry hash must be
the v3 registry hash, and the complete five pins must match the D4 policy. A caller
Vec, parsed wire manifest, v2 approval or arbitrary synthetic prefix cannot create
this capability. Independently check the fixed2452-session ETF11 grid against the
existing approved XKRX calendar, strict instrument/date ordering, complete26972rows,
and preserve each approved raw/adjusted field without coercion.

The first continuation must bind that exact frontier2026-08-28 and the same policy
hash. Re-resolve every discovered bar source in the captured scopes and require
identical FileEvidence. Inspect every original provider row, including surrounding
dates and other requested dates. Within the protected range, a missing session/key
or any changed raw OHLC, volume or optional value blocks the continuation. A later
row is not added merely because it appears in an earlier response. Only the11
exact target rows selected by the independent discovery enter the new session.
Target observations returned for another requested date must also agree with the
selected target's exact raw values. Retain at most one identical target value per
ETF identity and reject conflicting observations, instead of silently ignoring them.

Arithmetic comes from the original decimal strings or exact JSON integer tokens,
not the legacy normalized JSON Number/f64 representation. Reject numeric fractional
tokens, exponent strings, negative/zero prices, fractional/overflow volume, negative
value, invalid OHLC and unsupported precision. Exact scaled-integer comparisons
must fail closed on overflow; no float fallback, rounded rescaling or inferred zero
for a missing optional value. Original monetary strings remain independently bound
by Raw evidence. This does not change the existing normalizer's document bytes.

The result has private construction and retains both the approved base and captured
discovery lifetimes. Added raw and adjusted OHLC are exactly equal. Encode full bars
in the existing historical field order, instrument/date order, decimal strings and
one LF per row, preserving every base row's existing representation. Enforce128MiB
and20000session bounds before returning complete bytes, never a partial encoding.
The result is reconstructed evidence, not an admitted handle, trusted policy,
filesystem seal, SQL receipt or provider authorization.

The candidate builder accepts only that retained extension. It checks the complete
normalized-document descriptor set against exact reconstructed bytes, including
source keys, hashes, sizes and counts. It uses D4's existing canonical evidence,
normalization proof, draft/content hash and final manifest encoders unchanged.
The first parent is null; sessions are the full approved grid plus the sole proven
next session. The producer commit is explicit, and no sampled time/current HEAD/
random ID enters the bytes. Candidate accessors expose bars, normalization and
manifest bytes/hashes, never an admission hash or approved handle. No writes occur.

Fixtures must independently cover all raw fields and optional-value distinctions,
exact large-decimal comparisons, integer and resource boundaries, invalid rows at
non-target positions, missing/extra protected sessions, input order/duplicates,
historical adjusted preservation, new raw=adjusted, explicit context/policy binding
and deterministic sorted NDJSON bytes. A private small-prefix fixture cannot prove
the live approved v3 artifact; the production constructor must retain the full
embedded approval/calendar checks. All tests remain written/unexecuted until CI.
The real full-base approval/factory and complete snapshot-to-extension call are
not exercised by these private arithmetic/encoding fixtures; they remain explicit
integration requirements, including full first-candidate assembly. The four document
binding fixtures check corruption/omission/extra/duplicate/descriptor substitution
and order only, not a complete approved artifact. No synthetic Approved handle or
production bypass exists.

Later work still needs protected daily-parent readback and prefix reuse, source
frontier persistence/reopening, complete artifact/admission/supply, I3–I7 and the
runtime/Owner/three-trading-day/final C_R2 gates. This first-extension proof does not
claim those later capabilities or weaken their completion criteria.
