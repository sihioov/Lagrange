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
  ["GET", "/api/v1/strategy-configs", {}],
  ["GET", "/api/v1/strategy-configs/{config_id}", {}],
  // recommendations
  ["POST", "/api/v1/recommendations/runs", { mutating: true, idem: true, entitlement: "recommendation", audit: true }],
  ["GET", "/api/v1/recommendations/runs/{run_id}", { entitlement: "recommendation" }],
  ["GET", "/api/v1/recommendations/runs", { entitlement: "recommendation" }],
  ["GET", "/api/v1/recommendations/latest", { entitlement: "recommendation" }],
  // common individual-stock research (separate from ETF recommendations)
  ["GET", "/api/v1/candidates/feed/latest", { entitlement: "candidate" }],
  ["GET", "/api/v1/candidates/feed/{date}", { entitlement: "candidate" }],
  ["GET", "/api/v1/stocks/{instrument_id}/analysis", { entitlement: "candidate" }],
  ["POST", "/api/v1/screener/query", { body: true, entitlement: "candidate" }],
  ["GET", "/api/v1/screener/screens", {}],
  ["POST", "/api/v1/screener/screens", { mutating: true, idem: true, audit: true }],
  ["GET", "/api/v1/screener/screens/{id}", {}],
  ["PUT", "/api/v1/screener/screens/{id}", { mutating: true, idem: true, audit: true }],
  ["DELETE", "/api/v1/screener/screens/{id}", { mutating: true, noBody: true, idem: true, audit: true }],
  // backtests
  ["POST", "/api/v1/backtests", { mutating: true, idem: true, entitlement: "backtest", audit: true }],
  ["GET", "/api/v1/backtests", { entitlement: "backtest", shared: true }],
  ["GET", "/api/v1/backtests/{run_id}", { entitlement: "backtest", shared: true }],
  ["POST", "/api/v1/backtests/{run_id}/cancel", { mutating: true, idem: true, entitlement: "backtest", audit: true }],
  ["GET", "/api/v1/backtests/{run_id}/metrics", { entitlement: "backtest", shared: true }],
  ["GET", "/api/v1/backtests/{run_id}/equity", { entitlement: "backtest", shared: true }],
  ["GET", "/api/v1/backtests/{run_id}/trades", { entitlement: "backtest", shared: true }],
  ["POST", "/api/v1/backtests/{run_id}/robustness", { mutating: true, idem: true, entitlement: "backtest", audit: true }],
  ["POST", "/api/v1/backtests/compare", { entitlement: "backtest", shared: true }],
  // paper
  ["GET", "/api/v1/paper/accounts", { entitlement: "paper_view", shared: true }],
  ["POST", "/api/v1/paper/accounts", { mutating: true, idem: true, entitlement: "paper_view", audit: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}", { entitlement: "paper_view", shared: true }],
  ["POST", "/api/v1/paper/accounts/{account_id}/bind-strategy", { mutating: true, idem: true, entitlement: "paper_view", audit: true }],
  ["POST", "/api/v1/paper/accounts/{account_id}/recommendation-previews", { mutating: true, idem: true, owner: true, entitlement: "recommendation", audit: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}", { owner: true, entitlement: "paper_view" }],
  ["POST", "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply", { mutating: true, idem: true, owner: true, entitlement: "recommendation", audit: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/orders", { entitlement: "paper_view", shared: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/positions", { entitlement: "paper_view", shared: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/equity", { entitlement: "paper_view", shared: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/performance", { entitlement: "paper_view", shared: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/lineage", { entitlement: "paper_view", shared: true }],
  ["GET", "/api/v1/paper/accounts/{account_id}/parity", { entitlement: "paper_view", shared: true }],
  // admin (Owner-only, audited)
  ["GET", "/api/v1/admin/datasets", { owner: true, audit: true }],
  ["POST", "/api/v1/admin/datasets/{dataset_id}/approve", { mutating: true, idem: true, owner: true, audit: true }],
  ["POST", "/api/v1/admin/datasets/{dataset_id}/block", { mutating: true, idem: true, owner: true, audit: true }],
  ["GET", "/api/v1/admin/jobs", { owner: true, audit: true }],
  ["POST", "/api/v1/admin/jobs/{job_id}/retry", { mutating: true, idem: true, owner: true, audit: true }],
  ["GET", "/api/v1/admin/workers", { owner: true, audit: true }],
  ["GET", "/api/v1/admin/users", { owner: true, audit: true }],
  ["GET", "/api/v1/admin/audit-logs", { owner: true, audit: true }],
  ["GET", "/api/v1/admin/notifications/deliveries", { owner: true, audit: true }],
  // notifications
  ["GET", "/api/v1/notifications", {}],
  ["GET", "/api/v1/notifications/subscriptions", {}],
  ["PUT", "/api/v1/notifications/subscriptions", { mutating: true, idem: true, audit: true }],
  ["POST", "/api/v1/notifications/test", { mutating: true, idem: true, audit: true }],
  // Live (Phase 3, Owner-only)
  ["GET", "/api/v1/admin/live/connections", { owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/connections", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  // The Idempotency-Key is not bookkeeping on this route: it is the identity a
  // retransmission repeats, and the only thing stopping a timed-out retry from
  // placing a second real order (FR-LIVE-003).
  ["POST", "/api/v1/admin/live/orders", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/connections/{connection_id}/start", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/nodes/{node_id}/stop", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/kill-switch/enable", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["POST", "/api/v1/admin/live/kill-switch/disable", { mutating: true, idem: true, owner: true, audit: true, phase: PHASE3 }],
  ["GET", "/api/v1/admin/live/reconciliation", { owner: true, audit: true, phase: PHASE3 }],
  // licensing / artifacts / metrics
  ["GET", "/api/v1/licensing-status", {}],
  ["GET", "/api/v1/metrics", {}],
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
  ["BACKTEST_CAPACITY_EXCEEDED", 429], ["ROBUSTNESS_CAPACITY_EXCEEDED", 429],
  ["RECOMMENDATION_CAPACITY_EXCEEDED", 429],
  ["REBALANCE_PREVIEW_CAPACITY_EXCEEDED", 429],
  ["REBALANCE_PREVIEW_BINDING_REQUIRED", 409],
  ["REBALANCE_PREVIEW_NOT_READY", 409],
  ["REBALANCE_PREVIEW_DATA_BLOCKED", 422],
  ["REBALANCE_PREVIEW_ENTITLEMENT_REQUIRED", 403],
  ["REBALANCE_PREVIEW_STALE", 409],
  ["REBALANCE_PREVIEW_FAILED", 422],
  ["REBALANCE_PREVIEW_CONFLICT", 409],
  ["RESULT_INTEGRITY_FAILED", 422],
  ["LIVE_RECONCILIATION_REQUIRED", 409], ["LIVE_KILL_SWITCH_ENGAGED", 409],
  ["LIVE_CONNECTION_NOT_CONFIGURED", 409],
  ["RISK_LIMIT_EXCEEDED", 422],
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
  const shared = flags.shared === true;

  const op = {
    operationId: `${method}_${path.replace(/[\/{}\-]/g, "_")}`,
    summary: `${method.toUpperCase()} ${path}`,
    tags: [path.split("/")[2] || "root"],
    parameters: [],
    responses: {
      ...successResponsesFor(method, path),
      ...errorResponses(),
    },
    "x-lagrange": {
      auth: { required: true, session: "opaque __Host-lagrange_session cookie" },
      ownership: {
        owner_only: owner,
        scope: owner
          ? "Owner role; all admin operations are audited"
          : shared
            ? "authenticated invite-group read via SELECT-only admin role; mutations remain actor-scoped"
            : "actor-scoped via RLS (foreign resources are indistinguishable from missing)",
      },
      entitlement: entitlement
        ? { use: entitlement, fail_closed: true, dataset: entitlement === "candidate" ? "every exact pinned candidate source dataset" : "krx_eod_bars" }
        : { use: null, fail_closed: true },
      idempotency: mutating
        ? natural
          ? { required: false, natural: true, note: "idempotent by nature; no key required" }
          : { required: idemRequired, header: "Idempotency-Key", replay: "same key + same body returns the cached result; mismatch is 409 IDEMPOTENCY_KEY_MISMATCH" }
        : { required: false, note: "read-only" },
      audit: audit ? { writer: "audit_writer (append-only)", fields: "actor/time/target/before-after/reason/correlation_id" } : { writer: null },
      cache: {
        policy: "no-store",
        reason: shared
          ? "authenticated invite-group data is never stored in a shared cache"
          : "authenticated per-user data is never shared",
      },
      errors: errorCodesFor(route),
      phase,
    },
  };

  for (const [n, k] of Object.entries(pathParams(path))) {
    op.parameters.push(param(n, "path", { type: "string" }, true));
  }
  if (path === "/api/v1/recommendations/latest") {
    op.parameters.push(param("strategy_config_id", "query", { type: "string", format: "uuid" }, false));
  }
  if (path === "/api/v1/stocks/{instrument_id}/analysis") {
    op.parameters.push(param("date", "query", dateStr, false));
    op.parameters.push(param("universe", "query", { $ref: "#/components/schemas/UniverseKey" }, false));
  }
  if (path === "/api/v1/candidates/feed/latest" || path === "/api/v1/candidates/feed/{date}") {
    op.parameters.push(param("universe", "query", { $ref: "#/components/schemas/UniverseKey" }, false));
  }
  if (path.endsWith("/runs") && method === "get" || path === "/api/v1/backtests" && method === "get") {
    op.parameters.push(param("cursor", "query", { type: "string" }, false));
    op.parameters.push(param("limit", "query", { type: "integer", minimum: 1, maximum: 100 }, false));
  }

  if ((mutating && flags.noBody !== true) || flags.body === true) {
    op.requestBody = {
      required: true,
      content: { "application/json": { schema: { $ref: bodySchemaRef(path) } } },
    };
  }
  return op;
}

function successResponsesFor(method, path) {
  const json = (description, schema) => ({
    description,
    content: { "application/json": { schema: { $ref: schema } } },
  });
  if (path === "/api/v1/recommendations/runs" && method === "post") {
    return { "201": json("Recommendation run accepted", "#/components/schemas/RecommendationRun") };
  }
  if (path === "/api/v1/recommendations/runs" && method === "get") {
    return { "200": json("Recommendation run history", "#/components/schemas/RecommendationRunPage") };
  }
  if (path === "/api/v1/recommendations/runs/{run_id}" && method === "get") {
    return { "200": json("Recommendation run", "#/components/schemas/RecommendationRun") };
  }
  if (path === "/api/v1/recommendations/latest" && method === "get") {
    return { "200": json("Latest recommendation snapshot", "#/components/schemas/RecommendationLatest") };
  }
  if (path === "/api/v1/candidates/feed/latest" && method === "get" || path === "/api/v1/candidates/feed/{date}" && method === "get") {
    return { "200": json("Immutable daily stock-research Top-5 feed", "#/components/schemas/CandidateFeed") };
  }
  if (path === "/api/v1/stocks/{instrument_id}/analysis" && method === "get") {
    return { "200": json("Point-in-time deep stock analysis", "#/components/schemas/StockAnalysisResponse") };
  }
  if (path === "/api/v1/screener/query" && method === "post") {
    return { "200": json("Point-in-time candidate screen result", "#/components/schemas/ScreenerResult") };
  }
  if (path === "/api/v1/screener/screens" && method === "get") {
    return { "200": json("Actor-owned saved screens", "#/components/schemas/SavedScreenList") };
  }
  if (path === "/api/v1/screener/screens" && method === "post") {
    return { "201": json("Saved screen created", "#/components/schemas/SavedScreen") };
  }
  if (path === "/api/v1/screener/screens/{id}" && method === "get" || path === "/api/v1/screener/screens/{id}" && method === "put") {
    return { "200": json("Actor-owned saved screen", "#/components/schemas/SavedScreen") };
  }
  if (path === "/api/v1/screener/screens/{id}" && method === "delete") {
    return { "200": json("Saved screen deleted", "#/components/schemas/DeleteSavedScreenResult") };
  }
  if (path === "/api/v1/paper/accounts/{account_id}/recommendation-previews" && method === "post") {
    return {
      "200": json("Existing rebalance preview replayed", "#/components/schemas/RebalancePreview"),
      "202": json("Rebalance preview accepted", "#/components/schemas/RebalancePreview"),
    };
  }
  if (path === "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}" && method === "get") {
    return { "200": json("Paper rebalance preview", "#/components/schemas/RebalancePreview") };
  }
  if (path === "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply" && method === "post") {
    return { "200": json("Rebalance preview queued for Paper execution", "#/components/schemas/AppliedRebalancePreview") };
  }
  return {};
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
  if (path === "/api/v1/screener/query") return "#/components/schemas/ScreenerQueryBody";
  if (path === "/api/v1/screener/screens" || path === "/api/v1/screener/screens/{id}") return "#/components/schemas/SavedScreenBody";
  if (path.endsWith("/robustness")) return "#/components/schemas/RobustnessSuiteBody";
  if (path === "/api/v1/paper/accounts") return "#/components/schemas/NewAccountBody";
  if (path.endsWith("/bind-strategy")) return "#/components/schemas/BindStrategyBody";
  if (path.endsWith("/recommendation-previews")) return "#/components/schemas/RebalancePreviewBody";
  if (path.endsWith("/recommendation-previews/{preview_id}/apply")) return "#/components/schemas/ApplyRebalancePreviewBody";
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
  if (path === "/api/v1/screener/query") codes.push("INVALID_PARAMETER", "INVALID_DATE", "INVALID_CURSOR");
  if (path === "/api/v1/candidates/feed/latest" || path === "/api/v1/candidates/feed/{date}" || path === "/api/v1/stocks/{instrument_id}/analysis") {
    codes.push("INVALID_PARAMETER");
  }
  if (flags.phase === PHASE3) codes.push("NOT_IMPLEMENTED", "FORBIDDEN");
  if (path.includes("/backtests")) {
    codes.push("DATASET_BLOCKED", "DATA_STALE", "BACKTEST_CAPACITY_EXCEEDED", "RESULT_INTEGRITY_FAILED", "UNSUPPORTED_MARKET_CURRENCY", "INVALID_DECIMAL", "DUPLICATE_RESOURCE");
  }
  if (path === "/api/v1/recommendations/runs" && route[0] === "POST") {
    codes.push("RECOMMENDATION_CAPACITY_EXCEEDED");
  }
  if (path.includes("/paper/accounts")) {
    codes.push("UNSUPPORTED_MARKET_CURRENCY", "DUPLICATE_RESOURCE");
  }
  if (path.includes("/recommendation-previews")) {
    codes.push(
      "REBALANCE_PREVIEW_BINDING_REQUIRED",
      "REBALANCE_PREVIEW_NOT_READY",
      "REBALANCE_PREVIEW_DATA_BLOCKED",
      "REBALANCE_PREVIEW_ENTITLEMENT_REQUIRED",
      "REBALANCE_PREVIEW_STALE",
      "REBALANCE_PREVIEW_FAILED",
      "REBALANCE_PREVIEW_CONFLICT",
      "RESULT_INTEGRITY_FAILED",
    );
  }
  if (path.endsWith("/recommendation-previews")) {
    codes.push("REBALANCE_PREVIEW_CAPACITY_EXCEEDED");
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
    required: ["id", "strategy_config_id", "as_of", "status", "summary", "created_at", "trigger_kind", "provenance"],
    properties: {
      id: uuid,
      strategy_config_id: { type: ["string", "null"], format: "uuid" },
      as_of: dateStr,
      status: { type: "string", enum: ["PENDING", "SUCCEEDED", "FAILED", "BLOCKED"] },
      summary: { type: "object", additionalProperties: true },
      created_at: ts,
      trigger_kind: { type: "string", enum: ["MANUAL", "SCHEDULED"] },
      provenance: { $ref: "#/components/schemas/RecommendationProvenance" },
      job_id: { type: ["string", "null"], format: "uuid" },
      items: { type: "array", items: { $ref: "#/components/schemas/RecommendationItem" } },
    },
  },
  RecommendationProvenance: {
    type: "object",
    properties: {
      dataset_version_id: { type: "string", format: "uuid" },
      dataset_manifest_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  },
  RecommendationRunPage: {
    type: "object",
    required: ["items", "next_cursor", "has_more"],
    properties: {
      items: { type: "array", items: { $ref: "#/components/schemas/RecommendationRun" } },
      next_cursor: { type: ["string", "null"] },
      has_more: { type: "boolean" },
    },
  },
  RecommendationLatest: {
    type: "object",
    required: ["run", "latest_run"],
    properties: {
      run: { oneOf: [{ $ref: "#/components/schemas/RecommendationRun" }, { type: "null" }] },
      latest_run: { $ref: "#/components/schemas/RecommendationRun" },
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
  CandidateDatasetPins: {
    type: "object",
    required: ["universe_snapshot_id", "price", "market_status", "flow", "fundamental", "sector_version_id", "input_identity_sha256"],
    properties: {
      universe_snapshot_id: uuid,
      price: {
        type: "object",
        required: ["dataset_version_id", "curated_version", "manifest_sha256"],
        properties: {
          dataset_version_id: uuid,
          curated_version: { type: "integer", minimum: 1 },
          manifest_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
        },
      },
      market_status: { $ref: "#/components/schemas/CandidateSourcePin" },
      flow: { $ref: "#/components/schemas/CandidateSourcePin" },
      fundamental: { $ref: "#/components/schemas/CandidateSourcePin" },
      sector_version_id: uuid,
      input_identity_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  },
  UniverseKey: {
    type: "string",
    enum: ["kospi200", "kosdaq150"],
    description: "Point-in-time candidate universe; omitted API queries default to kospi200.",
  },
  CandidateSourcePin: {
    type: "object",
    required: ["dataset_version_id", "manifest_sha256"],
    properties: {
      dataset_version_id: uuid,
      manifest_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  },
  CandidateScores: {
    type: "object",
    required: ["flow", "fundamental", "technical", "total"],
    properties: {
      flow: { type: ["number", "null"], minimum: 0, maximum: 100 },
      fundamental: { type: ["number", "null"], minimum: 0, maximum: 100 },
      technical: { type: ["number", "null"], minimum: 0, maximum: 100 },
      total: { type: ["number", "null"], minimum: 0, maximum: 100 },
    },
  },
  CandidateCoverage: {
    type: "object",
    required: ["flow", "fundamental", "technical"],
    properties: {
      flow: { type: "number", minimum: 0, maximum: 1 },
      fundamental: { type: "number", minimum: 0, maximum: 1 },
      technical: { type: "number", minimum: 0, maximum: 1 },
    },
  },
  CandidateAnalysis: {
    type: "object",
    required: ["analysis_id", "run_id", "universe", "instrument_id", "sector_code", "fundamental_profile", "eligible", "exclusion_codes", "scores", "coverage", "evidence_strength", "normalization_scope", "factors", "scenarios", "provenance", "content_sha256"],
    properties: {
      analysis_id: uuid,
      run_id: uuid,
      universe: { $ref: "#/components/schemas/UniverseKey" },
      instrument_id: { type: "string", example: "005930.KRX" },
      name: { type: ["string", "null"] },
      sector_code: { type: "string" },
      fundamental_profile: {
        type: "string",
        enum: ["candidate-non-financial-v1", "candidate-financial-v1", "unsupported"],
        description: "Versioned fundamental scoring profile selected for the instrument.",
      },
      eligible: { type: "boolean" },
      exclusion_codes: { type: "array", items: { type: "string" } },
      scores: { $ref: "#/components/schemas/CandidateScores" },
      coverage: { $ref: "#/components/schemas/CandidateCoverage" },
      evidence_strength: { type: "string", enum: ["STRONG", "MODERATE", "WEAK"] },
      rank: { type: ["integer", "null"], minimum: 1 },
      normalization_scope: {
        type: "string",
        enum: ["SECTOR", "UNIVERSE_FALLBACK", "UNAVAILABLE"],
      },
      factors: { type: "object", additionalProperties: true },
      scenarios: {
        type: "object",
        description: "Deterministic upside/neutral/downside trigger records; never probabilities or target prices.",
        additionalProperties: true,
      },
      provenance: { type: "object", additionalProperties: true },
      content_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  },
  CandidateResearchEnvelope: {
    type: "object",
    required: ["state", "as_of", "cutoff_at", "scoring_config", "dataset_pins", "license_attributions", "disclaimer"],
    properties: {
      universe: { anyOf: [{ $ref: "#/components/schemas/UniverseKey" }, { type: "null" }] },
      // Candidate success payloads are row-bearing. A blocked source is an
      // entitlement error, never a BLOCKED+rows success variant.
      state: { type: "string", enum: ["READY", "STALE"] },
      as_of: dateStr,
      cutoff_at: ts,
      scoring_config: {
        type: "object",
        required: ["version", "sha256"],
        properties: {
          version: { type: "string" },
          sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
        },
      },
      dataset_pins: { $ref: "#/components/schemas/CandidateDatasetPins" },
      license_attributions: {
        type: "array",
        minItems: 1,
        items: { $ref: "#/components/schemas/CandidateLicenseAttribution" },
      },
      disclaimer: { type: "string" },
    },
  },
  CandidateLicenseAttribution: {
    type: "object",
    required: ["source", "dataset_id", "license_ref", "entitlement_id", "contract_reference", "contract_document_sha256"],
    additionalProperties: false,
    properties: {
      source: { type: "string", enum: ["price", "universe", "market_status", "flow", "fundamental", "sector"] },
      dataset_id: { type: "string", pattern: "^krx_[a-z0-9_]+$" },
      license_ref: { type: "string", minLength: 1 },
      entitlement_id: uuid,
      contract_reference: { type: "string", minLength: 1 },
      contract_document_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  },
  CandidateFeed: {
    allOf: [
      { $ref: "#/components/schemas/CandidateResearchEnvelope" },
      {
        type: "object",
        required: ["feed_id", "universe", "published_at", "computation_seq", "items"],
        properties: {
          feed_id: uuid,
          universe: { $ref: "#/components/schemas/UniverseKey" },
          published_at: ts,
          computation_seq: { type: "integer", minimum: 1 },
          items: { type: "array", minItems: 5, maxItems: 5, items: { $ref: "#/components/schemas/CandidateAnalysis" } },
        },
      },
    ],
  },
  StockAnalysisResponse: {
    allOf: [
      { $ref: "#/components/schemas/CandidateResearchEnvelope" },
      {
        type: "object",
        required: ["universe", "analysis"],
        properties: {
          universe: { $ref: "#/components/schemas/UniverseKey" },
          analysis: { $ref: "#/components/schemas/CandidateAnalysis" },
        },
      },
    ],
  },
  ScreenCriteria: {
    type: "object",
    additionalProperties: false,
    properties: {
      universes: { type: "array", minItems: 1, maxItems: 2, uniqueItems: true, items: { $ref: "#/components/schemas/UniverseKey" }, description: "One or both universes; omitted defaults to kospi200." },
      sectors: { type: "array", maxItems: 64, items: { type: "string", maxLength: 32 } },
      evidence_strength: { type: "array", items: { type: "string", enum: ["STRONG", "MODERATE", "WEAK"] } },
      min_total_score: { type: ["number", "null"], minimum: 0, maximum: 100 },
      min_flow_score: { type: ["number", "null"], minimum: 0, maximum: 100 },
      min_fundamental_score: { type: ["number", "null"], minimum: 0, maximum: 100 },
      min_technical_score: { type: ["number", "null"], minimum: 0, maximum: 100 },
    },
  },
  ScreenerQueryBody: {
    type: "object",
    required: ["criteria"],
    additionalProperties: false,
    properties: {
      run_id: { type: ["string", "null"], format: "uuid", description: "Legacy exact single-universe run pin; incompatible with both universes." },
      as_of: dateStr,
      criteria: { $ref: "#/components/schemas/ScreenCriteria" },
      cursor: { type: ["string", "null"], description: "opaque HMAC-signed frozen run-set/universe/decimal-score/instrument cursor (v2; legacy v1 is KOSPI-only)" },
      limit: { type: ["integer", "null"], minimum: 1, maximum: 100, default: 25 },
    },
  },
  ScreenerResult: {
    allOf: [
      { $ref: "#/components/schemas/CandidateResearchEnvelope" },
      {
        type: "object",
        required: ["universes", "run_ids", "items", "next_cursor"],
        properties: {
          universe: { anyOf: [{ $ref: "#/components/schemas/UniverseKey" }, { type: "null" }] },
          universes: { type: "array", minItems: 1, items: { $ref: "#/components/schemas/UniverseKey" } },
          run_id: { type: ["string", "null"], format: "uuid" },
          run_ids: { type: "array", minItems: 1, items: { type: "object", required: ["universe", "run_id"], properties: { universe: { $ref: "#/components/schemas/UniverseKey" }, run_id: uuid } } },
          items: { type: "array", items: { $ref: "#/components/schemas/CandidateAnalysis" } },
          next_cursor: { type: ["string", "null"] },
        },
      },
    ],
  },
  SavedScreenBody: {
    type: "object",
    required: ["name", "criteria"],
    additionalProperties: false,
    properties: {
      name: { type: "string", minLength: 1, maxLength: 80 },
      criteria: { $ref: "#/components/schemas/ScreenCriteria" },
    },
  },
  SavedScreen: {
    type: "object",
    required: ["id", "name", "criteria_schema_version", "criteria", "created_at", "updated_at"],
    properties: {
      id: uuid,
      name: { type: "string" },
      criteria_schema_version: { type: "integer", enum: [1, 2] },
      criteria: { $ref: "#/components/schemas/ScreenCriteria" },
      created_at: ts,
      updated_at: ts,
    },
  },
  SavedScreenList: {
    type: "object",
    required: ["items"],
    properties: { items: { type: "array", items: { $ref: "#/components/schemas/SavedScreen" } } },
  },
  DeleteSavedScreenResult: {
    type: "object",
    required: ["id", "deleted"],
    properties: { id: uuid, deleted: { type: "boolean", const: true } },
  },
  BacktestRun: {
    type: "object",
    required: ["id", "owner_user_id", "can_manage", "strategy_id", "strategy_version", "status"],
    properties: {
      id: uuid,
      owner_user_id: uuid,
      can_manage: { type: "boolean", description: "Whether the current actor may mutate or cancel this run" },
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
      cost_profile_id: { type: "string", enum: ["KRX_ETF_DEFAULT", "CUSTOM"], description: "Same identities as an account's profile; CUSTOM is not yet selectable and is rejected rather than substituted" },
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
  RobustnessSuiteBody: {
    type: "object",
    additionalProperties: false,
    description: "One entry per requested derived-run child; each entry changes exactly one axis (design 9.5). Omitting axes runs the standard adverse/extreme cost-stress pair. A canceled parent run cascades to every child job through the existing cancel route.",
    properties: {
      axes: {
        type: "array",
        minItems: 1,
        maxItems: 25,
        items: {
          type: "object",
          required: ["axis"],
          properties: {
            axis: { type: "string", enum: ["parameter_neighborhood", "cost_stress", "period_split", "walk_forward", "execution_delay", "benchmark_comparison"] },
            parameter: { type: "string" },
            delta: {},
            profile_id: { type: "string" },
            profile_version: { type: "integer" },
            train_end: dateStr,
            validation_end: dateStr,
            window_sessions: { type: "integer" },
            step_sessions: { type: "integer" },
            delay_sessions: { type: "integer" },
            benchmark_id: { type: "string" },
          },
        },
      },
      holdout: {
        type: "object",
        description: "The train/validation boundary a period_split child must never read past (FR-ROB-001).",
        required: ["train_end", "validation_end"],
        properties: { train_end: dateStr, validation_end: dateStr },
      },
    },
  },
  Robustness: {
    type: "object",
    required: ["run_id", "suite_id", "children"],
    properties: {
      run_id: uuid,
      suite_id: uuid,
      children: {
        type: "array",
        items: {
          type: "object",
          required: ["run_id", "job_id", "axis", "status"],
          properties: {
            run_id: uuid,
            job_id: uuid,
            axis: { type: "string" },
            status: { type: "string", enum: ["QUEUED", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"] },
          },
        },
      },
    },
  },
  Account: {
    type: "object",
    required: ["id", "owner_user_id", "can_manage", "account_type", "name", "currency", "status", "cost_profile_id", "cost_profile_version"],
    properties: {
      id: uuid,
      owner_user_id: uuid,
      can_manage: { type: "boolean", description: "Whether the current actor may change this account" },
      account_type: { type: "string", enum: ["PAPER"], description: "LIVE accounts are Phase 3 Owner-only and never creatable via this route" },
      name: { type: "string" },
      currency: { type: "string", enum: ["KRW"] },
      status: { type: "string", enum: ["ACTIVE", "SUSPENDED", "CLOSED"] },
      initial_cash: { type: ["string", "null"], description: "The opening deposit; current cash is derived from cash_ledger, never cached here" },
      cost_profile_id: { type: "string", enum: ["KRX_ETF_DEFAULT", "CUSTOM"] },
      cost_profile_version: { type: "integer" },
      created_at: ts,
      updated_at: ts,
    },
  },
  NewAccountBody: {
    type: "object",
    required: ["name", "currency", "initial_cash"],
    additionalProperties: false,
    properties: {
      name: { type: "string", minLength: 1 },
      currency: { type: "string", enum: ["KRW"] },
      initial_cash: decimalStr,
      cost_profile_id: { type: "string", enum: ["KRX_ETF_DEFAULT", "CUSTOM"], description: "Defaults to KRX_ETF_DEFAULT; CUSTOM is not yet configurable through this route" },
    },
  },
  BindStrategy: {
    type: "object",
    required: ["account_id", "strategy_config_id", "strategy_id", "strategy_version", "bound_at"],
    properties: {
      account_id: uuid,
      strategy_config_id: uuid,
      strategy_id: { type: "string" },
      strategy_version: { type: "string" },
      bound_at: ts,
    },
  },
  PerformancePoint: {
    type: "object",
    required: ["trading_date", "equity", "cash", "positions_value", "currency", "cash_reconciled"],
    properties: {
      trading_date: dateStr,
      equity: decimalStr,
      cash: decimalStr,
      positions_value: decimalStr,
      currency: { type: "string", enum: ["KRW"] },
      return_pct: { type: ["string", "null"], description: "Day-over-day return, computed on read from ledger-derived equity; absent on the first point" },
      cash_reconciled: { type: "boolean", description: "Whether cash agrees with cash_ledger, the authority, as of this date" },
    },
  },
  Performance: {
    type: "object",
    required: ["account_id", "points", "disclaimer"],
    properties: {
      account_id: uuid,
      points: { type: "array", items: { $ref: "#/components/schemas/PerformancePoint" } },
      disclaimer: { type: "string", description: "Rendered verbatim; Paper results are simulated and never a guarantee of future returns" },
    },
  },
  Lineage: {
    type: "object",
    required: ["account_id", "bindings", "targets"],
    properties: {
      account_id: uuid,
      bindings: {
        type: "array",
        description: "Immutable strategy-binding history; a rebind closes the old row and opens a new one (branching lineage)",
        items: {
          type: "object",
          required: ["strategy_config_id", "strategy_id", "strategy_version", "bound_at", "active"],
          properties: {
            strategy_config_id: uuid,
            strategy_id: { type: "string" },
            strategy_version: { type: "string" },
            bound_at: ts,
            unbound_at: { type: ["string", "null"], format: "date-time" },
            active: { type: "boolean" },
          },
        },
      },
      targets: {
        type: "array",
        description: "Each close(T) computation and the session T+1 it executed at",
        items: {
          type: "object",
          required: ["id", "computed_on", "effective_date", "status"],
          properties: {
            id: uuid,
            computed_on: dateStr,
            effective_date: dateStr,
            status: { type: "string", enum: ["PENDING", "EXECUTED", "SKIPPED"] },
            executed_at: { type: ["string", "null"], format: "date-time" },
          },
        },
      },
    },
  },
  Parity: {
    type: "object",
    required: ["account_id", "as_of", "status", "lineage", "divergences", "fill_model_difference", "warrants_alert"],
    description: "Computed on read, never stored, so it cannot go stale against the lineage it describes.",
    properties: {
      account_id: uuid,
      as_of: dateStr,
      status: { type: "string", enum: ["MATCH", "DIVERGENT", "NOT_COMPARABLE"], description: "NOT_COMPARABLE means the two sides came from different strategy/data/as-of inputs, so no parity claim is meaningful" },
      lineage: { type: "object", additionalProperties: true },
      divergences: { type: "array", items: { type: "object", additionalProperties: true } },
      fill_model_difference: { type: "string", description: "Stated on every report: backtest fills come from the NT engine, Paper fills are modeled at the next raw open plus slippage" },
      warrants_alert: { type: "boolean", description: "True for DIVERGENT and NOT_COMPARABLE (design 15.3 grades a Paper divergence WARNING)" },
    },
  },
  Notification: {
    type: "object",
    required: ["id", "kind", "title", "body", "created_at", "deliveries"],
    description: "One feed row plus every attempt made to deliver it, so an outage is visible to the recipient and not only in the Owner's admin view (FR-RPT-002).",
    properties: {
      id: uuid,
      kind: { type: "string", enum: ["job", "recommendation", "backtest", "alert"] },
      title: { type: "string" },
      body: { type: "string" },
      read_at: { type: ["string", "null"], format: "date-time" },
      created_at: ts,
      deliveries: {
        type: "array",
        items: {
          type: "object",
          required: ["channel", "status"],
          properties: {
            channel: { type: "string", enum: ["web", "email", "admin"] },
            status: { type: "string", enum: ["SUCCESS", "FAILED"] },
            error_detail: { type: "string", description: "present only on FAILED; a recorded outage is never silent" },
          },
        },
      },
    },
  },
  BindStrategyBody: {
    type: "object",
    required: ["strategy_config_id"],
    additionalProperties: false,
    properties: { strategy_config_id: uuid },
  },
  RebalancePreviewBody: {
    type: "object",
    required: ["recommendation_run_id"],
    additionalProperties: false,
    properties: { recommendation_run_id: uuid },
  },
  ApplyRebalancePreviewBody: {
    type: "object",
    required: ["preview_token"],
    additionalProperties: false,
    properties: {
      preview_token: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  },
  RebalancePreviewLineage: {
    type: "object",
    required: [
      "account_id",
      "recommendation_run_id",
      "target_portfolio_id",
      "strategy_config_id",
      "dataset_version_id",
      "curated_version",
      "dataset_manifest_sha256",
      "account_state_version",
      "account_state_sha256",
      "target_portfolio_sha256",
    ],
    additionalProperties: false,
    properties: {
      account_id: uuid,
      recommendation_run_id: uuid,
      target_portfolio_id: uuid,
      strategy_config_id: uuid,
      dataset_version_id: uuid,
      curated_version: { type: "integer", minimum: 0 },
      dataset_manifest_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
      account_state_version: { type: "integer", format: "int64", minimum: 0 },
      account_state_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
      target_portfolio_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  },
  RebalancePreviewDecision: {
    type: "object",
    required: [
      "instrument_id",
      "current_quantity",
      "current_value",
      "current_weight",
      "target_value",
      "target_weight",
      "delta_value",
      "action",
      "skip_reason",
    ],
    additionalProperties: false,
    properties: {
      instrument_id: { type: "string", example: "069500.KRX" },
      current_quantity: decimalStr,
      current_value: decimalStr,
      current_weight: decimalStr,
      target_value: decimalStr,
      target_weight: decimalStr,
      delta_value: decimalStr,
      action: { type: "string", enum: ["BUY", "SELL", "SKIP"] },
      skip_reason: {
        type: ["string", "null"],
        enum: [
          "BELOW_REBALANCE_THRESHOLD",
          "BELOW_MIN_TRADE",
          "NO_AVAILABLE_CASH",
          "NO_AFFORDABLE_LOT",
          null,
        ],
      },
    },
  },
  RebalancePreviewOrder: {
    type: "object",
    required: [
      "instrument_id",
      "side",
      "quantity",
      "raw_price",
      "estimated_execution_price",
      "notional",
      "commission",
      "tax",
      "informational_slippage",
    ],
    additionalProperties: false,
    properties: {
      instrument_id: { type: "string", example: "069500.KRX" },
      side: { type: "string", enum: ["BUY", "SELL"] },
      quantity: decimalStr,
      raw_price: decimalStr,
      estimated_execution_price: decimalStr,
      notional: decimalStr,
      commission: decimalStr,
      tax: decimalStr,
      informational_slippage: decimalStr,
    },
  },
  RebalancePreviewResult: {
    type: "object",
    required: [
      "schema_version",
      "price_basis",
      "price_date",
      "proposed_effective_date",
      "equity",
      "cash_before",
      "available_cash",
      "leftover_cash",
      "buy_notional",
      "sell_notional",
      "explicit_fees",
      "informational_slippage",
      "decisions",
      "orders",
      "warning_code",
      "lineage",
    ],
    additionalProperties: false,
    properties: {
      schema_version: { type: "integer", const: 1 },
      price_basis: { type: "string", const: "RECOMMENDATION_CLOSE" },
      price_date: dateStr,
      proposed_effective_date: dateStr,
      equity: decimalStr,
      cash_before: decimalStr,
      available_cash: decimalStr,
      leftover_cash: decimalStr,
      buy_notional: decimalStr,
      sell_notional: decimalStr,
      explicit_fees: decimalStr,
      informational_slippage: decimalStr,
      decisions: { type: "array", items: { $ref: "#/components/schemas/RebalancePreviewDecision" } },
      orders: { type: "array", items: { $ref: "#/components/schemas/RebalancePreviewOrder" } },
      warning_code: { type: "string", const: "INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED" },
      lineage: { $ref: "#/components/schemas/RebalancePreviewLineage" },
    },
  },
  RebalancePreviewError: {
    type: "object",
    required: ["code", "message"],
    additionalProperties: false,
    properties: {
      code: { type: "string" },
      message: { type: "string" },
    },
  },
  RebalancePreview: {
    type: "object",
    required: [
      "id",
      "account_id",
      "recommendation_run_id",
      "target_portfolio_id",
      "strategy_config_id",
      "job_id",
      "status",
      "price_basis",
      "price_date",
      "proposed_effective_date",
      "dataset_version_id",
      "dataset_manifest_sha256",
      "target_portfolio_sha256",
      "preview_token",
      "created_at",
      "started_at",
      "completed_at",
      "applied_at",
      "updated_at",
    ],
    additionalProperties: false,
    properties: {
      id: uuid,
      account_id: uuid,
      recommendation_run_id: uuid,
      target_portfolio_id: uuid,
      strategy_config_id: uuid,
      job_id: uuid,
      status: { type: "string", enum: ["PENDING", "RUNNING", "READY", "FAILED", "APPLIED"] },
      price_basis: { type: "string", const: "RECOMMENDATION_CLOSE" },
      price_date: dateStr,
      proposed_effective_date: { type: ["string", "null"], pattern: "^\\d{4}-\\d{2}-\\d{2}$" },
      dataset_version_id: uuid,
      dataset_manifest_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
      target_portfolio_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
      preview_token: { type: ["string", "null"], pattern: "^[0-9a-f]{64}$" },
      result: { $ref: "#/components/schemas/RebalancePreviewResult" },
      error: { $ref: "#/components/schemas/RebalancePreviewError" },
      created_at: ts,
      started_at: { type: ["string", "null"], format: "date-time" },
      completed_at: { type: ["string", "null"], format: "date-time" },
      applied_at: { type: ["string", "null"], format: "date-time" },
      updated_at: ts,
    },
  },
  AppliedRebalancePreview: {
    type: "object",
    required: ["preview_id", "pending_target_id", "effective_date", "source_kind", "status"],
    additionalProperties: false,
    properties: {
      preview_id: uuid,
      pending_target_id: uuid,
      effective_date: dateStr,
      source_kind: { type: "string", const: "MANUAL_RECOMMENDATION" },
      status: { type: "string", const: "APPLIED" },
    },
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
    required: ["trading_date", "equity", "cash", "positions_value", "currency", "cash_reconciled"],
    properties: {
      trading_date: dateStr,
      equity: { type: "string" },
      cash: { type: "string" },
      positions_value: { type: "string" },
      currency: { type: "string" },
      cash_reconciled: { type: "boolean", description: "Whether cash agrees with cash_ledger, the authority, as of this date" },
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
    required: [
      "user_id",
      "role",
      "expires_at_secs",
      "owner_beta_access_mode",
      "owner_beta_paper_mode",
    ],
    properties: {
      user_id: { type: "string", format: "uuid" },
      role: { type: "string", enum: ["owner", "member"] },
      expires_at_secs: { type: "integer" },
      auth_time_secs: { type: "integer" },
      owner_beta_access_mode: { type: "string", enum: ["disabled", "owner_only"] },
      owner_beta_paper_mode: { type: "string", enum: ["disabled", "enabled"] },
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
      { name: "candidates", description: "Common point-in-time stock research feed" },
      { name: "stocks", description: "Point-in-time deep stock analysis" },
      { name: "screener", description: "Candidate screening and actor-owned saved criteria" },
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
