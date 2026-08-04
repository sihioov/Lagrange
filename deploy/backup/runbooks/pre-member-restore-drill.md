# Pre-Member Restore Drill — MANDATORY GATE 1

**Status:** CONTRACT SKELETON. The automation referenced below (pg restore, WAL replay, file
extraction) is implemented in Todo 33 against these contracts. Until a drill passes with a
real executed transcript, **do not claim Member-launch readiness** and do not open the Member
surface.

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
