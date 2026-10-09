import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { ServiceStatusRequestError } from "@/lib/operations/service-status-client";
import {
  parseServiceStatusResponse,
  type ServiceStatusResponse,
} from "@/lib/operations/service-status-contract";
import {
  SERVICE_STATUS_BASE_POLL_MS,
  SERVICE_STATUS_MIN_REQUEST_GAP_MS,
  ServiceStatusController,
  type ServiceStatusRequest,
  type ServiceStatusScheduler,
} from "@/lib/operations/service-status-controller";

const baseResponse = parseServiceStatusResponse(
  JSON.parse(
    readFileSync(
      new URL("../../../tests/fixtures/service-status/base-output.json", import.meta.url),
      "utf8",
    ),
  ),
);
const wireFixture = JSON.parse(
  readFileSync(
    new URL("../../../tests/fixtures/service-status/wire-cases.json", import.meta.url),
    "utf8",
  ),
) as {
  readonly cases: readonly {
    readonly id: string;
    readonly layer: string;
    readonly input: Record<string, unknown>;
    readonly expected: Record<string, unknown>;
  }[];
};

function wireCase(id: string) {
  const result = wireFixture.cases.find((candidate) => candidate.id === id);
  if (result === undefined) throw new Error("Wire fixture case is missing");
  return result;
}

function responseWithValidity(validForMsAtGenerated: number): ServiceStatusResponse {
  const response = JSON.parse(JSON.stringify(baseResponse)) as {
    generated_at: string;
    services: Array<{ valid_until: string | null }>;
  };
  const validUntil = new Date(
    Date.parse(response.generated_at) + validForMsAtGenerated,
  ).toISOString();
  for (const service of response.services) service.valid_until = validUntil;
  return parseServiceStatusResponse(response);
}

class FakeScheduler implements ServiceStatusScheduler {
  private nextId = 0;
  private readonly timers = new Map<
    number,
    { readonly at: number; readonly callback: () => void }
  >();
  private current = 0;

  now(): number {
    return this.current;
  }

  setTimeout(callback: () => void, delayMs: number): unknown {
    const id = ++this.nextId;
    this.timers.set(id, { at: this.current + Math.max(0, delayMs), callback });
    return id;
  }

  clearTimeout(handle: unknown): void {
    if (typeof handle === "number") this.timers.delete(handle);
  }

  advance(milliseconds: number): void {
    const target = this.current + milliseconds;
    while (true) {
      const due = [...this.timers.entries()]
        .filter(([, timer]) => timer.at <= target)
        .sort((left, right) => left[1].at - right[1].at || left[0] - right[0])[0];
      if (due === undefined) break;
      this.current = due[1].at;
      this.timers.delete(due[0]);
      due[1].callback();
    }
    this.current = target;
  }

