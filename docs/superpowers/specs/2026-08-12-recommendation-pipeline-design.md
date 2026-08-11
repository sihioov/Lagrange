# Fixed-ETF Recommendation Pipeline Design

**Date:** 2026-08-12  
**Status:** Approved for implementation  
**Scope:** End-to-end recommendations for the existing fixed 11-instrument Korean ETF universe

## 1. Outcome

Lagrange Station will turn the recommendation surface from a queue-only shell
into a working product flow. A member can submit a recommendation run for an
active strategy configuration, a dedicated worker computes the strategy's
target portfolio from the immutable curated dataset, and the completed result
appears in the latest/history UI with factor evidence, reasons, exclusions,
cash weight, and provenance. A scheduler creates the same work after the
market-data close, and scheduled results can feed the next-session Paper target
for accounts already bound to that strategy configuration.

The first release is deliberately limited to
`configs/universes/kr-etf-core-v1.yaml`. It does not claim to recommend from all
KRX-listed shares. Dynamic individual-stock universes remain Phase 4 work and
require broader KRX data rights.

## 2. Existing State and Missing Seam

The repository already contains:

- a fixed, immutable 11-ETF universe and publication contract;
- Rust factor computation and deterministic selector primitives;
- five versioned strategy packages and Python target generators;
- leased PostgreSQL jobs with retries, attempts, cancellation, and sweeping;
- `recommendation_runs`, `recommendation_items`, and `target_portfolios`;
- create/list/get/latest recommendation HTTP routes;
- recommendation report/history UI and entitlement gates;
- Paper accounts, strategy bindings, pending targets, and a Paper runner.

The create route currently inserts a `PENDING` recommendation run and queues a
`recommendation` job, but no process consumes that job. Tests simulate success
by updating the tables directly. The first-run UI also cannot submit a run when
`latest` returns `RESOURCE_NOT_FOUND`, and no production seam converts a
successful recommendation into a persisted target portfolio or a Paper pending
target.

## 3. Chosen Approach

Three approaches were considered:

1. **Compute synchronously in the API.** This is small but makes request
   latency, process failure, retry behavior, and cancellation unacceptable for
   factor work.
2. **Create a Python-only queue worker.** This reuses target generators but
   duplicates queue leasing, tenant ownership checks, database state
   transitions, and data-rights enforcement.
3. **Use a Rust recommendation runner with an isolated Python target step.**
   Rust owns the job, validates inputs and data, computes factors with the
   existing factor engine, and persists the result. A small child process calls
   the existing versioned target generator so strategy semantics are not
   reimplemented.

Approach 3 is selected. It follows the existing backtest-runner boundary,
preserves one Rust factor definition and one target-generator definition, and
keeps infrastructure failures separate from deterministic strategy failures.

## 4. Product Scope

### 4.1 Included

- Manual recommendation runs for an active, owned strategy configuration.
- All five shipped baseline strategy packages when their required factor and
  history are available.
- The fixed 11 Korean ETF universe only.
- A dedicated recommendation job runner with typed claims, retries, heartbeat,
  sweep, and graceful shutdown.
- Deterministic factor computation at the requested market-data close.
- Existing Python target generators invoked out of process with a strict JSON
  request/response contract.
- Atomic publication of recommendation items, target portfolio, provenance,
  and terminal run state.
- A 16:30 Asia/Seoul scheduler with startup catch-up and idempotent daily
  planning.
- Automatic Paper pending-target creation only for scheduled runs and only for
  accounts actively bound to the same strategy configuration with an explicit
  auto-apply opt-in.
- A usable empty-state UI that lets a member choose an active strategy and
  create the first run.
- Deployment, health, operational documentation, and focused integration/E2E
  coverage.

### 4.2 Excluded

- KRX-wide individual-share discovery or a dynamic universe.
- Fabricated live market data or a claim that the current synthetic fixture is
  a live feed.
- Live brokerage orders.
- User-authored strategy code or arbitrary Python module paths.
- Automatic Paper publication from an ad-hoc manual preview.
- Changes to strategy formulas that are unrelated to making the existing
  packages executable.

## 5. Architecture

### 5.1 Typed queue ownership

`JobQueue` will gain a claim operation filtered by an allow-listed job type.
The existing backtest runner will claim only `backtest`; the recommendation
runner will claim only `recommendation`. One worker must never claim and fail a
job that belongs to another worker class.

The existing lease, heartbeat, orphan sweep, retry budget, and settlement
contracts remain authoritative. The recommendation runner does not introduce a
second queue implementation.

### 5.2 Recommendation runner

A focused Rust module and binary will:

1. claim one `recommendation` job;
2. parse `run_id`, `strategy_config_id`, and `as_of` from its payload;
3. read the immutable deployment dataset pin carried by the job and attest it
   against the matching `dataset_versions` row and manifest hash;
4. re-read the run and active strategy configuration while binding both owner
   IDs from the claimed job;
