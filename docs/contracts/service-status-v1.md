# Owner service status v1 — WP-1 acceptance contract

Status: **accepted by coordinator for implementation and isolated verification, 2026-10-08 KST**.
Baseline: `c3295be6bb618483c6b2594de5232faea66bd8f3`. Static source inspection only.
The user authorized WP-1 execution, superseding the plan's old “planning only” gate.
No production truth is asserted. Every fixture is synthetic. No provider, production,
Docker, systemd, database, build, test suite, secret, runtime env, or log was accessed.

## 1. Decisions to accept before WP-2/3/4

Accept these together; none is an authorization to install or activate anything:
The coordinator accepted the architecture, bounded repair and sequential
WP-3A/WP-3B ownership below after source and artifact validation. Acceptance does
not authorize production installation, activation, deployment or provider calls.

1. The closed wire schemas, catalog and fixture corpus in this change; `owner_daily`
   is the intended deployment policy, not a claim about today's installed runtime.
2. The root-owned policy receipt and optional directory mount in §3. Daily mode must
   exclude the research daemon from release startup. This is a necessary correction
   to the release integration, not a new collection schedule.
3. Reserve **0058_service_status_reads** and **0059_research_execution_status**
   (both up/down) for WP-3A, plus the exact instrumentation in §7. No migration is
   edited by WP-1. Baseline has contiguous up/down pairs 0001–0057 and no 0058/0059.
   Check free reservations with `--baseline-reservations` before implementation;
   default fixture validation accepts only the baseline or contiguous approved
   complete pairs through 0058/0059. Never renumber an existing migration.
4. Accept a two-hour **display grace** after the existing 16:30 KST EOD eligibility
   cutoff. At 18:30 precisely stale data becomes degraded. This changes neither
   scheduling nor producer deadlines. Stream TTLs remain their existing values.
5. Accept per-admitted-request admin audit and endpoint admission/cache bounds in
   §5. Successful cached responses are audited; no silent audit exemption.
6. Retain the explicit producer files and WP-2 release-policy integration in §8.
   The coordinator split WP-3 **sequentially**: WP-3A owns migrations/instrumentation
   and DB read tests; WP-3B owns API integration. The exact disjoint file map below
   reflects that clarification. WP-1 itself requires no further decomposition.

## 2. Inventory and source evidence

The catalog freezes 22 ordered logical IDs. Never build inventory from `docker ps`.
`research-worker` is an explanatory executor row and is excluded from the overall
work denominator; `kis-eod` incorporates its process evidence. Backtest rows share
one actor queue; their counts must never be added together. A capability has no
separate PID. Every row has its own source availability and validity.

| ID | Runtime/source evidence | Safe observation and actual reader |
|---|---|---|
| postgres | `deploy/compose/compose.yml:405`, `:436` | root Docker state/health; pg_isready is readiness, not dataset freshness |
| reverse-proxy | `deploy/compose/compose.yml:48`, `:79` | root Docker state/health; nginx health route, not end-to-end browser reachability |
| web | `deploy/compose/compose.yml:98`, `:127` | root Docker state/health; not chart or signal readiness |
| api-server | `deploy/compose/compose.yml:142`, `:267`; `crates/api-server/src/runtime.rs:1345`, `:1359` | root Docker readiness result covers DB/audit; keep existing liveness distinct |
| owner-equity-v2-runner | `deploy/compose/compose.yml:1102`; `crates/job-queue/src/bin/owner-equity-v2-runner.rs:603`, `:918`, `:969`; `crates/job-queue/src/owner_equity_v2.rs:112` | root process/health; app actor jobs of type owner_equity_v2, latest admitted generations and published signal coverage; EOD only |
| owner-beta-runner | `deploy/compose/compose.yml:1034`; `crates/job-queue/src/owner_beta/runner.rs:171`; `migrations/0049_owner_beta_price_recommendations.up.sql:4`, `:133` | root process/heartbeat health; app actor owner_beta_price_recommendation jobs and successful run finished_at/as_of; demand-driven |
| candidate-runner | `deploy/compose/compose.yml:974`; `crates/job-queue/src/bin/candidate-runner.rs:308`, `:438`; `migrations/0044_candidate_pipeline.up.sql:16`, `:42`; `migrations/0043_candidate_analysis_surfaces.up.sql:172` | root process/health; admin gated shared scheduler control and feed publication; actor-scoped candidate_compute jobs; no provider/source payload |
| recommendation-runner | `deploy/compose/compose.yml:892`; `crates/job-queue/src/bin/recommendation-runner.rs:511`, `:627` | root heartbeat health; app actor recommendation queue and successful runs; queued/running absent means queue idle, not dead |
| nt-backtest-worker-1 | `deploy/compose/compose.yml:1203`; `crates/job-queue/src/bin/backtest-runner.rs:725` | root process and DB/capacity readiness; app actor shared backtest queue; readiness failure is degraded, not proof daemon died |
| nt-backtest-worker-2 | `deploy/compose/compose.yml:1270`; `crates/job-queue/src/bin/backtest-runner.rs:725` | same semantics as worker 1; cannot assign old claims by guessing dynamic worker IDs |
| research-worker | `deploy/compose/compose.yml:560`; `data-pipelines/collectors/src/bin/research-worker.rs:219`; `scripts/ops/backfill-production.sh:575` | root daemon state; mode selects daemon vs bounded one-shot; stopped old daemon is normal in daily mode |
| kis-eod | `scripts/ops/install-kis-daily.sh:222`, `:252`, `:266`, `:279`; `scripts/ops/kis-daily-production.sh:363`, `:513`; `data-pipelines/collectors/src/worker.rs:2694`, `:2993` | root exact timer/service; admin shared Raw+Curated publication and proposed research execution summary; exit zero alone never means data published |
| market-stream | `crates/job-queue/src/bin/owner-equity-v2-runner.rs:889`; `migrations/0055_owner_market_stream.up.sql:173`; `migrations/0056_owner_market_stream_runtime.up.sql:39`; `crates/job-queue/src/owner_equity_v2/market_stream.rs:3906` | parent process evidence; proposed app actor read function without page lease, validates producer+subscription+cache lineage; independent of EOD task |
| intraday-calendar | `crates/job-queue/src/owner_equity_v2/intraday.rs:1341`; `data-pipelines/collectors/src/intraday_quotes.rs:303`, `:428`; `deploy/compose/compose.intraday-operational.yml:15` | app actor DB calendar read + existing RO operational window; two distinct outcomes, no provider call |
| production-backup | `deploy/systemd/lagrange-production-backup.timer:5`; `scripts/ops/run-production-backup.sh:305`, `:309`, `:346` | root fixed timer/service result; successful command has performed publication/verification; do not read archives, config, metrics paths or secrets |
| production-backup-verify | `deploy/systemd/lagrange-production-backup-verify.timer:5`; `scripts/ops/run-production-backup.sh:267` | root fixed timer/service result; command success is restore verification, not a new backup |
| tls-renewal | `deploy/systemd/lagrange-tailscale-tls-renewal.timer:5`; `scripts/ops/renew-tailscale-tls.sh:627`, `:642`, `:665` | root timer/result; successful validated no-op is success; not an independent expiry/TLS probe |
| docker | `deploy/systemd/lagrange-production-backup.service:3` | root fixed docker.service ActiveState/SubState; no Docker socket given to API |
| tailscaled | `deploy/systemd/lagrange-tailscale-tls-renewal.service:3` | root fixed tailscaled.service state; no peers/network addresses exposed |
| paper-scheduler | `deploy/compose/compose.yml:1340`; `scripts/ops/compose-release.sh:677` | root state; expected disabled by accepted Owner policy, never inferred from absence |
| metrics | `deploy/compose/compose.yml:1431` | root state; expected disabled, monitoring profile never enabled |
| live-node-owner | `deploy/compose/compose.yml:1471` | root state; forbidden live profile remains disabled; unexpected running is failure, no recovery action |

