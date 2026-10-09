# Owner service-status runbook

**Status:** implementation and required isolated QA completed. The final independent verdict is recorded in [the feature review](../reviews/2026-10-08-service-status-review.md). Not installed or activated.
The code and disposable QA fixtures do not establish production service health,
policy, or entitlement. Current results and outstanding gates are recorded in
[the QA review](../reviews/2026-10-08-service-status-qa.md). A status GET does not
re-enable or start any collection schedule.

Recheck this runbook against the accepted API/G1 source, final release manifest,
and current immutable release before an operator uses it. The supported installer
surface is implemented in `scripts/ops/install-service-status.py`; the collector
and policy are `scripts/ops/collect-service-status.py` and
`scripts/ops/lib/service-status-policy.py`.

## What the panel reports

The Owner panel has one collapsed summary and 22 fixed rows. Expanding it does not
create work or a live-quote subscription. `process_status` and `work_status` are
separate observations: a process can be running while its queue is idle, and a
daily executor can be inactive between scheduled runs while the daily data task
is current.

| State | Meaning |
|---|---|
| `healthy` | Required evidence is current and the expected work/data is current. |
| `waiting` | An intentional idle, scheduled, closed-market, or no-demand state. |
| `degraded` | Evidence exists, but work or data is partial, stale, limited, or recently failed. |
| `failed` | A required process, schedule, work prerequisite, or policy check failed. |
| `unknown` | Required evidence is absent, expired, invalid, future-dated, or unavailable. Missing evidence never means healthy; diagnosed stream clocks have the exception below. |
| `disabled` | The feature is disabled by a verified policy. Unexpected activation of a disabled feature is a policy failure. |

The overall status excludes `research-worker`, which explains how `kis-eod` is
executed and is not a second independent work item. The two backtest rows read one
shared actor queue; their counts are not added together. A capability such as the
calendar has no independent process to check.

### The 22 rows

| # | Row | What it observes and what it does not claim |
|---:|---|---|
| 1 | `postgres` | Container health / `pg_isready`; no SQL or data-freshness claim. |
| 2 | `reverse-proxy` | nginx `/healthz`; not browser reachability. |
| 3 | `web` | Web `/healthz`; not successful chart loading. |
| 4 | `api-server` | `/readyz`, including DB and audit readiness; distinct from liveness. |
| 5 | `owner-equity-v2-runner` | Docker heartbeat plus actor queue and current generation/snapshot coverage; excludes WebSocket health. |
| 6 | `owner-beta-runner` | Actor beta jobs and successful recommendation runs; request-driven work may be idle. |
| 7 | `candidate-runner` | Scheduler/control/feed state and actor queue; no user payload. |
| 8 | `recommendation-runner` | Actor queue and successful runs; an empty queue is valid waiting. |
| 9 | `nt-backtest-worker-1` | Process readiness and shared actor backtest queue. |
| 10 | `nt-backtest-worker-2` | Same shared queue as worker 1; never double-count its queue. |
| 11 | `research-worker` | Executor mode/state for `kis-eod`; explanatory row excluded from the overall denominator. |
| 12 | `kis-eod` | Exact commit-bound daily timer **or** research daemon, plus separate Raw frontier and ETF11 Curated coverage. |
| 13 | `market-stream` | Active Owner demand, producer, subscription and quote receipt lineage; independent of EOD. |
| 14 | `intraday-calendar` | Today's trading calendar and operational window, checked independently. |
| 15 | `production-backup` | Fixed timer and last completed command outcome; existing backup verification semantics remain authoritative. |
| 16 | `production-backup-verify` | Fixed timer and last isolated restore-check outcome. |
| 17 | `tls-renewal` | Renewal command result, including a validated no-op; not a separate certificate-expiry probe. |
| 18 | `docker` | `docker.service` ActiveState/SubState only. |
| 19 | `tailscaled` | `tailscaled.service` state only; no peer, node, or address details. |
| 20 | `paper-scheduler` | Must remain disabled under Owner production policy. |
| 21 | `metrics` | Must remain disabled under Owner production policy. |
| 22 | `live-node-owner` | Must remain disabled; no live, account, or order path is authorized. |

