# Lagrange Station — Backup, PITR, File-Restore & Secret-Recovery Contracts

Defines the **release contracts** for backup and restore (design §13.4, NFR-REL-005,
NFR-SEC-002/003). Everything here is a contract + policy gate; the executable restore
automation is implemented later in **Todo 33** against these contracts. The
`scripts/backup/validate-policy.*` policy gate is implemented and enforced **now**.

> **No-HA rule:** this document, its schemas, and its runbooks do NOT claim high
> availability or proven recovery. Recovery is proven only by a **successful drill**
> (pre-Member full restore, then pre-Live reconciliation-only restore) with executed,
> machine-checkable transcripts. Until a drill passes, Member/Live readiness stays blocked.

## 1. Backup classes (manifest `classes[]`)

| class | kind | dataset | Content | Encryption floor | Retention floor (days) |
|-------|------|---------|---------|------------------|------------------------|
| `db_base` | db | — | PostgreSQL base backup (daily logical or `pg_basebackup`) | `required` | 7 |
| `db_wal` | db | — | WAL archive for PITR (contiguous through the window) | `required` | 14 |
| `file_raw` | file | raw | KRX Raw ingestion files (immutable, incremental) | `allowed` + reference OK | 30 |
| `file_curated` | file | curated | Curated bars/corporate actions (versioned, incremental) | `allowed` + reference OK | 90 |
| `file_artifact` | file | artifact | Backtest/Paper artifacts (Parquet + manifests) | `allowed` + reference OK | 180 |

- All five classes are required in every full backup set and by every gate
  (`default`, `premember`, `prelive`).
- Strategy packages and configuration are backed up by Git + release tags (design §13.4),
  not by file classes.

## 2. Backup layout

- One backup **set** = one self-contained directory containing `backup-manifest.json` plus
  the archived files: DB archives under `pg/base/...` and `pg/wal/...`; file increments under
  `files/{raw,curated,artifact}/<completed_at>/...`.
- Manifest file `path` entries are **relative to the set root** and must never escape it
  (no `..`, no absolute/drive paths, no backslashes).
- Every file entry carries its `sha256`; the validator recomputes and compares each one.
- Host-side staging/layout hooks (per Todo 1): `deploy/backup/postgres/`,
  `deploy/backup/files/`, `deploy/backup/manifests/`, `deploy/backup/scripts/` hold cron /
  operator-driven staging. Nothing under this directory is runtime-composed.

## 3. Manifest contract

- Canonical schema: `deploy/backup/policy/backup-manifest.schema.json` (draft-07).
- Enforced policy: `deploy/backup/policy/backup-policy.json` (required classes, retention
  floors, storage/encryption rules, `secret_markers`, `forbidden_path_segments`, gates).
- Manifest declares per class: `backup_id`, UTC `started_at/completed_at`, `retention_days`,
  `expires_at` (must equal `completed_at + retention_days`), `storage.encryption` +
  `storage.location`, and `files[]` with `path` + `sha256`.
- Manifest declares `restore_policy.isolated_target_required: true` (isolated restore
  targets only) and per-gate assertions, including `prelive.startup_mode =
  "reconciliation_only"`.

## 4. Retention

- Policy floors (minimums) per class are in `backup-policy.json` (7/14/30/90/180 days).
  A class may retain longer, never shorter.
- `expires_at = completed_at + retention_days` is enforced deterministically by the
  validator (no wall clock) so the same input always yields the same verdict; the
  cleanup/retention automation (Todo 33) applies expiry.
- `db_wal` must span from the newest retained `db_base` through the present for a valid PITR
  window.

## 5. Encryption / reference rules

- `db_base` and `db_wal`: `storage.encryption` must be `required` (DB archives may contain
  row data); reference storage is forbidden for db classes.
- File classes: `encryption` may be `allowed`; content-addressed `reference` storage is
  allowed.
- `encryption: "none"` is forbidden for **every** class.

## 6. Secret-exclusion rule (hard policy)

- Secrets NEVER enter ordinary backup archives. They follow the dedicated procedure in
  `deploy/backup/runbooks/secret-recovery.md` (reference-based re-issue/rotation + runtime
  injection).
