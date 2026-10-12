# R1 release readiness — preliminary input review

2026-10-12. **Not accepted; no release build or production mutation performed.**
The candidate commit and its required hosted CI results remain pending. This
read-only preparation does not satisfy R1-09.

## Official build contract

Use `scripts/ops/build-production-images.sh` from a clean checkout of the exact
accepted commit. The image-only env file contains only the documented inactive
entitlement hash sentinel; never copy the operational Compose env. The official
root-only apply creates a new strict V2 manifest, validates all twelve image IDs
and OCI revisions, and builds one service at a time in its background systemd
unit. Cargo jobs2, Compose parallel1 and the documented health/resource gates
remain mandatory. No migration, release activation or provider call is authorized.

Source references: [builder](../../scripts/ops/build-production-images.sh),
[build gate](../../scripts/ops/lib/release-build-layout.sh), and
[release runbook](../runbooks/production-release-and-backup.md).

## Host observation at 12:04 UTC

The coordinator read only the exact `lagrange-station` Compose project's
container names/states/statuses and docker/containerd unit state. Ten monitored
containers were running and reported healthy, including
`lagrange-station-owner-equity-v2-runner-1`. The research worker was exited with
code2, reported as three weeks old. Both control units were active/running.
No container env, provider log or response body was read; no container or unit
was started, stopped, signaled or modified.

The ordinary build health predicate therefore has an unresolved research-worker
input. The existing legacy research exception is specifically a **running**
known-incident contract; it cannot cover this stopped worker. The optional
drained-reader contract requires both named readers stopped, with Owner V2 exit0
and research exit2, exact current observations, a commit-bound record and a
separately authorized bounded lifetime. The observed running Owner V2 does not
meet that contract. No old exception or attestation has been read, reused,
created or renewed.

Finish source validation and commit-pinned CI first. Then run the official
read-only plan/preflight against the clean candidate and record concrete build
inputs. A production lifecycle action or new exception is outside the current
R1 image-only execution scope and must not be inferred from this report. Leave
R1-09 open until its actual image/manifest evidence exists.
