# R2 Ralph elapsed-time audit — 2026-10-16

Owner asked why the work was still running after 17 hours. This audit uses the
native goal, Git history and persisted exact-candidate CI receipts. No new
implementation, worker, CI dispatch or production action was started during it.

At 2026-10-16 13:33:36 KST the active Ralph goal had run for about 18 hours
17 minutes, from 2026-10-15 19:16:41 KST. R2 is not complete and actual
three-trading-day observation has not started.

| Observed measure | Result |
|---|---|
| Distinct candidate CI runs since goal start | 36, all terminal |
| Conclusions | 15 success, 21 failure |
| Sum of CI run elapsed intervals | 43,205 seconds: about 12 hours |
| Median run interval | 21 minutes 30 seconds |
| Overlap between these run intervals | None |
| Commits since goal start | 81: 36 source/test/CI and 45 documentation |
| Tracked source/test/config change | 138 files, +35,612/-514 lines |
| Tracked documentation change | 79 files, +9,697/-12 lines |

CI elapsed time is not claimed as idle time: source review, evidence collection
and some isolated draft work overlapped CI. The serialized whole-workspace
verification cycles nevertheless dominate elapsed time. Progress was real, but
the coordinator fragmented implementation into too many separately verified
packages, repeatedly discovered compile/lint/fixture defects in full CI, and
spent excessive effort maintaining repeated wave records. This should have
been corrected earlier. Passing a small package was never overall R2 completion.

The most recent candidate `719a2e2820837c96af4b183b87659de6f58740f7`,
[CI38849276458 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38849276458),
also failed. All eight jobs/full log digests, seven actual checkouts, 1,016 pins
and 462 committed blobs were recovered. Format, strict Clippy, policy, Python,
Web and dedicated PostgreSQL verification passed. Workspace tests and the
required aggregate failed. The one named workspace failure was
`owner_equity_v2_repository_queue_and_publication_are_atomic_and_actor_scoped`,
at the recovered publication assertion in
[the frozen test](https://github.com/sihioov/Lagrange/blob/719a2e2820837c96af4b183b87659de6f58740f7/crates/api-server/tests/owner_equity_v2_db.rs#L969).
Its underlying cause has not been established; it is not dismissed as flaky.
All 29 new activation and eight transport tests passed, together with prior
required daily groups, but full-candidate acceptance remains withheld.

Private receipt: `target/r2-daily-beta/ci/719a2e2/ci-evidence.json`, 9,999 lines,
SHA256 `ea48ffebc874a749fd4dd46022a9c9e5864407f75279907987079b7582f256c5`.
The per-run timing list is in the private
`wp-4-integration/2026-10-16-ralph-elapsed-audit.json` receipt. Earlier successful
scope acceptances and every failed run remain preserved. No old run is repeated.

The remaining completion path is fixed to these delivery groups:

1. Close known verification failures and complete consistent backup/restore
   reconciliation. The isolated backup draft currently contains only two new
   files (snapshot helper and 25 written tests), none executed or accepted.
2. Verify the complete approved-base first-day/multiday path, update both
   architecture diagrams/evidence, and close integrated review. Direct
   coordinator review does not substitute for independent review, which is
   still open under the no-further-delegation restriction.
3. Prepare the concrete release/maintenance/catch-up plan, resolve any actual
   G4 permission differences, apply within authority, verify real Owner use,
   then observe three actual consecutive trading days and assess final C_R2.

Continue Ralph with these completion groups, batching coherent source/test
changes before each new exact-candidate full CI. Do not create a fresh full CI
for every helper or documentation update. Existing local-execution, worker,
provider and production restrictions remain; no tests or acceptance gates are
waived. Diagnose the current failure before choosing the next candidate.
The audit does not provide a reliable coding finish time. Three trading days
are an additional operational observation period after successful activation,
not a promise that waiting three calendar days from now completes R2.


## Follow-up diagnosis and bounded fixture correction

A read-only extraction from the same workspace job `116806660574` recovered
`publish recovered: STALE_CLAIM`; full-log SHA256 matched the recovered receipt:
`41ff6f697437242eaf03c79ecf8af7b7263434bce1291e532b1c226bfeeccd07`.
Only the fixed assertion/code, digest and identity were persisted in the private
`owner-equity-recovery-code.json` supplement; raw logs remained in memory.

The test used a 25ms lease to force a crashed attempt to expire, then reused that
same queue for legitimate recovered publication through multiple real database
checkpoints. The queue heartbeat rejects an expired lease; the worker maps that
result to `STALE_CLAIM`. The correction uses the test's existing normal 5s queue
only to claim and process the recovered attempt. The 25ms crash, expiry/sweep,
orphan result, zero prior admissions and exactly-one final admission remain.
Additional assertions require a new attempt ID, the next attempt number, and
rejection of the old worker even after recovery is claimed with the normal lease.
Production queue/worker behavior, lease settings and database contracts are unchanged.

The same exact test change is present in the primary worktree and the registered
isolated backup draft. Direct source review, scoped edition-2024 rustfmt and
whitespace checks passed. Compilation and execution of the changed test remain
UNEXECUTED. No fresh CI was dispatched; verification is grouped with the coherent
backup/reconciliation candidate. The original failed run remains failed.
