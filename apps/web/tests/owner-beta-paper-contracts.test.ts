import { describe, expect, it } from "vitest";
import {
  OWNER_BETA_PAPER_CONSENT_PATH,
  OWNER_BETA_PAPER_PREVIEWS_PATH,
  OWNER_BETA_PAPER_STATE_PATH,
  ownerBetaPaperApplyPath,
  ownerBetaPaperApplySchema,
  ownerBetaPaperConsentBodySchema,
  ownerBetaPaperPreviewBodySchema,
  ownerBetaPaperPreviewPath,
  ownerBetaPaperPreviewSchema,
  ownerBetaPaperSessionDetailSchema,
  ownerBetaPaperSessionPath,
  ownerBetaPaperStateSchema,
} from "@/lib/products/paper-contracts";

const ACCOUNT_ID = "00000000-0000-4000-8000-000000000401";
const PREVIEW_ID = "00000000-0000-4000-8000-000000000501";
const TARGET_ID = "00000000-0000-4000-8000-000000000421";
const HASH = `sha256:${"a".repeat(64)}`;

function preview(result: unknown = null) {
  return {
    schema: "owner-beta-paper-preview-v1",
    preview_id: PREVIEW_ID,
    job_id: "00000000-0000-4000-8000-000000000801",
    account_id: ACCOUNT_ID,
    status: "READY",
    signal_date: "2026-10-30",
    effective_session: "2026-11-02",
    control_epoch: 7,
    config_epoch: 3,
    daily_activation_epoch: 5,
    lineage: {
      recommendation_run_id: "00000000-0000-4000-8000-000000000201",
      strategy_config_id: "00000000-0000-4000-8000-000000000101",
      signal_date: "2026-10-30",
      effective_session: "2026-11-02",
      control_epoch: 7,
      config_epoch: 3,
      daily_activation_epoch: 5,
      cohort_sha256: HASH,
      daily_pins: {
        admission_sha256: HASH,
        content_sha256: HASH,
        manifest_sha256: HASH,
        policy_sha256: HASH,
      },
      factor_snapshot_sha256: HASH,
      target_snapshot_sha256: HASH,
      signal_allocation_sha256: HASH,
      target_weights: [{ instrument_id: "069500.KRX", weight: "1.000000000000000001" }],
    },
    preview_token: "b".repeat(64),
    cost_profile_id: "KRX_ETF_DEFAULT",
    cost_profile_version: 1,
    result,
    error: null,
    created_at: "2026-10-30T10:00:00Z",
    started_at: "2026-10-30T10:00:01Z",
    completed_at: "2026-10-30T10:00:02Z",
    applied_at: null,
    updated_at: "2026-10-30T10:00:02Z",
  };
}

