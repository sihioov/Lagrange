const CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const RUN_ID = "00000000-0000-4000-8000-000000000201";
const SUBMITTED_RUN_ID = "00000000-0000-4000-8000-000000000202";
const DATASET_VERSION_ID = "00000000-0000-4000-8000-000000000401";
const OWNER_BETA_RUN_ID = "00000000-0000-4000-8000-000000000601";
const OWNER_BETA_JOB_ID = "00000000-0000-4000-8000-000000000701";
const OWNER_BETA_SHA256 = `sha256:${"a".repeat(64)}`;
const OWNER_BETA_ETF_IDS = Object.freeze([
  "069500.KRX",
  "102110.KRX",
  "114260.KRX",
  "132030.KRX",
  "133690.KRX",
  "143850.KRX",
  "148070.KRX",
  "153130.KRX",
  "192090.KRX",
  "195930.KRX",
  "229200.KRX",
]);

let submitted = false;
let submittedPolls = 0;
const SECOND_CONFIG_ID = "00000000-0000-4000-8000-000000000102";
const DAILY_RUN_ID = "00000000-0000-4000-8000-000000000602";
const DAILY_PENDING_ID = "00000000-0000-4000-8000-000000000603";
let dailyEnabled = false;
let dailyEpoch = 0;

export function resetOwnerBetaDailyFixture(scenario) {
  dailyEnabled = scenario.dailyEnabled === "true";
  dailyEpoch = 0;
}

export function additionalDailyConfigs(scenario) {
  if (scenario.dailyConfigs !== "two") {
    return [];
  }
  return [
    {
      ...recommendationConfig(),
      id: SECOND_CONFIG_ID,
      owner_beta_daily_enabled: false,
      owner_beta_daily_epoch: 0,
    },
  ];
}

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
    owner_beta_daily_enabled: dailyEnabled,
    owner_beta_daily_epoch: dailyEpoch,
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

function ownerBetaRun() {
  return {
    action_manifest_sha256: OWNER_BETA_SHA256,
    approval_registry_sha256: OWNER_BETA_SHA256,
    artifact_manifest_sha256: OWNER_BETA_SHA256,
    as_of: "2026-08-19",
    audience: "OWNER_ONLY",
    candidate_content_sha256: OWNER_BETA_SHA256,
    capability: "PRICE_RETURN_ONLY",
    cash_weight: "0.200000",
    created_at: "2026-08-19T06:30:00Z",
    factor_snapshot_sha256: OWNER_BETA_SHA256,
    finished_at: "2026-08-19T06:30:02Z",
    id: OWNER_BETA_RUN_ID,
    input_kind: "owner_beta_historical_price_only_v1",
    items: OWNER_BETA_ETF_IDS.map((instrument_id, index) => ({
      excluded: index !== 0,
      exclusion_reason: index === 0 ? undefined : "NOT_SELECTED_BY_STRATEGY",
      factors: { momentum_12_1: index === 0 ? "0.912300" : "0.000000" },
      instrument: {
        asset_class: "ETF",
        exposure_group: null,
        id: instrument_id,
        name: index === 0 ? "KODEX 200" : null,
        tracking_index: null,
      },
      instrument_id,
      rank: index === 0 ? 1 : null,
      reason_codes: index === 0 ? ["SELECTED_TOP_N"] : ["NOT_SELECTED_BY_STRATEGY"],
      target_weight: index === 0 ? "0.800000" : null,
    })),
    job_id: OWNER_BETA_JOB_ID,
    stage5_manifest_sha256: OWNER_BETA_SHA256,
    started_at: "2026-08-19T06:30:01Z",
    status: "SUCCEEDED",
    strategy_config_id: CONFIG_ID,
    strategy_config_sha256: OWNER_BETA_SHA256,
    strategy_id: "relative_momentum",
    strategy_version: "1.0.0",
    strict_pit: false,
    target_snapshot_sha256: OWNER_BETA_SHA256,
    updated_at: "2026-08-19T06:30:02Z",
    vendor_snapshot: true,
  };
}