### Missing evidence and persisted attempts

If the host observer or snapshot is missing, stale, malformed, future-dated,
partially unreadable, or rejected by ownership/mode checks, affected host evidence
is `unknown`. A source timeout is also unknown. Database timeout/limit/schema
failure yields unknown for each row that depends on that read. Calendar and
operational-window evidence can fail independently, including when there is no
active quote demand or the market is closed.

The private collector history is one latest record per fixed scheduled task, not
an event history. `RUN_HISTORY_EMPTY` is unknown, never success. The timestamp is
the last persisted attempt/result; telemetry cannot recover an attempt whose
record was never written. A failed attempt does not erase a verified earlier
success. Daily process exit codes 74 and 75 are not proof of data publication;
when publication is not independently established, the result remains unknown.
Operations whose last success predates installation cannot have that time
reconstructed safely.

The API rejects an invalid host snapshot instead of relaxing the reader. The UI
also expires a displayed reading at its evidence deadline without waiting for a
successful later poll. Wall-clock changes do not extend a healthy badge.

For the market-stream row, an exposed future receipt, provider-event or producer
state-change clock is preserved as a raw diagnostic with `SOURCE_FUTURE` on a
degraded or independently failed row. Failure keeps precedence; valid partial
coverage and its original bounds are preserved. A future minimum provider clock
cannot support nonzero live/halted coverage; expired counts can be null. These
diagnostic clocks are not positive freshness evidence, and they do not invalidate
all 22 rows or force unrelated evidence to unknown.

## Read and security boundaries

The endpoint is Owner-only: `GET /api/v1/admin/service-status`. Every admitted
request receives a bounded audit record, including a cached response. The read
does not query a provider, enqueue work, run a collector, create a market-stream
lease, start a service, or change a timer. Member/anonymous pages do not render
the Owner panel.

The root observer uses fixed Docker labels and fixed systemd units/properties. It
does not read container environment, commands, mounts, health logs, journals,
provider responses, broker state, or systemd environment/ExecStart. The API gets
no Docker socket, systemd bus, sudo, or writable host path. A GET is not an
external availability check; the panel shares the web/API/host failure domain.

For safe host-field inspection, use only the fixed properties collected by the
observer, for example:

```sh
systemctl show lagrange-service-status.timer \
  -p LoadState -p ActiveState -p SubState -p UnitFileState \
  -p LastTriggerUSec -p NextElapseUSecRealtime
systemctl show lagrange-service-status.service \
  -p LoadState -p ActiveState -p SubState -p Result \
  -p ExecMainCode -p ExecMainStatus -p ExecMainStartTimestamp -p ExecMainExitTimestamp
stat -c '%U:%G %a %F %n' \
  /var/lib/lagrange/service-status/public \
  /var/lib/lagrange/service-status/public/snapshot.json
jq '{schema_version, generated_at,
     policy: (.policy | {profile, release_commit, compose_project, owner_access,
                        quote_transport, research_mode}),
     observations: [.observations[] |
       {id, result, observed_at,
        process: (.process | if . == null then null else {state, health} end),
        timer: (if .timer == null then null else
          (.timer | {active, enabled, last_trigger_at, next_run_at, unit_binding, last_result})
        end)}]}' \
  /var/lib/lagrange/service-status/public/snapshot.json
```

Do not dump the private receipt/history, full container inspection, Docker or
systemd environment/command fields, database rows, raw logs, or API request/body
debug output. Use the installer’s `--check` for receipt/release integrity rather
than reading protected files. Keep the existing entitlement reference/hash and
the owner's confirmed rights boundary; do not expose Raw or Curated data beyond
those rights.

