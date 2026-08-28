# Owner-beta price-only backtest P0 — Owner decision packet

**Decision status:** unapproved. Select exactly one decision at the end of this packet. No checkbox
is selected by this document.

## Decision in one page

Phase A rejected attaching a generic Nautilus/registered-`READY` backtest to the owner-beta
recommendation and stopped at a missing simulation seam
(`2026-08-24-owner-beta-backtest-phase-a.md:160-200`). Current source now contains one embedded
artifact approval record, but that changes only the historical Phase A pin-availability condition:
the approved value is still `OWNER_ONLY`, `vendor_snapshot=true`, `strict_pit=false`,
`PRICE_RETURN_ONLY`, `UNREGISTERED`, and `NOT_PUBLISHED`
(`historical_price_only_approval.rs:130-215`). Artifact approval is not approval of a backtest
simulation contract.

No already Owner-approved, versioned owner-beta simulation contract was found. The system-design
request example, the generic Nautilus path, and the synthetic `lagrange-golden-sim` fixtures are
different contracts; none silently supplies the decisions below.

**Recommendation:** defer owner-beta backtest implementation now, continue recommendation
remediation independently, and reopen the dedicated Rust path only after the Owner completes and
approves every P0 field below as one versioned contract. This is a recommendation, not Owner
approval.

## Why generic output cannot be attached to the recommendation

1. **Different input authority.** The recommendation can be constructed only from the approved
   in-memory artifact and its exact five pins: candidate content, artifact manifest, Stage5
   manifest, action manifest, and approval-registry hashes
   (`historical_price_only_approval.rs:20-65`; `owner_beta/input.rs:23-54,101-121`). The generic
   runner instead resolves one `dataset_version`, joins a registered Curated dataset whose status is
   `READY`, and attests that Curated pin (`runner.rs:395-417,1243-1258,3257-3284`). There is no
   approved conversion or registration seam between these authorities.
2. **Different data and time semantics.** Owner-beta factors consume split-adjusted close from
   date-only bars; the artifact separately carries raw and adjusted OHLC and explicitly has no
   market-open or market-close timestamp (`historical_price_only.rs:76-100`;
   `price_only.rs:1-5,102-138`). Generic results require timestamped orders, fills, equity, cash,
   fees, and benchmark points (`result-model/src/backtest.rs:65-186`). Attaching them would invent
   execution, valuation, split, and timestamp semantics.
3. **Different request/result boundary.** The generic API accepts `benchmark` and
   `execution_profile`, but `BacktestPayload` names neither and intentionally ignores unused fields;
   neither reaches its child request (`http/backtests.rs:225-235`; `runner.rs:570-599,2606-2627,
   2699-2761`). The child instead reports an equal-weight buy-and-hold benchmark of every dataset
   instrument (`simulate.py:1-14,369-385`). A generic result therefore does not prove the requested
   benchmark or execution convention.
4. **Different claim envelope.** The owner-beta recommendation is one pin-bound `as_of` target,
   `OWNER_ONLY` and `PRICE_RETURN_ONLY`. The generic report renders ending equity, drawdown, monthly
   returns, trades/costs, timestamps, and robustness (`backtest-report.tsx:18-165`). A generic
   `SUCCEEDED` state cannot truthfully upgrade a single-date recommendation into performance
   evidence. The two current production recommendation runs that reached `SUCCEEDED` but failed
   user acceptance do not authorize simulation.

## P0 simulation contract worksheet — every field unresolved, no defaults

For an implementation approval to be valid, the Owner must replace every `OWNER TO SPECIFY` entry
with an explicit value and check every row. Implementers must not infer a value from generic or
golden code.

| Done | Contract field | Owner decision that must be explicit |
| --- | --- | --- |
| [ ] | Schedule and warm-up | `OWNER TO SPECIFY`: run start/end eligibility; signal/rebalance cadence; factor warm-up start and whether pre-run history is visible; first tradable and last signal/execution sessions. |
| [ ] | Signal-return interval | `OWNER TO SPECIFY`: information cutoff for target `T`; exact return/holding interval; when a target becomes effective; terminal-signal handling; invariant that no post-cutoff observation can affect a signal. |
| [ ] | Execution and valuation price | `OWNER TO SPECIFY`: exact artifact field for each buy/sell fill and each portfolio mark; trade ordering; same-session behavior; deterministic rounding. |
| [ ] | Raw versus adjusted split treatment | `OWNER TO SPECIFY`: which series drives signals, fills, marks, and returns; how share quantities change over splits; how double counting is prevented. The result must remain price-return-only and must not imply dividend or total return. |
| [ ] | Missing sessions | `OWNER TO SPECIFY`: behavior for a missing whole-market session, an incomplete ETF11 session, an instrument gap, and a missing next execution/valuation observation; whether any carry-forward is allowed or the run fails closed. |
| [ ] | Notional, lot, and cash | `OWNER TO SPECIFY`: initial KRW notional; integer/fractional share and lot rules; target-to-quantity rounding; residual cash and cash return; insufficient-cash behavior; sells-versus-buys ordering; rebalance threshold, if any. |
| [ ] | Cost and slippage | `OWNER TO SPECIFY`: profile identity/version; commission, minimum, tax, and slippage inputs; side/base-price application; rounding; effective-date policy; behavior when a required value is unavailable. Existing placeholder/golden values are not approval. |
| [ ] | Benchmark | `OWNER TO SPECIFY`: instrument or portfolio identity; buy-and-hold/rebalanced policy; inception and cash allocation; prices, costs, split handling, missing observations, and comparison interval. |
| [ ] | Turnover | `OWNER TO SPECIFY`: numerator, denominator, inclusion of buys/sells/initial funding/final liquidation/costs, observation period, and any annualization. |
| [ ] | Timestamps | `OWNER TO SPECIFY`: date-only result model or explicitly synthetic times; timezone and event ordering if synthetic. `acquired_at` is retrieval provenance and must never be used as a session time. |
| [ ] | Permitted metrics | `OWNER TO SPECIFY`: closed allowlist, exact formulas, annualization basis, minimum sample rules, undefined-value behavior, rounding/units, benchmark-relative metrics, and mandatory `PRICE_RETURN_ONLY`, `vendor_snapshot=true`, `strict_pit=false` labels. |
| [ ] | Canonical result-hash preimage | `OWNER TO SPECIFY`: contract/version tag; canonical serialization and field order; numeric scales; all five pins; strategy config and factor/target identities; schedule and every convention above; code/engine identity; result sections covered; hash algorithm and exclusion rules. |

