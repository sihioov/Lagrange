# R2 approved-base whole-flow fixture

Roadmap R2-04/R2-07/R2-09; status updated2026-10-23. Historicale639169
approved-base integration was accepted in Wave108. Currenta6da63c Wave177
reported functional PASS and independently confirmed cleanup, but six of21
temporary container lifecycles lack complete independent event evidence. Its
full independent acceptance remains incomplete. Both original executions are
consumed; neither may be replayed to repair evidence.

See [current acceptance and limits](../reviews/2026-10-23-r2-beta-flow-acceptance.md).
Hosted fixture tests are a separate boundary and cannot close actual integration.

## Inputs and limits

The driver requires Linux root, the local Docker Unix socket, a protected
root-owned V2 manifest for all twelve images, the exact candidate checkout,
all images already built/byte-verified by the official image builder and the
actual approved V3 historical root. It never builds or pulls. It checks local
image IDs and OCI revisions against that manifest; it does not repeat the
builder's saved-image executable provenance check. The PostgreSQL image uses
the repository's exact 18.4 digest.

The historical root is mounted read-only. Producer and admitter call the
unaltered compiled approval API independently. No test approval registry or
dummy historical base is accepted. The source policy is the existing canonical
policy, hash `e9149ef56170877e5e9646f1fff1c370b3a34bf2c8b8ee8272bc30bd5fa8779c`.

The [extension builder](../../scripts/ops/lib/owner-beta-daily-wholeflow-fixture.py)
constructs all Raw records before execution: two KIS batches and one calendar
batch, sixty source files, immutable metadata/manifest/lock files in both
scopes. Requests use the existing redacted, credentialed **fixture grammar**;
no credentialed request or live provider observation occurs. Price/reference
rows cover all eleven ETFs; seven empty action channels cover each exact
addition interval. No provider transport exists in the harness.

Its calendar is deliberately synthetic: only 2026-08-31 and the real current
eligible civil date are Trading. Every intervening civil date is explicitly
Closed. It is not a claimed KRX calendar or holiday schedule. The eligible
date uses the existing Seoul16:30 cutoff, and the complete coverage is capped
at370 civil dates. An invalid/expired range or cutoff less than30 minutes away
fails before runtime resources are created. No process clock or poll checkpoint
is reset to force an outcome.

The default bound is1,200 seconds, configurable only600–1,800. File snapshot
bytes default to2GiB and retain the existing snapshot entry/metadata bounds.
Each Docker call, database startup, HTTP request and process is bounded. The
script waits62 real seconds between polls. It creates only nonce-labelled
containers/networks and private paths below a fresh `/var/lib/lagrange-wholeflow-*`.
No production release, existing Docker resource, timer, service or database is
changed. This description does not grant execution permission.

Wave178 adds continuous daemon-event recording for future distinct executions.
Two read-only Docker clients select only the exact container nonce label and
the two owned network names; network events do not carry the container labels.
Each owned create event must be received and durably stored before its process
starts. Starting an events client alone is not readiness. The collector permits
at most512 events,1MiB wire bytes and16KiB per line, within a fixed deadline no
longer than the execution bound plus300 seconds. It never reconnects.

The sibling `RECEIPT.events.jsonl` is created exclusively, root0600 under the
same protected directory. Only validated type, action, ID, nonce-derived name,
role, exact image ID, event timestamp and exit code are written. Arbitrary
labels, commands, provider output and raw errors are discarded. The result
includes the sanitized file's size/hash and collector drain state. After owned
cleanup, every inspected object must have its exact ordered lifecycle and
image binding; missing/duplicate/foreign events, OOM, stream loss or overflow
make the result FAIL even if application phases passed. The only processes the
collector stops are its own two read-only Docker clients. Existing cleanup
identity and volume guards remain unchanged.

These source changes do not reconstruct Wave177's missing records or authorize
replaying a consumed original. Offline pipe/subprocess fixtures and a hosted
Docker `/bin/true` fixture test recording separately from actual approved-base
execution. The latter has no application/database process or provider access.

## Required substantive sequence

1. Inspect exact images, source/policy pins and protected base filesystem;
   construct the complete synthetic extension and fresh role-owned state.