describe("Owner-beta Paper API contracts", () => {
  it("uses the established state, consent, preview, apply, and pending-target routes", () => {
    expect(OWNER_BETA_PAPER_STATE_PATH).toBe("/api/v1/paper/owner-beta/state");
    expect(OWNER_BETA_PAPER_CONSENT_PATH).toBe("/api/v1/paper/owner-beta/consent");
    expect(OWNER_BETA_PAPER_PREVIEWS_PATH).toBe("/api/v1/paper/owner-beta/previews");
    expect(ownerBetaPaperPreviewPath(PREVIEW_ID)).toBe(
      `/api/v1/paper/owner-beta/previews/${PREVIEW_ID}`,
    );
    expect(ownerBetaPaperApplyPath(PREVIEW_ID)).toBe(
      `/api/v1/paper/owner-beta/previews/${PREVIEW_ID}/apply`,
    );
    expect(ownerBetaPaperSessionPath(TARGET_ID)).toBe(
      `/api/v1/paper/owner-beta/sessions/${TARGET_ID}`,
    );
  });

  it("accepts disabled zero-runtime state and rejects undocumented state fields", () => {
    const disabled = {
      schema: "owner-beta-paper-state-v1",
      state: "DISABLED",
      account_id: null,
      strategy_config_id: null,
      strategy_id: null,
      strategy_version: null,
      control_epoch: null,
      consent_present: false,
      daily_cohort_sha256: null,
      process_binding_sha256: null,
      sessions: [],
    };
    expect(ownerBetaPaperStateSchema.parse(disabled).state).toBe("DISABLED");
    expect(
      ownerBetaPaperStateSchema.safeParse({ ...disabled, runtime_path: "/secret" }).success,
    ).toBe(false);
  });

  it("requires explicit consent and preview epoch values with closed request bodies", () => {
    expect(
      ownerBetaPaperConsentBodySchema.safeParse({ opt_in: true, expected_epoch: 3 }).success,
    ).toBe(true);
    expect(
      ownerBetaPaperConsentBodySchema.safeParse({
        opt_in: true,
        expected_epoch: 3,
        account_id: ACCOUNT_ID,
      }).success,
    ).toBe(false);
    expect(
      ownerBetaPaperPreviewBodySchema.safeParse({
        recommendation_run_id: "00000000-0000-4000-8000-000000000201",
        expected_epoch: 3,
      }).success,
    ).toBe(true);
  });

  it("preserves decimal values as exact strings and rejects numeric coercion", () => {
    const result = {
      schema: "owner-beta-paper-preview-result-v1",
      target_snapshot_sha256: HASH,
      signal_allocation_sha256: HASH,
      plan_status: "REBALANCE",
      no_trade_reasons: [],
      equity: "10000000.000000000000000001",
      cash_before: "9000000.000000000000000001",
      available_cash: "8990000.000000000000000001",
      leftover_cash: "123.000000000000000001",
      buy_notional: "100.000000000000000001",
      sell_notional: "0.000000000000000000",
      explicit_fees: "1.000000000000000001",
      informational_slippage: "0.500000000000000001",
      decisions: [],
      orders: [],
      cost_profile_id: "KRX_ETF_DEFAULT",
      cost_profile_version: 1,
      price_observations: [],
    };
    const parsed = ownerBetaPaperPreviewSchema.parse(preview(result));
    expect(parsed.result?.equity).toBe("10000000.000000000000000001");
    expect(
      ownerBetaPaperPreviewSchema.safeParse(preview({ ...result, equity: 10_000_000 })).success,
    ).toBe(false);
    expect(
      ownerBetaPaperPreviewSchema.safeParse(
        preview({
          ...result,
          price_observations: [
            {
              instrument_id: "069500.KRX",
              session_date: "2026-10-29",
              observed_at: "2026-10-30T06:00:00Z",
            },
          ],
        }),
      ).success,
    ).toBe(false);
  });

  it("keeps apply identity and comparison disposition bound to the API response", () => {
    const applied = ownerBetaPaperApplySchema.parse({
      schema: "owner-beta-paper-apply-v1",
      preview_id: PREVIEW_ID,
      pending_target_id: TARGET_ID,
      lineage_id: "00000000-0000-4000-8000-000000000601",
      effective_session: "2026-11-02",
      control_epoch: 7,
      status: "APPLIED",
      replayed: true,
    });
    expect(applied.preview_id).toBe(PREVIEW_ID);
    const sessionDetail = {
      schema: "owner-beta-paper-session-v1",
      pending_target_id: TARGET_ID,
      signal_date: "2026-10-30",
      effective_session: "2026-11-02",
      state: "WAITING_PRICE",
      processing_origin: "AUTO",
      lineage: preview().lineage,
      source_original_retrieved_at: null,
      input_admitted_at: null,
      usable_at: null,
      settled_at: null,
      valued_at: null,
      settlement_kind: null,
      reason_code: "PRICE_NOT_AVAILABLE",
      amounts: {
        cash_after: null,
        valuation_cash: null,
        positions_value: null,
        equity: null,
      },
      order_references: [],
      comparison: {
        disposition: "NOT_COMPARABLE",
        reason: "NO_MATCHED_BACKTEST_EXECUTION_CONTRACT",
      },
    };
    const session = ownerBetaPaperSessionDetailSchema.parse(sessionDetail);
    expect(session.comparison).toEqual({
      disposition: "NOT_COMPARABLE",
      reason: "NO_MATCHED_BACKTEST_EXECUTION_CONTRACT",
    });
    expect(
      ownerBetaPaperSessionDetailSchema.safeParse({
        ...sessionDetail,
        comparison: { disposition: "PRICE_RETURN_ONLY", reason: "different basis" },
      }).success,
    ).toBe(false);
  });
});
