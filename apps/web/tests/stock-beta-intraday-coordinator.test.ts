import { describe, expect, it, vi } from "vitest";
import {
  type IntradayQuoteClock,
  IntradayQuoteLoadCoordinator,
  type IntradayQuoteLoadPhase,
} from "@/components/stock-beta/quote/quote-load-coordinator";
import {
  IntradayQuoteApiError,
  type IntradayQuoteClient,
} from "@/lib/products/intraday-quotes-client";
import type {
  IntradayQuoteDemandRequest,
  IntradayQuoteDemandResponse,
  IntradayQuoteIdentity,
  IntradayQuoteReleaseRequest,
  IntradayQuoteResponse,
} from "@/lib/products/intraday-quotes-contracts";
import { INTRADAY_QUOTE_CACHE_MAX_AGE_MS } from "@/lib/products/intraday-quotes-contracts";

const IDENTITY_A: IntradayQuoteIdentity = {
  generation: 7,
  instrument_id: "069500.KRX",
  membership_id: "00000000-0000-4000-8000-000000000001",
};
const IDENTITY_B: IntradayQuoteIdentity = {
  generation: 8,
  instrument_id: "114800.KRX",
  membership_id: "00000000-0000-4000-8000-000000000004",
};
const NOW_MS = Date.parse("2026-09-08T03:00:00Z");

class FakeClock implements IntradayQuoteClock {
  private current = NOW_MS;
  private nextId = 0;
  private timers = new Map<number, { readonly at: number; readonly callback: () => void }>();

  now = (): number => this.current;

  setTimeout = (callback: () => void, delayMs: number): number => {
    const id = this.nextId++;
    this.timers.set(id, { at: this.current + delayMs, callback });
    return id;
  };

  clearTimeout = (handle: unknown): void => {
    if (typeof handle === "number") this.timers.delete(handle);
  };

  advance(delayMs: number): void {
    this.current += delayMs;
    while (true) {
      const due = [...this.timers.entries()]
        .filter(([, timer]) => timer.at <= this.current)
        .sort(([, left], [, right]) => left.at - right.at)[0];
      if (due === undefined) return;
      this.timers.delete(due[0]);
      due[1].callback();
    }
  }

  jump(delayMs: number): void {
    this.current += delayMs;
  }
}

async function flush(): Promise<void> {
  for (let index = 0; index < 8; index += 1) {
    await new Promise<void>((resolve) => queueMicrotask(() => resolve()));
  }
}

function demandFor(
  identity: IntradayQuoteIdentity,
  consumerId: string,
  sequence: number,
  leaseExpiresAt = "2026-09-08T03:00:30Z",
): IntradayQuoteDemandResponse {
  return {
    consumer_id: consumerId,
    demand_id: `00000000-0000-4000-8000-${String(sequence + 10).padStart(12, "0")}`,
    generation: identity.generation,
    instrument_id: identity.instrument_id,
    lease_expires_at: leaseExpiresAt,
    membership_id: identity.membership_id,
    renew_after_ms: 15_000,
    renewal_sequence: sequence,
    schema_version: 1,
  };
}

function quoteWithSuccessAt(
  identity: IntradayQuoteIdentity,
  clock: FakeClock,
  successAtMs: number,
  overrides: Partial<IntradayQuoteResponse> = {},
): IntradayQuoteResponse {
  const response = quoteFor(identity, clock, overrides);
  const successAt = new Date(successAtMs).toISOString().replace(".000", "");
  return {
    ...response,
    quote:
      response.quote === null
        ? null
        : { ...response.quote, last_success_at: successAt, received_at: successAt },
  };
}

