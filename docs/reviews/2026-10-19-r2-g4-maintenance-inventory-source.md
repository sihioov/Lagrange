# R2 G4 maintenance payload and retained terminal inventory

Wave109,2026-10-19. Source readiness only; hosted validation pending.
The [e639169 application image/wholeflow acceptance](2026-10-19-r2-e639169-host-acceptance.md) remains immutable. This change prepares a distinct maintenance tool revision; it neither rebuilds the accepted application nor authorizes a G4 outage or deployment.

Two source gaps prevented a concrete first-rollout recovery packet. The staged payload omitted the fixed historical-base helper imported transitively by recovery/backup/operations. The old contract required every lagrange unit to be a persistent inactive/dead unit, which cannot represent retained historical transient services without deleting evidence.

`prepare-production-recovery.py` now includes the helper among12required files and uses an explicit v2maintenance contract. Persistent units remain stopped; separately declared transient terminal services require canonical pinned fragments, completed immutable invocation timestamps, no job/restart/PIDs/activation hooks, and bounded recursive empty cgroups. The full union of both systemd inventories is rechecked; no units are reset, removed or filtered. Timers/socket/path activation edges must be stopped, reciprocal and fully bound. Every unsupported/missing field, extra unit, changed generation or uncertain process observation refuses.

Thirteen new named tests supplement all28prior maintenance definitions: two actual isolated-payload subprocess cases, six contract/mock-systemd inventory cases and five real private filesystem cgroup cases. They are authored, not locally executed. Existing encryption, source preservation, all-writer fencing, expiry, isolated PostgreSQL clone, no-clobber publication and ambiguous-cleanup tests stay required. A new exact maintenance candidate will run the full eight-job required CI once; application functional/host evidence belongs to the already accepted e639169 and is not relabelled as this tool's execution.

Same-agent direct source review checked dependency closure, category boundaries, finite observation grammar, preserved archive/lock/clone paths, fixture isolation and all retained test names. No new independent review. The cgroup test directories use protected /var/lib ancestry, consistent with the real protected-path reader; no broad permission exception was added. No local project parser/helper/compiler/test or production/root probe ran.

Architecture edges are unchanged. Their moved source citations were corrected and both PNGs re-rendered locally with bounded unprivileged Java/Smetana; decompressed pixel data and dimensions exactly match the prior images. Only embedded source metadata changed.

See the [v2contract and primary sources](../runbooks/r2-preupgrade-recovery.md#wave109-exact-unit-inventory-and-staged-payload-source-only). Actual host field values remain unobserved. G4 needs a fresh concrete packet for source staging, complete writer/network fence, clean same-generation DB shutdown, full backup/clone proof and deterministic restoration. Wave108 approval is exhausted; no new host question is pending while source validation remains unfinished. Deployment, Owner acceptance and three consecutive actual trading days remain open.

Private source inventory: `target/r2-daily-beta/wp-8/wave109/source-review.json`; full acceptance requires fresh named hosted results, never a source-only pass claim.
