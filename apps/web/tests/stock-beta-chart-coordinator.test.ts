import { describe, expect, it } from "vitest";
import {
  StockBetaChartLoadCoordinator,
  type StockBetaChartLoadRequest,
} from "@/components/stock-beta/chart-load-coordinator";
import {
  OwnerEquityV2ChartIntegrityError,
  type OwnerEquityV2ChartModel,
  type OwnerEquityV2ChartRange,
} from "@/lib/products/equity-signals-contracts";

const SNAPSHOT_ID = "00000000-0000-4000-8000-000000000201";
const AS_OF = "2026-09-03";

function chartFor(
  instrumentId: string,
  range: OwnerEquityV2ChartRange,
  snapshotId = SNAPSHOT_ID,
): OwnerEquityV2ChartModel {
  return {
    as_of: AS_OF,
    bars: [
      {
        close: 100,
        high: 101,
        low: 99,
        open: 100,
        session_date: AS_OF,
        sma_20: null,
        sma_60: null,
        volume: 10,
      },
    ],
    expected_as_of: AS_OF,
    freshness: "CURRENT",
    generation: 1,
    instrument_id: instrumentId,
    latest: {
      change: 1,
      change_rate: 0.01,
      close: 100,
      session_date: AS_OF,
      volume: 10,
    },
    price_semantics: "ORIGINAL_UNADJUSTED",
    range,
    snapshot_id: snapshotId,
    warnings: ["NOT_REALTIME", "CORPORATE_ACTIONS_NOT_ADJUSTED", "RESEARCH_ONLY"],
  };
}

function request(
  instrumentId: string,
  range: OwnerEquityV2ChartRange,
  snapshotId = SNAPSHOT_ID,
): StockBetaChartLoadRequest {
  return {
    asOf: AS_OF,
    generation: 1,
    instrumentId,
    range,
    snapshotId,
  };
}

function deferred<T>(): {
  readonly promise: Promise<T>;
  readonly reject: (error: unknown) => void;
  readonly resolve: (value: T) => void;
} {
  let resolvePromise!: (value: T) => void;
  let rejectPromise!: (error: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });
  return { promise, reject: rejectPromise, resolve: resolvePromise };
}

describe("stock beta chart load coordinator", () => {
  it("discards an abort-ignorant A-to-B response by token", async () => {
    const coordinator = new StockBetaChartLoadCoordinator();
    const pendingA = deferred<OwnerEquityV2ChartModel>();
    const pendingB = deferred<OwnerEquityV2ChartModel>();
    const signals = new Map<string, AbortSignal>();
    const events: string[] = [];
    const fetcher = (nextRequest: StockBetaChartLoadRequest, signal: AbortSignal) => {
      signals.set(nextRequest.instrumentId, signal);
      return nextRequest.instrumentId === "000001.KRX" ? pendingA.promise : pendingB.promise;
    };
    const handlers = {
      onFailure: () => events.push("failure"),
      onSuccess: (chart: OwnerEquityV2ChartModel) => events.push(`success:${chart.instrument_id}`),
    };

    const first = coordinator.run(request("000001.KRX", "1y"), fetcher, handlers);
    const second = coordinator.run(request("000002.KRX", "1y"), fetcher, handlers);
    expect(signals.get("000001.KRX")?.aborted).toBe(true);

    pendingB.resolve(chartFor("000002.KRX", "1y"));
    await expect(second).resolves.toBe("success");
    pendingA.resolve(chartFor("000001.KRX", "1y"));
    await expect(first).resolves.toBe("stale");
    expect(events).toEqual(["success:000002.KRX"]);
  });

  it("discards an older range response even when instrument and snapshot stay fixed", async () => {
    const coordinator = new StockBetaChartLoadCoordinator();
    const pendingLong = deferred<OwnerEquityV2ChartModel>();
    const pendingShort = deferred<OwnerEquityV2ChartModel>();
    const events: string[] = [];
    const fetcher = (nextRequest: StockBetaChartLoadRequest, _signal: AbortSignal) =>
      nextRequest.range === "1y" ? pendingLong.promise : pendingShort.promise;
    const handlers = {
      onFailure: () => events.push("failure"),
      onSuccess: (chart: OwnerEquityV2ChartModel) => events.push(`success:${chart.range}`),
    };

    const longRequest = coordinator.run(request("000001.KRX", "1y"), fetcher, handlers);
    const shortRequest = coordinator.run(request("000001.KRX", "1m"), fetcher, handlers);
    pendingShort.resolve(chartFor("000001.KRX", "1m"));
    await expect(shortRequest).resolves.toBe("success");
    pendingLong.resolve(chartFor("000001.KRX", "1y"));
    await expect(longRequest).resolves.toBe("stale");
    expect(events).toEqual(["success:1m"]);
  });

  it("fails closed on a current response whose snapshot identity differs", async () => {
    const coordinator = new StockBetaChartLoadCoordinator();
    const failures: unknown[] = [];
    const outcome = await coordinator.run(
      request("000001.KRX", "1y"),
      async () => chartFor("000001.KRX", "1y", "00000000-0000-4000-8000-000000000202"),
      {
        onFailure: (error) => failures.push(error),
        onSuccess: () => undefined,
      },
    );

    expect(outcome).toBe("failure");
    expect(failures[0]).toBeInstanceOf(OwnerEquityV2ChartIntegrityError);
  });

  it("suppresses late work after unmount/dispose", async () => {
    const coordinator = new StockBetaChartLoadCoordinator();
    const pending = deferred<OwnerEquityV2ChartModel>();
    let signal: AbortSignal | undefined;
    let callbackCount = 0;
    const load = coordinator.run(
      request("000001.KRX", "1y"),
      (_request, nextSignal) => {
        signal = nextSignal;
        return pending.promise;
      },
      {
        onFailure: () => {
          callbackCount += 1;
        },
        onSuccess: () => {
          callbackCount += 1;
        },
      },
    );

    coordinator.dispose();
    expect(signal?.aborted).toBe(true);
    pending.resolve(chartFor("000001.KRX", "1y"));
    await expect(load).resolves.toBe("stale");
    expect(callbackCount).toBe(0);
  });
});
