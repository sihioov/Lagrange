# Backtest artifact publication and cleanup

The Rust backtest runner writes each successful result to a fresh immutable
generation:

```text
<artifact-root>/backtest/runs/<run-id>/generations/<40-hex-code-commit>/<publication-uuid>/artifacts/
```

`result_artifacts.parquet_path` stores the same path relative to the API
artifact root (`/data/artifacts` in Compose). The publication UUID is generated
for one execution and is never reused. The runner validates the baked
`LAGRANGE_CODE_COMMIT`, rejects symlinked path components, and never replaces an
existing generation directory.

Older deployed runners may still write the compatibility names
`<run-id>/artifacts` and `<run-id>/status.json`. New publication never reads,
removes, or replaces those names, so a pre-isolation replica that continues to
write after settlement cannot change the bytes or hash addressed by the DB
row. No symlink is used to bridge the old and new layouts.

## Failure and cleanup contract

The runner validates and atomically renames the attempt-private artifact
directory into the immutable generation before opening the result-settlement
transaction. If the transaction fails before `COMMIT`, it removes only that
exact UUID directory and leaves the shared run/commit parents alone. If the
queue settles cancellation, the transaction commits without result rows and
the same exact generation is removed. A successful `COMMIT` disarms cleanup
before returning to the caller.

An error waiting for `COMMIT` is ambiguous: PostgreSQL may have committed while
the client lost its response. The runner therefore retains that generation;
deleting it could make a committed `result_artifacts` row point at missing
bytes. It emits the stable event code
`BACKTEST_PUBLICATION_COMMIT_UNKNOWN`, logs the run ID, job ID, and filesystem
generation path, and writes `COMMIT_UNKNOWN.json` beside the `artifacts/`
directory. The marker is diagnostic metadata; the Parquet/manifest bytes are
never rewritten. The retention/reconciliation loop runs inside every long-lived
Rust backtest daemon and periodically compares generation paths with
`result_artifacts` and:

1. retain every generation referenced by a live DB row;
2. retain unreferenced generations for a crash grace period (at least one
   queue lease plus the database recovery window);
3. delete only unreferenced, expired generation UUID directories, without
   following symlinks and without traversing outside the configured artifact
   root; and
4. leave referenced generations untouched. The normal run-retention owner must
   remove the DB reference before a generation becomes eligible here.

The DB read is authoritative and is taken before any deletion. If either the
reference query or the active-attempt query fails, the pass deletes nothing and
emits `BACKTEST_RECONCILE_DB_UNAVAILABLE`. Runs with a linked `RUNNING` or
`CANCELED` queue attempt retain all their generations so an active attempt or
publication cannot race cleanup; a stale run row whose queue job is already
terminal or requeued is not treated as an active attempt.
Malformed run/commit/publication names, non-directory components, and any
symlink (including one nested below `artifacts/`) are skipped without following
or deleting the target. Only a fully validated exact UUID generation directory
can be removed, and only after `BACKTEST_RECONCILE_GRACE_SECS` has elapsed.

The runner also has a fail-closed claim gate. Before every claim it checks the
authoritative queued-backtest count and filesystem `f_bavail`; if the DB is
unavailable, free bytes are below `BACKTEST_MIN_FREE_BYTES`, or queued work is
at/above `BACKTEST_MAX_QUEUED_BACKTESTS`, claims stop and the `readiness`
probe exits non-zero. `healthcheck` remains a liveness probe but includes the
same `ready`, free-space, backlog, and reason fields in its JSON output. In
production all four settings are required (Compose supplies them from
`deploy/compose/.env`); there are only conservative development/QA defaults.

The daemon logs `BACKTEST_GENERATION_RECONCILE` counters for scanned,
referenced, active, fresh, deleted, malformed, symlink, and failed-delete
generations, plus `BACKTEST_CLAIM_BACKPRESSURE` diagnostics whenever the gate
blocks claims. These are the operational metrics until the deployment's
Prometheus exporter consumes structured runner logs.

Until that reconciler is deployed, operators must treat an ambiguous-COMMIT
diagnostic as a cleanup action item rather than manually deleting a generation.
The canonical compatibility directory is not a substitute for a referenced
generation and must not be used to repair a missing DB path.