The old `/admin/workers` implementation at `crates/api-server/src/repos/ops.rs:560`
reads RUNNING claims only. Do not reuse it as idle-daemon inventory, grant
`worker_heartbeats`, or add process IDs to the public contract.
Bootstrap/migrate/raw-init/schema-check and manual backfill are not service rows.

### DB source map and bounded selections

Transactions use the existing actor boundary; see `crates/api-server/src/actor_tx.rs`.
`app` remains actor-local. `admin` has cross-user SELECT by RLS policy, not BYPASSRLS
(`migrations/0010_rls.up.sql:15`, `:143`); use it only for shared operational metadata.
Do not accidentally aggregate another owner's queue because the admin pool can see it.

| Read group | Selection / lineage | Source time, not invented time | Reader |
|---|---|---|---|
| Queue | jobs where owner_user_id = actor and job_type in the five catalog types; status, available_at, locked_at, finished_at only | locked_at is refreshed heartbeat, not started_at; lease per installed runner config (`crates/job-queue/src/queue.rs:12`, `:85`, `:319`) | app+actor; existing grants |
| Job outcomes | current jobs status plus finished_at; never error_message/payload_json or job/owner IDs in response | max finished_at for SUCCEEDED/FAILED; null if none | app+actor; no new job_attempts grant |
| ETF Raw | data_batches provider=KRX, market=KR, kind=EOD, fetch_mode=credentialed, complete 0022 lineage; group by batch_date | raw_as_of is batch_date; retrieved_at is source retrieval, never last publication success | admin; 0010+0022 |
| ETF Curated | candidate_price_publications + exact dataset_versions id/version/hash, dataset_id=krx_eod_bars and READY/WARNING; candidate_raw_batch_datasets surface=price,response_kind=bars + candidate_raw_batch_publications state=PUBLISHED,fetch_mode=credentialed; check all 11 fixed instruments' exact candidate_price_instrument_sessions coverage | data_as_of is latest fully covered verified session, not max unrelated dataset date; last_success_at is exact binding's published_at | admin only: `migrations/0042_candidate_source_contracts.up.sql:291`, `:346`, `:365`, `:388`, `:1956`; 0045 adds dataset_id binding |
| Owner V2 EOD | READY memberships, latest generation admitted by matching all five identity columns; published owner_equity_signal_snapshots and rows for current universe/admissions | v2_data_as_of = minimum current admitted generation last_session; v2_snapshot_as_of = exact fully covering snapshot as_of_session; success time is published_at, generation.created_at is NOT a success time | app+actor: `migrations/0053_owner_managed_equity_universe_v2.up.sql:146`, `:187`, `:224`, `:249`, `:476` |
| Candidate scheduler | candidate_scheduler_control enabled plus published feed for every enabled candidate_universe_registry entry; do not infer KOSPI200+KOSDAQ150 readiness from ETF11 | feed published_at/as_of_date; separate from EOD price coverage; scheduled failure threshold uses verified session+grace | admin: 0043:172, 0044:16, 0045:17 |
| Owner beta | owner_beta_recommendation_runs status=SUCCEEDED scoped to actor | finished_at/as_of; request-driven, no daily freshness promise | app+actor: 0049:4 |
| Calendar | exact KRX/date/type/source/version/batch/hash join of trading_calendars, trading_calendar_versions, data_batches, including credentialed CALENDAR/calendar.json | preserve each retrieved_at; independent query observed_at | app+actor: `crates/job-queue/src/owner_equity_v2/intraday.rs:1350` |
| Window | fixed operational activation + hash-pinned document; metadata/hash checks in existing loader; exactly today's entry | read time is observation; no source publication time exists, so none is fabricated | existing RO mount; `data-pipelines/collectors/src/intraday_quotes.rs:397`, `:590` |
| Stream | new 0058 safe aggregate described below; no account or order objects | DB observed_at; original producer heartbeat/status, received_at/provider_trade_at, actual expiry | app+actor; base private tables remain denied |
| Research attempts | new 0059 single bounded row; no provider metadata or protected daily-state read | DB attempt start/completion/success/failure times recorded when event occurs | research_writer writes; admin reads |

ETF11 fixed identities come from `configs/universes/kr-etf-core-v1.yaml:27`.
Never mix Owner stock V2 coverage, candidate universe coverage, or synthetic datasets
into ETF freshness. If no generation completely covers the expected fixed identities, data_as_of is
null unless a separately verified older complete generation exists. Partial
coverage cannot retain an unsupported complete-session claim. Raw progress alone gives RAW_ONLY and cannot make Curated current.
Rights checks retain the exact binding/entitlement, not the latest convenient grant;
reuse `price_dataset_entitlement_is_valid` semantics (0046), scoped source metadata
only. The dashboard does not return holdings, instrument values, IDs, paths, source
references, grants, hashes, SQL, free text or provider messages.

## 3. Authoritative deployment policy and host observation

The schema `$defs/policy` is the safe **installed policy receipt** at
`/var/lib/lagrange/service-status/policy.json`, written only by explicit installer
apply from an accepted immutable release. Snapshot embeds that
receipt. API additionally pins expected release commit from its immutable image and
checks receipt commit against it; it must not trust any self-declared snapshot mode
when the receipt is missing/invalid. The catalog defines supported profiles:

| Deployment | Expected execution |
|---|---|
| owner_daily | 10 regular serving daemons; research-worker daemon absent/stopped; exact current daily timer required; docker/tailscaled and all three ops timers required; Owner V2 EOD + owner_only/market_ws + operational_v1 required; paper/metrics/live disabled |
| owner_daemon | same, but research daemon required; current daily timer must be inactive/disabled; overlapping daemon and active daily timer = DAEMON_MODE_CONFLICT |
| development without installer | policy unavailable, host-dependent rows unknown, still return DB observations; never infer intentional disabled from absent containers; chart and existing API remain usable |
| infrastructure/backfill/other unrecognized mode | no supported Owner status policy; unknown/POLICY_MISSING, no attempt to activate services or manufacture a production expectation |

There is no auto-detection of daily mode from whichever timer happens to exist.
Installer accepts explicit `--research-mode daily|daemon`, release root and fixed
Compose project. It verifies the immutable release manifest/current symlink contract
used by `scripts/ops/compose-release.sh:235`; it copies only safe mode and validated
lease durations (from trusted effective configuration during installation, not a
collector env scan). Do not accept an ambient COMPOSE_PROFILES value.
Baseline release startup at `scripts/ops/compose-release.sh:673` always includes
research-worker: WP-2 must honor an installed receipt matching the exact commit to
exclude it in daily mode. With no receipt, preserve legacy release behavior.
Do not silently set owner_only/market_ws/off, stop a daemon, or enable a timer.
Both API refresh modes (`--refresh-market-stream` and `--refresh-intraday`) must
also validate a present receipt and preserve its project and optional snapshot
directory mount when their existing configuration gates admit the operation.
The legacy REST-only intraday refresh remains rejected by the current WS-only
configuration policy; do not reactivate REST to exercise it. Verify the supported
market-stream refresh with the real mode parser and synthetic valid WS settings.
Observed daily timer health is not an installation or API refresh
prerequisite: inactive/disabled timers and an unset next run are evidence to report.
Keep the separate daemon-versus-active-daily conflict gate when full release
startup actually selects the research daemon. This does not authorize changing
the state of a KIS service or timer during observer installation or API refresh.