  pendingCount(): number {
    return this.timers.size;
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flushPromises(): Promise<void> {
  for (let index = 0; index < 6; index += 1) await Promise.resolve();
}

async function advanceAndFlush(clock: FakeScheduler, milliseconds: number): Promise<void> {
  clock.advance(milliseconds);
  await flushPromises();
}

function makeActiveController(
  request: ServiceStatusRequest,
  clock: FakeScheduler,
): ServiceStatusController {
  const controller = new ServiceStatusController(request, clock);
  controller.start();
  controller.setActive(true);
  return controller;
}

describe("service status controller polling", () => {
  it("keeps one 30-second poll loop when the disclosure is collapsed or expanded", async () => {
    const vector = wireCase("collapsed_equals_expanded");
    expect(vector.input).toMatchObject({
      elapsed_seconds: 90,
      initial_visible: true,
      earlier_validity_deadline: false,
    });
    const response = responseWithValidity(180_000);

    async function pollCount(expanded: boolean): Promise<number> {
      const clock = new FakeScheduler();
      let requests = 0;
      const controller = makeActiveController(async () => {
        requests += 1;
        return response;
      }, clock);
      await advanceAndFlush(clock, 0);
      for (let interval = 0; interval < 6; interval += 1) {
        if (expanded) controller.getSnapshot();
        await advanceAndFlush(clock, 15_000);
      }
      controller.stop();
      return requests;
    }

    const collapsedPolls = await pollCount(false);
    const expandedPolls = await pollCount(true);
    expect(collapsedPolls).toBe(vector.expected["collapsed_polls"]);
    expect(expandedPolls).toBe(vector.expected["expanded_polls"]);
    expect(collapsedPolls).toBe(4);
  });

  it("pauses while hidden or offline and refreshes once on resume subject to the request floor", async () => {
    const hidden = wireCase("hidden_pause");
    const resume = wireCase("resume_once");
    const clock = new FakeScheduler();
    let requests = 0;
    const controller = new ServiceStatusController(async () => {
      requests += 1;
      return responseWithValidity(180_000);
    }, clock);

    controller.start();
    clock.advance(Number(hidden.input["elapsed_seconds"]) * 1_000);
    expect(requests).toBe(hidden.expected["polls"]);

    controller.setActive(true);
    await advanceAndFlush(clock, 0);
    expect(requests).toBe(resume.expected["polls"]);
    controller.setActive(false);
    expect(requests).toBe(1);

    controller.setActive(true);
    await advanceAndFlush(clock, SERVICE_STATUS_MIN_REQUEST_GAP_MS - 1);
    expect(requests).toBe(1);
    await advanceAndFlush(clock, 1);
    expect(requests).toBe(2);
    controller.stop();
  });

  it("keeps the expiry timer active while a request is pending and expires every badge at its bound", async () => {
    const shortVector = wireCase("short_validity_refresh");
    const initial = deferred<ServiceStatusResponse>();
    const pendingRefresh = deferred<ServiceStatusResponse>();
    const clock = new FakeScheduler();
    let requests = 0;
    const controller = makeActiveController(async () => {
      requests += 1;
      return requests === 1 ? initial.promise : pendingRefresh.promise;
    }, clock);

    await advanceAndFlush(clock, 0);
    expect(requests).toBe(1);
    clock.advance(Number(shortVector.input["request_duration_ms"]));
    initial.resolve(baseResponse);
    await flushPromises();

    expect(controller.getSnapshot().rows.some((row) => row.status === "healthy")).toBe(true);
    const expectedPollAt = Number(shortVector.expected["next_request_after_response_ms"]);
    const expectedExpiryAt = Number(shortVector.expected["expire_after_response_ms"]);
    await advanceAndFlush(clock, expectedPollAt - 1);
    expect(requests).toBe(1);
    await advanceAndFlush(clock, 1);
    expect(requests).toBe(2);
    expect(controller.getSnapshot().isFetching).toBe(true);

    await advanceAndFlush(clock, expectedExpiryAt - expectedPollAt);
    const stream = controller.getSnapshot().rows.find((row) => row.service.id === "market-stream");
    expect(stream?.status).toBe("unknown");
    expect(stream?.processStatus).toBe("unknown");
    expect(stream?.workStatus).toBe("unknown");
    expect(stream?.expired).toBe(true);
    expect(controller.getSnapshot().isFetching).toBe(true);
    expect(controller.getSnapshot().lastRecordedAt).toBe(baseResponse.generated_at);
    await advanceAndFlush(clock, SERVICE_STATUS_BASE_POLL_MS * 2);
    expect(requests).toBe(2);
    controller.stop();
  });

  it("lets evidence expire before the minimum next-request time without a retry loop", async () => {
    const vector = wireCase("short_validity_respects_minimum_interval");
    const initial = deferred<ServiceStatusResponse>();
    const clock = new FakeScheduler();
    let requests = 0;
    const controller = makeActiveController(async () => {
      requests += 1;
      return requests === 1 ? initial.promise : new Promise<ServiceStatusResponse>(() => {});
    }, clock);

    await advanceAndFlush(clock, 0);
    clock.advance(Number(vector.input["request_duration_ms"]));
    initial.resolve(responseWithValidity(Number(vector.input["valid_for_ms_at_generated_at"])));
    await flushPromises();
    expect(controller.getSnapshot().rows.every((row) => row.expired)).toBe(false);

    const expiryMs = Number(vector.expected["expire_after_response_ms"]);
    await advanceAndFlush(clock, expiryMs);
    expect(controller.getSnapshot().overallStatus).toBe("unknown");
    expect(requests).toBe(1);
    await advanceAndFlush(
      clock,
      Number(vector.expected["next_request_after_response_ms"]) - expiryMs - 1,
    );
    expect(requests).toBe(1);
    await advanceAndFlush(clock, 1);
    expect(requests).toBe(2);
    expect(SERVICE_STATUS_BASE_POLL_MS).toBe(30_000);
    controller.stop();
  });

  it("marks equality expired and ignores browser wall-clock skew", async () => {
    const vector = wireCase("expiry_equal");
    const clock = new FakeScheduler();
    const controller = makeActiveController(async () => responseWithValidity(0), clock);
    await advanceAndFlush(clock, 0);

    expect(vector.expected["display_status"]).toBe("unknown");
    expect(controller.getSnapshot().overallStatus).toBe("unknown");
    expect(controller.getSnapshot().counts.healthy).toBe(0);
    expect(controller.getSnapshot().counts.unknown).toBe(21);
    expect(controller.getSnapshot().rows.every((row) => row.status === "unknown")).toBe(true);
    expect(controller.getSnapshot().rows.every((row) => row.processStatus === "unknown")).toBe(
      true,
    );
    expect(controller.getSnapshot().rows.every((row) => row.workStatus === "unknown")).toBe(true);
    controller.stop();
  });

  it("aborts on unmount and fences a late response from the old session generation", async () => {
    const vector = wireCase("late_response");
    const lateResponse = deferred<ServiceStatusResponse>();
    const clock = new FakeScheduler();
    const signalHolder: { current: AbortSignal | null } = { current: null };
    const controller = makeActiveController((requestSignal) => {
      signalHolder.current = requestSignal;
      return lateResponse.promise;
    }, clock);

    await advanceAndFlush(clock, 0);
    expect(signalHolder.current).not.toBeNull();
    expect(vector.expected["accepted"]).toBe(false);
    controller.stop();
    expect(signalHolder.current?.aborted).toBe(true);
    lateResponse.resolve(baseResponse);
    await flushPromises();
    expect(controller.getSnapshot().rows).toHaveLength(0);
    expect(controller.getSnapshot().lastRecordedAt).toBeNull();
    expect(clock.pendingCount()).toBe(0);
  });

  it("erases all cached Owner details after authorization denial and stops polling", async () => {
    const vector = wireCase("auth_denial_erases");
    const clock = new FakeScheduler();
    let requests = 0;
    const controller = makeActiveController(async () => {
      requests += 1;
      if (requests === 1) return responseWithValidity(180_000);
      throw new ServiceStatusRequestError("denied");
    }, clock);

    await advanceAndFlush(clock, 0);
    expect(controller.getSnapshot().rows).toHaveLength(22);
    await advanceAndFlush(clock, SERVICE_STATUS_BASE_POLL_MS);
    expect(controller.getSnapshot().issue).toBe("denied");
    expect(controller.getSnapshot().rows).toHaveLength(Number(vector.expected["cached_rows"]));
    expect(controller.getSnapshot().lastRecordedAt).toBeNull();
    await advanceAndFlush(clock, SERVICE_STATUS_BASE_POLL_MS * 2);
    expect(requests).toBe(2);
    controller.stop();
  });

  it("retains observation times while a network or 503 failure marks cached rows stale", async () => {
    const outage = wireCase("api_disconnect");
    const clock = new FakeScheduler();
    let requests = 0;
    const controller = makeActiveController(async () => {
      requests += 1;
      if (requests === 1) return responseWithValidity(180_000);
      throw new ServiceStatusRequestError("unavailable");
    }, clock);

    await advanceAndFlush(clock, 0);
    const originalTime = controller.getSnapshot().rows[0]?.service.observed_at;
    await advanceAndFlush(clock, SERVICE_STATUS_BASE_POLL_MS);

    const snapshot = controller.getSnapshot();
    expect(snapshot.issue).toBe("unavailable");
    expect(snapshot.overallStatus).toBe("unknown");
    expect(snapshot.isStale).toBe(true);
    expect(snapshot.rows).toHaveLength(22);
    expect(snapshot.rows.every((row) => row.status === "unknown")).toBe(true);
    expect(
      snapshot.rows.every((row) => row.processStatus === "unknown" && row.workStatus === "unknown"),
    ).toBe(true);
    expect(snapshot.rows[0]?.service.observed_at).toBe(originalTime);
    expect(snapshot.lastRecordedAt).not.toBeNull();
    expect(outage.expected).toMatchObject({
      display_status: "unknown",
      retain_original_timestamp: true,
    });
    controller.stop();
  });

  it("honors Retry-After and keeps the five-second floor after the preceding request settles", async () => {
    const clock = new FakeScheduler();
    let requests = 0;
    const controller = makeActiveController(async () => {
      requests += 1;
      if (requests === 1) throw new ServiceStatusRequestError("throttled", 8_000);
      return responseWithValidity(180_000);
    }, clock);

    await advanceAndFlush(clock, 0);
    await advanceAndFlush(clock, 7_999);
    expect(requests).toBe(1);
    await advanceAndFlush(clock, 1);
    expect(requests).toBe(2);
    expect(controller.getSnapshot().issue).toBeNull();
    controller.stop();
  });
});
