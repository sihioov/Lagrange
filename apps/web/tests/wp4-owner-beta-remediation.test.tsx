import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  formatOwnerBetaFactor,
  OwnerBetaReport,
} from "@/components/recommendations/owner-beta-report";
import {
  isOwnerBetaAsOfSupported,
  OwnerBetaRunForm,
} from "@/components/recommendations/owner-beta-run-form";
import { createProductApiClient } from "@/lib/api/product-client";
import { recommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import {
  ownerBetaFactorsSchema,
  ownerBetaRunSchema,
  ownerBetaSupportedAsOfSchema,
} from "@/lib/products/owner-beta-contracts";

const ETF_IDS = [
  "069500.KRX",
  "102110.KRX",
  "114260.KRX",
  "132030.KRX",
  "133690.KRX",
  "143850.KRX",
  "148070.KRX",
  "153130.KRX",
  "192090.KRX",
  "195930.KRX",
  "229200.KRX",
] as const;

const RUN_ID = "00000000-0000-4000-8000-000000000201";
const JOB_ID = "00000000-0000-4000-8000-000000000301";
const CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const SHA = `sha256:${"a".repeat(64)}`;
const OWNER_BETA_REASON_CODES = [
  "SELECTED_TOP_N",
  "NOT_SELECTED_BEYOND_TOP_N",
  "EXCLUDED_MANDATORY_FACTOR_NULL",
  "ALL_CASH_NO_ELIGIBLE",
  "WEIGHT_CAPPED_AT_MAX",
  "WEIGHT_ROUNDING_RESIDUE_TO_CASH",
  "CASH_FLOOR_APPLIED",
  "BENCHMARK_HELD",
  "TREND_POSITIVE",
  "TREND_NEGATIVE_CASH",
  "ABSOLUTE_MOMENTUM_PASSED",
  "DEFENSIVE_CASH_SELECTED",
  "INVERSE_VOL_WEIGHTED",
  "NOT_SELECTED_BY_STRATEGY",
] as const;
type OwnerBetaReasonCode = (typeof OWNER_BETA_REASON_CODES)[number];

const EXCLUDED_REASON_CODES = new Set<OwnerBetaReasonCode>([
  "NOT_SELECTED_BEYOND_TOP_N",
  "EXCLUDED_MANDATORY_FACTOR_NULL",
  "NOT_SELECTED_BY_STRATEGY",
]);
const ALL_CASH_REASON_CODES = new Set<OwnerBetaReasonCode>([
  "ALL_CASH_NO_ELIGIBLE",
  "TREND_NEGATIVE_CASH",
  "DEFENSIVE_CASH_SELECTED",
]);
const FACTORS_BY_REASON: Partial<Record<OwnerBetaReasonCode, Record<string, string>>> = {
  TREND_POSITIVE: { trend_50: "0.012300" },
  ABSOLUTE_MOMENTUM_PASSED: { momentum_12_1: "0.123456" },
  INVERSE_VOL_WEIGHTED: { vol_60: "0.123456" },
};

function ownerRun(overrides: Record<string, unknown> = {}) {
  return {
    action_manifest_sha256: SHA,
    approval_registry_sha256: SHA,
    artifact_manifest_sha256: SHA,
    as_of: "2026-08-19",
    audience: "OWNER_ONLY",
    candidate_content_sha256: SHA,
    capability: "PRICE_RETURN_ONLY",
    cash_weight: "0.200000",
    created_at: "2026-08-19T06:30:00Z",
    factor_snapshot_sha256: SHA,
    finished_at: "2026-08-19T06:30:02Z",
    id: RUN_ID,
    input_kind: "owner_beta_historical_price_only_v1",
    items: ETF_IDS.map((instrument_id, index) => ({
      excluded: index !== 0,
      exclusion_reason: index === 0 ? undefined : "NOT_SELECTED_BY_STRATEGY",
      factors: {},
      instrument: {
        asset_class: index === 0 ? "ETF" : null,
        exposure_group: null,
        id: instrument_id,
        name: index === 0 ? "Core ETF" : null,
        tracking_index: null,
      },
      instrument_id,
      rank: index === 0 ? 1 : null,
      reason_codes: index === 0 ? ["SELECTED_TOP_N"] : ["NOT_SELECTED_BY_STRATEGY"],
      target_weight: index === 0 ? "0.800000" : null,
    })),
    job_id: JOB_ID,
    stage5_manifest_sha256: SHA,
    started_at: "2026-08-19T06:30:01Z",
    status: "SUCCEEDED",
    strategy_config_id: CONFIG_ID,
    strategy_config_sha256: SHA,
    strategy_id: "buy_and_hold",
    strategy_version: "1.0.0",
    strict_pit: false,
    target_snapshot_sha256: SHA,
    updated_at: "2026-08-19T06:30:02Z",
    vendor_snapshot: true,
    ...overrides,
  };
}

function ownerRunForReason(code: OwnerBetaReasonCode) {
  const allCash = ALL_CASH_REASON_CODES.has(code);
  const targetIndex = allCash ? -1 : EXCLUDED_REASON_CODES.has(code) ? 1 : 0;
  const selectedFactors = allCash ? {} : (FACTORS_BY_REASON[code] ?? { momentum_12_1: "0.123456" });

  return ownerRun({
    cash_weight: allCash ? "1.000000" : "0.200000",
    items: ETF_IDS.map((instrument_id, index) => {
      const excluded = allCash || index !== 0;
      const reason = allCash
        ? index === 0
          ? code
          : "NOT_SELECTED_BY_STRATEGY"
        : index === targetIndex
          ? code
          : index === 0
            ? "SELECTED_TOP_N"
            : "NOT_SELECTED_BY_STRATEGY";
      return {
        excluded,
        exclusion_reason: excluded ? reason : undefined,
        factors: excluded ? {} : selectedFactors,
        instrument: {
          asset_class: index === 0 ? "ETF" : null,
          exposure_group: null,
          id: instrument_id,
          name: index === 0 ? "Core ETF" : null,
          tracking_index: null,
        },
        instrument_id,
        rank: excluded ? null : 1,
        reason_codes: [reason],
        target_weight: excluded ? null : "0.800000",
      };
    }),
    strategy_id: "relative_momentum",
  });
}

describe("WP-4 owner-beta discovery and detail contracts", () => {
  it("accepts only sorted unique discovery dates whose default is the maximum", () => {
    const valid = {
      default_as_of: "2026-08-19",
      supported_as_of: ["2026-08-15", "2026-08-18", "2026-08-19"],
    };
    expect(ownerBetaSupportedAsOfSchema.safeParse(valid).success).toBe(true);
    for (const invalid of [
      { ...valid, default_as_of: "2026-08-18" },
      { ...valid, supported_as_of: ["2026-08-19", "2026-08-18"] },
      { ...valid, supported_as_of: ["2026-08-18", "2026-08-18"] },
      { ...valid, supported_as_of: [] },
      { ...valid, supported_as_of: ["2026-02-30"] },
      { default_as_of: valid.default_as_of },
      { ...valid, extra: true },
    ]) {
      expect(ownerBetaSupportedAsOfSchema.safeParse(invalid).success).toBe(false);
    }
  });

  it("fetches and strictly parses the dedicated discovery path", async () => {
    const requests: Array<{ readonly method: string; readonly url: string }> = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      requests.push({ method: request.method, url: request.url });
      return Response.json({
        default_as_of: "2026-08-19",
        supported_as_of: ["2026-08-15", "2026-08-18", "2026-08-19"],
      });
    };
    const client = createProductApiClient({ baseUrl: "https://api.internal", fetcher });

    await expect(client.getOwnerBetaSupportedAsOf()).resolves.toEqual({
      default_as_of: "2026-08-19",
      supported_as_of: ["2026-08-15", "2026-08-18", "2026-08-19"],
    });
    expect(requests).toEqual([
      {
        method: "GET",
        url: "https://api.internal/api/v1/recommendations/owner-beta/price-only/supported-as-of",
      },
    ]);
  });

  it("uses only discovered dates in the form and blocks a non-discovered date", () => {
    const supported = ["2026-08-15", "2026-08-18", "2026-08-19"] as const;
    const markup = renderToStaticMarkup(
      <OwnerBetaRunForm
        configs={[{ id: CONFIG_ID, label: "buy_and_hold@1.0.0" }]}
        defaultAsOf="2026-08-19"
        supportedAsOf={supported}
      />,
    );

    expect(markup).toContain("Sealed input as-of date");
    expect(markup).toContain("Latest supported as-of date: 2026-08-19");
    expect(markup).toContain('value="2026-08-19" selected=""');
    expect(markup).toContain('value="2026-08-15"');
    expect(markup).not.toContain("2026-08-01");
    expect(isOwnerBetaAsOfSupported("2026-08-19", supported)).toBe(true);
    expect(isOwnerBetaAsOfSupported("2026-08-01", supported)).toBe(false);
    expect(recommendationsDictionary.ko.ownerBetaSealedAsOfLabel).toBe("봉인된 입력 기준일");
    expect(recommendationsDictionary.ko.ownerBetaLatestSupportedAsOfLabel).toBe(
      "지원되는 최신 기준일",
    );
  });

  it("requires ETF11 nested identity and leaves metadata out of economic fields", () => {
    const valid = ownerBetaRunSchema.parse(ownerRun());
    const withMetadata = ownerBetaRunSchema.parse(
      ownerRun({
        items: valid.items.map((item, index) => ({
          ...item,
          instrument: {
            ...item.instrument,
            asset_class: index === 0 ? "ETF" : "Fund",
            name: `Instrument ${index}`,
          },
        })),
      }),
    );
    const economic = (run: typeof valid) =>
      run.items.map(({ instrument: _instrument, ...item }) => item);

    expect(economic(withMetadata)).toEqual(economic(valid));
    expect(withMetadata.cash_weight).toBe(valid.cash_weight);
    expect(
      ownerBetaRunSchema.safeParse({
        ...ownerRun(),
        items: ownerRun().items.map((item, index) =>
          index === 0 ? { ...item, instrument: { ...item.instrument, id: "102110.KRX" } } : item,
        ),
      }).success,
    ).toBe(false);
    expect(
      ownerBetaRunSchema.safeParse({
        ...ownerRun(),
        items: ownerRun().items.map((item, index) =>
          index === 0 ? { ...item, instrument_id: "000000.KRX" } : item,
        ),
      }).success,
    ).toBe(false);
    expect(
      ownerBetaRunSchema.safeParse({
        ...ownerRun(),
        items: ownerRun().items.map((item, index) =>
          index === 0 ? { ...item, instrument: { ...item.instrument, extra: "no" } } : item,
        ),
      }).success,
    ).toBe(false);
    for (const field of ["tracking_index", "exposure_group"] as const) {
      expect(
        ownerBetaRunSchema.safeParse({
          ...ownerRun(),
          items: ownerRun().items.map((item, index) =>
            index === 0
              ? { ...item, instrument: { ...item.instrument, [field]: "not permitted" } }
              : item,
          ),
        }).success,
      ).toBe(false);
    }
  });

  it("accepts every allowed factor pattern and rejects registry-only or malformed evidence", () => {
    for (const id of [
      "trend_5",
      "trend_37",
      "trend_500",
      "momentum_12_1",
      "return_12m",
      "vol_20",
      "vol_60",
      "vol_120",
    ]) {
      expect(ownerBetaFactorsSchema.safeParse({ [id]: "0.012300" }).success).toBe(true);
    }
    for (const value of ["-0.0", "1e-07", "3e-06", "5e-05", "1e+20"]) {
      expect(ownerBetaFactorsSchema.safeParse({ return_12m: value }).success).toBe(true);
    }
    expect(ownerBetaFactorsSchema.safeParse({}).success).toBe(true);

    for (const invalid of [
      { normalized_score: "0.9" },
      { return_1m: "0.1" },
      { return_3m: "0.1" },
      { return_6m: "0.1" },
      { drawdown: "-0.1" },
      { trend_4: "0.1" },
      { trend_05: "0.1" },
      { trend_501: "0.1" },
      { vol_60: "-0.1" },
      { return_12m: 0.1 },
      { return_12m: "NaN" },
      { return_12m: "Infinity" },
      { return_12m: "1E-07" },
      { return_12m: "1e07" },
      { return_12m: "1e-7" },
      { return_12m: "1e+7" },
      { return_12m: "1e+999" },
      { return_12m: "01.0" },
      { return_12m: "1" },
      { return_12m: null },
    ]) {
      expect(ownerBetaFactorsSchema.safeParse(invalid).success).toBe(false);
    }
  });

  it("rejects missing, duplicate, and unknown durable reason codes", () => {
    const source = ownerRun();
    for (const reason_codes of [[], ["SELECTED_TOP_N", "SELECTED_TOP_N"], ["UNKNOWN_REASON"]]) {
      expect(
        ownerBetaRunSchema.safeParse({
          ...source,
          items: source.items.map((item, index) =>
            index === 0 ? { ...item, reason_codes } : item,
          ),
        }).success,
      ).toBe(false);
    }
  });

  it("requires non-buy-and-hold selected evidence without consulting mutable configuration", () => {
    const missing = ownerRun({ strategy_id: "relative_momentum" });
    expect(ownerBetaRunSchema.safeParse(missing).success).toBe(false);
    const present = ownerRun({
      strategy_id: "relative_momentum",
      items: ownerRun().items.map((item, index) =>
        index === 0 ? { ...item, factors: { momentum_12_1: "0.1" } } : item,
      ),
    });
    expect(ownerBetaRunSchema.safeParse(present).success).toBe(true);
  });

  it("formats raw factor evidence as signed or unsigned two-decimal percentages", () => {
    expect(formatOwnerBetaFactor("0.012345", true)).toBe("+1.23%");
    expect(formatOwnerBetaFactor("-0.012345", true)).toBe("-1.23%");
    expect(formatOwnerBetaFactor("0.123456", false)).toBe("12.35%");
    expect(formatOwnerBetaFactor("1e-07", true)).toBe("+0.00%");
    expect(formatOwnerBetaFactor("3e-06", true)).toBe("+0.00%");
    expect(formatOwnerBetaFactor("5e-05", true)).toBe("+0.00%");
  });

  it("does not invent factor evidence for buy-and-hold", () => {
    expect(
      ownerBetaRunSchema.safeParse(
        ownerRun({
          items: ownerRun().items.map((item, index) =>
            index === 0 ? { ...item, factors: { momentum_12_1: "0.1" } } : item,
          ),
        }),
      ).success,
    ).toBe(false);

    const markup = renderToStaticMarkup(
      <OwnerBetaReport
        run={ownerBetaRunSchema.parse(ownerRun())}
        t={recommendationsDictionary.ko}
      />,
    );

    expect(markup).toContain("팩터 근거 없음(매수·보유 전략)");
    expect(markup).not.toContain("점수");
  });

  it("maps all durable reasons to static copy and never renders the raw codes", () => {
    const dictionaries = [recommendationsDictionary.en, recommendationsDictionary.ko];
    for (const t of dictionaries) {
      expect(Object.keys(t.ownerBetaReasonExplanations).sort()).toEqual(
        [...OWNER_BETA_REASON_CODES].sort(),
      );
      for (const code of OWNER_BETA_REASON_CODES) {
        const explanation = t.ownerBetaReasonExplanations[code];
        expect(explanation).toMatch(/\S/);
        expect(explanation).not.toMatch(/[{}]/);
        expect(explanation).not.toContain(code);
        expect(explanation).not.toContain("top_n");

        const parsed = ownerBetaRunSchema.safeParse(ownerRunForReason(code));
        expect(parsed.success, `invalid fixture for ${code}`).toBe(true);
        if (!parsed.success) continue;
        const markup = renderToStaticMarkup(<OwnerBetaReport run={parsed.data} t={t} />);
        expect(markup).not.toContain(code);
        expect(markup).toContain(explanation);
      }
    }
  });

  it("keeps the primary report free of audit commitments while retaining every audit value", () => {
    const values = {
      action_manifest_sha256: `sha256:${"b".repeat(64)}`,
      approval_registry_sha256: `sha256:${"c".repeat(64)}`,
      artifact_manifest_sha256: `sha256:${"d".repeat(64)}`,
      candidate_content_sha256: `sha256:${"e".repeat(64)}`,
      factor_snapshot_sha256: `sha256:${"f".repeat(64)}`,
      job_id: "00000000-0000-4000-8000-000000000901",
      strategy_config_id: "00000000-0000-4000-8000-000000000902",
      strategy_config_sha256: `sha256:${"1".repeat(64)}`,
      target_snapshot_sha256: `sha256:${"2".repeat(64)}`,
    };
    const runId = "00000000-0000-4000-8000-000000000903";
    const markup = renderToStaticMarkup(
      <OwnerBetaReport
        run={ownerBetaRunSchema.parse(ownerRun({ id: runId, ...values }))}
        t={recommendationsDictionary.en}
      />,
    );
    const primary = markup.slice(0, markup.indexOf("<details"));
    expect(primary).not.toContain(runId);
    expect(primary).not.toContain(values.job_id);
    expect(primary).not.toContain(values.strategy_config_id);
    for (const hash of Object.values(values).filter((value) => value.startsWith("sha256:"))) {
      expect(primary).not.toContain(hash);
      expect(markup).toContain(hash);
    }
    expect(markup).toContain("<summary>Audit details</summary>");
    expect(markup).toContain(runId);
    expect(markup).toContain(values.job_id);
    expect(markup).toContain(values.strategy_config_id);
    expect(markup).toContain("Run as-of date");
    expect(markup).toContain("buy_and_hold@1.0.0");
  });

  it("retains null snapshots verbatim and distinguishes an absent snapshot as not reported", () => {
    const source = ownerRun();
    const {
      cash_weight: _cashWeight,
      factor_snapshot_sha256: _factorSnapshot,
      items: _items,
      target_snapshot_sha256: _targetSnapshot,
      ...withoutResults
    } = source;
    const pending = ownerBetaRunSchema.parse({
      ...withoutResults,
      factor_snapshot_sha256: null,
      items: [],
      status: "PENDING",
      target_snapshot_sha256: null,
    });
    const markup = renderToStaticMarkup(
      <OwnerBetaReport run={pending} t={recommendationsDictionary.en} />,
    );
    expect(markup).toContain("<summary>Audit details</summary>");
    expect(markup).toContain(">null<");

    const absent = ownerBetaRunSchema.parse({
      ...withoutResults,
      items: [],
      status: "PENDING",
    });
    const absentMarkup = renderToStaticMarkup(
      <OwnerBetaReport run={absent} t={recommendationsDictionary.en} />,
    );
    expect(absentMarkup).toContain("Not reported");
  });

  it("does not expose or infer an exposure group or pair-specific selector warning", () => {
    const markup = renderToStaticMarkup(
      <OwnerBetaReport
        run={ownerBetaRunSchema.parse(ownerRun())}
        t={recommendationsDictionary.en}
      />,
    );
    expect(markup.toLowerCase()).not.toContain("exposure");
    expect(markup.toLowerCase()).not.toContain("duplicate");
    expect(markup.toLowerCase()).not.toContain("same group");
    expect(markup).toContain("069500.KRX");
    expect(markup).toContain("102110.KRX");
  });
});
