import { describe, expect, it } from "vitest";
import { strategyConfigSchema } from "@/lib/products/contracts";
import {
  ownerBetaInstrumentIdSchema,
  ownerBetaRunListItemSchema,
  ownerBetaRunSchema,
} from "@/lib/products/owner-beta-contracts";
import { strategyConfigSchema as paperStrategyConfigSchema } from "@/lib/products/paper-contracts";

const hash = (digit: string) => `sha256:${digit.repeat(64)}`;
const base = {
  candidate_content_sha256: hash("1"),
  artifact_manifest_sha256: hash("2"),
  stage5_manifest_sha256: hash("3"),
  action_manifest_sha256: hash("4"),
  approval_registry_sha256: hash("5"),
};
const time = "2026-10-15T07:00:00Z";

function pending(): Record<string, unknown> {
  return {
    ...base,
    as_of: "2026-10-15",
    audience: "OWNER_ONLY",
    capability: "PRICE_RETURN_ONLY",
    created_at: time,
    id: "00000000-0000-4000-8000-000000000601",
    input_kind: "owner_beta_historical_price_only_v1",
    items: [],
    job_id: "00000000-0000-4000-8000-000000000701",
    status: "PENDING",
    strategy_config_id: "00000000-0000-4000-8000-000000000101",
    strategy_config_sha256: hash("6"),
    strategy_id: "buy_and_hold",
    strategy_version: "1.0.0",
    updated_at: time,
    vendor_snapshot: true,
    strict_pit: false,
  };
}
function daily(): Record<string, unknown> {
  const value = pending();
  for (const key of Object.keys(base)) {
    delete value[key];
  }
  return {
    ...value,
    input_kind: "owner_beta_daily_etf11_price_only_v1",
    trigger_kind: "DAILY",
    input: {
      kind: "daily",
      content_sha256: hash("7"),
      manifest_sha256: hash("8"),
      admission_sha256: hash("9"),
      policy_sha256: hash("a"),
      base: { ...base },
    },
  };
}
function accepted(value: Record<string, unknown>, expected: boolean): void {
  expect(ownerBetaRunSchema.safeParse(value).success).toBe(expected);
  const list = { ...value };
  delete list["items"];
  expect(ownerBetaRunListItemSchema.safeParse(list).success).toBe(expected);
}
function succeeded(): Record<string, unknown> {
  return {
    ...daily(),
    status: "SUCCEEDED",
    finished_at: time,
    published_at: time,
    factor_snapshot_sha256: hash("b"),
    target_snapshot_sha256: hash("c"),
    cash_weight: "1.000000",
    items: ownerBetaInstrumentIdSchema.options.map((instrument_id) => ({
      instrument_id,
      instrument: {
        id: instrument_id,
        name: null,
        asset_class: null,
        exposure_group: null,
        tracking_index: null,
      },
      excluded: true,
      exclusion_reason: "ALL_CASH_NO_ELIGIBLE",
      factors: {},
      reason_codes: ["ALL_CASH_NO_ELIGIBLE"],
    })),
  };
}

