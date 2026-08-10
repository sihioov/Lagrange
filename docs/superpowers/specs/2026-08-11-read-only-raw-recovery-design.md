# Read-only Raw Recovery Design

## Problem

`research-raw-init` intentionally makes immutable evidence and `batch.json`
owner-readable (`0440`) while leaving only `manifest.jsonl` and `commit.lock`
owner-writable (`0640`). Orphan recovery currently opens every file for both
read and write before syncing it, so the unprivileged worker cannot recover an
otherwise valid pre-existing orphan on Linux.

## Design

- On Unix, file durability checks open existing evidence read-only and call
  `sync_all`; the pinned Alpine/Linux runtime supports `fsync` on this handle.
- On Windows and other platforms, retain the existing read/write open because
  Windows flush-handle access differs and existing behavior must remain stable.
- Keep immutable files at `0440`; keep only the manifest and commit lock at
  `0640`.
- Run the Raw initializer with all capabilities dropped and add back exactly
  `CHOWN`, `FOWNER`, and `DAC_OVERRIDE`. It remains root only for recursive
  ownership preparation, without network or secrets, and never follows
  symlinks or crosses filesystems.

## Verification

- A tracked one-file Rust probe calls `File::open(...).sync_all()` as UID 10001
  on a `0440` file in a named volume after the initializer runs. Its compiled
  artifact is transient and the probe source stays outside the worker build
  context.
- The full functional smoke removes the manually ingested manifest row after
  initialization, then requires real worker startup recovery to re-sync the
  `0440` evidence and `batch.json`, restore the exact manifest row, and publish
  the batch.
- Static validator mutations reject capability drift, a write-opening probe,
  and accidental inclusion of the QA probe in the worker build context.
