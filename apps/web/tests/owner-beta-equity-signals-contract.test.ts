import { describe, expect, it } from "vitest";
import { createProductApiClient } from "@/lib/api/product-client";
import {
  ownerBetaEquitySignalsDetailSchema,
  ownerBetaEquitySignalsLatestSchema,
  ownerBetaEquitySignalsScreenBody,
  ownerBetaEquitySignalsScreenBodySchema,
  ownerBetaEquitySignalsScreenSchema,
  parseOwnerBetaEquitySignalsSearchParams,
} from "@/lib/products/equity-signals-contracts";

const SHA = `sha256:${"a".repeat(64)}`;
const IDS = Array.from({ length: 30 }, (_, index) => `${String(index + 1).padStart(6, "0")}.KRX`);

const provenance = {
  activity_proxy: "20-session average trading value",
  artifact_content_sha256: SHA,
  as_of: "2026-08-29",
  audience: "OWNER_ONLY",
  batch_id: "batch-stock-beta-1",
  capability: "PRICE_VOLUME_RESEARCH_ONLY",
  entitlement_sha256: SHA,
  factor_version: "stock-price-beta-v1",
  index_membership: "not an index membership universe",
  materialization_status: "MATERIALIZED",
  original_price: true,
  publication_status: "OWNER_ONLY_API",
  redistribution: "restricted",
  registration_status: "registered",
  registry_sha256: SHA,
  selection_basis: "configured fixed observation list",
  snapshot_content_sha256: SHA,
  strict_pit: false,
  universe_sha256: SHA,
  vendor_snapshot: true,
  warning: "Corporate actions can distort returns and drawdowns.",
} as const;

function row(index: number) {
  return {
    average_trading_value_20: 1_000_000 + index,
    average_volume_20: 100_000 + index,
    condition: index % 3 === 0 ? "BULLISH" : index % 3 === 1 ? "NEUTRAL" : "BEARISH",
    instrument_id: IDS[index] as string,
    instrument_name: `Instrument ${index + 1}`,
    max_drawdown_120: -0.1 - index / 1_000,
    rank: index + 1,
    return_120: 0.4 - index / 100,
    return_20: 0.1 - index / 1_000,
    return_60: 0.2 - index / 1_000,
    score: 100 - index,
    sma_20: 100 + index,
    sma_60: 99 + index,
    volatility_120: 0.2 + index / 1_000,
    volatility_20: 0.1 + index / 1_000,
    volatility_60: 0.15 + index / 1_000,
    volume_ratio_20_60: 1.2 + index / 1_000,
  } as const;
}

const rows = IDS.map((_, index) => row(index));
const latest = ownerBetaEquitySignalsLatestSchema.parse({
  provenance,
  rows,
  top5: rows.slice(0, 5),
});
const screen = ownerBetaEquitySignalsScreenSchema.parse({ provenance, rows: rows.slice(0, 2) });
const detail = ownerBetaEquitySignalsDetailSchema.parse({
  condition_reasons: [
    "return_20 is at least 0.050000",
    "trend_up is true",
    "volatility_120 is at most 0.350000",
  ],
  factor_explanations: [
    { factor: "return_20", interpretation: "20-session price return", value: 0.1 },
    { factor: "return_60", interpretation: "60-session price return", value: 0.2 },
    { factor: "return_120", interpretation: "120-session price return", value: 0.4 },
    {
      factor: "volatility_120",
      interpretation: "120-session annualized volatility",
      value: 0.2,
    },
    {
      factor: "max_drawdown_120",
      interpretation: "120-session maximum drawdown",
      value: -0.1,
    },
    {
      factor: "average_trading_value_20",
      interpretation: "20-session activity proxy, not execution liquidity",
      value: 1_000_000,
    },
    { factor: "trend", interpretation: "sma20 minus sma60", value: 1 },
  ],
  provenance,
  signal: rows[0],
});