describe("R2 daily API contracts", () => {
  it("accepts daily config fields and preserves legacy omission", () => {
    const config = {
      id: "00000000-0000-4000-8000-000000000101",
      config: {},
      is_active: true,
      strategy_id: "buy_and_hold",
      strategy_version: "1.0.0",
      created_at: time,
      updated_at: time,
    };
    for (const schema of [strategyConfigSchema, paperStrategyConfigSchema]) {
      expect(schema.parse(config).owner_beta_daily_enabled).toBeUndefined();
      expect(
        schema.parse({ ...config, owner_beta_daily_enabled: true, owner_beta_daily_epoch: 2 })
          .owner_beta_daily_epoch,
      ).toBe(2);
      for (const extra of [
        { owner_beta_daily_enabled: null },
        { owner_beta_daily_enabled: "true" },
        { owner_beta_daily_epoch: null },
        { owner_beta_daily_epoch: -1 },
        { owner_beta_daily_epoch: 0.5 },
        { owner_beta_daily_epoch: Number.MAX_SAFE_INTEGER + 1 },
        { daily_provider_key: "denied" },
      ]) {
        expect(schema.safeParse({ ...config, ...extra }).success).toBe(false);
      }
    }
    console.info("R2_DAILY_WEB_TEST:daily_config_legacy_and_explicit:PASS");
  });

  it("checks historical commitments in detail and list", () => {
    accepted(pending(), true);
    const value = { ...pending(), trigger_kind: "MANUAL", input: { kind: "historical", ...base } };
    accepted(value, true);
    accepted({ ...value, trigger_kind: "DAILY" }, false);
    accepted({ ...value, input: { ...value.input, action_manifest_sha256: hash("f") } }, false);
    for (const key of Object.keys(base)) {
      const missing = { ...pending() };
      delete missing[key];
      accepted(missing, false);
    }
    console.info("R2_DAILY_WEB_TEST:historical_union_identity:PASS");
  });

  it("requires daily kind, trigger and every commitment", () => {
    accepted(daily(), true);
    accepted({ ...daily(), trigger_kind: "MANUAL" }, false);
    accepted({ ...daily(), trigger_kind: undefined }, false);
    accepted({ ...daily(), input: undefined }, false);
    accepted({ ...daily(), input: { kind: "historical", ...base } }, false);
    const input = daily()["input"] as Record<string, unknown>;
    for (const key of [
      "content_sha256",
      "manifest_sha256",
      "admission_sha256",
      "policy_sha256",
      "base",
    ]) {
      const missing = { ...input };
      delete missing[key];
      accepted({ ...daily(), input: missing }, false);
    }
    console.info("R2_DAILY_WEB_TEST:daily_commitments_required:PASS");
  });

  it("rejects mixed pins, nulls and unknown evidence fields", () => {
    for (const [key, value] of Object.entries(base)) {
      accepted({ ...daily(), [key]: value }, false);
      accepted({ ...daily(), [key]: null }, false);
      const missingBase: Record<string, unknown> = { ...base };
      delete missingBase[key];
      accepted(
        { ...daily(), input: { ...(daily()["input"] as object), base: missingBase } },
        false,
      );
    }
    accepted(
      { ...daily(), input: { ...(daily()["input"] as object), provider_path: "/denied" } },
      false,
    );
    accepted(
      { ...daily(), input: { ...(daily()["input"] as object), base: { ...base, extra: true } } },
      false,
    );
    console.info("R2_DAILY_WEB_TEST:mixed_null_unknown_rejected:PASS");
  });

  it("requires exact daily publication time and valid items", () => {
    accepted(succeeded(), true);
    accepted({ ...succeeded(), published_at: undefined }, false);
    accepted({ ...succeeded(), published_at: "2026-10-15T07:00:01Z" }, false);
    accepted({ ...succeeded(), published_at: null }, false);
    accepted({ ...succeeded(), finished_at: null }, false);
    expect(ownerBetaRunSchema.safeParse({ ...succeeded(), items: [] }).success).toBe(false);
    expect(ownerBetaRunSchema.safeParse({ ...succeeded(), cash_weight: "0.900000" }).success).toBe(
      false,
    );
    expect(
      ownerBetaRunSchema.safeParse({
        ...succeeded(),
        error_code: "OWNER_BETA_DAILY_AUTHORITY_DENIED",
      }).success,
    ).toBe(false);
    console.info("R2_DAILY_WEB_TEST:daily_publication_and_items:PASS");
  });

  it("separates queue status and forbids early publication", () => {
    accepted({ ...daily(), status: "RUNNING" }, false);
    accepted({ ...daily(), published_at: time }, false);
    accepted(
      { ...daily(), status: "FAILED", error_code: "OWNER_BETA_DAILY_AUTHORITY_DENIED" },
      true,
    );
    accepted({ ...daily(), status: "CANCELED", error_code: "DAILY_SESSION_SUPERSEDED" }, true);
    expect(ownerBetaRunSchema.safeParse({ ...daily(), status: "FAILED" }).success).toBe(false);
    expect(
      ownerBetaRunSchema.safeParse({ ...daily(), factor_snapshot_sha256: hash("b") }).success,
    ).toBe(false);
    console.info("R2_DAILY_WEB_TEST:queue_and_publication_separated:PASS");
  });
});
