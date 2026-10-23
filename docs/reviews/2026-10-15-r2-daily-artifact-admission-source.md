# R2 I1b-S5c — first candidate storage and independent admission

2026-10-15; R2-04/09, Wave46. Baseline is accepted S5a/S5b candidate
`5f5f4e845de5011f7ee2e7d0a1fe0e27d0d18f18`, with full recovery and bounded
[acceptance](2026-10-15-r2-daily-protected-prefix-acceptance.md) committed in
`36fa991`. This is direct coordinator source disposition, not independent review
or executable acceptance. No further workers were delegated.

The owned source files are `daily_price_only/artifact.rs`, `admission.rs`,
`history.rs` and module registration. D4, approved historical constructors, Raw
snapshot/normalizers, provider requests, SQL, dependencies and workflows are unchanged.
The first historical continuation remains the only supported parent form.

## Storage and authority boundary

Candidate and admission roots are separate opaque types opened with explicit
expected owner/group IDs. The API never provisions roots or selects live UIDs.
Linux descriptor-relative traversal rejects noncanonical paths, symlinks, writable
ancestors, mismatched owners/groups and any access/default POSIX ACL. The leaf root
must be0750; regular files must be0440 with one link. Unsupported platform or ACL
inspection fails closed. Installer ownership/mount checks remain required.
The reader accepts ancestor ownership by root or the explicitly trusted leaf owner;
the deployment must additionally provide the design's root-owned parent hierarchy.
That installer invariant is not established by these same-UID storage fixtures.

The candidate leaf contains exactly manifest.json, normalization.json and
bars.ndjson. An admission leaf additionally requires admission.json. Validated
content/admission hashes name the leaves. Readers retain directory and file
descriptors, full file stat snapshots, canonical bytes and digests. Revalidation
checks ancestry, names, metadata, ACLs and hashes of the retained file descriptions;
metadata equality alone does not accept changed content.

Publication creates new private staging inodes, writes copied bytes, seals modes
and group, fsyncs files/directory, reads back exact bytes and publishes with Linux
rename-NOREPLACE. Root fsync and final readback precede success. An existing exact
leaf is replayed without replacing its inodes; a conflict is rejected. Cleanup
only unlinks tracked, identity-matched staging files and uses nonrecursive rmdir.
Unexpected entries are not recursively deleted. Failure after visible publication
does not remove that leaf; root-fsync failure returns DurabilityUnknown. An error
between staging mkdir and descriptor acquisition may leave an unaccepted orphan.

The admission writer requires an opaque independently verified proof and a
different root identity and owner UID from the candidate. It copies into its own
four-file tree. Different paths with one UID do not establish process separation.
Actual distinct-UID access, mounts and privilege denial remain I6 integration work.

## Independent comparison and sealed readback

PinnedDailyPolicy accepts only exact canonical D4 bytes matching an explicit
expected hash. It binds bytes; the installer/config and DB gates establish current
policy authority. The verifier borrows the real ApprovedHistoricalPriceOnlyArtifact,
reconstructs the full protected base and independently discovers every captured
source scope. It does not select evidence from producer-declared file lists.

It reconstructs the first continuation and candidate, compares all three exact
files, independently derives counts, creates canonical admission bytes and validates
the complete D4 bundle/pins. Explicit producer/verifier release commits enter the
lineage. The private proof retains base/policy/source/candidate lifetimes. No clock,
random identity, provider call, SQL write or live authorization enters this step.

Sealed readback verifies all four files, admission hash, canonical wire encodings,
the full D4 bundle and the real base/policy pins. The bounded NDJSON reader requires
exact field order/encoding/LF, canonical decimal strings, valid OHLC, the complete
ordered ETF11/session grid, unchanged historical raw/adjusted representations and
one exact new eleven-row session with raw equal to adjusted. The opaque result is
offline file evidence; current admission ancestry/revocation, scheduling and
publication require the guarded DB path. A file write alone is never approval.

## Written regressions and remaining evidence

27 new definitions are WRITTEN_UNEXECUTED:17 Linux storage,4 canonical price
readback,5 policy binding and1 non-Linux unsupported-platform case. Hosted Linux
expects26 new named results; the last case is excluded, not passed. Storage covers
exact copy/replay/conflict, permissions/ownership/path/ACL/link rejection, missing/
extra files, oversized sparse files, in-place rewrite with and without changed
metadata, replacement, concurrent exact publication, three/four-file separation
and same-UID rejection. Readback tests cover exact historical representation,
ambiguous/noncanonical rows and row/line limits. All21 S5b and33 S5a definitions
remain present; their current line anchors are regenerated in the successor inventory.

Scoped standalone rustfmt2024 and whitespace checks are allowed static evidence.
No local compiler, Cargo, Clippy, tests, build, project helper, parser, SQL, Docker,
provider or runtime execution occurred. No fixture constructs a fake approved base.
The real approved-factory → Raw → candidate → distinct-UID admission flow is not
exercised by these small private tests. Fsync/rename fault injection and crash
durability, platform coverage and full-limit allocation are also not claimed.

Before executable acceptance, freeze exact source/reference pins and qualified
names, query all-event exact-head existing runs and verify remote SHA, then dispatch
one new candidate CI. Recover every terminal job, checkout/log hash, named result
and prior DB gate. Never rerun/cancel/duplicate earlier runs. Later daily-parent
continuation, sealed frontier persistence/refresh, supply and I3–I7 integration,
independent final review, deployment/Owner/three trading days/final C_R2 remain open.
