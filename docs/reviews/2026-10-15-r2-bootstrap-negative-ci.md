# R2 I0d bootstrap negative and rollback CI recovery

Candidate `dd1b92f3dc3c2bdc151a763e90fba734aa7af747`,
[CI 38670368360](https://github.com/sihioov/Lagrange/actions/runs/38670368360),
attempt 1, completed **FAILURE** at 2026-10-15 02:34:01 UTC. All **83 named I0d
cases passed**. The remaining failure is one existing WebSocket transport test;
the coordinator does not accept the whole candidate or final C_R2.

## Identity and complete recovery

Repository sihioov/Lagrange, excited-dolphin, CI/ci.yml, workflow_dispatch.
Remote SHA and all-event exact-head count zero preceded one dispatch at
02:19:24 UTC. No rerun, cancellation or duplicate dispatch occurred.
All eight terminal jobs, 84 step states (80 success, two failure, two skipped),
eight completed logs and seven actual checkout SHAs were recovered. The required
aggregate has no checkout. All 76 frozen pins and 56 candidate blobs matched.
Later documentation HEAD does not change the verified candidate.

| Job | Outcome | Evidence |
| --- | --- | --- |
| workspace-tests | FAIL | 99 required Linux Rust names and six separate chart names each have direct PASS rows. One other transport test failed; Cargo exit 101. QA cleanup passed. |
| clippy | PASS | --workspace --all-targets --all-features --locked -- -D warnings. |
| format | PASS | cargo fmt --all -- --check. |
| postgres-integration-validation | PASS | APPROVED; all 83 named bootstrap cases, PG59, 12 DB labels, roles/catalog/0039/cleanup. |
| web | PASS | OpenAPI, lint, typecheck, tests, build and E2E. |
| policy | PASS | All wrapper steps and ten directly printed CI-contract names. |
| python-tests | PASS | 485 passed, ten skipped, seven warnings. Quiet individual nodes remain unverified. |
| required | FAIL | Workspace failure propagated; exit 1. |

The 99 Rust names retain the 30 wire/18 S1/one collector/two helper/19 private S2/
29 integration S2 inventory. S1 non-Unix and S2 non-Linux definitions remain
excluded. Aggregate log counts do not replace named evidence.

## Bootstrap evidence and limits

The official disposable PG lane invoked the unchanged production bootstrap.
Three positive controls, seven role-attribute cases, two membership cases, eight
direct/transitive protected-role paths, six CREATE cases, 56 credential-file
cases and one late transaction rollback case each have a distinct PASS outcome.
The rollback fixture observes earlier statements, the native lock-timeout error,
new-role absence and complete catalog-state equality after rollback. Full role,
membership and ACL snapshots and credentials were never persisted in diagnostics.

Both migration passes match all 59 candidate SHA384 checksums and the second
pass is NO_OP. Both daily-admitter helper labels and the no-SKIP gate passed;
their unprinted nested result rows and numeric Cargo exits remain UNKNOWN.
Initial catalog, SQLSTATE 23505 conflict, four rolled-back 23514 negative probes,
staged rerun, seven role logins per lane and 0039 guard passed.
Full release cleanup is PASS_NO_OWNED_CONTAINERS. This is bootstrap transaction
rollback evidence, not migration 0060 DOWN, provisioner filesystem rollback or
runtime cross-principal access evidence.

## Remaining workspace failure

`buffered_pingpong_and_incomplete_following_frame_preserve_prior_receipts` failed
in its incomplete-frame half at market_stream_transport.rs:882:36. The decoded
workspace log has the failed row at 4014, panic location at 4057 and `Closed` at
4058. No other named FAILED row was present. This test source was unchanged by I0d.

Static inspection finds a fixture-lifetime race: session_with_buffered_followup
closes its incomplete-frame server after 200ms while the client uses a 50ms sleep
before returning to assertions. CI confirms Closed, but does not expose the exact
thread schedule. Wave35 will replace this timing dependency with bounded explicit
start, write-ready and assertion-complete signals in that test file only. The
production fail-closed transport behavior and test assertions must be preserved.

## Retained evidence

Private directory target/r2-daily-beta/ci/dd1b92f is 0700; receipts are 0600.
Raw logs were memory-only and cleared after sanitized recovery.

- ci-evidence.json: 5,105 lines, SHA256
  `8331eac9b1b17db6936d61f487cb3e81760b76daaaafc71505d8f0f64e4eb48b`.
- pg-evidence.json: SHA256
  `392ae1befa9ec137cf691e73b6a5f34d7295c20d3eea5e8f02d30de0dac00baa`.
- Frozen wave34 inputs/inventory and prior C05 acceptance remain unchanged.

R2-04/09 remain DOING. No new delegation, local executable verification, main
merge, provider request or production action occurred. 0060, provider/copy
admission, I6 diagrams before structural main integration, runtime/Owner,
three trading days and final C_R2 remain open.
