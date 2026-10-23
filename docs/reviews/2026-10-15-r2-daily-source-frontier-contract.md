# R2 I1b-S5 — integrated discovery and source frontier

S5a source boundary, 2026-10-15. S4 candidate48beb25/CI38722595047 was fully recovered: tests and PG pass, strict lint fails on one fixture type. C01 candidate302f29a/run38725299504 is fully recovered and accepted for S4 only. S5a now integrates discovery/observation; protected-prefix and storage/admission work remains separate. This preparation is separate from its frozen candidate.

## Next concrete boundary

Build the D5 verify_daily_sources seam over complete S2 snapshots and the S3/S4
pure reconstruction capabilities. The caller supplies trusted Owner/policy and a
requested extension, not an authoritative list of files. No provider request,
filesystem write, DB mutation, scheduling eligibility or admission is granted.
New frontier.rs and its dedicated integration fixtures may own the orchestration;
observation.rs owns the fixed D5 operational observation/event encoding, without
claiming source existence or admission. It can be prepared as an unregistered
module while S4 runs, then registered and tested within the integrated S5 candidate.
narrow sequential adapters in calendar/reconstruct/verify/contract remain possible.
Do not edit S4 while its current hosted candidate is unresolved.

1. Enumerate relevant committed bars/reference/calendar and complete action bundles.
   Resolve every selected or competing file through one trusted RawEvidenceSnapshots
   context. S3 validates all copies, entitlement and times. Action discovery uses
   requests rather than partition dates. A producer subset cannot hide a conflict.
2. Bind a continuous calendar proof to the exact extension. The next session is its
   sole TRADING date; intervening days are CLOSED. Preserve all equivalent source
   observations even where D4 permits only one selected calendar binding per date.
   Calendar source selection follows A2's evidence-hash order and carries the exact
   reconstructed document. Legacy projection is absent unless independently proven;
   never fabricate its separate normalized hash/DB tuple.
3. Check every observed bar row against any supplied verified historical/daily
   prefix before accepting the new target. S4 currently validates surrounding rows
   but returns only its target; a narrow pure observation adapter is needed. The
   approved v3 raw/adjusted semantics must survive. Caller-provided arbitrary bars
   cannot stand in for the protected base or an admitted prefix.
4. Reconstruct all four kinds, assemble a canonical D4 SessionEvidence and bind
   every document's bytes/count/source keys. D4 still requires exactly11 bars and
   references. Do not silently discard competing identities to satisfy that shape;
   disposition of multiple eligible bar/reference sources must be explicit before
   implementation. Calendar's one-binding rule and A2's multiple witnesses likewise
   need a concrete representation preserving both requirements without new D4 fields.
5. Retain an operational observation of all relevant FileEvidence, independent of
   the selected D4 sources. D5 fixes observation bytes: prefix
   owner-beta-daily-source-observation-v1 followed by LF and the compact array of
   canonical FileEvidence sorted by full bytes; only byte-identical duplicates may
   be removed. This enables deterministic conflicts before a candidate/admission.
   Match SQL0060's canonical observation contract; do not hash free-form errors.
6. Compare a fresh complete observation to the protected sealed one. Exact evidence
   replays. Added alternatives touching sealed dates block even with equal numbers;
   disappearance or changed identity/bytes blocks. Unrelated provider/date changes
   do not invalidate the relevant frontier. Never reseal a historical date merely
   because code version, current time or release changed.

## Evidence and authority

VerifiedDailyEvidence must have private construction and retain snapshot lifetime.
It is not an admitted artifact. Separate later storage/admission code reopens the
protected base/prefix, independently verifies and copies bytes, installs with durable
no-replace semantics, and commits SQL0060. Only that boundary creates an admitted
handle. A source-frontier record loaded from caller JSON is not trusted provenance.

Expected conflicts use D5's finite codes and deterministic observation/event hashes.
A parse/IO/gap failure without attributable FileEvidence produces only a typed poll
failure; do not invent an event identity. SQL adapters must commit BLOCKED outcomes
and read a receipt after uncertain commit replies. These adapters are subsequent
work, not a claim made by this pure source slice.

Required fixtures: complete discovery versus a malicious producer subset; alternate
equal/conflicting bars and calendar witnesses; missing/changed sealed sources; unrelated
append; release-only replay; full-row overlap against the approved prefix; calendar
gaps/conflicts/intervening TRADING; deterministic FileEvidence observation/event
preimages; exact D4 session/doc commitments and private-borrow lifetime. Resource
limits reject incomplete scans. Source/CI acceptance is pending implementation.

## Concrete discovery disposition — source preparation

D4 v1 has exactly one bar/reference file per symbol and one calendar binding per
civil date. The new discovery function therefore rejects duplicate target price/
reference deliveries, including equal alternatives, instead of choosing a source
whose competitors cannot be represented. Calendar selects A2's smallest evidence
hash and retains all equivalent projections in the operational observation; its
selected one-file D4 document remains unchanged. Complete action witnesses follow
S4. This changes no D4 schema or source-acquisition policy.

The request contains Owner/policy/previous/target only, with no producer source
list. Discovery enumerates both captured scopes, resolves original date/copies,
reconstructs all four kinds, checks the sole next trading session and produces
structurally validated D4 session evidence. All captured daily bar rows, including
surrounding and other requested dates, are retained for the subsequent protected
prefix comparison. A bounded scan/parse budget fails closed. The result is named
DiscoveredSessionSources: it does not claim protected-base/prefix verification,
sealed-storage provenance, artifact approval or SQL authority.

Operational observation comparison is pure over canonical claims. Exact repeats
replay; additions, conflicting variants and removal (including all files missing)
produce deterministic SEALED_SOURCE_CHANGED evidence from the expected/observed
union. Only the later protected store/admitter may treat expected claims as a seal.
No release/time/random value changes identity. The two modules are unregistered
preparation until S4's frozen candidate is fully recovered. Tests, registration,
protected-prefix verification and integrated error/event disposition remain work.

## Wave44 S5a integration

After S4 full recovery, the two modules and33 written tests are registered. Existing
verify.rs adds only module-private accessors for the same captured scope references
and observation instant. Discovery has no producer source-list parameter. Structurally
canonical unrelated six-digit non-ETF price identities stay outside fixed ETF11;
relevant price duplicates fail closed. A source count cap and128MiB aggregate parsed
price/calendar bytes reject incomplete scans. Observation budgets include the exact
prefix/array/separators and canonical input bytes before deduplication; exceeding
16MiB or262144 claims fails. The byte-bound test uses the independently pinned
1005-byte example through a private lower-bound seam; no production limit is changed.

This slice does not close the full S5/frontier requirement: protected expected-state
loading, raw/adjusted base-prefix overlap comparison, attribution of discovery
failures to durable events and artifact/admission transactions remain subsequent
work. No source/runtime acceptance is claimed before exact-candidate CI.
