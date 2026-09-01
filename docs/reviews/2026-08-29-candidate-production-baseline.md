# Candidate production baseline — 2026-08-29

## Scope and baseline

Read-only audit of the KOSPI200/KOSDAQ150 **stock research candidate** vertical.
It is distinct from ETF `recommendation_*`, target-weight, Paper, account, and order
domains (ADR-0003 D1, `docs/decisions/0003-stock-research-candidates.md:22-31`).

- HEAD: `559c6dab40cdf19301a2e1cd41c62943140d2eb3` (2026-08-29 09:23:40 +09:00).
- Initial worktree: one pre-existing untracked file,
  `docs/superpowers/plans/2026-08-29-kospi200-production-candidate-activation.md`.
  It was not modified. No running provider call, credential, production DB write,
  restart, deploy, or other external side effect was performed.
- Environment: Docker CLI exists but this process cannot access
  `/var/run/docker.sock`; `psql` is absent. Therefore disposable-PostgreSQL tests
  were not run. Compose with no variables correctly fails closed on missing exact
  release/runtime inputs; the repository's candidate static check supplies its
  harmless fixed commit and passes.

**Verdict: implementation and fixture testing are substantial; production is
BLOCKED.** There is no approved, reachable, point-in-time, rights-pinned source
bundle or installed credentialed runtime. A green synthetic test is not evidence
of live data, rights, PIT availability, or deployment readiness.

## Source and runtime matrix

| Dataset/source | Implemented / tested evidence | Runtime-wired | Rights approved | Actually live |
|---|---|---|---|---|
| Price (`krx_eod_bars`) | DB/source contract and six-pin guard (`migrations/0042_candidate_source_contracts.up.sql:1-2091`; static assertions `deploy/compose/candidate-static-check.sh:72-91`). | Candidate runner reads only curated bytes under `/data` (`deploy/compose/compose.yml:853-875`; static check lines 43-46). No candidate collector feeds it. | No candidate-specific active contract verified; example only. | No. |
| Investor flow | KIS adapter has path/TR-ID and strict envelopes (`crates/market-data/src/providers/kis_candidate.rs:24-42,109-217`); Raw→catalog test fixture. | Not reachable from research-worker: supported only in provider code and candidate source collection needs a provider/catalog binding. | Root allowlist excludes `investor-trade-by-stock-daily`; code is not approval. | No. |
| Fundamentals | KIS balance/income adapter exists (`kis_candidate.rs:29-42,165-195`); strict/PIT contracts exist. | Same unreachable seam. | Root allowlist excludes both finance paths; OpenDART financial APIs are forbidden. | No. |
| Membership: KOSPI200/KOSDAQ150 | Typed partition contract accepts both keys (`crates/market-data/src/candidate.rs:15-54,211-243`); registry schema rows (`0045...up.sql:15-32`). | Master ZIP parser deliberately emits no publishable membership (`kis_candidate_master.rs:1-9`); no history adapter is installed. | No licensed PIT membership contract verified. | No. |
| Sector | Candidate type/normalizer and fixture contract exist; master parser preserves only snapshot evidence. | REST adapter rejects sector before I/O (`kis_candidate.rs:44-54,197-213`). | No approved mapping/rights. | No. |
| Market status | Candidate type and sink exist. | REST adapter rejects it to avoid a partial quote projection (`kis_candidate.rs:44-54`). | No approved all-flags mapping/rights. | No. |
| Entitlements/dataset pins | Exact active `candidate`-use checks, sealed Raw bindings and RLS are in 0042–0045; runtime static check requires seven IDs (`candidate-static-check.sh:72-80`). | Compose declares candidate secret and runner, but no installed systemd candidate unit, entitlement registration, dataset pins, or data volume evidence was found. | `configs/data-rights/krx.entitlement.example.json` is explicitly an example, not verified active authority. | No. |
| API/OpenAPI/Web | Candidate/screener routes, cursor, auth/RLS fixtures and React/Playwright fixtures exist (`crates/api-server/tests/http_candidates.rs:1003-1300`; `apps/web/tests/candidate-research-surface.test.tsx:293-445`; `apps/web/tests/e2e/candidates.spec.ts:17-71`). | API/Web services are Compose definitions, not proof of a deployed instance. | User display is contingent on every governing entitlement (ADR-0003 D8). | No. |

