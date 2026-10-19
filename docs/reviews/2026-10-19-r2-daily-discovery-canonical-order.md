# R2 daily discovery canonical order — Wave107

R2-04/R2-09. Candidate e639169 HOSTED_ACCEPTED_ONLY; full recovery02:21:48UTC.
New host sequence remains prepared/unapproved; see the Wave108 packet below.

## Recovered Wave106 outcome

Owner approval `79bbfcb7fd01ec56407f7399575a2c6585f6d81f2655e9179157a909e0b57e67`
was consumed by exactly one eighth image attempt and one fifth actual wholeflow.
Candidate `b6b2ddbd7a5d63f482f1a0fa43a02cb704a70e93` remains hosted/image accepted.
Its 12-image build passed 704 file checks and 49 official gates, with 2152 source
pins, 11 image controls and 12 wholeflow critical pins. Owner V2 exited normally0
and the same container returned healthy at01:23:58.251473UTC. Build and independent
recovery actually finished successfully at01:23:58.260219/01:24:00.630673UTC;
PIDs/cgroups were empty, exact11 production states/current release unchanged,
postrestore READ ONLY settlement passed. The image closure SHA is
`a285575b82fd3e886d50eeb5f42918559cf0d25719a3b44b89c65fb9b8f6e9e3`, manifest SHA
`79555de12f99b36bea64733eb382ee04ae8233e1bf62c78111fc3c865a10d096`.
Preserve the images, manifest and consumed01:06:44.667022UTC window.

Wholeflow a6 dispatched once01:24:23.799612UTC, failed exit1 at01:25:36.496125UTC.
Both the first real producer and admitter exited0, reaching ACTUAL_FIRST_ADMISSION.
The second producer emitted only `owner_beta_daily_cycle/INPUT_BLOCKED`: stderr58
bytes SHA `1ad63606925e39df661e15d0418951f36843c1e8fc3672e432b4832d85c7aaaa`,
stdout empty. The exact inner runtime predicate was not emitted. No second admission,
publication, restart, capture or restore was reached. Sessions were synthetic
2026-08-31/2026-10-18; these are not actual trading-day observations.

Full recovery completed01:27:03.305882UTC, closure SHA
`ec8484703eaf3c422c37de2a23e3905b7cd7ef5edb4f3092107dbedfcca9bfd0` (23 pins).
All9 actual containers and the exact network were destroyed, nonce namespace/stage
absent, driver/PIDs/cgroup0, production11/currentfce9b90/retainedvolume unchanged,
kernelOOM0. PG/API137 occurred during private cleanup. Preserve the failed unit and
root result (2033 bytes, SHA `147cef671a4d91ff4fe34102ac3ccf21bcdb65ec9c73f985bd73e1a94ea9c4f0`).
Original-base final byte comparison and unprinted individual62 checksum rows remain
UNKNOWN. Original512 inner cause and exact a7825fa rejected predicate remain UNKNOWN.
Prior0120 cleanup stays INCOMPLETE. No reset, relaunch, automatic retry or successor
host authorization follows from this failure.

## Source defect and bounded correction

`reconstruct_actions` sorts action witnesses by the Rust enum declaration order:
paidin-subscription precedes bonus-issue. `contract::canonicalize_session` sorts
by the established wire literals, where bonus-issue comes first. Discovery previously
hashed a canonicalized **copy**, then returned the uncanonicalized session. The first
sealed manifest contained canonical order; `refresh_source_frontier` later compared
that stored session with the returned discovery using structural equality and mapped
an unequal session to InvalidCandidate, then producer INPUT_BLOCKED. Every complete
seven-class discovery therefore contains this deterministic ordering mismatch.
This establishes a source defect consistent with the observed failure, not a recovered
runtime stack trace or proof that every later wholeflow phase succeeds.

The correction reuses the existing module-private wire canonicalizer on the discovered
session before returning/hash construction. Wire grammar, ordering, hashes, evidence
validation, immutable source checks, approval registry, limits and failure mapping
remain the same. No runtime subprocess, retry, provider call or host control changes.
Reconstruction's action-selection indices continue to refer to its original witness
vector; only the copied session is canonicalized. No structural deployment/architecture
edge changes, so the diagrams are unchanged.

