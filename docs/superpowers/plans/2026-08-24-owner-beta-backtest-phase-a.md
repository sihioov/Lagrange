Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Owner-beta price-only backtest — Phase A decision plan

## Goal and boundaries

- Goal: choose one implementable, fail-closed owner-beta backtest architecture that consumes only
  the embedded-registry-approved historical artifact and preserves the exact five pins,
  `OWNER_ONLY`, `vendor_snapshot=true`, `strict_pit=false`, and `PRICE_RETURN_ONLY` through durable
  results and reads.
- Completion: two independent read-only analyses are reconciled into a coordinator-approved
  execution contract with an explicit simulation convention, owned file scopes, database/API/UI
  shape, tests, and stop conditions. Phase B implementation packages are not launched until that
  contract is written into this plan.
- Canonical workspace: `/data/workspace/lagrange`, branch `main`, reviewed at
  `e14936c317e25203c6c61fcb6ab1c631f170f591`. The coordinator may prepare a documentation commit in
  the clean `audit-project-status` worktree at the same commit and fast-forward it into `main`.
- In scope: existing sealed artifact/approval/factor/target code, ordinary and owner-beta queue
  patterns, existing Nautilus backtest path, dedicated persistence/API/UI/runtime boundaries, and
  deterministic offline tests.
- Out of scope: inventing approval registry records or KSD pins; live KIS/OpenDART/KIND/network
  access; account/order/Live surfaces; production DB writes; image installation; systemd/Compose
  activation; Member KR access; strict PIT or total-return claims.
- Instruction sources: repository `AGENTS.md`; `apps/web/AGENTS.md` and `apps/web/CLAUDE.md` for any
  later Web package; `$paseo-delegate-plan`; `$paseo-delegate`. The KIS/OpenDART deny-by-default
  boundaries remain mandatory.
- Unresolved requirement: the repository has no approved definition for converting adjusted-close
  artifact bars and daily target snapshots into a backtest result. In particular, signal timing,
  rebalance execution price, transaction costs, benchmark, warm-up, cash return, and whether the
  existing Nautilus worker can truthfully consume this capability must be decided from existing
  evidence, not invented by an implementation worker.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | hard | Crosses sealed artifact, factor/target, job, result, DB, API, and runtime boundaries; the correct reuse/isolation architecture is the deliverable | high | Existing code proves one path already consumes approved bars without widening capability; otherwise report the missing seam rather than lowering complexity |
| WP-2 | hard | Look-ahead and execution semantics determine whether reported performance is truthful; there is no direct mechanical oracle | high | Official project contract already fixes every simulation convention and provides a golden test; if so, name the evidence and the later implementation may become intermediate |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | hard | Select the narrowest viable implementation architecture and exact component/file boundaries | Read-only inspection of Rust, SQL, Python worker, deploy and plan files; no edits | none | Codex `gpt-5.6-sol`, high, via `$paseo-delegate` | Evidence-backed architecture report with one recommendation, rejected alternatives, dependency graph, file ownership, and blockers | Symbol/call-site/path evidence; no network, DB, or writes |
| WP-2 | 1 | hard | Define truthful price-only backtest semantics and adversarial acceptance criteria | Read-only inspection of domain/result/factor/target/artifact contracts and tests; no edits | none | Codex `gpt-5.6-sol`, high, via `$paseo-delegate` | Threat/semantics report with exact invariants, required labels, failure cases, and unresolved decisions | Evidence citations plus testable invariants; no network, DB, or writes |

Wave 1 is parallel because both packages are read-only. Their scopes may overlap for inspection but
they own no mutable files. Phase B packages will be added only after the coordinator gate resolves
their conclusions.

## Coordinator preflight evidence (not an architecture decision)

The coordinator recorded the following source facts while worker launch was blocked. They narrow the
questions for both workers but do not replace the required independent analyses:

- `tests/golden/robustness/runner.py` defines a deterministic `T`-close target to next-session-open
  simulation, sells before buys, integer lots, and `KRX_ETF_DEFAULT` v1 costs. It explicitly says
  this is `lagrange-golden-sim`, not the NautilusTrader engine.
- `portfolio-model::CostProfile` fixes the currently shipped ETF profile at version 1 and derives
  execution prices from raw opens with side-specific 10 bps slippage. This is reusable arithmetic,
  but its source comments still call the values operator-confirmation placeholders.