## Receipt, release, mode, and mount invariants

The installer binds the private root-owned receipt and history to the exact full
40-character release commit, manifest, Compose project and selected research
mode. The default release link is not a substitute for the immutable full-commit
path when planning or checking a release. The selected daily unit is bound to
that commit; an older timer is never a fallback.

The public snapshot directory is `/var/lib/lagrange/service-status/public`,
root:`10001`, mode `0750`. `snapshot.json` is a root:`10001` regular file, mode
`0640`, with one link. The API process is UID/GID `10001:10001`. If the optional
Compose overlay is included, it mounts the **directory** read-only at
`/run/lagrange/service-status` with `create_host_path: false`. Atomic rename
requires a directory mount; do not replace it with a single-file bind. No host
`/run` or `/var/lib` path is written during QA.

Keep `OWNER_INTRADAY_QUOTES_MODE` and `OWNER_INTRADAY_QUOTE_TRANSPORT` aligned
with the verified release policy. The supported UI transport is `off` or
`market_ws`; this status feature does not add a REST quote fallback. The current
Owner policy uses `market_ws`. Do not enable the live, account, or order profile.

`owner_daily` means the bounded daily EOD one-shot and a normally stopped
research executor between runs. `owner_daemon` means the work-driven research
daemon. Do not run the daily timer and daemon together. Existing daily-vs-daemon
conflict gates remain required when a later release starts research work. Installing
the status observer does not install or enable the KIS daily schedule.

## Supported installer commands

Choose `STATUS_RELEASE` as the existing immutable release directory whose name
is the full commit. Choose `STATUS_PROJECT` from the verified Compose deployment.
Use `daily` only when that is the verified deployment policy; use `daemon` only
for an authorized daemon deployment. These examples are documentation only and
were not executed during WP-5 QA. In particular, the sudo examples
below describe supported installer flags; they are not authorization to install,
activate, or inspect production state.

```sh
STATUS_RELEASE='/opt/lagrange/releases/<FULL_COMMIT>'
STATUS_PROJECT='<VERIFIED_COMPOSE_PROJECT>'

python3 "$STATUS_RELEASE/scripts/ops/install-service-status.py" \
  --plan --install-root /opt/lagrange --release-root "$STATUS_RELEASE" \
  --compose-project "$STATUS_PROJECT" --research-mode daily

sudo python3 "$STATUS_RELEASE/scripts/ops/install-service-status.py" \
  --preflight --install-root /opt/lagrange --release-root "$STATUS_RELEASE" \
  --compose-project "$STATUS_PROJECT" --research-mode daily

sudo python3 "$STATUS_RELEASE/scripts/ops/install-service-status.py" \
  --check --install-root /opt/lagrange --release-root "$STATUS_RELEASE" \
  --compose-project "$STATUS_PROJECT" --research-mode daily

sudo python3 "$STATUS_RELEASE/scripts/ops/install-service-status.py" \
  --apply --install-root /opt/lagrange --release-root "$STATUS_RELEASE" \
  --compose-project "$STATUS_PROJECT" --research-mode daily
```

`--plan` (also the no-flag default) prints a preview and returns before
filesystem preflight, Docker, systemd, or provider operations; it is not a
validation result. `--preflight` performs read-only checks. `--check` also
compares the installed receipt, history, unit bytes, and selected immutable
release. They require root in real mode because they inspect protected state.
`--apply` is the distinct activation command: it writes the observer receipt,
history layout, public directory and fixed units, then enables/starts only the
status observer timer. These commands must be revalidated and applied only by an
authorized operator against the final accepted release.

The 30-second observer timer is not a data-collection timer. A successful status
GET or observer install does not enable or run the KIS timer, backfill data,
restart the application, create market demand, or make live trading available.

## Recovery and removal boundaries

