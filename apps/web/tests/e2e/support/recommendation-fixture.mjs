const CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const RUN_ID = "00000000-0000-4000-8000-000000000201";
const SUBMITTED_RUN_ID = "00000000-0000-4000-8000-000000000202";
const DATASET_VERSION_ID = "00000000-0000-4000-8000-000000000401";

let submitted = false;
let submittedPolls = 0;

function runIdFor(scenario, id = RUN_ID) {
  const match = /^u(\d)$/.exec(scenario.user ?? "u1");
  const index = match ? Number(match[1]) : 1;
  return index === 1 ? id : `${id.slice(0, -4)}${index}${id.slice(-3)}`;
}

const strategy = Object.freeze({
  description: "Ranks the governed Korean ETF universe by 12-minus-1 momentum.",
  display_name: "Relative momentum",
  id: "relative_momentum",
  latest_version: "1.0.0",
  parameter_schema: {
    properties: {
      lookback_months: {
        default: 12,
        maximum: 12,
        minimum: 6,
        title: "Lookback months",
        type: "integer",
      },
      top_n: { default: 3, maximum: 10, minimum: 1, title: "Top holdings", type: "integer" },
    },
    required: ["top_n", "lookback_months"],
    type: "object",
  },
  risk_description: "Momentum crash, reversal, concentration, and monthly turnover risk.",
  state: "Validated",
});

export function recommendationConfig() {
  return {
    config: { lookback_months: 12, top_n: 3 },
    created_at: "2026-01-31T06:00:00Z",
    id: CONFIG_ID,
    is_active: true,
    strategy_id: "relative_momentum",
    strategy_version: "1.0.0",
    updated_at: "2026-01-31T06:00:00Z",
  };
}

function recommendationItems(exclusions, submitted = false) {
  const selected = [
    {
      excluded: false,
      factors: { momentum_12_1: "0.184200", normalized_score: "0.912300" },
      instrument_id: "069500.KRX",
      rank: 1,
      reason_codes: ["RELATIVE_RANK_1"],
      target_weight: "0.400000",
    },
    {
      excluded: false,
      factors: { momentum_12_1: "0.121000", normalized_score: "0.654000" },
      instrument_id: "102110.KRX",
      rank: 2,
      reason_codes: ["RELATIVE_RANK_2"],
      target_weight: "0.400000",
    },
    ...(submitted
      ? [
          {
            excluded: false,
            factors: { momentum_12_1: "0.097000", normalized_score: "0.511000" },
            instrument_id: "132030.KRX",
            rank: 3,
            reason_codes: ["RELATIVE_RANK_3"],
            target_weight: "0.200000",
          },
        ]
      : []),
  ];
  if (exclusions === "empty") return selected;
  return [
    ...selected,
    {
      excluded: true,
      exclusion_reason: "Ranked below the configured top-N cutoff.",
      factors: { normalized_score: "-0.112000" },
      instrument_id: "132030.KRX",
      rank: null,
      reason_codes: ["TOP_N_EXCLUSION"],
      target_weight: null,
    },
  ];
}

function provenance() {
  return {
    dataset_manifest_sha256: "a".repeat(64),
    dataset_version_id: DATASET_VERSION_ID,
  };
}

function recommendationRun(
  scenario,
  { id = RUN_ID, includeItems = false, status = "SUCCEEDED" } = {},
) {
  const warning =
    scenario.recommendation === "stale"
      ? "Stale result: Data is two sessions old; review the as-of date before acting."
      : "Portfolio constraints leave 20% in cash.";
  return {
    as_of: "2026-01-31",
    created_at: id === SUBMITTED_RUN_ID ? "2026-01-31T07:30:00Z" : "2026-01-31T06:30:00Z",
    id: runIdFor(scenario, id),
    items: includeItems
      ? recommendationItems(scenario.exclusions, id === SUBMITTED_RUN_ID)
      : undefined,
    job_id: "00000000-0000-4000-8000-000000000301",
    provenance: provenance(),
    status,
    strategy_config_id: CONFIG_ID,
    summary: {
      cash_weight: "0.200000",
      dataset_id: "kr-etf-daily",
      dataset_version: "phase0-v2",
      factor_snapshot_hash: `sha256:${"b".repeat(64)}`,
      manifest_sha256: "a".repeat(64),
      origin: "synthetic",
      portfolio_snapshot_id: `sha256:${"c".repeat(64)}`,
      universe_snapshot_id: `sha256:${"d".repeat(64)}`,
      warnings: [warning],
    },
    trigger_kind: "MANUAL",
  };
}