function dailyRun({ pending = false, failed = false } = {}) {
  const {
    action_manifest_sha256,
    approval_registry_sha256,
    artifact_manifest_sha256,
    candidate_content_sha256,
    stage5_manifest_sha256,
    ...base
  } = ownerBetaRun();
  const unfinished = pending || failed;
  return {
    ...base,
    id: unfinished ? DAILY_PENDING_ID : DAILY_RUN_ID,
    as_of: unfinished ? "2026-10-16" : "2026-10-15",
    created_at: unfinished ? "2026-10-16T08:00:00Z" : "2026-10-15T07:40:00Z",
    updated_at: unfinished ? "2026-10-16T08:00:00Z" : "2026-10-15T08:00:00Z",
    started_at: unfinished ? undefined : "2026-10-15T07:50:00Z",
    finished_at: unfinished ? undefined : "2026-10-15T08:00:00Z",
    published_at: unfinished ? undefined : "2026-10-15T08:00:00Z",
    trigger_kind: "DAILY",
    input_kind: "owner_beta_daily_etf11_price_only_v1",
    input: {
      kind: "daily",
      content_sha256: OWNER_BETA_SHA256,
      manifest_sha256: OWNER_BETA_SHA256,
      admission_sha256: OWNER_BETA_SHA256,
      policy_sha256: OWNER_BETA_SHA256,
      base: {
        action_manifest_sha256,
        approval_registry_sha256,
        artifact_manifest_sha256,
        candidate_content_sha256,
        stage5_manifest_sha256,
      },
    },
    status: failed ? "FAILED" : pending ? "PENDING" : "SUCCEEDED",
    error_code: failed ? "OWNER_BETA_COMPUTATION_FAILED" : undefined,
    items: unfinished ? [] : base.items,
    factor_snapshot_sha256: unfinished ? undefined : base.factor_snapshot_sha256,
    target_snapshot_sha256: unfinished ? undefined : base.target_snapshot_sha256,
    cash_weight: unfinished ? undefined : base.cash_weight,
  };
}

function dailyState(scenario, configId) {
  const secondary = configId === SECOND_CONFIG_ID;
  const mode = scenario.dailyState ?? "disabled";
  const current = mode === "current";
  const hasDailyResult =
    !secondary &&
    ["current", "blocked", "stale", "queued", "failed"].includes(mode) &&
    scenario.dailyHistory !== "manual";
  const enabled = !secondary && dailyEnabled;
  const runtime = scenario.dailyRuntime === "enabled";
  let state = !enabled || !runtime ? "DISABLED" : "INPUT_PENDING";
  let reason = !enabled ? "CONFIG_DISABLED" : !runtime ? "RUNTIME_DISABLED" : "INPUT_NOT_READY";
  if (enabled && runtime) {
    if (mode === "current" && hasDailyResult) {
      state = "CURRENT";
      reason = null;
    } else if (mode === "blocked") {
      state = "INPUT_BLOCKED";
      reason = "SOURCE_CONFLICT";
    } else if (mode === "stale") {
      state = "STALE";
      reason = "VERIFICATION_EXPIRED";
    } else if (mode === "queued") {
      state = "QUEUED";
      reason = "JOB_QUEUED";
    } else if (mode === "failed") {
      state = "FAILED";
      reason = "JOB_FAILED";
    }
  }
  return {
    enabled,
    runtime_enabled: runtime,
    expected_session: secondary ? null : current ? "2026-10-15" : "2026-10-16",
    data_support_end: hasDailyResult ? "2026-10-15" : null,
    recommendation_as_of: hasDailyResult ? "2026-10-15" : null,
    published_at: hasDailyResult ? "2026-10-15T08:00:00Z" : null,
    input_kind: hasDailyResult ? "owner_beta_daily_etf11_price_only_v1" : null,
    trigger_kind: hasDailyResult ? "DAILY" : null,
    current_reservation_run_id: secondary
      ? null
      : ["queued", "failed"].includes(mode)
        ? DAILY_PENDING_ID
        : hasDailyResult
          ? DAILY_RUN_ID
          : null,
    state,
    reason_code: reason,
    data_due_at: secondary ? null : current ? "2026-10-15T09:30:00Z" : "2026-10-16T09:30:00Z",
    observed_at: current ? "2026-10-15T08:01:00Z" : "2026-10-16T09:31:00Z",
  };
}

function listRun(run) {
  const { items: _items, ...item } = run;
  return item;
}

