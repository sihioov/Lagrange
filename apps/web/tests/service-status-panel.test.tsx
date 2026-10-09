import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ServiceStatusPanel,
  ServiceStatusPanelView,
} from "@/components/service-status/service-status-panel";
import {
  parseServiceStatusResponse,
  type ServiceStatusResponse,
  type ServiceStatusService,
  type ServiceStatusState,
  serviceStatusContributesToOverall,
} from "@/lib/operations/service-status-contract";
import type { ServiceStatusSnapshot as ControllerSnapshot } from "@/lib/operations/service-status-controller";
import { renderAsyncPage } from "./support/render-async-page";

const baseOutput = JSON.parse(
  readFileSync(
    new URL("../../../tests/fixtures/service-status/base-output.json", import.meta.url),
    "utf8",
  ),
) as unknown;
const projectionFixture = JSON.parse(
  readFileSync(
    new URL("../../../tests/fixtures/service-status/cases.json", import.meta.url),
    "utf8",
  ),
) as {
  readonly cases: readonly {
    readonly id: string;
    readonly expected_patch: readonly {
      readonly op: string;
      readonly path: string;
      readonly value?: unknown;
    }[];
  }[];
};

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function responseWithPositiveV2Coverage(): ServiceStatusResponse {
  const response = clone(baseOutput) as {
    services: Array<{ metrics: Record<string, unknown> }>;
  };
  const fixtureCase = projectionFixture.cases.find(
    (candidate) => candidate.id === "v2_supported_identity_count_100",
  );
  if (fixtureCase === undefined) throw new Error("100-instrument projection fixture is missing");
  for (const patch of fixtureCase.expected_patch) {
    if (patch.op !== "replace") throw new Error("Expected a replacement fixture patch");
    const parts = patch.path.split("/");
    const serviceIndex = parts[2];
    const metricKey = parts[4];
    if (parts[1] !== "services" || parts[3] !== "metrics")
      throw new Error("Unexpected fixture target");
    const service = response.services[Number(serviceIndex)];
    if (service === undefined || metricKey === undefined)
      throw new Error("Fixture target is missing");
    service.metrics[metricKey] = patch.value;
  }
  return parseServiceStatusResponse(response);
}

function snapshotFor(
  response: ServiceStatusResponse,
  options: { readonly stale?: boolean; readonly issue?: ControllerSnapshot["issue"] } = {},
): ControllerSnapshot {
  const stale = options.stale ?? false;
  const counts = {
    healthy: 0,
    waiting: 0,
    degraded: 0,
    failed: 0,
    unknown: 0,
    disabled: 0,
  };
  const rows = response.services.map((service) => {
    const status: ServiceStatusState = stale ? "unknown" : service.status;
    if (serviceStatusContributesToOverall(service.id)) counts[status] += 1;
    const reasonCodes: ServiceStatusService["reason_codes"] = stale
      ? ["OBSERVATION_EXPIRED"]
      : service.reason_codes;
    return {
      service,
      status,
      processStatus: stale ? ("unknown" as const) : service.process_status,
      workStatus: stale ? ("unknown" as const) : service.work_status,
      reasonCodes,
      expired: stale,
    };
  });
  return {
    rows,
    overallStatus: stale ? "unknown" : response.overall_status,
    counts,
    issue: options.issue ?? null,
    isFetching: false,
    isStale: stale,
    lastRecordedAt: response.generated_at,
    policyState: response.policy_state,
    receivedAtMono: 0,
    nowMono: 0,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Owner service status panel", () => {
  it("renders a compact keyboard disclosure collapsed by default without fetching during SSR", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const markup = await renderAsyncPage(<ServiceStatusPanel locale="en" />);
    const details = markup.match(/<details[^>]*>/)?.[0] ?? "";

    expect(markup).toContain('data-testid="service-status-panel"');
    expect(markup).toContain("<summary");
    expect(details).not.toContain(" open");
    expect(markup).toContain("Service status");
    expect(markup).toContain("Loading service status");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("renders all 22 readable rows while counting the research executor only through daily market data", async () => {
    const response = parseServiceStatusResponse(baseOutput);
    const markup = await renderAsyncPage(
      <ServiceStatusPanelView locale="en" snapshot={snapshotFor(response)} />,
    );
    const body = markup.match(/<tbody>([\s\S]*?)<\/tbody>/)?.[1] ?? "";

    expect(body.match(/<tr\b/g) ?? []).toHaveLength(22);
    expect(markup).toContain("Research executor");
    expect(markup).toContain("Daily market data");
    expect(markup).toContain("21 services counted");
    expect(markup).toContain('<th scope="col">Process</th>');
    expect(markup).toContain('<th scope="col">Work and data</th>');
    expect(markup).toContain("Last success");
    expect(markup).toContain("Next run");
    expect(markup).not.toContain("owner-equity-v2-runner");
    expect(markup).not.toContain("actor_shared");
    expect(markup).not.toContain("QUEUE_IDLE");
  });

  it("keeps the 100-instrument V2 coverage independent from live quote counts", async () => {
    const response = responseWithPositiveV2Coverage();
    const markup = await renderAsyncPage(
      <ServiceStatusPanelView locale="en" snapshot={snapshotFor(response)} />,
    );

    expect(markup).toContain("Expected instruments");
    expect(markup).toContain("Covered instruments");
    expect(markup).toContain(">100<");
    expect(markup).toContain("Live quote instruments");
    expect(markup).toContain(">2<");
    expect(markup).not.toContain(">30<");
  });

  it("shows old observations as unknown after an outage without retaining green process or work badges", async () => {
    const response = parseServiceStatusResponse(baseOutput);
    const snapshot = snapshotFor(response, { stale: true, issue: "unavailable" });
    const markup = await renderAsyncPage(
      <ServiceStatusPanelView locale="ko" snapshot={snapshot} />,
    );

    expect(markup).toContain("오래된 정보");
    expect(markup).toContain("확인 불가");
    expect(markup).toContain("마지막 확인");
    expect(markup).toContain("KST");
    expect(snapshot.rows.every((row) => row.processStatus === "unknown")).toBe(true);
    expect(snapshot.rows.every((row) => row.workStatus === "unknown")).toBe(true);
    expect(snapshot.lastRecordedAt).toBe(response.generated_at);
  });

  it("shows unavailable instead of loading after a first-request transport failure", async () => {
    const response = parseServiceStatusResponse(baseOutput);
    const snapshot = {
      ...snapshotFor(response, { issue: "unavailable" }),
      rows: [],
      counts: { healthy: 0, waiting: 0, degraded: 0, failed: 0, unknown: 0, disabled: 0 },
      overallStatus: "unknown" as const,
      isStale: false,
      lastRecordedAt: null,
      receivedAtMono: null,
    };
    const markup = await renderAsyncPage(
      <ServiceStatusPanelView locale="en" snapshot={snapshot} />,
    );

    expect(markup).toContain("Status is unavailable.");
    expect(markup).toContain("No service details are available.");
    expect(markup).not.toContain("Loading service status");
  });
});
