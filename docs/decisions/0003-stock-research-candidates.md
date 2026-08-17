# ADR-0003: Separate stock research candidates from ETF portfolio recommendations

- **Status:** Accepted for implementation
- **Date:** 2026-08-16
- **Deciders:** Product owner, implementation coordinator, Claude Fable 5 design review

## Context

Lagrange already publishes owner-scoped target allocations for a fixed Korean
11-ETF universe. Those records are execution lineage: they are tied to a user,
strategy configuration, dataset pin, job, target portfolio, and optional Paper
workflow. A daily list of individual-stock research candidates has different
ownership and semantics. Reusing `recommendation_runs`, `recommendation_items`,
or the `recommendation:scheduled:*` job namespace would make a shared research
surface look like an executable portfolio and would violate the existing
database guards.

The new vertical connects three experiences over one immutable daily analysis
snapshot:

1. a common post-close Top-5 research-candidate feed;
2. a configurable, user-owned saved screener; and
3. a deep stock-analysis page covering investor flow, fundamentals, technical
   evidence, and deterministic bullish/neutral/bearish scenarios.

## Decisions

### D1: A separate `candidate_*` domain

- Existing `recommendation_*` names remain reserved for ETF target allocation.
- `candidate_*` names describe shared daily candidate runs and feed publication.
- `stock_analysis_*` names describe immutable per-instrument evidence snapshots.
- `screener_*` names describe user-owned screen definitions.
- Product copy says **research candidate**, never buy/sell recommendation.

No candidate record contains target weight, order quantity, execution intent, or
broker instruction.

### D2: Point-in-time KOSPI 200 is the v1 universe

Membership is stored one instrument per row with `announced_at`,
`effective_from`, `effective_until`, `available_at`, and source revision. The
daily run pins the exact universe snapshot and every input dataset version.
Historical computation may only consume rows that were available by its cutoff.

If licensed point-in-time KOSPI 200 membership cannot be obtained, production
publication stays `BLOCKED`. A different liquidity-based universe would be a
separately named product and ADR, never a silent substitute.

### D3: Shared system output, tenant-owned saved screens

Analysis runs, snapshots, and feeds are system-owned immutable data readable by
authenticated `app` sessions. Serving roles receive no direct DML privilege;
workers publish through narrow `SECURITY DEFINER` functions.

Saved screens carry `owner_user_id`, use FORCE RLS, and follow the existing
actor-GUC tenant boundary. The public actor GUC is only a row selector, never a
system-publication capability.

### D4: Existing queue semantics with a non-login service principal

The current jobs schema requires `owner_user_id`. Candidate scheduling therefore
uses one reserved internal user (`urn:lagrange:internal` /
`candidate-scheduler-v1`) stored in a locked scheduler-control row. It has no
role, invite, or web session and cannot authenticate through Auth0. Only the
migration-owned scheduler function may create its `candidate_compute` jobs.

This preserves the existing leased claim, immutable attempt, retry, orphan, and
capacity contracts without weakening tenant job ownership or creating a second
queue implementation.

### D5: Immutable provider-neutral data contracts

The vertical adds five pinned domains: EOD prices, investor flow, financial
statements, KOSPI 200 membership, and sector classification. Every observation
retains provider identity, entitlement reference, source revision, event/effective
time, provider publication time, first-available time, retrieval time, and the
immutable Raw/curated manifest hashes.

`available_at` is the no-lookahead boundary. Corrections append a new revision;
they never mutate an observation used by a published run.

### D6: Deterministic v1 scoring, not probability

Hard exclusions run before scoring. The initial versioned configuration weights
investor flow 35%, fundamentals 30%, and technical evidence 35%. Missing values
are not zero-filled or forward-filled and missing axes are not silently
reweighted. Coverage controls evidence strength and candidate eligibility.

The v1 response always includes bullish, neutral, and bearish rule-based
scenarios with explicit triggers. It does not claim a probability, target price,
or expected return. Calibrated probabilities require separately accumulated
forward outcomes, leakage-safe evaluation, and a later ADR.

Financial companies use a separate versioned fundamental profile. Until that
profile is implemented and covered, they remain visible in the screener with a
typed `INSUFFICIENT_FUNDAMENTAL_PROFILE` state but cannot enter Top 5.

### D7: Publication is all-or-nothing and freshness-gated

The scheduler time is a wake-up hint, not permission to publish. Compute starts
only after required price and flow datasets are READY and fresh for the trading
date and PIT membership/sector/fundamental pins are resolvable. Publication
atomically commits the completed analysis run, per-stock snapshots, and Top-5
feed. A failed day leaves the previous feed explicitly `STALE`; it never presents
old output as current.

### D8: Initial scope and links

The v1 horizon is 20 trading sessions with 5- and 60-session historical context.
The feed is common to authenticated users only while every governing dataset
has an active `candidate` entitlement; saved screens remain private per user.
Watchlist, backtest, and Paper are navigation handoffs only; automatic execution
is a non-goal.

## Consequences

- The next schema work begins at migration `0042`; migrations `0039` through
  `0041` already belong to auth and Paper reliability.
- Four licensed data sources and their derived-display rights are external
  production prerequisites. Synthetic providers are required for deterministic
  CI but never satisfy the production readiness gate.
- The feature reuses factor snapshots, dataset pins, the job queue, API/OpenAPI
  conventions, RLS patterns, and Next.js state components while adding a new
  product domain.
- Initial weights and thresholds are hypotheses encoded by content hash and
  version. They may change only through an explicit config version and golden
  result review.

## Production gate

Production requires all of the following:

1. point-in-time/restatement/survivorship tests and scoring golden tests;
2. live PostgreSQL migration, RLS, rollback, idempotency, and concurrency tests;
3. synthetic end-to-end ingest-to-Web tests including stale/blocked failure
   paths;
4. active entitlements for every displayed source and derived output;
5. provider freshness observed under real publication SLAs; and
6. no calibrated probability language until its later validation ADR.
