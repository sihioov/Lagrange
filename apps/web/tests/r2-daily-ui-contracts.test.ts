import { describe, expect, it } from "vitest";
import {
  type OwnerBetaDailyStateModel,
  ownerBetaDailyConfigPath,
  ownerBetaDailyStatePath,
  ownerBetaDailyStateSchema,
} from "@/lib/products/owner-beta-daily-contracts";

const CONFIG = "00000000-0000-4000-8000-000000000101";
function current(): OwnerBetaDailyStateModel {
  return {
    enabled: true,
    runtime_enabled: true,
    expected_session: "2026-10-15",
    data_support_end: "2026-10-15",
    recommendation_as_of: "2026-10-15",
    published_at: "2026-10-15T08:00:00Z",
    input_kind: "owner_beta_daily_etf11_price_only_v1",
    trigger_kind: "DAILY",
    current_reservation_run_id: "00000000-0000-4000-8000-000000000201",
    state: "CURRENT",
    reason_code: null,
    data_due_at: "2026-10-15T09:30:00Z",
    observed_at: "2026-10-15T08:01:00Z",
  };
}

describe("R2 daily state browser contract", () => {
  it("requires all state fields and rejects raw or unknown fields", () => {
    expect(ownerBetaDailyStateSchema.safeParse(current()).success).toBe(true);
    for (const key of Object.keys(current())) {
      const value: Record<string, unknown> = { ...current() };
      delete value[key];
      expect(ownerBetaDailyStateSchema.safeParse(value).success).toBe(false);
    }
    for (const extra of [
      { enabled: null },
      { enabled: "true" },
      { runtime_enabled: 1 },
      { expected_session: "2026-02-30" },
      { state: "READY" },
      { reason_code: "FREE_FORM_PROVIDER_MESSAGE" },
      { provider_path: "/denied" },
    ]) {
      expect(ownerBetaDailyStateSchema.safeParse({ ...current(), ...extra }).success).toBe(false);
    }
    console.info("R2_DAILY_UI_TEST:closed_state_contract:PASS");
  });

  it("never accepts manual or incomplete publication as daily current", () => {
    for (const change of [
      { trigger_kind: "MANUAL" },
      { input_kind: "owner_beta_historical_price_only_v1" },
      { published_at: null },
      { recommendation_as_of: null },
      { input_kind: null },
      { trigger_kind: null },
      { enabled: false },
      { runtime_enabled: false },
      { expected_session: null },
      { data_support_end: null },
      { data_support_end: "2026-10-14" },
      { recommendation_as_of: "2026-10-14" },
      { reason_code: "JOB_FAILED" },
      { published_at: "2026-10-15T08:02:00Z" },
    ]) {
      expect(ownerBetaDailyStateSchema.safeParse({ ...current(), ...change }).success).toBe(false);
    }
    console.info("R2_DAILY_UI_TEST:current_requires_daily_publication:PASS");
  });

  it("preserves previous publication after disable, stale input and failure", () => {
    for (const state of ["DISABLED", "UNVERIFIABLE", "STALE", "INPUT_BLOCKED", "FAILED"]) {
      const value = {
        ...current(),
        enabled: state !== "DISABLED",
        state,
        data_support_end: null,
        recommendation_as_of: "2026-10-14",
        published_at: "2026-10-14T08:00:00Z",
        reason_code: "VERIFICATION_EXPIRED",
      };
      const parsed = ownerBetaDailyStateSchema.parse(value);
      expect(parsed.expected_session).toBe("2026-10-15");
      expect(parsed.data_support_end).toBeNull();
      expect(parsed.recommendation_as_of).toBe("2026-10-14");
      expect(parsed.published_at).toBe("2026-10-14T08:00:00Z");
      expect(parsed.state).not.toBe("CURRENT");
    }
    console.info("R2_DAILY_UI_TEST:retained_dates_are_not_current:PASS");
  });

  it("allows queued work without inventing publication or input provenance", () => {
    for (const state of ["INPUT_PENDING", "QUEUED", "RUNNING", "RETRYING"]) {
      const parsed = ownerBetaDailyStateSchema.parse({
        ...current(),
        state,
        recommendation_as_of: null,
        published_at: null,
        input_kind: null,
        trigger_kind: null,
        reason_code: "JOB_QUEUED",
      });
      expect(parsed.published_at).toBeNull();
      expect(parsed.input_kind).toBeNull();
      expect(parsed.current_reservation_run_id).not.toBeNull();
    }
    console.info("R2_DAILY_UI_TEST:queue_has_no_publication:PASS");
  });

  it("uses only a validated config identity in read and mutation paths", () => {
    expect(ownerBetaDailyStatePath(CONFIG)).toBe(
      "/api/v1/recommendations/owner-beta/price-only/daily-state?strategy_config_id=" + CONFIG,
    );
    expect(ownerBetaDailyConfigPath(CONFIG)).toBe("/api/v1/strategy-configs/" + CONFIG);
    for (const id of ["", "../runs", CONFIG + "&extra=1", "00000000-0000-0000-0000-000000000000"]) {
      expect(() => ownerBetaDailyStatePath(id)).toThrow();
      expect(() => ownerBetaDailyConfigPath(id)).toThrow();
    }
    console.info("R2_DAILY_UI_TEST:validated_config_paths:PASS");
  });
});
