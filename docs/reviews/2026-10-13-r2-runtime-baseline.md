# R2 WP-1A — current runtime baseline and bounded DB probe preparation

Observation window: **2026-10-13 13:48:18–13:52:45 UTC (22:48:18–22:52:45 KST)**. The root check returned UID 0. This is a point-in-time read-only host observation, not an activation or an R2 acceptance.

## Current installation and policy

| Evidence | Observed | Interpretation boundary |
|---|---|---|
| `/opt/lagrange/current` | Resolves to `/opt/lagrange/releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c`; root-owned release directory, mode 0755. The symlink is root-owned mode 0777. | Exact current release revision is `fce9b90542cf9b1a911d46e4c4d3530db7efba4c`. This is not C3 `862fa954…`. |
| Image manifest | `/etc/lagrange/release-manifests/fce9b90542cf9b1a911d46e4c4d3530db7efba4c.manifest`; root:root 0600, 2,597 bytes, 12 canonical image records, revision matches release. SHA-256 `0d8071068f1ed0c215fb5387cc4c3353e8367a6987371c4a9f5ddabd0fe3eaf6`. | Shape and all 12 records were parsed internally against `scripts/ops/lib/release-image-manifest.sh`; no bytes were emitted. |
| Installed deployment policy | `/var/lib/lagrange/service-status/policy.json`; root:root 0600, 471 bytes, SHA-256 `7c87ef7b1708bc6e7b44c071c6d94148f44bfdd9dc78d4b0e47cd01eec0e3446`. Validated policy reports `research_mode=daily`, `profile=owner_daily`, and release commit equal to current. | The fixed safe fields were parsed without printing policy contents. This is the authoritative installed policy path per `docs/contracts/service-status-v1.md` §3. The file owner/mode/schema were checked; full trusted-parent-chain validation was not run. |
| Protected Compose env | `/opt/lagrange/deploy/compose/.env`; root:root 0600, 7,145 bytes, SHA-256 `7c77e9f57b6067bb4592967ad61586772408e8417028f650e07467f159288313`. Validated fixed fields: `LAGRANGE_CODE_COMMIT=fce9b90542cf9b1a911d46e4c4d3530db7efba4c`, `OWNER_BETA_ACCESS_MODE=owner_only`, `OWNER_BETA_PAPER_MODE=disabled`, `OWNER_EQUITY_V2_RUNTIME_MODE=owner_only`, `OWNER_INTRADAY_QUOTES_MODE=owner_only`, `OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws`, `OWNER_INTRADAY_SESSION_WINDOWS_SOURCE=operational_v1`, `KIS_READ_COORDINATION_MODE=shared_required`. | Parsed internally; no environment was sourced or dumped. Values were validated against source allowlists. |

## Daily unit and installed services

Six commit-suffixed KIS daily service/timer pairs are installed: `66b2a8c`, `9b4a175`, `aa84739`, `b08ea38`, `cbb7357`, and current `fce9b90`. All six services report `UnitFileState=disabled`; all six timers are `inactive/dead`, disabled, and have empty `LastTriggerUSec` and `NextElapseUSecRealtime`. Of the six services, five are inactive/dead with empty start/exit timestamps; their default `Result=success` and `ExecMainStatus=0` are not historical success evidence.

The most recent recorded service attempt is `lagrange-kis-daily-66b2a8c.service`: loaded, failed, `Result=exit-code`, `ExecMainStatus=1`, `MainPID=0`, started 2026-09-07 16:30:07 KST and exited 16:30:09 KST. The current-release service `lagrange-kis-daily-fce9b90.service` is loaded, inactive/dead, disabled, `MainPID=0`, with empty start/exit timestamps. I did not inspect logs and do not infer the failure cause. The unit list does not establish that these six names are the complete history of transient units.

The service-status observer, production backup, backup verification, and Tailscale TLS renewal timers were active/waiting. No calendar/window timer appeared in the filtered unit-file or timer inventory. Three transient calendar QA services appeared in the filtered inventory: `lagrange-calendar-core-qa-20260914.service`, `lagrange-calendar-core-qa-20260914-r2.service`, and `lagrange-calendar-core-qa-20260914-r3.service`; none was listed as a calendar timer.

## Container and resource snapshot

Docker inspection used only fixed identity, Compose labels, OCI revision, state, health, timestamps, and restart count. Eleven `lagrange-station` containers were present. The full fixed-field inventory is in private `target/r2-daily-beta/wp-1/host-evidence.json`.

