# R2 S2 correction handoff — 2026-10-14

WP-4/I1b-S2-F addresses R2-04/09. This is a source handoff for independent review;
S2 source, compilation, lint, tests and runtime acceptance remain withheld.

The correction worker `3591c5d4-6111-4961-9423-c4f6ab1c6f60` ran with
codex/gpt-6-luna/max from `2026-10-14T11:44:10.164Z`. Actual idle, activeTurn null
and pending0 were recovered at `12:22:17UTC`; the snapshot was updated at
`12:22:07.583UTC`, within the original90-minute reporting bound. The callback
contained only Mem0 status, so the coordinator recovered the full substantive
source report from activity before deleting heartbeat `57665dd0`.

## Frozen correction inputs and delivered bytes

Baseline: `411326865706ea9e7a1690aa49742cd0254d8450`.
The [496-line review](2026-10-14-r2-raw-snapshot-source-review.md) and
[898-line accepted contract](2026-10-14-r2-raw-snapshot-implementation-contract.md)
remain unchanged. All20 readonly references and8 evidence artifacts matched the
Wave25 inventory; the two original source blobs also matched their starting pins.
The exact worktree delta contained only the two authorized source/test files.

| Artifact | Lines | SHA-256 |
| --- | ---: | --- |
| Worker source delivery, preserved in report |3986|`c3233eb434bcc4af4769e31601e7cbc61fc0c21dd52c152aaf79389bc10d3a80`|
| Current [source.rs](../../crates/market-data/src/daily_price_only/source.rs), after C01 below |3987|`4577d5f10bbe4dda508c89b25ff04c638a274bc5050e5f9e9b7e03b5ba4d8bb5`|
| [Snapshot test](../../crates/market-data/tests/daily_price_only_snapshot.rs) |1896|`754a32ec22ba32f7b6c57da9e3dfb335cdeb064926ecedd1591f4ff3c74d9dd7`|
| Existing [registration](../../crates/market-data/src/daily_price_only/mod.rs), unchanged |9|`700469d6a510aa3ae7ad6a27fb42a78a76b243256e5c31cb5292104b4a6be74a`|

The worker delta was +1795/-382 lines. The coordinator's C01 adds one net line;
the frozen review delta is +1796/-382. No public API, dependency or platform
allowance was expanded, and no provider/runtime operation was performed.

## Written correction groups awaiting independent disposition

| Original groups | Written changes inspected by coordinator |
| --- | --- |
| R01–R04 | Linux entry visibility, pending-vector type, iterator capture, integration imports/shadowing/error-source assertions |
| R05 | Private context grouping and slices, named states, used crate-only getters, checked stat conversions and Linux-only pending state |
| R06 | Duplicate known-field checks before consuming the second value; incomplete-tail and legacy sequence comparisons |
| R07–R08 | Partial-construction resource owner with lock last; separate lock identity/type/UID/mode/link fingerprint |
| R09–R11 | 64MiB policy oracle, replacement fixture outside the leaf set, actual WouldBlock checks, scoped FD observations and restored valid root-limit controls |
| R12 | Additional bounded-read, materialization, resource-order, symlink, metadata-mode, final-name, partition-date and cooperating-writer assertions |
| R13–R16 | Stat-size mapping, full lexical root validation before copying/opening, recorded-size preflight and explicit Raw-down fixture directory modes |

These are inspected source changes, not a declaration that all findings are
resolved. The independent reviewer must disposition every R01–R16, P1–P10 and
S2-T01–32 requirement against the actual assertions and production paths.

### C01 — coordinator correction after worker idle

The new same-lock-inode test moved its `PathBuf` into a phase-hook closure and
then borrowed it for the post-hook metadata check. The coordinator kept the
original path and moved a cloned `hook_path` into the closure. The change is
limited to that test; the worker's full report and original reported hash remain
preserved. This is a static ownership correction, not an executed compiler result.

The next review also examines strict-warning exposure in new test iterators,
resource-failing EOF-tail fixture shape, duplicate invalid-value coverage, and
whether raw-FD/device/inode observations remain deterministic when parallel tests
reuse descriptors for shared ancestor directories. These are review questions;
no compiler, test or platform result is inferred.

## Verification and remaining gates

Coordinator checks passed: exact delta inspection,28 unchanged pins,2 baseline
blob pins,94 worker-report links/line bounds, scoped rustfmt check and whitespace
diff check. An initial bookkeeping script assumed list-shaped inventory rows and
exited with KeyError; the corrected mapping-shaped read completed all pin checks.

There are45 test definitions:15 private Linux tests,29 Linux integration tests and
one non-Linux test. All are WRITTEN_UNEXECUTED. The non-Linux case is excluded from
Linux CI expectations, not passed. The previous34-definition inventory is preserved.

Private0600 receipts under `target/r2-daily-beta/wp-4-integration/` preserve the
full78-line final (`2e3b32b5…`), actual lifecycle, coordinator verification
(`5d71a98b…`), baseline blobs,3156-line exact diff (`d69c90f2…`) and new test
inventory (`2902947e…`). No raw operational logs or credentials were persisted.

[Wave26](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md) freezes
these inputs for one bounded independent review. No new hosted CI was dispatched.
After finding disposition, the coordinator queries existing runs before one new
exact-candidate hosted run. Earlier a888cd6 success remains partial I0c evidence
and excludes all S2 source/tests. Old successes and failures remain preserved.

Negative bootstrap/rollback,0060, provider/copy reconstruction and admission,
I6 diagrams before main, runtime/Owner acceptance and final C_R2 remain open.
Mem0 context returned MCP_ERROR; the worker's later callback reported SKIPPED.
Neither constitutes a durable memory save, and no stale hook was retried.
