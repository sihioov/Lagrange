const MEMBERSHIPS_PATH = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const LATEST_PATH = "/api/v1/research/owner-beta/equity-universe-v2/signals/latest";
const SCREEN_PATH = "/api/v1/research/owner-beta/equity-universe-v2/signals/screen";
const DETAIL_PATH = "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments";
const HASH = `sha256:${"a".repeat(64)}`;
const BASE_TIME = "2026-08-30T06:00:00Z";
const JOB_ID = "00000000-0000-4000-8000-000000000701";

let runtime = {
  disabledInstrumentIds: new Set(),
  memberships: [],
  pollCount: 0,
};

function rowCountFor(scenario) {
  const count = Number.parseInt(String(scenario.stockBetaRows ?? "31"), 10);
  return Number.isInteger(count) && count > 0 ? count : 31;
}

function instrumentIdForCode(code) {
  return `${code}.KRX`;
}

function generatedInstrumentId(index) {
  return `${String(index + 1).padStart(6, "0")}.KRX`;
}

function membershipIdFor(code) {
  const numeric = Number.parseInt(code, 10);
  return `00000000-0000-4000-8000-${String(600 + numeric).padStart(12, "0")}`;
}

function lifecycleCoverage(lifecycle) {
  const observed = {
    REQUESTED: 0,
    VALIDATING: 20,
    BACKFILLING: 80,
    MATERIALIZING: 200,
    READY: 261,
    INSUFFICIENT_HISTORY: 40,
    FAILED: 40,
    DISABLED: 261,
  }[lifecycle];
  return {
    first_session: observed >= 261 ? "2025-08-01" : undefined,
    last_session: observed >= 261 ? "2026-08-29" : undefined,
    minimum_observed_sessions: 121,
    observed_sessions: observed,
    target_observed_sessions: 261,
  };
}

function membership(code, lifecycle, failure) {
  return {
    coverage: lifecycleCoverage(lifecycle),
    ...(failure === undefined ? {} : { failure }),
    generation: ["READY", "DISABLED"].includes(lifecycle) ? 1 : 0,
    id: membershipIdFor(code),
    instrument_id: instrumentIdForCode(code),
    lifecycle,
    requested_at: BASE_TIME,
    updated_at: BASE_TIME,
  };
}

function policy() {
  const active = runtime.memberships.filter((item) => item.lifecycle !== "DISABLED").length;
  return {
    active_instruments: active,
    max_active_instruments: 100,
    minimum_observed_sessions: 121,
    remaining_capacity: 100 - active,
    target_observed_sessions: 261,
  };
}

function advancePendingMemberships() {
  if (
    !runtime.memberships.some((item) =>
      ["REQUESTED", "VALIDATING", "BACKFILLING", "MATERIALIZING"].includes(item.lifecycle),
    )
  ) {
    return;
  }
  runtime.pollCount += 1;
  const nextLifecycle = ["VALIDATING", "BACKFILLING", "MATERIALIZING", "READY"][
    Math.min(runtime.pollCount - 1, 3)
  ];
  runtime.memberships = runtime.memberships.map((item) => {
    if (!["REQUESTED", "VALIDATING", "BACKFILLING", "MATERIALIZING"].includes(item.lifecycle)) {
      return item;
    }
    return {
      ...item,
      coverage: lifecycleCoverage(nextLifecycle),
      failure: undefined,
      lifecycle: nextLifecycle,
    };
  });
}

export function resetStockBetaFixture(scenario = {}) {
  const seed = scenario.stockBetaSeed ?? "ready";
  const seedCode = String(scenario.stockBetaSeedCode ?? "000001").padStart(6, "0");
  if (seed === "empty") {
    runtime = { disabledInstrumentIds: new Set(), memberships: [], pollCount: 0 };
    return;
  }
  if (seed === "failed") {
    runtime = {
      disabledInstrumentIds: new Set(),
      memberships: [
        membership(seedCode, "FAILED", {
          code: "OWNER_EQUITY_BACKFILL_RETRYABLE",
          retryable: true,
        }),
      ],
      pollCount: 0,
    };
    return;
  }
  runtime = {
    disabledInstrumentIds: new Set(),
    memberships: [membership(seedCode, "READY")],
    pollCount: 0,
  };
}

