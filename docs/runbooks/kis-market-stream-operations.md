# KIS market-stream installation and recovery

This is the WP-6 operator procedure for the fixed 30-stock, single-Owner market stream.
As of 2026-10-04, local packaging checks, the corrected disposable initializer UID/mount
checks, owned-container cleanup checks, and grant SQL checks have passed. The UID/mount
checks used an injected CLI, and the SQL checks used isolated actual database roles; they
do not establish final packaged-image or installed-helper behavior. Actual installation,
provider access, and final acceptance have not occurred. The completion plan indexes the
raw evidence and its limits.

Use the [contract](../superpowers/specs/2026-09-21-kis-market-stream-contract.md),
[completion plan](../superpowers/plans/2026-10-01-kis-market-stream-completion.md), and
[live-readiness gates](kis-market-stream-live-readiness.md) together. Commands below are
templates for a separately reviewed operation. Placeholders are not runnable defaults or
approval artifacts. Never place credentials, session cookies, provider messages, or state
contents in the command record.

## Release and mode boundaries

| State | Required evidence |
| --- | --- |
| `SOURCE_LOCAL_ACCEPTED` | Current source, actual-role DB and end-to-end API/nginx/browser checks, fault matrix, and at least 30-minute soak accepted |
| `INSTALLED_OFF` | Exact clean commit, complete V2 manifest, current installed inputs, compatible migrations, persistent state/anchor identity, quotes off, and current service health |
| `LIVE_ACCEPTED` | G1–G4, separately authorized pilot, actual market ACK/receipt evidence, all 30 admissions, authenticated display and lifecycle evidence |
| `FINAL_ACCEPTED` | Live acceptance plus same-source EOD and the completion-plan acceptance matrix |

`OWNER_INTRADAY_QUOTES_MODE=off` prevents quote production. When it is `owner_only`,
the protected dotenv must explicitly contain `OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws`.
Missing, empty, `rest`, or other transport values reject activation. Ambient shell values
cannot supply or override that protected choice, and `_FILE` aliases remain forbidden.
The REST default applies only to historical releases; new releases cannot activate a REST
quote producer. Off mode tolerates missing or `rest` transport without starting quotes.
Stream failure yields stale, unavailable, or off state, never REST quote fallback.
EOD REST collection remains a separate, previously authorized path.
The `live` profile and all account/order paths remain forbidden.