The only credentialed candidate provider class is `KisCandidateProvider` and it
declares `FetchMode::Credentialed` (`kis_candidate.rs:63-107`). It supports just
flow/fundamentals; requesting membership, sector, or status returns a permanent
typed unsupported-kind error before a network call (`:109-137`). The master ZIP
module records snapshots but explicitly refuses publication because it lacks
announcement/effective/availability history (`kis_candidate_master.rs:1-9`).
Thus no complete runtime-entrypoint-to-provider call graph exists today:

```text
Compose candidate-runner -> candidate_compute queue -> attest exact DB pins
 -> factor-engine -> atomic feed/API/Web
                             ^
research-worker -> Raw/catalog/sink --(no installed approved provider bundle)--X
KisCandidateProvider/master parser (implemented seams, not selected/reachable)
```

## Required identity and readiness graph

KOSPI requires six exact pins:
`krx_eod_bars`, `krx_market_status`, `krx_investor_flows`,
`krx_fundamentals`, `krx_kospi200_membership`, and
`krx_sector_classification`; KOSDAQ adds/replaces membership with
`krx_kosdaq150_membership` (`candidate-static-check.sh:72-80`). Each needs
immutable Raw hash, READY/WARNING dataset version, active exact candidate-use
entitlement, provider/revision and PIT availability. The runner then requires a
confirmed trading day, 60 price sessions, exact as-of flow/status, cutoff-valid
membership/sector/fundamentals, score config, and atomic publication. Registry
identity keeps runs/feed/rank separate by universe (`0045...up.sql:103-179`);
the migration seeds both as enabled (`:27-32`), which is schema state—not an
authorization or live activation.

## Observed fail-closed behaviour

- Missing/partial capability or duplicate response kind: rejected before typed
  publication (market-data test `candidate_ingestion.rs:76-107`).
- Future availability and duplicate source identity: validation errors
  (`crates/market-data/src/candidate.rs:346-505`; unit test `:835-871`).
- Stale/missing price/status/flow: input attestation maps missing data to typed
  unavailable/blocked outcomes; no zero-fill/reweight (ADR-0003 D6–D7).
- Corrections: source observations are append-only revisions; exact input pin is
  retained. Fixture suites cover rolling second-day provenance, but live
  correction evidence was not available.
- Replay: fixture paths assert exact replays and immutable binding conflict
  rejection (`candidate_catalog.rs:719-728`; `candidate_runner.rs:889-913`).
- Duplicate membership: parser rejects duplicate `(index, instrument)` natural
  keys before publication (`candidate_ingestion.rs:162-183`).
- Cross-universe: API fixture requires explicit KOSDAQ and no KOSPI fallback;
  same instruments retain two rank contexts (`http_candidates.rs:1225-1300`).
- Disabled universe: registry supports `enabled`; API/runner code tests are
  fixture-level. Production disabled-KOSDAQ staging is not installed or proven.

## Migration, security, and operations assessment

Migrations 0042–0045 are present with paired down files. 0042/0043/0044 down
scripts block rollback when durable source/analysis/job lineage exists
(`0042...down.sql:1-25`, `0043...down.sql:1-16`, `0044...down.sql:1-27`). 0045
uses an advisory fence, FORCE RLS registry, composite universe lineage and
universe-scoped active-feed uniqueness (`0045...up.sql:7-51,103-179`). Static
migration checking passed, but no live up/no-op/down/up was possible without
PostgreSQL.

Compose is hardened in source (read-only runner, `/data/curated:ro`, secret file,
healthcheck, graceful drain) (`compose.yml:853-875`; static check lines 41-70).
There is no `lagrange-candidate-runner.service` in `deploy/systemd`; compose
source alone is not runtime wiring. The candidate Web E2E shell is also a
misleadingly broad test: it runs all `tests/e2e/`, not only candidate tests
(`scripts/qa/candidate-web-e2e.sh:1-9,66-68`).

