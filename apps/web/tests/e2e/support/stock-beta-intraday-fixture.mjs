import { stockBetaMembershipForIntraday } from "./stock-beta-fixture.mjs";

const DEMAND_PATH = "/api/v1/research/owner-beta/equity-universe-v2/quote-demands";
const CACHE_PATH = "/api/v1/research/owner-beta/equity-universe-v2/instruments";
const REQUEST_ID = "request-synthetic-stock-beta-intraday";
const CALENDAR_HASH = "a".repeat(64);
const WINDOW_HASH = `sha256:${"b".repeat(64)}`;
const RETAINED_REASON_BY_STATE = {
  "retained-provider-timeout": "PROVIDER_TIMEOUT",
  "retained-provider-rate-limited": "PROVIDER_RATE_LIMITED",
  "retained-provider-unavailable": "PROVIDER_UNAVAILABLE",
  "retained-provider-response-invalid": "PROVIDER_RESPONSE_INVALID",
  "retained-quote-value-invalid": "QUOTE_VALUE_INVALID",
  "retained-quote-budget-exhausted": "QUOTE_BUDGET_EXHAUSTED",
  "retained-producer-unavailable": "PRODUCER_UNAVAILABLE",
  "retained-no-active-demand": "NO_ACTIVE_DEMAND",
};

let runtime = createRuntime();

function createRuntime() {
  return {
    activeDemands: new Map(),
    demandPosts: 0,
    demandReleases: 0,
    demandRenewals: 0,
    createdDemands: [],
    maxActiveIdentityCount: 0,
    nextDemandNumber: 1,
    quoteGets: 0,
    quoteGetsByInstrument: new Map(),
    quoteSequenceIndexes: new Map(),
    quoteTimestamps: new Map(),
    quoteVersions: new Map(),
    rejectedDemandMembershipIds: [],
    rejectedQuoteIdentities: [],
    releasedDemandIds: [],
  };
}

