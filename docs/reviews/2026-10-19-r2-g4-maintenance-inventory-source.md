# R2 G4 maintenance payload and retained terminal inventory

Wave109,2026-10-19. **Maintenance-tool hosted CI accepted; G4 operating execution remains unperformed.**
The [e639169 application image/wholeflow acceptance](2026-10-19-r2-e639169-host-acceptance.md) remains immutable. This change prepares a distinct maintenance tool revision; it neither rebuilds the accepted application nor authorizes a G4 outage or deployment.

Two source gaps prevented a concrete first-rollout recovery packet. The staged payload omitted the fixed historical-base helper imported transitively by recovery/backup/operations. The old contract required every lagrange unit to be a persistent inactive/dead unit, which cannot represent retained historical transient services without deleting evidence.

`prepare-production-recovery.py` now includes the helper among12required files and uses an explicit v2maintenance contract. Persistent units remain stopped; separately declared transient terminal services require canonical pinned fragments, completed immutable invocation timestamps, no job/restart/PIDs/activation hooks, and bounded recursive empty cgroups. The full union of both systemd inventories is rechecked; no units are reset, removed or filtered. Timers/socket/path activation edges must be stopped, reciprocal and fully bound. Every unsupported/missing field, extra unit, changed generation or uncertain process observation refuses.

Thirteen new named tests supplement all28prior maintenance definitions: two actual isolated-payload subprocess cases, six contract/mock-systemd inventory cases and five real private filesystem cgroup cases. They were not locally executed; all41 now passed in the hosted run recorded below. Existing encryption, source preservation, all-writer fencing, expiry, isolated PostgreSQL clone, no-clobber publication and ambiguous-cleanup tests stay required. The exact maintenance candidate ran the full eight-job required CI once; application functional/host evidence belongs to the already accepted e639169 and is not relabelled as this tool's execution.

Same-agent direct source review checked dependency closure, category boundaries, finite observation grammar, preserved archive/lock/clone paths, fixture isolation and all retained test names. No new independent review. The cgroup test directories use protected /var/lib ancestry, consistent with the real protected-path reader; no broad permission exception was added. No local project parser/helper/compiler/test or production/root probe ran.

Architecture edges are unchanged. Their moved source citations were corrected and both PNGs re-rendered locally with bounded unprivileged Java/Smetana; decompressed pixel data and dimensions exactly match the prior images. Only embedded source metadata changed.

See the [v2contract and primary sources](../runbooks/r2-preupgrade-recovery.md#wave109-exact-unit-inventory-and-staged-payload-source-only). Actual host field values remain unobserved. G4 needs a fresh concrete packet for source staging, complete writer/network fence, clean same-generation DB shutdown, full backup/clone proof and deterministic restoration. Wave108 approval is exhausted. The next [bounded read-only inventory packet](2026-10-19-r2-g4-operating-inventory-packet.md) needs a new scoped Owner decision before any host observation. Deployment, Owner acceptance and three consecutive actual trading days remain open.

## Hosted acceptance — 2026-10-19 04:00 UTC

Candidate `3de6420a17e6093bfe05656b4ff1406857b5404c` completed
[CI39195322025 attempt1](https://github.com/sihioov/Lagrange/actions/runs/39195322025)
with all8jobs successful at03:53:57UTC. Full recovery at04:00:50UTC reconciled
8full logs,7actual checkouts,1447input pins,540tracked blobs and2158tree entries.
All187required definitions passed, including all41maintenance cases (13new),
15image outcomes,44wholeflow fixtures,3367Rust named rows/231root blocks,
36workspace groups, strictClippy/fmt,Python497 with10existing skips,
PG19processes/146names/0060groups32+73/83bootstrap/62checksums twice andWeb143.
Three non-Linux exclusions, six ignored workspace rows and unprinted independent
nested rows/exits remain explicit. One fragmented nested name is UNKNOWN.
No new application functional smoke, image build or wholeflow was requested for
this tool-only change; e639169 acceptance is separately pinned, never relabelled.

The first coordinator log parser stopped because Cargo stdout/stderr interleaving
printed two open `running` records before their corresponding binary headers.
A preserved second coordinator binds only the still-open unassigned prefix to
the actual following same-phase header. Exact prior root summaries, required
names and all other gates remain enforced. No project code, workflow or source
was changed or retried. Original parser/error/diagnostic remain preserved.

Private receipts under `target/r2-daily-beta/`:

- `ci/3de6420/hosted-terminal-evidence.json`: `a2a0e307ff8a52bcc6f7c38d787ff9af145d3c18a0395beb255b7488bc913deb`.
- `ci/3de6420/required-ci-input-reconciliation.json`: `392f4502bd14585463340ab563c2cef065d136740985de66eee7da92953bf842`.
- `ci/3de6420/workspace-reconciliation.json`: `0a571e0aeeb784a699e84ca7f97cb1b6de8ccb815fdfd48350670905923f31bd`.
- `ci/3de6420/policy-pg-web-reconciliation.json`: `5754c27596a2d3931a0461bf93098b437badbfc7558bf94167d2d86710e48776`.
- `wp-8/wave109/workspace-parser-correction.json`: `b9744a8504d3ef5b10daad6fe5e4e9b333feadcb5707bcfd53caa4db28c83110`.

The accepted earlier host metadata includes a disabled persistent failed KIS
service, which this tool's strict persistent inactive/dead category does not
accept. Fresh complete fields and a precise disposition are required; no reset,
delete, broad inventory filter or inferred success is allowed. Hosted validation
does not establish the actual G4 fence, backup, restoration or deployment.
