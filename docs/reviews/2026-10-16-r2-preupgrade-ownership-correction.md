# R2 pre-upgrade ownership and native failure-stage correction

Roadmap R2-04/R2-09; Wave85. Source correction after full terminal recovery of
`6572bff`/CI38896226384 attempt1. No execution or operational acceptance.

The [prior report](2026-10-16-r2-first-rollout-metadata-ci.md) preserves all8
job/log outcomes,7 checkout SHAs,1114 pins/504 blobs and97 printed PASS/1 ERROR
among98 definitions. Its positive native restore failed PGDATA ownership.
Three native negatives accepted any RuntimeErrorCode and therefore did not
prove their intended boundaries. The rejected uid/gid/mode tuple was unprinted
and remains UNKNOWN; this correction does not fabricate that observation.

## Bounded changes

The [recovery library](../../scripts/ops/lib/production-preupgrade-recovery.py)
adds exactly999/0/0700 to the two existing PGDATA ownership tuples. The
PostgreSQL user still owns the directory; group/other have no permission in the
added case. No metadata is rewritten. The existing PG_VERSION/control owners,
complete restored-manifest comparison, ACLs, descriptor/name binding and private
volume-parent checks remain. Root-group0750 and all other new combinations
are rejected. The [runbook](../runbooks/r2-preupgrade-recovery.md) links pinned
official source evidence and explicitly distinguishes it from native metadata.

The [native verifier](../../deploy/db/preupgrade-recovery-verify.sh) reserves
exit73 for control-file/shutdown-state rejection. The library maps that to
`PREUPGRADE_VERIFY_CONTROL` only after observing the exact owned clone stopped,
without OOM, runtime error or restart, at that exact exit. Other failures retain
their original code. Existing owned-resource cleanup remains mandatory.

The [native fixture](../../scripts/ops/production-preupgrade-recovery-docker-self-test.py)
requires exact errors for catalog mismatch, injected lost create response and
corrupt control. The lost-response case also requires an actual returned
container identity. Finite source/restored metadata is collected per named
case and printed at class cleanup so it cannot split unittest result lines.
Its positive schema59 round-trip and all previous fixture-parent protections
remain required.

Two [filesystem regressions](../../scripts/ops/production-preupgrade-recovery-self-test.py)
exercise actual private directories: all3 permitted tuples survive archive
and restore without normalization, while9 wrong owner/group/mode combinations
fail the precise cluster-owner check. All98 prior definition names remain;
the new inventory has100 definitions, all requiring current-candidate evidence.

## Verification and limits

Coordinator direct source/diff review only; no independent integrated review
claim. No local project import/parser/compiler/test, Docker, SQL, provider,
root or production action. No new worker. The reviewed change is4 executable
files plus this documentation and the runbook; workflow and unrelated sources
are unchanged. Private draft/source evidence lives under
`target/r2-daily-beta/wp-4-integration/wave85-*`.

Freeze exact bytes and100 named definitions. Query all events/workflows for the
exact candidate, verify remote SHA and dispatch once only if absent. Recover
native source/restored metadata, all7 preparation/native rows, both new
filesystem rows, all carried required groups and all8 jobs/logs. The earlier
failure and unknown metadata remain preserved. Runtime images/approved-base
host flow, diagrams/integrated review, G3/G4, Owner and three trading days are
still open; no R3 or final C_R2 completion follows from this source change.