export function resetStockBetaIntradayFixture() {
  runtime = createRuntime();
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

function isUuid(value) {
  return (
    typeof value === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[4-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)
  );
}

function instrumentIdFromPath(pathname) {
  const match = new RegExp(`^${CACHE_PATH}/([^/]+)/quote$`).exec(pathname);
  return match === null ? null : decodeURIComponent(match[1] ?? "");
}

function kstDate(timestamp) {
  const parts = new Intl.DateTimeFormat("en-CA", {
    day: "2-digit",
    month: "2-digit",
    timeZone: "Asia/Seoul",
    year: "numeric",
  }).formatToParts(new Date(timestamp));
  const values = new Map(parts.map((part) => [part.type, part.value]));
  return `${values.get("year") ?? ""}-${values.get("month") ?? ""}-${values.get("day") ?? ""}`;
}

function timestampFor(state, instrumentId) {
  const now = Date.now();
  if (state === "future-timestamp") return new Date(now + 24 * 60 * 60 * 1_000).toISOString();
  if (state === "prior-session-after-rollover") {
    return new Date(now - 24 * 60 * 60 * 1_000).toISOString();
  }
  if (state === "stale") return new Date(now - 31 * 1_000).toISOString();
  if (state === "advancing") {
    const previous = runtime.quoteTimestamps.get(instrumentId);
    const candidate = previous === undefined ? now - 10_000 : previous + 1_000;
    const timestamp = Math.min(candidate, now - 100);
    const value = new Date(timestamp).toISOString();
    runtime.quoteTimestamps.set(instrumentId, timestamp);
    return value;
  }
  return new Date(now - 1_000).toISOString();
}

function sessionFor(timestamp) {
  return {
    calendar_content_sha256: CALENDAR_HASH,
    calendar_source: "kis",
    calendar_source_version: "kis-chk-holiday-v1:schema-1",
    date: kstDate(timestamp),
    timezone: "Asia/Seoul",
    window_contract_sha256: WINDOW_HASH,
  };
}

function quoteValues(instrumentId, state, version) {
  const code = instrumentId.slice(0, 6);
  const defaults = {
    "000001": {
      base_price: "100000",
      change_from_previous_day: "1200",
      change_percent_from_previous_day: "1.20",
      direction: "UP",
      price: "101200.00",
    },
    "000002": {
      base_price: "200000",
      change_from_previous_day: "-1600",
      change_percent_from_previous_day: "-0.80",
      direction: "DOWN",
      price: "198400.00",
    },
  };
  const fallback = {
    base_price: String(100_000 + Number.parseInt(code, 10)),
    change_from_previous_day: "100",
    change_percent_from_previous_day: "0.10",
    direction: "UP",
    price: String(100_100 + Number.parseInt(code, 10)),
  };
  const values = { ...(defaults[code] ?? fallback) };
  if (state === "advancing" && code === "000001") {
    const step = Math.max(0, version - 1);
    values.price = (101_200 + step * 100).toFixed(2);
    values.change_from_previous_day = String(1_200 + step * 100);
    values.change_percent_from_previous_day = (1.2 + step * 0.1).toFixed(2);
  }
  if (state === "flat") {
    values.change_from_previous_day = "0";
    values.change_percent_from_previous_day = "0.00";
    values.direction = "FLAT";
  }
  if (state === "limit-up") values.direction = "LIMIT_UP";
  if (state === "limit-down") {
    values.change_from_previous_day = "-1200";
    values.change_percent_from_previous_day = "-1.20";
    values.direction = "LIMIT_DOWN";
    values.price = String(Math.max(1, Number.parseInt(values.base_price, 10) - 1_200));
  }
  return values;
}

function quotePayload(instrumentId, state) {
  const nextVersion = (runtime.quoteVersions.get(instrumentId) ?? 0) + 1;
  runtime.quoteVersions.set(instrumentId, nextVersion);
  const timestamp = timestampFor(state, instrumentId);
  const values = quoteValues(instrumentId, state, nextVersion);
  const payload = {
    base_price: values.base_price,
    change_from_previous_day: values.change_from_previous_day,
    change_percent_from_previous_day: values.change_percent_from_previous_day,
    direction: values.direction,
    last_success_at: timestamp,
    price: values.price,
    quote_version: String(nextVersion),
    received_at: timestamp,
  };
  if (state === "zero") payload.price = "0";
  if (state === "invalid") payload.direction = "NOT_A_DIRECTION";
  return payload;
}

function retainedReasonForState(state) {
  return RETAINED_REASON_BY_STATE[state] ?? null;
}

function quoteResponse(instrumentId, membershipId, generation, state) {
  if (state === "unknown-calendar" || state === "session-window-unknown") {
    return {
      body: {
        currency: "KRW",
        freshness: "UNAVAILABLE",
        generation,
        instrument_id: instrumentId,
        market_state: "UNKNOWN",
        membership_id: membershipId,
        next_poll_after_ms: 5_000,
        quote: null,
        reason_code:
          state === "unknown-calendar" ? "CALENDAR_UNAVAILABLE" : "SESSION_WINDOW_UNAVAILABLE",
        schema_version: 1,
        session: null,
        venue: "KRX",
      },
      status: 200,
    };
  }
  if (state === "closed") {
    const timestamp = timestampFor(state, instrumentId);
    return {
      body: {
        currency: "KRW",
        freshness: "UNAVAILABLE",
        generation,
        instrument_id: instrumentId,
        market_state: "CLOSED",
        membership_id: membershipId,
        next_poll_after_ms: 5_000,
        quote: null,
        reason_code: "SESSION_CLOSED",
        schema_version: 1,
        session: sessionFor(timestamp),
        venue: "KRX",
      },
      status: 200,
    };
  }
  const halted = state === "halted" || state === "temporary-halt";
  const quote = quotePayload(instrumentId, state);
  const response = {
    body: {
      currency: "KRW",
      freshness: state === "stale" ? "STALE" : "RECENT",
      generation,
      instrument_id: instrumentId,
      market_state: halted ? "HALTED" : "OPEN",
      membership_id: membershipId,
      next_poll_after_ms: 5_000,
      quote,
      reason_code: retainedReasonForState(state),
      schema_version: 1,
      session: sessionFor(quote.last_success_at),
      venue: "KRX",
    },
    status: 200,
  };
  if (state === "rate-limited" || state === "provider-timeout" || state === "unavailable") {
    response.body.freshness = "UNAVAILABLE";
    response.body.quote = null;
    response.body.reason_code =
      state === "rate-limited"
        ? "PROVIDER_RATE_LIMITED"
        : state === "provider-timeout"
          ? "PROVIDER_TIMEOUT"
          : "PROVIDER_UNAVAILABLE";
  }
  return response;
}

function nextQuoteState(scenario, instrumentId) {
  const sequence = scenario.stockBetaIntradayQuoteSequence;
  if (!Array.isArray(sequence) || sequence.length === 0) {
    return scenario.stockBetaIntradayState ?? "open";
  }
  const index = runtime.quoteSequenceIndexes.get(instrumentId) ?? 0;
  runtime.quoteSequenceIndexes.set(instrumentId, index + 1);
  const state = sequence[Math.min(index, sequence.length - 1)];
  return typeof state === "string" ? state : "open";
}

function delayFor(scenario, instrumentId) {
  const configured = scenario.stockBetaIntradayDelays;
  if (configured === null || typeof configured !== "object" || Array.isArray(configured)) return 0;
  const value = configured[instrumentId] ?? 0;
  return Number.isInteger(value) && value > 0 && value <= 5_000 ? value : 0;
}

function demandIdFor(number) {
  return `00000000-0000-4000-8000-${String(900 + number).padStart(12, "0")}`;
}

function activeIdentityKeys() {
  return new Set(
    [...runtime.activeDemands.values()].map((demand) =>
      [demand.membership_id, demand.instrument_id, String(demand.generation)].join("\u0000"),
    ),
  );
}

function demandResponse(body, demand) {
  return {
    body: {
      consumer_id: body.consumer_id,
      demand_id: demand.demand_id,
      generation: demand.generation,
      instrument_id: demand.instrument_id,
      lease_expires_at: new Date(Date.now() + 30_000).toISOString(),
      membership_id: demand.membership_id,
      renew_after_ms: 15_000,
      renewal_sequence: body.renewal_sequence,
      schema_version: 1,
    },
    status: 200,
  };
}

export function stockBetaIntradayResponse(request) {
  const { body, headers, method, pathname, query, scenario } = request;
  const instrumentId = instrumentIdFromPath(pathname);
  const isDemandPath = pathname === DEMAND_PATH || pathname.startsWith(`${DEMAND_PATH}/`);
  if (!isDemandPath && instrumentId === null) return null;
  if (scenario.role !== "owner") return responseError(403, "FORBIDDEN");
  if (scenario.stockBetaIntradayState === "disabled")
    return responseError(404, "RESOURCE_NOT_FOUND");

  if (method === "POST" && pathname === DEMAND_PATH) {
    runtime.demandPosts += 1;
    if (!mutationAuthorized(headers)) return responseError(403, "CSRF_DENIED");
    if (
      !isUuid(body?.consumer_id) ||
      !isUuid(body?.membership_id) ||
      !Number.isSafeInteger(body?.generation) ||
      !Number.isSafeInteger(body?.renewal_sequence)
    ) {
      return responseError(400, "INVALID_PARAMETER");
    }
    const identity = stockBetaMembershipForIntraday(body.membership_id);
    if (identity === null || identity.generation !== body.generation) {
      runtime.rejectedDemandMembershipIds.push(body.membership_id);
      return responseError(404, "RESOURCE_NOT_FOUND");
    }
    if (
      scenario.stockBetaIntradayFailure === "capacity" ||
      (scenario.stockBetaIntradayFailure === "capacity-after-two" &&
        runtime.activeDemands.size >= 2)
    ) {
      return responseError(429, "QUOTE_DEMAND_CAPACITY");
    }
    const existing = runtime.activeDemands.get(body.consumer_id);
    if (existing === undefined) {
      const demand = {
        demand_id: demandIdFor(runtime.nextDemandNumber),
        generation: body.generation,
        instrument_id: identity.instrument_id,
        membership_id: body.membership_id,
      };
      runtime.nextDemandNumber += 1;
      runtime.activeDemands.set(body.consumer_id, demand);
      runtime.createdDemands.push({
        consumer_id: body.consumer_id,
        demand_id: demand.demand_id,
        generation: demand.generation,
        instrument_id: demand.instrument_id,
        membership_id: demand.membership_id,
      });
      const identityCount = activeIdentityKeys().size;
      runtime.maxActiveIdentityCount = Math.max(runtime.maxActiveIdentityCount, identityCount);
      return demandResponse(body, demand);
    }
    runtime.demandRenewals += 1;
    return demandResponse(body, existing);
  }

  if (method === "DELETE" && pathname.startsWith(`${DEMAND_PATH}/`)) {
    runtime.demandReleases += 1;
    const demandId = decodeURIComponent(pathname.slice(`${DEMAND_PATH}/`.length));
    for (const [consumerId, demand] of runtime.activeDemands) {
      if (demand.demand_id === demandId) {
        runtime.activeDemands.delete(consumerId);
        runtime.releasedDemandIds.push(demandId);
      }
    }
    return { body: null, status: 204 };
  }

  if (method === "GET" && instrumentId !== null) {
    runtime.quoteGets += 1;
    runtime.quoteGetsByInstrument.set(
      instrumentId,
      (runtime.quoteGetsByInstrument.get(instrumentId) ?? 0) + 1,
    );
    if (scenario.stockBetaIntradayFailure === "503") {
      return responseError(503, "QUOTE_CACHE_UNAVAILABLE");
    }
    const params = new URLSearchParams(query ?? "");
    const membershipId = params.get("membership_id");
    const generationText = params.get("generation");
    const generation = generationText === null ? Number.NaN : Number.parseInt(generationText, 10);
    if (!isUuid(membershipId) || !Number.isSafeInteger(generation)) {
      return responseError(400, "INVALID_PARAMETER");
    }
    const identity = stockBetaMembershipForIntraday(membershipId);
    if (
      identity === null ||
      identity.instrument_id !== instrumentId ||
      identity.generation !== generation
    ) {
      runtime.rejectedQuoteIdentities.push({
        generation,
        instrument_id: instrumentId,
        membership_id: membershipId,
      });
      return responseError(404, "RESOURCE_NOT_FOUND");
    }
    const result = quoteResponse(
      instrumentId,
      identity.membership_id,
      identity.generation,
      nextQuoteState(scenario, instrumentId),
    );
    const delayMs = delayFor(scenario, instrumentId);
    return delayMs === 0 ? result : { ...result, delayMs };
  }

  return null;
}

export function stockBetaIntradayTestState() {
  const identities = [...activeIdentityKeys()].sort();
  return {
    active_consumer_count: runtime.activeDemands.size,
    active_identity_count: identities.length,
    active_identity_keys: identities,
    demand_posts: runtime.demandPosts,
    demand_releases: runtime.demandReleases,
    demand_renewals: runtime.demandRenewals,
    created_demands: runtime.createdDemands,
    max_active_identity_count: runtime.maxActiveIdentityCount,
    quote_gets: runtime.quoteGets,
    quote_gets_by_instrument: Object.fromEntries(
      [...runtime.quoteGetsByInstrument.entries()].sort(([left], [right]) =>
        left.localeCompare(right),
      ),
    ),
    rejected_demand_membership_ids: runtime.rejectedDemandMembershipIds,
    rejected_quote_identities: runtime.rejectedQuoteIdentities,
    released_demand_ids: runtime.releasedDemandIds,
  };
}
