import { createHash } from "node:crypto";

const MEMBERSHIPS_PATH = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const LATEST_PATH = "/api/v1/research/owner-beta/equity-universe-v2/signals/latest";
const SCREEN_PATH = "/api/v1/research/owner-beta/equity-universe-v2/signals/screen";
const DETAIL_PATH = "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments";
const BASE_TIME = "2026-08-30T06:00:00Z";
const AS_OF = "2026-08-28";
const SESSION_COUNT = 261;
const CHART_RANGES = ["1m", "3m", "6m", "1y"];
const CHART_CHANGES = [80, -50, 0, 120, -90, 45, -35, 0, 60, -70, 25];
const JOB_ID = "00000000-0000-4000-8000-000000000701";
const REQUEST_ID = "request-synthetic-stock-beta-v2";

let runtime = {
  disabledInstrumentIds: new Set(),
  memberships: [],
  pollCount: 0,
};

function rowCountFor(scenario) {
  const count = Number(scenario.stockBetaRows ?? 31);
  return Number.isInteger(count) && count >= 0 ? count : 31;
}

function normalizeCode(value) {
  const code = String(value ?? "000001");
  return /^\d{6}$/.test(code) ? code : code.padStart(6, "0").slice(-6);
}

function instrumentIdForCode(code) {
  return `${code}.KRX`;
}

