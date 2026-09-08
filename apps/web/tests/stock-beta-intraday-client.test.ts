import { describe, expect, it } from "vitest";
import {
  createIntradayQuoteDemand,
  getIntradayQuote,
  releaseIntradayQuoteDemand,
} from "@/lib/products/intraday-quotes-client";
import {
  intradayQuoteDemandResponseSchema,
  intradayQuoteResponseSchema,
} from "@/lib/products/intraday-quotes-contracts";

const IDENTITY = {
  generation: 7,
  instrument_id: "069500.KRX",
  membership_id: "00000000-0000-4000-8000-000000000001",
} as const;
const CONSUMER_ID = "00000000-0000-4000-8000-000000000002";
const DEMAND_ID = "00000000-0000-4000-8000-000000000003";

function demand(): Record<string, unknown> {
  return {
    consumer_id: CONSUMER_ID,
    demand_id: DEMAND_ID,
    generation: 7,
    instrument_id: IDENTITY.instrument_id,
    lease_expires_at: "2026-09-08T03:00:30Z",
    membership_id: IDENTITY.membership_id,
    renew_after_ms: 15_000,
    renewal_sequence: 0,
    schema_version: 1,
  };
}

function quote(): ReturnType<typeof intradayQuoteResponseSchema.parse> {
  return intradayQuoteResponseSchema.parse({
    currency: "KRW",
    freshness: "RECENT",
    generation: 7,
    instrument_id: IDENTITY.instrument_id,
    market_state: "OPEN",
    membership_id: IDENTITY.membership_id,
    next_poll_after_ms: 5_000,
    quote: {
      base_price: "100000",
      change_from_previous_day: "1200",
      change_percent_from_previous_day: "1.2",
      direction: "UP",
      last_success_at: "2026-09-08T02:59:00Z",
      price: "101200.00",
      quote_version: "1",
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
  });
}

function quoteAt(timestamp: string) {
  const response = quote();
  if (response.quote === null) throw new Error("fixture quote is required");
  return {
    ...response,
    quote: {
      ...response.quote,
      last_success_at: timestamp,
      received_at: timestamp,
    },
  };
}

function deferred<T>(): {
  readonly promise: Promise<T>;
  readonly resolve: (value: T) => void;
} {
  let resolvePromise!: (value: T) => void;
  const promise = new Promise<T>((resolve) => {
    resolvePromise = resolve;
  });
  return { promise, resolve: resolvePromise };
}

describe("Stock Beta intraday quote browser client", () => {
  it("uses the fixed same-origin routes, CSRF mutation seam, exact body, and no-store GET", async () => {
    const demandContract = intradayQuoteDemandResponseSchema.safeParse(demand());
    if (!demandContract.success) throw new Error(JSON.stringify(demandContract.error.issues));
    const calls: Array<{
      readonly headers: Headers;
      readonly init: RequestInit;
      readonly cache: RequestCache | undefined;
      readonly credentials: RequestCredentials | undefined;
      readonly bodyText: string;
      readonly method: string | undefined;
      readonly url: string;
    }> = [];
    const fetcher: typeof fetch = async (input, init = {}) => {
      const request = input instanceof Request ? input : null;
      const url =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const method = request?.method ?? init.method;
      const bodyText =
        request === null
          ? typeof init.body === "string"
            ? init.body
            : ""
          : await request.clone().text();
      calls.push({
        bodyText,
        headers: new Headers(request?.headers ?? init.headers),
        init,
        cache: request?.cache ?? init.cache,
        credentials: request?.credentials ?? init.credentials,
        method,
        url,
      });
      if (url.endsWith("/auth/csrf"))
        return new Response(JSON.stringify({ csrf_token: "csrf" }), { status: 200 });
      if (url.endsWith("/quote-demands") && method === "POST") {
        return new Response(JSON.stringify(demand()), { status: 200 });
      }
      if (url.endsWith(`/${DEMAND_ID}`) && method === "DELETE") {
        return new Response(null, { status: 204 });
      }
      return new Response(JSON.stringify(quote()), { status: 200 });
    };

    const created = await createIntradayQuoteDemand(
      {
        consumer_id: CONSUMER_ID,
        generation: 7,
        membership_id: IDENTITY.membership_id,
        renewal_sequence: 0,
        schema_version: 1,
      },
      { fetcher, idempotencyKey: "quote-demand-0", origin: "https://app.example" },
    );
    await releaseIntradayQuoteDemand(
      DEMAND_ID,
      { consumer_id: CONSUMER_ID, renewal_sequence: 1, schema_version: 1 },
      { fetcher, idempotencyKey: "quote-release-1", origin: "https://app.example" },
    );
    const received = await getIntradayQuote(IDENTITY, {
      fetcher,
      now: () => Date.parse("2026-09-08T03:00:00Z"),
      origin: "https://app.example",
    });

    expect(created).toEqual(demand());
    expect(received).toEqual(quote());
    expect(calls.map((call) => [call.url, call.method])).toEqual([
      ["https://app.example/api/v1/auth/csrf", "GET"],
      ["https://app.example/api/v1/research/owner-beta/equity-universe-v2/quote-demands", "POST"],
      ["https://app.example/api/v1/auth/csrf", "GET"],
      [
        `https://app.example/api/v1/research/owner-beta/equity-universe-v2/quote-demands/${DEMAND_ID}`,
        "DELETE",
      ],
      [
        "https://app.example/api/v1/research/owner-beta/equity-universe-v2/instruments/069500.KRX/quote?membership_id=00000000-0000-4000-8000-000000000001&generation=7",
        undefined,
      ],
    ]);
    expect(calls[1]).toMatchObject({ cache: "no-store", credentials: "same-origin" });
    expect(JSON.parse(calls[1]?.bodyText ?? "")).toEqual({
      consumer_id: CONSUMER_ID,
      generation: 7,
      membership_id: IDENTITY.membership_id,
      renewal_sequence: 0,
      schema_version: 1,
    });
    expect(calls[1]?.headers.get("Idempotency-Key")).toBe("quote-demand-0");
    expect(calls[1]?.headers.get("X-CSRF-Token")).toBe("csrf");
    expect(JSON.parse(calls[3]?.bodyText ?? "")).toEqual({
      consumer_id: CONSUMER_ID,
      renewal_sequence: 1,
      schema_version: 1,
    });
    expect(calls[4]).toMatchObject({ cache: "no-store", credentials: "same-origin" });
    expect(calls[4]?.init.body).toBeUndefined();
  });

  it("samples the injected clock after receipt and rejects future or prior-KST-session payloads", async () => {
    const dispatchedAt = Date.parse("2026-09-08T03:00:00Z");
    let nowMs = dispatchedAt;
    const samples: number[] = [];
    const delayedResponse = deferred<Response>();
    const delayed = getIntradayQuote(IDENTITY, {
      fetcher: async () => delayedResponse.promise,
      now: () => {
        samples.push(nowMs);
        return nowMs;
      },
    });

    expect(samples).toEqual([]);
    nowMs = dispatchedAt + 2_000;
    delayedResponse.resolve(
      new Response(JSON.stringify(quoteAt(new Date(dispatchedAt + 1_000).toISOString())), {
        status: 200,
      }),
    );
    await expect(delayed).resolves.toMatchObject({ quote: { price: "101200.00" } });
    expect(samples).toEqual([dispatchedAt + 2_000]);

    const futureResponse = deferred<Response>();
    const future = getIntradayQuote(IDENTITY, {
      fetcher: async () => futureResponse.promise,
      now: () => nowMs,
    });
    futureResponse.resolve(
      new Response(JSON.stringify(quoteAt(new Date(nowMs + 1_000).toISOString())), {
        status: 200,
      }),
    );
    await expect(future).rejects.toThrow("approved contract");

    let crossingNowMs = dispatchedAt;
    const priorSessionResponse = deferred<Response>();
    const priorSession = getIntradayQuote(IDENTITY, {
      fetcher: async () => priorSessionResponse.promise,
      now: () => crossingNowMs,
    });
    crossingNowMs = Date.parse("2026-09-08T15:00:00Z");
    priorSessionResponse.resolve(new Response(JSON.stringify(quote()), { status: 200 }));
    await expect(priorSession).rejects.toThrow("approved contract");
  });
});