describe("Owner-beta equity signal contracts", () => {
  it("rejects unknown fields and invalid nested values", () => {
    expect(ownerBetaEquitySignalsLatestSchema.safeParse({ ...latest, unknown: true }).success).toBe(
      false,
    );
    expect(
      ownerBetaEquitySignalsLatestSchema.safeParse({
        ...latest,
        rows: latest.rows.map((item, index) =>
          index === 0 ? { ...item, score: Number.NaN } : item,
        ),
      }).success,
    ).toBe(false);
    expect(
      ownerBetaEquitySignalsDetailSchema.safeParse({
        ...detail,
        factor_explanations: detail.factor_explanations.map((factor, index) =>
          index === 0 ? { ...factor, extra: "reject" } : factor,
        ),
      }).success,
    ).toBe(false);
    expect(
      ownerBetaEquitySignalsScreenBodySchema.safeParse({
        conditions: { return_20: { min: 0.2, max: 0.1 } },
        extra: true,
      }).success,
    ).toBe(false);
  });

  it("maps GET filters to the exact screen body without changing field names", () => {
    const filters = parseOwnerBetaEquitySignalsSearchParams({
      activity_min: "1000",
      condition: ["BULLISH", "BEARISH"],
      max_drawdown_120_max: "-0.1",
      return_20_max: "0.2",
      return_20_min: "0.1",
      trend: "up",
      volatility_60_min: "0.3",
    });

    expect(ownerBetaEquitySignalsScreenBody(filters)).toEqual({
      condition: ["BULLISH", "BEARISH"],
      conditions: {
        average_trading_value_20: { min: 1000 },
        max_drawdown_120: { max: -0.1 },
        return_20: { max: 0.2, min: 0.1 },
        trend_up: true,
        volatility_60: { min: 0.3 },
      },
    });
  });

  it("validates duplicate and reversed URL filters locally", () => {
    expect(() =>
      parseOwnerBetaEquitySignalsSearchParams({ condition: ["BULLISH", "BULLISH"] }),
    ).toThrow("unique");
    expect(() =>
      parseOwnerBetaEquitySignalsSearchParams({ return_20_min: "0.3", return_20_max: "0.2" }),
    ).toThrow("maximum");
  });

  it("uses the three exact read-only paths and sends the screen POST without CSRF or idempotency", async () => {
    const requests: Request[] = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      requests.push(request);
      if (request.method === "GET" && request.url.endsWith("/latest")) {
        return Response.json(latest);
      }
      if (request.method === "POST" && request.url.endsWith("/screen")) {
        return Response.json(screen);
      }
      if (request.method === "GET" && request.url.endsWith("/000001.KRX")) {
        return Response.json(detail);
      }
      throw new Error(`unexpected request ${request.method} ${request.url}`);
    };
    const client = createProductApiClient({ baseUrl: "https://api.internal", fetcher });
    const body = ownerBetaEquitySignalsScreenBody(
      parseOwnerBetaEquitySignalsSearchParams({ return_20_min: "0.1" }),
    );

    await expect(client.getOwnerBetaEquitySignalsLatest()).resolves.toEqual(latest);
    await expect(client.screenOwnerBetaEquitySignals(body)).resolves.toEqual(screen);
    await expect(client.getOwnerBetaEquitySignalDetail("000001.KRX")).resolves.toEqual(detail);

    expect(requests.map((request) => `${request.method} ${new URL(request.url).pathname}`)).toEqual(
      [
        "GET /api/v1/research/owner-beta/equity-price-signals/latest",
        "POST /api/v1/research/owner-beta/equity-price-signals/screen",
        "GET /api/v1/research/owner-beta/equity-price-signals/instruments/000001.KRX",
      ],
    );
    expect(requests[1]?.headers.get("x-csrf-token")).toBeNull();
    expect(requests[1]?.headers.get("idempotency-key")).toBeNull();
    await expect(requests[1]?.clone().json()).resolves.toEqual(body);
  });
});
