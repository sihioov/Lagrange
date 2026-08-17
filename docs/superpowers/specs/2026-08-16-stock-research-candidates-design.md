# Stock research candidates: daily feed, screener, and deep analysis

## Goal

Add a point-in-time individual-stock research vertical without changing the
meaning or storage of the existing ETF portfolio recommendation flow.

The v1 surface consists of:

- `/candidates`: one common post-close Top-5 feed;
- `/screener`: ad-hoc criteria plus user-owned saved screens; and
- `/stocks/[instrument]`: flow, fundamentals, technical evidence, scenarios,
  provenance, and freshness for one instrument.

## Non-goals

- order or target generation;
- live KIS integration;
- personalized ranking;
- intraday signals;
- news or narrative-LLM scoring;
- target prices, expected returns, or uncalibrated probabilities; and
- a universe outside licensed point-in-time KOSPI 200 membership.

## Terminology and boundaries

| Product | Code namespace | Ownership | Execution semantics |
|---|---|---|---|
| ETF portfolio recommendation | `recommendation_*` | tenant | target weights and Paper lineage |
| Market-data metadata publication | `research_*` | system | immutable data lineage |
| Stock research candidates | `candidate_*` | system | research evidence only |
| Saved screens | `screener_*` | tenant | criteria only |

Candidate code must not write to `recommendation_runs`,
`recommendation_items`, `target_portfolios`, or use the
`recommendation:scheduled:*` namespace.

## Product flow

```text
provider Raw + entitlement
  -> immutable Raw manifest
  -> validated curated dataset version
  -> point-in-time universe + factor snapshot
  -> immutable stock analysis run/snapshots
  -> atomic Top-5 feed publication
       |-> /candidates
       |-> /screener (latest READY snapshot)
       `-> /stocks/{instrument}
```

The worker may wake at 16:30 Asia/Seoul, but it must wait for explicit data
readiness. EOD prices and investor flow are daily-required. Membership, sector,
and fundamentals may reuse their latest eligible point-in-time revision when no
new revision is expected that day. Missing, stale, blocked, unlicensed, or
future-only inputs fail closed.

## Data contracts

All records carry `provider`, `license_ref`, `source_revision`,
`retrieved_at`, and `dataset_version_id`. All source hashes are lowercase SHA-256.

### Universe membership

Natural identity:

```text
(index_id, instrument_id, effective_from, source_revision)
```

Required times: `announced_at`, `effective_from`, optional `effective_until`,
`available_at`, and `retrieved_at`. A run at cutoff `T` may consume only rows
with `available_at <= T` and whose effective interval contains the as-of date.

### Investor flow

Natural identity:

```text
(instrument_id, trade_date, investor_class, source_revision)
```

`investor_class` is `FOREIGN` or `INSTITUTION`. Values preserve net amount and
net volume in vendor units plus explicit currency/unit metadata. Corrections
append a revision. Factor computation selects the latest revision available at
the run cutoff, never simply the latest currently known revision.

### Financial observations

Natural identity:

```text
(issuer_id, fiscal_period_end, statement_scope, metric, disclosed_at,
 source_revision)
