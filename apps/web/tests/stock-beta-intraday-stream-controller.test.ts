import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  type IntradayStreamContext,
  IntradayStreamController,
} from "@/components/stock-beta/quote/intraday-stream-controller";
import {
  type IntradayStreamClient,
  IntradayStreamHttpError,
} from "@/lib/products/intraday-stream-client";
import type { IntradayStreamRow } from "@/lib/products/intraday-stream-contracts";
import type {
  IntradayStreamLease,
  IntradayStreamLeaseRequest,
} from "@/lib/products/intraday-stream-lease-contracts";

function must<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("Expected fixture value is missing");
  return value;
}

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const SERVER = "2026-10-03T03:00:00.500Z";
const identities = (count = 30) =>
  Array.from({ length: count }, (_, index) => ({
    membership_id: uuid(index + 1),
    instrument_id: `${String(index + 1).padStart(6, "0")}.KRX`,
    generation: 7,
  }));
const context = (overrides: Partial<IntradayStreamContext> = {}): IntradayStreamContext => ({
  enabled: true,
  owner: true,
  visible: true,
  online: true,
  sessionKey: "local-session-a",
  identities: identities(),
  ...overrides,
});
const accepted = (request: IntradayStreamLeaseRequest): IntradayStreamLease => ({
  ...request,
  lease_id: uuid(Number(request.consumer_id.slice(-12)) + 10_000),
  lease_expires_at: new Date(
    Date.parse(SERVER) + 30_000 + request.renewal_sequence * 15_000,
  ).toISOString(),
  renew_after_ms: 15_000,
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

class Source extends EventTarget {
  closed = false;
  readonly oldCallbacks: EventListener[] = [];
  constructor(readonly path: string) {
    super();
  }
  override addEventListener(
    type: string,
    callback: EventListenerOrEventListenerObject | null,
    options?: AddEventListenerOptions | boolean,
  ): void {
    if (typeof callback === "function") this.oldCallbacks.push(callback);
    super.addEventListener(type, callback, options);
  }
  close() {
    this.closed = true;
  }
  data(kind: string, body: unknown, sequence = 1, stream = uuid(900), time = SERVER) {
    this.dispatchEvent(
      new MessageEvent(kind, {
        data: JSON.stringify({
          schema_version: 2,
          stream_id: stream,
          event_sequence: String(sequence),
          server_time: time,
          body,
        }),
        lastEventId: `${stream}:${sequence}`,
      }),
    );
  }
}

function row(): IntradayStreamRow {
  return {
    ...must(identities(1)[0]),
    row_generation: uuid(90),
    venue: "KRX",
    currency: "KRW",
    source: "KIS_MARKET_WS",
    wire_version: "kis-h0stcnt0-20260914-v1",
    session: {
      date: "2026-10-03",
      timezone: "Asia/Seoul",
      calendar_source: "kis",
      calendar_source_version: "kis-chk-holiday-v1:schema-1",
      calendar_content_sha256: "a".repeat(64),
      window_contract_sha256: `sha256:${"b".repeat(64)}`,
    },
    subscription: "ACKED",
    connection: "CONNECTED",
    market_state: "OPEN",
    freshness: "RECENT",
    availability: "LIVE",
    reason_code: null,
    state_version: "1",
    gap_open: false,
    session_has_gap: false,
    gap_generation: "0",
    quote: {
      price: "100000",
      base_price: null,
      base_price_reason: "NOT_PROVIDED_BY_CHANNEL",
      change_from_previous_day: "0",
      change_percent_from_previous_day: "0",
      direction: "FLAT",
      trade_volume: "1",
      cumulative_volume: "2",
      halted: false,
      business_date: "2026-10-03",
      trade_time: "12:00:00",
      provider_trade_at: "2026-10-03T12:00:00+09:00",
      received_at: "2026-10-03T03:00:00.100Z",
      committed_at: "2026-10-03T03:00:00.200Z",
      epoch: uuid(99),
      quote_version: "1",
      receive_ordinal: "1",
    },
  };
}

function harness() {
  const sources: Source[] = [];
  let counter = 1_000;
  let maximumSources = 0;
  const replace = vi.fn<IntradayStreamClient["replaceLease"]>(async (request) => accepted(request));
  const release = vi.fn<IntradayStreamClient["releaseLease"]>(async () => undefined);
  const controller = new IntradayStreamController({
    client: { replaceLease: replace, releaseLease: release },
    uuid: () => uuid(++counter),
    clock: { now: () => Date.now(), setTimeout, clearTimeout },
    createSource: (path) => {
      const source = new Source(path);
      sources.push(source);
      maximumSources = Math.max(maximumSources, sources.filter((value) => !value.closed).length);
      return source;
    },
  });
  function snapshot(source = must(sources.at(-1)), stream = uuid(900)) {
    const request = must(replace.mock.calls.at(-1))[0];
    source.data(
      "snapshot",
      {
        lease_id: accepted(request).lease_id,
        lease_expires_at: accepted(request).lease_expires_at,
        rows: [row()],
      },
      1,
      stream,
    );
  }
  return { controller, replace, release, sources, snapshot, maximumSources: () => maximumSources };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
});
afterEach(() => {
  vi.useRealTimers();
});
const flush = () => vi.advanceTimersByTimeAsync(0);

describe("one tab market-stream lifecycle owner", () => {
  it("has no network effect before Owner/feature/visibility/online/session/demand gates all permit it", async () => {
    const h = harness();
    for (const denied of [
      { enabled: false },
      { owner: false },
      { visible: false },
      { online: false },
      { sessionKey: null },
      { identities: [] },
    ])
      h.controller.setContext(context(denied));
    await flush();
    expect(h.replace).not.toHaveBeenCalled();
    expect(h.sources).toHaveLength(0);
    h.controller.destroy();
  });

  it("owns one thirty-item lease and EventSource across repeated board/detail renders", async () => {
    const h = harness();
    h.controller.setContext(context());
    await flush();
    for (let i = 0; i < 10; i++)
      h.controller.setContext(context({ identities: [...identities()].reverse() }));
    await flush();
    expect(h.replace).toHaveBeenCalledTimes(1);
    expect(must(h.replace.mock.calls[0])[0].identities).toHaveLength(30);
    expect(h.sources).toHaveLength(1);
    expect(h.maximumSources()).toBe(1);
    expect(h.sources[0]?.path).toMatch(
      /^\/api\/v1\/research\/owner-beta\/equity-universe-v2\/market-stream\?lease_id=[0-9a-f-]+$/,
    );
    h.controller.destroy();
    await flush();
    expect(h.release).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("renews serially with the same consumer/lease and exactly the next accepted sequence at fifteen seconds", async () => {
    const h = harness();
    h.controller.setContext(context({ identities: identities(1) }));
    await flush();
    h.snapshot();
    // Authenticated status heartbeats maintain the transport; they never create demand.
    for (let second = 1; second <= 15; second++) {
      await vi.advanceTimersByTimeAsync(1_000);
      must(h.sources[0]).data(
        "status",
        {
          connection: "CONNECTED",
          reason_code: null,
          gap_open: false,
          session_has_gap: false,
          gap_generation: "0",
        },
        second + 1,
        uuid(900),
        new Date(Date.parse(SERVER) + second * 1_000).toISOString(),
      );
    }
    await flush();
    expect(h.replace).toHaveBeenCalledTimes(2);
    const first = must(h.replace.mock.calls[0])[0];
    const next = must(h.replace.mock.calls[1]);
    expect(next[0].consumer_id).toBe(first.consumer_id);
    expect(next[0].renewal_sequence).toBe(1);
    expect(next[1].previousLeaseId).toBe(accepted(first).lease_id);
    expect(next[1].idempotencyKey).not.toBe(must(h.replace.mock.calls[0])[1].idempotencyKey);
    expect(h.sources).toHaveLength(1);
    h.controller.destroy();
    await flush();
    expect(must(h.release.mock.calls[0])[1].renewal_sequence).toBe(1);
  });

  it("purges immediately on hide, releases once and resumes with a new consumer while stale callbacks cannot re-enter", async () => {
    const h = harness();
    const active = context({ identities: identities(1) });
    h.controller.setContext(active);
    await flush();
    h.snapshot();
    expect(h.controller.view().rows[0]?.availability).toBe("LIVE");
    const old = must(h.sources[0]);
    const oldConsumer = must(h.replace.mock.calls[0])[0].consumer_id;
    h.controller.setContext({ ...active, visible: false });
    h.controller.setContext({ ...active, visible: false });
    expect(old.closed).toBe(true);
    expect(h.controller.view().rows).toEqual([]);
    await flush();
    expect(h.release).toHaveBeenCalledTimes(1);
    h.controller.setContext(active);
    await flush();
    expect(must(h.replace.mock.calls[1])[0].consumer_id).not.toBe(oldConsumer);
    expect(must(h.replace.mock.calls[1])[0].renewal_sequence).toBe(0);
    for (const callback of old.oldCallbacks)
      callback(new MessageEvent("snapshot", { data: "old invalid data" }));
    expect(h.sources[1]?.closed).toBe(false);
    expect(h.controller.view().failure).toBeNull();
    h.controller.destroy();
    await flush();
    expect(h.maximumSources()).toBe(1);
  });

  it("stops on offline/logout/destroy, and a rerender of the logged-out session never reopens", async () => {
    const h = harness();
    h.controller.setContext(context());
    await flush();
    h.controller.setContext(context({ online: false }));
    await flush();
    expect(h.sources[0]?.closed).toBe(true);
    h.controller.setContext(context());
    await flush();
    expect(h.sources).toHaveLength(2);
    h.controller.logout();
    h.controller.setContext(context());
    await flush();
    expect(h.sources).toHaveLength(2);
    h.controller.setContext(context({ sessionKey: "local-session-b" }));
    await flush();
    expect(h.sources).toHaveLength(3);
    h.controller.destroy();
    h.controller.setContext(context({ sessionKey: "local-session-c" }));
    await flush();
    expect(h.sources).toHaveLength(3);
    expect(h.release).toHaveBeenCalledTimes(3);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("handles a late create after hide by releasing that exact old lease without opening a source", async () => {
    const h = harness();
    const pending = deferred<IntradayStreamLease>();
    h.replace.mockImplementationOnce(() => pending.promise);
    h.controller.setContext(context());
    const submitted = must(h.replace.mock.calls[0]);
    h.controller.setContext(context({ visible: false }));
    expect(submitted[1].signal.aborted).toBe(true);
    pending.resolve(accepted(submitted[0]));
    await flush();
    expect(h.sources).toHaveLength(0);
    expect(h.release).toHaveBeenCalledTimes(1);
    expect(must(h.release.mock.calls[0])[0]).toBe(accepted(submitted[0]).lease_id);
    h.controller.destroy();
    await flush();
  });

  it("times out an unknown create without retry and still observes its late accepted response for one cleanup", async () => {
    const h = harness();
    const pending = deferred<IntradayStreamLease>();
    h.replace.mockImplementationOnce(() => pending.promise);
    h.controller.setContext(context());
    const submitted = must(h.replace.mock.calls[0]);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(h.controller.view()).toMatchObject({
      lifecycle: "failed",
      failure: "timeout",
      rows: [],
    });
    h.controller.setContext(context());
    await vi.advanceTimersByTimeAsync(60_000);
    expect(h.replace).toHaveBeenCalledTimes(1);
    expect(submitted[1].signal.aborted).toBe(true);
    pending.resolve(accepted(submitted[0]));
    await flush();
    expect(h.sources).toHaveLength(0);
    expect(h.release).toHaveBeenCalledTimes(1);
    h.controller.destroy();
    await flush();
  });

  it("coalesces identity changes during a pending mutation, and removed identities disappear before replacement completes", async () => {
    const h = harness();
    h.controller.setContext(context({ identities: identities(1) }));
    await flush();
    h.snapshot();
    const next = deferred<IntradayStreamLease>();
    h.replace.mockImplementationOnce(() => next.promise);
    h.controller.setContext(context({ identities: identities(2) }));
    h.controller.setContext(context({ identities: identities(3) }));
    expect(h.controller.view().rows).toEqual([]);
    expect(h.sources[0]?.closed).toBe(true);
    expect(h.replace).toHaveBeenCalledTimes(2);
    next.resolve(accepted(must(h.replace.mock.calls[1])[0]));
    await flush();
    expect(h.replace).toHaveBeenCalledTimes(3);
    expect(must(h.replace.mock.calls[2])[0]).toMatchObject({
      renewal_sequence: 2,
      identities: identities(3),
    });
    expect(h.sources).toHaveLength(2);
    expect(h.maximumSources()).toBe(1);
    h.controller.destroy();
    await flush();
  });

  it("never accepts a foreign or reordered lease response and does not release an unowned ID", async () => {
    const h = harness();
    h.replace.mockImplementationOnce(async (request) => ({
      ...accepted(request),
      consumer_id: uuid(99),
    }));
    h.controller.setContext(context());
    await flush();
    expect(h.controller.view().failure).toBe("invalid_response");
    expect(h.sources).toHaveLength(0);
    expect(h.release).not.toHaveBeenCalled();
    h.controller.destroy();
  });

  it("marks a lost connection nonlive immediately, uses bounded backoff and requires reset on reconnect", async () => {
    const h = harness();
    h.controller.setContext(context({ identities: identities(1) }));
    await flush();
    h.snapshot();
    must(h.sources[0]).dispatchEvent(new Event("error"));
    expect(h.controller.view().rows[0]?.availability).toBe("LAST_KNOWN");
    await vi.advanceTimersByTimeAsync(1_000);
    expect(h.sources).toHaveLength(2);
    // A new stream without the required reset cannot restore LIVE.
    h.snapshot(must(h.sources[1]), uuid(901));
    expect(h.controller.view().rows).toEqual([]);
    expect(h.sources[1]?.closed).toBe(true);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(h.sources).toHaveLength(3);
    must(h.sources[2]).data("reset", { reason_code: "RESYNC_REQUIRED" }, 1, uuid(902));
    const request = must(h.replace.mock.calls[0])[0];
    must(h.sources[2]).data(
      "snapshot",
      {
        lease_id: accepted(request).lease_id,
        lease_expires_at: accepted(request).lease_expires_at,
        rows: [row()],
      },
      2,
      uuid(902),
    );
    expect(h.controller.view().rows[0]?.availability).toBe("LIVE");
    expect(h.replace).toHaveBeenCalledTimes(1);
    expect(h.maximumSources()).toBe(1);
    h.controller.destroy();
    await flush();
  });

  it("exhausts reconnect attempts without a renewal or a hidden quote GET loop", async () => {
    const h = harness();
    h.controller.setContext(context());
    await flush();
    for (const delay of [1_000, 2_000, 4_000]) {
      must(h.sources.at(-1)).dispatchEvent(new Event("error"));
      await vi.advanceTimersByTimeAsync(delay);
    }
    must(h.sources.at(-1)).dispatchEvent(new Event("error"));
    await flush();
    expect(h.controller.view()).toMatchObject({
      lifecycle: "failed",
      failure: "unavailable",
      rows: [],
    });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(h.sources).toHaveLength(4);
    expect(h.replace).toHaveBeenCalledTimes(1);
    expect(h.release).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
    h.controller.destroy();
  });

  it("closes a silent stream within five seconds and retains only nonlive last-known data", async () => {
    const h = harness();
    h.controller.setContext(context({ identities: identities(1) }));
    await flush();
    h.snapshot();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(h.sources[0]?.closed).toBe(true);
    expect(h.controller.view().rows[0]?.availability).toBe("LAST_KNOWN");
    h.controller.destroy();
    await flush();
  });

  it("purges and stops renewals immediately on ACCESS_REVOKED", async () => {
    const h = harness();
    h.controller.setContext(context({ identities: identities(1) }));
    await flush();
    h.snapshot();
    must(h.sources[0]).data(
      "status",
      {
        connection: "STOPPED",
        reason_code: "ACCESS_REVOKED",
        gap_open: false,
        session_has_gap: false,
        gap_generation: "0",
      },
      2,
    );
    expect(h.controller.view()).toMatchObject({ failure: "forbidden", rows: [] });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(h.replace).toHaveBeenCalledTimes(1);
    expect(h.sources[0]?.closed).toBe(true);
    expect(h.release).toHaveBeenCalledTimes(1);
    h.controller.destroy();
  });

  it("does not retry failed renewals and bounds a missing release response", async () => {
    const h = harness();
    h.controller.setContext(context());
    await flush();
    const releasePending = deferred<void>();
    h.release.mockImplementationOnce(() => releasePending.promise);
    h.replace.mockRejectedValueOnce(new IntradayStreamHttpError("conflict"));
    h.controller.setContext(context({ identities: identities(2) }));
    await flush();
    expect(h.controller.view().failure).toBe("conflict");
    expect(h.release).toHaveBeenCalledTimes(1);
    const releaseSignal = must(h.release.mock.calls[0])[2].signal;
    await vi.advanceTimersByTimeAsync(3_000);
    expect(releaseSignal.aborted).toBe(true);
    releasePending.resolve();
    await flush();
    expect(vi.getTimerCount()).toBe(0);
    h.controller.destroy();
  });

  it("stops renewal as soon as the authenticated stream reports FEATURE_DISABLED", async () => {
    const h = harness();
    h.controller.setContext(context({ identities: identities(1) }));
    await flush();
    h.snapshot();
    must(h.sources[0]).data(
      "status",
      {
        connection: "STOPPED",
        reason_code: "FEATURE_DISABLED",
        gap_open: false,
        session_has_gap: false,
        gap_generation: "0",
      },
      2,
    );
    expect(h.controller.view()).toMatchObject({ failure: "feature_disabled", rows: [] });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(h.replace).toHaveBeenCalledTimes(1);
    expect(h.release).toHaveBeenCalledTimes(1);
    h.controller.destroy();
  });

  it("treats a backwards renewal expiry as a failure, rather than swallowing the post-response validation error", async () => {
    const h = harness();
    h.controller.setContext(context({ identities: identities(1) }));
    await flush();
    h.snapshot();
    h.replace.mockImplementationOnce(async (request) => ({
      ...accepted(request),
      lease_expires_at: "2026-10-03T03:00:01Z",
    }));
    for (let second = 1; second < 15; second++) {
      await vi.advanceTimersByTimeAsync(1_000);
      must(h.sources[0]).data(
        "status",
        {
          connection: "CONNECTED",
          reason_code: null,
          gap_open: false,
          session_has_gap: false,
          gap_generation: "0",
        },
        second + 1,
        uuid(900),
        new Date(Date.parse(SERVER) + second * 1_000).toISOString(),
      );
    }
    await vi.advanceTimersByTimeAsync(1_000);
    expect(h.controller.view()).toMatchObject({ failure: "invalid_response", rows: [] });
    expect(h.sources[0]?.closed).toBe(true);
    expect(h.release).toHaveBeenCalledTimes(1);
    h.controller.destroy();
  });
});
