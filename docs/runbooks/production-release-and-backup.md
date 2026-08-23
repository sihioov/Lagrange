# Production release installation and encrypted backups

These workflows remain repository artifacts until an operator explicitly
applies them as root. Do not apply them until the repository is committed and
clean, `/opt/lagrange` capacity is confirmed, and backup sizing is approved.

## Install an exact release

`build-production-images.sh` and `deploy-production-release.sh` refuse every
tracked change and every untracked path except the exact operator-supplied
`docs/kis_openapi_entiredocs_20260818_030007.xlsx`. `--commit` must equal
`git rev-parse HEAD`. Deployment uses `git archive`, so it never copies the
mutable worktree, `.git`, host Raw/Curated, caches, secrets, or the untracked
KIS XLSX.

This is a single-host, single-platform owner-beta release. Its V2 manifest
records Docker's exact local immutable `.Id` as `image_id` (`sha256:` plus 64
lowercase hex), not a `RepoDigest`, registry digest, or multi-architecture
claim. The manifest scope is exactly these ten locally built serving services:

- `db-role-bootstrap`, `db-migrate`, `api-server`, `web`, `research-worker`
- `recommendation-runner`, `candidate-runner`, `nt-backtest-worker-1`,
  `nt-backtest-worker-2`, `paper-scheduler`

Postgres and reverse-proxy are independently content-pinned upstream images and
are outside this local-image manifest. `research-range-raw` is an
operator-confirmed historical-capture profile, not an owner-beta serving
release service; use its dedicated Stage5 runbook. `live-node-owner` and the
`live` profile remain forbidden and excluded.

The protected Compose `.env` and the V2 manifest are separate root-owned
mode-0600 regular files. The manifest must also live below a root-owned,
non-group/other-writable, non-symlink path. The deployer validates that external
file, copies it once into release staging, and validates the installed copy
again before atomically changing `current`.

```bash
export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
# Output must be empty or exactly the one allowlisted workbook above.
git status --porcelain=v1 --untracked-files=all

sudo install -o root -g root -m 0600 deploy/compose/.env \
  /etc/lagrange/compose.env.pending
sudo install -d -o root -g root -m 0755 /etc/lagrange/release-manifests

sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/build-production-images.sh --apply \
  --manifest-file "/etc/lagrange/release-manifests/$LAGRANGE_CODE_COMMIT.manifest"

scripts/ops/deploy-production-release.sh --dry-run \
  --commit "$LAGRANGE_CODE_COMMIT" \
  --env-source /etc/lagrange/compose.env.pending
sudo scripts/ops/deploy-production-release.sh --apply \
  --commit "$LAGRANGE_CODE_COMMIT" \
  --env-source /etc/lagrange/compose.env.pending \
  --release-manifest "/etc/lagrange/release-manifests/$LAGRANGE_CODE_COMMIT.manifest"
sudo scripts/ops/deploy-production-release.sh --check \
  --commit "$LAGRANGE_CODE_COMMIT"
```

Each immutable directory is `/opt/lagrange/releases/<commit>`. `current` is an
atomically replaced relative symlink. Compatibility links such as
`/opt/lagrange/deploy -> current/deploy` provide an operator convenience view.
Protected TLS and backup configs keep their no-symlink fence and must pin
`/opt/lagrange/releases/<commit>/deploy/...`; after switching a release,
customize/check the TLS config for that exact release before renewal activation.
`/opt/lagrange/bin` stays installer-owned for stable helpers.
Applying a second commit never overwrites or deletes the first. `--check` and
`--rollback` use only the installed trusted manifest; they reject an optional
external manifest and explicitly block a legacy manifest-less release. Roll
back with:

```bash
sudo scripts/ops/deploy-production-release.sh --rollback \
  --commit <previous-exact-40-hex>
```

The release installer never runs Compose, rebuilds/restarts services, migrates
a DB, or contacts a provider. Rollout is a separate installed-release action:

