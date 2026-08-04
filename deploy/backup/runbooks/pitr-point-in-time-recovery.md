# Point-in-Time Recovery (PITR) — Contract Skeleton

**Status:** CONTRACT SKELETON. This file defines the PITR **contract** only. The actual
automation (pg_basebackup invocation, `archive_command`/WAL shipping, `recovery_target`
handling, restore orchestration) is implemented in **Todo 33** against the contracts below.
No HA or recovery capability is claimed by this document.

---

## 1. Contract inputs (a PITR operation consumes exactly these)

| Input | Source | Policy link |
|-------|--------|-------------|
| `db_base` class | base backup (daily logical or `pg_basebackup`) | manifest `classes[].class = db_base` |
| `db_wal` class | WAL archive covering the base through the target recovery point | manifest `classes[].class = db_wal` |
| Target recovery point | UTC timestamp or LSN chosen at restore time | runbook step input |
| `validate-policy` pass | gate exit 0 BEFORE any restore step | mandatory precondition |
| Isolated target | disposable DB name / fresh empty directory | `restore_policy.isolated_target_required = true` |

## 2. Mandatory preconditions

1. `scripts/backup/validate-policy.* -SetPath <set> -Gate <gate>` exits `0` (all required
   classes present, all sha256 present and matching, retention honored, storage rules
   honored, no secret markers). Nonzero ⇒ PITR must not start.
2. The WAL class is contiguous from the base backup through the target point (TODO-33
   automation asserts no gap in WAL segment names / LSN range).
3. The target database name and restore directories are disposable and distinct from
   production (isolation, enforced by policy + drill).

## 3. PITR steps (TODO-33 automation fills exact commands)

1. Run the policy gate; save the transcript.
2. Restore `db_base` into the isolated target (recovery mode, not normal startup).
3. Replay `db_wal` up to the target recovery point (TODO-33: `recovery_target_time` /
   `recovery_target_lsn` mapping + WAL shipping config contract).
4. Bring the target up read-only; run the assertions below.
5. On success, either promote the target (disaster-recovery path) or tear it down (drill
   path). Production is never touched by a drill.

## 4. Exact success assertions (ALL must be true; any false ⇒ PITR FAILED)

| # | Assertion | Machine check |
|---|-----------|---------------|
| P1 | Policy gate exit 0 + transcript saved | same check as A1/B1 |
| P2 | WAL replay reached the target point | `pg_controldata`/`pg_waldump` shows recovery ended at ≥ target LSN; `SELECT pg_last_wal_replay_lsn();` ≥ target |
| P3 | No rows after the target point | `SELECT count(*) FROM <table> WHERE created_at > '<target>';` → `0` |
| P4 | Data at the target point intact | spot count query at `<target>` matches pre-backup snapshot count (recorded at backup time) |
| P5 | Restored file classes (Raw/Curated/Artifact) hash-match manifest | `sha256sum -c` → `0` mismatches |
| P6 | No secret marker in the recovered set | grep of `secret_markers` → `0` hits |

## 5. Failure handling

- Any assertion false: treat the set as unusable for that recovery point; investigate (WAL
  gap, torn base, retention miss); re-run the drill after fix. Never declare PITR success on
  partial assertions.

## 6. Retention contract for PITR

- `db_wal` retention must cover the full desired PITR window and span from the newest
  retained `db_base` through today (policy floor: 14 days, configurable in
  `deploy/backup/policy/backup-policy.json`).
- A WAL segment older than `db_base`'s oldest retained base is outside the PITR window; the
  validator enforces retention floors, not wall-clock expiry — expiry policy is applied by
  the retention/cleanup automation (Todo 33) using the deterministic
  `expires_at = completed_at + retention_days` contract.

## 7. Evidence to record

- Gate transcript (P1), `pg_waldump`/replay-LSN output (P2), post-target row query (P3),
  spot count (P4), `sha256sum -c` transcript (P5), secret scan (P6). `.omo/evidence/`.