- Owner V2, web, API, reverse proxy, both backtest workers, recommendation, candidate, and owner-beta containers were running and healthy with exit code 0 and no OOM kill.
- Owner V2 is on the current `fce9b905…` revision, started `2026-10-13T07:10:57.548664871Z`, restart count 0. Web/API also report current `fce9b905…`.
- The reverse proxy reports revision `61141700…`; research-worker, both backtest workers, recommendation, candidate, and owner-beta report `74b39fa7…`. Research-worker is exited 2/unhealthy, finished `2026-09-19T03:45:12.078101835Z`, restart count 7,935, OOM false. No cause is inferred.
- Eleven extant containers does not prove the full Compose service inventory; for example, this observation does not assert a container for every manifest image.

Host sample: MemAvailable 3,964,884 KiB; SwapFree 44 KiB; memory PSI `some/full avg10=0.00`; root filesystem 56% used (same root filesystem for the inspected `/opt/lagrange` and `/var/lib/lagrange` paths); no `cargo`, `rustc`, `rustdoc`, `buildkitd`, or `buildx` process was found by `comm`/PID scan. The selected KIS daily lock is root:root mode 0600, empty, mtime 2026-08-23 03:04:23 KST.

## Calendar and local daily state

The selected calendar files validated as root-owned mode 0600 and their bytes/hash agree; parent-directory chain trust was not independently revalidated. `calendar.json` matches the manifest SHA-256 `sha256:b4caff423ea76c1af1fa8a482da517677d281385c5cf0c17f335b5a7d74e40c5`; manifest file SHA-256 is `06fb19f1664c672eb824bb5ee0adce82dc27e4b2be36da820656ae1003896bb8`. Safe metadata: calendar ID `xkrx-historical-session-dates`, range 2020-01-31 through 2027-12-31, 1,945 sessions and 947 non-sessions. These artifact bounds do not prove current DB publication or an official current-day calendar observation.

There are 16 root-owned mode-0600 daily state TSVs, dated 2026-08-23 through 2026-09-07, all structurally valid under `kis-daily-state.py`'s V4 format. Their 162 rows contain 116 `PUBLISHED`, 9 `FAILED`, and 37 `RUNNING` entries. The latest state filename and latest recorded row date are both 2026-09-07. These are local state outcomes only, not verified DB publication, ETF coverage, or last successful run. No state file for the observation date was found.

## Database probe preparation — not run

`target/r2-daily-beta/wp-1/db-probe.sql` contains an explicit `REPEATABLE READ READ ONLY` transaction, verifies `transaction_read_only`, `default_transaction_read_only`, isolation, 10-second statement timeout, 1-second lock timeout, UTC timezone, and a superuser/`BYPASSRLS` operator-visibility guard. Its sole success SELECT reports guard settings; the actual data query body is intentionally absent. `db-probe-runner-draft.sh` exits 64 before any command and only documents the proposed future local Unix-socket transport through a freshly verified, already-running `lagrange-station/postgres` container. Draft SHA-256: SQL `b3bcdea6f79bc06d052e855d033868bb48896ec70231c64201df1405651ee9d6`; runner `1e2102504febc6216cbda329dfdb3990c7b150e8ff64e1011686e379034fd8eb`.

The requested ETF publication/session/11-instrument coverage, recommendation successful run and `as_of` lineage, Owner membership/READY/generation/signal coverage, and migration version/checksum/failed-index/0050 precondition aggregates remain unverified. Mapping the exact SQL and bounded predicates requires schema and migration analysis beyond mechanical preparation; WP-1A leaves it for coordinator/WP-2 review. No database SQL was executed.

## Official check command effects for coordinator review

- Candidate provider-free inspection: current release `scripts/ops/kis-daily-production.sh --plan`. Source describes no DB, Docker, worker, or provider call; it validates protected dotenv/calendar inputs, emits the selected calendar range to temporary files, then removes those files. This command was not run.
- Candidate exact unit-contract check: current release `scripts/ops/install-kis-daily.sh --check --release-root /opt/lagrange/releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c --code-commit fce9b90542cf9b1a911d46e4c4d3530db7efba4c`. Source path is a comparison of generated service/timer bytes and trusted installed files; do not treat it as an activation. This command was not run.
- Do **not** run `kis-daily-production.sh --check` as part of WP-1A. `scripts/ops/lib/db.sh:30-65` implements `db_psql` with `docker compose run --rm --no-deps ... db-migrate`; that creates a one-shot container even though the SQL intent is read-only. This phase excludes new containers and the request also expressly forbids DB execution.

## Required report

