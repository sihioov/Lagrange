# Reconciled Recovery Identity and Exact Function Gate

## Problem

`RawStore::read_manifest` preserves JSONL order and then synthesizes durable
orphan batches as a sorted suffix. A recovery pass can therefore observe an
orphan `O` as its high-water even though `O` has no manifest line. If a normal
batch `N` is appended before the completion check, the next synthetic view is
`[N, O]`; resuming strictly after `O` skips `N`.

The schema gate also accepts the append-only trigger function by metadata and
an error-message substring. A same-name function can retain that substring in
an unreachable branch, return `NULL`, and silently suppress mutations instead
of raising the required error.

## Recovery Design

Keep the public `read_manifest` contract unchanged. Add a recovery-only
reconciled read that acquires the Raw commit lock exclusively and performs one
already-locked transaction-like sequence:

1. read and fully validate existing manifest records, including the existing
   complete-tail repair rules;
2. discover canonical orphan batch metadata and re-sync every evidence file,
   batch metadata file, and containing directory before exposure;
3. sort only the newly discovered orphans deterministically by
   `(retrieved_at, batch_id)`;
4. append each orphan through an already-locked append primitive that validates
   and deduplicates against the manifest state without taking a nested lock;
5. sync the manifest file and its parent directories before returning the
   exact durable JSONL line order.

Recovery pages use only this reconciled API. Consequently `snapshot_after`,
`snapshot_high_water`, and `cursor` remain immutable batch IDs, but their
ordering identity is now an actual durable JSONL line, never a synthetic
suffix position. While orphan `O` is repaired, a normal writer `N` waits on the
same exclusive lock, so the committed order is necessarily `[O, N]`.

If a fault occurs before an orphan line is durable, the batch remains a
discoverable orphan. If the line becomes durable but a later operation reports
failure, replay detects the identical entry and does not append a duplicate.
Conflicting entries remain permanent corruption. Parent/helper cursor and
high-water validation do not change.

## Schema Gate Design

Retain all trigger metadata checks and compare the normalized
`pg_get_functiondef` output against the exact definition emitted by the pinned
PostgreSQL service after migrations. Normalize whitespace only, then use
bidirectional `EXCEPT` over actual and expected singleton definitions so a
same-name or message-preserving body replacement fails closed.

The functional smoke replaces the function with a `RETURN NULL` body that
keeps the original error message in an unreachable branch, asserts that the
gate fails, restores the exact migration definition, and asserts that the gate
passes. Existing disabled-trigger and other drift mutations remain.

## Verification

- RED then GREEN recovery test for orphan `O`, concurrent normal `N`, and final
  `[O, N]` durable manifest order with both batches published.
- Deterministic barrier test proving `N` blocks while `O` is reconciled.
- Fault tests proving no lost orphan and no duplicate manifest line after
  identical replay.
- RED then GREEN full Compose mutation for the no-op append-only function.
- Focused market-data, collectors, migration-contract, static validators,
  strict clippy, formatting, diff checks, and full PowerShell Compose smoke.