function licensing(scenario) {
  const active = scenario.entitlement !== "blocked";
  return {
    as_of: "2026-01-31",
    datasets: [
      {
        covered: active,
        dataset_id: "kr-etf-daily",
        effective_from: "2026-01-01",
        effective_until: "2026-12-31",
        state: active ? "ACTIVE" : "REVOKED",
        use_kind: "recommendation",
      },
      {
        covered: active,
        dataset_id: "kr-etf-daily",
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
  if (method === "GET" && pathname === "/api/v1/licensing-status")
    return { body: licensing(scenario), status: 200 };
  if (method === "GET" && pathname === "/api/v1/strategies") {
    return { body: { has_more: false, items: [strategy], next_cursor: null }, status: 200 };
  }
  if (method === "POST" && /^\/api\/v1\/strategies\/[^/]+\/configs$/.test(pathname)) {
    if (!mutationAuthorized(headers))
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    const lookback = body?.config?.lookback_months;
    const topN = body?.config?.top_n;
    if (![6, 12].includes(lookback) || !Number.isInteger(topN) || topN < 1 || topN > 10) {
      return error(
        422,
        "INVALID_STRATEGY_PARAMETER",
        "lookback_months must be 6 or 12 and top_n must be between 1 and 10",
      );
    }
    return { body: { ...recommendationConfig(), config: body.config }, status: 201 };
  }
  if (scenario.entitlement === "blocked" && pathname.startsWith("/api/v1/recommendations")) {
    return error(403, "DATA_ENTITLEMENT_REQUIRED", "recommendation entitlement is inactive");
  }
  if (method === "GET" && pathname === "/api/v1/recommendations/latest") {
    const success = recommendationRun(scenario, { includeItems: true });
    const newest = submitted
      ? recommendationRun(scenario, {
          id: SUBMITTED_RUN_ID,
          status: submittedPolls === 0 ? "PENDING" : "SUCCEEDED",
        })
      : success;
    const completedSubmittedRun = recommendationRun(scenario, {
      id: SUBMITTED_RUN_ID,
      includeItems: true,
      status: "SUCCEEDED",
    });
    return {
      body: { latest_run: newest, run: submittedPolls > 0 ? completedSubmittedRun : success },
      status: 200,
    };
  }
  if (method === "GET" && pathname === "/api/v1/recommendations/runs") {
    const runs = [recommendationRun(scenario)];
    if (submitted)
      runs.unshift(
        recommendationRun(scenario, {
          id: SUBMITTED_RUN_ID,
          status: submittedPolls === 0 ? "PENDING" : "SUCCEEDED",
        }),
      );
    return { body: { has_more: false, items: runs, next_cursor: null }, status: 200 };
  }
  if (
    method === "GET" &&
    pathname === `/api/v1/recommendations/runs/${runIdFor(scenario, SUBMITTED_RUN_ID)}`
  ) {
    submittedPolls += 1;
    return {
      body: recommendationRun(scenario, {
        id: SUBMITTED_RUN_ID,
        includeItems: submittedPolls > 1,
        status: submittedPolls > 1 ? "SUCCEEDED" : "PENDING",
      }),
      status: 200,
    };
  }
  if (method === "GET" && pathname === `/api/v1/recommendations/runs/${runIdFor(scenario)}`) {
    return { body: recommendationRun(scenario, { includeItems: true }), status: 200 };
  }
  if (method === "POST" && pathname === "/api/v1/recommendations/runs") {
    if (!mutationAuthorized(headers))
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    submitted = true;
    submittedPolls = 0;
    return {
      body: recommendationRun(scenario, { id: SUBMITTED_RUN_ID, status: "PENDING" }),
      status: 201,
    };
  }
  return null;
}
