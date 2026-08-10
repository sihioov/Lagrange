# Live Risk Input Wiring Design

**Date:** 2026-08-10  
**Scope:** `docs/STATUS.md` §4.3 item 2

## Goal

Replace the three deliberately-unsourced `RiskSnapshot` inputs with reads from
the repository's existing sources of truth, while preserving the fail-closed
contract: missing rows, stale metadata, unsupported timezones, and database
errors produce `Unknown` and therefore deny the order.

## Sources and decisions

### Market session

Read the current KRX session row from `trading_calendars` using the snapshot's
`now_secs`, converted to `Asia/Seoul`. A row is usable only when its exchange is
`KRX`, its `session_type` is `TRADING`, its timezone is `Asia/Seoul`, and the
instant is inside the documented 09:00–15:30 KST continuous session. A missing,
contradictory, or unreadable row returns `MarketSession::Unknown`; outside the
window or a non-trading row returns `Closed`.

### Data freshness

Read the newest KRX/KR EOD `data_batches.retrieved_at` value and compare its age
at `now_secs` to the configured `RiskLimits::max_data_age_secs`. A missing batch,
future timestamp, negative/overflowing age, or database error returns
`DataFreshness::Unknown`. Otherwise return `Age(age_secs)` and let the existing
risk check compare it with the configured limit. This keeps the limit row as the
policy authority and the batch manifest as the data authority.

### Intent conflict

Read `order_intents` in the same actor-scoped transaction for the requested
account and instrument. Any non-terminal state (`INTENT_CREATED`,
`RISK_APPROVED`, `SUBMITTING`, `SUBMITTED`, `UNKNOWN`, `ACCEPTED`, or
`PARTIALLY_FILLED`) is a conflict; only terminal states are ignored. A query
error returns `IntentConflict::Unknown` rather than assuming no conflict.

## Data flow and safety

`for_submission` continues to build a snapshot before the claim/gate sequence.
The three helpers are read-only and receive the already validated `GateOrder`,
actor, and deterministic `now_secs`. The gate remains unchanged: every
`Unknown` input maps to `InputUnavailable`, and no helper can mint an approval.
The existing fixture snapshot is untouched and remains test-only.

## Testing

Add database-gated API tests for:

1. an open KRX session, a fresh EOD batch, and no active intent;
2. a closed/non-session date and a stale batch;
3. an active intent conflict;
4. missing rows and malformed timezone/timestamps staying `Unknown` and
   denying the decision;
5. actor isolation: another owner's intent cannot create a conflict.

Pure time/age calculations are unit-tested separately so database tests cover
only SQL and tenancy boundaries. Existing risk-gateway tests continue to prove
that `Unknown` denies.
