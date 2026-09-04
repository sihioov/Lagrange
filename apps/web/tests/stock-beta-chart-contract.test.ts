import { describe, expect, it } from "vitest";
import { createProductApiClient } from "@/lib/api/product-client";
import { getOwnerEquityV2Chart } from "@/lib/products/equity-signals-client";
import {
  assertOwnerEquityV2ChartMatchesExpectation,
  OwnerEquityV2ChartIntegrityError,
  ownerEquityV2ChartMatchesExpectation,
  ownerEquityV2ChartPath,
  ownerEquityV2ChartSchema,
} from "@/lib/products/equity-signals-contracts";

const SNAPSHOT_ID = "00000000-0000-4000-8000-000000000101";
const INSTRUMENT_ID = "005930.KRX";

function validChart() {
  return {
    as_of: "2026-09-03",
    bars: [
      {
        close: 71_000,
        high: 71_500,
        low: 70_500,
        open: 70_800,
        session_date: "2026-09-02",
        sma_20: null,
        sma_60: null,
        volume: 10_000,
      },
      {
        close: 72_100,
        high: 72_500,
        low: 71_600,
        open: 71_800,
        session_date: "2026-09-03",
        sma_20: 71_425.5,
        sma_60: 69_882.25,
        volume: 12_345_678,
      },
    ],
    expected_as_of: "2026-09-03",
    freshness: "CURRENT",
    generation: 3,
    instrument_id: INSTRUMENT_ID,
    latest: {
      change: 1_100,
      change_rate: 0.0069832402,
      close: 72_100,
      session_date: "2026-09-03",
      volume: 12_345_678,
    },
    price_semantics: "ORIGINAL_UNADJUSTED",
    range: "1y",
    snapshot_id: SNAPSHOT_ID,
    warnings: ["NOT_REALTIME", "CORPORATE_ACTIONS_NOT_ADJUSTED", "RESEARCH_ONLY"],
  };
}

