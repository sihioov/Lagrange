import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  requestServiceStatus,
  retryAfterMilliseconds,
  SERVICE_STATUS_API_PATH,
  ServiceStatusRequestError,
} from "@/lib/operations/service-status-client";
import {
  parseServiceStatusResponse,
  SERVICE_STATUS_IDS,
  ServiceStatusContractError,
  type ServiceStatusState,
  serviceStatusOverall,
} from "@/lib/operations/service-status-contract";

type JsonPatch = {
  readonly op: "add" | "remove" | "replace";
  readonly path: string;
  readonly value?: unknown;
};

type ProjectionCase = {
  readonly id: string;
  readonly expected_patch: readonly JsonPatch[];
};

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
) as { readonly cases: readonly ProjectionCase[] };
const wireFixture = JSON.parse(
  readFileSync(
    new URL("../../../tests/fixtures/service-status/wire-cases.json", import.meta.url),
    "utf8",
  ),
) as {
  readonly cases: readonly {
    readonly id: string;
    readonly layer: string;
    readonly input: unknown;
    readonly expected: Record<string, unknown>;
  }[];
};

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function pointerParts(path: string): string[] {
  if (path === "") return [];
  if (!path.startsWith("/")) throw new Error("Fixture pointer must be absolute");
  return path
    .slice(1)
    .split("/")
    .map((segment) => segment.replace(/~1/g, "/").replace(/~0/g, "~"));
}

function applyPointerPatch(root: unknown, patch: JsonPatch): unknown {
  const parts = pointerParts(patch.path);
  if (parts.length === 0) {
    if (patch.op === "remove") throw new Error("Root removal is not supported by this fixture");
    return clone(patch.value);
  }

  let parent = root;
  for (const part of parts.slice(0, -1)) {
    if (Array.isArray(parent)) {
      const index = Number(part);
      if (!Number.isInteger(index) || index < 0 || index >= parent.length)
        throw new Error("Missing array pointer");
      parent = parent[index];
    } else if (typeof parent === "object" && parent !== null) {
      const object = parent as Record<string, unknown>;
      if (!Object.hasOwn(object, part)) throw new Error("Missing object pointer");
      parent = object[part];
    } else {
      throw new Error("Pointer traversed a scalar");
    }
  }

  const key = parts.at(-1);
  if (key === undefined) throw new Error("Missing pointer key");
  if (Array.isArray(parent)) {
    const index = key === "-" ? parent.length : Number(key);
    if (!Number.isInteger(index) || index < 0 || index > parent.length)
      throw new Error("Invalid array pointer");
    if (patch.op === "remove") {
      if (index >= parent.length) throw new Error("Missing array entry");
      parent.splice(index, 1);
    } else if (patch.op === "add") {
      parent.splice(index, 0, clone(patch.value));
    } else {
      if (index >= parent.length) throw new Error("Missing array entry");
      parent[index] = clone(patch.value);
    }
  } else if (typeof parent === "object" && parent !== null) {
    const object = parent as Record<string, unknown>;
    if (patch.op === "remove") {
      if (!Object.hasOwn(object, key)) throw new Error("Missing object entry");
      delete object[key];
    } else {
      if (patch.op === "replace" && !Object.hasOwn(object, key))
        throw new Error("Missing object entry");
      object[key] = clone(patch.value);
    }
  } else {
    throw new Error("Pointer target is a scalar");
  }
  return root;
}

function expectedProjection(id: string): unknown {
  const fixtureCase = projectionFixture.cases.find((candidate) => candidate.id === id);
  if (fixtureCase === undefined) throw new Error("Projection case is missing");
  return fixtureCase.expected_patch.reduce(
    (document, patch) => applyPointerPatch(document, patch),
    clone(baseOutput),
  );
}

function wireCase(id: string) {
  const result = wireFixture.cases.find((candidate) => candidate.id === id);
  if (result === undefined) throw new Error("Wire case is missing");
  return result;
}

