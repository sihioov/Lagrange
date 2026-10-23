# R2 snapshot C03 compilation corrections

The continuation request authorizes the bounded corrections identified in the
[Wave30 failure recovery](2026-10-15-r2-snapshot-ci-recovery.md). Roadmap tasks
R2-04/R2-09 remain DOING. The coordinator performs this one-file change and its
verification directly; no further worker delegation or local executable check.

## Source disposition

Baseline is failed candidate `245f4d9a911d03d58936f177904588826031d83e`;
coordinator documentation baseline is `70bd6ec0c31bf83f623c7fbc267b2368de1116bd`.
The source delta is exactly three hunks in
[source.rs](../../crates/market-data/src/daily_price_only/source.rs).

| Finding | Correction | Preserved condition |
| --- | --- | --- |
| C03-01: E0599 at 918 and 926 | Use `Errno::ACCESS`, the constant defined by the pinned rustix 1.1.4 Linux/libc backends. | Both `PERM` branches, missing/unsafe classifications and opaque `PermissionDenied`/`Io` errors remain unchanged. |
| C03-02: E0425 at 3839 | Count `Component::Normal` values directly from the private fixture's absolute data-root path at 3837–3841. | The assertion at 3842 still requires exactly `C + 7` opened descriptors; it does not call the production root parser or derive the expectation from observed descriptor totals. Pre-open, metadata/provider exclusion, real close and lock-last assertions remain intact. |

For C03-02, C excludes the absolute root marker. The seven additional owners are
`/`, `raw`, the three manifest directories, and the existing lock/manifest files.
The fixture construction and production preflight graph were read directly.
No public API, limits, lock ownership, permissions, dependencies or test names
changed. The existing independent Wave28/C02 reports remain historical evidence;
this narrow correction received a direct coordinator static review, not a new
independent worker review or executable acceptance.

## Static verification

- The current source equals the baseline after only the two errno identifier
  substitutions and the single private assertion-oracle substitution.
- The other 50 Wave30 source/reference/artifact pins match. Earlier reports,
  receipts, failed runs and partial successes are preserved unchanged.
- All 49 S2 definitions are retained: 19 private Linux, 29 integration Linux and
  one non-Linux exclusion. The complete hosted inventory still contains 99 Linux
  Rust names, six separate chart names and two platform exclusions.
- Standalone `rustfmt --edition 2024 --check` on this source and Git whitespace
  checks pass. No Cargo, rustc, Clippy, tests, project scripts, SQL/DB or Docker
  executed locally. No additional test was needed for these bounded repairs.

Corrected source: 4,447 lines, SHA256
`629a68e6d49e1fbf63380ed12dc858e75727cbbb624113bcb9a64eab9828e092`.
Unchanged integration test: 1,972 lines, SHA256
`765e62b8101d2d5fa3440148332bbc69f5c2169a802921f6a2d28fbcb1773875`.
Private source-check receipt:
`target/r2-daily-beta/wp-4-integration/wave31-C03-source-checks.json`
(338 lines, SHA256
`457104c6b8b08bbbe0e6ef14b43aecaee014b4ca8d1f64be9967580e79b491d4`).

## Hosted verification gate

Commit and pin this exact candidate, confirm the feature-branch remote, and query
all events for existing CI runs at that SHA. Reuse an existing run; otherwise
issue one `workflow_dispatch`. No previous run is rerun, cancelled or duplicated.
Collect all jobs/steps, actual checkout SHAs, strict lint, named Rust/chart rows,
PG migrations/role/helper/cleanup evidence and explicit unprinted limits.
Source/compiler/test acceptance stays pending those outcomes. Negative bootstrap/
rollback, migration 0060, provider/copy reconstruction/admission, diagrams before
main, runtime, Owner and final C_R2 remain open.
