import { describe, expect, it } from "vitest";
import { StockBetaSignalRefreshCoordinator } from "@/components/stock-beta/signal-refresh-coordinator";

type TestSnapshot = {
  readonly label: string;
  readonly rows: readonly { readonly instrument_id: string }[];
};

function deferred<Value>() {
  let resolve: (value: Value) => void = () => undefined;
  let reject: (error: unknown) => void = () => undefined;
  const promise = new Promise<Value>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, reject, resolve };
}

function snapshot(label: string, ...instrumentIds: string[]): TestSnapshot {
  return {
    label,
    rows: instrumentIds.map((instrument_id) => ({ instrument_id })),
  };
}

describe("StockBetaSignalRefreshCoordinator", () => {
  it("applies only the newest success when deferred requests finish out of order", async () => {
    const coordinator = new StockBetaSignalRefreshCoordinator<TestSnapshot>();
    const older = deferred<TestSnapshot>();
    const newer = deferred<TestSnapshot>();
    let visibleState = "initial";
    const handlers = {
      onFailure: () => {
        visibleState = "failure";
      },
      onSuccess: (value: TestSnapshot) => {
        visibleState = value.label;
      },
    };

    const olderRun = coordinator.run(() => older.promise, handlers);
    const newerRun = coordinator.run(() => newer.promise, handlers);

    newer.resolve(snapshot("newer", "000002.KRX"));
    await expect(newerRun).resolves.toBe("success");
    expect(visibleState).toBe("newer");

    older.resolve(snapshot("older", "000001.KRX"));
    await expect(olderRun).resolves.toBe("stale");
    expect(visibleState).toBe("newer");
  });

  it("keeps disabled state when stale success and stale failure settle afterward", async () => {
    const coordinator = new StockBetaSignalRefreshCoordinator<TestSnapshot>();
    const staleSuccess = deferred<TestSnapshot>();
    const staleFailure = deferred<TestSnapshot>();
    let visibleState = "ready";
    const handlers = {
      onFailure: () => {
        visibleState = "failure";
      },
      onSuccess: (value: TestSnapshot) => {
        visibleState = value.label;
      },
    };

    const successRun = coordinator.run(() => staleSuccess.promise, handlers);
    const failureRun = coordinator.run(() => staleFailure.promise, handlers);
    coordinator.blockInstrument("000001.KRX");
    visibleState = "disabled";

    staleSuccess.resolve(snapshot("restored", "000001.KRX"));
    staleFailure.reject(new Error("late failure"));

    await expect(successRun).resolves.toBe("stale");
    await expect(failureRun).resolves.toBe("stale");
    expect(visibleState).toBe("disabled");
  });

  it("rejects a latest snapshot containing a blocked instrument until removal is visible", async () => {
    const coordinator = new StockBetaSignalRefreshCoordinator<TestSnapshot>();
    let visibleState = "disabled";
    const handlers = {
      onFailure: () => {
        visibleState = "failure";
      },
      onSuccess: (value: TestSnapshot) => {
        visibleState = value.label;
      },
    };

    coordinator.blockInstrument("000001.KRX");
    await expect(
      coordinator.run(async () => snapshot("still-present", "000001.KRX", "000002.KRX"), handlers),
    ).resolves.toBe("blocked");
    expect(visibleState).toBe("disabled");

    await expect(
      coordinator.run(async () => snapshot("removed", "000002.KRX"), handlers),
    ).resolves.toBe("success");
    expect(visibleState).toBe("removed");
  });
});
