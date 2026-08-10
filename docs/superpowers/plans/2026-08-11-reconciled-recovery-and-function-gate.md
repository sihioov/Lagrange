# Reconciled Recovery and Exact Function Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give recovery a durable manifest append-order identity and make the schema gate reject any drift in the append-only trigger function body.

**Architecture:** Preserve the shared, synthetic `read_manifest` API, but add an exclusive recovery read that repairs manifest tails, re-syncs orphan evidence, durably appends each orphan without a nested lock, and returns only real JSONL order. Keep the existing batch-ID parent/helper protocol because every boundary now identifies an immutable manifest line. Compare the pinned PostgreSQL normalized `pg_get_functiondef` output bidirectionally against one exact expected definition.

**Tech Stack:** Rust 2024, `fs2` file locks, JSONL Raw storage, Tokio tests, PostgreSQL 18 catalog SQL, PowerShell/Bash Compose QA.

---

### Task 1: Reproduce unstable orphan ordering

**Files:**
- Modify: `data-pipelines/collectors/tests/research_worker.rs`
- Modify: `crates/market-data/src/storage.rs`

- [ ] **Step 1: Write the failing collector regression test**

Create an ingested batch `O`, remove only its manifest so the immutable batch is
an orphan, recover one page, append normal batch `N`, and assert the suffix pass
publishes `N` and the durable manifest is `[O, N]`.

```rust
let orphan = ingest_bundle(&store, &provider(), &request("2026-08-05T07:00:00Z"), None)?.entry;
std::fs::remove_file(store.manifest_path(PROVIDER_KRX, MARKET_KR))?;
let first = recover_unpublished_page_with(/* page size 1 */).await?;
let normal = ingest_bundle(&store, &provider(), &request("2026-08-06T07:00:00Z"), None)?.entry;
let suffix = recover_unpublished_page_with(
    RecoveryPosition { snapshot_after: first.snapshot_high_water, snapshot_high_water: None, cursor: None },
    /* ... */
).await?;
assert_eq!(suffix.cursor, Some(normal.batch_id));
assert_eq!(store.read_manifest(PROVIDER_KRX, MARKET_KR)?, vec![orphan, normal]);
```

- [ ] **Step 2: Run the collector test and verify RED**

Run:

```powershell
cargo test -p collectors --test research_worker pipeline_recovery_reconciles_orphan_before_later_normal_append -- --exact --nocapture
```

Expected: FAIL because the current synthetic `[N, O]` view completes at `O`
and never emits `N`.

- [ ] **Step 3: Add wished-for Raw storage fault and barrier tests**

In the storage unit tests, add injectable operations that pause immediately
before orphan append, fail before append, or fail manifest sync after append.
Assert that a normal writer blocks during the pause, final JSONL order is
`[O, N]`, a pre-write fault leaves `O` recoverable, and a post-write fault is
deduplicated to one identical line on replay.

- [ ] **Step 4: Run the storage tests and verify RED**

Run:

```powershell
cargo test -p market-data storage::tests::reconciled_manifest -- --nocapture
```

Expected: compile failure because `read_reconciled_manifest_with_ops` and its
fault hooks do not exist.

### Task 2: Implement the durable reconciled manifest API

**Files:**
- Modify: `crates/market-data/src/storage.rs`
- Modify: `data-pipelines/collectors/src/pipeline.rs`

- [ ] **Step 1: Extract an already-locked manifest loader/appender**

Refactor the existing append path into an internal state loader that validates
all complete records, repairs a complete or truncated final tail, and retains
the open manifest file, batch map, and line order. Add an already-locked append
primitive that rejects conflicts, deduplicates identical records, writes one
JSONL record, and updates the in-memory order without locking again.

- [ ] **Step 2: Add the recovery-only reconciled read**

```rust
pub fn read_reconciled_manifest(
    &self,
    provider: &str,
    market: &str,
) -> Result<Vec<ManifestEntry>, StoreError>
```

Acquire `commit.lock` exclusively, load/repair the manifest, discover and
re-sync canonical orphan batches, sort only new orphan IDs by
`(retrieved_at, batch_id)`, append them through the already-locked primitive,
sync the manifest and parent directories, unlock, then return actual JSONL
order. Do not change `read_manifest`.

- [ ] **Step 3: Route recovery through the reconciled API**

Replace only the recovery page call in `pipeline.rs`:

```rust
let mut entries = store
    .read_reconciled_manifest(PROVIDER_KRX, MARKET_KR)
    .map_err(|source| RecoveryError::Pipeline(PipelineError::Manifest { source }))?;
```

Keep `RecoveryPosition`, helper argv, NDJSON, and strict supervisor validation
unchanged.

- [ ] **Step 4: Run RED tests and relevant suites for GREEN**

Run:

