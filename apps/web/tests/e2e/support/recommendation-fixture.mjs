const CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const RUN_ID = "00000000-0000-4000-8000-000000000201";

const strategy = Object.freeze({
  description: "Ranks the governed universe with absolute and relative momentum.",
  display_name: "Dual momentum",
  id: "dual_momentum",
  latest_version: "2.3.1",
  parameter_schema: {
    properties: {
      lookback_months: {
        default: 12,
        description: "Trailing month window used by the server strategy.",
        maximum: 24,
        minimum: 1,
        title: "Lookback months",
        type: "integer",
      },
      top_n: {
        default: 4,
        description: "Maximum selected instruments before portfolio constraints.",
        maximum: 11,
        minimum: 1,
        title: "Top holdings",
        type: "integer",
      },
    },
    required: ["lookback_months", "top_n"],
    type: "object",
  },
  risk_description: "May hold cash when the absolute momentum gate fails.",
  state: "Validated",
});

function recommendationItems(exclusions) {
  const selected = {
    excluded: false,
    factors: { momentum_12m: "0.184200", normalized_score: "0.912300" },
    instrument_id: "069500.KRX",
    rank: 1,
    reason_codes: ["ABSOLUTE_MOMENTUM_PASS", "RELATIVE_RANK_1"],
    target_weight: "0.400000",
  };
  if (exclusions === "empty") {
    return [selected];
  }
  return [
    selected,
    {
      excluded: true,
      exclusion_reason: "Inverse products are outside the governed universe.",
      factors: { normalized_score: "-0.112000" },
      instrument_id: "114800.KRX",
      rank: null,
      reason_codes: ["UNIVERSE_POLICY_EXCLUSION"],
      target_weight: null,
    },
  ];
}

function recommendationRun(scenario, includeItems) {
  const warning =
    scenario.recommendation === "stale"
      ? "Stale result: Data is two sessions old; review the as-of date before acting."
      : "Portfolio constraints can leave part of the proposal in cash.";
  return {
    as_of: "2026-01-31",
    created_at: "2026-01-31T06:30:00Z",
    id: RUN_ID,
    items: includeItems ? recommendationItems(scenario.exclusions) : undefined,
    status: "SUCCEEDED",
    strategy_config_id: CONFIG_ID,
    summary: {
      data_version: "krx-eod@2026-01-31",
      engine_version: "selector@1.4.0",
      strategy_version: "dual_momentum@2.3.1",
      warnings: [warning],
    },
  };
}

function licensing(scenario) {
  const active = scenario.entitlement !== "blocked";
  return {
    as_of: "2026-01-31",
    datasets: [
      {
        covered: active,
        dataset_id: "krx-eod",
        effective_from: "2026-01-01",
        effective_until: "2026-12-31",
        state: active ? "ACTIVE" : "REVOKED",
        use_kind: "recommendation",
      },
      {
        covered: active,
        dataset_id: "krx-eod",
        effective_from: "2026-01-01",
        effective_until: "2026-12-31",
        state: active ? "ACTIVE" : "REVOKED",
        use_kind: "backtest",
      },
    ],
  };
}

function error(status, code, message) {
  return {
    body: { error: { code, message, request_id: "request-synthetic-recommendation" } },
    status,
  };
}

function mutationAuthorized(headers) {
  return Boolean(headers["x-csrf-token"] && headers["idempotency-key"]);
}

export function recommendationResponse(request) {
  const { body, headers, method, pathname, scenario } = request;
  if (method === "GET" && pathname === "/api/v1/auth/csrf") {
    return { body: { csrf_token: "synthetic-csrf" }, status: 200 };
  }
  if (method === "GET" && pathname === "/api/v1/licensing-status") {
    return { body: licensing(scenario), status: 200 };
  }
  if (method === "GET" && pathname === "/api/v1/strategies") {
    return {
      body: { has_more: false, items: [strategy], next_cursor: null },
      status: 200,
    };
  }
  if (method === "POST" && /^\/api\/v1\/strategies\/[^/]+\/configs$/.test(pathname)) {
    if (!mutationAuthorized(headers)) {
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    }
    const lookback = body?.config?.lookback_months;
    if (!Number.isInteger(lookback) || lookback < 1 || lookback > 24) {
      return error(422, "INVALID_STRATEGY_PARAMETER", "lookback_months must be 1 through 24");
    }
    return {
      body: {
        config: body.config,
        created_at: "2026-01-31T06:00:00Z",
        id: CONFIG_ID,
        is_active: true,
        strategy_id: "dual_momentum",
        strategy_version: "2.3.1",
        updated_at: "2026-01-31T06:00:00Z",
      },
      status: 201,
    };
  }
  if (scenario.entitlement === "blocked" && pathname.startsWith("/api/v1/recommendations")) {
    return error(403, "DATA_ENTITLEMENT_REQUIRED", "recommendation entitlement is inactive");
  }
  if (method === "GET" && pathname === "/api/v1/recommendations/latest") {
    return { body: { run: recommendationRun(scenario, true) }, status: 200 };
  }
  if (method === "GET" && pathname === "/api/v1/recommendations/runs") {
    return {
      body: {
        has_more: false,
        items: [recommendationRun(scenario, false)],
        next_cursor: null,
      },
      status: 200,
    };
  }
  if (method === "POST" && pathname === "/api/v1/recommendations/runs") {
    return mutationAuthorized(headers)
      ? { body: recommendationRun(scenario, false), status: 201 }
      : error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
  }
  return null;
}