Four new Linux hosted Rust tests require: returned session equals its canonical
commitment; later observation of unchanged source equals that saved session without
rewrites; a changed source stays distinct; and both exact synthetic wholeflow planner
sessions pass the actual Rust Raw reader/discovery and canonical roundtrip. The last
case runs the pure Python planner in hosted CI only, with no private V3 base, database,
Docker or provider. These tests do not manufacture a sealed approval or claim actual
second admission. Existing tests and exclusions remain required.

No local project parser/helper/compiler/tests or new independent review have run.
At source preparation only source/byte review was used. The frozen candidate needed
its own full CI and functional smoke once, with full logs, named outcomes, checkouts
and cleanup recovered before a new concrete host packet/one Owner decision.

Separate G4 readiness gaps are recorded in wave106/g4-static-readiness-gaps.json:
the standalone cold-recovery payload omits its newly imported base helper; the strict
unit inventory cannot represent preserved terminal historical transient units. They
remain open, outside this minimal producer correction; no G4 host action is authorized.
G4 backup/transition/deployment, actual Owner use, three consecutive actual trading
days and final C_R2 remain open. Preserve all previous accepted/failure evidence and
retained volume159b052fed63a476fba2c51c8f15f532a837096e4bdd453bb753789672034f07;
no mount/content/delete/prune.

Historical source-stage tracking: successor dda1a840 confirmed01:30:27.983UTC;
persisted handoff before1d3e4474 deletion. Its renewal bound was12:20UTC. No delegation;
native Ralph blocked/no resume API, same-agent bounded heartbeat only.


## Full hosted acceptance — 2026-10-19 02:21 UTC

Candidate `e639169018519fd6636c5579d1957e323e2c7802` was frozen before one
[CI39186174905](https://github.com/sihioov/Lagrange/actions/runs/39186174905)
and one [functional39186177169](https://github.com/sihioov/Lagrange/actions/runs/39186177169),
both attempt1. All8 CI jobs succeeded02:03:23UTC; functional succeeded02:20:59UTC.
Full logs recovered02:21:48.111212UTC:9 logs/8 actual checkouts,1437 frozen pins,
537 candidate blobs/2155 tree entries,174 required definitions,15 image outcomes,
44 wholeflow definitions. New named Linux Rust outcomes all PASS:

- `daily_discovery_session_matches_canonical_commitment_order`
- `daily_discovery_later_observation_matches_committed_session_without_rewrite`
- `daily_discovery_canonical_order_keeps_changed_source_distinct`
- `daily_wholeflow_fixture_both_sessions_roundtrip_through_rust_discovery`

Carried Rust groups remain intact:3367 named rows/231 root blocks/36 workspace
and33 carried groups; only discovery integration17→21 increased. Strict Clippy,
format, Python497 passed/10 existing skips, PG19 process exits0/146 named cases,
0060 32+73/83 bootstrap/62 migration checksums twice/0039 cleanup and Web143 passed.
Functional evidence contains actual publication/source lineage and unchanged repeat,
10 cleanup exits0, owned resource absence, raw-probe/global original+cleanup exits0.
No separate unprinted raw-init marker is invented. Three non-Linux exclusions,
six ignored Rust cases,10 Python skips and independent nested/unprinted UNKNOWN
remain. No rerun/cancel/duplicate or accepted source/evidence alteration.

Private receipts under `target/r2-daily-beta/ci/e639169/`:

| Receipt | SHA-256 |
| --- | --- |
| `required-ci-evidence.json` | `006273fc662daf091e597f47e9e21d6c39526a9d643d5d41eaffe7921bf1cb6b` |
| `functional-smoke-evidence.json` | `2cd651b96fe22caafd763cf32a276f0e287afd5ef401470f40105f53d79fb983` |
| `hosted-terminal-evidence.json` | `f44b66b1b1ce613fa0836385475cc165d8d021bd16cfd93c63fad2307a9c9314` |

This accepts hosted verification only. Exact previous runtime predicate remains
UNKNOWN. Actual image/host integration/G4/deployment/Owner/three trading days/final
C_R2 are not accepted. [Wave108 concrete packet](2026-10-19-r2-e639169-host-sequence-packet.md)
proposes one ninth image attempt plus conditional sixth actual isolated wholeflow.
No new host decision exists yet. Current tracking68a23346 confirmed02:32:49UTC,
handoffcfb61e58 persisted; renew13:00UTC, expiry14:32:49UTC. The packet records the
corrected heartbeat path and same-ID replacement error; no host action occurred.