function responseError(status, code) {
  return {
    body: { error: { code, message: code, request_id: "request-synthetic-stock-beta-v2" } },
    status,
  };
}

function mutationAuthorized(headers) {
  return Boolean(headers["x-csrf-token"] && headers["idempotency-key"]);
}

function signal(index, instrumentId) {
  return {
    average_trading_value_20: 1_000_000 + index,
    average_volume_20: 100_000 + index,
    condition: index % 3 === 0 ? "BULLISH" : index % 3 === 1 ? "NEUTRAL" : "BEARISH",
    generation: 1,
    instrument_id: instrumentId,
    max_drawdown_120: -0.1 - index / 1_000,
    rank: index + 1,
    return_120: 0.4 - index / 100,
    return_20: 0.1 - index / 1_000,
    return_60: 0.2 - index / 1_000,
    score: 100 - index,
    sma_20: 100 + index,
    sma_60: 99 + index,
    volatility_120: 0.2 + index / 1_000,
    volatility_20: 0.1 + index / 1_000,
    volatility_60: 0.15 + index / 1_000,
    volume_ratio_20_60: 1.2 + index / 1_000,
  };
}

function signalRows(scenario) {
  const count = rowCountFor(scenario);
  const membershipIds = runtime.memberships
    .filter((item) => item.lifecycle === "READY")
    .map((item) => item.instrument_id);
  const ids = [
    ...new Set([
      ...membershipIds,
      ...Array.from({ length: count + runtime.disabledInstrumentIds.size }, (_, index) =>
        generatedInstrumentId(index),
      ),
    ]),
  ]
    .filter((id) => !runtime.disabledInstrumentIds.has(id))
    .slice(0, count);
  return ids.map((id, index) => signal(index, id));
}

function snapshot(rows) {
  return {
    as_of: "2026-08-29",
    published_at: BASE_TIME,
    row_count: rows.length,
    snapshot_id: "00000000-0000-4000-8000-000000000501",
    universe_sha256: HASH,
  };
}

function signalPayload(scenario) {
  const rows = signalRows(scenario);
  return { rows, snapshot: snapshot(rows), top5: rows.slice(0, 5) };
}

