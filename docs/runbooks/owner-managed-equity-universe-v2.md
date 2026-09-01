# Owner-managed equity universe V2 runbook

Status: owner-only runtime rollout authorized on 2026-08-31. The V2 profile
remains disabled unless the protected mode and process-local rollout
confirmation are both supplied. This authorization covers only the exact
read-only KIS allowlist and never an account/order operation.

## Operating boundary

V2 is an owner-only, owner-scoped lifecycle. The fixed V1 profiles, mounts,
wrappers, registries, artifacts, and behavior remain separate and unchanged.
V2 never changes the V1 universe or its release gate.

The owner must confirm the configured market-data entitlement reference and
SHA-256 pin before any provider-backed run. Keep those typed pins in the
protected release configuration. Do not copy Raw or Curated data outside the
owner-confirmed rights boundary, and do not put a credential, access token,
account identifier, request body, response body, or provider prose in a log,
ticket, manifest, or runbook.

Only `owner-equity-v2-runner` receives the V2 KIS credential file references,
the worker DB password file, and the dedicated Raw/artifact read-write
mounts. API and Web receive neither the V2 credential files nor the V2 Raw or
artifact roots; they receive only the typed entitlement reference/hash pins
already required by the application contract. The provider-free verifier has
no DB setting, KIS secret, account identifier, or network access, and mounts
Raw and the V2 artifact root read-only.

KIS remains read-only. This runbook grants no account, balance, buying-power,
execution, order, correction, cancellation, or trading API access.

## Policy and coverage

Migration 0053 provisions the policy for every existing Owner and installs a
role-grant trigger for future Owners. The provisioned policy recommends at most 100 active instruments, a target of
261 observed sessions, and a minimum of 121 observations. A six-digit KRX
instrument id is owner input; membership and generation evidence remain
owner-scoped and immutable after admission. A generation below 121 observations
is typed `INSUFFICIENT_HISTORY` and cannot become `READY`.

The lifecycle is:

```text
REQUESTED -> VALIDATING -> BACKFILLING -> MATERIALIZING -> READY
```

The other typed outcomes are `INSUFFICIENT_HISTORY`, `FAILED`, `DISABLED`,
`RETRYING`, and `CANCELED`, subject to the existing WP-4 transition contract.
Do not manually update a worker state or bypass the queue/API transition.

## Initial backfill

1. Confirm owner identity, the owner policy, the 100-active capacity, the
   entitlement reference/hash, and the intended six-digit instrument ids.
2. Submit additions through the existing owner API. Each accepted addition
   creates one V2 queue job; do not batch unrelated instruments into one Raw
   evidence identity.
3. Enable the explicitly authorized `owner-equity-v2` profile only after the
   immutable release and host preflight have passed. The queue worker is
   sequential (`concurrency=1`) and applies one request per second per V2
   endpoint/TR channel by the WP-4/KIS runtime contract.
4. An initial Add/Retry job is limited to 7 GET requests. The aggregate initial
   backfill ceiling is 700 GET requests, which is the 100-instrument policy
   ceiling multiplied by the 7-request job ceiling. The runtime rejects an
   active limit above 100, an invalid request ceiling, or concurrency other
   than exactly 1 before work is started.
5. The numeric disk estimate is 1 MiB per GET: 7 MiB per initial job and 700
   MiB for the aggregate initial ceiling. This is a preflight estimate, not a
   claim about provider response size. Stop and resolve capacity if the
   dedicated Raw/artifact storage cannot safely hold the estimate plus the
   host reserve; the runtime fails closed on arithmetic overflow and invalid
   limits.
6. Raw visibility and generation admission occur only after the existing
   typed response validation, hash commit, materialization, and artifact
   checks. Never mark a generation READY by hand. A failed or insufficient
   job reports a typed code without provider text or a response body.

The policy may contain fewer than 100 active members. The 100 value is a hard
runtime maximum, not an instruction to add 100 instruments in one operation.

## Daily incremental run

For a previously admitted generation, the runner schedules one deterministic
incremental job after the 16:30 KST close gate using the latest provenance-bound
KRX trading-calendar session. PostgreSQL revalidates the exact READY membership,
policy, prior admitted generation, entitlement, code commit, and idempotency
identity before inserting the job. An already queued/running incremental job is
not duplicated. Its runtime ceiling is 2 GET requests per job, still sequential
at one request per second per endpoint/TR channel. The worker refuses a stale or
mismatched prior candidate and writes a new immutable generation/artifact rather
than modifying the prior generation.

Daily work remains read-only market-data collection. It does not read account
state and does not create, amend, cancel, reserve, or simulate an order.

## Immutable release and optional rollout

The serving image manifest contains the exact local image ID and OCI revision
for all twelve locally-built serving images, including
`owner-equity-v2-runner`. Production image builds are sequential; the runtime
stage contains the compiled runner and shared libraries only, not Cargo, a
compiler, or source. The installed release must be the current release whose
`.lagrange-release-manifest` and `LAGRANGE_CODE_COMMIT` agree.

The default is inactive. The Owner authorized the optional V2 rollout on
2026-08-31. In the protected installed Compose env, use:

```text
OWNER_EQUITY_V2_RUNTIME_MODE=owner_only
```