export function ownerBetaRecommendationResponse(request) {
  const { body, headers, method, pathname, scenario } = request;
  const runsPath = "/api/v1/recommendations/owner-beta/price-only/runs";
  const supportedAsOfPath = "/api/v1/recommendations/owner-beta/price-only/supported-as-of";
  const statePath = "/api/v1/recommendations/owner-beta/price-only/daily-state";
  const preferencePath = `/api/v1/strategy-configs/${CONFIG_ID}`;
  if (
    pathname !== supportedAsOfPath &&
    pathname !== statePath &&
    pathname !== preferencePath &&
    !pathname.startsWith(runsPath)
  ) {
    return null;
  }
  if (scenario.ownerBetaAccessMode !== "owner_only" || scenario.role !== "owner") {
    return error(403, "FORBIDDEN", "forbidden");
  }
  if (scenario.entitlement === "blocked") {
    return error(403, "DATA_ENTITLEMENT_REQUIRED", "recommendation entitlement is inactive");
  }
  if (method === "PATCH" && pathname === preferencePath) {
    if (!mutationAuthorized(headers) || scenario.dailyPatch === "denied") {
      return error(403, "CSRF_DENIED", "daily preference refused");
    }
    if (Object.keys(body).length !== 1 || typeof body.owner_beta_daily_enabled !== "boolean") {
      return error(422, "INVALID_PARAMETER", "daily preference only");
    }
    dailyEnabled = body.owner_beta_daily_enabled;
    dailyEpoch += 1;
    return { body: recommendationConfig(), status: 200 };
  }
  if (method === "GET" && pathname === statePath) {
    if (scenario.dailyState === "unavailable") {
      return error(503, "INTERNAL", "daily observation unavailable");
    }
    const id = new URLSearchParams(request.query ?? "").get("strategy_config_id");
    if (id !== CONFIG_ID && !(id === SECOND_CONFIG_ID && scenario.dailyConfigs === "two")) {
      return error(404, "RESOURCE_NOT_FOUND", "configuration not found");
    }
    return { body: dailyState(scenario, id), status: 200 };
  }
  if (method === "GET" && pathname === supportedAsOfPath) {
    if (scenario.dailyDiscovery === "unavailable") {
      return error(503, "OWNER_BETA_PRICE_INPUT_UNAVAILABLE", "historical discovery unavailable");
    }
    return {
      body: {
        default_as_of: "2026-08-19",
        supported_as_of: ["2026-08-15", "2026-08-18", "2026-08-19"],
      },
      status: 200,
    };
  }
  const hasDailyHistory = ["current", "blocked", "stale", "queued", "failed"].includes(
    scenario.dailyState,
  );
  const previous =
    hasDailyHistory && scenario.dailyHistory !== "manual" ? dailyRun() : ownerBetaRun();
  const queued = scenario.dailyState === "queued" || scenario.dailyState === "failed";
  const pending = dailyRun({ pending: true, failed: scenario.dailyState === "failed" });
  if (method === "GET" && pathname === runsPath) {
    const cursor = new URLSearchParams(request.query ?? "").get("cursor");
    if (scenario.dailyHistory === "paged" && queued && cursor === null) {
      return {
        body: { has_more: true, items: [listRun(pending)], next_cursor: "daily-page-2" },
        status: 200,
      };
    }
    return {
      body: {
        has_more: false,
        items:
          queued && cursor === null ? [listRun(pending), listRun(previous)] : [listRun(previous)],
        next_cursor: null,
      },
      status: 200,
    };
  }
  if (method === "GET" && pathname === `${runsPath}/${OWNER_BETA_RUN_ID}`) {
    return { body: ownerBetaRun(), status: 200 };
  }
  if (method === "GET" && pathname === `${runsPath}/${DAILY_RUN_ID}`) {
    return { body: dailyRun(), status: 200 };
  }
  if (method === "GET" && pathname === `${runsPath}/${DAILY_PENDING_ID}`) {
    return { body: pending, status: 200 };
  }
  return null;
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
    if (body.owner_beta_daily_enabled !== undefined) {
      if (
        scenario.ownerBetaAccessMode !== "owner_only" ||
        scenario.role !== "owner" ||
        typeof body.owner_beta_daily_enabled !== "boolean"
      ) {
        return error(403, "FORBIDDEN", "daily preference refused");
      }
      dailyEnabled = body.owner_beta_daily_enabled;
      dailyEpoch += 1;
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
