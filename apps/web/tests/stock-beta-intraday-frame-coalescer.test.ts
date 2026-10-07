import { describe, expect, it, vi } from "vitest";
import {
  classifyIntradayStreamUpdate,
  createIntradayFrameCoalescer,
} from "@/components/stock-beta/quote/intraday-frame-coalescer";
import type { IntradayStreamControllerView } from "@/components/stock-beta/quote/intraday-stream-controller";
import { streamRow } from "./fixtures/market-stream";

function frameScheduler() {
  let nextHandle = 0;
  const pending = new Map<number, FrameRequestCallback>();
  const callbacks = new Map<number, FrameRequestCallback>();
  const request = vi.fn((callback: FrameRequestCallback) => {
    const handle = ++nextHandle;
    pending.set(handle, callback);
    callbacks.set(handle, callback);
    return handle;
  });
  const cancel = vi.fn((handle: number) => {
    pending.delete(handle);
  });
  return {
    scheduler: { request, cancel },
    pending,
    run(handle: number, includeCancelled = false) {
      const callback = callbacks.get(handle);
      if (!callback || (!includeCancelled && !pending.has(handle))) return;
      pending.delete(handle);
      callback(0);
    },
  };
}

function readyView(rows: IntradayStreamControllerView["rows"]): IntradayStreamControllerView {
  return {
    lifecycle: "active",
    failure: null,
    phase: "ready",
    rows,
    terminalReason: null,
  };
}

function quoteUpdate(view: IntradayStreamControllerView, index: number, version: string) {
  return readyView(
    view.rows.map((row, rowIndex) => {
      if (rowIndex !== index || row.quote === null) return row;
      return {
        ...row,
        state_version: version,
        quote: {
          ...row.quote,
          price: String(100 + Number(version)),
          quote_version: version,
          receive_ordinal: version,
        },
      };
    }),
  );
}

function submitClassifiedUpdate(
  next: IntradayStreamControllerView,
  previous: { current: IntradayStreamControllerView },
  coalescer: ReturnType<typeof createIntradayFrameCoalescer<IntradayStreamControllerView>>,
) {
  const kind = classifyIntradayStreamUpdate(previous.current, next);
  previous.current = next;
  if (kind === "quote") coalescer.schedule(next);
  else if (kind === "control") coalescer.flush(next);
  return kind;
}

function staleFirstRow(view: IntradayStreamControllerView): IntradayStreamControllerView {
  const first = view.rows[0];
  const second = view.rows[1];
  if (!first || !second) throw new Error("Expected two stream rows");
  return readyView([{ ...first, freshness: "STALE", availability: "LAST_KNOWN" }, second]);
}

describe("intraday animation-frame delivery", () => {
  it("coalesces quote bursts with stable stale rows into one latest-view callback", () => {
    const frames = frameScheduler();
    const deliver = vi.fn();
    const coalescer = createIntradayFrameCoalescer<IntradayStreamControllerView>(
      deliver,
      frames.scheduler,
    );
    const live = streamRow(0);
    const stale = {
      ...streamRow(1),
      freshness: "STALE" as const,
      availability: "LAST_KNOWN" as const,
    };
    const previous = { current: readyView([live, stale]) };
    const first = quoteUpdate(previous.current, 0, "2");
    const latest = quoteUpdate(first, 0, "3");

    expect(submitClassifiedUpdate(first, previous, coalescer)).toBe("quote");
    expect(submitClassifiedUpdate(latest, previous, coalescer)).toBe("quote");

    expect(frames.scheduler.request).toHaveBeenCalledTimes(1);
    expect(frames.pending.size).toBe(1);
    frames.run(1);
    expect(deliver).toHaveBeenCalledExactlyOnceWith(latest);
  });

  it.each([
    [
      "terminal clear",
      (view: IntradayStreamControllerView) => ({
        ...view,
        lifecycle: "failed" as const,
        failure: "forbidden" as const,
        phase: "idle" as const,
        rows: [],
      }),
    ],
    ["row purge", (view: IntradayStreamControllerView) => readyView(view.rows.slice(0, 1))],
    ["stale transition", staleFirstRow],
  ])("flushes %s immediately and cancels a queued quote update", (_label, transition) => {
    const frames = frameScheduler();
    const deliver = vi.fn();
    const coalescer = createIntradayFrameCoalescer<IntradayStreamControllerView>(
      deliver,
      frames.scheduler,
    );
    const base = readyView([streamRow(0), streamRow(1)]);
    const previous = { current: base };
    const queued = quoteUpdate(base, 0, "2");
    expect(submitClassifiedUpdate(queued, previous, coalescer)).toBe("quote");
    const terminalOrControl = transition(queued);

    expect(submitClassifiedUpdate(terminalOrControl, previous, coalescer)).toBe("control");

    expect(deliver).toHaveBeenCalledExactlyOnceWith(terminalOrControl);
    expect(frames.scheduler.cancel).toHaveBeenCalledExactlyOnceWith(1);
    expect(frames.pending.size).toBe(0);
    frames.run(1, true);
    expect(deliver).toHaveBeenCalledExactlyOnceWith(terminalOrControl);
  });

  it("cancels pending work on disposal and ignores a callback that still arrives", () => {
    const frames = frameScheduler();
    const deliver = vi.fn();
    const coalescer = createIntradayFrameCoalescer<string>(deliver, frames.scheduler);

    coalescer.schedule("queued");
    coalescer.dispose();
    frames.run(1, true);
    coalescer.schedule("after-disposal");

    expect(frames.scheduler.cancel).toHaveBeenCalledExactlyOnceWith(1);
    expect(frames.scheduler.request).toHaveBeenCalledTimes(1);
    expect(deliver).not.toHaveBeenCalled();
  });
});
