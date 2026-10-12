# R1 Rust lint repair — WP-2e Scope A

**Status: READY_FOR_HEAVY_VALIDATION.** Source/static implementation only. Branch `orange-dugong`, base HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`. This package covers all 74 canonical Scope A groups from the accepted 118-group inventory; Scope B remains separately owned.

## Implementation map

| Contract item | Implemented change and final source spans |
|---|---|
| A1; G023–G056, G078–G080 | `crates/job-queue/tests/owner_market_stream_boundary_support/mod.rs`: 35 exact item `cfg_attr` placements on declaration lines 56, 64, 72, 177, 210, 371, 396, 404, 412, 424, 723, 745, 789, 830, 906, 928, 943, 977, 1018, 1128, 1161, 1176, 1190, 1210, 1231, 1256, 1336, 1347, 1372, 1442, 1467, 1490, 1506, 1538, 1568; direct `connect_supervisor` return at 167; `!failures.is_empty()` at 1074; the unchanged four-test `loopback_lifetime_tests` module moved to 1936–2029. |
| A2; G081 | `crates/job-queue/src/bin/owner-equity-v2-runner.rs:1353–1363` — one variant-scoped C-REST expectation on `QuoteBackend::Rest`. |
| A3; G082–G087 | `crates/job-queue/src/owner_equity_v2/market_stream.rs:123–145, 628–629, 1315–1318, 3340–3345, 3432–3441, 5971–5979` — exactly seven private DB-test cfg gates. |
| A4; G088–G090 | `crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs:2512–2527, 2806–2811` — local row alias and two direct i64 timestamp conversions; the u64 ordinal conversion remains. |
| A5; G091–G095 | `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs:126–151, 876–940, 2356–2364, 2935, 5690–5712` — DB-only test controls, redundant borrow/assert simplifications, and private snapshot alias. |
| A6; G096 | `crates/job-queue/src/owner_equity_v2/market_stream_producer_test_support.rs:1191–1200, 1283–1292` — the captured clock origin/anchor travel together in a tuple and are destructured at entry. |
| A7; G097–G104 | `crates/job-queue/src/owner_equity_v2/market_stream_producer_tests.rs:311–323, 1254–1262, 1685, 1996, 2303, 3569, 4088` — only the three named let-chains, two `expect_err` changes, matching boxed error return, direct relay result, and boolean assertion. |
| A8; G105 | `crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs:214–216` — production-only cfg on the private window loader. |
| A9; G106–G108 | `crates/job-queue/src/owner_equity_v2/market_stream_runtime_storage_tests.rs:1071–1072, 1348–1360, 1602–1615` — local aliases for the exact existing SQL row tuples. |
| A10; G109–G112 | `crates/job-queue/src/owner_equity_v2/market_stream_runtime_test_support.rs:134–148, 197–199, 228, 285, 907–916, 1218–1222` — only named observation fields are underscored; timestamp/counter/digest calculations remain; `header_value` returns the same stripped slice directly. |
| A11; G113 | `crates/job-queue/src/owner_equity_v2/market_stream_runtime_tests.rs:275–280` — cleanup let-chain retains `take`, await, deadline, and first-error behavior. |
| A12; G114 | `crates/job-queue/tests/owner_market_stream_boundary.rs` — removed only the six named unused imports formerly at lines 6–9; DTO test and assertions remain. |
| A13; G115–G117 | `crates/kis-client/src/market_stream_approval.rs:508`; `crates/kis-client/src/market_stream_provisioning.rs:405, 556` — exact assertion/reference simplifications. |
| A14; G065–G067, G118 | `crates/kis-client/src/market_stream_wire.rs:211–223, 293–301, 983–987` — two exact C-WIRE item exceptions and direct `ParseContext` initialization; public error variants and existing cfg remain. |

Approved scoping decisions were applied exactly: C-SHARED has 35 item-level `cfg_attr(all(test, feature = "market-stream-db-tests"), allow(dead_code, reason = "shared fixture item is used by job-queue tests; API fixture targets compile independently"))` placements; C-REST has one non-test `expect(dead_code)` on `QuoteBackend::Rest` with the approved reason; C-WIRE has only the two approved item-scoped test allowances with their exact reasons. No module/impl/crate suppression or additional lint exception was introduced. No SQL, feature defaults, manifests, configs, public/wire signatures, or C2 fixture changes were made.

The 15 Scope A input files were SHA-256 checked against the contract before editing; every hash matched. Final content hashes follow.

| Scope A source file | Final SHA-256 |
|---|---|
| `crates/job-queue/src/bin/owner-equity-v2-runner.rs` | `f21d135c11a7cbfd2180aef4067bfd8c2e1f7da0db75c78f28213069f9fdd3da` |
| `crates/job-queue/src/owner_equity_v2/market_stream.rs` | `11f1e2d7f1e95ef98f4092760710ccb6a6ce6ef0cc692140e2ae8104d0a9e84c` |
| `crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs` | `af67896c3d9c51a761b155e9f01710db0120e7dcfb3742a94f50b365994d2aed` |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs` | `ba394ab4f8b6b5d5eea8f0fc43bc0c6d3814121643afa5e7923c8ce3efede7e8` |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer_test_support.rs` | `8695e035c73d8b0096f1e509e636a4bfa25c23f5e9a9f834cbc8fbb8410c7660` |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer_tests.rs` | `b966c213df98bbd060c2f67a80b383cccbfb961d524eca7800706fedc24ef96a` |
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs` | `7d0eddfbcbec2a03901e402ac95c44c95a00e44f3a6c689fd3f99e12d9444984` |
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime_storage_tests.rs` | `e446a1f1f43bb6515a388bd84debb01b8f2c0685a69f2ac741797b81fb05aa07` |
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime_test_support.rs` | `1d3f97c7cb5536b9cfb09f8e5c027937c03e6344df990a27bc525c3aa073790a` |
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime_tests.rs` | `9b09e01718b8a5d945aae302945cbafe6dfe27a9fc7d256321f451a72851169c` |
| `crates/job-queue/tests/owner_market_stream_boundary.rs` | `517c61a2e7bfd7d02db94cd7b5bfc69b114718287b0e7494e9b047f12e71fbdf` |
| `crates/job-queue/tests/owner_market_stream_boundary_support/mod.rs` | `a438aa74bb4bffbacead26cc67a22a6684d80aa7e0b6bd544d2cec88a217d1ed` |
| `crates/kis-client/src/market_stream_approval.rs` | `02a93dc56bbe4bdd8382f2b4d4d9e3809a0be8dd71f2ff1eecd377647df5ff04` |
| `crates/kis-client/src/market_stream_provisioning.rs` | `a1fce11c93dbc8af474f8eba3788e55257842c8d9e5e6f79578a4d01eb057f08` |
| `crates/kis-client/src/market_stream_wire.rs` | `1dff19badfbbc05225cfdf861ad8a491b14a7b932187e52a50edc5d15a7aa476` |