```

Required metadata: fiscal-period start/end, annual/quarterly period kind,
consolidated/separate scope, audited flag, currency, unit scale, `disclosed_at`,
`available_at`, and restatement reference. The PIT value is the latest revision
that was available by the run cutoff. Restatements never rewrite history.

### Sector classification

Natural identity:

```text
(taxonomy_id, taxonomy_version, instrument_id, effective_from)
```

KRX sector classification is the initial intended taxonomy, subject to rights
verification. A different taxonomy is a distinct version; classifications are
never silently mapped between taxonomies.

### EOD bars

The existing curated bar contract and dataset version pin remain authoritative.
Candidate computation consumes exact trading sessions from the KRX calendar and
never substitutes calendar days.

## Scoring v1

### Hard exclusions

- not a member of the pinned PIT universe;
- suspended, administrative, liquidation, or inactive instrument;
- fewer than 60 eligible sessions;
- daily-required price or flow data missing/stale;
- 20-session average trading value below the versioned floor;
- disqualifying audit opinion or complete capital impairment when available;
- unsupported fundamental profile; or
- any required entitlement inactive.

Each exclusion is a typed reason stored in the snapshot. No excluded instrument
can enter Top 5.

### Axis scores

The immutable scoring config starts with:

```json
{
  "version": "candidate-score-v1",
  "weights": {"flow": 0.35, "fundamental": 0.30, "technical": 0.35},
  "primary_horizon_sessions": 20,
  "context_sessions": [5, 60]
}
```

- Flow: foreign/institutional 5/20/60-session accumulation, amount intensity,
  acceleration, joint accumulation, and price/flow divergence.
- Fundamental: growth, profitability, balance-sheet safety, cash conversion,
  valuation, and sector-relative rank. Financial companies use a separate
  profile.
- Technical: existing momentum, trend, volatility, liquidity, drawdown, plus
  exact 5/20/60-session returns and distance from the 20-session high.

Each factor is winsorized by config then normalized within sector. A sector with
fewer than eight eligible names falls back to the whole universe and records
`normalization_scope=UNIVERSE_FALLBACK`.

Missing values stay null. Axis coverage is the available configured factor
weight divided by configured axis weight. Axes under 60% are unavailable and are
not reweighted. Top-5 eligibility requires every axis at least 60% and overall
evidence at least `MODERATE`.

Evidence labels are deterministic:

- `STRONG`: all axes >=80% coverage and signs agree;
- `MODERATE`: all axes >=60% and at least two signs agree;
- `WEAK`: otherwise.

Scenario labels are not probabilities. Each snapshot stores three scenarios
with typed trigger expressions and evidence references. Display copy cannot add
percent signs, target prices, or buy/sell verbs.

## Persistence

Migration `0042` introduces source-observation contracts. Migration `0043`
introduces versioned analysis and feed tables plus tenant saved screens.
Migration `0044` introduces the scheduler control, internal service principal,
immutable scheduled-job namespace, and narrow schedule/publish functions.

System output tables use RLS SELECT policies for `app`, `worker`, and `admin`;
direct serving-role DML is revoked. Saved screens use FORCE RLS and
`owner_user_id = NULLIF(current_setting('app.actor_user_id', true), '')::uuid`.

Published analysis rows are immutable. A correction creates a new computation
sequence and atomically supersedes the previous feed. Rollback migrations refuse
to drop undrained jobs, published runs, feeds, snapshots, user screens, or source
observations.

## Jobs and publication

`candidate_compute` is the initial job type. Data collection remains behind
provider-neutral seams; synthetic CI can insert validated dataset contracts
without pretending to be a production provider.

The scheduling identity is a hash of:

```text
as_of | cutoff | scoring_config_hash | universe_dataset_pin |
price_pin | flow_pin | fundamental_pin | sector_pin
```

Only a migration-owned function may insert `candidate:scheduled:<hash>` jobs.
The function recomputes the key, validates every pin and entitlement, takes an
advisory transaction lock, and returns the existing job/run on exact replay.
Conflicting lineage fails with `23514`.

Publication validates the claimed job, run identity, service principal, all
instrument membership, exactly one snapshot per universe member, Top-5 ranks,
and content hashes. It writes snapshots and the active feed in one transaction.

## API contracts

- `GET /api/v1/candidates/feed/latest`
- `GET /api/v1/candidates/feed/{date}`
- `GET /api/v1/stocks/{instrument_id}/analysis?date=YYYY-MM-DD`
- `POST /api/v1/screener/query`
- `GET|POST /api/v1/screener/screens`
- `GET|PUT|DELETE /api/v1/screener/screens/{id}`

Every research response includes `as_of`, `cutoff_at`, state
(`READY|STALE|BLOCKED`), scoring-config version/hash, exact dataset pins,
license attributions, and a research-only disclaimer. Screener results use an
opaque signed cursor and query only an explicitly selected READY run.

## Verification

- deterministic factor unit and golden tests;
- PIT tests for future revisions, late restatements, membership changes, and
  delisted constituents;
- migration live-DB tests for privileges, RLS, direct-DML denial, concurrent
  schedule/publish, exact replay, rollback guards, and service-user isolation;
- API contract tests for ready/stale/blocked and cross-user saved screens;
- Web accessibility, mobile, and Playwright paths for all three surfaces;
- failure injection for one missing dataset, partial publication, lease loss,
  retry exhaustion, and stale feed fallback; and
- CI production-blocked proof when provider entitlements are absent.