describe("owner equity V2 chart contract", () => {
  it("accepts a valid strict chart payload", () => {
    expect(ownerEquityV2ChartSchema.parse(validChart())).toEqual(validChart());
  });

  it.each([
    ["unknown root fields", () => ({ ...validChart(), extra: true })],
    ["closed range", () => ({ ...validChart(), range: "2y" })],
    ["closed freshness", () => ({ ...validChart(), freshness: "LIVE" })],
    [
      "closed warning",
      () => ({ ...validChart(), warnings: ["NOT_REALTIME", "UNKNOWN", "RESEARCH_ONLY"] }),
    ],
    ["warning cardinality", () => ({ ...validChart(), warnings: ["NOT_REALTIME"] })],
    ["invalid ISO date", () => ({ ...validChart(), as_of: "2026-02-30" })],
    [
      "unsafe OHLC integer",
      () => ({
        ...validChart(),
        bars: [
          { ...validChart().bars[0], open: Number.MAX_SAFE_INTEGER + 1 },
          validChart().bars[1],
        ],
      }),
    ],
    [
      "negative volume",
      () => ({
        ...validChart(),
        bars: [{ ...validChart().bars[0], volume: -1 }, validChart().bars[1]],
      }),
    ],
    [
      "non-finite derived value",
      () => ({
        ...validChart(),
        latest: { ...validChart().latest, change_rate: Number.POSITIVE_INFINITY },
      }),
    ],
    [
      "reversed bars",
      () => ({ ...validChart(), bars: [validChart().bars[1], validChart().bars[0]] }),
    ],
    [
      "duplicate bars",
      () => ({
        ...validChart(),
        bars: [validChart().bars[0], { ...validChart().bars[1], session_date: "2026-09-02" }],
      }),
    ],
    [
      "invalid high-low invariant",
      () => ({
        ...validChart(),
        bars: [{ ...validChart().bars[0], high: 70_700 }, validChart().bars[1]],
      }),
    ],
    [
      "missing as-of bar",
      () => ({
        ...validChart(),
        bars: [
          { ...validChart().bars[0], session_date: "2026-09-01" },
          { ...validChart().bars[1], session_date: "2026-09-02" },
        ],
      }),
    ],
    [
      "latest date mismatch",
      () => ({ ...validChart(), latest: { ...validChart().latest, session_date: "2026-09-02" } }),
    ],
    [
      "latest OHLCV mismatch",
      () => ({ ...validChart(), latest: { ...validChart().latest, close: 72_000 } }),
    ],
  ])("rejects %s", (_name, makePayload) => {
    expect(ownerEquityV2ChartSchema.safeParse(makePayload()).success).toBe(false);
  });

  it("checks every snapshot-pinned request identity field", () => {
    const chart = ownerEquityV2ChartSchema.parse(validChart());
    const expectation = {
      asOf: chart.as_of,
      generation: chart.generation,
      instrumentId: chart.instrument_id,
      range: chart.range,
      snapshotId: chart.snapshot_id,
    };
    expect(ownerEquityV2ChartMatchesExpectation(chart, expectation)).toBe(true);
    for (const changed of [
      { ...expectation, asOf: "2026-09-02" },
      { ...expectation, generation: 4 },
      { ...expectation, instrumentId: "000005.KRX" },
      { ...expectation, range: "1m" as const },
      { ...expectation, snapshotId: "00000000-0000-4000-8000-000000000102" },
    ]) {
      expect(ownerEquityV2ChartMatchesExpectation(chart, changed)).toBe(false);
      expect(() => assertOwnerEquityV2ChartMatchesExpectation(chart, changed)).toThrow(
        OwnerEquityV2ChartIntegrityError,
      );
    }
  });

  it("builds the encoded V2 chart URL exactly", () => {
    expect(ownerEquityV2ChartPath(INSTRUMENT_ID, SNAPSHOT_ID, "1y")).toBe(
      "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/005930.KRX/chart?snapshot_id=00000000-0000-4000-8000-000000000101&range=1y",
    );
    expect(ownerEquityV2ChartPath("000001.KRX", "uuid/value?", "1m")).toContain(
      "/000001.KRX/chart?snapshot_id=uuid%2Fvalue%3F&range=1m",
    );
  });

  it("uses no-store, same-origin credentials, and the caller AbortSignal in the browser", async () => {
    const controller = new AbortController();
    let requestUrl: string | URL | Request | undefined;
    let requestInit: RequestInit | undefined;
    const fetcher: typeof fetch = async (input, init) => {
      requestUrl = input;
      requestInit = init;
      return Response.json(validChart());
    };

    await expect(
      getOwnerEquityV2Chart(INSTRUMENT_ID, SNAPSHOT_ID, "1y", {
        fetcher,
        origin: "https://app.test",
        signal: controller.signal,
      }),
    ).resolves.toEqual(validChart());
    expect(String(requestUrl)).toBe(
      "https://app.test/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/005930.KRX/chart?snapshot_id=00000000-0000-4000-8000-000000000101&range=1y",
    );
    expect(requestInit).toMatchObject({
      cache: "no-store",
      credentials: "same-origin",
      signal: controller.signal,
    });
  });

  it("uses the same exact V2 URL through the server product client", async () => {
    let request: Request | undefined;
    const fetcher: typeof fetch = async (input, init) => {
      request = new Request(input, init);
      return Response.json(validChart());
    };
    const client = createProductApiClient({ baseUrl: "https://api.internal", fetcher });

    await expect(client.getOwnerEquityV2Chart(INSTRUMENT_ID, SNAPSHOT_ID, "1y")).resolves.toEqual(
      validChart(),
    );
    expect(request?.url).toBe(
      "https://api.internal/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/005930.KRX/chart?snapshot_id=00000000-0000-4000-8000-000000000101&range=1y",
    );
    expect(request?.cache).toBe("no-store");
    expect(request?.credentials).toBe("omit");
  });

  it("maps typed chart failures in both browser and server product clients", async () => {
    const unavailable = {
      error: {
        code: "OWNER_EQUITY_CHART_UNAVAILABLE",
        message: "typed test failure",
        request_id: "request-test",
      },
    };
    const browserFetcher: typeof fetch = async () => Response.json(unavailable, { status: 503 });
    await expect(
      getOwnerEquityV2Chart(INSTRUMENT_ID, SNAPSHOT_ID, "1y", { fetcher: browserFetcher }),
    ).rejects.toMatchObject({ code: "OWNER_EQUITY_CHART_UNAVAILABLE", status: 503 });

    const serverFetcher: typeof fetch = async () => Response.json(unavailable, { status: 503 });
    const serverClient = createProductApiClient({
      baseUrl: "https://api.internal",
      fetcher: serverFetcher,
    });
    await expect(
      serverClient.getOwnerEquityV2Chart(INSTRUMENT_ID, SNAPSHOT_ID, "1y"),
    ).rejects.toMatchObject({ code: "OWNER_EQUITY_CHART_UNAVAILABLE", status: 503 });
  });
});