Source and fixture validation do not accept the remaining live gates. The Owner accepted the
exact market channel and plaintext transport on 2026-10-07, as recorded in the
[live-readiness update](kis-market-stream-live-readiness.md#owner의-평문-연결-수락--2026-10-07).
Current slot capacity, reviewed rights/grant inputs, and a real current-day calendar/window
proof still require their own evidence before activation.
For the Owner's subsequent 2026-10-07 deployment instruction, apply the narrow
[ADR-0009 activation exception](../decisions/0009-kis-owner-directed-websocket-activation.md)
to the G2 pre-activation hold. Keep G2 unverified and retain every runtime limit and failure
boundary. This exception does not grant full live acceptance or waive protected grant,
current-day proof, state identity or immutable-release checks.

An installed env and manifest belong to one immutable commit. Do not patch the installed
`.env`, overwrite an existing release directory, or use shell mode overrides. Off preparation
and later activation must have separately pinned release inputs. If an off release already
occupies the commit path, a different env requires a new clean activation commit and its
matching image set. Keep the compatible off release for rollback. The deployment helper does
not manufacture a second env variant of the same installed commit.

## Prepare and install with quotes off

First recover current deployment, image/daemon, migration, backup/restore, reader ownership,
memory/swap, OOM, and service-health evidence under the approved operational scope. Historical
test reports do not establish current host state. Verify the actual 0055/0056/0057 migration
history; do not infer that an existing table is absent or safe to replace.

Follow [release installation](production-release-and-backup.md) for the official builder and
installer. Each production Compose build names one service. Use `CARGO_BUILD_JOBS=2` and
`COMPOSE_PARALLEL_LIMIT=1`, a background systemd service with lowered CPU/I/O priority, and
logical batches of two or three services. Between batches check memory/swap/OOM, exit status,
service health, and absence of prior compiler processes. Never build the entire image set
concurrently. Final validation must produce the strict twelve-image V2 manifest.

D4 builds both `owner-equity-v2-runner` and `kis-market-stream-state` with the opt-in
`market-stream-provisioning` feature. The initializer is at
`/usr/local/bin/kis-market-stream-state` in the runner image. Other recipes retain their
default feature selection. Validate the actual packaged binaries and artifact receipts;
Cargo metadata or a source-only Dockerfile check is insufficient.

Use the exact candidate's off env and a root-owned 0600 manifest below trusted ancestors:

```bash
scripts/ops/deploy-production-release.sh --dry-run --commit "$stream_off_commit"
sudo scripts/ops/deploy-production-release.sh --apply --commit "$stream_off_commit" \
  --env-source "$stream_off_env" --release-manifest "$stream_off_manifest"
sudo scripts/ops/deploy-production-release.sh --check --commit "$stream_off_commit"
```

The installer copies an exact clean Git archive and protected inputs, then switches `current`.
It does not call Docker, run migrations, start services, provision state, or call a provider.
Every such operation has a separate scope and evidence record. The artifact set must contain
the new scripts and SQL; untracked worktree files are not installed by `git archive`.

## Create one new persistent domain

The installed off env must select `shared_required`, a canonical nonnil credential-slot UUID,
and the existing positive credential generation. Do not rotate or increment the generation
to activate WS. The host provisioning procedure creates only the persistent parent as
`root:10001` mode 0750. Both WS leaves must be absent on first installation; the initializer
creates them. Preserve the existing REST coordination leaf and its counters.

Stop and verify the exact runner under the approved service operation before provisioning.
The provisioning command also rejects a running runner. It binds the executing current
release, complete manifest, local image ID/revision, slot/generation, and protected parent:

```bash
/usr/bin/python3 -I /opt/lagrange/current/scripts/ops/provision-owner-market-stream.py \
  --plan --expected-commit "$stream_off_commit"
sudo /usr/bin/python3 -I /opt/lagrange/current/scripts/ops/provision-owner-market-stream.py \
  --initialize-new --expected-commit "$stream_off_commit"
sudo /usr/bin/python3 -I /opt/lagrange/current/scripts/ops/provision-owner-market-stream.py \
  --check --expected-commit "$stream_off_commit"
```

Initialization consumes a fsynced, exclusive attempt record before invoking the fixed Rust
initializer in a bounded, network-disabled container. Root creation is followed by validation
as UID:GID 10001:10001 with a read-only parent bind. The state leaf is 10001:10001 mode 0700;
its state file is mode 0600. The separate anchor leaf is root:10001 mode 0750 and contains
distinct, zero-length, single-link root:10001 mode-0440 anchors. Runtime receives only the
state leaf read-write and anchors read-only, both with `create_host_path: false`.

Reapply uses `--check` only. It preserves state, budgets, ambiguity, connection history, and
anchor identities. An attempted or partial installation, missing/empty state, identity or
permission mismatch, failed fsync, or uncertain cleanup is an incident. Preserve it; never
delete the record, reset the state, repair/chmod old anchors, or relabel it as first install.
Do not use tmpfs or per-container copied anchors in production. Record only hashes, metadata,
closed result codes, and the exact owned process/container identity.

## Install or revoke a reviewed grant

G1–G4 and a reviewed approval artifact are prerequisites for an actual grant operation.
The 2026-10-07 ADR-0009 exception permits the reviewed operation with G2 explicitly unverified;
it supplies no invented capacity or global key-ownership evidence.
Source fixtures never supply production approval. Section 17 of the contract fixes the
canonical JSON fields, exact hash, slot/generation/Owner/entitlement binding, wire/identity
constants, inclusive dates, and activation commit. Keep the input root:root mode 0600 under
trusted ancestors. Its exact hash is an operator input; do not derive rights from a CLI flag.
Canonical encoding uses literal UTF-8 for non-ASCII characters, sorted keys, compact
separators, and one trailing LF. An alternative Unicode-escaped encoding has different bytes
and is rejected. The protected env file itself must contain the exact executing commit;
an inherited shell value cannot fill an absent or empty commit.

```bash
/opt/lagrange/current/scripts/ops/install-owner-market-stream-grant.sh --plan
sudo /opt/lagrange/current/scripts/ops/install-owner-market-stream-grant.sh --apply \
  --expected-commit "$stream_activation_commit" \
  --approval-input "$stream_approval_file" --approval-sha256 "$stream_approval_sha256"
```

The helper uses only the installed, image-pinned `db-migrate` service and its existing secret
path; it does not run migrations or start dependencies. The single transaction rechecks the
Owner role and exact ACTIVE entitlement, then inserts the immutable grant. An identical
ACTIVE replay is a no-op. A changed or revoked replay and active slot/Owner conflicts fail.
Never use manual SQL to make a grant, admission, producer lease, or membership look valid.

Explicit revocation only changes the selected grant from ACTIVE to REVOKED:

```bash
sudo /opt/lagrange/current/scripts/ops/install-owner-market-stream-grant.sh --apply \
  --expected-commit "$stream_current_commit" --revoke --grant-id "$stream_grant_id"
```

Already revoked is a no-op; missing is an error. A timeout or uncertain COMMIT is not success
and is not automatically retried. Reconcile the exact operation and selected grant before
another approved action. Keep entitlement amendment, grant installation, and state
initialization distinct; none substitutes for another.
Revocation can run from a verified current off release without WS binding settings: it still
requires the exact installed commit, manifest, local image and explicit grant ID. It consumes
no new approval artifact. Every attempted operation checks that its named container is gone;
cleanup may remove only a matching full container ID, actual image ID, name and operation
label, then verifies both ID and name are absent. Failed cleanup never counts as success.

## Activation and off rollback

Complete G1–G4 first, including current capacity for 30, exclusive slot ownership, rights for
the WS/cache/Owner SSE path, and a real same-day calendar/window proof with a next-day supply
procedure. Follow [daily evidence installation](stock-beta-intraday-quotes.md#operational-day-evidence-and-manual-activation).
Apply ADR-0009 only to its expressly bounded Owner-directed deployment; other operations
retain these prerequisites, and missing later day proof remains a closed collection state.
Reuse the same committed KIS calendar source for EOD; never recapture an uncertain daily claim.

Install the separately pinned activation release through the same immutable installer. Its
protected env must bind `owner_only`/`market_ws`, shared coordination, exact grant/slot/contract,
canonical HTTPS Owner origin, and the reviewed window source. API `DB_APP_MAX_CONNECTIONS`
defaults to 32; an explicit value below 24 is rejected so twenty LISTEN clients leave read
capacity. No WS credential or endpoint override is added to API/Web.

```bash
/opt/lagrange/current/scripts/ops/compose-release.sh --scope release \
  --refresh-market-stream --refresh-from-commit "$stream_off_commit" --plan
/opt/lagrange/current/scripts/ops/compose-release.sh --scope release \
  --refresh-market-stream --refresh-from-commit "$stream_off_commit" --preflight
```

Only after the separate activation authorization, invoke `--apply` with both existing
process-local acknowledgements:
`OWNER_EQUITY_V2_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS` and
`OWNER_MARKET_STREAM_ROLLOUT_CONFIRM=I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS`.
Acknowledgements do not create a grant, day proof, admission, or provider permission.
Use the same `--refresh-from-commit` in the approved apply command. It selects only the
trusted manifest at `<release-root>/releases/<commit>` for verifying the running API/Web/runner.
The executing current release still supplies every replacement image. Missing, symlinked,
malformed manifests and foreign or mixed running images fail before any recreation.
Omit the option only when the running services already belong to current. Refresh recreates
API, Web, and runner serially with exact image IDs. It performs no build, migration, grant,
or state creation. These transitions have provider-free fake-Docker coverage; actual host
transition and service health still require the separately approved operation.

For a failure, stop the exact producer through the approved service procedure, verify its
owned socket and all child tasks have closed/joined, and preserve uncertain lease/state
evidence. Switch `current` with `deploy-production-release.sh --rollback --commit` to the
prevalidated compatible off release, then use its `--refresh-market-stream` plan/preflight/apply
procedure with `--refresh-from-commit "$stream_activation_commit"`. A stopped or mixed set
fails the running-container guard; recover that incident through a separately reviewed exact
service operation before refresh. Never bypass the guard or rewrite the manifest.
Do not hand-edit the installed `.env` to turn quotes off. Use the compatible immutable off
release and its verified refresh procedure. No REST fallback or migration down is implied.

## Acceptance evidence

Record source/commit and command hashes, actual selected tests, process exit status before
parsing logs, timestamps, role/cluster identity, final catalog, and exact owned cleanup.
Keep failed, skipped, compile-only, and unexecuted cases visible. Synthetic command tests,
root/bind tests using an injected binary, and final packaged-image tests are distinct evidence.

Before source/local acceptance, exercise the real runtime-to-writer-role-DB-to-authenticated
API/nginx/browser chain, fault matrix and at least 30-minute soak. Measure unique upstream
subscriptions with one and ten tabs, ACK-before-publication, last-demand drain, slow consumers,
commit uncertainty, day/rights/generation changes, buffer and SSE bounds, write cadence,
latency and coalesced updates. A healthy service or periodic status event is not a new quote.
Live acceptance additionally requires the G5 one-stock pilot, then all 30 actual ACKs and
fresh authenticated display updates. Ordinary EOD acceptance is separate and remains required.