## Tests actually run

| Command | Count/result | Notes |
|---|---|---|
| `cargo test -p market-data --test candidate_ingestion --locked` | 4/4 PASS, 0.01s | exact typed bundle, dual universe, missing/duplicate/duplicate membership. |
| `cargo test -p factor-engine candidate --locked` | 6 candidate unit tests PASS; 51 unrelated tests filtered/zero-test binaries | 42.85s compile/run; zero-test output was not counted as a pass. |
| `cd apps/web && pnpm exec vitest run tests/candidate-research-surface.test.tsx` | 9/9 PASS, 0.624s | fixture UI only. |
| `deploy/compose/candidate-static-check.sh` | PASS | includes supplied compose config validation. |
| `deploy/db/migrate-static-check.sh` | PASS | static only, not migration execution. |
| `scripts/qa/candidate-web-e2e.sh` | **not concluded** | initial sandbox run blocked loopback bind; approved rerun started its local synthetic API and listed candidate cases 4–6 of 44, but no final summary was returned by the harness. The coordinator later found the rerun's orphaned Next process at PID 3665588, cwd `apps/web`, start 16:17:11 KST, listening on 33001; it was terminated by exact PID with `TERM`, and the child/listener disappeared. Do not count this run as PASS. |
| collectors catalog (4 discovered), job runner (2), API HTTP (9) | BLOCKED | disposable PostgreSQL unavailable: Docker socket permission denied and no `psql`; not replaced with a running service. |

## Gate A blockers and expected WP overlap

1. Owner must approve exact provider, host/path/TR-ID/download contract and
credential operation for price, flow, fundamentals, PIT membership, sector and
status. Current KIS candidate REST paths are outside root allowlist.
2. Owner must approve rights for Raw retention, internal/derived display, user
audience, immutable entitlement reference/hash, backfill budget and lifecycle.
3. PIT evidence must provide announced/effective/available/revision semantics;
current master snapshots are insufficient and must not be backdated.
4. Credentialed provider request/response validation, pagination, rate and
correction contracts need focused fixtures after approval.
5. Production data volumes, exact pins, runtime secret mount and an immutable
release installer must be provisioned and independently rehearsed.

Expected mutable overlap: WP-3: `crates/market-data`, fixtures/tests and possibly
source contracts; WP-4: providers/normalizers/Raw fixtures; WP-5:
collectors/job-queue/migrations/runtime binding; WP-6: api/OpenAPI/generated
client/apps-web; WP-7: compose/systemd/ops/QA/runbook. 0042–0045 must remain
unchanged per the activation plan; any new migration needs a separate owner.

## Documentation contradictions / stale claims

- `docs/STATUS.md:4.4` calls the vertical “code·QA complete, actual feed
  inactive,” which matches this audit. Elsewhere its E1/KIS entitlement history
  describes KIS personal-use readiness, but that does not approve the candidate
  REST/master surfaces or PIT contract. Treat it as ETF/EOD entitlement evidence,
  not this six-source candidate authorization.
- The 2026-08-29 activation plan correctly calls both candidate KIS REST paths
  outside the root allowlist and says Gate A must close seven decisions first;
  any older “vertical complete” wording is implementation-only.
- 0045 seeds KOSDAQ as `enabled=true`; its deployment plan calls for KOSPI-only
  staged activation and KOSDAQ disabled. Seed data is not a production approval,
  but this must be explicitly reconciled during WP-5/WP-10 rather than assumed.

## Hard-incident assessment

No credentialed request was reachable or made. No documented implementation
claim failed its focused non-DB test. The blocked DB/E2E execution is an
environment limitation, not a candidate correctness/security incident. The
Paseo permission-path E2E run left a process requiring coordinator cleanup, so
that harness lifecycle remains operationally unverified rather than green. Production
readiness remains a hard **external/PIT/rights/runtime blocker**, not an incident
in the existing synthetic vertical.