Select daily unit `lagrange-kis-daily-<first 7 chars of exact 40-char commit>`.
Installer validates full-commit wrapper/manifest binding and unit digest; collector
checks fixed FragmentPath, empty DropInPaths, digest and timer Unit target. Another
release's timer is never a fallback. Short-prefix collision, altered unit, unmatched
receipt/current link or unexpected drop-in => POLICY_MISMATCH. Unit hashes may be
compared as opaque bytes, never decode/copy Environment/ExecStart into snapshot.
`Unit` is the timer target property; do not require it on the service. Installation
may require the exact physical unit files for integrity binding, but must separate
that binding from their current enabled, active or next-run state.
Ops units are exactly `lagrange-production-backup`,
`lagrange-production-backup-verify`, `lagrange-tailscale-tls-renewal` plus suffixes.
Daily schedule 16:30 KST; backup 02:15; verify Sunday 04:15; TLS 03:15.
Always read actual NextElapseUSecRealtime; do not calculate next run from wall dates.

### Safe collector contract

Run one-shot every 30 seconds with an exclusive nonblocking lock. Command timeout
2s, total collection deadline 20s, bounded stdout 16 KiB per command and 256 KiB
whole snapshot, no inherited free-form diagnostics. Fill unfinished entries with
`deadline`; never omit an ID. `TimeoutStartSec=25s`; no provider/HTTP checks.
Root needs the existing host Docker/systemd read surface; no new network access.
Use argv subprocesses with a fixed executable and allowlisted arguments, no shell.

* Docker: resolve exact Compose project+service labels with `docker ps -a` format
  output, exclude oneoff=True; formatted `docker inspect --format` selecting only
  State.Status, State.Health.Status, StartedAt, FinishedAt, ExitCode and the three
  Compose labels needed for matching. At most two matches then fail ambiguous;
  never choose arbitrarily. Do not read full inspect, Config.Env, mounts, args,
  healthcheck output/history or logs. sampled health is Docker's last known result,
  not a new probe. Missing health status is unknown, not healthy.
* systemd: `systemctl show` only fixed units/properties: LoadState, ActiveState,
  SubState, UnitFileState, Result, ExecMainCode (1 means normal exit), ExecMainStatus,
  ExecMainStartTimestamp, ExecMainExitTimestamp, LastTriggerUSec,
  NextElapseUSecRealtime, Unit, FragmentPath, DropInPaths. Canonicalize timestamps
  to UTC; zero/unset is null. Do not emit unit paths or arbitrary result strings.
  Map known success vs non-success to closed enum, reject unknown states.
  Validate the properties actually provided by each unit kind: services provide
  ExecMain fields; timers provide Unit/LastTriggerUSec/NextElapseUSecRealtime.
  Do not require timer-only keys on services or ExecMain keys on timers. Keep
  common identity/state keys required and duplicate/unknown-field checks strict.
* Host boot: read `/proc/stat` btime only, not boot UUID or host identity. On boot
  change discard any in-progress correlation and prior-boot process observations.
  Re-sample all units; no elapsed-time inference across reboot.
* Safe completion cache: root-private `/var/lib/lagrange/service-status/history.json`
  contains only the fixed scheduled IDs, exact release binding, completed invocation
  start/end/outcome and last observed successful finish. Initial absent history is
  RUN_HISTORY_EMPTY, not success. Update only after a positive start, positive end
  >= start, exited normally, matching unit and Result. Unset ExecMainStatus=0 is
  not a completion. Daily 74/75 are accepted systemd exits (`install-kis-daily.sh:266`)
  but are **not data success**; show RUN_RESULT_UNKNOWN unless DB publication proves
  the useful work. Preserve verified prior success when a later run fails or a host
  reboots; never seed history from current time, mtime, timer trigger or old log.

This cache is one latest record per fixed job, not a status history service. Its
absence does not permanently hide a mandatory observation: the next completed run
populates it. Unknown history must remain visible until then. Operations whose last
success predates installation cannot have that timestamp reconstructed safely.

### Filesystem and activation

Host serving directory `/var/lib/lagrange/service-status/public`: root:10001 0750;
`snapshot.json`: root:10001 0640 regular, nlink=1. Private parent/history/lock are
root:root with 0700/0600. No application writable ancestor. API container is
10001:10001 (`crates/api-server/Dockerfile:69`, `:76`). Mount **public directory**
RO at `/run/lagrange/service-status`, using `create_host_path:false` in the optional
overlay. Never single-file bind mount: rename must become visible to API readers.
No Docker socket, systemd bus, sudo, broker state or new host-path read permission.

Write temp on same filesystem O_CREAT|O_EXCL|O_NOFOLLOW, fchmod/fchown, serialize,
validate, fsync file, os.replace, fsync directory. Reject symlink/nonregular/hardlink,
unsafe ownership/mode, duplicate keys, invalid UTF-8, truncated/oversize data, NaN,
unknown fields/enums/version, duplicate/missing IDs and inconsistent times.
Use directory FD + openat O_NOFOLLOW for reader; fstat before/after bounded read.
Re-open snapshot by name on each cache refresh. Failed reads never refresh validity.
Installer defaults to plan; check/preflight are read-only; apply is separate and
explicit, not authorized in this assignment. Failed installation must restore prior
files and activation/enabled state of only the observer's own timer/service, with
the restored units reloaded. A partial activation failure must not leave a new
success receipt or observer running; a failed rollback is a typed incomplete
recovery outcome. Optional overlay is used only after validated
installation; absent collector must not break an otherwise valid old release.

## 4. Exact wire and temporal semantics

`configs/operations/service-status.schema.json` is JSON Schema 2020-12 with local
refs. Root oneOf accepts host snapshot, API response, deployment policy or catalog.
The additional `$defs/evaluationInput` is fixture-only and never a public endpoint.
Host and API are deliberately different DTOs. Every object rejects extra fields.
API `/api/v1/admin/service-status` returns only `$defs/apiResponse`.
Host observations contain the 20 non-capability IDs; API always contains all 22 in
catalog order. Semantic validation additionally checks IDs are exactly the set and
policy profile/mode correspond, timer/process fields match catalog source, safe
source and timestamp order, and count identities are consistent. Null = applicable
but unknown/no history; omission is never overloaded to mean healthy. Metrics are
fixed nullable fields; `not_applicable` explicitly distinguishes inapplicability.
For roles without work, work_status=not_applicable and all work counts/times null.

All clocks use UTC wire instants with microsecond precision at most, KST only for
session selection/display. `generated_at` is response assembly, **never evidence**.
`observed_at` on a row is minimum of the applicable successful source observation
instants; null if none. Unknown source has null observed_at/valid_until; a composite DB source with some
successful read groups uses result=partial, preserves their observation time, and
has null combined valid_until; it does not
invalidate other source objects. `valid_until` is minimum accepted evidence bound;
unknown combined row has null bound. Preserve individual source timestamps when
partial failure occurs. DB query time is an actual new observation of stored state;
it must not replace retrieved_at, heartbeat, event, success or publication times.

* Host observation valid through observed_at +90s inclusive, expired when now is
  later. Collection start <= every successful observation <= generated_at, whole
  duration <=20s. Each host observation must be >= booted_at. Any timestamp > now
  rejects that source (no silent future-skew allowance). Envelope future/invalid
  rejects entire host source; isolated command failure only affects that entry.
* DB read evidence valid for at most original observed_at +30s, or an earlier
  relevant domain boundary. Cache refresh is due at age >=5s. Cache TTL is 5s and
  cache hit does not move original observed_at. Snapshot parser cache key includes
  file identity; no permanently opened stale inode.
* Current calendar is exactly today KST. All three joined retrieved_at values
  must be <= DB now and >= now−36h (equality accepted), source kis and version
  kis-chk-holiday-v1:schema-1, exact batch/hash/type/date consistency. Missing,
  future, stale or duplicate/contradictory evidence fails closed. Current-day
  calendar bound includes next KST midnight and each retrieved_at+36h.
