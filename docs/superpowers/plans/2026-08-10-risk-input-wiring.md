# Live Risk Input Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire market session, data freshness, and intent conflict into real Live risk snapshots without weakening fail-closed behavior.

**Architecture:** Keep the pure `risk-gateway` evaluator unchanged. Add small read-only helpers in `api-server::risk_snapshot` that query the actor-scoped PostgreSQL sources and translate any unavailable source to the existing `Unknown` enum variants. `for_submission` composes those answers using its deterministic timestamp.

**Tech Stack:** Rust 1.97, Tokio, SQLx/PostgreSQL, chrono, existing `risk-gateway` snapshot types.

---

## File map

- Modify: `crates/api-server/src/risk_snapshot.rs` — source queries, timestamp conversion, snapshot wiring.
- Test: `crates/api-server/tests/risk_snapshot_seam.rs` — DB-backed happy paths, failure paths, and tenant isolation.
- Modify: `docs/STATUS.md` — record the wiring and remaining operational/external blockers.

## Task 1: Add failing source-wiring tests

- [ ] Add tests that seed a KRX `TRADING` calendar row, a recent KRX EOD batch, and no active intent, then assert `Open`, `Age(…)`, and `None` in the snapshot.
- [ ] Add tests for a non-trading date, stale batch, active same-account intent, and another-owner intent.
- [ ] Add tests for missing calendar/batch rows and unsupported timezone; assert `Unknown` and a denied gate decision.
- [ ] Run `cargo test -p api-server --test risk_snapshot_seam -- --nocapture` and confirm the new assertions fail because `for_submission` still returns `Unknown`.

## Task 2: Implement source helpers

- [ ] Add a deterministic `Seoul` timestamp helper from `now_secs`; reject invalid/negative timestamps.
- [ ] Add `market_session_for(...)` querying `trading_calendars`, requiring `KRX`, `Asia/Seoul`, and `TRADING`, then applying 09:00–15:30 KST.
- [ ] Add `data_freshness_for(...)` querying the newest KRX/KR EOD `data_batches.retrieved_at`, returning `Age` only for a non-negative, representable age.
- [ ] Add `intent_conflict_for(...)` querying active states for the same actor-owned account/instrument and returning `Conflicting` when any row exists.
- [ ] Wire all three helpers into `for_submission`; map every SQL/parse failure to `Unknown`.

## Task 3: Verify and document

- [ ] Run the focused risk snapshot test and `cargo test -p risk-gateway --test twelve_checks`.
- [ ] Run `cargo clippy -p api-server --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo test --workspace` with `CARGO_TARGET_DIR` on a disk with at least 20 GB free.
- [ ] Update `docs/STATUS.md` to state the three code-level sources are wired and that real calendar/data ingestion/credential operations remain deployment responsibilities.
- [ ] Run `git diff --check`, verify the worktree, and commit with `feat(risk): wire live snapshot inputs`.
