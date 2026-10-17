# R2 image maintenance: drain failure and verified restoration

R2-04/09, Wave92, candidate `ab48a7953dd07fc479407cf80a58f37e2c0c6a83`.
The Owner approved the prepared one-attempt maintenance packet with
“승인해 이어서 진행해”. The authorized attempt ran once. **The Owner V2
container exited1 after SIGTERM, so pre-start validation rejected the drain.
The image builder never started. The same container was restored and healthy
within approximately six seconds.** No image or release acceptance is claimed.

## Actual execution and recovery

All times below are UTC on2026-10-17; add nine hours for2026-10-18 KST.

| Evidence | Actual result |
| --- | --- |
| Frozen source and controls | 2,125 candidate files and eleven prepared controls matched; clean detached HEAD and inert0600 env unchanged |
| Fresh host and work packet | PASS; eleven exact containers, ten healthy and research historically exited2; no compiler or recent OOM, resource policy passed |
| Read-only aggregate checks | Explicit READ ONLY/repeatable-read/operator-wide visibility; running and queued jobs0, running attempts0, no unexpired producer or pending subscription operation |
| Official plan/preflight | Both exit0; exact twelve services; source-input SHA256 `454cad1f9cb8f12ddafca6729897acc9ef2eccd5947adf40942020b1e9a1cf76` |
| Independent recovery | Active/running before launch; root window started17:24:41.662284, maximum end2026-10-18 05:24:41.662284 |
| Main unit dispatch | Exactly once at17:25:09.491359; `lagrange-r2-build-ab48a79.service` |
| Docker lifecycle | Signal15 at17:25:09.858087955; observed die exit1 at17:25:09.939767745; no forced-kill fallback |
| Build pre-start | Exit1, fixed code `reader-drain-state-invalid`; required normal exit0 was not satisfied |
| Normal restore hook | Exit0; original container running/healthy at17:25:16.269605 |
| Independent recovery | Same-container healthy receipt17:25:21.752239; successful service deactivation observed in its journal |
| Post-restoration | All eleven fixed container fields matched the prior observation; nine other serving containers healthy, research still stopped; READ ONLY work probe exit0 and settled |
| Build outputs | Main process start timestamp absent, candidate build state/manifest/drained attestation absent, no compiler; zero images built |

The restored identity is
`f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`, image
`sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`.
The main transient unit remains failed as evidence. The independent transient
unit was collected after successful completion; its later `not-found` status
alone was not treated as proof of success.

The private closure
`target/r2-daily-beta/wp-8/maintenance-ab48a79/attempt-closure.json` pins twelve
receipts; SHA256
`60f05c04b8b0d0ac63d27af5851751d7fa60ab7a28f12fb180e276e2b2515680`.
The finite unit-journal capture hash is
`d9465528b9fc7d87a31eed77b00730d30aa004cadead610d96b8a4e13eae0257`.
Only allowlisted diagnostic markers and structured observations were persisted;
raw logs stayed in memory. No new CI, product modification, gate relaxation,
deployment, migration or research-worker start occurred.

## Cause and evidence limits

The concrete admission failure is the container's nonzero exit after the
authorized normal signal. Its internal application cause is **UNKNOWN**.
The retained startup window contained no rows; the bounded pre-stop tail and
stop/restore window contained no matching fixed application-failure diagnostic.
This does not prove there was no earlier application failure.

Static inspection of the installed-release source at
`fce9b90542cf9b1a911d46e4c4d3530db7efba4c`,
`crates/job-queue/src/bin/owner-equity-v2-runner.rs:873–906,1007–1028`, shows
that a prior quote-task failure can leave the daemon's exit result nonzero
while EOD processing continues. That is a possible path, **not a demonstrated
cause of this instance**. The corresponding Git blob SHA256 is
`e7b75462fac5009237013d1b6d9bf0e2503318235c19a756146891b2f396bfac`.
No change to that behavior or to the required exit0 guard was made.

## Prepared second attempt — new decision required

The [approved packet](2026-10-18-r2-image-maintenance-packet.md) explicitly
covered one attempt. Actual SIGTERM and restoration consumed that attempt;
the twelve-hour bound was not an unlimited retry allowance. A second
interruption has therefore been prepared separately and has **not** executed.

It uses the same exact candidate, container, inert env, official twelve-image
builder and unchanged guards. Eleven files are prepared under
`target/r2-daily-beta/wp-8/maintenance-ab48a79-a2/`. Only attempt-specific paths,
unit names and dependent hashes differ; reversing those substitutions restores
every first-attempt byte, and the three runtime/probe/SQL files are identical.
The exact launch argv agrees with its configuration. No new independent review
or fresh control-test result is claimed.

| Second-attempt binding | Prepared value |
| --- | --- |
| Root control directory | `/etc/lagrange/release-manifests/.r2-maintenance-ab48a79-a2-20261018` |
| Build unit | `lagrange-r2-build-ab48a79-a2.service` |
| Independent recovery unit | `lagrange-r2-recovery-ab48a79-a2.service` |
| Preparation SHA256 | `b1fcfa9ab3599a87330e2f68b9ca62778d452b2f5ee9c2d6fb7ed9d66417e9f3` |
| Exact command-ledger SHA256 | `d747d101d87f678b5b39cdf0383096fe2584bd66e785d69c996d0005ff7f5bc8` |

The proposed scope is one more attempt with a new shared maximum12-hour
window: stop only in its first hour, build runtime10hours, independent stop
request at10.5hours, attestation expiry11hours, immediate same-ID restoration
on completion or abort. Fresh source/host/work-state/plan/preflight checks
must pass first; independent recovery must be active before the new stop.
Exit0 remains mandatory. If the nonzero exit recurs, the attempt fails and
restores again; a successful retry is not assumed. No forced kill, deployment,
migration, research start or new provider permission is included.

Actual images, approved-base host flow, G4 deployment, Owner use, three
consecutive trading days and final C_R2 remain open. Hosted CI/smoke acceptance
of `ab48a79` and all prior failure evidence remain unchanged.


After full first-attempt terminal/recovery evidence was persisted, successor
heartbeata84ee487 was created2026-10-17T17:33:15.653Z; expiry2026-10-18T05:33:15.650Z,
renew/handoff by04:50UTC and before47thtick. Handoff was recorded before
confirmed deletion of50f3f2ad. It awaits the already-sent second-attempt
decision and preserves full R2 tracking; it is not an active build or worker.
