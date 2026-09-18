# Production build improvement: resumed execution

The owner requested continued execution through completion on 2026-09-15, after
the proposal to amend only the blocked execution conditions. This amendment
applies to the existing production-build-architecture plan and G1 layout design.
Starting checkpoint: `ff4a67dd0c113955ac1bca98db7d9825fa4fef20`.

## Execution scope

Proceed through actual fixture verification, G2 selection, product build-code
implementation, and image-only release/performance verification. The existing
12-image/revision/strict-V2 contracts, feature/profile choices and A/B comparison
commits remain unchanged. There is no authorization for rollout, service repair
or restart, migrations, provider calls, data changes, extra servers, global cache
pruning, or greater build concurrency. Use Paseo CLI delegation only.

Memory reclamation has removed the earlier resource shortage. The observed
research-worker failure is a separate existing incident. For this execution,
permit that exact incident to be monitored during image-only experiments/builds
instead of requiring its repair before every build. This is not a release-ready
or service-health PASS and must remain explicit in all reports.

## Bounded research-worker exception

- Default execution remains strict. An explicit, owner-private, canonical JSON
  exception file enables only `lagrange-station-research-worker-1`, pinned to its
  exact container ID and image ID. No wildcard, arbitrary service, missing health
  target, or generic ignore-health option is allowed.
- Record issue/expiry times, initial restart count and the already observed
  `PRICE_CURATION_FAILED`/exit-2 incident. Each file expires within 24 hours;
  expiry is checked at each gate. Its immutable identity/hash is bound to a run
  and its resume state. A renewed file starts a new monitored run.
- The worker must remain running, without OOM. Permit its existing restart loop
  and the short starting window: health healthy/unhealthy/starting; restarting
  true with exit 2, or restarting false with exit 0 or 2. Refuse stopped/missing
  containers, other exit codes, missing/unknown health, changed IDs or images.
- Record every restart count. Counts cannot decrease, and growth from the
  exception observation must not exceed `ceil(elapsed_seconds / 30) + 2`.
  This bounds the observed approximately one-restart-per-minute loop while
  allowing short sample-boundary differences.
- All other listed services retain strict health, identity and restart checks.
  Keep MemAvailable >= 2 GiB, SwapFree >= 512 MiB, proven kernel-journal access,
  no new OOM, one actual build slot, Cargo jobs=2 and Compose parallel limit=1.
- Preserve current gate evidence and label an exception-assisted PASS as
  image-build-only with the existing service incident still open.

Actual control-plane service names observed at preflight are `docker.service`
and `containerd.service`. The Paseo daemon is not in a system `.service` cgroup;
do not invent a unit name for it. The coordinator also monitors its live process
identity while actual builds run. Include all eleven currently present long-lived
Lagrange containers, including the exceptional research worker, in health checks.
Preflight evidence: `/tmp/lagrange-build-resume-preflight-20260915T014823Z.json`.

## Order and acceptance

First implement and test the bounded exception while preserving strict defaults
and the completed offline contracts. Freeze a clean tool checkpoint. Run actual
existing cache smoke and L0/L1/L2 experiments sequentially, beginning with cold
and exact repeat, then the remaining correctness/route/failure/resume cases.
Fix observed defects in their owned scope; do not use cache deletion or relaxed
assertions to hide failures. Choose the product structure only at G2 using actual
results. Continue the original WP-4/5/6 implementation and acceptance sequence.

Use temporary images/caches and retained evidence for experiments. Long builds
run in background systemd units at lower CPU/I/O priority with 2–3-service logical
batches and one service invocation at a time. Production image verification uses
a clean candidate commit. Performance must include shared preparation and final
verification; no speedup is claimed from fixture results alone.

## Complete kernel journal capture

Actual fixture activity exposed an observation limit: the fixed interval
2026-09-15 02:18:21–02:48:21 UTC contained 1,014 kernel entries, so the existing
`-n 1000` range query correctly failed as truncated. A separate diagnostic read
found Docker network messages and no OOM matches; that diagnostic was not a gate
PASS. Evidence: `reports/real-gate.vliy1a/gates.jsonl` under
`/data/worktrees/3puw275b/build-verification-20260915-02bthtb5`.
More build activity must not silently discard earlier entries.

Replace only that tail-limited range collection with a complete streaming
`journalctl --no-tail` read of the same fixed since/until interval. Preserve the
current-boot access probe, all schema/boot/time checks, OOM matching, empty-stderr
and successful-exit requirements, fixed resume origin, and all service/resource
checks. EOF and successful process termination must be proven. Bound collection
to 10 seconds, 64 MiB stdout, 64 KiB stderr, 1 MiB per JSONL line and fewer than
100,000 records; exceeding a limit, partial output or any unknown result fails
closed. Retain completion/limit metadata, counts and hashes, not kernel message
text. The obsolete 1,000-record rejection is replaced by this complete-capture
contract for both strict and explicitly exceptional runs. No caller limit
override or new CLI bypass is introduced. Verify earlier-than-last-1,000 OOMs,
complete/incomplete output and the existing full offline suite before another
actual layout run. This is a coordinator correction within the approved
verification scope, not a service-health exception.