There is **no `--uninstall` or `--rollback` CLI**. Do not invent one or run a
blanket Compose/systemd cleanup. A separately authorized observer-only manual
removal must first identify the exact receipt-owned service/timer and activation
state, stop/disable only `lagrange-service-status.timer`, verify its service is
inactive, and remove/restore only verified observer-owned artifacts. Preserve
the public directory while any running API still mounts it. Detach the optional
mount through an authorized normal API deployment before removing that directory.
Never broadly remove `/var/lib/lagrange`, stop another timer, re-enable a
collector, or change `market_ws` during observer removal.

An update or rollback must use a matching release commit, policy, immutable image
manifest and Compose project. Restoring an old receipt under a new release is
not a valid rollback. A failed `--apply` attempts to restore only prior observer
files and this observer's enabled/active state; `ROLLBACK_INCOMPLETE` means
recovery is unresolved, not successful. Preserve the typed installer result and
perform bounded manual reconciliation of only the observer. Do not dump the
private receipt/history or free-form systemd/Docker/provider text.

## Release resource policy

For later production release work on the 14 GiB host, split the image list into
logical batches of two or three services and finish/verify one batch before the
next. A batch is only a reporting unit: invoke Compose for exactly one service
at a time and never place two Rust-producing services in one Buildx graph. Keep
`COMPOSE_PARALLEL_LIMIT=1` and `CARGO_BUILD_JOBS=2`. Run long builds in a
background systemd service with lower CPU/I/O priority so a Paseo disconnect
does not terminate them and production remains responsive. Between batches,
check available memory and swap, kernel OOM events, build exit status and
production health; do not advance while a prior compiler/build is active. On an
OOM/control-plane termination, stop the build, restore affected services, then
resume the one-service policy. Once cached, validate each image with the official
image builder and write the immutable release manifest. Never retry an
all-images parallel build.

## Isolated QA commands and evidence boundary

Use these launchers only with the coordinator's sole heavy-slot handoff. They are
local QA commands, independent of the production installer examples above.
`SERVICE_STATUS_QA_ROOT` is a unique user-owned, nonsymlink
`/tmp/lagrange-service-status-wp5-*` directory, mode `0700`. Metadata, receipts,
trusted host fixtures and synthetic session handoffs stay there. New large
browser outputs and standalone copies use `SERVICE_STATUS_QA_ARTIFACT_ROOT`, a
normal owned directory under the exact ignored repository prefix
`test-results/service-status-qa/<unique-WP5-run-name>`. The guard rejects symlink
ancestors or an escape; `artifact-index.json` links the private root to disk.
Credentials do not belong in the artifact directory.

```sh
export SERVICE_STATUS_QA_ROOT='/tmp/lagrange-service-status-wp5-<UNIQUE>'
export SERVICE_STATUS_QA_ARTIFACT_ROOT="$PWD/test-results/service-status-qa/lagrange-service-status-wp5-<UNIQUE>"
export SERVICE_STATUS_QA_SLOT_CONFIRMED=yes
```

Create the private root with mode `0700` before running a launcher. A private
`slot.lock` serializes stages. If interrupted, reconcile its exact recorded
unit/process before clearing only that owned lock. Do not stop an unrelated
service or use port `33001`. Every preflight writes current `MemAvailable`, its
threshold and UTC time to `preflight.tsv`; failure stops that stage. Do not
bypass it, raise caps or alter host caches/swap.

| Stage | Memory ceiling | Required available RAM before adding it |
|---|---:|---:|
| Web production build or offline Rust example build | 1536 MiB | 1792 MiB |
| Web unit/type/lint or contract/OpenAPI check | 768 MiB | 1024 MiB |
| Synthetic API | 128 MiB | 1408 MiB initial synthetic-stack preflight |
| Prebuilt Next | 256 MiB, Node heap160 | 1280 MiB |
| Playwright and one Chromium worker | 768 MiB, Node heap320 | 1024 MiB |
| Actual Postgres | 512 MiB | 2048 MiB initial actual-stack preflight |
| Actual Rust API | 256 MiB | 1536 MiB |
| Local cached diagram renderer | 768 MiB, Java heap512 | 1024 MiB |

