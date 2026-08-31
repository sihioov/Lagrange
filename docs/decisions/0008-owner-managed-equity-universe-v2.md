# ADR-0008: Owner-managed equity universe V2 foundation

- **Status:** Accepted; owner-only read-only runtime rollout authorized 2026-08-31
- **Date:** 2026-08-31
- **Audience:** Owner only
- **Related:** ADR-0005, ADR-0007,
  `docs/superpowers/plans/2026-08-31-owner-managed-equity-universe-v2.md`

## Context

The deployed individual-stock beta uses a sealed fixed-30 V1 universe and
filesystem-pinned artifacts. Making that list mutable would change durable V1
identity, approval, API, and replay behavior. The new product instead needs an
owner-managed list whose additions proceed asynchronously, preserve evidence
per instrument, and publish cross-sectional signals against an exact READY set.

WP-1 fixes only the domain and database contract. It authorizes no provider
call, credential use, collector, job, API, Web, image, Compose, installation, or
production activation. Account, balance, order, execution, and live-trading
surfaces remain outside this project.

## Decision

V2 is a separate owner-scoped database surface introduced by reversible
migration 0053. Nothing is copied from or written to the fixed-30 V1 universe,
artifact, approval registry, or API path.

### Runtime policy

`OwnerEquityUniversePolicy` is the shared typed policy. A newly provisioned
policy recommends 100 active instruments, targets 261 observed sessions, and
requires at least 121 observations. Persisted owner policy rows contain all
three values and have no SQL cardinality default, so operators can change the
active limit or coverage target without a schema migration or rebuild. The
minimum cannot be configured below 121, and the target cannot be below the
minimum.

Every membership request requires an existing owner policy. The policy row is
locked while active membership count is checked, making concurrent requests
fail closed at the configured limit.

### Membership lifecycle and retry contract

The exact persisted lifecycle is:

```text
REQUESTED -> VALIDATING -> BACKFILLING -> MATERIALIZING -> READY
```

The complete legal edge set is:

| From | To |
| --- | --- |
| `REQUESTED` | `VALIDATING`, `DISABLED` |
| `VALIDATING` | `BACKFILLING`, `FAILED`, `DISABLED` |
| `BACKFILLING` | `MATERIALIZING`, `INSUFFICIENT_HISTORY`, `FAILED`, `DISABLED` |
| `MATERIALIZING` | `READY`, `INSUFFICIENT_HISTORY`, `FAILED`, `DISABLED` |
| `READY` | `DISABLED` |
| `INSUFFICIENT_HISTORY` | `REQUESTED`, `DISABLED` |
| `FAILED` | `REQUESTED` only when retryable; otherwise `DISABLED` only |
| `DISABLED` | none |

The database trigger and Rust enum implement the same graph. A membership starts
only in `REQUESTED`. An Owner cannot directly write worker states: the app role
may insert a request and call narrow retry/disable functions, while workers may
update only lifecycle/evidence columns. Physical deletion is denied. Re-adding
an instrument after disable creates new membership lineage.

Each state change records an append-only event containing owner, canonical
six-digit KRX instrument id, current generation number, old/new state, owner
actor, code commit, entitlement hash, typed failure code, retry classification,
and timestamp. No error-message or provider-prose column exists. Failure codes
must match `[A-Z][A-Z0-9_]{0,63}` and retryability is an explicit boolean; a
terminal failure cannot use the retry function.

A partial unique index enforces one non-disabled membership for each
`(owner_user_id, instrument_id)`. The canonical id is stored before provider
validation as `{six digits}.KRX`; it deliberately does not require an existing
shared `instruments` row at request time.

### Generations and admission

Generations are owner/membership/instrument scoped and allocated as consecutive
positive integers. The insert guard locks the membership row before comparing
the next number, so concurrent writers cannot create gaps or duplicate sequence
claims. A generation pins the target/minimum policy used and exact observed
coverage. Generation rows are append-only.

Admission is a separate append-only row. It is allowed only when the generation
has at least its pinned minimum observations and its membership is not disabled.
The admission records exact Raw-manifest, artifact-manifest, and entitlement
SHA-256 hashes plus capture and materializer code commits. These pins can never
be updated or deleted. `READY` requires at least one admitted generation.

### Signal snapshot lineage

A signal snapshot is built as an unpublished header plus rows, then published by
one `published_at` update in the worker transaction. Each row carries owner,
membership, canonical instrument, generation id/number, rank, and a JSON signal
object. A composite foreign key targets the admission key, so an unadmitted
generation cannot enter a snapshot.

The active-ready universe hash is:

1. take every active `READY` canonical instrument id exactly once;
2. sort ids in ascending byte order;
3. join them with one newline and no trailing newline;
4. hash the UTF-8 bytes as `sha256:<lowercase hex>`.

Rust computes the same order-independent value. Before publication, PostgreSQL
recomputes it from snapshot rows and proves that row ids equal the current READY
membership set in both directions and that the stored row count is exact. A
published header and every snapshot row are immutable. Historical snapshots
therefore remain reproducible after later generations or soft disables.

### RLS and rollback

All seven V2 tables are owned by `migration_owner` with RLS enabled and forced.
App and migration-owner policies require
`app.actor_user_id = owner_user_id`; unset actor context sees no tenant rows.
The trusted worker retains only the SELECT/INSERT/update capabilities needed for
the lifecycle and publication contract, and admin is read-only. Audit and
research writers receive no capability. Owner-facing snapshot reads expose only
published headers.

The down migration takes access-exclusive locks and temporarily removes FORCE
RLS only inside its transaction so it can inspect all rows. If any V2 policy,
membership, event, generation, admission, snapshot, or row exists, rollback
raises SQLSTATE `55000`. With an empty surface it drops triggers/functions and
tables in dependency order without `CASCADE`, and migration 0053 can be applied
again.

## Consequences

- The fixed-30 V1 universe remains byte- and behavior-compatible.
- Later packages have one typed policy, one explicit lifecycle, per-instrument
  evidence generations, and exact snapshot lineage to implement against.
- Runtime/API work must provision an owner policy before accepting additions and
  must use the narrow retry/disable surfaces rather than direct state mutation.
- Provider validation, collection, factor computation, queue orchestration, API,
  Web UX, runtime installation, and production QA remain follow-up work under
  the execution plan and existing read-only safety boundaries.
