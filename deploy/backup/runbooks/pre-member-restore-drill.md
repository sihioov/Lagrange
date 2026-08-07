# Pre-Member Restore Drill — MANDATORY GATE 1

**Status:** AUTOMATED (Todo 33). The pg restore, WAL replay, and file extraction referenced
below are implemented by `scripts/backup/restore-and-verify.{sh,ps1}`, which runs the drill
into a disposable Compose project and emits a machine-readable verdict. Until a drill passes
with a real executed transcript, **do not claim Member-launch readiness** and do not open the
Member surface.

```bash
scripts/backup/restore-and-verify.sh \
    --set <backup-set-dir> --sidecar <backup-sidecar.json> \
    --gate premember --key-file /etc/lagrange/backup.key \
    --verdict .omo/evidence/premember-drill.json
```

The `premember` gate is not cosmetic: `deploy/backup/policy/backup-policy.json` requires a
FULL restore for it, so a set that would satisfy the everyday `default` gate can still be
refused here.

The drill is only meaningful if it also fails when it should. Prove that with
`scripts/backup/tests/test-restore-failures.{sh,ps1}`, which asserts that a wrong key, a
missing WAL segment, a corrupt archive, a planted secret marker, a partial DB, and an expired
set each abort — and that no drill container survives the failure.

**Scheduling.** `create.*` and `restore-and-verify.*` drive Docker themselves, so they run on
the HOST scheduler, not as a container inside the stack:

Always `--key-file`, never `--key`: an argv passphrase is readable by every user via `ps`.

```cron
# daily verified backup, weekly restore drill (UTC)
17 18 * * *  /srv/lagrange/scripts/backup/create.sh --out /srv/backups/$(date -u +\%Y\%m\%d) \
                 --key-file /etc/lagrange/backup.key \
                 --metrics /var/lib/node_exporter/textfile/lagrange_backup.prom
41 19 * * 0  /srv/lagrange/scripts/backup/restore-and-verify.sh --set /srv/backups/latest/set \
                 --sidecar /srv/backups/latest/backup-sidecar.json \
                 --metrics /var/lib/node_exporter/textfile/lagrange_restore.prom
23 20 * * *  /srv/lagrange/scripts/backup/prune.sh --root /srv/backups --apply
```

Staleness of either metric alerts via `deploy/compose/alerts/backup-recovery.rules.yml`.

**Gate rule (NFR-REL-005, System Design §13.4):** a full PostgreSQL + Raw/Curated/Artifact
file restore must be rehearsed into an ISOLATED target BEFORE the Member surface is enabled.
This drill is the gate. No restore starts without a passing `validate-policy` run.

---

## 1. Precondition — policy gate (machine-checkable, runs NOW)

```pwsh
scripts/backup/validate-policy.ps1 -SetPath <backup-set-dir> -Gate premember
# or: bash scripts/backup/validate-policy.sh --set <backup-set-dir> --gate premember
```

**Success assertion A1:** the command exits `0` and prints
`POLICY OK: backup set <set-id> valid for gate premember`. Any nonzero exit (missing WAL /
base class, missing/mismatched sha256, retention below floor, secret marker found) **aborts
the drill before any restore command runs** — this is the fail-closed pre-restore gate.

## 2. Restore into isolated targets (TODO automation — Todo 33)

- **DB target:** a disposable database `lagrange_restore_drill_<UTC-ts>` on a scratch cluster
  (or disposable container). Must NOT be the production database name.
- **File targets:** freshly created, EMPTY directories for Raw, Curated, and Artifact
  (`<scratch>/restore/raw`, `/curated`, `/artifact`). Record their pre-restore emptiness.
- TODO-33 steps: restore `db_base` (pg_basebackup / logical dump), replay `db_wal` to the
  target recovery point, extract each declared increment file into the matching empty target.

## 3. Exact success assertions (ALL must be true; any false ⇒ DRILL FAILED, gate not passed)

| # | Assertion | Machine check |
|---|-----------|---------------|
| A2 | Restored DB schema matches source snapshot | `pg_dump --schema-only` of restored DB vs reference snapshot produces `0` diff lines |
| A3 | Every restored file's sha256 equals its manifest sha256 | `sha256sum -c` against manifest entries → `0` mismatches |
| A4 | No secret marker anywhere in the restored tree or restored DB dump | `grep -a -F` for each `secret_markers` entry → `0` hits |
| A5 | Latest curated bar present at the expected point in time | `SELECT count(*) FROM curated_bars WHERE trading_date = '<expected>';` → `> 0` |
| A6 | Audit rows restored and intact | `SELECT count(*) FROM audit_logs;` equals the recorded pre-backup count |
| A7 | Isolation holds | restored DB name ≠ production name; restore targets were empty before restore (recorded in step 2) |
| A8 | Validator transcript saved | A1 transcript saved to the evidence bundle under `.omo/evidence/` |

## 4. Failure handling

- Any assertion A1–A8 false: drill fails; Member surface stays blocked; record the failing
  assertion + exact output; investigate; re-run after fix. Never "pass" on operator judgment.
- Assertion A1 failure automatically means no restore command was executed (validator is the
  gate and aborts).

## 5. Evidence to record

- `validate-policy` transcript (A1), the `sha256sum -c` transcript (A3), the secret-scan
  transcript (A4), the SQL results (A5, A6), and the empty-target proof (A7). Evidence lives
  under `.omo/evidence/` and is never committed into product commits.