All user units use swap0, CPUWeight10, OOMPolicy kill and finite90-minute
limits; tiny probes and rendering have shorter inner deadlines. Docker memory
plus swap equals memory, so its swap allowance is zero. On this Docker runtime,
shares256 measured CPU weight35; lower shares48 measured the required weight10.
The dedicated actual launcher checks live memory/swap/CPU values rather than
assuming the mapping. Rust uses one Cargo job, debug0, incremental0, offline
resolution and `ionice -c 3`. No I/O-controller property is claimed.

Lifecycle receipts run in `ExecStartPre` and `ExecStopPost`. They verify live
caps, retain original `SERVICE_RESULT`, `EXIT_CODE` and `EXIT_STATUS`, UTC main
start/exit times, peak/current memory and OOM events before transient-unit GC.
External supervision records `systemd-run` exit separately. `memory.stat`
records numeric `anon`, `file`, `shmem`; shmem is a subset of file and must not
be added again. End counters are post-child-exit observations, not a breakdown
of the peak. A missing field is explicitly unavailable. Next may exit143 during
requested cleanup; this is separate from the browser's original result.

Builds and compile stages require all owned runtime/DB sets stopped. Each lane
builds production Next with its exact API rewrite and quote mode, then stages
`server.js`, `.next/static` and optional public assets on disk. The three URLs
and modes are distinct; a successful off build supplies the Web production
build gate, so no fourth generic build is needed. Node build/check/API heaps
are1024/512/80 MiB, below their respective unit caps.

### Web checks and complete off selection

```sh
bash scripts/qa/service-status-web-checks.sh typecheck
bash scripts/qa/service-status-web-checks.sh lint
bash scripts/qa/service-status-web-off.sh
```

The full unit command is
`npm test --workspace=@lagrange/web -- --maxWorkers=1`; reuse the accepted
61-file/483-test result while its three source hashes remain unchanged. E2E
changes still require full typecheck/lint. Those commands are
`npm run typecheck --workspace=@lagrange/web` and
`npm run lint --workspace=@lagrange/web`.

The off lane uses synthetic API/Next ports38192/33042, mode `off+market_ws`.
It collects the exact three files under Chromium with `--grep-invert @actual`:
`service-status.spec.ts`, `stock-beta.spec.ts`, `stock-beta-intraday.spec.ts`.
Current inventory is88:37 required status cases,39 required chart cases and
12 intentional legacy REST/off skips. Inventory is not a passing aggregate.
The dedicated helper validates unique keys, disjoint exact-title `--test-list`
files and the complete result union, rejecting missing/extra/duplicate/flaky or
unexpectedly skipped results. Each serial batch has at most8tests in a new
768 MiB process unit. Before the next batch, its unit, child PID and cgroup must
be disposed. Default traces, videos and failure screenshots remain unchanged;
each split source, locale and viewport also has a separate browser context.
A nonzero batch stops the lane and retains its original failure.

Expired conservative UI is checked separately from its frozen API diagnosis:
a null or expired deadline makes row, process and work all Unknown. An API
Not applicable work diagnosis does not keep expired UI evidence fresh.

### Separate market-stream test

```sh
bash scripts/qa/service-status-market-stream.sh
```

This uses ports38193/33043 and `owner_only+market_ws`. Its one required test
measures a page-owned in-process SSE server, with the real browser controller
and existing membership/lease/SSE parsers. It proves one lease and one SSE
connection through disclosure and repeated status polls, renewal and release,
working chart/current quote and no extra REST demands or provider requests.
It does not contact a provider or prove live market transport.