2. Start one private PostgreSQL instance on an internal-only network. Execute
   the real role-bootstrap and SQLx migration image entrypoints, then compare
   all62 successful migration checksums. Bootstrap is not manually simulated.
3. Seed only synthetic Owner/session/entitlement/instrument fixture rows. Use
   the real migration-owner control transaction. Create the strategy config
   through the real production API using the catalog's actual BuyAndHold
   parameters. Do not insert an admission, poll receipt, reservation, job,
   recommendation, publication claim or result item through the fixture SQL.
4. Execute the actual producer as10002 with network`none`, no DB secrets and
   only candidate/private-state writes; require `CANDIDATE_READY`. Execute the
   real admitter as10003 with only the private DB connection and allowed
   admission/frontier/private-state writes; require `ADMITTED` and the first
   parentless admission.
5. After real polling time, repeat for the second session. Require the exact
   parent chain, preserved2016-08-29 start,2,453 then2,454 sessions and corresponding
   eleven-per-session bar counts. Execute the real worker as10001 and require
   `SUCCEEDED`, one run/job, real HTTP`CURRENT`, exact support/as_of and admission
   hashes, factor/target hashes and nonempty result items matching the DB.
6. Stop/restart the API and start fresh one-shot producer/admitter/worker
   processes. Require `REPLAY`/`REPLAY`/`IDLE`; compare complete immutable DB
   witnesses, source bytes and publication identity. Exit0/healthy/`NOT_DUE`
   alone never passes a phase.
7. Stop the API, capture the six file classes through the existing descriptor
   snapshot format and a real custom-format DB dump, with unchanged DB/file
   witnesses. Stop the original DB. Restore to a new empty private file root
   and new PostgreSQL instance, retaining database owners/ACLs. Before any
   recovered process runs, require exact migration checksums, file metadata/
   content, DB history and poll clocks. Restart the real processes and repeat
   the replay/HTTP checks without changing run, job, admission or source identity.
8. Remove only resources whose exact nonce label and intended name match;
   verify container/network absence before removing private staged bytes.
   A lost create reply without a recovered object retains the private stage
   and intent, and reports failure. An unexpected object is never removed.

The HTTP checks use a synthetic, hashed server-side session and the real
app/admin/audit roles. Requests go directly to the private API address without
published ports or redirects. They prove neither normal Auth0 login nor browser
Secure-cookie/TLS behavior nor actual Owner use. Auth0 configuration contains
fixture-only values, and no Auth0 endpoint is invoked.

The capture is a disposable integration recovery, not a production encrypted
backup acceptance. Temporary private archive/DB files stay on the host, mode0600
behind root-only parents. Successful owned cleanup removes them; uncertain
cleanup retains them for explicit reconciliation. No private base, Raw body,
session cookie, DB password or raw log is uploaded to hosted CI or written into
the sanitized result receipt.

## Command shape after candidate/images/authority are bound

```sh
/usr/bin/python3 /protected/exact-candidate/scripts/ops/verify-owner-beta-daily-wholeflow.py \
  --execute --candidate FULL_CANDIDATE_SHA \
  --image-manifest /protected/exact-candidate-images.manifest \
  --approved-base /protected/artifacts/historical-price-beta-root \
  --receipt /protected/evidence/fresh-wholeflow-result.json \
  --timeout-seconds 1200 --max-bytes 2147483648
```

These are placeholders, not an execution-ready approval packet. The concrete
candidate, real image manifest, protected host paths, fresh admission and
applicable G3/G4 authority must be fixed first. Existing read-only KIS/EOD/OwnerV2
approvals remain unchanged. Source preparation does not renew an expired sudo
grant or authorize production activation.

Hosted CI runs synthetic fixture/mocked-driver tests and a separate disposable
Docker mount-boundary suite. Their version-specific results are recorded with
the exact candidate CI closure. They do not establish actual approved-base
admission, HTTP publication or recovery.

Remaining: complete current-candidate independent lifecycle evidence, admitted
production inputs, actual Owner use, three consecutive scheduled trading days
and final`C_R2`. Historical G4 acceptance remains preserved and is not repeated
by this disposable integration. D-06/D-07 remain deferred follow-ups.
