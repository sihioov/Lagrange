import { describe, expect, it } from "vitest";
import {
  assertIntradayQuoteIdempotencyKey,
  getIntradayQuote,
  type IntradayQuoteApiError,
} from "@/lib/products/intraday-quotes-client";
import {
  INTRADAY_QUOTE_DEMAND_PATH,
  intradayQuoteDemandPath,
  intradayQuoteDemandRequestSchema,
  intradayQuotePath,
  intradayQuoteReleaseRequestSchema,
  intradayQuoteResponseSchema,
  isCanonicalIntradayDecimal,
  isIntradayQuoteIdentityMatch,
  parseIntradayQuoteResponse,
} from "@/lib/products/intraday-quotes-contracts";

const MEMBERSHIP_ID = "00000000-0000-4000-8000-000000000001";
const CONSUMER_ID = "00000000-0000-4000-8000-000000000002";
const DEMAND_ID = "00000000-0000-4000-8000-000000000003";
const IDENTITY = {
  generation: 7,
  instrument_id: "069500.KRX",
  membership_id: MEMBERSHIP_ID,
} as const;
const NOW_MS = Date.parse("2026-09-08T03:00:00Z");

function quoteResponse(overrides: Record<string, unknown> = {}) {
  return {
    currency: "KRW",
    freshness: "RECENT",
    generation: 7,
    instrument_id: "069500.KRX",
    market_state: "OPEN",
    membership_id: MEMBERSHIP_ID,
    next_poll_after_ms: 5_000,
    quote: {
      base_price: "100000",
      change_from_previous_day: "1200",
      change_percent_from_previous_day: "1.2",
      direction: "UP",
      last_success_at: "2026-09-08T02:59:00Z",
      price: "101200.00",
      quote_version: "9223372036854775807",
      received_at: "2026-09-08T02:59:00Z",
    },
    reason_code: null,
    schema_version: 1,
    session: {
      calendar_content_sha256: "a".repeat(64),
      calendar_source: "kis",
      calendar_source_version: "kis-chk-holiday-v1:schema-1",
      date: "2026-09-08",
      timezone: "Asia/Seoul",
      window_contract_sha256: `sha256:${"b".repeat(64)}`,
    },
    venue: "KRX",
    ...overrides,
  };
}

describe("Stock Beta intraday quote application contract", () => {
  it("keeps exact routes, canonical decimal strings, and bigint quote versions", () => {
    expect(INTRADAY_QUOTE_DEMAND_PATH).toBe(
      "/api/v1/research/owner-beta/equity-universe-v2/quote-demands",
    );
    expect(intradayQuotePath(IDENTITY)).toBe(
      "/api/v1/research/owner-beta/equity-universe-v2/instruments/069500.KRX/quote?membership_id=00000000-0000-4000-8000-000000000001&generation=7",
    );
    expect(intradayQuoteDemandPath(DEMAND_ID)).toBe(`${INTRADAY_QUOTE_DEMAND_PATH}/${DEMAND_ID}`);
    expect(isCanonicalIntradayDecimal("0")).toBe(true);
    expect(isCanonicalIntradayDecimal("101200.00")).toBe(true);
    for (const invalid of [
      "+1",
      " 1",
      "1 ",
      "01",
      "1e3",
      "-0",
      "-0.00",
      `${"1".repeat(13)}`,
      "1.123456789",
    ]) {
      expect(isCanonicalIntradayDecimal(invalid)).toBe(false);
    }
    expect(intradayQuoteResponseSchema.parse(quoteResponse()).quote?.quote_version).toBe(
      "9223372036854775807",
    );
    expect(parseIntradayQuoteResponse(quoteResponse(), { nowMs: NOW_MS }).quote?.price).toBe(
      "101200.00",
    );
  });

  it("rejects identity, sign, session, timestamp, and quote-version drift", () => {
    const wrongInstrument = intradayQuoteResponseSchema.parse(
      quoteResponse({ instrument_id: "069501.KRX" }),
    );
    expect(isIntradayQuoteIdentityMatch(wrongInstrument, IDENTITY)).toBe(false);
    expect(() =>
      intradayQuoteResponseSchema.parse(
        quoteResponse({
          quote: {
            ...quoteResponse().quote,
            change_from_previous_day: "-1200",
          },
        }),
      ),
    ).toThrow();
    expect(() =>
      intradayQuoteResponseSchema.parse(
        quoteResponse({
          market_state: "UNKNOWN",
          quote: quoteResponse().quote,
          session: null,
        }),
      ),
    ).toThrow();
    expect(() =>
      parseIntradayQuoteResponse(quoteResponse(), {
        nowMs: Date.parse("2026-09-09T00:00:00Z"),
      }),
    ).toThrow();
    expect(() =>
      intradayQuoteResponseSchema.parse(
        quoteResponse({
          quote: { ...quoteResponse().quote, quote_version: "9223372036854775808" },
        }),
      ),
    ).toThrow();
    expect(() =>
      intradayQuoteDemandRequestSchema.parse({
        consumer_id: CONSUMER_ID,
        generation: Number.MAX_SAFE_INTEGER + 1,
        membership_id: MEMBERSHIP_ID,
        renewal_sequence: 0,
        schema_version: 1,
      }),
    ).toThrow();
    expect(() =>
      intradayQuoteReleaseRequestSchema.parse({
        consumer_id: CONSUMER_ID,
        renewal_sequence: -1,
        schema_version: 1,
      }),
    ).toThrow();
  });

  it("accepts only visible ASCII idempotency keys and never exposes typed error prose", async () => {
    expect(assertIntradayQuoteIdempotencyKey("attempt-0/abc")).toBe("attempt-0/abc");
    for (const invalid of ["", "has:colon", "has\\slash", "has space", "é"]) {
      expect(() => assertIntradayQuoteIdempotencyKey(invalid)).toThrow();
    }

    const fetcher: typeof fetch = async () =>
      new Response(
        JSON.stringify({
          error: {
            code: "QUOTE_CACHE_UNAVAILABLE",
            details: {},
            message: "broker prose must not cross the client boundary",
            request_id: "request-1",
          },
        }),
        { status: 503, headers: { "Retry-After": "15" } },
      );
    await expect(getIntradayQuote(IDENTITY, { fetcher, nowMs: NOW_MS })).rejects.toMatchObject({
      code: "QUOTE_CACHE_UNAVAILABLE",
      retryAfterMs: 15_000,
    } satisfies Partial<IntradayQuoteApiError>);
    await expect(getIntradayQuote(IDENTITY, { fetcher, nowMs: NOW_MS })).rejects.not.toThrow(
      /broker prose/,
    );
  });
});