The scenario must seed a READY member of the real fixed observation universe
(`005930.KRX` here); generic synthetic `000001.KRX` is excluded by the product
controller. Delay only the first status response when measuring independence;
clear that dedicated fault before subsequent polls.
Use the real Dashboard Link to unmount the stock provider while keeping the
document alive. A full-document navigation can terminate its bounded asynchronous
release before any DELETE is sent; the controller deliberately leaves ambiguous
release to TTL. The required test still asserts exactly one release and SSE close.
The lane retains a JSON result and its measured counters alongside the line log.

After an E2E-only correction, `--reuse-prebuilt` may reuse the same task's
successful exact-mode build. It verifies the original build receipt, recorded
URL/mode, current accepted Web/CSS hashes and baked API rewrite. It still runs
all synthetic-runtime/Next/browser preflights and caps. A changed product hash
requires a new build.

### Actual isolated router chain

```sh
bash scripts/qa/service-status-rust-build.sh start
bash scripts/qa/service-status-rust-build.sh wait
bash scripts/qa/service-status-actual-api.sh
```

The compile command is only
`cargo build -p api-server --example service-status-browser-harness --offline`,
with one job/debug0/incremental0 and runtime/DB stopped. The dedicated launcher
uses `--expand-environment=no` for its shell PID writer; systemd must not turn
literal `$$` into a dollar character. Do not infer a failed Cargo compile from
a separate writer/supervision failure: retain both original receipts.

The actual lane has a separate URL-specific Next build, cached isolated
Postgres, real common Harness/Axum router, and trusted root-owned temporary
snapshot directory. Ports are55445/38194/33044. Its configured runtime sum is
1792 MiB plus the256 MiB initial headroom. The status endpoint is not intercepted.
The API stays the same process across two admitted Owner reads. Same-commit
atomic daily-to-daemon policy replacement changes the mounted directory entry
while preserving original source timestamps. The second read is spaced six
seconds after the first browser process exits; starting a new browser does not
bypass per-Owner admission. Audit evidence must include both reads.

The production reader path and root/mode/nlink checks remain unchanged. API
UID/GID is10001:10001; only the declared private `/tmp` host-snapshot and handoff
trees receive narrowly scoped native root permission. The API mounts the public
**directory** read-only at `/run/lagrange/service-status`. No production `/run`
or `/var/lib` path is written. If the cached image's loader is incompatible,
copy only the built example's resolved regular ELF loader/libraries into the
private `actual/loader-runtime` and mount it read-only. No broad host library
mount or image pull is permitted. This proves the router chain, not
production-main startup or an installed release.

On this host, general sudo requires interactive authentication. The disposable
publisher instead uses fixed filesystem operations in the same cached image,
with network none, a read-only container root,128 MiB/swap0/CPUWeight10 and a
30-second timeout per operation. Its only writable mount is the validated private
`actual` directory; root writes are limited to `host-snapshot` and `handoff`.
It verifies root:10001/0640/nlink1, changed inode, same commit and exact original
timestamps. The owning QA user copies session/audit into private mode0600 files;
the session is removed during cleanup. No production reader check is bypassed.

Readiness uses authenticated `/stock-beta` HTML and a real local static asset;
anonymous `/` redirects with the actual router. Playwright derives a cookie's
Secure flag from its setting URL, so the actual test uses an HTTPS setting URL
without sending a request to it, and asserts Secure/HttpOnly/path `/`. Both
policy phases retain JSON results and safe proof attachments. After a harness-only
repair, `--reuse-prebuilt` verifies the same successful actual build receipt,
accepted Web/CSS hashes, URL/mode and API rewrite before reusing its output.

### Visual, contract and diagram evidence

