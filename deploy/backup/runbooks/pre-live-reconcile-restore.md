# Pre-Live Restore + Reconciliation-Only Startup — MANDATORY GATE 2

**Status:** CONTRACT SKELETON. Restore automation is Todo 33; startup/reconciliation and
broker reconciliation machinery is Todo 38+ (Live node). Until this drill passes with a real
executed transcript, **do not claim Live readiness** and do not activate the Live profile.

**Gate rule (System Design §16 fail-closed policy):** before ANY Owner-only Live enablement,
a full restore must succeed AND the restored system must start in **reconciliation-only**
mode — no order submission — until startup reconciliation reports zero mismatches and an
explicit Owner approval is recorded. A restore without a reconciliation-only startup is not a
valid pre-Live gate.

---

## 1. Precondition — policy gate (machine-checkable, runs NOW)

```pwsh
scripts/backup/validate-policy.ps1 -SetPath <backup-set-dir> -Gate prelive
# or: bash scripts/backup/validate-policy.sh --set <backup-set-dir> --gate prelive
```

**Success assertion B1:** the command exits `0` and prints
`POLICY OK: backup set <set-id> valid for gate prelive`. The `prelive` gate additionally
requires the manifest to declare `restore_policy.assertions.prelive.startup_mode =
"reconciliation_only"` — a manifest that omits this fails the gate. Any nonzero exit aborts
before any restore command runs.

## 2. Restore into isolated targets (same isolation rules as Gate 1)

Same disposable-DB / empty-fresh-directory isolation as the pre-Member drill. A pre-Live
restore into production paths is FORBIDDEN by policy.

## 3. Reconciliation-only startup

- Start the restored stack with the Live profile in **reconciliation-only** mode (no new
  order intents; broker/position/cash/order-state reconciliation runs at startup, Todo 38).
- Reconciliation compares internal state (orders, fills, positions, cash, equity) against
  the broker's reported state (KIS in vendor-truth mode).

## 4. Exact success assertions (ALL must be true; any false ⇒ DRILL FAILED, Live stays off)

| # | Assertion | Machine check |
|---|-----------|---------------|
| B2 | Validator exit 0 (B1) + saved transcript | same check as A1, saved to evidence |
| B3 | Restored files hash-match manifest | `sha256sum -c` → `0` mismatches |
| B4 | Restored DB schema matches snapshot | `pg_dump --schema-only` diff → `0` lines |
| B5 | No secret marker in restored tree | grep of `secret_markers` → `0` hits |
| B6 | Startup ran in reconciliation-only mode | `startup_mode` in node startup log equals `reconciliation_only`; **zero** order intents submitted |
| B7 | Reconciliation completed with zero mismatches | `SELECT count(*) FROM reconciliation_mismatches WHERE resolved_at IS NULL;` → `0` rows |
| B8 | Fail-closed hold honored | if B7 is false OR `kill_switch_state` active, Live remains locked (assert via node state = `RECONCILING`/`LOCKED`, not `LIVE`) |
| B9 | Owner approval recorded | an audit row for the explicit Live-approval action exists (append-only) before Live mode becomes available |

## 5. Failure handling

- Any assertion false: Live stays off (fail-closed). Never override a reconciliation
  mismatch with operator judgment — the policy is §16: "내부·브로커 포지션 불일치 → Live 전략
  일시정지, 관리자 승인 필요" (pause Live strategy, admin approval required).
- DB write failure during the drill: new Live order submission stays blocked; order state must
  remain recoverable from the raw log (design §16 last row).

## 6. Evidence to record

- B1/B2 transcript, B3–B5 scan transcripts, B6 startup-mode log line, B7 SQL result, B8 node
  state assertion output, B9 audit-row query result. Saved under `.omo/evidence/`.
