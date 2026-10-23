# R2 I4g3 complete runtime connection — source disposition

2026-10-16 KST; R2-04/09, Wave62. Baseline
`9d0d4591708cc618b9d050d2e223d5eea8b15b64`; candidate recorded by the successor
launch receipt. Direct coordinator review, **not an independent review**.
Source, compiler, test, runtime and final R2 acceptance remain open.

The executable now connects the existing bounded Raw/approved-v3 machinery to
producer, direct-admitter and mixed-worker loops. Daily mode remains OFF by
default. No dependency, migration, provider, database privilege or workflow change.

| Owned path (relative to repository) | Lines | SHA256 |
| --- | ---: | --- |
| `crates/job-queue/src/owner_beta/runner.rs` | 671 | `d1abf7f121d65d8c6c98dfe67d10f41c5da516b930ddf5a14cab1e63cc3e8ee0` |
| `crates/job-queue/src/owner_beta/runner/daily_local_state.rs` | 957 | `3eca1dd12bd3039286e33ed2410eeb003cac72b92a854ae81339052e4b6b533f` |
| `crates/job-queue/src/owner_beta/runner/daily_catalog.rs` | 363 | `0a06f77353976081b21e936b13fd50bc91dfe0f67eebf1de30ec4cbcaa350be9` |
| `crates/job-queue/src/owner_beta/runner/daily_ingest.rs` | 1230 | `b9fa6131824cd56e655c966cfed4af5a2fa558e27cdd4a41f62cb67f13b7bc8a` |
| `crates/job-queue/src/owner_beta/runner/daily_admission.rs` | 820 | `8e1cbb520e34ebb6f655bdf57db6ec3e2e2b16938adbd0035068f5e8ed09d72e` |
| `crates/job-queue/src/bin/owner-beta-runner.rs` | 578 | `b190b4770caa3cfdfb17324b12648f088ee822d15023c5ac2deccbd2388a23dd` |
| `crates/job-queue/src/bin/owner-beta-runner/daily.rs` | 403 | `0000f55825941be77120f852b1ecb0fe51b4670257e698c7d60acdbc280651b8` |
| `crates/job-queue/tests/owner_beta_daily_schedule.rs` | 1083 | `1f7a159fbe7b7b52a4487e7ad0ab54b0af8b6528438458ce7d70abb3ea13bfdf` |
| `crates/job-queue/tests/owner_beta_daily_runner.rs` | 1118 | `1bbb308293f7088907b0d4e293b92d1423b37744ddc141b07b2a3f7b192eca0c` |
| `deploy/db/integration-validation/validate.sh` | 2004 | `03e22662b8cdaf8aebf13380ad5d71fbcd4b0c937a147dfc05939b2dcaa44da5` |

## Reviewed behavior

- Private state: one full-cycle nonblocking flock; durable minute slot before work;
  owner/policy/stage binding; exact candidate content/manifest and receipt hints.
  Missing/corrupt/foreign checkpoints and clock rollback cannot become first use.
  Complete exact-next temporary state can recover; partial/disconnected bytes stay
  preserved and fail closed, requiring explicit operator repair. No reset fallback.
  See [state boundary](../../crates/job-queue/src/owner_beta/runner/daily_local_state.rs#L151).
- Descriptor-held guarded ancestors, exact UID/GID/modes, no ACL/symlink/hardlink,
  fsync/rename/root-fsync persistence; sole lock owner closes last on early failure
  and normal completion. Private state is 0700/0600, separate from shared admission.
- [Catalog](../../crates/job-queue/src/owner_beta/runner/daily_catalog.rs#L34)
  is unapproved navigation only. Single complete parent chain, no fork/cycle or
  duplicate binding, at most one incomplete admission and one candidate. Every
  selected parent/input still requires the existing full sealed replay capability.
- [Pinned process config](../../crates/job-queue/src/owner_beta/runner/daily_ingest.rs#L182)
  bounds bytes, names, replay, scan resources and elapsed cycle time; roots are
  canonical absolute and pairwise disjoint. Only the real approved-v3 factory is
  used. Existing KIS and calendar Raw scopes are read; no provider request is added.
- Producer persists exact intent before publication, reconstructs continuously
  proven eligible sessions in order, one day per cycle, and recovers using the
  original code commit. It waits for an incomplete frontier instead of replacing it.
- Admitter holds direct-role full-cycle SQL and local guards, reads context and
  exact old receipt, replaces the receipt hint atomically, independently reopens
  fresh Raw and physical sealed parents, and reconciles at most one physical orphan.
  A completed historical receipt never becomes current admission authority.
  Unknown commit status retains the hint; an unresolved old hint is not cleared.
- [Typed source event completion](../../crates/job-queue/src/owner_beta/runner/daily_admission.rs#L301)
  compares returned SQL event identity/code before commit. Foreign-owner mismatch
  rolls back. Operator revocation remains outside this method. Existing SQL intact.
- [CLI](../../crates/job-queue/src/bin/owner-beta-runner/daily.rs#L147): OFF exits before
  protected IO/DB; producer UID10002 has no DB environment, admitter UID10003 uses
  only its direct credential, mixed worker UID10001 reads installed pins. Foreground
  cycles, finite diagnostics, SIGTERM flag and joined shutdown; no detached writer.
  Cancellation/time budget are checked between bounded operations, not hard preemption.
- Mixed worker retains the historical OFF path and uses one minute scheduler plus
  existing typed daily execution when enabled; no interval backlog of recommendations.
- The previous failing supersession DOWN/UP test now compares the complete ordered
  successful migration version set with embedded UP migrations. Populated-row,
  privilege and rollback assertions remain. Full release has62 UP versions; the
  intentionally staged0060 harness remains60.

## Written verification and limits

23 new definitions:9 Linux state,4 catalog,4 ingest (1 Linux),2 CLI and4 actual-DB
source-event tests. All are **WRITTEN_UNEXECUTED**. Dedicated PG schedule inventory
becomes32; daily-runner remains24; all17 dedicated targets retain exit/no-SKIP gates.
The successor inventory carries all prior named groups, including18 readback tests.
Counts are not substituted for individually recovered results.

Scoped standalone Rust2024 formatting and whitespace checks pass. Source signatures,
ownership, guards, receipt/error flow, registered file scope and old function regions
were reviewed directly. No local Cargo/rustc/Clippy/test/build/project helper/SQL,
provider/runtime execution or worker delegation occurred. Hosted verification is pending.

These tests do not demonstrate the complete real approved-v3 + Raw + three-UID
runtime, injected process crashes/transport loss, installer rollback or Owner use.
Installer must provision valid initial private state, primary UID groups and shared
reader-group membership; this source does not initialize or reset production state.
I5–I7, independent final review, I6 diagrams before main, G4 operational evidence,
Owner use, three trading days and final C_R2 remain open. Ralph continues.
