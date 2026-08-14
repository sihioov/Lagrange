import { createServer } from "node:http";
import { backtestResponse } from "./backtest-fixture.mjs";
import { liveResponse } from "./live-fixture.mjs";
import { paperResponse, paperStrategyConfigs } from "./paper-fixture.mjs";
import { recommendationConfig, recommendationResponse } from "./recommendation-fixture.mjs";

const port = Number.parseInt(process.env.SYNTHETIC_API_PORT ?? "38180", 10);
const defaultScenario = Object.freeze({
  backtest: "running",
  entitlement: "active",
  exclusions: "present",
  notification: "delivered",
  paperAccount: "present",
  paperEntitlement: "active",
  liveMfa: "fresh",
  // Never reconciled by default, matching `Readiness::NeverReconciled`: a
  // fresh install, a restored backup and a crashed-before-first-run process
  // all land there, and all of them must block. A test that wants Live
  // re-enabled has to say so, which is the same order of events an operator
  // faces.
  //
  // There is deliberately no `killSwitch` key: the page has no read route for
  // the state yet and renders ENGAGED unconditionally, so a scenario key would
  // imply a capability the product does not have.
  reconciliation: "never",
  parity: "match",
  recommendation: "fresh",
  tradePagination: "normal",
  user: "u1",
  role: "member",
});
let scenario = { ...defaultScenario };

// Five invited identities: u1..u5 map to deterministic distinct user ids.
function userIndex(scenario) {
  const match = /^u(\d)$/.exec(scenario.user ?? "u1");
  return match ? Number(match[1]) : 1;
}

function userIdFor(scenario) {
  const idx = userIndex(scenario);
  return `00000000-0000-4000-8000-${"00000000000"}${idx}`;
}

function json(response, status, body) {
  response.writeHead(status, {
    "Cache-Control": "no-store",
    "Content-Type": "application/json; charset=utf-8",
  });
  response.end(JSON.stringify(body));
}

async function requestBody(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }
  if (chunks.length === 0) {
    return {};
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  const body = request.method === "POST" ? await requestBody(request) : {};
  if (request.method === "POST" && url.pathname === "/__test/scenario") {
    // Reset to defaults, THEN apply. Merging into the previous scenario let
    // state outlive the test that set it: a key no later spec mentions kept
    // whatever the last one left. `role` made that dangerous rather than
    // merely untidy — `no-member-live.spec.ts` is the only spec that sets
    // `role: "owner"`, and under merge semantics it stayed owner for every
    // spec that ran afterwards. A suite asserting a Member cannot reach
    // something would then be asserting it about an Owner, and passing for the
    // wrong reason.
    //
    // Every call site is unaffected: each states the keys it varies and wants
    // documented defaults for the rest, which is now what it gets.
    scenario = { ...defaultScenario, ...body };
    json(response, 200, { scenario });
    return;
  }
  if (request.method === "GET" && url.pathname === "/api/v1/strategy-configs") {
    // This is one shared production endpoint, not a recommendation-only or
    // Paper-only route. Keep the synthetic response equally coherent: every
    // saved config must have a unique identity and all product surfaces must
    // observe the same list regardless of fixture dispatch order.
    json(response, 200, {
      has_more: false,
      items: [recommendationConfig(), ...paperStrategyConfigs(scenario)],
      next_cursor: null,
    });
    return;
  }
  const product = recommendationResponse({
    body,
    headers: request.headers,
    method: request.method ?? "GET",
    pathname: url.pathname,
    scenario,
  });
  if (product !== null) {
    json(response, product.status, product.body);
    return;
  }
  const backtest = backtestResponse({
    body,
    headers: request.headers,
    method: request.method ?? "GET",
    pathname: url.pathname,
    scenario,
  });
  if (backtest !== null) {
    json(response, backtest.status, backtest.body);
    return;
  }
  const live = liveResponse({
    body,
    headers: request.headers,
    method: request.method ?? "GET",
    pathname: url.pathname,
    scenario,
  });
  if (live !== null) {
    json(response, live.status, live.body);
    return;
  }
  const paper = paperResponse({
    body,
    headers: request.headers,
    method: request.method ?? "GET",
    pathname: url.pathname,
    scenario,
  });
  if (paper !== null) {
    json(response, paper.status, paper.body);
    return;
  }
  if (request.method === "GET" && url.pathname === "/api/v1/auth/session") {
    json(response, 200, {
      expires_at_secs: 2_000_000_000,
      role: scenario.role === "owner" ? "owner" : "member",
      user_id: userIdFor(scenario),
    });
    return;
  }
  json(response, 501, {
    error: {
      code: "NOT_IMPLEMENTED",
      message: `Synthetic endpoint ${request.method} ${url.pathname} is not implemented`,
      request_id: "request-synthetic-red",
    },
  });
});

server.listen(port, "127.0.0.1", () => {
  process.stdout.write(`SYNTHETIC_API_READY http://127.0.0.1:${port}\n`);
});

function shutdown() {
  server.close(() => process.exit(0));
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