```bash
sudo /opt/lagrange/current/scripts/ops/compose-release.sh --scope release --apply
```

That command requires the installed V2 manifest, rechecks every local image ID
and OCI revision immediately before startup, creates a mode-0600 temporary
Compose override mapping the ten services to `image: sha256:<image_id>` with
build reset, and passes `--no-build` to every `up`/`run`. It checks the actual
`.Image` and OCI revision of each persistent local service after startup. The
one-shot services consume the same override but are intentionally not claimed as
post-start inspected after `--rm`. A mismatch returns failure without printing
container/environment configuration and without automatically stopping or
rolling back services.

No real registry, multi-architecture, or remote-Docker verification is claimed
or required for this host-local beta. A future registry promotion needs separate
owner approval and a new provenance contract.

## Size and configure backups

Backups contain exactly three encrypted classes: a consistent PostgreSQL
custom-format dump, Raw, and Curated. They exclude `/etc/lagrange`, Compose
`.env`, TLS/Auth0/KIS credentials, the encryption key, checkout, and untracked
files.

Choose a dedicated filesystem and size `MAX_TOTAL_BYTES`, `MIN_FREE_BYTES`,
`RETENTION_DAYS`, and `MIN_KEEP` from current DB + Raw + Curated size and growth.
`MIN_KEEP` is a hard floor. If the byte cap cannot be met without crossing it,
the run fails visibly instead of deleting additional sets.

```bash
sudo install -o root -g root -m 0600 \
  deploy/systemd/production-backup.conf.example \
  /etc/lagrange/production-backup.conf.pending
sudoedit /etc/lagrange/production-backup.conf.pending
```

Replace the commit placeholder and verify every path. `KEY_FILE` points to the
existing exact 64-lowercase-hex, no-newline
`/etc/lagrange/secrets/backup_encryption_key`; its value never enters argv,
environment, logs, manifests, or metrics.
Pin `COMPOSE_FILE` and `COMPOSE_ENV_FILE` to the same immutable
`/opt/lagrange/releases/<commit>` directory rather than the mutable `current`
link; `LAGRANGE_CODE_COMMIT` must be that identical commit.

```bash
sudo scripts/ops/run-production-backup.sh --check \
  --config-file /etc/lagrange/production-backup.conf.pending
scripts/ops/install-production-backup.sh --dry-run \
  --config-source /etc/lagrange/production-backup.conf.pending
sudo scripts/ops/install-production-backup.sh --apply \
  --config-source /etc/lagrange/production-backup.conf.pending
sudo scripts/ops/install-production-backup.sh --check \
  --config-source /etc/lagrange/production-backup.conf.pending
```

Apply enables both timers but deliberately does not start either and never
calls Docker. After the configured immutable release paths pass check, the
PostgreSQL image is present, and disk headroom is confirmed,
activate scheduling separately:

```bash
sudo systemctl start lagrange-production-backup.timer
sudo systemctl start lagrange-production-backup-verify.timer
```

The daily timer runs at 02:15 KST with a randomized 30-minute delay. Each set is
staged on the backup filesystem, encrypted with AES-256-CBC/PBKDF2, hashed,
decrypted into a private temporary directory, and restored into a networkless
disposable PostgreSQL instance. Raw and Curated member paths are validated and
extracted. Only then are `VERIFIED` and `COMPLETE` written and the set atomically
published. The weekly timer repeats the isolated restore of `latest`.

Retention runs only after a new verified set exists. It deletes only strictly
named, complete, verified directories under configured `BACKUP_ROOT`, oldest
first, and reports each deletion as unrecoverable. Releases are never pruned.

## Repository-only verification

These use namespace root and fake Docker; they do not touch host `/opt`,
systemd, PostgreSQL, production data, or credentials.

```bash
bash scripts/ops/production-ops-static-check.sh
bash scripts/ops/production-ops-self-test.sh
git diff --check
```
