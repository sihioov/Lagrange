# Production release installation and encrypted backups

These workflows remain repository artifacts until an operator explicitly
applies them as root. Do not apply them until the repository is committed and
clean, `/opt/lagrange` capacity is confirmed, and backup sizing is approved.

## Install an exact release

`deploy-production-release.sh` refuses every tracked change and every untracked
path except the exact operator-supplied
`docs/kis_openapi_entiredocs_20260818_030007.xlsx`. It requires `--commit` to
equal `git rev-parse HEAD`. It uses `git archive`, so
it never copies the mutable worktree, `.git`, host Raw/Curated, caches, secrets,
or the untracked KIS XLSX. The Compose `.env` is the sole host-specific addition
and must be supplied separately as a root-owned mode-0600 regular file.

```bash
export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
# Output must be empty or exactly the one allowlisted workbook above.
git status --porcelain=v1 --untracked-files=all
sudo install -o root -g root -m 0600 deploy/compose/.env \
  /etc/lagrange/compose.env.pending

scripts/ops/deploy-production-release.sh --dry-run \
  --commit "$LAGRANGE_CODE_COMMIT" \
  --env-source /etc/lagrange/compose.env.pending
sudo scripts/ops/deploy-production-release.sh --apply \
  --commit "$LAGRANGE_CODE_COMMIT" \
  --env-source /etc/lagrange/compose.env.pending
sudo scripts/ops/deploy-production-release.sh --check \
  --commit "$LAGRANGE_CODE_COMMIT" \
  --env-source /etc/lagrange/compose.env.pending
```

Each immutable directory is `/opt/lagrange/releases/<commit>`. `current` is an
atomically replaced relative symlink. Compatibility links such as
`/opt/lagrange/deploy -> current/deploy` provide an operator convenience view.
Protected TLS and backup configs keep their no-symlink fence and must pin
`/opt/lagrange/releases/<commit>/deploy/...`; after switching a release,
customize/check the TLS config for that exact release before renewal activation.
`/opt/lagrange/bin` stays installer-owned for stable helpers.
Applying a second commit never overwrites or deletes the first. Roll back with:

```bash
sudo scripts/ops/deploy-production-release.sh --rollback \
  --commit <previous-exact-40-hex> \
  --env-source /etc/lagrange/compose.env.pending
```

The release installer never runs Compose, restarts services, migrates a DB, or
contacts a provider. Rollout remains a separate validated action.

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