```powershell
cargo test -p market-data storage::tests::reconciled_manifest -- --nocapture
cargo test -p collectors --test research_worker pipeline_recovery_reconciles_orphan_before_later_normal_append -- --exact --nocapture
cargo test -p market-data
cargo test -p collectors --lib
```

Expected: all selected tests pass; barrier/fault tests contain no timing sleeps.

### Task 3: Reproduce and close append-only function drift

**Files:**
- Modify: `scripts/qa/research-worker-smoke.ps1`
- Modify: `scripts/qa/research-worker-smoke.sh`
- Modify: `tests/integration/migration-contract/tests/migration_contract.rs`
- Modify: `deploy/compose/research-schema-check.sql`

- [ ] **Step 1: Add static and functional RED tests**

Require `pg_get_functiondef` and exact-definition comparison markers in both
static validators. In each functional mutation sequence, install this
same-name/message-preserving no-op and require the gate to fail:

```sql
CREATE OR REPLACE FUNCTION public.trading_calendar_versions_reject_mutation()
RETURNS trigger LANGUAGE plpgsql AS $fn$
BEGIN
  IF false THEN
    RAISE EXCEPTION
      'trading_calendar_versions is append-only: % is refused', TG_OP
      USING ERRCODE = '55000';
  END IF;
  RETURN NULL;
END
$fn$;
```

Restore the migration body and require the gate to pass. Retain the disabled
trigger mutation.

- [ ] **Step 2: Run static tests and full smoke to verify RED**

Run:

```powershell
pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1 -SelfTest
pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1
```

Expected: static test first fails for missing gate marker; after only adding
markers/mutation, full smoke fails because the old message-substring gate
accepts the no-op function.

- [ ] **Step 3: Capture the exact pinned PostgreSQL definition**

On the disposable migrated Compose database, query:

```sql
SELECT regexp_replace(btrim(pg_get_functiondef(p.oid)), E'\\s+', ' ', 'g')
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname = 'public'
  AND p.proname = 'trading_calendar_versions_reject_mutation';
```

Copy this exact output into the gate's expected singleton relation.

- [ ] **Step 4: Implement bidirectional exact comparison**

Build `actual(definition)` and `expected(definition)` singleton CTEs and fail
if either `actual EXCEPT expected` or `expected EXCEPT actual` returns a row.
Keep language, return type, security-definer, trigger identity/type/enabled
checks.

- [ ] **Step 5: Run schema tests for GREEN**

Run:

```powershell
pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1 -SelfTest
wsl.exe bash -lc "cd /mnt/d/develop/repositories/lagrange/.worktrees/research-metadata-publication && bash scripts/qa/research-worker-smoke.sh --self-test"
$env:DATABASE_URL='postgres://postgres:lagrange@127.0.0.1:55432/postgres'; cargo test -p migration-contract --test migration_contract
```

Expected: both validators and all migration-contract tests pass.

### Task 4: Document, verify, and commit

**Files:**
- Modify: `data-pipelines/collectors/README.md`
- Modify: `docs/STATUS.md`
- Modify: `docs/superpowers/specs/2026-08-11-reconciled-recovery-and-function-gate-design.md`

- [ ] **Step 1: Update operator contracts**

State that recovery high-water IDs identify reconciled, durable JSONL lines;
orphan evidence is appended under the same exclusive commit lock before
exposure; and the gate validates the exact append-only function definition.

- [ ] **Step 2: Run focused and deployment verification**

```powershell
cargo test -p market-data
$env:DATABASE_URL='postgres://postgres:lagrange@127.0.0.1:55432/postgres'; cargo test -p collectors --test research_worker
$env:DATABASE_URL='postgres://postgres:lagrange@127.0.0.1:55432/postgres'; cargo test -p migration-contract --test migration_contract
cargo clippy -p market-data -p collectors -p migration-contract --all-targets -- -D warnings
cargo fmt -p market-data -p collectors -p migration-contract -- --check
pwsh -NoProfile -File scripts/qa/research-worker-smoke.ps1
git diff --check
```

Expected: all focused suites, strict clippy, formatting, full Compose smoke,
and diff checks pass.

- [ ] **Step 3: Commit and verify clean status**

```powershell
git add -- crates/market-data/src/storage.rs data-pipelines/collectors/src/pipeline.rs data-pipelines/collectors/tests/research_worker.rs deploy/compose/research-schema-check.sql scripts/qa/research-worker-smoke.ps1 scripts/qa/research-worker-smoke.sh tests/integration/migration-contract/tests/migration_contract.rs data-pipelines/collectors/README.md docs/STATUS.md docs/superpowers/specs/2026-08-11-reconciled-recovery-and-function-gate-design.md docs/superpowers/plans/2026-08-11-reconciled-recovery-and-function-gate.md
git commit -m "fix research recovery manifest identity"
git status --short
```

Expected: one implementation commit and empty status after the already committed
design document.