Any unchecked or placeholder row keeps P0 closed. Ambiguity, conflicting rules, a result that cannot
bind all five pins, or a required observation that is absent must stop before enqueue or fail closed
without publishing a partial result.

## Owner choices

| Choice | User value | Principal risk | Cost | Stop conditions |
| --- | --- | --- | --- | --- |
| **A — implement a dedicated Rust path after complete P0 approval** | Adds owner-only performance evidence derived from the same approved five-pin artifact and fixed labels; makes the result independently reproducible. | Look-ahead, split/cash/benchmark errors, misleading metrics, or cross-boundary leakage if any convention or pin is lost. | High: new result/hash contract, deterministic simulator/job path, persistence/RLS/publication, owner-only API/Web, isolated runtime, and cross-layer verification. No generic Nautilus/Python reuse credit. | Do not start Phase B until all worksheet rows and one versioned contract are approved. Stop on any unresolved semantic, pin mismatch, non-ETF11/incomplete session, unsupported metric, non-determinism, or inability to keep the path isolated. |
| **B — defer** | Keeps the current recommendation remediation moving without presenting unverified performance as evidence. | Owner-beta users receive no backtest metric or historical performance view. | Low immediate engineering cost; opportunity cost is the unavailable capability. | Keep pre-enqueue refusal and no result creation. End defer only when the Owner supplies and approves the complete versioned contract and implementation/verification capacity. |

## Exact Owner decision wording

Select exactly one. The implementation checkbox is invalid while any worksheet row remains unchecked
or says `OWNER TO SPECIFY`.

- [ ] **APPROVE DEDICATED IMPLEMENTATION:** “I approve the completed P0 worksheet above as
  versioned simulation contract `<OWNER-SUPPLIED CONTRACT ID AND VERSION>` and authorize Phase B to
  implement a dedicated owner-beta Rust backtest path. The implementation must preserve the exact
  five pins and the `OWNER_ONLY`, `PRICE_RETURN_ONLY`, `vendor_snapshot=true`, `strict_pit=false`
  envelope; must not reuse or relabel generic Nautilus/registered-`READY` output; must fail closed on
  any contract or integrity mismatch; and must pass verified deterministic, isolation, persistence,
  API, Web, and release gates before any metric is exposed. This approval does not authorize market
  data collection, database or production mutation during planning, Paper/Live, account, or order
  actions.”
- [ ] **DEFER:** “I defer the owner-beta price-only backtest. Keep it unavailable with pre-enqueue
  refusal and no run/result creation. Continue current recommendation remediation independently,
  and do not expose any backtest or performance metric in the recommendation report. Reopen P0 only
  with a complete versioned simulation contract.”

Owner name: `____________________`  Date: `____________`  Contract version if approved:
`____________________`

## Conditional Phase B dependency graph — no implementation authorized here

1. **B1 — dedicated result and canonical-hash contract** depends on approved P0.
2. **B2 — deterministic Rust simulation core plus dedicated input/compute/job/runner** depends on B1.
3. **B3 — owner-scoped persistence, forced RLS, immutable pin constraints, and atomic publication**
   depends on B1 and B2.
4. **B4 — owner-only API and OpenAPI read/enqueue surface** depends on B3.
5. **B5 — owner-only Web result surface with the fixed evidence labels** depends on B4.
6. **B6 — isolated Rust runtime and release/static gates, with no Curated, Python, or Nautilus
   dependency** depends on B2 and B3.
7. **B7 — deterministic, tamper, tenancy, race/recovery, API/Web, and release verification** depends
   on B4, B5, and B6; only its verified pass may enable metric exposure.

## Independent recommendation-release rule

Current recommendation remediation proceeds independently of this P0 decision. Its two
`SUCCEEDED` production runs and failed user acceptance remain recommendation evidence only. **No
backtest, return, drawdown, benchmark, turnover, risk-adjusted, cost, or other performance metric may
appear in the recommendation report before P0 Owner approval and verified Phase B implementation.**
