import { describe, expect, it } from "vitest";
import { createProductApiClient } from "@/lib/api/product-client";
import {
  addOwnerEquityV2Membership,
  disableOwnerEquityV2Membership,
  retryOwnerEquityV2Membership,
} from "@/lib/products/equity-signals-client";
import {
  type OwnerEquityV2LatestSignalsModel,
  type OwnerEquityV2MembershipListModel,
  ownerBetaEquitySignalsDetailSchema,
  ownerBetaEquitySignalsLatestSchema,
  ownerBetaEquitySignalsScreenBody,
  ownerBetaEquitySignalsScreenBodySchema,
  ownerBetaEquitySignalsScreenSchema,
  ownerEquityV2AddBodySchema,
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2MembershipStatusSchema,
  ownerEquityV2MutationSchema,
  ownerEquityV2ScreenBodySchema,
  ownerEquityV2SignalDetailSchema,
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

const V2_IDS = Array.from(
  { length: 100 },
  (_, index) => `${String(index + 1).padStart(6, "0")}.KRX`,
);
const V2_SHA = `sha256:${"b".repeat(64)}`;
const V2_SNAPSHOT = {
  as_of: "2026-08-29",
  published_at: "2026-08-30T06:00:00Z",
  row_count: V2_IDS.length,
  snapshot_id: "00000000-0000-4000-8000-000000000501",
  universe_sha256: V2_SHA,
};

function v2Row(index: number) {
  return {
    average_trading_value_20: 2_000_000 + index,
    average_volume_20: 200_000 + index,
    condition: index % 3 === 0 ? "BULLISH" : index % 3 === 1 ? "NEUTRAL" : "BEARISH",
    generation: 1,
    instrument_id: V2_IDS[index] as string,
    max_drawdown_120: -0.08 - index / 1_000,
    rank: index + 1,
    return_120: 0.32 - index / 100,
    return_20: 0.08 - index / 1_000,
    return_60: 0.16 - index / 1_000,
    score: 80 - index / 10,
    sma_20: 120 + index,
    sma_60: 118 + index,
    volatility_120: 0.18 + index / 1_000,
    volatility_20: 0.09 + index / 1_000,
    volatility_60: 0.13 + index / 1_000,
    volume_ratio_20_60: 1.1 + index / 1_000,
  } as const;
}

const v2Rows = V2_IDS.map((_, index) => v2Row(index));
const v2Latest: OwnerEquityV2LatestSignalsModel = ownerEquityV2LatestSignalsSchema.parse({
  rows: v2Rows,
  snapshot: V2_SNAPSHOT,
  top5: v2Rows.slice(0, 5),
});
const v2MembershipList: OwnerEquityV2MembershipListModel = ownerEquityV2MembershipListSchema.parse({
  memberships: [
    {
      coverage: {
        first_session: "2025-08-01",
        last_session: "2026-08-29",
        minimum_observed_sessions: 121,
        observed_sessions: 261,
        target_observed_sessions: 261,
      },
      generation: 1,
      id: "00000000-0000-4000-8000-000000000601",
      instrument_id: "000001.KRX",
      lifecycle: "READY",
      requested_at: "2026-08-29T06:00:00Z",
      updated_at: "2026-08-30T06:00:00Z",
    },
  ],
  policy: {
    active_instruments: 1,
    max_active_instruments: 100,
    minimum_observed_sessions: 121,
    remaining_capacity: 99,
    target_observed_sessions: 261,
  },
});
const v2Status = ownerEquityV2MembershipStatusSchema.parse({
  membership: v2MembershipList.memberships[0],
  policy: v2MembershipList.policy,
});
const v2Mutation = ownerEquityV2MutationSchema.parse({
  duplicate_active: false,
  job_id: "00000000-0000-4000-8000-000000000701",
  resource: v2MembershipList.memberships[0],
});

const v2RequestedMutation = ownerEquityV2MutationSchema.parse({
  duplicate_active: false,
  job_id: "00000000-0000-4000-8000-000000000702",
  resource: {
    coverage: {
      minimum_observed_sessions: 121,
      observed_sessions: 0,
      target_observed_sessions: 261,
    },
    generation: 0,
    id: "00000000-0000-4000-8000-000000000602",
    instrument_id: "000002.KRX",
    lifecycle: "REQUESTED",
    requested_at: "2026-08-30T06:00:00Z",
    updated_at: "2026-08-30T06:00:00Z",
  },
});
const v2Detail = ownerEquityV2SignalDetailSchema.parse({
  signal: v2Rows[99],
  snapshot: V2_SNAPSHOT,
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

  it("accepts the V2 contract at policy-defined capacity, including 100 rows and ranks above 30", () => {
    expect(v2Latest.rows).toHaveLength(100);
    expect(v2Latest.rows[99]?.rank).toBe(100);
    expect(v2MembershipList.policy.max_active_instruments).toBe(100);
    expect(v2RequestedMutation.resource.generation).toBe(0);
    expect(v2RequestedMutation.resource.lifecycle).toBe("REQUESTED");
    expect(
      ownerEquityV2LatestSignalsSchema.safeParse({
        ...v2Latest,
        unknown: true,
      }).success,
    ).toBe(false);
    expect(
      ownerEquityV2LatestSignalsSchema.safeParse({
        ...v2Latest,
        rows: [{ ...v2Rows[0], rank: 0 }, ...v2Rows.slice(1)],
      }).success,
    ).toBe(false);
  });

  it("requires an exact six-digit V2 add body and rejects undocumented fields", () => {
    expect(ownerEquityV2AddBodySchema.safeParse({ instrument_code: "005930" }).success).toBe(true);
    for (const instrument_code of ["5930", "0059300", "00593A", " 005930"]) {
      expect(ownerEquityV2AddBodySchema.safeParse({ instrument_code }).success).toBe(false);
    }
    expect(
      ownerEquityV2AddBodySchema.safeParse({ instrument_code: "005930", name: "not accepted" })
        .success,
    ).toBe(false);
    expect(
      ownerEquityV2ScreenBodySchema.safeParse({
        conditions: ["BULLISH", "BULLISH"],
      }).success,
    ).toBe(false);
  });

  it("uses the exact V2 read paths and validates membership, screen, latest, and detail payloads", async () => {
    const requests: Request[] = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      requests.push(request);
      const path = new URL(request.url).pathname;
      if (request.method === "GET" && path.endsWith("/memberships")) {
        return Response.json(v2MembershipList);
      }
      if (request.method === "GET" && path.endsWith("/00000000-0000-4000-8000-000000000601")) {
        return Response.json(v2Status);
      }
      if (request.method === "GET" && path.endsWith("/signals/latest")) {
        return Response.json(v2Latest);
      }
      if (request.method === "POST" && path.endsWith("/signals/screen")) {
        return Response.json({ snapshot: V2_SNAPSHOT, rows: v2Rows.slice(0, 1) });
      }
      if (request.method === "GET" && path.endsWith("/signals/instruments/000100.KRX")) {
        return Response.json(v2Detail);
      }
      throw new Error(`unexpected V2 request ${request.method} ${path}`);
    };
    const client = createProductApiClient({ baseUrl: "https://api.internal", fetcher });

    await expect(client.getOwnerEquityV2Memberships()).resolves.toEqual(v2MembershipList);
    await expect(
      client.getOwnerEquityV2MembershipStatus("00000000-0000-4000-8000-000000000601"),
    ).resolves.toEqual(v2Status);
    await expect(client.getOwnerEquityV2LatestSignals()).resolves.toEqual(v2Latest);
    await expect(
      client.screenOwnerEquityV2Signals({
        conditions: ["BULLISH"],
        instrument_ids: ["000100.KRX"],
      }),
    ).resolves.toEqual({ snapshot: V2_SNAPSHOT, rows: v2Rows.slice(0, 1) });
    await expect(client.getOwnerEquityV2SignalDetail("000100.KRX")).resolves.toEqual(v2Detail);

    expect(requests.map((request) => `${request.method} ${new URL(request.url).pathname}`)).toEqual(
      [
        "GET /api/v1/research/owner-beta/equity-universe-v2/memberships",
        "GET /api/v1/research/owner-beta/equity-universe-v2/memberships/00000000-0000-4000-8000-000000000601",
        "GET /api/v1/research/owner-beta/equity-universe-v2/signals/latest",
        "POST /api/v1/research/owner-beta/equity-universe-v2/signals/screen",
        "GET /api/v1/research/owner-beta/equity-universe-v2/signals/instruments/000100.KRX",
      ],
    );
  });

  it("sends CSRF and idempotency headers for V2 add, retry, and soft-disable mutations", async () => {
    const requests: Request[] = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      requests.push(request);
      const path = new URL(request.url).pathname;
      if (request.method === "GET" && path.endsWith("/auth/csrf")) {
        return Response.json({ csrf_token: "synthetic-csrf" });
      }
      if (request.method === "POST" && path.includes("equity-universe-v2")) {
        return Response.json(v2Mutation);
      }
      throw new Error(`unexpected V2 mutation ${request.method} ${path}`);
    };

    await expect(
      addOwnerEquityV2Membership(
        { instrument_code: "005930" },
        {
          fetcher,
          origin: "https://app.test",
        },
      ),
    ).resolves.toEqual(v2Mutation);
    await expect(
      retryOwnerEquityV2Membership(v2Mutation.resource.id, {
        fetcher,
        origin: "https://app.test",
      }),
    ).resolves.toEqual(v2Mutation);
    await expect(
      disableOwnerEquityV2Membership(v2Mutation.resource.id, {
        fetcher,
        origin: "https://app.test",
      }),
    ).resolves.toEqual(v2Mutation);

    expect(requests.filter((request) => request.method === "POST")).toHaveLength(3);
    for (const request of requests.filter((item) => item.method === "POST")) {
      expect(request.headers.get("x-csrf-token")).toBe("synthetic-csrf");
      expect(request.headers.get("idempotency-key")).toBeTruthy();
      await expect(request.clone().json()).resolves.toEqual(
        request.url.endsWith("/memberships") ? { instrument_code: "005930" } : {},
      );
    }
    expect(requests.map((request) => new URL(request.url).pathname)).toEqual([
      "/api/v1/auth/csrf",
      "/api/v1/research/owner-beta/equity-universe-v2/memberships",
      "/api/v1/auth/csrf",
      "/api/v1/research/owner-beta/equity-universe-v2/memberships/00000000-0000-4000-8000-000000000601/retry",
      "/api/v1/auth/csrf",
      "/api/v1/research/owner-beta/equity-universe-v2/memberships/00000000-0000-4000-8000-000000000601/disable",
    ]);
  });
});
