# Report worker decision

The Compose skeleton names `report-worker`, but the repository does not have
a separate report-generation domain to deploy:

* `jobs.job_type` accepts a generic `report` label, but no executable claims
  or settles `report` jobs. The only queue consumers are the typed
  recommendation, backtest, and Paper runners.
* Report and benchmark views are read paths. `crates/api-server/src/repos/parity.rs`
  computes the backtest-vs-Paper parity report on read; the metrics/artifacts
  repositories read the already validated `backtest_*` and `result_artifacts`
  rows. No report artifact producer or report payload contract exists.
* The entitlement registry's `Report`/`DevReport` values describe authorization
  surfaces, not a worker protocol. There is no report migration table, CLI,
  Python package, or result schema that a small standalone worker could
  implement without inventing behavior.

Consequently a separate report worker is not production-ready and should stay
disabled/removed from the deployment graph until a report job contract is
specified (payload, owner/entitlement boundary, artifact manifest, idempotent
publication, and queue settlement). A placeholder that sleeps would be worse:
it would report healthy while permanently leaving report jobs queued.