5. reload the current entitlement rows and authorize recommendation use for
   the run's date;
6. resolve the immutable fixed-universe manifest and verify that the curated
   dataset contains exactly compatible canonical instruments;
7. require a usable, non-blocked dataset version and the requested close;
8. derive the exact factors and lookback from the strategy plus its validated
   parameters;
9. compute the factor snapshot in `spawn_blocking` with the Rust factor engine;
10. invoke the closed-set Python target generator in a child process;
11. validate the returned portfolio before any database write; and
12. publish the result and settle the queue claim in one database transaction.

The runner will use explicit repository/data/artifact paths and an explicit
`uv` executable. No request or database row may choose a Python module path;
strategy IDs resolve through the shipped baseline allow-list.

The dataset pin contains the database dataset-version identity, logical dataset
ID/version, curated numeric version, storage root, and manifest hash. It is
chosen from deployment configuration and verified at submission; the worker
never silently selects the newest `READY` row. Existing historical requests
are accepted only when their immutable pinned store ends at the requested
as-of ceiling. Missing or later unpinned data blocks the run instead of
weakening the factor engine's future-row guard.

### 5.3 Target-generator child contract

A small CLI under `nt/` accepts a JSON file containing:

- strategy ID and immutable version;
- schema-validated member parameters;
- as-of date;
- canonical fixed-universe members;
- finite raw factor values keyed by instrument and factor; and
- provenance identifiers supplied by Rust.

It imports only `strategies.<allow-listed-id>.target`, calls
`generate_target`, attaches the supplied provenance fields, and writes one
strict JSON result. Standard output is not used as a data channel. The parent
requires a zero exit status, a size-bounded response, the expected
`strategy_id@version`, the same as-of date, only universe instruments, finite
weights and scores, unique instruments, and weights plus cash equal to one
within the declared tolerance.

Python cannot read PostgreSQL, choose a dataset, claim a job, or publish a
result.

### 5.4 Result publication

Successful publication and queue settlement are one transaction:

- lock the `PENDING` run owned by the job owner;
- reject a stale or already-terminal run without overwriting it;
- insert every selected and excluded `recommendation_item`;
- insert one `target_portfolios` row containing selected positive weights and
  cash weight;
- store a structured summary with dataset version, universe snapshot ID,
  factor snapshot hash, portfolio snapshot ID, selected/excluded counts,
  cash weight, trigger kind, and warnings;
- mark the run `SUCCEEDED`;
- finalize the matching job attempt and job through a guarded, lease-valid
  settlement using the same transaction.

A crash before commit leaves no partial items or target. A retry recomputes the
same deterministic bytes and publishes once. Database constraints prevent
duplicate instruments within a run and duplicate successful publication.

The schema will persist the queue job ID and trigger kind on the run, enforce
one target portfolio per succeeded recommendation, and standardize the
provenance key as `dataset_version`. Worker grants will be extended only to the
recommendation and target rows it must write. Existing and new migration
contract tests must prove that unrelated tenant writes remain denied.

### 5.5 Scheduler and Paper bridge

The recommendation service runs a daily scheduling cycle at 16:30 KST after
the research-data worker's default close publication time. On startup it first
checks whether the current eligible close was missed. The cycle:

1. reads the latest published KRX trading close and usable dataset metadata;
2. stops without creating member-visible output when the close is absent,
   stale, synthetic in production, or blocked;
3. finds active strategy configurations referenced by an explicitly enabled
   recommendation-automation binding;
4. creates at most one scheduled run/job per owner, config, close, and dataset
   version using a database idempotency key; and
5. lets the normal recommendation runner process those jobs.

When a **scheduled** run succeeds, the publication transaction also locates
active Paper accounts bound to that exact config whose binding has explicitly
enabled automatic recommendation application, then queues a `pending_targets`
row for the next KRX trading session. Existing uniqueness on
`(account_id, effective_date)` keeps restart/catch-up safe. Manual runs publish
recommendation and target-portfolio records but do not change Paper accounts.

If there is no next trading session in the published calendar, the
recommendation may succeed but Paper planning is blocked and recorded as a
warning; no guessed calendar date is used.

The Paper runner rechecks entitlement and dataset readiness immediately before
execution. Revocation or newly blocked data settles that target with a durable
non-executed blocked/skipped reason; it never leaves an order-eligible target
behind and never silently executes using authorization from an earlier day.

### 5.6 API and web behavior

The create API continues to be asynchronous. It validates ownership,
configuration activity, entitlement, date syntax, the configured immutable
dataset pin, per-owner queue capacity, and idempotency before returning
`201 PENDING`. Run creation and job submission use one actor transaction (or a
transactional outbox with the same externally observable guarantee), so a
queue failure cannot leave an immortal `PENDING` run. The run persists the
created job ID.