- **Files:** this report and private `target/r2-daily-beta/wp-1/{README.md,host-evidence.json,db-probe.sql,db-probe-runner-draft.sh,binding-comparison.json}`. Only this report is tracked. Private directory is mode 0700; all five private files are mode 0600.
- **Deviations and reasons:** no SQL/schema body was prepared because exact SQL mapping exceeds mechanical preparation and coordinator owns scope; runner is an inert exit-64 draft. This follows the WP-1A boundary.
- **Checks/commands and results:** read required guidance and assigned source documents; `sudo -n id -u` returned 0; read-only `readlink`, `stat`, SHA-256, filtered systemd, selected-field Docker inspection, protected metadata parsing, `/proc`, `df`, `ps comm`, and strict calendar/state metadata parsing completed. The manifest/policy/calendar/state structures validated. No tests or builds were run.
- **Unresolved/follow-up:** coordinator must review/authorize a separately bounded SQL query, verify exact production schemas and operator visibility, and decide how to inspect current DB publication and last success. Root-cause evidence for the failed daily unit is not available from this assignment's allowed fields.
- **Not found or not verified:** no official-current session calendar/provider receipt; no current DB aggregates or migration/index state; no daily `--plan`/installer `--check` execution; no full transient unit history or Compose inventory; no cause for either failed runtime. No required source file was missing.
- **Background work/permission waits:** none running; no permission request pending. No lifecycle action, DB query, provider call, build, test, or new container was started.

## WP-1A bounded follow-up — installed input binding and image identity

Fresh observation: **2026-10-13 14:00:42 UTC (23:00:42 KST)**. Receipt: private `target/r2-daily-beta/wp-1/binding-comparison.json`, SHA-256 `6f0da4cb141a25dce5eae73a5ebc24ce9ed5c691d22e0a05eb7753dedf22b8b7` (mode 0600). It contains the allowlisted path metadata, unit fields, and full image comparison values.

### Installed daily unit input binding

The current link is the intentional root-owned `/opt/lagrange/current` symlink to `releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c`. The exact installed `lagrange-kis-daily-fce9b90.service` and `.timer` files are root-owned, non-symlink files with trusted parent chains; `systemctl show` reports both loaded from those exact fragment paths. The service's allowlisted fields validate as:

- `ExecStart` executable `/opt/lagrange/releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c/scripts/ops/kis-daily-production.sh`, mode `--execute`; `WorkingDirectory` is that physical release root.
- `LAGRANGE_CODE_COMMIT=fce9b90542cf9b1a911d46e4c4d3530db7efba4c`.
- `LAGRANGE_ENV_FILE=/opt/lagrange/releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c/deploy/compose/.env`.
- `LAGRANGE_XKRX_CALENDAR_DIR` and `KIS_DAILY_CALENDAR_DIR` are `/var/lib/lagrange/state/kis-daily/calendar`; `KIS_DAILY_STATE_DIR=/var/lib/lagrange/state/backfill`; `KIS_DAILY_LOCK_FILE=/var/lib/lagrange/state/backfill/kis-daily.lock`.
- Timer binds to that exact service, with `OnCalendar=*-*-* 16:30:00 Asia/Seoul` and `Persistent=true`.

The earlier report's `/opt/lagrange/deploy/compose/.env` path was the global alias, not the unit's literal environment path. The alias `/opt/lagrange/deploy -> current/deploy` resolves through the intentional current link to the physical release-local `.env`. The alias-resolved and release-local files both validate root:root 0600 and have the same SHA-256 `7c77e9f57b6067bb4592967ad61586772408e8417028f650e07467f159288313`. The resolved physical release path has no symlink components. The policy file and current manifest also have root-owned, non-writable, non-symlink parent chains; `/etc/lagrange` has group `982` but is mode 0750 and not group/other writable. No malformed allowlisted unit value or untrusted protected path was found.

### Current manifest versus running image objects

The exact current manifest passed the strict 14-line V2 grammar and canonical service-order checks: 12 records, commit `fce9b90542cf9b1a911d46e4c4d3530db7efba4c`, SHA-256 unchanged at `0d8071068f1ed0c215fb5387cc4c3353e8367a6987371c4a9f5ddabd0fe3eaf6`. For each present manifest service, the comparison uses the container's `.Image` ID and the OCI revision read from that image object (not the container revision label).