function futureStreamDiagnostic(
  key: "last_received_at" | "oldest_provider_event_at" | "last_state_change_at",
  status: "degraded" | "failed",
  live: number | null,
  halted: number | null,
) {
  const response = clone(baseOutput) as Record<string, unknown>;
  const services = response["services"] as Array<Record<string, unknown>>;
  const stream = services.find((row) => row["id"] === "market-stream");
  if (stream === undefined) throw new Error("Base fixture has no market-stream row");
  const metrics = stream["metrics"] as Record<string, unknown>;
  response["overall_status"] = status;
  stream["status"] = status;
  stream["work_status"] = status === "failed" ? "failed" : "stale";
  stream["reason_codes"] =
    status === "failed" ? ["CONNECTION_LOST", "SOURCE_FUTURE"] : ["SOURCE_FUTURE"];
  metrics[key] = "2032-02-12T01:00:00.000001Z";
  metrics["live_identities"] = live;
  metrics["halted_identities"] = halted;
  return { response, services, stream, metrics };
}

describe("service status response contract", () => {
  it("accepts the base response and every independent expected projection from the 124-case corpus", () => {
    expect(projectionFixture.cases).toHaveLength(124);
    expect(() => parseServiceStatusResponse(baseOutput)).not.toThrow();
    for (const fixtureCase of projectionFixture.cases) {
      let response: ReturnType<typeof parseServiceStatusResponse>;
      try {
        response = parseServiceStatusResponse(expectedProjection(fixtureCase.id));
      } catch (cause) {
        throw new Error(`Projection ${fixtureCase.id} was rejected`, { cause });
      }
      expect(response.services).toHaveLength(22);
    }
  });

  it("retains a future quote event only when the projection diagnoses it as non-live", () => {
    const diagnosed = expectedProjection("oldest_provider_trade_at_future1efuture06");
    expect(() => parseServiceStatusResponse(diagnosed)).not.toThrow();

    const undiagnosed = clone(diagnosed) as Record<string, unknown>;
    const services = undiagnosed["services"] as Array<Record<string, unknown>>;
    const marketStream = services[12];
    if (marketStream === undefined) throw new Error("Projection has no market-stream row");
    marketStream["reason_codes"] = ["DATA_STALE"];
    expect(() => parseServiceStatusResponse(undiagnosed)).toThrow(ServiceStatusContractError);

    const impossibleHalted = clone(diagnosed) as Record<string, unknown>;
    const haltedServices = impossibleHalted["services"] as Array<Record<string, unknown>>;
    const haltedStream = haltedServices[12];
    if (haltedStream === undefined) throw new Error("Projection has no market-stream row");
    const haltedMetrics = haltedStream["metrics"] as Record<string, unknown>;
    haltedMetrics["halted_identities"] = 1;
    expect(() => parseServiceStatusResponse(impossibleHalted)).toThrow(ServiceStatusContractError);
  });

  it.each([
    ["last_received_at", "degraded", 0, 0],
    ["last_received_at", "degraded", 1, 0],
    ["last_state_change_at", "degraded", 2, 0],
    ["oldest_provider_event_at", "failed", 0, 0],
    ["oldest_provider_event_at", "failed", null, null],
  ] as const)(
    "preserves diagnosed %s with %s status and coverage %s/%s without losing other rows",
    (key, status, live, halted) => {
      const { response, metrics } = futureStreamDiagnostic(key, status, live, halted);
      const parsed = parseServiceStatusResponse(response);
      const stream = parsed.services.find((row) => row.id === "market-stream");
      expect(parsed.services).toHaveLength(22);
      expect(stream?.status).toBe(status);
      expect(stream?.metrics[key]).toBe(metrics[key]);
      expect(stream?.metrics.live_identities).toBe(live);
      expect(stream?.metrics.halted_identities).toBe(halted);
      expect(stream?.valid_until).toBe("2032-02-12T01:00:09Z");
      const otherRows = parsed.services.filter((row) => row.id !== "market-stream");
      const original = parseServiceStatusResponse(baseOutput);
      expect(otherRows).toEqual(original.services.filter((row) => row.id !== "market-stream"));
    },
  );

  it.each(["last_received_at", "oldest_provider_event_at", "last_state_change_at"] as const)(
    "rejects future %s without a stream diagnosis or on another service",
    (key) => {
      const undiagnosed = futureStreamDiagnostic(key, "degraded", 0, 0);
      undiagnosed.stream["reason_codes"] = ["QUOTE_STALE"];
      expect(() => parseServiceStatusResponse(undiagnosed.response)).toThrow(
        ServiceStatusContractError,
      );

      const otherService = futureStreamDiagnostic(key, "degraded", 0, 0);
      const first = otherService.services[0];
      if (first === undefined) throw new Error("Base fixture has no first row");
      first["status"] = "degraded";
      first["reason_codes"] = ["SOURCE_FUTURE"];
      (first["metrics"] as Record<string, unknown>)[key] = otherService.metrics[key];
      expect(() => parseServiceStatusResponse(otherService.response)).toThrow(
        ServiceStatusContractError,
      );
    },
  );

  it("rejects positive coverage with a future minimum provider clock even alongside failure", () => {
    for (const [live, halted] of [
      [1, 0],
      [0, 1],
    ] as const) {
      const { response } = futureStreamDiagnostic(
        "oldest_provider_event_at",
        "failed",
        live,
        halted,
      );
      expect(() => parseServiceStatusResponse(response)).toThrow(ServiceStatusContractError);
    }
  });

  it.each(["healthy", "waiting", "unknown"] as const)(
    "rejects a future stream diagnostic presented as %s even with its reason code",
    (status) => {
      const { response, stream } = futureStreamDiagnostic("last_received_at", "degraded", 0, 0);
      stream["status"] = status;
      response["overall_status"] = status === "healthy" ? "waiting" : status;
      expect(() => parseServiceStatusResponse(response)).toThrow(ServiceStatusContractError);
    },
  );

  it("keeps generated-clock fixtures valid regardless of browser wall time", () => {
    const response = parseServiceStatusResponse(baseOutput);
    expect(response.generated_at).toBe("2032-02-12T01:00:00Z");
  });

  it("compares source timestamps to generated_at at microsecond precision", () => {
    const value = clone(baseOutput) as Record<string, unknown>;
    value["generated_at"] = "2032-02-12T01:00:00.000001Z";
    const services = value["services"] as Array<Record<string, unknown>>;
    const firstSources = services[0]?.["sources"] as Record<string, Record<string, unknown>>;
    const host = firstSources["host"];
    if (host === undefined) throw new Error("Base fixture has no host observation");
    host["observed_at"] = "2032-02-12T01:00:00.000002Z";
    expect(() => parseServiceStatusResponse(value)).toThrow(ServiceStatusContractError);

    host["observed_at"] = "2032-02-12T01:00:00.000001Z";
    expect(() => parseServiceStatusResponse(value)).not.toThrow();
  });

  it("accepts a future next run while rejecting future observations and mismatched totals", () => {
    const response = parseServiceStatusResponse(baseOutput);
    expect(response.services.find((row) => row.id === "kis-eod")?.metrics.next_run_at).toBe(
      "2032-02-12T07:30:00Z",
    );

    const futureObservation = clone(baseOutput) as Record<string, unknown>;
    const services = futureObservation["services"] as Array<Record<string, unknown>>;
    const first = services[0];
    if (first === undefined) throw new Error("Base fixture has no first row");
    first["observed_at"] = "2032-02-12T01:00:00.000001Z";
    expect(() => parseServiceStatusResponse(futureObservation)).toThrow(ServiceStatusContractError);

    const wrongTotal = clone(baseOutput) as Record<string, unknown>;
    wrongTotal["overall_status"] = "healthy";
    expect(() => parseServiceStatusResponse(wrongTotal)).toThrow(ServiceStatusContractError);
  });

  it("matches the independent fixture priorities for overall status", () => {
    for (const id of [
      "overall_all_healthy",
      "overall_all_disabled",
      "overall_failed_beats_unknown",
      "overall_unknown_prevents_healthy",
    ]) {
      const vector = wireCase(id);
      const input = vector.input as { readonly statuses: readonly ServiceStatusState[] };
      const expected = vector.expected as { readonly overall_status: ServiceStatusState };
      const rows = input.statuses.map((status, index) => {
        const serviceId = SERVICE_STATUS_IDS[index];
        if (serviceId === undefined) throw new Error("Overall fixture has too many statuses");
        return { id: serviceId, status };
      });
      expect(serviceStatusOverall(rows)).toBe(expected.overall_status);
    }
  });

  it.each([
    [
      "unknown top-level field",
      (value: Record<string, unknown>) => {
        value["provider_message"] = "private prose";
      },
    ],
    [
      "wrong row order",
      (value: Record<string, unknown>) => {
        const rows = value["services"] as unknown[];
        [rows[0], rows[1]] = [rows[1], rows[0]];
      },
    ],
    [
      "unknown reason",
      (value: Record<string, unknown>) => {
        const rows = value["services"] as Array<Record<string, unknown>>;
        const first = rows[0];
        if (first === undefined) throw new Error("Fixture has no first service");
        first["reason_codes"] = ["PRIVATE_PROVIDER_TEXT"];
      },
    ],
    [
      "unexpected row field",
      (value: Record<string, unknown>) => {
        const rows = value["services"] as Array<Record<string, unknown>>;
        const first = rows[0];
        if (first === undefined) throw new Error("Fixture has no first service");
        first["debug"] = "provider response";
      },
    ],
  ])("rejects %s without accepting raw provider text", (_name, mutate) => {
    const value = clone(baseOutput) as Record<string, unknown>;
    mutate(value);
    expect(() => parseServiceStatusResponse(value)).toThrow(ServiceStatusContractError);
  });
});

