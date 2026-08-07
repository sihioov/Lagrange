// Lagrange Station API: OpenAPI 3.1 spec generator.
//
// The route table below mirrors `crates/api-server/src/contract.rs`
// (CONTRACT_ROUTES) 1:1 — the Rust integration test
// `openapi_contract_routes_match_router_inventory` asserts spec == inventory,
// so any drift fails `cargo test -p api-server`.
//
// `npm run openapi:check` (scripts/openapi-check.mjs) regenerates this spec,
// fails on drift against the committed `openapi.json`, lints the required
// per-operation metadata, and emits TypeScript types.

const PHASE3 = "phase3";

/** Mirror of api-server CONTRACT_ROUTES. [method, path, flags] */
const ROUTES = [
  // auth / session
  ["GET", "/api/v1/auth/session", {}],
  ["POST", "/api/v1/auth/logout", { mutating: true, natural: true, audit: true }],
  ["GET", "/api/v1/auth/csrf", { mutating: true, natural: true }],
  ["GET", "/api/v1/auth/step-up-check", { audit: true }],
  // strategies
  ["GET", "/api/v1/strategies", {}],
  ["GET", "/api/v1/strategies/{strategy_id}", {}],
  ["POST", "/api/v1/strategies/{strategy_id}/configs", { mutating: true, idem: true, audit: true }],
  ["GET", "/api/v1/strategy-configs/{config_id}", {}],
  // recommendations
  ["POST", "/api/v1/recommendations/runs", { mutating: true, idem: true, entitlement: "recommendation", audit: true }],
  ["GET", "/api/v1/recommendations/runs/{run_id}", { entitlement: "recommendation" }],
  ["GET", "/api/v1/recommendations/runs", { entitlement: "recommendation" }],
  ["GET", "/api/v1/recommendations/latest", { entitlement: "recommendation" }],
  // backtests
  ["POST", "/api/v1/backtests", { mutating: true, idem: true, entitlement: "backtest", audit: true }],
  ["GET", "/api/v1/backtests", { entitlement: "backtest" }],
  ["GET", "/api/v1/backtests/{run_id}", { entitlement: "backtest" }],
  ["POST", "/api/v1/backtests/{run_id}/cancel", { mutating: true, idem: true, entitlement: "backtest", audit: true }],
  ["GET", "/api/v1/backtests/{run_id}/metrics", { entitlement: "backtest" }],
  ["GET", "/api/v1/backtests/{run_id}/equity", { entitlement: "backtest" }],
  ["GET", "/api/v1/backtests/{run_id}/trades", { entitlement: "backtest" }],
  ["POST", "/api/v1/backtests/{run_id}/robustness", { mutating: true, idem: true, entitlement: "backtest", audit: true }],
  ["POST", "/api/v1/backtests/compare", { entitlement: "backtest" }],
  // paper
  ["POST", "/api/v1/paper/accounts", { mutating: true, idem: true, entitlement: "paper_view", audit: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}", { entitlement: "paper_view" }],
  ["POST", "/api/v1/paper/accounts/{account_id}/bind-strategy", { mutating: true, idem: true, entitlement: "paper_view", audit: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/orders", { entitlement: "paper_view" }],
  ["GET", "/api/v1/paper/accounts/{account_id}/positions", { entitlement: "paper_view" }],
  ["GET", "/api/v1/paper/accounts/{account_id}/equity", { entitlement: "paper_view" }],
  // admin (Owner-only, audited)
  ["GET", "/api/v1/admin/datasets", { owner: true, audit: true }],
  ["POST", "/api/v1/admin/datasets/{dataset_id}/approve", { mutating: true, idem: true, owner: true, audit: true }],
  ["POST", "/api/v1/admin/datasets/{dataset_id}/block", { mutating: true, idem: true, owner: true, audit: true }],
  ["GET", "/api/v1/admin/jobs", { owner: true, audit: true }],
  ["POST", "/api/v1/admin/jobs/{job_id}/retry", { mutating: true, idem: true, owner: true, audit: true }],
  ["GET", "/api/v1/admin/workers", { owner: true, audit: true }],
  ["GET", "/api/v1/admin/users", { owner: true, audit: true }],
  ["GET", "/api/v1/admin/audit-logs", { owner: true, audit: true }],
  // Live (Phase 3, Owner-only)
  ["POST", "/api/v1/admin/live/connections", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/nodes/{node_id}/start", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/nodes/{node_id}/stop", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/kill-switch/enable", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/kill-switch/disable", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["GET", "/api/v1/admin/live/reconciliation", { owner: true, audit: true, phase: PHASE3 }],
  // licensing / artifacts
  ["GET", "/api/v1/licensing-status", {}],
  ["GET", "/api/v1/artifacts/{artifact_id}", { entitlement: "download" }],
  ["GET", "/api/v1/artifacts/{artifact_id}/download", { entitlement: "download" }],
];

/** Stable error codes (mirror of contract::ERROR_CODES). */
const ERROR_CODES = [
  ["SESSION_UNKNOWN", 401], ["SESSION_EXPIRED", 401],
  ["FORBIDDEN", 403], ["DATA_ENTITLEMENT_REQUIRED", 403],
  ["OWNER_ONLY_DEVELOPMENT_PATH", 403], ["CSRF_DENIED", 403],
  ["STEP_UP_NOT_OWNER", 403], ["STEP_UP_MFA_REQUIRED", 403],
  ["STEP_UP_AUTH_TIME_ABSENT", 403], ["STEP_UP_AUTH_TIME_STALE", 403],
  ["RESOURCE_NOT_FOUND", 404],
  ["INVALID_PARAMETER", 400], ["INVALID_DATE", 400], ["INVALID_DECIMAL", 400],
  ["INVALID_CURSOR", 400], ["IDEMPOTENCY_KEY_REQUIRED", 400],
  ["IDEMPOTENCY_KEY_MISMATCH", 409], ["DUPLICATE_RESOURCE", 409],
  ["PAYLOAD_TOO_LARGE", 413],
  ["DATASET_BLOCKED", 422], ["DATA_STALE", 422],
  ["INVALID_STRATEGY_PARAMETER", 422], ["UNSUPPORTED_MARKET_CURRENCY", 422],
  ["BACKTEST_CAPACITY_EXCEEDED", 429], ["RESULT_INTEGRITY_FAILED", 422],
  ["LIVE_RECONCILIATION_REQUIRED", 409], ["RISK_LIMIT_EXCEEDED", 422],
  ["ORDER_STATE_UNKNOWN", 409],
  ["NOT_IMPLEMENTED", 501], ["INTERNAL", 500],
];

const ERROR_CODES_SET = new Set(ERROR_CODES.map(([c]) => c));
if (ERROR_CODES_SET.size !== ERROR_CODES.length) {
  throw new Error("duplicate error codes");
}

const ENVELOPE = { $ref: "#/components/schemas/ErrorEnvelope" };

function errorResponses() {
  const responses = {
    "400": { $ref: "#/components/responses/Error400" },
    "401": { $ref: "#/components/responses/Error401" },
    "403": { $ref: "#/components/responses/Error403" },
    "404": { $ref: "#/components/responses/Error404" },
    "409": { $ref: "#/components/responses/Error409" },
    "413": { $ref: "#/components/responses/Error413" },
    "422": { $ref: "#/components/responses/Error422" },
    "429": { $ref: "#/components/responses/Error429" },
    "500": { $ref: "#/components/responses/Error500" },
    "501": { $ref: "#/components/responses/Error501" },
  };
  return responses;
}

function param(name, inKind, schema, required = false) {
  const p = {
    name,
    in: inKind,
    schema,
    required,
  };
  return p;
}

function operation(route) {
  const [, path, flags] = route;
  const method = route[0].toLowerCase();
  const owner = flags.owner === true;
  const phase = flags.phase === PHASE3 ? "3" : "current";
  const entitlement = flags.entitlement || null;
  const mutating = flags.mutating === true;
  const idemRequired = flags.idem === true;
  const natural = flags.natural === true;
  const audit = flags.audit === true;

  const op = {
    operationId: `${method}_${path.replace(/[\/{}\-]/g, "_")}`,
    summary: `${method.toUpperCase()} ${path}`,
    tags: [path.split("/")[2] || "root"],
    parameters: [],
    responses: {
      ...errorResponses(),
    },
    "x-lagrange": {
      auth: { required: true, session: "opaque __Host-lagrange_session cookie" },
      ownership: {
        owner_only: owner,
        scope: owner ? "Owner role; all admin operations are audited" : "actor-scoped via RLS (foreign resources are indistinguishable from missing)",
      },
      entitlement: entitlement
        ? { use: entitlement, fail_closed: true, dataset: "krx_eod_bars" }
        : { use: null, fail_closed: true },
      idempotency: mutating
        ? natural
          ? { required: false, natural: true, note: "idempotent by nature; no key required" }
          : { required: idemRequired, header: "Idempotency-Key", replay: "same key + same body returns the cached result; mismatch is 409 IDEMPOTENCY_KEY_MISMATCH" }
        : { required: false, note: "read-only" },
      audit: audit ? { writer: "audit_writer (append-only)", fields: "actor/time/target/before-after/reason/correlation_id" } : { writer: null },
      cache: { policy: "no-store", reason: "authenticated per-user data is never shared" },
      errors: errorCodesFor(route),
      phase,
    },
  };

  for (const [n, k] of Object.entries(pathParams(path))) {
    op.parameters.push(param(n, "path", { type: "string" }, true));
  }
  if (path.includes("latest")) {
    op.parameters.push(param("strategy_config_id", "query", { type: "string", format: "uuid" }, false));
  }
  if (path.endsWith("/runs") && method === "get" || path === "/api/v1/backtests" && method === "get") {
    op.parameters.push(param("cursor", "query", { type: "string" }, false));
    op.parameters.push(param("limit", "query", { type: "integer", minimum: 1, maximum: 100 }, false));
  }

  if (mutating) {
    op.requestBody = {
      required: true,
      content: { "application/json": { schema: { $ref: bodySchemaRef(path) } } },
    };
  }
  return op;
}

function pathParams(path) {
  const out = {};
  for (const m of path.matchAll(/\{(\w+)\}/g)) {
    out[m[1]] = true;
  }
  return out;
}

function bodySchemaRef(path) {
  if (path.endsWith("/configs")) return "#/components/schemas/NewStrategyConfigBody";
  if (path.endsWith("/recommendations/runs")) return "#/components/schemas/RecommendationRunBody";
  if (path === "/api/v1/backtests") return "#/components/schemas/BacktestBody";
  if (path === "/api/v1/backtests/compare") return "#/components/schemas/CompareBody";
  if (path === "/api/v1/paper/accounts") return "#/components/schemas/NewAccountBody";
  if (path.endsWith("/bind-strategy")) return "#/components/schemas/BindStrategyBody";
  return "#/components/schemas/EmptyBody";
}

function errorCodesFor(route) {
  const path = route[1];
  const flags = route[2];
  const codes = ["SESSION_UNKNOWN", "SESSION_EXPIRED", "INTERNAL"];
  if (flags.entitlement) codes.push("DATA_ENTITLEMENT_REQUIRED", "INVALID_DATE");
  if (flags.owner) codes.push("FORBIDDEN");
  if (flags.idem) codes.push("IDEMPOTENCY_KEY_REQUIRED", "IDEMPOTENCY_KEY_MISMATCH");
  if (flags.mutating) codes.push("CSRF_DENIED", "INVALID_PARAMETER", "PAYLOAD_TOO_LARGE");
  if (flags.phase === PHASE3) codes.push("NOT_IMPLEMENTED", "FORBIDDEN");
  if (path.includes("/backtests")) {
    codes.push("DATASET_BLOCKED", "DATA_STALE", "BACKTEST_CAPACITY_EXCEEDED", "RESULT_INTEGRITY_FAILED", "UNSUPPORTED_MARKET_CURRENCY", "INVALID_DECIMAL", "DUPLICATE_RESOURCE");
  }
  if (path.includes("/paper/accounts")) {
    codes.push("UNSUPPORTED_MARKET_CURRENCY", "DUPLICATE_RESOURCE");
  }
  return [...new Set(codes)];
}

const dateStr = { type: "string", pattern: "^\\d{4}-\\d{2}-\\d{2}$", example: "2026-01-31" };
const decimalStr = { type: "string", pattern: "^-?\\d+(\\.\\d+)?$", example: "100000000" };
const ts = { type: "string", format: "date-time" };
const uuid = { type: "string", format: "uuid" };

const SCHEMAS = {
  Error: {
    type: "object",
    required: ["code", "message", "request_id"],
    properties: {
      code: { $ref: "#/components/schemas/ErrorCode" },
      message: { type: "string" },
      request_id: { type: "string", description: "X-Request-Id echoed into the envelope" },
      details: { type: "object", additionalProperties: true },
    },
  },
  ErrorEnvelope: {
    type: "object",
    required: ["error"],
    properties: { error: { $ref: "#/components/schemas/Error" } },
  },
  ErrorCode: {
    type: "string",
    enum: ERROR_CODES.map(([c]) => c),
  },
  Page: {
    type: "object",
    required: ["items", "next_cursor", "has_more"],
    properties: {
      items: { type: "array", items: { type: "object" } },
      next_cursor: { type: ["string", "null"], description: "opaque signed cursor; null when the last page" },
      has_more: { type: "boolean" },
    },
  },
  Strategy: {
    type: "object",
    required: ["id", "display_name", "state"],
    properties: {
      id: { type: "string" },
      display_name: { type: "string" },
      description: { type: "string" },
      risk_description: { type: "string" },
      state: { type: "string", enum: ["Draft", "Validated", "Paper", "LiveCandidate", "Retired"] },
      latest_version: { type: ["string", "null"] },
    },
  },
  StrategyConfig: {
    type: "object",
    required: ["id", "strategy_id", "strategy_version", "config", "is_active"],
    properties: {
      id: { type: "string", format: "uuid" },
      strategy_id: { type: "string" },
      strategy_version: { type: "string" },
      config: { type: "object", additionalProperties: true, description: "schema-bound parameters only; code is never accepted" },
      is_active: { type: "boolean" },
      created_at: ts,
      updated_at: ts,
    },
  },
  NewStrategyConfigBody: {
    type: "object",
    required: ["strategy_version", "config"],
    additionalProperties: false,
    properties: {
      strategy_version: { type: "string" },
      config: { type: "object", additionalProperties: true },
      is_active: { type: "boolean", default: true },
    },
  },
  RecommendationItem: {
    type: "object",
    required: ["instrument_id", "excluded"],
    properties: {
      instrument_id: { type: "string", example: "069500.KRX" },
      rank: { type: ["integer", "null"] },
      target_weight: { type: ["string", "null"], description: "decimal string (scale 6)" },
      excluded: { type: "boolean" },
      exclusion_reason: { type: ["string", "null"] },
      reason_codes: { type: "array", items: { type: "string" } },
      factors: { type: "object", additionalProperties: true },
    },
  },
  RecommendationRun: {
    type: "object",
    required: ["id", "as_of", "status"],
    properties: {
      id: uuid,
      strategy_config_id: { type: ["string", "null"], format: "uuid" },
      as_of: dateStr,
      status: { type: "string", enum: ["PENDING", "SUCCEEDED", "FAILED", "BLOCKED"] },
      summary: { type: "object", additionalProperties: true },
      created_at: ts,
      job_id: { type: ["string", "null"], format: "uuid" },
      items: { type: "array", items: { $ref: "#/components/schemas/RecommendationItem" } },
    },
  },
  RecommendationRunBody: {
    type: "object",
    required: ["strategy_config_id", "as_of"],
    additionalProperties: false,
    properties: {
      strategy_config_id: uuid,
      as_of: dateStr,
    },
  },
  BacktestRun: {
    type: "object",
    required: ["id", "strategy_id", "strategy_version", "status"],
    properties: {
      id: uuid,
      strategy_id: { type: "string" },
      strategy_version: { type: "string" },
      dataset_version: { type: "string" },
      engine: { type: "string" },
      engine_version: { type: "string" },
      status: { type: "string", enum: ["PENDING", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"] },
      job_id: { type: ["string", "null"], format: "uuid" },
      config_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
      benchmark: { type: ["string", "null"] },
      start_date: { type: ["string", "null"], pattern: "^\\d{4}-\\d{2}-\\d{2}$" },
      end_date: { type: ["string", "null"], pattern: "^\\d{4}-\\d{2}-\\d{2}$" },
      started_at: { type: ["string", "null"], format: "date-time" },
      finished_at: { type: ["string", "null"], format: "date-time" },
      created_at: ts,
      summary: { type: "object", additionalProperties: true },
    },
  },
  BacktestBody: {
    type: "object",
    required: ["strategy_config_id", "dataset_version_id", "start_date", "end_date", "initial_cash", "benchmark", "cost_profile_id", "execution_profile"],
    additionalProperties: false,
    properties: {
      strategy_config_id: uuid,
      dataset_version_id: uuid,
      start_date: dateStr,
      end_date: dateStr,
      initial_cash: {
        type: "object",
        required: ["currency", "amount"],
        additionalProperties: false,
        properties: {
          currency: { type: "string", enum: ["KRW"], description: "KRW first; unsupported currencies are 422 UNSUPPORTED_MARKET_CURRENCY" },
          amount: { type: "string", pattern: "^-?\\d+(\\.\\d+)?$", description: "fixed-point decimal string" },
        },
      },
      benchmark: { type: "string", example: "069500.KRX", description: "member of the fixed Korean ETF v1 universe" },
      cost_profile_id: { type: "string" },
      execution_profile: { type: "string", example: "daily-close-next-open@1" },
      robustness: { type: "boolean", default: false },
    },
  },
  Metric: {
    type: "object",
    required: ["metric_key", "metric_value"],
    properties: {
      metric_key: { type: "string" },
      metric_value: { type: "string", description: "decimal string" },
    },
  },
  Artifact: {
    type: "object",
    required: ["id", "run_id", "artifact_type", "row_count", "sha256", "size_bytes", "download_path"],
    properties: {
      id: uuid,
      run_id: uuid,
      artifact_type: { type: "string", enum: ["EQUITY_CURVE", "DRAWDOWN_CURVE", "MONTHLY_RETURNS", "ORDERS", "FILLS", "POSITIONS", "CASH_LEDGER", "FEES", "BENCHMARK"] },
      row_count: { type: "integer" },
      sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
      size_bytes: { type: "integer" },
      summary: { type: "object", additionalProperties: true },
      download_path: { type: "string", description: "versioned API path; never a filesystem URL" },
    },
  },
  Equity: {
    type: "object",
    required: ["run_id", "artifact", "summary"],
    properties: {
      run_id: uuid,
      artifact: { $ref: "#/components/schemas/Artifact" },
      summary: { type: "object", additionalProperties: true },
    },
  },
  Trade: {
    type: "object",
    required: ["run_id", "artifact_type", "artifact"],
    properties: {
      run_id: uuid,
      artifact_type: { type: "string" },
      artifact: { $ref: "#/components/schemas/Artifact" },
    },
  },
  Compare: {
    type: "object",
    required: ["run_ids", "runs", "deltas"],
    properties: {
      run_ids: { type: "array", items: uuid },
      runs: { type: "array", items: { type: "object", properties: { run_id: uuid, strategy_id: { type: "string" }, status: { type: "string" }, summary: { type: "object" } } } },
      deltas: { type: "object", additionalProperties: { type: "string", description: "decimal string difference" } },
    },
  },
  CompareBody: {
    type: "object",
    required: ["run_ids"],
    additionalProperties: false,
    properties: { run_ids: { type: "array", minItems: 2, items: uuid } },
  },
  Cancel: {
    type: "object",
    required: ["run_id", "status"],
    properties: {
      run_id: uuid,
      job_id: { type: ["string", "null"], format: "uuid" },
      status: { type: "string", enum: ["CANCEL_REQUESTED"] },
    },
  },
  Robustness: {
    type: "object",
    required: ["run_id", "job_id", "status"],
    properties: {
      run_id: uuid,
      job_id: uuid,
      status: { type: "string", enum: ["QUEUED"] },
    },
  },
  Account: {
    type: "object",
    required: ["id", "account_type", "name", "currency", "status"],
    properties: {
      id: uuid,
      account_type: { type: "string", enum: ["PAPER"], description: "LIVE accounts are Phase 3 Owner-only and never creatable via this route" },
      name: { type: "string" },
      currency: { type: "string", enum: ["KRW"] },
      status: { type: "string", enum: ["ACTIVE", "SUSPENDED", "CLOSED"] },
      created_at: ts,
      updated_at: ts,
    },
  },
  NewAccountBody: {
    type: "object",
    required: ["name", "currency"],
    additionalProperties: false,
    properties: {
      name: { type: "string", minLength: 1 },
      currency: { type: "string", enum: ["KRW"] },
    },
  },
  BindStrategy: {
    type: "object",
    required: ["account_id", "strategy_config_id", "bound_at"],
    properties: {
      account_id: uuid,
      strategy_config_id: uuid,
      bound_at: ts,
    },
  },
  BindStrategyBody: {
    type: "object",
    required: ["strategy_config_id"],
    additionalProperties: false,
    properties: { strategy_config_id: uuid },
  },
  Order: {
    type: "object",
    required: ["id", "order_ref", "instrument_id", "side", "quantity", "status"],
    properties: {
      id: uuid,
      order_ref: { type: "string" },
      instrument_id: { type: "string" },
      side: { type: "string", enum: ["BUY", "SELL"] },
      quantity: { type: "string", description: "decimal string (scale 4)" },
      price: { type: ["string", "null"] },
      status: { type: "string" },
      submitted_at: { type: ["string", "null"], format: "date-time" },
      created_at: ts,
    },
  },
  Position: {
    type: "object",
    required: ["instrument_id", "quantity"],
    properties: {
      instrument_id: { type: "string" },
      quantity: { type: "string" },
      avg_price: { type: ["string", "null"] },
      updated_at: ts,
    },
  },
  EquityPoint: {
    type: "object",
    required: ["trading_date", "equity", "cash", "positions_value", "currency"],
    properties: {
      trading_date: dateStr,
      equity: { type: "string" },
      cash: { type: "string" },
      positions_value: { type: "string" },
      currency: { type: "string" },
    },
  },
  AdminDataset: {
    type: "object",
    required: ["id", "dataset_id", "version", "status"],
    properties: {
      id: uuid,
      dataset_id: { type: "string" },
      version: { type: "string" },
      status: { type: "string", enum: ["READY", "WARNING", "BLOCKED"] },
      manifest_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
      created_at: ts,
      blocking_issues: {
        type: "array",
        items: { type: "object", properties: { issue_code: { type: "string" }, severity: { type: "string" }, detail: { type: "object" } } },
      },
    },
  },
  DatasetVerdict: {
    type: "object",
    required: ["dataset_id", "version", "status", "verdict", "reason"],
    properties: {
      dataset_id: { type: "string" },
      version: { type: "string" },
      status: { type: "string" },
      verdict: { type: "string" },
      reason: { type: "string" },
    },
  },
  Job: {
    type: "object",
    required: ["id", "job_type", "status"],
    properties: {
      id: uuid,
      job_type: { type: "string" },
      status: { type: "string", enum: ["QUEUED", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"] },
      priority: { type: "integer" },
      idempotency_key: { type: ["string", "null"] },
      attempt_count: { type: "integer" },
      created_at: ts,
      started_at: { type: ["string", "null"], format: "date-time" },
      finished_at: { type: ["string", "null"], format: "date-time" },
      error_code: { type: ["string", "null"] },
      error_message: { type: ["string", "null"] },
    },
  },
  Worker: {
    type: "object",
    required: ["worker_id", "active_job_count"],
    properties: {
      worker_id: { type: "string" },
      last_heartbeat_at: { type: ["string", "null"], format: "date-time" },
      active_job_count: { type: "integer" },
    },
  },
  AuditEntry: {
    type: "object",
    required: ["id", "action", "actor_role", "created_at"],
    properties: {
      id: uuid,
      action: { type: "string" },
      actor_role: { type: "string" },
      actor_user_id: { type: ["string", "null"], format: "uuid" },
      target_type: { type: ["string", "null"] },
      target_id: { type: ["string", "null"] },
      reason: { type: ["string", "null"] },
      correlation_id: { type: ["string", "null"] },
      created_at: ts,
    },
  },
  LicensingStatus: {
    type: "object",
    required: ["as_of", "datasets"],
    properties: {
      as_of: dateStr,
      datasets: {
        type: "array",
        items: {
          type: "object",
          required: ["dataset_id", "use_kind", "state", "covered"],
          properties: {
            dataset_id: { type: "string" },
            use_kind: { type: "string" },
            state: { type: "string", enum: ["PENDING", "ACTIVE", "EXPIRED", "REVOKED"] },
            effective_from: { type: ["string", "null"], pattern: "^\\d{4}-\\d{2}-\\d{2}$" },
            effective_until: { type: ["string", "null"], pattern: "^\\d{4}-\\d{2}-\\d{2}$" },
            covered: { type: "boolean" },
          },
        },
      },
    },
  },
  Session: {
    type: "object",
    required: ["user_id", "role", "expires_at_secs"],
    properties: {
      user_id: { type: "string", format: "uuid" },
      role: { type: "string", enum: ["owner", "member"] },
      expires_at_secs: { type: "integer" },
      auth_time_secs: { type: "integer" },
    },
  },
  EmptyBody: { type: "object", additionalProperties: false },
};

function build() {
  const paths = {};
  for (const route of ROUTES) {
    const [method, path] = route;
    paths[path] = paths[path] || {};
    paths[path][method.toLowerCase()] = operation(route);
  }

  const responses = {};
  for (const code of ["400", "401", "403", "404", "409", "413", "422", "429", "500", "501"]) {
    responses[`Error${code}`] = {
      description: `${code} typed error envelope`,
      content: {
        "application/json": { schema: ENVELOPE },
      },
    };
  }

  return {
    openapi: "3.1.0",
    info: {
      title: "Lagrange Station API",
      version: "1.0.0",
      description:
        "Versioned REST/OpenAPI contract for Lagrange Station. Common rules (design §12.1): `/api/v1` prefix, JSON bodies, cursor pagination, `X-Request-Id` correlation in/out, the typed `{error:{code,message,request_id,details?}}` envelope, and Idempotency-Key on every mutating route. No database/NT/provider models leak into payloads.",
    },
    servers: [{ url: "/" }],
    tags: [
      { name: "auth", description: "Session, CSRF, and Owner step-up" },
      { name: "strategies", description: "Strategy catalog and schema-bound user configs" },
      { name: "recommendations", description: "Recommendation runs (entitlement-gated)" },
      { name: "backtests", description: "Backtest orchestration and results" },
      { name: "paper", description: "Paper accounts and ledger views" },
      { name: "admin", description: "Owner-only operational surface (audited)" },
      { name: "licensing", description: "KR data entitlement status" },
      { name: "artifacts", description: "Authorized result artifacts" },
    ],
    paths,
    components: {
      schemas: SCHEMAS,
      responses,
      securitySchemes: {
        sessionCookie: {
          type: "apiKey",
          in: "cookie",
          name: "__Host-lagrange_session",
          description: "Opaque first-party session cookie issued by the auth router",
        },
      },
    },
    security: [{ sessionCookie: [] }],
  };
}

export { ROUTES, ERROR_CODES, build };
