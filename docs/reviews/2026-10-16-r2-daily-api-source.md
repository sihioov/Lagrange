# R2 Wave63 daily API/config/read source review

R2-04/09; WP-4/I5. Baseline eed17d2, whose Wave62 run38804040503 attempt1
is preserved. This report records direct coordinator source review, not an
independent reviewer or executable acceptance. No workers or local project
execution. All 25 new Rust definitions and six Web cases are WRITTEN_UNEXECUTED.

The package adds Owner-only opt-in through the existing config create and new
PATCH update. Missing fields preserve persisted values, explicit null/empty/unknown
fields fail, and the SQL-owned epoch is read-only. CSRF, actor RLS, current Owner
checks, request/resource-bound idempotency and fixed audit evidence apply.
Runtime remains disabled by default and enabled mode requires OwnerOnly access.

The dedicated GET daily-state reads one app-RLS SQL snapshot and one DB clock.
It keeps expected session, eligible input end, result date and publication time
separate. Calendar proof is bounded to 370 civil days, verification to 180 seconds,
and the inclusive 18:30 KST deadline never moves for retry. Ordinary READY data
and manual history cannot produce DAILY CURRENT. Freshness loss preserves the
last publication. Control rights, reservation/queue/config/admission identities,
receipt binding and publication shape fail closed. Calendar source conflict
invalidates the expected date; a BAR blocker preserves the proven calendar date.
A canceled queue claim awaiting drain is failed/canceled state, never queued.

Run detail/list retain historical pins and add historical/daily input variants.
Daily results carry four daily hashes and nested base hashes, omit historical
top-level pins, and publish only after SUCCEEDED. Publication time equals finished_at.
The queue payload is decoded against the immutable row and reserved identities.
Review found daily SQL exclusions store exact zero while historical exclusions
store NULL: validation now distinguishes both and presents the existing HTTP
exclusion shape only after validation. Nonzero or malformed daily exclusions fail.

OpenAPI table/JSON/TypeScript and three existing strict Web decoders are updated.
Legacy historical/config responses remain readable without inventing opt-in.
Daily input/trigger/pins/publication rules are explicit in both list and detail.
The new browser controls and displayed state are still I7 work.

Ten real-HTTP synthetic DB cases cover opt-in/epoch updates, Member/CSRF/null fields,
idempotency/audit, expired sessions/Owner/entitlement/RLS, read-only/no fallback,
fresh/expired/blocked inputs, historical/daily pagination, retry/failure, successful
publication followed by disable/expiry, and exact control rights/calendar gaps.
Successful response bodies are checked against the versioned schemas.
The fixture uses production roles and SQL gates with synthetic evidence; it never
creates approved Raw/filesystem authority. Scratch cleanup is checked after cases.

The hosted PG harness adds a required api-owner-beta-daily target: exactly 15
named rows (ten new plus five carried compatibility cases), exit0 and no SKIP.
Existing 17 targets and prior 0060/bootstrap/rollback/checksum gates remain intact.
The six Web cases emit fixed PASS markers only after their assertions finish.
All actual compilation/lint/DB/Web/OpenAPI results require the new exact candidate CI.

Scoped standalone Rust2024 formatting and git diff --check passed.
All prior frozen pins except the two owned common-harness/PG-harness files are unchanged.
No migrations, roles, workflow definitions, provider paths or dependency files changed.
The HTTP structural/line evidence will be updated with I6 diagrams before main.
Independent integrated review, I7 UI, I6 release/runtime, real approved-base three-UID
execution, G4, Owner use, three trading days and final C_R2 remain open.
No source, compiler, native-runtime or R2 completion is claimed here.

## Source inventory