function quoteFor(
  identity: IntradayQuoteIdentity,
  clock: FakeClock,
  overrides: Partial<IntradayQuoteResponse> = {},
): IntradayQuoteResponse {
  const received = new Date(clock.now()).toISOString().replace(".000", "");
  return {
    currency: "KRW",
    freshness: "RECENT",
    generation: identity.generation,
    instrument_id: identity.instrument_id,
    market_state: "OPEN",
    membership_id: identity.membership_id,
    next_poll_after_ms: 5_000,
    quote: {
      base_price: "100000",
      change_from_previous_day: "1200",
      change_percent_from_previous_day: "1.2",
      direction: "UP",
      last_success_at: received,
      price: "101200.00",
      quote_version: "1",
      received_at: received,
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

function deferred<T>(): {
  readonly promise: Promise<T>;
  readonly reject: (reason?: unknown) => void;
  readonly resolve: (value: T) => void;
} {
  let resolvePromise!: (value: T) => void;
  let rejectPromise!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });
  return { promise, reject: rejectPromise, resolve: resolvePromise };
}

function fakeClient(
  clock: FakeClock,
  overrides: Partial<{
    createDemand: IntradayQuoteClient["createDemand"];
    getQuote: IntradayQuoteClient["getQuote"];
    releaseDemand: IntradayQuoteClient["releaseDemand"];
  }> = {},
): IntradayQuoteClient & {
  readonly createDemand: ReturnType<typeof vi.fn>;
  readonly getQuote: ReturnType<typeof vi.fn>;
  readonly releaseDemand: ReturnType<typeof vi.fn>;
} {
  const createDemand = vi.fn(async (body: IntradayQuoteDemandRequest) =>
    demandFor(IDENTITY_A, body.consumer_id, body.renewal_sequence),
  );
  const getQuote = vi.fn(async (identity: IntradayQuoteIdentity) => quoteFor(identity, clock));
  const releaseDemand = vi.fn(
    async (_demandId: string, _body: IntradayQuoteReleaseRequest) => undefined,
  );
  return {
    createDemand: (overrides.createDemand ?? createDemand) as typeof createDemand,
    getQuote: (overrides.getQuote ?? getQuote) as typeof getQuote,
    releaseDemand: (overrides.releaseDemand ?? releaseDemand) as typeof releaseDemand,
  } as IntradayQuoteClient & {
    readonly createDemand: ReturnType<typeof vi.fn>;
    readonly getQuote: ReturnType<typeof vi.fn>;
    readonly releaseDemand: ReturnType<typeof vi.fn>;
  };
}

function startContext(coordinator: IntradayQuoteLoadCoordinator, identity = IDENTITY_A): void {
  coordinator.setContext({
    enabled: true,
    identity,
    mounted: true,
    online: true,
    snapshotKey: `${identity.instrument_id}:snapshot-1`,
    visible: true,
  });
}

describe("Stock Beta intraday quote lifecycle", () => {
  it("creates sequence zero, polls after demand, renews independently, and never overlaps GETs", async () => {
    const clock = new FakeClock();
    const get = deferred<IntradayQuoteResponse>();
    const client = fakeClient(clock, {
      getQuote: vi.fn(async () => get.promise),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
      createIdempotencyKey: (operation, sequence) => `${operation}/${sequence}`,
    });

    startContext(coordinator);
    await flush();
    expect(client.createDemand).toHaveBeenCalledWith(
      expect.objectContaining({ renewal_sequence: 0 }),
      expect.objectContaining({ idempotencyKey: expect.stringContaining("/0") }),
    );
    expect(client.getQuote).toHaveBeenCalledOnce();
    clock.advance(5_000);
    await flush();
    expect(client.getQuote).toHaveBeenCalledOnce();

    get.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState().phase).toBe("ready");
    expect(coordinator.getState().lastSuccessAt).toBe(
      new Date(clock.now()).toISOString().replace(".000", ""),
    );

    clock.advance(5_000);
    await flush();
    expect(client.getQuote).toHaveBeenCalledTimes(2);
    clock.advance(10_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    expect(client.createDemand.mock.calls[1]?.[0]).toMatchObject({ renewal_sequence: 1 });
  });

  it("recovers an already-expired accepted create by advancing the same consumer sequence", async () => {
    const clock = new FakeClock();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) => {
        const expiresAtMs = body.renewal_sequence === 0 ? clock.now() : clock.now() + 30_000;
        return demandFor(
          IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
          new Date(expiresAtMs).toISOString().replace(".000", ""),
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
    });

    startContext(coordinator);
    await flush();

    expect(client.getQuote).not.toHaveBeenCalled();
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });
    clock.advance(15_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    expect(client.createDemand.mock.calls[1]?.[0]).toMatchObject({
      consumer_id: client.createDemand.mock.calls[0]?.[0].consumer_id,
      renewal_sequence: 1,
    });
    expect(client.getQuote).toHaveBeenCalledOnce();
    expect(coordinator.getState()).toMatchObject({ phase: "ready" });
  });

  it("fences an expired GET and recovers the same visible consumer with a future renewal", async () => {
    const clock = new FakeClock();
    const expiredGet = deferred<IntradayQuoteResponse>();
    const recoveredGet = deferred<IntradayQuoteResponse>();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) =>
        demandFor(
          IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
          new Date(clock.now() + 10_000).toISOString().replace(".000", ""),
        ),
      ),
      getQuote: vi
        .fn()
        .mockReturnValueOnce(expiredGet.promise)
        .mockReturnValueOnce(recoveredGet.promise),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });

    startContext(coordinator);
    await flush();
    expect(client.getQuote).toHaveBeenCalledOnce();

    clock.advance(10_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });

    clock.advance(15_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    expect(client.createDemand.mock.calls[1]?.[0]).toMatchObject({
      consumer_id: client.createDemand.mock.calls[0]?.[0].consumer_id,
      renewal_sequence: 1,
    });
    expect(client.getQuote).toHaveBeenCalledTimes(2);

    expiredGet.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState()).not.toMatchObject({ phase: "ready" });

    recoveredGet.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "ready", quote: expect.any(Object) });
  });

  it("fences polling while a renewal is hung, then accepts its late future lease", async () => {
    const clock = new FakeClock();
    const renewal = deferred<IntradayQuoteDemandResponse>();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) => {
        if (body.renewal_sequence === 1) return renewal.promise;
        return demandFor(
          IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
          new Date(clock.now() + 20_000).toISOString().replace(".000", ""),
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
    });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);

    clock.advance(5_000);
    await flush();
    const getCallsAtExpiry = client.getQuote.mock.calls.length;
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });

    renewal.resolve(
      demandFor(
        IDENTITY_A,
        "00000000-0000-4000-8000-000000000010",
        1,
        new Date(clock.now() + 20_000).toISOString().replace(".000", ""),
      ),
    );
    await flush();
    expect(client.getQuote.mock.calls.length).toBeGreaterThan(getCallsAtExpiry);
    expect(coordinator.getState()).toMatchObject({ phase: "ready" });
    expect(client.releaseDemand).not.toHaveBeenCalled();
  });

  it("accepts a future renewal settling after the lease boundary before its timer callback", async () => {
    const clock = new FakeClock();
    const renewal = deferred<IntradayQuoteDemandResponse>();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) => {
        if (body.renewal_sequence === 1) return renewal.promise;
        return demandFor(
          IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
          new Date(clock.now() + 20_000).toISOString().replace(".000", ""),
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
    });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    const getCallsBeforeLateRenewal = client.getQuote.mock.calls.length;
    clock.jump(5_000);
    renewal.resolve(
      demandFor(
        IDENTITY_A,
        "00000000-0000-4000-8000-000000000010",
        1,
        new Date(clock.now() + 20_000).toISOString().replace(".000", ""),
      ),
    );
    await flush();

    expect(client.getQuote.mock.calls.length).toBeGreaterThan(getCallsBeforeLateRenewal);
    expect(coordinator.getState()).toMatchObject({ phase: "ready" });
    expect(client.releaseDemand).not.toHaveBeenCalled();
  });

  it("preserves a pre-expiry Retry-After across expiry before replaying the same renewal", async () => {
    const clock = new FakeClock();
    let renewalCalls = 0;
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) => {
        if (body.renewal_sequence === 1) {
          renewalCalls += 1;
          if (renewalCalls === 1) {
            throw new IntradayQuoteApiError(429, "QUOTE_DEMAND_CAPACITY", "request", 60_000);
          }
        }
        return demandFor(
          IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
          new Date(clock.now() + 30_000).toISOString().replace(".000", ""),
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
      createIdempotencyKey: (operation, sequence) => `${operation}/${sequence}`,
    });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);

    clock.advance(15_000);
    await flush();
    const getCallsAtExpiry = client.getQuote.mock.calls.length;
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });

    clock.advance(44_999);
    await flush();
    expect(clock.now()).toBe(NOW_MS + 74_999);
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    expect(client.getQuote).toHaveBeenCalledTimes(getCallsAtExpiry);

    clock.advance(1);
    await flush();
    expect(clock.now()).toBe(NOW_MS + 75_000);
    expect(client.createDemand).toHaveBeenCalledTimes(3);
    expect(client.createDemand.mock.calls[2]?.[0]).toEqual(client.createDemand.mock.calls[1]?.[0]);
    expect(client.createDemand.mock.calls[2]?.[1]).toMatchObject({
      idempotencyKey: client.createDemand.mock.calls[1]?.[1].idempotencyKey,
    });
    expect(client.getQuote.mock.calls.length).toBeGreaterThan(getCallsAtExpiry);
    expect(new Set(client.createDemand.mock.calls.map(([body]) => body.consumer_id)).size).toBe(1);
  });

  it("fences an old GET before accepting a future renewal after an unfired lease deadline", async () => {
    const clock = new FakeClock();
    const oldGet = deferred<IntradayQuoteResponse>();
    const freshGet = deferred<IntradayQuoteResponse>();
    const renewal = deferred<IntradayQuoteDemandResponse>();
    let oldSignal: AbortSignal | undefined;
    const client = fakeClient(clock, {
      createDemand: vi.fn((body) => {
        if (body.renewal_sequence === 1) return renewal.promise;
        return Promise.resolve(
          demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 20_000).toISOString().replace(".000", ""),
          ),
        );
      }),
      getQuote: vi.fn((_identity, options) => {
        if (oldSignal === undefined) {
          oldSignal = options.signal;
          return oldGet.promise;
        }
        return freshGet.promise;
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });

    startContext(coordinator);
    await flush();
    expect(client.getQuote).toHaveBeenCalledOnce();
    clock.advance(15_000);
    await flush();
    clock.jump(5_000);
    renewal.resolve(
      demandFor(
        IDENTITY_A,
        client.createDemand.mock.calls[0]?.[0].consumer_id,
        1,
        new Date(clock.now() + 30_000).toISOString().replace(".000", ""),
      ),
    );
    await flush();
    expect(oldSignal?.aborted).toBe(true);
    expect(client.getQuote).toHaveBeenCalledTimes(2);

    oldGet.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState()).toMatchObject({ fetching: true, phase: "polling" });

    freshGet.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "ready", quote: expect.any(Object) });
  });

  it("does not let an old GET rejection clobber the fresh poll after an unfired lease deadline", async () => {
    const clock = new FakeClock();
    const oldGet = deferred<IntradayQuoteResponse>();
    const freshGet = deferred<IntradayQuoteResponse>();
    const renewal = deferred<IntradayQuoteDemandResponse>();
    let requestCount = 0;
    const client = fakeClient(clock, {
      createDemand: vi.fn((body) => {
        if (body.renewal_sequence === 1) return renewal.promise;
        return Promise.resolve(
          demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 20_000).toISOString().replace(".000", ""),
          ),
        );
      }),
      getQuote: vi.fn(() => {
        requestCount += 1;
        return requestCount === 1 ? oldGet.promise : freshGet.promise;
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    clock.jump(5_000);
    renewal.resolve(
      demandFor(
        IDENTITY_A,
        client.createDemand.mock.calls[0]?.[0].consumer_id,
        1,
        new Date(clock.now() + 30_000).toISOString().replace(".000", ""),
      ),
    );
    await flush();
    expect(client.getQuote).toHaveBeenCalledTimes(2);

    oldGet.reject(new Error("old GET failed after its lease ended"));
    await flush();
    expect(coordinator.getState()).toMatchObject({ fetching: true, phase: "polling" });

    freshGet.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "ready", quote: expect.any(Object) });
  });

  it("replays a transient renewal with the same body and key, then advances once", async () => {
    const clock = new FakeClock();
    const client = fakeClient(clock, {
      createDemand: vi
        .fn()
        .mockImplementationOnce(async (body) =>
          demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 60_000).toISOString().replace(".000", ""),
          ),
        )
        .mockRejectedValueOnce(
          new IntradayQuoteApiError(503, "QUOTE_CACHE_UNAVAILABLE", "request", 15_000),
        )
        .mockImplementationOnce(async (body) =>
          demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 60_000).toISOString().replace(".000", ""),
          ),
        ),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
      createIdempotencyKey: (operation, sequence) => `${operation}/${sequence}`,
    });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);

    clock.advance(15_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(3);
    expect(client.createDemand.mock.calls[1]?.[0]).toEqual(client.createDemand.mock.calls[2]?.[0]);
    expect(client.createDemand.mock.calls[1]?.[1]).toMatchObject({
      idempotencyKey: "quote-demand-00000000-0000-4000-8000-000000000010/1",
    });
    expect(client.createDemand.mock.calls[2]?.[1]).toMatchObject({
      idempotencyKey: "quote-demand-00000000-0000-4000-8000-000000000010/1",
    });

    clock.advance(15_000);
    await flush();
    expect(client.createDemand.mock.calls[3]?.[0]).toMatchObject({ renewal_sequence: 2 });
  });

  it("keeps the quote recent at exactly 30 seconds and cache-valid at exactly 24 hours", async () => {
    const clock = new FakeClock();
    const pending = deferred<IntradayQuoteResponse>();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) =>
        demandFor(
          IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
          new Date(clock.now() + INTRADAY_QUOTE_CACHE_MAX_AGE_MS * 2)
            .toISOString()
            .replace(".000", ""),
        ),
      ),
      getQuote: vi
        .fn()
        .mockImplementationOnce(async () => quoteWithSuccessAt(IDENTITY_A, clock, NOW_MS))
        .mockReturnValue(pending.promise),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });

    startContext(coordinator);
    await flush();
    clock.advance(5_000);
    await flush();
    clock.advance(INTRADAY_QUOTE_CACHE_MAX_AGE_MS - 5_000);
    await flush();
    expect(coordinator.getState().quote).not.toBeNull();
    expect(coordinator.getState().phase).toBe("stale");

    clock.advance(1);
    expect(coordinator.getState().quote).toBeNull();
  });

  it("keeps the semantic phase while a retained quote GET is in flight", async () => {
    const clock = new FakeClock();
    const pending = deferred<IntradayQuoteResponse>();
    const client = fakeClient(clock, {
      getQuote: vi
        .fn()
        .mockResolvedValueOnce(quoteFor(IDENTITY_A, clock))
        .mockReturnValueOnce(pending.promise),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });

    startContext(coordinator);
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "ready", fetching: false });

    clock.advance(5_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "ready", fetching: true });
    expect(coordinator.getState().quote?.quote?.price).toBe("101200.00");

    pending.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "ready", fetching: false });
  });

  it("keeps a young retained quote visibly failed through a background poll until verified success", async () => {
    const clock = new FakeClock();
    const verifiedRecovery = deferred<IntradayQuoteResponse>();
    const client = fakeClient(clock, {
      getQuote: vi
        .fn()
        .mockResolvedValueOnce(quoteFor(IDENTITY_A, clock))
        .mockRejectedValueOnce(new Error("cache unavailable"))
        .mockReturnValueOnce(verifiedRecovery.promise),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });

    startContext(coordinator);
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "ready", errorCode: null });

    clock.advance(5_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({
      errorCode: "INTRADAY_QUOTE_REQUEST_FAILED",
      fetching: false,
      lastSuccessAt: new Date(NOW_MS).toISOString().replace(".000", ""),
      phase: "stale",
      reasonCode: "PRODUCER_UNAVAILABLE",
    });

    clock.advance(5_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({
      errorCode: "INTRADAY_QUOTE_REQUEST_FAILED",
      fetching: true,
      phase: "stale",
      reasonCode: "PRODUCER_UNAVAILABLE",
    });

    verifiedRecovery.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState()).toMatchObject({
      errorCode: null,
      fetching: false,
      phase: "ready",
      reasonCode: null,
    });
  });

  it("replays an ambiguous demand with the same sequence and key, then releases only its demand id", async () => {
    const clock = new FakeClock();
    const first = deferred<IntradayQuoteDemandResponse>();
    const client = fakeClient(clock, {
      createDemand: vi
        .fn()
        .mockReturnValueOnce(first.promise)
        .mockResolvedValue(demandFor(IDENTITY_A, "00000000-0000-4000-8000-000000000010", 0)),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
      createIdempotencyKey: (operation, sequence) => `${operation}/${sequence}`,
    });
    startContext(coordinator);
    await flush();
    coordinator.stop();
    first.resolve(demandFor(IDENTITY_A, "00000000-0000-4000-8000-000000000010", 0));
    await flush();
    await flush();
    expect(client.getQuote).not.toHaveBeenCalled();
    expect(client.releaseDemand).toHaveBeenCalledWith(
      "00000000-0000-4000-8000-000000000010",
      expect.objectContaining({ renewal_sequence: 1 }),
      expect.objectContaining({ idempotencyKey: expect.stringContaining("/1") }),
    );
  });

  it("ignores a delayed A response after selecting B and restarts with the exact B identity", async () => {
    const clock = new FakeClock();
    const requestA = deferred<IntradayQuoteResponse>();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) =>
        demandFor(
          body.membership_id === IDENTITY_B.membership_id ? IDENTITY_B : IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
        ),
      ),
      getQuote: vi
        .fn()
        .mockReturnValueOnce(requestA.promise)
        .mockImplementation(async (identity) => quoteFor(identity, clock)),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: (() => {
        let index = 10;
        return () => `00000000-0000-4000-8000-${String(index++).padStart(12, "0")}`;
      })(),
    });
    startContext(coordinator, IDENTITY_A);
    await flush();
    coordinator.setContext({
      enabled: true,
      identity: IDENTITY_B,
      mounted: true,
      online: true,
      snapshotKey: "114800.KRX:snapshot-2",
      visible: true,
    });
    await flush();
    requestA.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    await flush();
    expect(coordinator.getState().identity).toEqual(IDENTITY_B);
    expect(coordinator.getState().quote?.instrument_id).not.toBe(IDENTITY_A.instrument_id);
    expect(client.getQuote.mock.calls.at(-1)?.[0]).toEqual(IDENTITY_B);
  });

  it("ignores stale-session auth recovery and stops the active session before redirecting", async () => {
    const clock = new FakeClock();
    const requestA = deferred<IntradayQuoteResponse>();
    let navigateA: ((href: string) => void) | undefined;
    const navigate = vi.fn();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) =>
        demandFor(
          body.membership_id === IDENTITY_B.membership_id ? IDENTITY_B : IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
        ),
      ),
      getQuote: vi.fn(async (identity, options) => {
        if (identity.instrument_id === IDENTITY_A.instrument_id) {
          navigateA = options?.navigate;
          return requestA.promise;
        }
        return quoteFor(identity, clock);
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock, navigate });

    startContext(coordinator, IDENTITY_A);
    await flush();
    coordinator.setContext({
      enabled: true,
      identity: IDENTITY_B,
      mounted: true,
      online: true,
      snapshotKey: "114800.KRX:snapshot-2",
      visible: true,
    });
    await flush();
    navigateA?.("/login");
    expect(navigate).not.toHaveBeenCalled();
    expect(coordinator.getState()).toMatchObject({
      identity: IDENTITY_B,
      phase: "ready",
    });
    requestA.resolve(quoteFor(IDENTITY_A, clock));
    await flush();
    expect(coordinator.getState().identity).toEqual(IDENTITY_B);
    expect(coordinator.getState().quote?.instrument_id).toBe(IDENTITY_B.instrument_id);
  });

  it("stops the current session before login recovery", async () => {
    const clock = new FakeClock();
    const navigatePhases: IntradayQuoteLoadPhase[] = [];
    const navigate = vi.fn();
    let coordinator: IntradayQuoteLoadCoordinator;
    const client = fakeClient(clock, {
      getQuote: vi.fn(async (_identity, options) => {
        options?.navigate?.("/login");
        navigatePhases.push(coordinator.getState().phase);
        return quoteFor(IDENTITY_A, clock);
      }),
    });
    coordinator = new IntradayQuoteLoadCoordinator({ client, clock, navigate });
    startContext(coordinator);
    await flush();
    expect(navigate).toHaveBeenCalledWith("/login");
    expect(navigatePhases).toEqual(["idle"]);
    expect(coordinator.getState()).toMatchObject({ phase: "idle", quote: null, identity: null });
  });

  it("clears on terminal auth/resource errors, marks stale after 30 seconds, and rejects reversed versions", async () => {
    const clock = new FakeClock();
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) =>
        demandFor(
          IDENTITY_A,
          body.consumer_id,
          body.renewal_sequence,
          new Date(clock.now() + INTRADAY_QUOTE_CACHE_MAX_AGE_MS * 2)
            .toISOString()
            .replace(".000", ""),
        ),
      ),
      getQuote: vi
        .fn()
        .mockResolvedValueOnce(quoteFor(IDENTITY_A, clock))
        .mockRejectedValue(new Error("cache unavailable")),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });
    startContext(coordinator);
    await flush();
    expect(coordinator.getState().phase).toBe("ready");
    clock.advance(30_000);
    expect(coordinator.getState().phase).toBe("ready");
    clock.advance(1);
    expect(coordinator.getState().phase).toBe("stale");
    expect(coordinator.getState().lastSuccessAt).not.toBeNull();

    client.getQuote.mockImplementationOnce(async () =>
      (() => {
        const current = quoteFor(IDENTITY_A, clock).quote;
        if (current === null) throw new Error("fixture quote missing");
        return quoteFor(IDENTITY_A, clock, {
          quote: { ...current, quote_version: "0" },
        });
      })(),
    );
    clock.advance(5_000);
    await flush();
    expect(coordinator.getState().quote).toBeNull();

    client.getQuote.mockRejectedValueOnce(
      new IntradayQuoteApiError(401, "SESSION_EXPIRED", "r", null),
    );
    clock.advance(5_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "error", quote: null });
  });

  it("retains a validated same-session value for an explicit stale response", async () => {
    const clock = new FakeClock();
    const client = fakeClient(clock, {
      getQuote: vi
        .fn()
        .mockResolvedValueOnce(quoteFor(IDENTITY_A, clock))
        .mockResolvedValueOnce(
          quoteFor(IDENTITY_A, clock, {
            freshness: "UNAVAILABLE",
            quote: null,
            reason_code: "QUOTE_STALE",
          }),
        ),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });
    startContext(coordinator);
    await flush();
    clock.advance(5_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({
      phase: "stale",
      reasonCode: "QUOTE_STALE",
    });
    expect(coordinator.getState().quote?.quote?.price).toBe("101200.00");
    expect(coordinator.getState().lastSuccessAt).toBe(
      new Date(NOW_MS).toISOString().replace(".000", ""),
    );
  });

  it("stops without any demand or quote I/O while disabled, hidden, offline, or unmounted", async () => {
    const clock = new FakeClock();
    const client = fakeClient(clock);
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });
    coordinator.setContext({
      enabled: false,
      identity: IDENTITY_A,
      mounted: true,
      online: true,
      visible: true,
    });
    await flush();
    expect(client.createDemand).not.toHaveBeenCalled();
    startContext(coordinator);
    await flush();
    const demandCalls = client.createDemand.mock.calls.length;
    coordinator.setContext({ visible: false });
    coordinator.setContext({ visible: true });
    coordinator.setContext({ online: false });
    coordinator.setContext({ mounted: false });
    await flush();
    expect(coordinator.getState().quote).toBeNull();
    expect(client.createDemand.mock.calls.length).toBeGreaterThanOrEqual(demandCalls);
    expect(coordinator.getState().phase).toBe("offline");
  });

  it.each([
    [401, "SESSION_EXPIRED"],
    [403, "FORBIDDEN"],
    [404, "RESOURCE_NOT_FOUND"],
  ] as const)("clears and stops on typed HTTP %s/%s", async (status, code) => {
    const clock = new FakeClock();
    const client = fakeClient(clock, {
      getQuote: vi
        .fn()
        .mockResolvedValueOnce(quoteFor(IDENTITY_A, clock))
        .mockRejectedValueOnce(new IntradayQuoteApiError(status, code, "request", null)),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });
    startContext(coordinator);
    await flush();
    clock.advance(5_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "error", quote: null });
  });

  it("backs off capacity without a tight loop and keeps the failed sequence/key for replay", async () => {
    const clock = new FakeClock();
    const client = fakeClient(clock, {
      createDemand: vi
        .fn()
        .mockRejectedValueOnce(
          new IntradayQuoteApiError(429, "QUOTE_DEMAND_CAPACITY", "request", 15_000),
        )
        .mockResolvedValue(demandFor(IDENTITY_A, "00000000-0000-4000-8000-000000000010", 0)),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
      createIdempotencyKey: (operation, sequence) => `${operation}/${sequence}`,
    });
    startContext(coordinator);
    await flush();
    expect(client.createDemand).toHaveBeenCalledOnce();
    clock.advance(14_999);
    await flush();
    expect(client.createDemand).toHaveBeenCalledOnce();
    clock.advance(1);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    expect(client.createDemand.mock.calls[0]?.[0]).toEqual(client.createDemand.mock.calls[1]?.[0]);
    expect(client.createDemand.mock.calls[0]?.[1]).toEqual(
      expect.objectContaining({
        idempotencyKey: "quote-demand-00000000-0000-4000-8000-000000000010/0",
      }),
    );
    expect(client.createDemand.mock.calls[1]?.[1]).toEqual(
      expect.objectContaining({
        idempotencyKey: "quote-demand-00000000-0000-4000-8000-000000000010/0",
      }),
    );
  });

  it("replays an ambiguous expired renewal exactly once before advancing to its next sequence", async () => {
    const clock = new FakeClock();
    const ambiguousRenewal = deferred<IntradayQuoteDemandResponse>();
    const originalExpiry = new Date(NOW_MS + 20_000).toISOString().replace(".000", "");
    let sequenceOneCalls = 0;
    const client = fakeClient(clock, {
      createDemand: vi.fn((body) => {
        if (body.renewal_sequence === 0) {
          return Promise.resolve(
            demandFor(IDENTITY_A, body.consumer_id, body.renewal_sequence, originalExpiry),
          );
        }
        if (body.renewal_sequence === 1) {
          sequenceOneCalls += 1;
          if (sequenceOneCalls === 1) return ambiguousRenewal.promise;
          return Promise.resolve(
            demandFor(IDENTITY_A, body.consumer_id, body.renewal_sequence, originalExpiry),
          );
        }
        return Promise.resolve(
          demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 30_000).toISOString().replace(".000", ""),
          ),
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
      createIdempotencyKey: (operation, sequence) => `${operation}/${sequence}`,
    });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    expect(client.createDemand.mock.calls[1]?.[0]).toMatchObject({ renewal_sequence: 1 });

    clock.advance(5_000);
    await flush();
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });
    ambiguousRenewal.reject(
      new IntradayQuoteApiError(503, "QUOTE_CACHE_UNAVAILABLE", "request", 15_000),
    );
    await flush();

    clock.advance(14_999);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    clock.advance(1);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(3);
    expect(client.createDemand.mock.calls[2]?.[0]).toEqual(client.createDemand.mock.calls[1]?.[0]);
    expect(client.createDemand.mock.calls[2]?.[1]).toMatchObject({
      idempotencyKey: client.createDemand.mock.calls[1]?.[1].idempotencyKey,
    });

    clock.advance(15_000);
    await flush();
    expect(client.createDemand.mock.calls[3]?.[0]).toMatchObject({ renewal_sequence: 2 });
    expect(client.createDemand.mock.calls[3]?.[1].idempotencyKey).not.toBe(
      client.createDemand.mock.calls[2]?.[1].idempotencyKey,
    );
    expect(coordinator.getState()).toMatchObject({ phase: "ready" });
  });

  it("honors recovery Retry-After, caps three expired attempts, and never churns consumers", async () => {
    const clock = new FakeClock();
    const recoveryAttemptsAt: number[] = [];
    const client = fakeClient(clock, {
      createDemand: vi.fn(async (body) => {
        if (body.renewal_sequence === 0) {
          return demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 10_000).toISOString().replace(".000", ""),
          );
        }
        recoveryAttemptsAt.push(clock.now());
        throw new IntradayQuoteApiError(
          429,
          "QUOTE_DEMAND_CAPACITY",
          "request",
          recoveryAttemptsAt.length === 1 ? 30_000 : 15_000,
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
      createIdempotencyKey: (operation, sequence) => `${operation}/${sequence}`,
    });

    startContext(coordinator);
    await flush();
    clock.advance(10_000);
    await flush();
    const getCallsAtExpiry = client.getQuote.mock.calls.length;
    clock.advance(15_000);
    await flush();
    clock.advance(29_999);
    await flush();
    expect(recoveryAttemptsAt).toEqual([NOW_MS + 25_000]);
    clock.advance(1);
    await flush();
    clock.advance(15_000);
    await flush();

    expect(recoveryAttemptsAt).toEqual([NOW_MS + 25_000, NOW_MS + 55_000, NOW_MS + 70_000]);
    expect(client.createDemand).toHaveBeenCalledTimes(4);
    expect(new Set(client.createDemand.mock.calls.map(([body]) => body.consumer_id)).size).toBe(1);
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });
    clock.advance(60_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(4);
    expect(client.getQuote).toHaveBeenCalledTimes(getCallsAtExpiry);
  });

  it("does not overlap or hot-loop recovery while an expired mutation has no classified outcome", async () => {
    const clock = new FakeClock();
    const hungRenewal = deferred<IntradayQuoteDemandResponse>();
    const client = fakeClient(clock, {
      createDemand: vi.fn((body) => {
        if (body.renewal_sequence === 1) return hungRenewal.promise;
        return Promise.resolve(
          demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 20_000).toISOString().replace(".000", ""),
          ),
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({ client, clock });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    clock.advance(5_000);
    await flush();
    const getCallsAtExpiry = client.getQuote.mock.calls.length;
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });

    clock.advance(120_000);
    await flush();
    expect(client.createDemand).toHaveBeenCalledTimes(2);
    expect(client.getQuote).toHaveBeenCalledTimes(getCallsAtExpiry);
    expect(coordinator.getState()).toMatchObject({ phase: "unavailable", quote: null });
  });

  it("lets lifecycle cleanup win when a renewal succeeds after visibility is lost", async () => {
    const clock = new FakeClock();
    const lateRenewal = deferred<IntradayQuoteDemandResponse>();
    const client = fakeClient(clock, {
      createDemand: vi.fn((body) => {
        if (body.renewal_sequence === 1) return lateRenewal.promise;
        return Promise.resolve(
          demandFor(
            IDENTITY_A,
            body.consumer_id,
            body.renewal_sequence,
            new Date(clock.now() + 30_000).toISOString().replace(".000", ""),
          ),
        );
      }),
    });
    const coordinator = new IntradayQuoteLoadCoordinator({
      client,
      clock,
      createConsumerId: () => "00000000-0000-4000-8000-000000000010",
    });

    startContext(coordinator);
    await flush();
    clock.advance(15_000);
    await flush();
    const getCallsBeforeCleanup = client.getQuote.mock.calls.length;
    coordinator.setContext({ visible: false });
    lateRenewal.resolve(
      demandFor(
        IDENTITY_A,
        "00000000-0000-4000-8000-000000000010",
        1,
        new Date(clock.now() + 30_000).toISOString().replace(".000", ""),
      ),
    );
    await flush();
    await flush();

    expect(coordinator.getState()).toMatchObject({ phase: "idle", quote: null });
    expect(client.getQuote).toHaveBeenCalledTimes(getCallsBeforeCleanup);
    expect(client.releaseDemand).toHaveBeenCalledWith(
      "00000000-0000-4000-8000-000000000011",
      expect.objectContaining({
        consumer_id: "00000000-0000-4000-8000-000000000010",
        renewal_sequence: 2,
      }),
      expect.any(Object),
    );
  });
});
