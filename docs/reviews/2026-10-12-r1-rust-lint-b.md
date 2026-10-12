# R1 Rust lint repair — Scope B

- Date: 2026-10-12
- Base: `orange-dugong` at `ac0a08364788e6c2f234cf85025a8d975f362d31`
- Status: **READY_FOR_HEAVY_VALIDATION** — source and static checks only; strict lint and runtime acceptance remain pending.

All eight Scope B source hashes matched the corresponding entries in
[`2026-10-12-r1-rust-lint-contract.md`](2026-10-12-r1-rust-lint-contract.md) before editing. The source edits cover all 38 canonical Scope B diagnostic groups. SQL text and bindings, handler/wire APIs, status/error/body/request-ID values, test names/assertions, feature gates, and caller awaits were preserved.

## Repairs

- **B1 — G001–G003:** Boxed only the private `configuration` and `key` error payloads, dereferencing them at their existing HTTP returns. Added `StreamDriveIdentity` for the exact existing state and identity clones passed to the stream driver.
- **B2 — G004–G006, G057–G058:** Added `DeliveryFlags` for the final two encoder booleans and converted the production and all eleven source-test calls with the same values and time-read placement. Kept saturating consumer cleanup while collapsing its nested checks; gated test-only `pending` with `cfg(test)`.
- **B3 — G007–G011:** Applied the specified `set_actor_guc` autoderef, exact `StreamPinsRow` alias, `actor_uuid(actor).ok()`, redundant binding removal, and owner `if let`. SQL and the denied/no-read path are unchanged.
- **B4 — G012–G013:** Removed only the two unnecessary `return` keywords in the tail match.
- **B5 — G014–G022:** Applied the named let-chains and `contains`, replaced the identical policy-mismatch literal branches, and added the exact private `EodRuntimeInputs` and `ScheduledFact` types. Kept the existing evaluation order, time reads, return tuple values, and reason/validity precedence.
- **B6 — G059–G064:** Removed the unused `Role` import and unit binding, collapsed the named SSE check, added `FixtureRequest` and converted all eleven `send` calls, and added `ActorSessionContext` for all six binding checks. Actual actor/session GUC values remain distinct from desired owner/session values; temporary formatted paths remain borrowed through each awaited call.
- **B7 — G068–G076:** Removed only the unreachable runtime-exit wildcard, applied the five cleanup let-chains, removed `mut` from the read-only file binding, changed publication-failure initialization to definite assignment, and chained the final failure check. Cleanup order, deadlines, first-failure handling, and publication precedence are preserved.
- **B8 — G077:** Converted `audit_rows_for_actor` to `async fn`; callers still await it.

## Changed source files and spans

| File | Edited regions |
|---|---|
| [`owner_market_stream.rs`](../../crates/api-server/src/http/owner_market_stream.rs) | 27–30, 87–181, 226–303, 571–734 |
| [`owner_market_stream_delivery.rs`](../../crates/api-server/src/http/owner_market_stream_delivery.rs) | 50–61, 97–101, 151–210, 310–642 |
| [`service_status.rs` repository](../../crates/api-server/src/repos/service_status.rs) | 373, 773–788, 899–1008 |
| [`runtime.rs`](../../crates/api-server/src/runtime.rs) | 303–307 |
| [`service_status.rs` projection](../../crates/api-server/src/service_status.rs) | 1030–1035, 1572–1590, 1619–1630, 1770–1774, 2170–2198, 3880–3904, 4218–4277, 4351–4374, 4546–4553 |
| [`owner_market_stream_http_support/mod.rs`](../../crates/api-server/tests/owner_market_stream_http_support/mod.rs) | 14, 519–547, 621–728, 772–778, 847–1114, 1253–1834, 1909–1924 |
| [`owner_market_stream_integrated_support/mod.rs`](../../crates/api-server/tests/owner_market_stream_integrated_support/mod.rs) | 258–280, 997–1080, 1136–1209 |
| [`service_status_support/mod.rs`](../../crates/api-server/tests/service_status_support/mod.rs) | 274–287 |

The delivery and integrated-support files already had accepted formatting-only worktree edits when this task began; those edits were retained. Other owners' dirty files remain outside Scope B. No shared Scope A or `kis-client` source was edited.

## Static checks

- Pre-edit `sha256sum` over the eight owned sources: all eight matched the contract table.
- `rustfmt --edition 2024 --config skip_children=true` on only the eight owned files: exit 0.
- The same direct `rustfmt` command with `--check`: exit 0.
- `git diff --check` scoped to the eight owned files: exit 0.
- Read-only `rg` checks confirmed eleven `FixtureRequest` call records, six `ActorSessionContext` call records, and the intended boxed-error return sites.

No Cargo, `rustc`, Clippy, test, build, Docker, database, provider, or external-network command was run. Compilation, strict Clippy, test execution/counts, and fixture behavior are **not verified** and await the separately notified heavy-validation slot. No missing or contradictory Scope B contract input was found. No further decomposition is needed. The roadmap was not changed because the assignment's closed output allowlist excludes it; R1 task tracking remains for coordinator integration.

**READY_FOR_HEAVY_VALIDATION**