| Owned path | Lines | SHA256 |
| --- | ---: | --- |
| `crates/api-server/src/http/owner_beta.rs` | 921 | `c9d33b70c4d0d428699832407993d190ce3dbc006ff0b91c68f4c369df1946ed` |
| `crates/api-server/src/http/owner_beta_daily.rs` | 63 | `2ea076dc3796044308acc7d8b41c86c0a85c4d03f547a8368af5f3ea6fe7f6f8` |
| `crates/api-server/src/http/dto.rs` | 874 | `0bf8a9a11841e897e75aec23c04bdc187874382fc345ee63cd77c9b5054c60a2` |
| `crates/api-server/src/http/strategies.rs` | 374 | `93c430421f5fcba415b5b59db3f477245f1de39ac61b8b1f6ac2a35154e50b29` |
| `crates/api-server/src/http/mod.rs` | 770 | `c13fd06eb33c087cf648fa1bb3b3655482b047f39df029c93ce6ac382e0c229b` |
| `crates/api-server/src/http/state.rs` | 591 | `342b8a72468973263825b59bb6986cd2d58394fa392a51e9a7831271b21dc165` |
| `crates/api-server/src/repos/owner_beta.rs` | 1752 | `931b05acab13f14d5153f00f7c48b3016e4a10c704e8affe1bfe263b86fdd338` |
| `crates/api-server/src/repos/owner_beta/daily.rs` | 736 | `c786bc6e5e5f16a22d1e9c21a1929fd6003194403c18298f3dff1eab4db30df2` |
| `crates/api-server/src/repos/strategy_configs.rs` | 156 | `cc46ff26c681bc12083d258a574c40a380622b476b322f3bf0aa1e6c0997c850` |
| `crates/api-server/src/contract.rs` | 1617 | `0827ed060bb3371cbd171e5804b2deb3eca98f8f8591097a9b17fd87b7c26b2d` |
| `crates/api-server/src/runtime.rs` | 2794 | `397f1ecceb3e9d286851a703ce5bd5d35e0120cd2235df76ff00251dcead329e` |
| `crates/api-server/tests/http_owner_beta_daily.rs` | 438 | `44f8bd1ccafece1f59c30f32925c58b70ff4a2c742b4a2100b7014d2538b1813` |
| `crates/api-server/tests/common/mod.rs` | 945 | `e4c5b01a89545635b014881fb66bd487c143fd6f6bea13d66b029cd58c97a7e6` |
| `crates/api-server/src/bin/paper-runner.rs` | 871 | `72657da18e0bf36d2b705426647aa6c809c6db78b014c5ea35b62f82f58b237b` |
| `crates/api-server/tests/owner_market_stream_integrated_support/mod.rs` | 2409 | `30bae2d0a3e7704bbd3e585ca54b03c4377ba431bdd063787e33711aa474ac3e` |
| `crates/api-server/tests/owner_market_stream_http_support/mod.rs` | 2378 | `d62762ec8dbae33c7745b2563d1ae84200784799eef52615b90a2e09af730c01` |
| `apps/api-server/scripts/openapi-spec.mjs` | 3579 | `aedf6eed8df7664ae128a571c209779f8b9ea34f1c9e5636b347802488c883c2` |
| `apps/api-server/openapi.json` | 16235 | `dd9b77d7b750bf9793994248cd0b19db7882858a45a9fae5075d4dfca9d0f6a8` |
| `apps/api-server/generated/openapi.ts` | 5873 | `2e93b9d1e5a1b42279a26715f67bfb0f1a389684a96934a515ab6c755912de52` |
| `deploy/db/integration-validation/validate.sh` | 2028 | `2c7db527fda1034b724f862364db2a31068296f06412819d5604c89c95f36e69` |
| `crates/api-server/tests/owner_beta_daily_support/mod.rs` | 462 | `ce0519e66a452aff09f566f3193fea4a24502a61cb25d55de0ab1bc3d29a7626` |
| `apps/web/lib/products/contracts.ts` | 195 | `25f5ae8e8ebaf6408e1acca8bfc50600ce6b155dcc9f2b2bcf5718d6e71bfeec` |
| `apps/web/lib/products/paper-contracts.ts` | 338 | `3b820702d92df1e6035af03972cc50eb6c2c37a31ea08380b9f035bd841f82ce` |
| `apps/web/lib/products/owner-beta-contracts.ts` | 403 | `a9bc0e6dd7d9cae7bfb080f99aa053a47280eda8358c1acd7d79c11f6879b873` |
| `apps/web/tests/r2-daily-api-contracts.test.ts` | 212 | `a4a59e7f6287c721d4f477556db60020050cf4b515f2a9d515c65412173f26b8` |