* Window uses existing `IntradaySessionWindowContract::from_source(OperationalV1)`.
  Require today's entry and valid metadata/activation/hash; preserve its actual
  evidence_retrieved_at and require it to be non-future on the same KST date
  (`data-pipelines/collectors/src/intraday_quotes.rs:349`). Distinguish missing
  window, invalid window and missing calendar. Open interval is [open,close):
  open equality is open, close equality is closed (`market_stream_runtime.rs:209`).
  Dashboard checks BOTH calendar and window even for CLOSED; this is intentionally
  stricter display assurance than runtime's early window-closed return at :331.
* EOD eligibility reproduces `owner_equity_v2/schedule.rs:11`: before 16:30 KST
  use the previous calendar date, at/after use today; then select latest verified
  TRADING session <= eligible date. To assert it is *latest*, require continuous
  verified TRADING/CLOSED evidence from candidate session through eligible date,
  bounded to 370 calendar days; a gap is CALENDAR_COVERAGE_GAP, not “holiday”.
  Historical calendar evidence uses exact lineage with no fabricated retrieval
  and no current-day 36h test; historical rows are not intraday session proofs.
  No weekdays, UTC dates, max arbitrary dataset or 24-hour freshness heuristics.
* For newly eligible session, prior complete publication before 18:30 KST is
  waiting/EOD_PENDING; absent first-ever publication is also pending only when
  the latest eligible deadline is still future. At deadline equality, lag is
  degraded/EOD_STALE. Existing older missing sessions are already overdue.
  Holidays/weekends retain the prior trading session and its deadline.
* Stream semantics are exactly `market_stream.rs:3906–3984`: positive cache quote
  evidence, ACKED subscription/current epoch+revision, exact admission and all
  session proof/calendar/window/fence/gap lineage, connected/no reason/no gap,
  producer heartbeat <= now and age <=10s; producer expiry > now+5s; both original
  receipt and provider trade age <=30s, no future. Publication max age of 3s at
  commit (:43) is independent of display TTL. Do not substitute received_at for
  provider_trade_at. Healthy stream must cover **every demanded identity**; one
  fresh symbol cannot hide 29 stale ones. Per-identity halted is waiting only
  when validated, not a proxy for disconnected producer.
* Row validity includes next relevant semantic transition: producer heartbeat+10s,
  receipt/event+30s, lease expiry−5s (exclusive), demand expiry (exclusive),
  calendar expiry/midnight, window open/close, EOD cutoff/deadline, queued
  available_at and available_at+120s while pending, claim locked_at+configured
  lease, recent failure finished_at+24h, RUNNING research heartbeat+90s, and
  scheduled run start+configured timeout. EOD cutoff/deadline bounds apply only
  to rows whose work depends on that eligibility/publication assertion.
  Exact strict boundary wins over generic inclusive validity. UI expires at the
  boundary conservatively, so a 30s poll cannot display a stream healthy for 90s.
  Accepted inclusive equality retains the actual boundary, even at now; never
  add one microsecond to the published bound to compensate for an inclusive
  predicate. Predicate truth and conservative published validity are distinct.
* A fresh DB observation of closed/no-demand or diagnosed degradation/failure
  keeps its observation bound; an elapsed positive-assurance deadline that
  already caused that diagnosis is not a future transition. Preserve original
  source clocks as diagnostic data. This does not discard all bounds <=now or
  give every waiting/failed row 30s: each accepted positive assertion, including
  live/halted metrics and pending quote readiness, retains every prerequisite's
  assurance boundary, even within a degraded row. Valid demand expiry, calendar
  expiry/midnight and window transitions still shorten validity. Cache hits do
  not refresh observations, and arbitrary aggregate bounds must not be widened.

### State reducer (all predicates use only valid evidence)

Evaluate expectation first. Disabled + observed running => failed/UNEXPECTED_ACTIVE;
disabled + confirmed absent/inactive => disabled/FEATURE_DISABLED. A source failure
cannot prove disabled; return unknown instead. Missing/invalid policy => unknown
expectation and status even if some individual facts remain displayable.

Evaluate process and work separately, then combine by severity:
**failed > degraded > unknown > waiting > healthy**. Disabled is neutral in overall;
all neutral yields disabled, never healthy. Required unknown prevents healthy;
known failure wins over an unrelated unknown. `research-worker` alone does not
contribute, but its effective execution state contributes to `kis-eod`.

| Evidence | Process | Work | Result / reason |
|---|---|---|---|
| Required running + healthy probe, no work contract | running | not_applicable | healthy/PROCESS_RUNNING |
| Required container absent on successful list | missing | independently evaluated | failed/PROCESS_MISSING |
| Running but unhealthy probe | running | independently evaluated | degraded/PROCESS_UNHEALTHY (readiness not death) |
| Restarting/created/activating | starting | independently evaluated | waiting/PROCESS_STARTING for <=90s since valid start; then degraded; absent start unknown |
| Exited required daemon or failed unit | failed | independently evaluated | failed/PROCESS_FAILED |
| Command unavailable/denied/timeout | unknown | preserve DB result | unknown unless independent known work failure/degradation |
| Actor queue empty + process healthy | running | idle | waiting/QUEUE_IDLE, no fabricated heartbeat |
| Eligible queued, no RUNNING | running | pending | waiting/QUEUE_PENDING for <=120s since available_at, then degraded/QUEUE_DELAYED |
| Only future queued | running | scheduled | waiting/QUEUE_PENDING; validity ends at earliest available_at |
| Valid RUNNING claim | running | active | healthy/QUEUE_ACTIVE; claimed work is not completed success |
| RUNNING locked_at+lease <= DB now | independently evaluated | failed | failed/CLAIM_EXPIRED; never silently requeue |
| Terminal job FAILED in [now−24h,now], not requeued/succeeded | independently evaluated | degraded | degraded/RECENT_JOB_FAILURE; show count and latest failure |
| Same job succeeds after requeue | independently evaluated | idle/active | historical failure no longer drives state; RECOVERED fixture; no unrelated success erases another failed job |
| Older failed terminal jobs (>24h) | independently evaluated | idle if no demand | waiting; count bounded to last 24h, no eternal red daemon |
| Daily enabled+active timer, matching unit, service inactive between runs | inactive | scheduled or current | waiting; never fail old stopped research container |
| Expected timer disabled/missing/inactive or next run absent | independently evaluated | failed | failed/TIMER_DISABLED,TIMER_MISSING,TIMER_INACTIVE,NEXT_RUN_MISSING |
| Scheduled run in progress | running | active | healthy/RUN_ACTIVE through existing unit timeout inclusive (daily/backup 12h, verify 6h, TLS 10min); later RUN_OVERDUE; data may independently be overdue |
| Completed non-success run | inactive | failed | failed/RUN_FAILED until later verified success; preserve prior success timestamp |
| Successful command but history/DB data unknown | inactive | unknown | unknown/RUN_RESULT_UNKNOWN; exit 0 is not data freshness |
| Verified calendar/window absent, stale or invalid | not_applicable | failed | failed/specific calendar or window reason, before no-demand logic |
| DB calendar query failed | not_applicable | unknown | unknown/DB_TIMEOUT or DB_UNAVAILABLE; absence is not query failure |
| Valid closed session or before open | parent process | closed | waiting/SESSION_CLOSED or SESSION_PREOPEN |
| Valid open session + zero valid demand | parent process | no_demand | waiting/NO_ACTIVE_DEMAND; status GET creates no demand |
| Open demand, missing/expired producer | parent process | failed | failed/PRODUCER_UNAVAILABLE or PRODUCER_STALE |
| Open demand, disconnected/stopped producer | parent process | failed | failed/STREAM_DISCONNECTED |
| Connecting/backoff or known gap | parent process | degraded | degraded/STREAM_CONNECTING,STREAM_BACKOFF,STREAM_GAP |
| Valid producer + all requested live receipts | parent process | current | healthy/DATA_CURRENT |
| Some stale/missing receipts | parent process | stale/pending | degraded/QUOTE_STALE or QUOTE_PENDING |
| ETF/V2 required data behind verified session | independent | pending/stale | waiting before grace, degraded at/after deadline; each surface evaluated separately |


