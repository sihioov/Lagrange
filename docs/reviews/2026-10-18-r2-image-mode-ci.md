# R2 image contract correction: hosted verification

R2-04/09, Wave93. Candidate `0d407f8ba613d75d99c1cbc8e974a72073b9d5bb`
contains the six-file correction described in the frozen
[source and recovery report](2026-10-18-r2-image-mode-correction.md).
It has **not been accepted**. At 2026-10-18 01:20 UTC, CI had terminated
with failure; its eight jobs and complete logs have now been recovered. The
separate functional smoke remains running. Sources and frozen inputs remain
unchanged until both executions have terminal full recovery.

## Exact dispatch and recovery scope

The all-event/all-workflow exact-head query returned zero runs at
00:55:10.407883 UTC, after the remote branch SHA was verified. Each workflow
was dispatched exactly once, with exit0 and no retry:

| Workflow | Actual run / attempt | Created UTC | Full recovery bound UTC |
| --- | --- | --- | --- |
| CI | [39084560317 / 1](https://github.com/sihioov/Lagrange/actions/runs/39084560317) | 00:55:26 | 02:40:26 |
| Research worker smoke | [39084562837 / 1](https://github.com/sihioov/Lagrange/actions/runs/39084562837) | 00:55:28 | 02:40:28 |

Both are `workflow_dispatch` on `excited-dolphin` at the exact candidate SHA.
The frozen current inputs contain 1,166 pins and 517 committed blobs; SHA256
`8e9b3a690b3e410b943b396f7d91665d4a9f78a2287ccba46ff2c82529273baa`.
The current inventory SHA256 is
`143594332c5dd83e4f3ccccf0abedeb6f534a9dbd1b9ddaea42eece6f6ec0716`:
118 required definitions and nine separate image-suite markers. Preparation
flags saying no dispatch are historical; the two launch receipts record the
actual executions. Earlier unlaunched inventories are preserved.

## Complete CI failure recovery; functional smoke pending

The CI execution jobs ended with six successes and one failure; the required
aggregate consequently failed. The only failing execution job is policy, at
the new full synthetic image-builder fixture. All eight full log hashes and
seven actual checkout SHAs were recovered. All 1,166 current input pins and
517 candidate blob contents still match. The immutable CI-only recovery
receipt SHA256 is
`be3a23b91f70db0ec4ffe6e1b332ff3619f9b67ab3fcfc8844828d2a46cc7bc9` at
`target/r2-daily-beta/ci/0d407f8/required-ci-evidence.json`.

| Evidence scope | Actual result |
| --- | --- |
| 118 required definitions | 106 named PASS; 12 NOT_EXECUTED at explicitly skipped policy steps |
| Full image fixture | Six required markers PASS; three UNKNOWN; step exit1 with no failing assertion printed |
| Workspace Rust | 3,350 complete named PASS rows and six ignored; all applicable carried required groups match their qualified binary/phase; six chart cases separately PASS |
| Policy carried groups | 71 named PASS; 110 entries NOT_EXECUTED at skipped steps; inventories overlap and are not unique-count totals |
| Original ten correction cases | Six PASS; four NOT_EXECUTED; affected four-module inventory77 PASS /27 NOT_EXECUTED |
| Web | 34 supervisor, six contract and18 UI named cases; browser129+14 identities/outcomes and both cleanup/reservation gates pass |
| PostgreSQL | 19 direct processes exit0/no SKIP;146 named rows;0060 names32/subcases73;83 bootstrap cases;62 source migration checksums match twice;0039 guard and owned cleanup pass |
| Native recovery fixture | Eight actual source/restored ownership observations999:0/0700 match; no production-state claim |
| Format/lint/Python | Workspace format and strict all-target/all-feature Clippy pass; Python495 passed/10 skipped/7 warnings is aggregate-only evidence |

A fragmented nonrequired Rust name and unprinted nested helper rows/exits
remain UNKNOWN. Three source-verified non-Linux exclusions remain excluded.
The earlier receipt's empty-target PASS label for one exclusion is not carried
forward as execution evidence. Functional smoke has not yet been accepted.

## Historical partial recovery at 01:02 UTC

Four complete logs and their actual checkout SHAs have been recovered.
Format, strict Clippy and Python jobs passed. The Python summary is
495 passed / 10 skipped / 7 warnings; this aggregate is not named evidence.
Policy failed specifically at the new full synthetic image-builder step.

Six of nine image-suite markers passed, including both new daily-control
0775 rejection cases. The positive payload/mode marker, recipe-omission
marker and overall full-suite marker are absent and remain UNKNOWN.
The log prints exit1 without the failing assertion or underlying cause;
it does not establish that the product correction is defective or accepted.

Of 118 required definitions, 102 have direct named PASS rows, twelve are
NOT_EXECUTED because their policy steps were skipped, and four Rust cases are
UNKNOWN pending workspace evidence. Subsequent policy steps have
explicit skipped outcomes. Workspace, Web, PostgreSQL and functional smoke
still require complete recovery; all required historical groups must be
reconciled again for this candidate.

Sanitized partial evidence is stored in
`target/r2-daily-beta/ci/0d407f8/ci-evidence.json`; full raw logs are not
persisted. Complete job/step metadata, log hashes, actual checkouts, named
rows, PostgreSQL processes/checksums and real smoke cleanup are required
before final disposition. Unprinted nested helper rows/exits stay UNKNOWN.

## Continuation and production boundary

The second production maintenance attempt is closed with verified restoration
and independent recovery. Its one built image remains unaccepted, with
zero of twelve verified images and no manifest. Earlier failed units,
receipts and the old `ab48a79` hosted success remain preserved.

Same-agent heartbeat `e9dd6173` was confirmed active and its handoff persisted
before predecessor `bab15aae` was deleted. It expires at 12:56:26.726 UTC;
renew by 12:05 UTC and before its 47th tick. This tracks full R2 continuation,
not only this CI package. Native Ralph remains blocked; the heartbeat does
not protect against daemon outage.

Finish complete recovery before changing the candidate. A third maintenance
interruption has not been authorized; prepare its concrete candidate packet
only after resolving hosted verification. Actual images, approved-base host
flow, G4 deployment, Owner use, three consecutive trading days and final
C_R2 remain open.
