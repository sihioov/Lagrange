# R2 Wave105 — fixed historical base copies for daily readers

Status: source prepared; new candidate CI/functional results and host execution
are not yet accepted. R2 remains DOING. No delegation or new host authorization.

## Observed failure and verified source incompatibility

Wave104 used both Owner-approved dispatches. Candidate `a7825fa` produced the
accepted seventh set of12 images and restored the same Owner container before
the independent recovery unit terminated successfully. The fourth actual isolated
wholeflow then failed at producer exit1 with the finite diagnostic
`owner_beta_daily_cycle / INVALID_ARTIFACT`. It reached only
`PREPARED_SYNTHETIC_EXTENSION`; admission, publication, replay and restoration
were not reached. Its owned resources were fully removed and production11 states,
current release and the retained anonymous volume were unchanged.

- Image closure SHA `1048f7858e2d941d182551b304091be96d407fde61c1badda3cb23b03ad5bd64`.
- Wholeflow closure SHA `889ad46afa1c8d72e25e650ac3042700f578299ff8fcf1d7ab51dd7db46c874a`.
- Accepted manifest SHA `6e27840a52d0e54e5001a697ca51a0e42a7f7be82d035ab1fa0defa430d02bf5`.
- Full receipts remain under private `wp-8/maintenance-a7825fa-a7` and
  `wp-8/wholeflow-a7825fa-a5`. See the [consumed packet](2026-10-18-r2-a7825fa-host-sequence-packet.md).

Static source inspection establishes a separate, deterministic incompatibility.
`daily_ingest.rs:513,803` calls the existing historical artifact approver from
producer UID10002 and admitter UID10003. The V3 reader's
`historical_price_only_v3_artifact.rs:1230-1232,1569-1571` requires the root and
private candidate to belong to the current effective UID. The actual admission
receipt recorded the original base root as10001:10001/0750. The original bind
therefore cannot satisfy either daily reader's ownership contract. This does not
identify the precise rejected runtime predicate from the finite diagnostic, and
does not retrospectively resolve the original512 failure. Both remain UNKNOWN.
The original candidate/file metadata was not printed and is not inferred.

## Change

Keep the original approved base and the Rust reader/approval rules unchanged.
During protected preparation, read only the fixed approved V3 leaf and create
two byte-identical copies under `owner-beta-daily/historical-base`:

| Path | Owner/group | Mode |
| --- | --- | --- |
| `historical-base` |0:10001|0750|
| `producer`, `admitter` roots |10002:10002,10003:10003|0750|
| Each fixed candidate directory |matching role|0700|
| Each `manifest.json`, `bars.ndjson` |matching role|0600|

The helper has fixed candidate, unsigned canonical manifest, bars hash and size
pins matching the existing V3 registry. It adds no artifact selector or fallback.
The original source must match10001:10001/0750 root,0700 candidate and0600 files;
fresh verification of those previously unprinted facts is a prerequisite to a
future host attempt. Symlinks, hardlinks, ACLs, changed identities, extra entries
and different bytes fail closed. Source descriptors stay held and are rechecked.
The original root may retain unrelated historical siblings; those are untouched.

Both copies are verified in a root-only staging directory and published together
with exclusive rename. A complete matching pair is verified and reused without
changes; partial or different output is rejected without repair. Failure retains
an unpublished stage and blocks reuse. Root preparation holds the existing release
lock with the cohort stopped. DB preparation follows successful copy preparation;
enable and its preflight require successful copy verification.

Compose and the independent runtime policy bind each role's own root read-only at
`/data/owner-beta-artifacts`. Worker/API retain the original base. Existing UID,
network, capability, secret, admission and publication restrictions are unchanged.
The isolated wholeflow uses the same helper and role mounts, includes the copies
in its immutable-source comparisons, and verifies them after restoration. Copies
are within the existing six-class backup's `daily` tree. Original-base comparison,
resource ownership, cleanup, exit and OOM guards remain required.

Copy/pre-restore verification failures produce fixed wholeflow codes, without
exception text, retry or a new subprocess. Existing raw command outputs remain
RAM-only; finite approved diagnostics and hashes are the only recorded output.

## Verification scope

Twenty new required definitions are written:12 filesystem/real-UID/pin/restore,
three activation-ordering, one actual Docker read-only mount, three wholeflow
copy-failure/invariant, and one CI requirement test. All existing150 required
definitions,15 image outcomes and41 wholeflow cases are carried; the wholeflow
set becomes44. The exact definition inventory and source pins will be frozen
after commit, before one CI and one functional smoke dispatch.

Filesystem cases use disposable root fixtures and real UID10001/10002/10003
children. Their bytes are synthetic with test-process-only pins; they do not
prove real V3 content approval. Docker coverage uses the existing synthetic
isolation fixture. It proves actual read-only role mounts, not production data
admission or publication. Full Rust, strict Clippy, PostgreSQL, Web and functional
smoke evidence must still be recovered for the new candidate.

No local project test, source parser, helper or compiler was executed. Direct
source/byte review and `git diff --check` are preparation evidence only; no new
independent review is claimed. Both affected PlantUML sources and evidence
comments were updated. PNGs were rendered locally with verified official
PlantUML1.2026.9/Temurin21 JRE artifacts and the documented internal
[Smetana layout](https://plantuml.com/smetana02), unprivileged, with one CPU,
384MiB Java heap and a60-second bound. No Docker, external rendering service,
root or production observation was used. The first default-size images were
visibly clipped; a16384 image limit produced the complete diagrams.

## Remaining gates

Accepted a7825fa source, runs, images, manifests and closure receipts are immutable.
No automatic host retry, G4, deployment or provider activation is authorized.
Only after new exact-candidate hosted acceptance may a new concrete image →
conditional wholeflow packet be prepared for one new explicit Owner decision.
The packet must disclose the new private copies and their fresh source metadata
and byte gates. The original retained volume remains unmounted and undeleted;
prior0120 cleanup stays INCOMPLETE. G4 consistent pre-upgrade backup/transition,
deployment, Owner use, three consecutive actual trading days and final C_R2 are open.
