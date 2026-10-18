# R2 Wave103: preserve finite producer failure evidence

R2-04/R2-09 remain DOING. The Owner's Wave102 “이어서 진행해” authorized both
ordered phases once; both are now consumed. Candidate512625a images are accepted,
but its approved-base integration failed. No successor host attempt is authorized.

## Verified host outcome

Candidate `512625a31fdb59d284cbd13fa6f2334a4737cc13` image maintenance dispatched
once at13:01:51.009414UTC on2026-10-18. All12 images,704 saved-byte checks,49 gates,
7 bundles/18 binaries,2144 source pins and11 controls matched. The manifest hash is
`cc5994bc79d4a11cd187b83457b4a3ec1c7b75d777d075bae39b67695bc6df8e`.
Same Owner ID/image became healthy13:19:00.865571UTC; build and independent recovery
successfully deactivated13:19:00.876428 and13:19:06.314685UTC. No processes remained.
Exact11 production identities/states and postrestore read-only settlement passed.
The release remainsfce9b90, research remains stopped with exit2.

Image closure `target/r2-daily-beta/wp-8/maintenance-512625a-a6/attempt-closure.json`
SHA256 `6566857ad75d9b7048582fe0f9a03c6f81fb5b8109ecbf019252e3cc1ae65f26`.
An initial coordinator receipt reader expected `startedAt` rather than the current
intent's `at`. It failed before Docker/kernel reads; its failed receipt/helper are
retained beside the corrected exact timestamp recovery. No build was repeated.

Fresh wholeflow admission passed13:20:12UTC. The third actual integration attempt
(fourth prepared packet) dispatched once13:20:18.312216UTC and failed with
`WHOLEFLOW_DOCKER_FAILED` at13:20:25.585082UTC. Bootstrap, migration and both control
processes exited0; the API started. The actual producer started13:20:23.645844UTC
and exited1 at13:20:23.676942UTC. Only `PREPARED_SYNTHETIC_EXTENSION` was recorded.
First admission, publication, replay, restart and capture/restore were not reached.
The original producer's inner failure code is **UNKNOWN**: the driver discards
captured output when Docker returns nonzero. This is a demonstrated evidence gap,
not proof of the producer's underlying cause or that later phases would pass.

All7 current-run containers were destroyed; the exact network was destroyed and
nonce containers/network/stage are absent. Unit PIDs/cgroup/exact driver process
are absent. Production11 states/current release are unchanged; the bounded kernel
range contained0 OOM events. PG/API exit137 occurred during private cleanup.
Final original-base byte comparison and independently printed62 schema audit rows
were not recovered and remain UNKNOWN.

Wholeflow closure `target/r2-daily-beta/wp-8/wholeflow-512625a-a4/attempt-closure.json`
SHA256 `4d3770a7550ebec06cb1041803ab5fb34f960f4a3b9c6849dc92328b379a1e8e`
pins15 receipts. Its status is
`FAILED_TERMINAL_NEW_OWNED_RESOURCES_ABSENT_PRIOR_VOLUME_PRESERVED`.
The previous0120 anonymous volume remains unchanged/unreferenced. Its ownership
is unproven; do not mount, inspect contents, delete or prune it. Previous cleanup
remains INCOMPLETE even though this attempt's resource absence was established.
Preserve all failed units, root results, accepted images and immutable evidence.

## Bounded source correction

Two source files change: the wholeflow driver and its mocked fixture self-test.
Before raising the unchanged nonzero-exit failure, attached one-shot commands
record their fixed service identity, Docker command exit code and complete stdout/
stderr byte counts and hashes. These are not independent container-state results.
Only producer/admitter output may contribute exact two-field daily startup/cycle
events with closed code/outcome allowlists taken from the Rust source. Duplicate
JSON keys, additional fields, unknown labels, malformed data, oversized lines,
oversized output and excessive diagnostic rows cannot enter the receipt as text.
DB clients/worker use hash-only evidence. Timeout/OS uncertainty remains unknown.
Raw bytes stay in memory; no argv, environment, SQL, provider bodies or free-form
error text is persisted. There are no new subprocesses, retries or guard changes.

Nine required regressions cover finite stderr capture/no retry, hostile JSON,
stream/service separation, byte/row bounds, nonzero with a success-shaped outcome,
unchanged running/exit/OOM guards, timeout uncertainty, diagnostic scope rejection,
and unscoped SQL confidentiality. They are WRITTEN_UNEXECUTED for this successor.
Coordinator static review and `git diff --check` only; no local project parser,
test, compiler, helper execution or new independent review is claimed.

## Next evidence and authorization boundary

Freeze all prior141 required definitions plus9 new ones,15 image outcomes and
32 prior wholeflow cases plus9 new ones; retain inherited input pins and all
Rust/strict Clippy/format/PG/Web/smoke acceptance scopes. A separately justified
new candidate receives exactly one CI and one functional smoke after remote SHA
and all-workflow/all-event exact-head absence checks. Do not rerun512625a runs.
Recover all jobs/steps/pages,9 full log hashes,8 actual checkout SHAs and named
outcomes before acceptance. Source and frozen inputs stay unchanged while active.

After hosted recovery, prepare a concrete successor image/isolated integration
packet before one new Owner decision. No new host execution follows from this
diagnostic correction. G4 recovery/backup transition, deployment, Owner use,
three consecutive trading days and final C_R2 remain open.
Heartbeatf7c37450 was confirmed13:24:06UTC; bounded handoff was persisted before
cad36e21 deletion. Renew before2026-10-19T00:00UTC and before tick47; expiry01:24:06UTC.
It is same-agent bounded continuation, not a worker or daemon-outage protection.