Typed producer status mapping is closed and takes precedence over receipt freshness.
Preserve the exact safe 0056 code in reason_codes. FEATURE_DISABLED,
CALENDAR_UNAVAILABLE, SESSION_WINDOW_UNAVAILABLE, CONNECTION_LOST,
SUBSCRIPTION_REJECTED, APPROVAL_UNAVAILABLE, PRODUCER_UNAVAILABLE and ACCESS_REVOKED
are failed. Other nonnull 0056 codes are degraded. NO_ACTIVE_DEMAND and
SESSION_CLOSED are waiting only when fresh independent demand/day evidence agrees;
otherwise degraded/QUOTE_LINEAGE_INVALID. producer_status_at maps to last_state_change_at: it is the actual persisted
state-transition time, not a failure onset or historical maximum. Stream
last_failure_at stays null because this source has no failure-history timestamp. A null status
code still requires all independent liveness/receipt tests.

Ordered reason codes: failed reasons, degraded reasons, unknown reasons, waiting
reasons, positive/neutral reasons; each group lexicographic, unique, maximum 12.
Truncation must preserve all reasons establishing the winning status. Do not append
arbitrary SQL/OS/broker errors. Source-specific failures retain typed source result.
Overall uses rows, not a sum of queue counts or process health badges.

## 5. HTTP authorization, audit and bounded load

Use authenticated session extraction + Owner role check on every request before
cache lookup or any operational read. Owner product entitlement failure does not
block this diagnostics GET. Member=403, anonymous/expired session=401, no detail
body. Use the existing error envelope and request ID contract. No CSRF requirement
for GET; no query parameters, no arbitrary paths/IDs, no SSE, no mutation handler.
`Cache-Control: private, no-store`, `Vary: Cookie`; never CDN/Next fetch cache.
HTTP 200 may be partial or entirely unknown if the authorized query ran but sources
failed; 503 is for endpoint/audit infrastructure failure, not a missing collector.

`OpsRepo` Owner checks/denials and each successful admin read are audited
(`crates/api-server/src/repos/ops.rs:119`, `:585`). Preserve that policy:
one audit event `admin.service_status.read`, target type `service_status`, target
`all`, null before/after; reason null or fixed DENIED/PARTIAL/SOURCE_UNAVAILABLE.
Every admitted response, including cache hits, requires audit success before body.
Audit failure => 503 and no data; do not leak raw TenancyError Database text through
the generic mapper. Use typed status errors. Authentication's own audits stay intact.

Admission before source access: per-authenticated actor at most one request/5s (elapsed >=5s admits),
burst 1; global at most 10 admitted requests/s, burst 10; 429 with Retry-After=5,
no source read, status payload or status audit event. This is a capacity rejection,
not an authorization verdict. Admitted Member denial is audited. Limit in-memory
actor slots to 128, TTL eviction 5 minutes, never cache across actor/session binding;
new slots under saturation get 429. Max two concurrent source refreshes; avoid
unbounded waiting queues (try-acquire ->503). Accepted polling yields <=12 status
audit writes/min/actor, globally <=600/min/process. Capacity limits are per API
process; scale-out requires a separately reviewed shared limiter before multiplying
replicas. Existing authentication DB work is outside this new endpoint budget.

Host parser refresh once/5s globally with single-flight; shared DB metadata once/5s;
actor DB projection once/5s keyed by actor, session authorization and policy revision.
Auth still revalidates each request. Never share Owner-specific output across users.
At most eight aggregate statements per refresh (setup SET LOCAL statements do not
count as evidence queries): queue/outcome (all five types),
ETF Raw/Curated, V2 coverage/snapshot, candidate scheduler/feed, research execution,
calendar current/history, stream aggregate; eighth reserved for required rights
validation. Transaction READ ONLY for evidence, SET LOCAL statement_timeout=250ms,
lock_timeout=50ms, total evidence deadline 2s, bounded 256 KiB output. Query timeout
marks only dependent groups unknown, separate transactions/savepoints avoid poisoning
other groups. Audit has separate 500ms deadline; total handler <=3s.

No unbounded in-memory result scans: queue aggregate at most 10,001 rows per group
(current active or last-24h finished; 10,001 signals QUERY_LIMIT); history times use
indexed LIMIT 1. V2 EOD membership coverage supports up to 100 identities, matching
the worker runtime ceiling (`crates/job-queue/src/owner_equity_v2/runtime.rs:42`,
`:59`); read at most 101 to detect overflow. The configured Owner policy may be
lower. This is distinct from stream demand, which remains <= existing
20 leases/30 unique identities, ETF exactly 11; candidate registry max 16; calendar
<=370 dates; candidate price search <=100 pinned generations, excess QUERY_LIMIT.
Do not truncate then label complete. New status query indexes in 0058 are listed
below; use actual isolated EXPLAIN and statement timeouts to verify bounds.

## 6. Dashboard placement and polling

Read `apps/web/AGENTS.md` and `CLAUDE.md`. Next resolves from apps/web to the root node_modules/next package. WP-1 inspected
its local `dist/docs/01-app/01-getting-started/05-server-and-client-components.md`,
`06-fetching-data.md`, and `15-route-handlers.md` relevant composition/fetch/caching
sections. WP-4 must re-resolve and read the installed guides before code. Do not fetch documentation or install dependencies
as part of WP-1.

`apps/web/app/(authenticated)/stock-beta/page.tsx:141` calls the lazy Owner product
gate. `apps/web/components/pages/owner-beta-product-route.tsx:24` authenticates
before invoking renderProduct. Keep that gate. Inside its authorized callback,
render one ServiceStatusPanel sibling **before** the async product subtree in
Suspense. Its locale/session-reset props come from this same authenticated session.
The panel is outside the try/catch at page.tsx:72 and outside entitlement/membership/
signal/chart failures. Do not await the status GET for SSR or the chart. This needs
no new product entitlement bypass and no change to OwnerBetaProductRoute.

`stock-beta-workspace.tsx`, terminal shell and dashboard are read-only references
for this version: wrapping the lazy product subtree avoids invasive prop plumbing.
The chart remains catalog's first full-width widget
(`apps/web/components/stock-beta/dashboard/widget-registry.ts:34`). Use a compact
panel-local CSS summary above the product subtree; details collapse by default.
Do not insert the status panel as a dashboard widget or change chart renderer,
data adapter, market-stream controller, lease, subscription or SSE behavior.

One client controller per mounted Owner page uses a 30s base interval, serial
requests only; expanded/collapsed share it. For an earlier future row validity
deadline, schedule the next refresh one second before that deadline, with a floor
of five seconds after the preceding request settles. Subtract the full measured
request duration from the received validity budget. This avoids leaving a fresh
stream's ten-second heartbeat evidence expired throughout most of a fixed 30s
interval, while respecting the existing admission budget. Expired evidence still
expires independently while a request is pending; never extend validity to cover
the next poll. An already-past deadline cannot cause an immediate retry loop.
Retry-After takes precedence over this schedule. Initial/visible/online refresh
is immediate subject to the same request floor and active backoff; pause on
hidden/offline, abort on unmount/auth identity change, reject late generation
responses. Use monotonic elapsed time from response plus original source bounds
for expiry; never use browser wall clock to extend validity. At response error
mark transport unavailable immediately, retain timestamp and rows visibly stale;
expire every green status on its own validity bound even without another poll.
401/403 clears all cached operational details immediately; no stale Owner data on
logout/permission denial. 429 follows Retry-After without duplicate loop or spin.
Keyboard disclosure, stable keys, text+icon status, ko/en copy, KST absolute and
relative observation time, forced-colors and narrow-screen wrapping are required.
Sort problem groups only on deliberate data reconciliation, not every countdown
tick; preserve focused row. Don't expose internal IDs/source role jargon as UI copy.

