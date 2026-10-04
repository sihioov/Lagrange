import { afterEach, describe, expect, it, vi } from "vitest";
import { createIntradayStreamClient } from "@/lib/products/intraday-stream-client";
import {
  canonicalStreamIdentities,
  type IntradayStreamLeaseRequest,
  intradayStreamEventPath,
  intradayStreamLeaseRequestSchema,
  intradayStreamReleaseRequestSchema,
  matchStreamLease,
} from "@/lib/products/intraday-stream-lease-contracts";

function must<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("Expected fixture value is missing");
  return value;
}

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const origin = "https://stream.example.test";
const request = (count = 1): IntradayStreamLeaseRequest => ({
  schema_version: 2,
  consumer_id: uuid(100),
  renewal_sequence: 0,
  identities: Array.from({ length: count }, (_, index) => ({
    membership_id: uuid(index + 1),
    instrument_id: `${String(index + 1).padStart(6, "0")}.KRX`,
    generation: 7,
  })),
});
const lease = (input = request()) => ({
  ...input,
  lease_id: uuid(101),
  lease_expires_at: "2026-10-03T03:00:30.000Z",
  renew_after_ms: 15_000,
});
const options = () => ({ signal: new AbortController().signal, idempotencyKey: uuid(102) });

afterEach(() => vi.useRealTimers());

describe("schema-2 lease contract", () => {
  it("accepts sequence zero and thirty canonical identities, rejects unsafe numbers and unknown authority", () => {
    expect(intradayStreamLeaseRequestSchema.safeParse(request(30)).success).toBe(true);
    for (const bad of [
      request(31),
      { ...request(), renewal_sequence: -1 },
      { ...request(), renewal_sequence: Number.MAX_SAFE_INTEGER + 1 },
      { ...request(), owner_user_id: uuid(900) },
      { ...request(), identities: [{ ...request().identities[0], generation: 0 }] },
    ])
      expect(intradayStreamLeaseRequestSchema.safeParse(bad).success).toBe(false);
    expect(
      intradayStreamReleaseRequestSchema.safeParse({
        schema_version: 2,
        consumer_id: uuid(100),
        renewal_sequence: 0,
      }).success,
    ).toBe(true);
  });

  it("orders a copied set, but refuses duplicates and never silently discards an identity", () => {
    const values = [...request(3).identities].reverse();
    expect(canonicalStreamIdentities(values)).toEqual(request(3).identities);
    expect(values[0]?.membership_id).toBe(uuid(3));
    expect(
      intradayStreamLeaseRequestSchema.safeParse({ ...request(), identities: values }).success,
    ).toBe(false);
    expect(() =>
      canonicalStreamIdentities([must(request().identities[0]), must(request().identities[0])]),
    ).toThrow();
    expect(() =>
      canonicalStreamIdentities([
        must(request().identities[0]),
        { ...must(request().identities[0]), membership_id: uuid(2) },
      ]),
    ).toThrow();
  });

  it("binds every response field to the requested consumer, sequence, set and prior lease", () => {
    expect(matchStreamLease(lease(), request(), uuid(101))).toEqual(lease());
    for (const bad of [
      { ...lease(), consumer_id: uuid(999) },
      { ...lease(), renewal_sequence: 1 },
      { ...lease(), lease_id: uuid(999) },
      { ...lease(), identities: request(2).identities },
      { ...lease(), renew_after_ms: 1 },
      { ...lease(), lease_expires_at: "2026-02-30T00:00:00Z" },
      { ...lease(), source: "REST" },
    ])
      expect(() => matchStreamLease(bad, request(), uuid(101))).toThrow();
    expect(intradayStreamEventPath(uuid(101))).toBe(
      `/api/v1/research/owner-beta/equity-universe-v2/market-stream?lease_id=${uuid(101)}`,
    );
    expect(() => intradayStreamEventPath(`${uuid(101)}&owner=other`)).toThrow();
  });
});