An apply operation must additionally receive the process-local confirmation
below. It is intentionally not stored in `.env` and is not a credential:

```text
OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS
```

The release script verifies every manifest image before the first Compose
operation and starts the V2 profile service only after this separate gate.
Ambient `COMPOSE_PROFILES` does not activate it. The existing V1 Paper policy
remains disabled by default and its scheduler selection is unchanged. Do not
set the mode or run the confirmation in this task's verification environment.

## Health, shutdown, and recovery

The daemon has `daemon`, `--once`, and `healthcheck` modes. Production health
is written to the private tmpfs path and is checked by the Compose healthcheck.
The configured recovery discipline is:

| Control | Value | Purpose |
| --- | ---: | --- |
| heartbeat interval | 10 s | extend the claimed-job lease while work runs |
| lease | 60 s | bound a stale claim |
| recovery sweep | 30 s | reclaim expired claims and reconcile exhausted work |
| retry backoff | 30 s | preserve bounded queue retry spacing |
| work timeout | 900 s | bound one claimed job |
| stop grace period | 16 min | allow the 15-minute work timeout to settle |

On shutdown, the daemon lets the current work settle when possible. On restart,
the existing WP-4 sweep reclaims stale claims. Exhausted `Add` and `Retry`
claims are reconciled to a retryable typed membership failure
`WORKER_CRASH_ATTEMPTS_EXHAUSTED`; no exhausted claim is silently published.
Do not delete queue rows or artifact directories as a recovery shortcut.

## Provider-free verifier

The queue worker materializes through the reviewed provider-free Rust seam. For
an operator verification of an immutable identity/candidate pair, use the
installed current-release wrapper with explicit absolute paths under the
dedicated V2 artifact root:

```text
/opt/lagrange/current/scripts/ops/owner-equity-v2-verify.sh --check \
  --identity-file /var/lib/lagrange/data/owner-equity-v2-artifacts/<identity>.json \
  --candidate-file /var/lib/lagrange/data/owner-equity-v2-artifacts/owner-equity-v2/<manifest-sha256>/candidate.json \
  --materializer-commit <exact-lowercase-40-hex-commit> \
  --candidate-sha256 sha256:<64-lowercase-hex>
```

The operator substitutes immutable, already-reviewed paths and hashes; no
secret is substituted. The wrapper proves the current release and exact
`research-worker` image ID/revision, then invokes only
`/usr/local/bin/owner-equity-v2-check` with:

- `--network none`, a read-only root filesystem, dropped capabilities,
  `no-new-privileges`, and UID/GID `10001:10001`;
- Raw mounted at `/data/raw:ro` and the dedicated V2 artifact root at
  `/data/artifacts:ro`;
- no Compose profile, DB connection, KIS credential, or provider endpoint.

The check emits only a sanitized success/failure code. It never emits the
candidate, manifest, hash input, Raw bytes, or provider response. The
materializer binary is installed in the reviewed collector image alongside
the checker; the queue worker owns the production artifact write path. A
materialize/check operation must not be turned into a networked one-shot.

## Capacity and typed failure handling

Before authorizing work, check the policy count and the dedicated data-root
capacity against the estimates above. The runtime rejects at least these
conditions before provider work: active limit over 100, initial ceiling below
the reviewed minimum or over the aggregate ceiling, incremental ceiling above
the initial ceiling, total initial ceiling below `7 × max_active`, total above
700, concurrency not equal to 1, zero bytes-per-GET, stale prior evidence, and
unsupported job actions.

Treat `CONFIG_INVALID`, `DATABASE_CONFIG_INVALID`, `PROVIDER_CONFIG_INVALID`,
`STORAGE_CONFIG_INVALID`, `RECOVERY_UNAVAILABLE`, `WORKER_UNAVAILABLE`,
`ARTIFACT_ROOT_UNSAFE`, `ARTIFACT_TAMPERED`, and
`WORKER_CRASH_ATTEMPTS_EXHAUSTED` as typed operational failures. Preserve the
queue/artifact evidence and investigate the code path. Do not retry a terminal
artifact or evidence mismatch by editing its bytes; use the existing retry
transition only when the persisted failure is retryable.

## Rollback

Rollback is an installed-release operation, not an image rebuild. The target
release must have its own exact `.lagrange-release-manifest`, including the V2
image record, and the immutable current link must be switched by the existing
release installer. After rollback, leave
`OWNER_EQUITY_V2_RUNTIME_MODE` disabled unless the owner separately authorizes
the target release. Never replace a manifest, image tag, current link, Raw
batch, or artifact in place, and never use a mutable tag to bypass the
manifest.

The rollback path does not delete or rewrite V2 database rows. If a worker
claim was active during the release change, allow the next worker's bounded
lease sweep to recover it and inspect the typed outcome. A rollback does not
authorize a KIS request outside the read-only allowlist.

## Verification record for this implementation

The provider-free checks cover Compose expansion with synthetic placeholders,
exact image manifest/build/static wiring, the fake V2 runtime boundary, the
network-none verifier command, the 100/7/2/700 ceilings, disk estimates,
health/lease/recovery settings, and disabled/owner-only release gates. No Docker
image build, Docker container start, installer, deployment, credential read, or
provider call occurred during this verification. The fake runtime test replaces
Docker with a local command stub and does not contact a Docker daemon.