| Running service | Result | Expected image ID | Actual image ID | Expected / actual image OCI revision |
|---|---|---|---|---|
| `owner-equity-v2-runner` | MATCH | `sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50` | same | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / same |
| `web` | MATCH | `sha256:4f85b1f929b7f4952b992d5e62b9ab7eaf4a0da4cf053eb193ec8388aee1e0c7` | same | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / same |
| `api-server` | MATCH | `sha256:14ab00a754b03ba086e108fddac8f7df4d18752209ef706375cfdd75a6446e87` | same | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / same |
| `nt-backtest-worker-2` | MISMATCH | `sha256:d2b7d3b60531fb55d968fba807eb48e5a54eab8ade2a8785d6d9726513274969` | `sha256:510c1fb19c7118cf73792566445c25d89e017dd4d9c84336c0146845ea9989e5` | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / `74b39fa7b1342f4e6e069369da9f0f9017a74aa8` |
| `nt-backtest-worker-1` | MISMATCH | `sha256:11519f9fd948651fc7c442d8d4d090a92ac17f4dde6c2a0bcbd23b03437d4f0d` | `sha256:a973019bf8174ccf880ac2c85b05990bfea7a1eaa511d63d5b316cc11efcabdf` | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / `74b39fa7b1342f4e6e069369da9f0f9017a74aa8` |
| `recommendation-runner` | MISMATCH | `sha256:c61e4a6c8caf38f894585e6096da9e1011911107584921866da756e1bc497ae2` | `sha256:45bf776dd140009a9a38c53969966cef031587f61e7d840c81d9e6c52ae84884` | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / `74b39fa7b1342f4e6e069369da9f0f9017a74aa8` |
| `candidate-runner` | MISMATCH | `sha256:c2a8fe71107566c67272c2071ed5e52d9aadc534e68d53213ede9f5f2e1c36df` | `sha256:ebb6bf1e2a0663c4187df1c64457a96daad4a5cde90f2d7f2e98ff3152c7c686` | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / `74b39fa7b1342f4e6e069369da9f0f9017a74aa8` |
| `owner-beta-runner` | MISMATCH | `sha256:871a06bfac513dd91162a32c1c3819632dbb6424fc192b872dd7e951438e54e6` | `sha256:a1aa7ef463521449fd87829552fc38d32cacc205041fdf00313095e8e53c3dff` | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` / `74b39fa7b1342f4e6e069369da9f0f9017a74aa8` |
| `reverse-proxy` | NOT_APPLICABLE | outside 12-service manifest | `sha256:7377697a821c131a924a7105fafbe7414db4e9fcc77a6f08f776f33f141ec3f8` | outside manifest |
| `postgres` | NOT_APPLICABLE | outside 12-service manifest | `sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a` | outside manifest |

No running containers were present for manifest services `db-role-bootstrap`, `db-migrate`, `research-worker`, and `paper-scheduler`. This comparison establishes installed identity only; no mismatch cause is inferred and no deployment permission is implied.

### Follow-up report

- **Files:** appended this bounded delta to this report; added private `target/r2-daily-beta/wp-1/binding-comparison.json` (1 JSON line, mode 0600). The existing SQL draft and prior receipts are unchanged.
- **Deviations:** none. The previous untrusted-path result came from the first follow-up script requiring parent directory GID 0; the strict manifest source requires root ownership and no group/other write bits. The corrected check uses that source contract and records `/etc/lagrange`'s non-writable group-owned parent. The failed script emitted no host metadata and no host state was changed.
- **Checks/commands and results:** fresh `sudo -n id -u` returned 0; `readlink`/`lstat`/`stat`/hash were limited to the fixed paths; exact systemd `LoadState`/`FragmentPath` properties and allowlisted installed-file fields validated; the strict manifest parser passed; running-container image IDs and OCI revisions were inspected with fixed Docker formats. Three manifest services matched, five mismatched, and two non-manifest services were not applicable. Receipt JSON is one line, mode 0600, SHA-256 recorded above. No tests, builds, SQL, provider calls, or lifecycle actions were run.
- **Unresolved/follow-up:** investigate the five identity mismatches only under a separately authorized scope; no explanation is asserted here. Current DB freshness, publication, SQL coverage, and the earlier daily service failure cause remain pending coordinator review.
- **Not found/not verified:** no current DB facts, no cause for image mismatches or failed daily execution, and no service mapping for the four manifest entries without a running container. No trusted-path failure was found for the policy, physical manifest, or effective release-local env.
- **Background work/permissions:** none running and no permission wait pending. No further decomposition is required for these fixed inspections.

## Coordinator historical diagnosis cross-check

The separate [September 7 incident record](../runbooks/kis-daily-stale-release-20260907.md)
records an Owner-executed disable of the obsolete `66b2a8c` daily timer after its
failed 16:30 run. Its old release admitted corporate-action-only Raw evidence
into EOD normalization and failed before the new pending date was collected.
The existing `db7de37` recovery fix excludes only nonempty all-action evidence;
coordinator verified that commit is an ancestor of the R2 source baseline and
that the exclusion remains in `data-pipelines/collectors/src/pipeline.rs`.
This is preserved historical diagnosis, not fresh reproduction or proof of the
research daemon's separate exited2 cause. The current `fce9b90` disabled timer
was freshly observed above; the incident does not establish that newer unit's
installation/activation history. Current DB publication and exact missing dates
still require the reviewed aggregate probe. No Raw/state deletion or retry was run.