describe("bounded same-origin stream mutations", () => {
  it("uses existing CSRF preflight, strict cookie-only POST and a single DELETE with the accepted sequence", async () => {
    const calls: Array<{ url: string; init: RequestInit }> = [];
    const fetcher = vi.fn<typeof fetch>(async (url, init = {}) => {
      calls.push({ url: String(url), init });
      if (String(url).endsWith("/auth/csrf"))
        return Response.json({ csrf_token: "synthetic-csrf" });
      if (init.method === "DELETE")
        return Response.json({ schema_version: 2, lease_id: uuid(101), released: true });
      return Response.json(lease());
    });
    const client = createIntradayStreamClient({ fetcher, origin });
    await expect(client.replaceLease(request(), options())).resolves.toEqual(lease());
    await client.releaseLease(
      uuid(101),
      { schema_version: 2, consumer_id: uuid(100), renewal_sequence: 0 },
      options(),
    );
    expect(calls.map(({ init }) => init.method)).toEqual(["GET", "POST", "GET", "DELETE"]);
    for (const { url, init } of calls) {
      expect(url.startsWith(`${origin}/api/v1/`)).toBe(true);
      expect(init).toMatchObject({
        mode: "same-origin",
        credentials: "same-origin",
        cache: "no-store",
        redirect: "error",
      });
      expect(url).not.toContain("synthetic-csrf");
    }
    expect(new Headers(calls[1]?.init.headers).get("x-csrf-token")).toBe("synthetic-csrf");
    expect(new Headers(calls[1]?.init.headers).get("Idempotency-Key")).toBe(uuid(102));
    expect(JSON.parse(String(calls[1]?.init.body))).toEqual(request());
    expect(JSON.parse(String(calls[3]?.init.body)).renewal_sequence).toBe(0);
  });

  it("rejects malformed input before CSRF or a mutation can be sent", async () => {
    const fetcher = vi.fn<typeof fetch>();
    const client = createIntradayStreamClient({ fetcher, origin });
    await expect(client.replaceLease(request(31), options())).rejects.toThrow();
    await expect(
      client.replaceLease(request(), { ...options(), idempotencyKey: "bad\nheader" }),
    ).rejects.toThrow();
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("fails closed on unexpected success schemas and does not expose a raw response message", async () => {
    const fetcher = vi.fn<typeof fetch>(async (url) =>
      String(url).endsWith("/auth/csrf")
        ? Response.json({ csrf_token: "synthetic-csrf" })
        : Response.json(
            { error: { code: "FEATURE_DISABLED", message: "private provider detail" } },
            { status: 503 },
          ),
    );
    const client = createIntradayStreamClient({ fetcher, origin });
    const failure = await client
      .replaceLease(request(), options())
      .catch((error: unknown) => error);
    expect(failure).toMatchObject({ kind: "feature_disabled" });
    expect(String(failure)).not.toContain("private provider detail");
    expect(fetcher).toHaveBeenCalledTimes(2);
    fetcher.mockImplementation(async (url) =>
      String(url).endsWith("/auth/csrf")
        ? Response.json({ csrf_token: "synthetic-csrf" })
        : Response.json({ ...lease(), credentials: "forbidden" }),
    );
    await expect(client.replaceLease(request(), options())).rejects.toThrow();
    expect(fetcher).toHaveBeenCalledTimes(4);
  });

  it("bounds chunked JSON without a Content-Length and cancels the reader on overflow", async () => {
    const cancel = vi.fn();
    const fetcher = vi.fn<typeof fetch>(async (url) => {
      if (String(url).endsWith("/auth/csrf"))
        return Response.json({ csrf_token: "synthetic-csrf" });
      return new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(new Uint8Array(16_385));
          },
          cancel,
        }),
        { headers: { "Content-Type": "application/json" } },
      );
    });
    await expect(
      createIntradayStreamClient({ fetcher, origin }).replaceLease(request(), options()),
    ).rejects.toMatchObject({ kind: "invalid_response" });
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("shares a five-second deadline across CSRF and mutation instead of restarting it", async () => {
    vi.useFakeTimers();
    const signals: AbortSignal[] = [];
    const fetcher = vi.fn<typeof fetch>((url, init) => {
      const signal = init?.signal as AbortSignal;
      signals.push(signal);
      return new Promise((resolve, reject) => {
        signal.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), {
          once: true,
        });
        if (String(url).endsWith("/auth/csrf"))
          setTimeout(() => resolve(Response.json({ csrf_token: "synthetic-csrf" })), 4_000);
      });
    });
    const result = createIntradayStreamClient({ fetcher, origin })
      .replaceLease(request(), options())
      .catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(4_001);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[0]).toBe(signals[1]);
    await vi.advanceTimersByTimeAsync(999);
    expect(await result).toMatchObject({ kind: "timeout" });
    expect(signals[1]?.aborted).toBe(true);
  });

  it("aborts a stalled response body and sends no mutation after cancelled CSRF", async () => {
    vi.useFakeTimers();
    const cancel = vi.fn();
    const fetcher = vi.fn<typeof fetch>(
      async () =>
        new Response(new ReadableStream({ cancel }), {
          headers: { "Content-Type": "application/json" },
        }),
    );
    const abort = new AbortController();
    const result = createIntradayStreamClient({ fetcher, origin })
      .replaceLease(request(), { ...options(), signal: abort.signal })
      .catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(1);
    abort.abort();
    expect(await result).toMatchObject({ kind: "aborted" });
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });
});