- The generic API accepts a free-form `execution_profile` and a fixed-universe `benchmark`, but
  `job_queue::runner::BacktestPayload` names neither field and deliberately ignores unused payload
  fields. The child simulator therefore receives neither choice. Tests use both `next_open` and
  `daily-close-next-open@1`, so no single enforced profile identity exists at this boundary.
- `nt/backtest-worker/backtest_worker/simulate.py` computes the reported benchmark as equal-weight
  buy-and-hold of every dataset instrument. It does not consume the API's requested benchmark. A
  generic result therefore cannot be reused as evidence that the requested owner-beta benchmark was
  simulated.
- The generic worker reads registered Curated Parquet and rebuilds a Nautilus catalog. The approved
  owner-beta value is a nonconstructible `ApprovedHistoricalPriceOnlyArtifact` with exact five pins
  and in-memory bars; there is no approved conversion or registration seam between them.
- Each approved artifact bar contains raw and split-adjusted OHLC plus an adjusted close usable by
  the factor path, but it is date-only and explicitly carries no market-open or market-close
  timestamp. Any timestamped order/fill/equity contract needs a documented synthetic convention or
  a date-only result model; it cannot infer a time from `acquired_at`.
- A target is recomputed and pin-validated for one `as_of` date. The repository has not yet specified
  the schedule that turns these point targets into a multi-date owner-beta run, nor whether execution
  and valuation use raw or adjusted prices across a split. Phase B must not choose those semantics by
  convenience.

## Worker briefs

### WP-1 — architecture and reuse boundary

- Target: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: hard; high confidence. Escalate only by reporting a missing contract—do not choose a
  convenient design without evidence.
- Objective: determine whether owner-beta backtesting should reuse the existing Nautilus
  `backtest` job/results, add a dedicated owner-beta runner/result path, or stop at a missing data
  registration/conversion seam. Recommend exactly one.
- Known facts: `ApprovedHistoricalPriceOnlyArtifact` exposes validated bars and five pins; factors
  consume adjusted close only; owner-beta recommendation input/worker/persistence/API/UI are
  isolated; the existing backtest path consumes registered curated datasets and emits a richer
  result contract; the embedded registry was empty at this analysis time; real DB evidence was
  unavailable.
- Inspect at minimum: `crates/market-data/src/historical_price_only_*`,
  `crates/factor-engine/src/{bars,price_only}.rs`, `crates/job-queue/src/owner_beta/**`,
  `crates/job-queue/src/runner.rs`, `crates/result-model/src/backtest.rs`,
  `crates/api-server/src/{http,repos}/backtest*`, migrations `0049..0051`, existing backtest tests,
  and backtest deployment definitions.
- Prohibited: file edits, new pins, production/network/DB actions, changing the launch plan, or
  assuming the existing Nautilus worker accepts data it does not accept.
- Required output: recommended architecture; exact data/control flow; all new/changed files grouped
  into disjoint implementation packages; migration/job/API/runtime consequences; rejected options;
  objective tests; unresolved blockers.
- Required report: changed files and lines (`none`); deviations and reasons; checks/evidence;
  unresolved/follow-up; not found/not verified (say `none` when empty).
- Mandatory prompt sentence: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-2 — simulation semantics and threat model

- Target: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: hard; high confidence. Escalate if signal/execution timing or price/cost semantics are
  not already approved; report the gap instead of creating a convention.
- Objective: specify what this artifact can and cannot support as a backtest, with acceptance
  criteria strong enough to prevent look-ahead, total-return/PIT overclaims, cross-owner leakage,
  tampered pins, impossible result states, and unsafe reuse of generic outputs.
- Known facts: the artifact range is `2020-01-31..2026-08-19`, exact ETF11 and 1,608 sessions;
  factor input uses adjusted close only; `strict_pit=false`; only price-return capability is
  approved; recommendation targets are deterministic and pin-bound.
- Inspect at minimum: the same artifact/factor/target contracts, `result-model` backtest invariants,
  Phase-0/golden strategy evidence, product labels, and the approved owner-beta launch plan.
- Prohibited: edits, performance claims, invented execution prices/costs/dividends, treating adjusted
  close as proof of total return, network/DB/production actions, or relaxing owner-only policy.