describe("service status HTTP client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("makes only the fixed read-only status GET and does not parse failure prose", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify(baseOutput), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const response = await requestServiceStatus(new AbortController().signal);

    expect(response.schema_version).toBe("service-status.api.v1");
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(fetchMock.mock.calls[0]?.[0]).toBe(SERVICE_STATUS_API_PATH);
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
      method: "GET",
      cache: "no-store",
      credentials: "same-origin",
    });
    expect(SERVICE_STATUS_API_PATH).toBe("/api/v1/admin/service-status");
    expect(wireFixture.cases).toHaveLength(60);
    expect(wireCase("no_new_market_demand").expected).toMatchObject({
      new_leases: 0,
      renewals: 0,
      sse_connections: 0,
      provider_calls: 0,
    });
  });

  it.each([401, 403])("classifies HTTP %s as denial without reading its body", async (status) => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response("private provider prose", { status }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(requestServiceStatus(new AbortController().signal)).rejects.toMatchObject({
      kind: "denied",
    });
    expect(fetchMock).toHaveBeenCalledOnce();
  });

  it("classifies 503 as unavailable without exposing its response body", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response("private provider prose", { status: 503 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(requestServiceStatus(new AbortController().signal)).rejects.toMatchObject({
      kind: "unavailable",
    });
    expect(fetchMock).toHaveBeenCalledOnce();
  });

  it("maps a 429 Retry-After header into the typed throttle result", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response("private provider prose", {
          status: 429,
          headers: {
            Date: "Thu, 12 Feb 2032 01:00:00 GMT",
            "Retry-After": "8",
          },
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(requestServiceStatus(new AbortController().signal)).rejects.toMatchObject({
      kind: "throttled",
      retryAfterMs: 8_000,
    });
    expect(fetchMock).toHaveBeenCalledOnce();
  });

  it("uses Retry-After with server time when supplied", () => {
    const atBoundary = wireCase("rate_limit_equal");
    const beforeBoundary = wireCase("rate_limit_before");
    expect(atBoundary.expected["admit"]).toBe(true);
    expect(beforeBoundary.expected["admit"]).toBe(false);
    expect(retryAfterMilliseconds("5", "2032-02-12T01:00:00Z", 0)).toBe(5_000);
    expect(
      retryAfterMilliseconds("Thu, 12 Feb 2032 01:00:05 GMT", "Thu, 12 Feb 2032 01:00:00 GMT", 0),
    ).toBe(5_000);
  });

  it("rejects invalid response bodies as a typed unavailable result", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify({ provider_message: "private" }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(requestServiceStatus(new AbortController().signal)).rejects.toBeInstanceOf(
      ServiceStatusRequestError,
    );
  });
});