Each ko/en375x800,1024x900,1280x720,1440x900 capture records compact summary,
chart width, overflow, caption bounds and natural expanded scrolling. Desktop
and tablet scroll the shell's `main`; mobile scrolls the document. All22rows
must fit their viewport/clipping-ancestor bounds after scrolling, followed by
return-to-summary keyboard collapse. `-expanded-top.png` and `-table-end.png`
are exact viewport frames. A tall full-page image with blank space below the
fixed shell cannot prove reachability; retain it only as history. Current
375px captions measured355.40625px wide and31.5px high in both locales.
Keyboard summary focus and forced-colors frames are reviewed visually.
640x360 is a viewport equivalent for200% zoom; browser UI zoom was not set.
Chart selection/pan/zoom/page-scroll and no-JS loading assertions remain.

```sh
bash scripts/qa/service-status-contract-checks.sh fixtures
bash scripts/qa/service-status-contract-checks.sh temporal
bash scripts/qa/service-status-contract-checks.sh openapi
bash scripts/qa/service-status-diagrams.sh
```

The official OpenAPI check uses the already available exact locked7.13.0 tool,
regenerates/typechecks and requires identical protected output bytes. It removes
only its temporary ignored module link. Contract validators are schema/vector
checks, separate from DB/router behavior. Both diagrams carry current concrete
source citations and render from cached local PlantUML with `--pull=never` and
network none. The renderer records its final metrics inside the container before
GC and retains both PNGs with the PUML sources.

All old failures remain linked from the QA review, including predecessor
640/768 MiB OOMs and Sol's actual10:57:36Z OOM without final totals. The exact
old preflight line28 required1024 MiB, contrary to the stale896 claim. Tmpfs
occupancy and long traced contexts are hypotheses, not a demonstrated sole
cause. The coordinator archived exactly five closed leaves/349486031bytes to
ignored disk, verified bytes/modes/link targets and left historical symlinks;
this does not authorize further archival. Around12:59–13:01 UTC, the coordinator
also archived seven previously completed, actually idle chart/inventory/preflight
workers after recovering their reports and confirming no pending permissions.
Available memory rose from1590284 to3815544 KiB during that interval. Concurrent
memory changes prevent exact per-worker attribution; recovery was not solely
natural. No live app or unrelated process was stopped. See
[/tmp/lagrange-service-status-prior-worker-archive-receipt.json](/tmp/lagrange-service-status-prior-worker-archive-receipt.json).
Final5's pre-caption76pass/12skips and final6's snapshot failure remain history.
The accepted chart observer now reads React's committed tree with bounded
256-ancestor/5000-node traversal, exact server-seed object identity/context,
null errors and final publication; its single1M request must match its own200
response and24bars. React internals are a narrow test-maintenance dependency,
not a product seam; missing bindings fail closed. The earlier100-level bound
failed against measured107 ancestry and remains recorded. The original low-level
Chromium failure-event cause is unproved and nonblocking.

Current final off build `34dc40hAnti4DF1oVb0Ce` passed all88 unique cases:
76passed/0failed/12existing legacy REST-off skips, retry0,11 fresh browser batches.
Current typecheck/lint and protected integrity passed. Actual daily→daemon and
market-stream gates are accepted reused evidence, not rerun here. See the
[final QA review](../reviews/2026-10-08-service-status-qa.md) and
[/tmp/lagrange-service-status-wp5-final-off-cxduhv_8/web-off/aggregate.json](/tmp/lagrange-service-status-wp5-final-off-cxduhv_8/web-off/aggregate.json).
This is bounded Chromium viewport/emulation evidence, not a physical-device,
real browser UI200% zoom, production heap/load or production installation claim.
Overall feature acceptance requires the frozen independent review linked above.

The final host-observer review corrected the public snapshot writer to require
the installed `0750` parent directory while preserving file `0640` and the private
history directory's `0700` requirement. Raw Docker UTC timestamps may carry up to
nine fractional digits; the collector explicitly truncates them to the existing
microsecond wire format. The real writer function and Docker observation mapping
are covered by the final 41-test fake-host suite, with no failures or skips.
This evidence does not establish a production installation or provider connection.
Both diagram sources have updated collector citations and were rendered locally
from the cached image after these repairs; the QA review records the receipts.