- The validator rejects a set if any file **path** matches a `forbidden_path_segments` entry
  (`secrets/`, `credentials/`, `.env`, `kis-credentials`, `id_rsa`, `id_ed25519`, ...) or any
  file **content** / the manifest itself contains a `secret_markers` entry
  (`kis_app_secret`, `BEGIN OPENSSH PRIVATE KEY`, `LAGRANGE_SECRET_MARKER`, ...).
- A rejected archive is treated as potentially compromised: rotate the exposed secret,
  quarantine the archive, record an incident. Never restore from it.

## 7. Policy gate — `scripts/backup/validate-policy.*`

`validate-policy.ps1` (Windows/pwsh) and `validate-policy.sh` (POSIX/WSL2/CI, requires bash +
python3 + sha256sum) are logic twins.

```pwsh
scripts/backup/validate-policy.ps1 -SetPath <backup-set-dir-or-manifest> [-Gate default|premember|prelive]
bash scripts/backup/validate-policy.sh --set <backup-set-dir-or-manifest> [--gate default|premember|prelive]
```

| Exit | Meaning |
|------|---------|
| 0 | POLICY OK — all required classes, hashes, retention, storage rules, secret exclusions confirmed. Restore may proceed. |
| 1 | POLICY REJECTED — every violation printed as `VIOLATION[n] <field>: <reason>` with the exact missing/rejected field. Restore MUST NOT start. |
| 2 | USAGE/LOAD error. |

Checks performed (deterministic — identical input ⇒ identical output):

1. required classes present, exactly once (missing/duplicate named);
2. per class: kind/dataset, retention ≥ floor, UTC `completed_at`, `expires_at` consistency,
   storage encryption/location/reference rules;
3. per file: path safety (no escape from set root), `sha256` present + well-formed,
   file exists, hash matches (declared vs computed named), forbidden path segments;
4. secret content scan of every archive file and the manifest itself;
5. restore-policy assertions: `isolated_target_required`, per-gate required classes,
   `prelive.startup_mode = "reconciliation_only"`.

Tests: `scripts/backup/tests/test-validate-policy.*` (red-first harness, 6/6 assertions) over
synthetic fixtures in `scripts/backup/tests/fixtures/`.

## 8. Mandatory release gates (runbooks under `deploy/backup/runbooks/`)

| Gate | Runbook | Blocking |
|------|---------|----------|
| GATE 1 — pre-Member | `pre-member-restore-drill.md` | full DB + file restore into isolated targets; exact assertions A1–A8. Blocks the Member surface. |
| GATE 2 — pre-Live | `pre-live-reconcile-restore.md` | full restore + **reconciliation-only startup**; exact assertions B1–B9. Blocks Owner-only Live enablement. |
| Secrets | `secret-recovery.md` | separate reference-based recovery; never via archives. |
| PITR | `pitr-point-in-time-recovery.md` | base + WAL replay to a target point; assertions P1–P6; automation in Todo 33. |

Every runbook success assertion is a machine-checkable command + expected result (validator
exit, `sha256sum -c` zero mismatches, `pg_dump` schema diff zero lines, row-count SQL,
zero secret-marker hits, zero open reconciliation mismatches). No assertion may be settled by
operator judgment.

## 9. File inventory

```
deploy/backup/
  README.md                                 this contract
  policy/backup-manifest.schema.json        canonical manifest JSON Schema
  policy/backup-policy.json                 enforced policy (classes, retention, storage, secrets, gates)
  runbooks/pre-member-restore-drill.md      GATE 1
  runbooks/pre-live-reconcile-restore.md    GATE 2
  runbooks/secret-recovery.md               separate secret procedure
  runbooks/pitr-point-in-time-recovery.md   PITR contract skeleton
scripts/backup/
  validate-policy.ps1                       policy gate (pwsh)
  validate-policy.sh                        policy gate (POSIX twin)
  tests/test-validate-policy.ps1/.sh        red-first acceptance harness (6/6)
  tests/fixtures/complete|incomplete-missing-wal|incomplete-missing-hash|tampered-hash|fake-secret/
```
