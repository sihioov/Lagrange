const CONNECTION_ID = "00000000-0000-4000-8000-000000000901";

// The synthetic broker connection. Note what it does NOT contain: no app key,
// no app secret, no full account number. The real server has no field capable
// of holding one, so a fixture that invented such a field would let an e2e
// test pass against a shape the product cannot produce.
function connection() {
  return {
    account_no_masked: "****6-01",
    account_product_code: "01",
    id: CONNECTION_ID,
    kis_app_key_ref: "env:KIS_APP_KEY",
    kis_app_secret_ref: "file:/run/secrets/kis_app_secret",
    label: "KIS simulator",
    profile: "mock",
    status: "CONFIGURED",
  };
}

function error(status, code, message) {
  return {
    body: { error: { code, message, request_id: "request-synthetic-live" } },
    status,
  };
}

/**
 * Owner-only Live endpoints.
 *
 * The refusal shapes mirror the server exactly, because that is what the
 * no-member-live spec asserts:
 *   * a non-Owner gets 404 RESOURCE_NOT_FOUND — never 403, which would confirm
 *     the route exists;
 *   * an Owner without fresh MFA gets 403 with a STEP_UP_* code, because they
 *     may perform the action, just not right now.
 */
export function liveResponse(request) {
  const { method, pathname, scenario } = request;
  if (!pathname.startsWith("/api/v1/admin/live/")) {
    return null;
  }

  // Role first: a Member must not be able to distinguish these paths from
  // paths that were never built.
  if (scenario.role !== "owner") {
    return error(404, "RESOURCE_NOT_FOUND", "not found");
  }

  // Then freshness. `liveMfa: "stale"` models an Owner whose second factor is
  // too old to authorise a Live action.
  if (scenario.liveMfa === "stale") {
    return error(
      403,
      "STEP_UP_AUTH_TIME_STALE",
      "this action requires a fresh multi-factor authentication",
    );
  }
  if (scenario.liveMfa === "absent") {
    return error(
      403,
      "STEP_UP_MFA_REQUIRED",
      "this action requires a fresh multi-factor authentication",
    );
  }

  if (method === "GET" && pathname === "/api/v1/admin/live/connections") {
    return {
      body: { has_more: false, items: [connection()], next_cursor: null },
      status: 200,
    };
  }
  // DISENGAGING additionally requires a green reconciliation (FR-LIVE-004).
  // The asymmetry below is the product's, not the fixture's: engaging is never
  // refused, because a precondition on stopping Live is a precondition that
  // fails at the worst possible moment.
  if (method === "POST" && pathname === "/api/v1/admin/live/kill-switch/disable") {
    const readiness = readinessFor(scenario);
    if (readiness !== "READY") {
      return {
        body: {
          error: {
            code: "LIVE_RECONCILIATION_REQUIRED",
            details: { readiness },
            message:
              "Live requires a green reconciliation before the kill switch may be disengaged",
            request_id: "request-synthetic-live",
          },
        },
        status: 409,
      };
    }
    return { body: { engaged: false }, status: 200 };
  }
  if (method === "POST" && pathname === "/api/v1/admin/live/kill-switch/enable") {
    return { body: { engaged: true }, status: 200 };
  }
  return null;
}

/**
 * The scenario's reconciliation readiness, in the server's own vocabulary.
 *
 * Mirrors `api_server::repos::reconciliation::Readiness::reason`. Only "green"
 * yields READY — everything else blocks, including a run still in progress,
 * because "we do not know yet" is not permission.
 */
function readinessFor(scenario) {
  switch (scenario.reconciliation) {
    case "green":
      return "READY";
    case "mismatch":
      return "RECONCILIATION_MISMATCH";
    case "running":
      return "RECONCILIATION_IN_PROGRESS";
    default:
      return "NEVER_RECONCILED";
  }
}

export const LIVE_FIXTURE_IDS = { CONNECTION_ID };