function generatedCode(index) {
  return String(index + 1).padStart(6, "0");
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
    ...(observed >= SESSION_COUNT ? { first_session: "2025-08-29", last_session: AS_OF } : {}),
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

function pending(lifecycle) {
  return ["REQUESTED", "VALIDATING", "BACKFILLING", "MATERIALIZING"].includes(lifecycle);
}

function advancePendingMemberships() {
  if (!runtime.memberships.some((item) => pending(item.lifecycle))) return;

  runtime.pollCount += 1;
  const nextLifecycle = ["VALIDATING", "BACKFILLING", "MATERIALIZING", "READY"][
    Math.min(runtime.pollCount - 1, 3)
  ];
  runtime.memberships = runtime.memberships.map((item) => {
    if (!pending(item.lifecycle)) return item;
    return {
      ...item,
      coverage: lifecycleCoverage(nextLifecycle),
      failure: undefined,
      lifecycle: nextLifecycle,
      updated_at: BASE_TIME,
    };
  });
}

function seededCodes(count, seedCode) {
  const codes = [seedCode];
  for (let index = 0; codes.length < count; index += 1) {
    const code = generatedCode(index);
    if (!codes.includes(code)) codes.push(code);
  }
  return codes;
}

export function resetStockBetaFixture(scenario = {}) {
  const seed = scenario.stockBetaSeed ?? "ready";
  const seedCode = normalizeCode(scenario.stockBetaSeedCode ?? "000001");
  const count = rowCountFor(scenario);

  if (seed === "empty" || count === 0) {
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
    memberships: seededCodes(count, seedCode).map((code) => membership(code, "READY")),
    pollCount: 0,
  };
}

function responseError(status, code) {
  return {
    body: { error: { code, message: code, request_id: REQUEST_ID } },
    status,
  };
}

function mutationAuthorized(headers = {}) {
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

function signalRows() {
  return runtime.memberships
    .filter((item) => item.lifecycle === "READY")
    .filter((item) => !runtime.disabledInstrumentIds.has(item.instrument_id))
    .map((item, index) => signal(index, item.instrument_id));
}

function isChartPath(pathname) {
  return new RegExp(`^${DETAIL_PATH}/[^/]+/chart$`).test(pathname);
}

function chartInstrumentId(pathname) {
  const prefix = `${DETAIL_PATH}/`;
  const encoded = pathname.slice(prefix.length, -"/chart".length);
  return decodeURIComponent(encoded);
}

function chartQuery(request) {
  return new URLSearchParams(request.query ?? "");
}

function sessionDates() {
  const dates = [];
  const cursor = new Date(`${AS_OF}T00:00:00Z`);
  while (dates.length < SESSION_COUNT) {
    const day = cursor.getUTCDay();
    if (day !== 0 && day !== 6) dates.push(cursor.toISOString().slice(0, 10));
    cursor.setUTCDate(cursor.getUTCDate() - 1);
  }
  return dates.reverse();
}

const SESSION_DATES = sessionDates();

function fullChartBars(instrumentId) {
  const code = Number.parseInt(instrumentId.slice(0, 6), 10);
  let previousClose = 10_000 + code * 100;
  const bars = [];

  for (let index = 0; index < SESSION_COUNT; index += 1) {
    const open = previousClose;
    const change =
      index === 0
        ? 0
        : index === SESSION_COUNT - 1
          ? 75
          : (CHART_CHANGES[index % CHART_CHANGES.length] ?? 0);
    const close = Math.max(1, open + change);
    const wick = 20 + (index % 4) * 10;
    const volume = index % 47 === 0 ? 0 : 100_000 + code * 1_000 + (index % 17) * 1_000;
    bars.push({
      close,
      high: Math.max(open, close) + wick,
      low: Math.max(1, Math.min(open, close) - wick),
      open,
      session_date: SESSION_DATES[index],
      sma_20: null,
      sma_60: null,
      volume,
    });
    previousClose = close;
  }

  return bars.map((bar, index) => ({
    ...bar,
    sma_20:
      index < 19
        ? null
        : Number(
            (
              bars.slice(index - 19, index + 1).reduce((sum, item) => sum + item.close, 0) / 20
            ).toFixed(2),
          ),
    sma_60:
      index < 59
        ? null
        : Number(
            (
              bars.slice(index - 59, index + 1).reduce((sum, item) => sum + item.close, 0) / 60
            ).toFixed(2),
          ),
  }));
}

function chartBarsForRange(bars, range) {
  const months = { "1m": 1, "3m": 3, "6m": 6, "1y": 12 }[range];
  const boundary = new Date(`${AS_OF}T00:00:00Z`);
  boundary.setUTCMonth(boundary.getUTCMonth() - months);
  const boundaryDate = boundary.toISOString().slice(0, 10);
  return bars.filter((bar) => bar.session_date >= boundaryDate);
}

function chartDelayFor(scenario, instrumentId, range) {
  const configured = scenario.stockBetaChartDelays;
  if (configured === null || typeof configured !== "object" || Array.isArray(configured)) return 0;
  const value = configured[`${instrumentId}:${range}`] ?? configured[instrumentId] ?? 0;
  return Number.isInteger(value) && value > 0 && value <= 2_000 ? value : 0;
}

function chartPayload(signalRow, snapshotData, range, scenario) {
  const allBars = fullChartBars(signalRow.instrument_id);
  const bars = chartBarsForRange(allBars, range);
  const latestBar = allBars[allBars.length - 1];
  const previousBar = allBars[allBars.length - 2];
  const freshness = ["CURRENT", "STALE", "UNVERIFIABLE"].includes(scenario.stockBetaChartFreshness)
    ? scenario.stockBetaChartFreshness
    : "CURRENT";
  return {
    as_of: AS_OF,
    bars,
    expected_as_of:
      freshness === "UNVERIFIABLE" ? null : freshness === "STALE" ? "2026-08-31" : AS_OF,
    freshness,
    generation: signalRow.generation,
    instrument_id: signalRow.instrument_id,
    latest: {
      change: latestBar.close - previousBar.close,
      change_rate: (latestBar.close - previousBar.close) / previousBar.close,
      close: latestBar.close,
      session_date: AS_OF,
      volume: latestBar.volume,
    },
    price_semantics: "ORIGINAL_UNADJUSTED",
    range,
    snapshot_id: snapshotData.snapshot_id,
    warnings: ["NOT_REALTIME", "CORPORATE_ACTIONS_NOT_ADJUSTED", "RESEARCH_ONLY"],
  };
}

function universeHash(rows) {
  const bytes = rows.map((row) => row.instrument_id).join("\n");
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function snapshot(rows) {
  const hash = universeHash(rows);
  return {
    as_of: AS_OF,
    published_at: BASE_TIME,
    row_count: rows.length,
    snapshot_id: `00000000-0000-4000-8000-${hash.slice(-12)}`,
    universe_sha256: hash,
  };
}

function signalPayload() {
  const rows = signalRows();
  return { rows, snapshot: snapshot(rows), top5: rows.slice(0, 5) };
}

function isSignalPath(pathname) {
  return (
    pathname === LATEST_PATH || pathname === SCREEN_PATH || pathname.startsWith(`${DETAIL_PATH}/`)
  );
}

function scenarioSignalError(scenario, pathname) {
  if (!isSignalPath(pathname)) return null;
  if (scenario.stockBeta === "unavailable") {
    return responseError(503, "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE");
  }
  if (scenario.stockBeta === "integrity") {
    return responseError(503, "OWNER_EQUITY_INTEGRITY_FAILED");
  }
  if (scenario.stockBeta === "generic") return responseError(500, "INTERNAL");
  if (isChartPath(pathname)) {
    if (scenario.stockBetaChartState === "unavailable") {
      return responseError(503, "OWNER_EQUITY_CHART_UNAVAILABLE");
    }
    if (scenario.stockBetaChartState === "integrity") {
      return responseError(503, "OWNER_EQUITY_INTEGRITY_FAILED");
    }
    if (scenario.stockBetaChartState === "forbidden") {
      return responseError(403, "FORBIDDEN");
    }
    if (scenario.stockBetaChartState === "not_found") {
      return responseError(404, "RESOURCE_NOT_FOUND");
    }
  }
  return null;
}

function snapshotUnavailableIfNeeded(scenario) {
  const ready = runtime.memberships.some((item) => item.lifecycle === "READY");
  if (!ready && scenario.stockBetaSnapshot !== "empty") {
    return responseError(404, "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE");
  }
  return null;
}

export function stockBetaResponse(request) {
  const { body, headers, method, pathname, scenario } = request;
  const isMembershipPath =
    pathname === MEMBERSHIPS_PATH || pathname.startsWith(`${MEMBERSHIPS_PATH}/`);
  if (!isMembershipPath && !isSignalPath(pathname)) return null;
  if (scenario.role !== "owner") return responseError(403, "FORBIDDEN");

  const scenarioError = scenarioSignalError(scenario, pathname);
  if (scenarioError !== null) return scenarioError;

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
      existing.lifecycle !== "INSUFFICIENT_HISTORY" &&
      !(existing.lifecycle === "FAILED" && existing.failure?.retryable)
    ) {
      return responseError(409, "OWNER_EQUITY_INVALID_STATE");
    }
    const resource = {
      ...existing,
      coverage: lifecycleCoverage("REQUESTED"),
      failure: undefined,
      lifecycle: "REQUESTED",
      updated_at: BASE_TIME,
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
    const unavailable = snapshotUnavailableIfNeeded(scenario);
    if (unavailable !== null) return unavailable;
    return { body: signalPayload(), status: 200 };
  }

  if (method === "GET" && isChartPath(pathname)) {
    const unavailable = snapshotUnavailableIfNeeded(scenario);
    if (unavailable !== null) return unavailable;
    const instrumentId = chartInstrumentId(pathname);
    const payload = signalPayload();
    const signalRow = payload.rows.find((row) => row.instrument_id === instrumentId);
    if (signalRow === undefined) return responseError(404, "RESOURCE_NOT_FOUND");

    const query = chartQuery(request);
    const snapshotId = query.get("snapshot_id");
    const range = query.get("range");
    if (snapshotId === null || !CHART_RANGES.includes(range)) {
      return responseError(400, "INVALID_PARAMETER");
    }
    if (snapshotId !== payload.snapshot.snapshot_id) {
      return responseError(503, "OWNER_EQUITY_INTEGRITY_FAILED");
    }
    const result = {
      body: chartPayload(signalRow, payload.snapshot, range, scenario),
      status: 200,
    };
    const delayMs = chartDelayFor(scenario, instrumentId, range);
    return delayMs === 0 ? result : { ...result, delayMs };
  }

  if (method === "POST" && pathname === SCREEN_PATH) {
    const unavailable = snapshotUnavailableIfNeeded(scenario);
    if (unavailable !== null) return unavailable;
    const payload = signalPayload();
    const conditions = Array.isArray(body?.conditions) ? new Set(body.conditions) : null;
    const ids = Array.isArray(body?.instrument_ids) ? new Set(body.instrument_ids) : null;
    const rows = payload.rows.filter(
      (row) =>
        (conditions === null || conditions.has(row.condition)) &&
        (ids === null || ids.has(row.instrument_id)),
    );
    return { body: { rows, snapshot: snapshot(rows) }, status: 200 };
  }

  if (method === "GET" && pathname.startsWith(`${DETAIL_PATH}/`)) {
    const unavailable = snapshotUnavailableIfNeeded(scenario);
    if (unavailable !== null) return unavailable;
    const instrumentId = decodeURIComponent(pathname.slice(`${DETAIL_PATH}/`.length));
    const payload = signalPayload();
    const signalRow = payload.rows.find((row) => row.instrument_id === instrumentId);
    return signalRow === undefined
      ? responseError(404, "RESOURCE_NOT_FOUND")
      : { body: { signal: signalRow, snapshot: payload.snapshot }, status: 200 };
  }

  return null;
}

resetStockBetaFixture({ stockBetaRows: 31, stockBetaSeed: "ready" });