## 7. Mandatory gaps and minimal closure proposals

These are implementation requirements, not permission to leave permanent unknown.
Unknown is allowed on actual runtime missing/failure, or before first observation;
API release acceptance requires implemented, tested read paths for every gap.

### G1 — Stream observation without creating demand (0058)

Current 0055:498 denies app/admin producer/subscription tables. 0056:39 delivery
function requires a live lease from the current page; using it would either hide
no-demand producer state or tempt creation of a lease. Do neither.

Add `public.owner_service_status_stream(p_owner_id uuid, p_session_hash text,
p_slot_id uuid, p_grant_id uuid, p_contract_sha256 text, p_window_sha256 text,
p_open_at timestamptz, p_close_at timestamptz)` returning exactly one safe aggregate:
`observed_at timestamptz, demand_count bigint, live_count bigint, halted_count bigint,
producer_present boolean, connection_state text, status_code text,
producer_status_at timestamptz,
producer_heartbeat_at timestamptz, producer_lease_expires_at timestamptz,
oldest_received_at timestamptz, oldest_provider_trade_at timestamptz,
last_received_at timestamptz, lineage_valid boolean, gap_present boolean,
valid_until timestamptz`. Nullable producer fields for an absent producer.
No identities, UUIDs, lease IDs, session hashes, prices, grant contents or payloads
returned. `status_code` is the existing producer closed enum, mapped by API into
the status reason enum; never pass text through unchecked.

SECURITY DEFINER owner migration_owner, fixed search_path=pg_catalog, qualified
names, no dynamic SQL/DML/row locks/advisory locks/notifications. REVOKE ALL FROM
PUBLIC, app, worker, admin, audit_writer, research_writer; GRANT EXECUTE only app.
Guard session_user=app, actor GUC equality, stream session GUC equality, nonnil
identity/session/slot/grant/network/window-hash pins, current Owner user_role,
active unexpired unrevoked web session, and exact
existing `owner_market_stream_api_binding_valid` (0057) pins/rights. Fail without
returning any state on invalid authorization. This is the same trusted server
actor/session model, not caller-chosen cross-user read access.

Bounds are either both present, ordered and on today's KST date, or both NULL
for a separately validated current CLOSED operational entry. Exactly one NULL
is invalid. Retain every identity/session/grant/rights guard in either mode;
NULL bounds alone never prove CLOSED. Producer proof/lineage success and positive
live/halted coverage require a validated currently open [open,close) window.
CLOSED, preopen and close equality return no positive proof or coverage; preserve
the authenticated demand denominator and real diagnostic clocks, never fabricate
a window or infer zero demand. Missing/invalid calendar/window is still diagnosed
independently, and cannot turn failed binding into closure or no demand.

Demand aggregates ALL current valid leases belonging to the authenticated Owner,
including other still-valid Owner browser sessions. Recheck each lease's session
via `owner_market_stream_session_valid`, state=ACTIVE, expiry>DB now, current READY
membership/admission (anti-join newer generation), exact owner/instrument/generation.
This GET never creates, renews, releases or locks a lease. Pin producer to the exact
current active grant revision and slot, even with zero demand. Count distinct
identities, not duplicate tabs; max 30. Cache eligibility repeats all predicates
in `market_stream.rs:3906–3984`, including calendar proof join and supplied validated
window hash/bounds. Return counts of all expected identities, not only visible
cache rows; missing/stale/revoked rows must not disappear from the denominator.
SQL live_count is inclusive validated coverage; halted_count is its validated
halted subset. The adapter first validates integral 0 <= halted_count <=
live_count <= demand_count <=30, then converts once to the exclusive evaluation
input: live=SQL live_count-halted_count, halted=SQL halted_count. Never clamp
invalid counts or convert already-exclusive fixture inputs. All-halted is
waiting/INSTRUMENT_HALTED with work_status=closed, under the same positive
assurance requirements as normal live delivery.

The existing 16-column result is unchanged. valid_until is the minimum original
DB observation+30s and relevant demand/window/calendar transitions, additionally
capped by each currently accepted positive producer/quote prerequisite. Include
heartbeat+10s and producer expiry-5s when readiness/coverage relies on them;
include receipt/event+30s minima over validated covered identities, also for
partial coverage. Compute filtered assurance minima internally; retain unfiltered
oldest receipt/provider clocks for diagnosis. An already-failed prerequisite's
old success deadline does not instantly expire a fresh negative observation.
Do not extend healthy or validated halted assurance, even though halted is waiting.
The aggregate bound carries these internal minima without exposing identities or
adding columns; independent current calendar/window bounds also cap the row.

A zero-row function result after successful HTTP Owner authentication maps to
failed/STREAM_BINDING_UNAVAILABLE with stream counts/times null; it must not reveal
whether the hidden grant, config or rights check failed, and is never zero-demand. Missing
calendar/window is independently diagnosed before using no-demand.

Add queue status indexes on jobs(owner_user_id,job_type,status,available_at) and
jobs(owner_user_id,job_type,finished_at DESC) WHERE finished_at IS NOT NULL;
add owner_beta_recommendation_runs(owner_user_id,finished_at DESC) WHERE
status='SUCCEEDED' AND finished_at IS NOT NULL for its indexed latest-success lookup.
add shared publication index only if EXPLAIN demonstrates existing indexes cannot
meet the 250ms bound. No job_attempts read grant. Down migration revokes/drops this
function and only its new indexes, with no edits to 0055–0057 or existing RLS.
Existing stream helpers are private; keep producer source unchanged: new SQL safe
read is tested against current delivery semantics rather than exposing internals.

### G2 — Research attempt and success (0059)

`scripts/ops/lib/kis-daily-state.py:102` records dates/status/identity, no attempt
clock. `WorkerObserver` output is a log (`research-worker.rs:135`), not an API source.
Do not parse either or infer timestamps from filenames/mtime. Systemd is sufficient
for daily wrapper attempts but not daemon attempts or confirmed Curated success.

Add `public.research_execution_status` singleton with:
`id text PRIMARY KEY CHECK(id='kis-eod'), attempt_started_at timestamptz NOT NULL,
heartbeat_at timestamptz NOT NULL, attempt_finished_at timestamptz,
attempt_state text CHECK IN ('RUNNING','SUCCEEDED','FAILED','NO_WORK'),
last_success_at timestamptz, last_failure_at timestamptz, failure_code text,
target_session date, published_session date, updated_at timestamptz NOT NULL`.
Closed failure codes: PIPELINE_FAILED, PUBLICATION_FAILED, RIGHTS_UNAVAILABLE,
EXECUTION_INTERRUPTED. No provider error body, references, batch IDs or counters
requiring history. `published_session` is updated only after exact successful
Curated DB binding, never on Raw-only success. NO_WORK updates completion but
preserves last_success. Insert initial row only on actual first attempt.