The recommendation page will fetch active strategy configurations together
with latest/history. It always renders a run form when at least one active
configuration exists, including the no-result state. The form uses a strategy
selector rather than inheriting the latest run's configuration. The latest
report continues to show selected candidates, exclusions, weights, factors,
and reasons, and will additionally surface cash weight and provenance from the
summary. History is newest-first and preserves pagination. After submission,
the page polls the created run by ID; a new pending/failed run does not hide the
last successful report.

No recommendation payload is rendered when the fresh entitlement gate denies
use.

## 6. State and Error Model

The recommendation run and queue job carry related but distinct state:

| Condition | Recommendation run | Queue behavior |
|---|---|---|
| Success | `SUCCEEDED` | settle success |
| Revoked/expired entitlement | `BLOCKED` | permanent settlement |
| Missing/stale/blocked data or insufficient history | `BLOCKED` | permanent settlement |
| Invalid/unsupported config or invalid target output | `FAILED` | permanent settlement |
| Temporary database, filesystem, or child-process launch failure | remains `PENDING` until retry budget ends | retry with backoff |
| Retry budget exhausted | `FAILED` with sanitized infrastructure code | terminal failure |
| Process crash | unchanged until lease sweep | orphan and requeue |

Errors stored for users contain stable codes and safe details. Child stderr,
provider bodies, paths containing secrets, and database connection strings are
never copied into recommendation summaries or notifications.

## 7. Data and Security Invariants

- Every run, config, item, target portfolio, Paper account, and pending target
  is matched to the claimed job owner.
- Worker cross-tenant reads are permitted only where needed; writes use exact
  owner predicates and the smallest grants required by the runner.
- Entitlements are reloaded at execution time, not trusted from submission.
- The universe comes from the versioned manifest, never from a request.
- Factors use only rows at or before `as_of`; future rows fail closed.
- The target child receives raw factor quantities, not normalized z-scores,
  because several strategy thresholds have physical return/volatility units.
- Strategy module resolution is a compiled allow-list.
- Recommendation output is a proposal, not investment advice, and never emits
  live orders.
- Production mode refuses synthetic source metadata.

## 8. Testing

### 8.1 Rust unit and contract tests

- Job-type-filtered claims do not steal work from other runners.
- Exact factor requirements are derived for every allowed parameter variant.
- Current-close snapshots contain no future data and reject non-session dates.
- Portfolio validation rejects wrong versions, foreign instruments, duplicate
  rows, non-finite values, invalid sums, and malformed provenance.
- Result publication is atomic and idempotent across retry/crash seams.
- Error classification maps to `BLOCKED`, `FAILED`, or retryable behavior.
- Scheduler catch-up and concurrent cycles create one scheduled run.
- Paper bridging happens for scheduled runs only and queues the next published
  trading session exactly once.

### 8.2 Python tests

- The CLI supports all five allow-listed baseline generators.
- Unknown strategies and unknown fields are rejected.
- Output is deterministic for identical input bytes.
- Generator errors produce bounded structured status without partial output.
- The CLI cannot import an arbitrary module supplied in input.

### 8.3 PostgreSQL/API tests

- Happy path: POST -> queued job -> runner -> items/target -> latest/history.
- Tenant isolation across run, config, item, target, and Paper binding.
- Revocation between POST and execution produces `BLOCKED` and no items.
- Queue failure compensation prevents immortal `PENDING` runs.
- Worker-role grant tests prove allowed writes and forbidden unrelated writes.
- Duplicate scheduling and retries produce no duplicate publication.

### 8.4 Web and E2E tests

- A member with a saved active config can create the first run from the empty
  page.
- Multiple configs are selectable.
- Pending, succeeded, failed, blocked, no-config, and entitlement-denied states
  render accurately.
- The completed report shows candidates, exclusions, weights, cash, factors,
  reasons, as-of date, and provenance.

## 9. Deployment and Operations

Compose will run a real recommendation service instead of a sleep placeholder.
It receives read-only curated/universe mounts, worker database credentials,
an explicit repository/`uv` path, KST schedule settings, retry/lease settings,
and a production-mode flag. It runs without broker credentials and without
write access to Raw or Curated data.

Health distinguishes:

- process alive;
- database/queue reachable;
- last scheduler cycle completed;
- current market close unavailable or data blocked; and
- worker backlog/oldest queued age.

`docs/STATUS.md` and deployment runbooks will state that the feature is
end-to-end executable against approved curated data while real KRX production
activation still depends on licensed provider credentials, endpoints, and
operator provisioning.

## 10. Completion Criteria

The feature is complete when:

1. a real queued recommendation job is consumed without test-only SQL;
2. all five supported baseline configurations either return a correct result
   or a typed honest data/config block;
3. the latest/history API and web page display the persisted result;
4. the first run can be started from the UI;
5. one scheduled close produces one result and one next-session Paper target
   per eligible bound account;
6. entitlement, stale-data, synthetic-production, tenant, crash, and retry
   cases fail closed;
7. focused Rust, Python, PostgreSQL, web, E2E, formatting, lint, and Compose
   checks pass; and
8. documentation does not imply that the licensed live KRX feed is active.