## Source/static checks

- Executed the 12 caller/inclusion `rg -n` searches verbatim from the contract’s “Reproducible caller and inclusion inventory,” from the repository root. All exited 0. Match-line counts by search order: 54, 98, 823, 31, 77, 83, 41, 64, 80, 75, 57, 224. Outputs are in the ignored private evidence directory `target/r1-runtime-fxi6ltmv/wp2e-scope-a-ap2wts8k/`. Scope-relevant inclusion results confirm the two API boundary-support inclusions, the job-queue source inclusion, and the API reviewed wire source inclusion specified by the contract; the broad `.session`/`.encode` searches also include unrelated symbols, which were not edited.
- Pre-edit input hash check: all 15 assigned Scope A files matched the accepted contract hashes; no input drift was found.
- Formatter: `rustfmt --edition 2024 --config skip_children=true` run directly on exactly the 15 owned files; completed successfully.
- Formatter check: the same 15 paths with `rustfmt --check --edition 2024 --config skip_children=true`; exit 0.
- Whitespace check: `git diff --check` on the 15 owned source paths; exit 0. The final check including this report also passed after the report write.
- Static assertions passed: 35 shared allowances; lifetime test module at file end with all four original tests; seven A3 cfg gates; five A5 methods plus test-gate impl restricted as specified; A6 clock tuple; A10 observation names; exact C-REST/C-WIRE reasons; A12 import list removal.
- Branch/HEAD check: `orange-dugong` / `ac0a08364788e6c2f234cf85025a8d975f362d31`.

## Deviations and pending verification

- Deviations from A1–A14 and the approved C-SHARED/C-REST/C-WIRE placements: none.
- Scope B sources were not edited by this package. No staging or commit was performed.
- The repository `AGENTS.md` ordinarily requires a roadmap update for advancing work. The task’s closed write allowlist explicitly excludes the roadmap, so it remains untouched in this package.
- Not run: Cargo, rustc, Clippy, tests, build, database, Docker, npm, Web, provider, network, runtime, or strict acceptance. WP-4c owns the heavy validation slot.
- Not verified pending WP-4c: compilation of cfg-gated branches and source-included copies, lint elimination under the strict matrix, tests, and runtime/database behavior. In particular, compile proof must confirm the C-WIRE `unexpected_cfgs` item attribute covers the source-included API target; if not, return to the coordinator without widening it.
- Not found: input hash drift, contradictory A1–A14 instructions, or an additional caller/inclusion requiring source changes. The accepted contract’s source-backed closures matched this snapshot.
- Further decomposition needed: no. Heavy validation remains assigned separately.