Owner migration_owner, ENABLE+FORCE RLS. REVOKE ALL from every serving role/PUBLIC.
research_writer SELECT/INSERT/UPDATE only, constrained singleton USING/WITH CHECK;
admin SELECT only with read policy; app/worker/audit_writer get no access. No grant
on any existing table changes. Constraints: heartbeat>=start, finish>=start,
RUNNING iff finish null, failure code null except FAILED; timestamps <= writer DB
clock; success publication checks derive exact existing binding in writer code.
Do not fabricate historical initial success. Down refuses if row exists unless
an explicit future rollback data-retention decision removes it; no silent deletion.
Acquire an exclusive table lock before the emptiness check, retaining it through
the drop in the migration transaction. A concurrent first writer that commits
before the guard may run must cause refusal, not silent loss of that first row.

In collectors add a typed repository module, wired by `bootstrap_worker` in
worker.rs. Instrument **only** the parent ResearchWorker run_once cycle (:4314)
and each run_daemon scheduled cycle (:4325), covering target recovery and
run_target_with_retry as one attempt. The daemon's startup recovery outside a
scheduled cycle is not a new EOD attempt. The parent owns heartbeat/completion;
internal ingest/recovery subprocesses (`research-worker.rs:228–230`) never update
this row. This prevents children from overwriting parent outcome. For the separate
credentialed bounded backfill function (`worker.rs:1412`), put one attempt around
the complete call, including final Curated publication, using its already-created
pool. It does not call the parent ResearchWorker cycle, so these scopes do not nest.

**Do not instrument the daily-range raw-only functions at :1515/:1595.** They have
no DB publication (`research-worker.rs:490`) and must remain credential-free on
reuse. Their wrapper attempts/failures are observed via systemd, and Curated
success comes from existing publication records. No DB connection is added to
calendar-only, healthcheck, Raw-only, KIND, OpenDART or child recovery commands.

Repository updates use an attempt_started_at conditional fence and one producer
operation at a time; concurrent standalone attempts fail the status write with a
typed instrumentation-unavailable code rather than overwriting another active
attempt. A producer may heartbeat/finish only after its start write confirms its
own fence; an unconfirmed start never borrows the preceding attempt's fence.
Heartbeat every 30s while active; observed RUNNING with heartbeat age >90s becomes
degraded/EXECUTION_INTERRUPTED, without repair, inferred completion time or a
write-back failure record. At exactly 90s it is not stale on this predicate.
A subsequent **actual producer attempt** may replace a RUNNING record only when
its persisted heartbeat is strictly older than 90s, acquiring a new confirmed
fence atomically. This prevents a crash or failed finish write from blocking all
future attempt records. Preserve verified prior success/failure/publication
fields; do not fabricate the interrupted attempt's completion or failure time.
Concurrent starts still have one winner, and the older fence can no longer
heartbeat or finish after takeover. This is recording a new attempt, never an
observer-triggered restart, repair, retry or write-back.

**Observability limit:** this singleton is the last observed/persisted attempt,
with genuine source timestamps. It does not prove that no later attempt happened.
If a start write fails and an older terminal success remains stored, a successful
read can show only that older recorded success; it cannot infer a later start or
failure, or substitute unknown solely because an unobserved write might have failed.
An absent row or unavailable read makes attempt evidence unknown. Completely
unrecorded attempts are undetectable through this row. Do not add log parsing or
another persistence channel to close that limit. Raw/Curated publication facts
remain independently evaluated, even when attempt recording fails; a prior
recorded success never proves a new attempt or current publication freshness.

Required WP-3A failure-injection expectations (not new API input fields):

| Status operation failure | Required result and observable evidence |
|---|---|
| Start definitely rejected before commit | Preserve the pipeline's own outcome; no confirmed fence means no heartbeat/finish for this attempt. A prior row stays unchanged (or absent); never manufacture a new success or timestamp. |
| Heartbeat definitely rejected before commit | Preserve the pipeline's own outcome and original fence; do not advance heartbeat locally as evidence. If the observed row stays RUNNING, its actual persisted heartbeat ages into degraded after 90s. A later confirmed finish can record its genuine outcome under the same fence. |
| Finish definitely rejected before commit | Preserve the pipeline's own outcome and original fence; no new status success/failure/completion is asserted. A retained RUNNING row ages into degraded; independently committed publication remains independently visible. |
| Stale fence or ambiguous write timeout | An older attempt cannot update a newer attempt's row. A timeout does not prove rollback: project only what a subsequent read actually observes, never a guessed commit or locally synthesized success. An unconfirmed start still grants no fence ownership. |

Never retry collection or replace its result because status recording failed.
Tests must cover prior terminal success and no prior row on rejected start, rejected
heartbeat/finish, stale-fence denial and ambiguous completion, preserving existing
last-success timestamps unless an actual fenced success is persisted. No projection
fixture changes: identical observed inputs remain indistinguishable. Exactly the
new module, existing worker.rs and declaration in lib.rs are needed; CLI code
remains read-only.

### G3 — Durable scheduled last success and expected mode

WP-2 implements the fixed safe completion cache and policy receipt (§3); no wrapper
log parsing or backup/TLS file access needed. Last success before first observation
is explicitly unavailable, not invented. Current failed result and prior success
are separate. Across reboot/collector restart a validated last success persists;
a stale snapshot is still unknown. Existing scheduled timeouts are authoritative;
collector doesn't infer a TLS validity duration or backfill completion from exit 0.

### G4 — Complete EOD session evidence

Existing complete price publications and V2 admissions are sufficient read sources;
no new publication timestamp/last_session columns needed. Incomplete historical
calendar coverage is a visible failure requiring an already-authorized operator
publication path, never an automatic provider call. A per-day raw batch frontier
cannot replace Curated exact coverage. This is data absence at runtime, not an
unimplemented permanent unknown. No status schema widens broker/data rights.

## 8. Mutually disjoint downstream ownership

Paths are repository-relative. No implicit “related files” permission. WP-1
contract/schema/catalog/shared fixtures remain read-only after acceptance; changes
return to coordinator/WP-1. No worker edits another worker's file or diagrams.

**WP-2 only (new)**:
`scripts/ops/collect-service-status.py`, `scripts/ops/install-service-status.py`,
`scripts/ops/test_service_status.py`, `scripts/ops/test_install_service_status.py`,
`scripts/ops/lib/service-status-policy.py`,
`deploy/systemd/lagrange-service-status.service`,
`deploy/systemd/lagrange-service-status.timer`,
`deploy/compose/compose.service-status.yml`.
**WP-2 only (existing)**: `scripts/ops/compose-release.sh`,
`scripts/ops/validate-production-config.sh`, `scripts/ops/production-ops-self-test.sh`,
`scripts/ops/production-ops-static-check.sh`,
`scripts/ops/kis-read-compose-self-test.sh` (synthetic release fixture and refresh
regression only; the shared KIS mode parser remains read-only).
No backup/TLS/KIS wrapper or installer changes required: receipt pins existing units;
install-service-status owns the new activation. Existing release-image-manifest
and dotenv libraries are reused read-only, no shared library edits necessary.

**WP-3A only (new; completes before WP-3B)**:
`data-pipelines/collectors/src/service_status.rs`,
`data-pipelines/collectors/tests/service_status.rs`,
`crates/api-server/tests/service_status_reads.rs`,
`crates/api-server/tests/service_status_reads_support/mod.rs`,
`migrations/0058_service_status_reads.up.sql`,
`migrations/0058_service_status_reads.down.sql`,
`migrations/0059_research_execution_status.up.sql`,
`migrations/0059_research_execution_status.down.sql`.
**WP-3A only (existing)**:
`data-pipelines/collectors/src/lib.rs`, `data-pipelines/collectors/src/worker.rs`.

