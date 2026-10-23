# R2 snapshot second correction handoff — Wave27

R2-04/09 remain DOING. The second source correction is recovered for independent
review. No source, compiler, Clippy, test, platform or CI acceptance is claimed.

## Recovered worker and exact bytes

Implementer `00e0a229-8710-4ed5-8dde-599cb7a7d84f` (Sol/max) started
2026-10-14T13:47:13.127Z and became idle at14:50:24.974UTC, within its90-minute
bound. At14:50:48UTC the coordinator confirmed idle, activeTurn null and pending0,
and recovered the full substantive final through Paseo activity. The truncated
notification was not treated as the result. The176-line final is preserved privately:
`f52a070c927450b2adc9eef07c75d9b74052551ac355f887dd9f815ee2f2f2bf`.
Final/recovery receipts were saved before heartbeat1f6d4170 was deleted14:52:14UTC.

| File | Final lines | SHA256 |
| --- | ---: | --- |
| [source.rs](../../crates/market-data/src/daily_price_only/source.rs) | 4413 | `0a6c29c7fff0bdbc48c5af0f1950c8522542270e43d374b9ee8ae6243b2729d3` |
| [snapshot tests](../../crates/market-data/tests/daily_price_only_snapshot.rs) | 1970 | `a5db9e0adc4ca909835be3a717c5579a8c1b356f7c1b7036f4dc6cae8cbb7f61` |
| [registration](../../crates/market-data/src/daily_price_only/mod.rs) | 9 | `700469d6a510aa3ae7ad6a27fb42a78a76b243256e5c31cb5292104b4a6be74a` |

Only the first two files changed from baseline
`7545094404628974c1bd626c84f6c3f704555f5f`; no coordinator source correction was
added. Plain diff confirms source+694/-268 and tests+159/-85. Git's different
hunk alignment reports source+692/-266 with identical final bytes. The1749-line
Git delta is preserved as `wave27-S2-second-correction.diff`,
SHA256 `ec132c244bf22e22bcd06f6a749014a33d7305b8760506a768a4f45b92c2a19c`.
The exact committed handoff is recorded in the subsequent frozen review inventory.

## Inspected changes and independent questions

| Finding | Inspected source change; independent disposition still required |
| --- | --- |
| FR01/02 | Fresh ScanLimits per scan at [S3414](../../crates/market-data/src/daily_price_only/source.rs#L3414), and opaque parse error mapped to a fieldless error at [S3600](../../crates/market-data/src/daily_price_only/source.rs#L3600). No capability traits or payload Debug added. |
| FR03 | Reverse selection at [S3774](../../crates/market-data/src/daily_price_only/source.rs#L3774) replaces double-ended last; strict Clippy remains unexecuted. |
| FR04 | Private real file owner at [S541](../../crates/market-data/src/daily_price_only/source.rs#L541) closes File before unique-lifetime observation. Five production open sites wrap before later checks. The lock transfers into the construction guard before validation at [S1532](../../crates/market-data/src/daily_price_only/source.rs#L1532). Shared-inode/scope-exit test at [S3249](../../crates/market-data/src/daily_price_only/source.rs#L3249). Review all early failures, relocated AfterLockOpen semantics, test observer borrowing and native-evidence limits. |
| FR05 | Malformed and genuine EOF cases are separate; permissive is_eof and finite-cap/no-open assertions at [S3414](../../crates/market-data/src/daily_price_only/source.rs#L3414). Independent total-file/batch controls at [S3496](../../crates/market-data/src/daily_price_only/source.rs#L3496) and [S3549](../../crates/market-data/src/daily_price_only/source.rs#L3549). |
| FR06 | [I1686](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1686) bounds all receives, preserves actual WouldBlock, probes post-Drop release before append and avoids joining a worker without completion. Timeout/error cleanup still needs independent review. |
| FR07 | [I398](../../crates/market-data/tests/daily_price_only_snapshot.rs#L398) checks every fixture-derived batch/file view, ordering, lengths, association and exact commitments. |
| FR08 | [S1852](../../crates/market-data/src/daily_price_only/source.rs#L1852) accepts a phase-specific missing error; final body/metadata move-outside regressions at [S4123](../../crates/market-data/src/daily_price_only/source.rs#L4123). Initial missing/extra errors remain. |
| FR09 | Full parsed metadata Eq has a short lexical scope ending [S1775](../../crates/market-data/src/daily_price_only/source.rs#L1775), before body-bearing capture. This is static lifetime evidence, not runtime allocation proof. |
| E01 | Final names/lines regenerated from corrected source in source-wave-28-S2-test-inventory.json; old incorrect and corrected-baseline inventories preserved. All execution states remain WRITTEN_UNEXECUTED. |

Original R01-R16 including R12a-h, C01, P1-P10 and T01-T32 remain review obligations.
The worker's resolved labels are not adopted as an independent acceptance result.
The two prior reviews and accepted898-line contract remain immutable.

## Coordinator verification and limits

All46 non-target reference/artifact pins matched; both supplied baseline copies
matched their committed blobs. The complete two-file delta was read. Eight regions
are byte-identical: public API/getters, errors/name policy, fingerprints/name pool,
construction/final-lock layout, root/preflight, size/preflight, complete Raw parser
and C01. Scoped rustfmt check and whitespace diff check returned0.

All45 baseline test names remain. Four private cases were added: same-inode owner
lifetimes, total-file preflight, unique-batch preflight and final captured-name loss.
There are49 definitions:19 private Linux,29 Linux integration and1 non-Linux.
All are WRITTEN_UNEXECUTED; the non-Linux case is excluded from Linux expectations.
No compiler, Clippy, parser, test, platform, ACL or runtime checks were executed.

Private0600 final/recovery/verification/diff/inventory artifacts live under
target/r2-daily-beta/wp-4-integration/. The reported worker scope has no deviations,
missing/conflicting inputs, background work, pending permissions or auto-review
denials. Required memory receipts are operational bookkeeping; the current
coordinator context read returned MCP_ERROR, not a save, and was not retried.

[Wave28](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md) defines
one independent bounded Sol/high review. Source bytes are frozen before launch.
No new S2 push or CI has occurred. Only review closure followed by an existing-run
query permits one new exact-candidate hosted CI. Prior a888cd6/run38545976070
remains I0c-only partial evidence; nested helper rows/exits remain UNKNOWN.
Preserve prior successes and failures. Negative bootstrap/rollback,0060,
provider/copy reconstruction/admission,I6 diagrams before main,runtime/Owner and
final C_R2 remain open.