export function stockBetaResponse(request) {
  const { body, headers, method, pathname, scenario } = request;
  if (
    pathname !== MEMBERSHIPS_PATH &&
    !pathname.startsWith(`${MEMBERSHIPS_PATH}/`) &&
    pathname !== LATEST_PATH &&
    pathname !== SCREEN_PATH &&
    !pathname.startsWith(`${DETAIL_PATH}/`)
  ) {
    return null;
  }
  if (scenario.role !== "owner") return responseError(403, "FORBIDDEN");
  if (scenario.stockBeta === "unavailable")
    return responseError(503, "OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE");
  if (scenario.stockBeta === "integrity")
    return responseError(503, "OWNER_EQUITY_INTEGRITY_FAILED");

  if (method === "GET" && pathname === MEMBERSHIPS_PATH) {
    advancePendingMemberships();
    return { body: { memberships: runtime.memberships, policy: policy() }, status: 200 };
  }
  if (method === "GET" && pathname.startsWith(`${MEMBERSHIPS_PATH}/`)) {
    const id = decodeURIComponent(pathname.slice(`${MEMBERSHIPS_PATH}/`.length));
    advancePendingMemberships();
    const resource = runtime.memberships.find((item) => item.id === id);
    return resource === undefined
      ? responseError(404, "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND")
      : { body: { membership: resource, policy: policy() }, status: 200 };
  }
  if (method === "POST" && pathname === MEMBERSHIPS_PATH) {
    if (!mutationAuthorized(headers)) return responseError(403, "CSRF_DENIED");
    if (typeof body?.instrument_code !== "string" || !/^\d{6}$/.test(body.instrument_code)) {
      return responseError(422, "INVALID_PARAMETER");
    }
    const instrumentId = instrumentIdForCode(body.instrument_code);
    const existing = runtime.memberships.find(
      (item) => item.instrument_id === instrumentId && item.lifecycle !== "DISABLED",
    );
    if (existing !== undefined) {
      return { body: { duplicate_active: true, job_id: JOB_ID, resource: existing }, status: 200 };
    }
    const resource = membership(body.instrument_code, "REQUESTED");
    runtime.memberships = [...runtime.memberships, resource];
    runtime.pollCount = 0;
    return { body: { duplicate_active: false, job_id: JOB_ID, resource }, status: 202 };
  }
  if (method === "POST" && pathname.endsWith("/retry")) {
    if (!mutationAuthorized(headers)) return responseError(403, "CSRF_DENIED");
    const id = decodeURIComponent(pathname.slice(`${MEMBERSHIPS_PATH}/`.length, -"/retry".length));
    const existing = runtime.memberships.find((item) => item.id === id);
    if (existing === undefined) return responseError(404, "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND");
    if (
      !(
        existing.lifecycle === "INSUFFICIENT_HISTORY" ||
        (existing.lifecycle === "FAILED" && existing.failure?.retryable)
      )
    ) {
      return responseError(409, "OWNER_EQUITY_INVALID_STATE");
    }
    const resource = {
      ...existing,
      coverage: lifecycleCoverage("REQUESTED"),
      failure: undefined,
      lifecycle: "REQUESTED",
    };
    runtime.memberships = runtime.memberships.map((item) => (item.id === id ? resource : item));
    runtime.pollCount = 0;
    return { body: { duplicate_active: false, job_id: JOB_ID, resource }, status: 202 };
  }
  if (method === "POST" && pathname.endsWith("/disable")) {
    if (!mutationAuthorized(headers)) return responseError(403, "CSRF_DENIED");
    const id = decodeURIComponent(
      pathname.slice(`${MEMBERSHIPS_PATH}/`.length, -"/disable".length),
    );
    const existing = runtime.memberships.find((item) => item.id === id);
    if (existing === undefined) return responseError(404, "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND");
    const resource = { ...existing, disabled_at: BASE_TIME, lifecycle: "DISABLED" };
    runtime.memberships = runtime.memberships.map((item) => (item.id === id ? resource : item));
    runtime.disabledInstrumentIds.add(existing.instrument_id);
    return { body: { duplicate_active: false, job_id: JOB_ID, resource }, status: 202 };
  }
  if (method === "GET" && pathname === LATEST_PATH) {
    if (!runtime.memberships.some((item) => item.lifecycle === "READY")) {
      return responseError(404, "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE");
    }
    const payload = signalPayload(scenario);
    return {
      body: { rows: payload.rows, snapshot: payload.snapshot, top5: payload.top5 },
      status: 200,
    };
  }
  if (method === "POST" && pathname === SCREEN_PATH) {
    if (!runtime.memberships.some((item) => item.lifecycle === "READY")) {
      return responseError(404, "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE");
    }
    const payload = signalPayload(scenario);
    const conditions = Array.isArray(body?.conditions) ? new Set(body.conditions) : null;
    const rows =
      conditions === null
        ? payload.rows
        : payload.rows.filter((item) => conditions.has(item.condition));
    return { body: { rows, snapshot: payload.snapshot }, status: 200 };
  }
  if (method === "GET" && pathname.startsWith(`${DETAIL_PATH}/`)) {
    if (!runtime.memberships.some((item) => item.lifecycle === "READY")) {
      return responseError(404, "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE");
    }
    const instrumentId = decodeURIComponent(pathname.slice(`${DETAIL_PATH}/`.length));
    const payload = signalPayload(scenario);
    const signalRow = payload.rows.find((item) => item.instrument_id === instrumentId);
    return signalRow === undefined
      ? responseError(404, "RESOURCE_NOT_FOUND")
      : { body: { signal: signalRow, snapshot: payload.snapshot }, status: 200 };
  }
  return null;
}

resetStockBetaFixture({ stockBetaRows: "31", stockBetaSeed: "ready" });