**WP-3B only (new)**:
`crates/api-server/src/http/service_status.rs`,
`crates/api-server/src/repos/service_status.rs`,
`crates/api-server/src/service_status.rs`,
`crates/api-server/tests/service_status.rs`,
`crates/api-server/tests/service_status_support/mod.rs`,
`apps/api-server/scripts/openapi-service-status.mjs`.
**WP-3B only (existing)**:
`crates/api-server/src/lib.rs`, `crates/api-server/src/http/mod.rs`,
`crates/api-server/src/http/state.rs`, `crates/api-server/src/repos/mod.rs`,
`crates/api-server/src/runtime.rs`, `crates/api-server/src/contract.rs`,
`crates/api-server/tests/http_admin.rs`, `crates/api-server/tests/openapi_contract.rs`,
`apps/api-server/scripts/openapi-spec.mjs`, `apps/api-server/openapi.json`,
`apps/api-server/generated/openapi.ts`.
Each package uses its own new test support wrapping existing Harness; no edits to shared test harness or
job-queue source are planned. No Cargo/workspace/npm dependency change is authorized.

**WP-4 only (new)**:
`apps/web/components/service-status/service-status-panel.tsx`,
`apps/web/components/service-status/service-status-panel.module.css`,
`apps/web/lib/operations/service-status-client.ts`,
`apps/web/lib/operations/service-status-contract.ts`,
`apps/web/lib/operations/service-status-controller.ts`,
`apps/web/lib/i18n/dictionaries/service-status.ts`,
`apps/web/tests/service-status-panel.test.tsx`,
`apps/web/tests/service-status-controller.test.ts`,
`apps/web/tests/service-status-contract.test.ts`,
`apps/web/tests/service-status-page.test.tsx`,
`apps/web/tests/support/render-async-page.tsx`.
**WP-4 only (existing)**:
`apps/web/app/(authenticated)/stock-beta/page.tsx`,
`apps/web/tests/stock-beta-intraday-page-seam.test.tsx`,
`apps/web/tests/owner-beta-equity-signals-surface.test.tsx`.
No dashboard/terminal/market-stream/controller/Owner route source edit is required
by the sibling composition in §6. The two existing page tests may switch from
synchronous renderToStaticMarkup to completed async SSR through the new helper,
preserving all product/auth/stream assertions. Other dashboard/Owner tests remain
read-only. Do not retain blocking product SSR merely to preserve a synchronous test.
If installed Next makes this seam infeasible, return a concrete proposal instead
of adding writable files silently.

WP-5 owns E2E/harness/runbook/diagrams from the plan; WP-6 owns review only. Two
structure diagrams+PNGs must be updated once actual edges exist; WP-1 creates no
runtime edges and leaves them untouched.

### Downstream checks (specified, NOT executed by WP-1)

WP-2:
```sh
python3 -m unittest discover -s scripts/ops -p 'test*service_status*.py'
bash -n scripts/ops/compose-release.sh scripts/ops/validate-production-config.sh
bash scripts/ops/production-ops-static-check.sh
bash scripts/ops/production-ops-self-test.sh
bash scripts/ops/kis-daily-production-self-test.sh
bash scripts/ops/kis-read-compose-self-test.sh
```
Use fake commands only for root/systemd/Docker tests; temporary directories owned
by test UID substitute for root only inside explicit test seams. Required FS cases:
symlink/hardlink/permissions/oversize/partial write, restart/reboot, temp rename reader
sees new inode, old unit/short-prefix collision, timer disabled/missing/next unset,
unit condition skipped, exit 74/75, incomplete deadline, secret sentinels suppressed.

WP-3 isolated DB (existing source: `deploy/qa/qa-db.compose.yml:24`; harness embeds
all migrations at `crates/api-server/tests/common/mod.rs:83`, creates disposable DB
at :203 and **silently skips without DATABASE_URL** at :163). Use an unused dedicated
loopback port and unique project; no production compose/project/env. Example exact
commands after confirming ownership/port/image presence (do not pull implicitly):
```sh
export LAGRANGE_QA_DB_PORT=55439
docker compose -p lagrange-service-status-wp3 -f deploy/qa/qa-db.compose.yml up -d --pull never --wait qa-db
export DATABASE_URL=postgres://postgres:lagrange@127.0.0.1:55439/postgres
: "${DATABASE_URL:?isolated DB required}"
# WP-3A: migrations, producer writes/fences and safe SQL reads.
CARGO_BUILD_JOBS=2 cargo test -p api-server --test service_status_reads -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p collectors --test service_status -- --test-threads=1
# WP-3B: API integration, authorization/audit and wire contract.
CARGO_BUILD_JOBS=2 cargo test -p api-server --test service_status -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p api-server --test http_admin -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p api-server --test openapi_contract
npm run openapi:check --workspace=@lagrange/api-server
docker compose -p lagrange-service-status-wp3 -f deploy/qa/qa-db.compose.yml down -v
unset DATABASE_URL LAGRANGE_QA_DB_PORT
```
Use trap cleanup in execution harness; never down a pre-existing project. New DB
tests must call Harness::new().await.expect("isolated DATABASE_URL required") and
report executed/skipped counts. Test grants as app/admin/worker/research_writer,
forged actor/session, Member, second Owner, another user's demand/jobs, private
producer SELECT denial, lack of DML, function-only safe read, exact window/calendar
lineage, all fixed fixture boundaries, last-success fencing, query timeouts and
source partial failures. Test in READ ONLY transaction to prove function has no
hidden locks/writes. Auth/audit failure must return no data. No live provider.
WP-3A also runs the §G2 write-failure cases, including the undetectable unrecorded
attempt with a prior terminal success; tests must not require impossible detection.
Each sequential package runs only its labeled commands with isolated setup/cleanup;
WP-3B proceeds after coordinator acceptance of WP-3A.

WP-4 (after reading locally installed Next guides):
```sh
npm test --workspace=@lagrange/web -- service-status
npm test --workspace=@lagrange/web -- owner-beta-product-route stock-beta-dashboard
npm run typecheck --workspace=@lagrange/web
npm run lint --workspace=@lagrange/web
```
WP-5 runs full build/browser/diagram checks from the plan, sequentially with Rust
work. WP-1 did not run these commands, start servers/containers or allocate ports.

## 9. Fixture and acceptance protocol

`tests/fixtures/service-status/base-input.json` is a synthetic safe host snapshot
plus DB/window read inputs at a fixed clock. `base-output.json` is the exact API
oracle. `cases.json` uses JSON Pointer **replace/add/remove** patches over independent
copies of those two files: `input_patch` produces raw input, `expected_patch` produces
expected output. Paths use exact array indices from catalog order. This is a compact
lossless fixture encoding, not a status reducer. Consumers must materialize both
sides then run their implementation only on the input. Never regenerate expected
statuses from the implementation being tested. `wire-cases.json` separately contains
host parsing/FS/authorization faults and exact reject/HTTP outcomes. `README.md`
explains domains, raw normalization boundary and validator commands.

The bounded WP-1 validator checks JSON parse/duplicate keys, Draft 2020-12 schema and formats,
full host/API/catalog shapes, fixed inventories, line references, expected enum
coverage, explicit time-boundary case IDs and migration inventory integrity.
Default mode permits complete contiguous approved pairs through 0058/0059;
`--baseline-reservations` requires both still free at 0057. Both reject malformed,
unpaired, duplicate or colliding revisions; neither executes SQL. It checks
fixture structure and internal oracle invariants; it is **not execution of the
future Rust/Python status implementation**, not SQL permission proof, and not a
claim of production detection latency. Downstream tests must prove those separately.

Coordinator acceptance must cover all 22 rows, role denial preservation, no page
lease demand side effect, independent ETF/V2/window/calendar semantics, source clock
non-fabrication, disabled/missing/partial distinctions, exact file map, reservations,
required instrumentation and per-request audited polling. Source drift, missing
mandatory implementation or a skipped DB test blocks downstream acceptance.