- Required output: minimum honest result schema/labels; signal-to-return timing; warm-up and missing
  session rules; weight/cash/turnover arithmetic; benchmark and cost decision evidence; deterministic
  hash preimage; state machine and sanitised errors; negative/race/tamper tests; explicit gaps.
- Required report: changed files and lines (`none`); deviations and reasons; checks/evidence;
  unresolved/follow-up; not found/not verified (say `none` when empty).
- Mandatory prompt sentence: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

## Coordinator gates

1. Pre-launch: verify clean worktree, exact branch/HEAD, instructions, provider/model availability,
   and that both prompts contain the no-delegation sentence. No user decision is needed for this
   read-only phase.
2. Wave 1 integration: require both Paseo agents to finish `idle`, inspect their evidence directly,
   reject unsupported assumptions, and choose one architecture. If a simulation convention is not
   already authorized, stop Phase B and leave the gap for owner review rather than inventing it.
3. Phase B revision: add classified, disjoint implementation packages and their exact tests to this
   plan before launching any editor worker.
4. Final later acceptance: coordinator reruns focused and full Rust/OpenAPI/Web/ops gates, obtains
   independent security review, commits logical units, and records skipped DB/image/production
   evidence explicitly.

## Execution status (2026-08-24)

- The owner explicitly approved transmitting repository source to external Paseo/Codex workers for
  read-only analysis. That approval did not extend to market-data network access, database writes,
  deployment, service activation, or any account/order surface.
- WP-1 ran through Paseo as Codex `gpt-5.6-sol`, high, agent
  `c8f54f53-cc45-4b3c-9d7b-8601a2055305`; WP-2 used the same provider/model as agent
  `a240148d-7033-412c-84dd-4f4fa30d0b39`. Both finished with Paseo status `idle` after read-only
  inspection of `main` at `e14936c317e25203c6c61fcb6ab1c631f170f591`.
- Neither worker changed files, used network market data, accessed a database, deployed, activated a
  service, or touched the preserved untracked KIS XLSX.

## Coordinator decision and stop condition

The two independent reports agree, and direct source inspection supports the same conclusion:

1. **Stop at the missing conversion/simulation seam.** At this analysis time the embedded approval
   registry was empty, so there was no constructible approved artifact or authorized five-pin value
   to enqueue.
2. **Reject generic Nautilus-path reuse.** That path requires a registered `READY` Curated dataset,
   timestamped market inputs, one dataset pin, and generic order/fill/benchmark results. It cannot
   preserve the date-only artifact's five-pin trust chain and fixed owner-only price-return envelope.
   Its worker request also does not carry the API's requested benchmark or execution profile.
3. **Use a dedicated owner-beta Rust path only after P0 approval.** Isolation is the narrowest future
   architecture, but starting it now would invent schedule, timing, price, adjustment, cost,
   benchmark, warm-up, cash, turnover, timestamp, and result-hash semantics.
4. **Do not put backtesting on the critical path for the first owner beta release.** Continue the
   already isolated single-date recommendation launch path when its real artifact pins are approved;
   keep owner-beta backtesting unavailable rather than delaying or weakening that release.

Phase B remains prohibited until the owner approves and this plan records all of the following as one
versioned simulation contract:

- the multi-date signal/rebalance schedule and warm-up start;
- the signal-to-return interval and execution/valuation price fields;
- raw-versus-adjusted price treatment across splits;
- missing-session behavior;
- initial notional, share/lot sizing, rebalance and cash-return rules;
- cost/slippage model, performance benchmark, and turnover definition;
- date-only output versus explicitly synthetic timestamps;
- permitted metrics and canonical result-hash preimage; and
- one real embedded approval-registry record containing the five reviewed pins.

Once P0 is approved, the conditional implementation order is:

1. dedicated owner-beta result contract and integrity tests;
2. dedicated job/input/compute/runner and deterministic simulation tests;
3. dedicated migrations with forced RLS, immutable pin constraints, and atomic publication;
4. owner-only API/OpenAPI/Web result surface preserving all fixed labels; and
5. isolated Rust runtime and release/static checks with no Curated, Python, or Nautilus dependency.

No editor worker may start these packages before the P0 gate is satisfied. Until then the honest
runtime behavior is pre-enqueue refusal with no run/result creation, not a fabricated failed or
successful backtest.
