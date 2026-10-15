# R2 I1b-S3 — original lineage and calendar reconstruction boundary

2026-10-15; roadmap R2-04/09; baseline `440d4a813d2c9048847e6127f38bdcb2945ca9ad`.
This bounded implementation follows D4 and D5 A2/A6. It does not complete I1b,
grant admission, activate collection, or begin the three-trading-day observation.
The accepted S2 snapshot and I2 migration0060 remain unchanged.

## Concrete scope

The actual calendar copy path crosses `kis-calendar` and `kis` scopes. A borrowed
`RawEvidenceSnapshots` set therefore contains at most those two existing complete
snapshots, with the same explicit trusted Raw UID. It adds no file opens or locks.
Its expected entitlement reference and observation instant are explicit context,
not rights approval, installer attestation, or an independently trusted DB clock.

`resolve_original` follows exact committed references, including reference hash
and retrieval time, until an actual uncopied file. It validates every batch/file
mode, entitlement, copy identity, byte/hash/size/kind/request/continuation equality,
nonincreasing retrieval times, no future retrieval, no repeated identity, and at
most16 links. Provider and batch date may change across a copy. It retains exact
metadata commitments and the recorded batch retrieval precision. No generated
normalized UUID, wall-clock lookup, filesystem write or provider call is used.

Calendar projection validates D4's exact credentialed first-page request against
the terminal original date. A duplicate-rejecting, depth32, byte-bounded JSON read
requires `rt_cd="0"`, an object/array output, every row's valid date and Y/N flag,
unique dates and exactly one originally requested target. The SQL0060 limits are
also enforced:1..370 rows, span less than370 civil days, unique original ordinals.
Surrounding/future dated rows are evidence; future retrieval is rejected. Response
continuation is preserved but does not authorize pagination.

The existing calendar field mapper is extracted without changing generic
normalizer filtering, output bytes or errors. Daily projection validates all
dates before filtering. Row hashes use all parsed original row fields with
recursive key sorting; arrays and scalar values remain ordered/preserved.
Projection identity uses A2's Owner/policy/version domain and canonical D4
FileEvidence. One-day reconstructed documents use A6's exact sorted JSON fields
and D4 source commitment. Their constructors remain private and convey no approval.

Explicit range proofs require every civil date, retain equivalent witnesses
sorted by evidence hash, and reject any conflicting fact in the supplied source
set. They do not guess weekdays or select a newer conflicting observation.
Ranges are limited to370 days; total supplied row claims are limited to D4's
262144 evidence cap before witness construction. Expected-session selection uses
an explicit clock instant: eligible today from16:30 Asia/Seoul, otherwise the
previous civil day. The caller supplies the beginning of the continuous proof;
no missing interval or absent trading row is guessed.

## Ownership and verification

New files: `verify.rs`, `calendar.rs`, `tests/daily_price_only_evidence.rs`.
Narrow existing changes: private D4 canonical/validation access, pure calendar
field extraction in `normalize.rs`, and sequential module registration.
No snapshot/storage/SQL/provider/workflow/legacy test changes are intended.
Errors are fixed fieldless codes; provider bodies/messages never enter errors.

Coordinator source disposition and named adversarial fixtures must precede one
new exact-candidate hosted CI, after all-event existing-run query and remote SHA
verification. No local project execution; standalone scoped rustfmt2024 and inert
diff/hash/inventory inspection only. No worker/delegation or old CI rerun/cancel.
Source inspection and written tests are not compiler/runtime acceptance.

Bars/reference/action reconstruction, sealed source observation/frontier,
artifact/admission/supply, I3–I7, runtime UID/ACL, installer filesystem rollback,
I6 diagrams before main, Owner use and final C_R2 remain open.
