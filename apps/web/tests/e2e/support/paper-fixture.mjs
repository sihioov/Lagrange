const ACCOUNT_ID = "00000000-0000-4000-8000-000000000401";
const CONFIG_ID = "00000000-0000-4000-8000-000000000103";
const PRIOR_CONFIG_ID = "00000000-0000-4000-8000-000000000102";
const ORDER_ID = "00000000-0000-4000-8000-000000000411";
const TARGET_ID = "00000000-0000-4000-8000-000000000421";
const NOTICE_ID = "00000000-0000-4000-8000-000000000431";

// u1 keeps the canonical ids; every other invited identity gets its own set,
// stamped one digit above the trailing number exactly as the backtest and
// recommendation fixtures do, so no remapping walks into another id block.
function userIndex(scenario) {
  const match = /^u(\d)$/.exec(scenario.user ?? "u1");
  return match ? Number(match[1]) : 1;
}

function idFor(scenario, baseId) {
  const idx = userIndex(scenario);
  return idx === 1 ? baseId : `${baseId.slice(0, -4)}${idx}${baseId.slice(-3)}`;
}

function account(scenario, viewer = scenario) {
  return {
    account_type: "PAPER",
    cost_profile_id: "KRX_ETF_DEFAULT",
    cost_profile_version: 1,
    created_at: "2026-01-02T00:30:00Z",
    currency: "KRW",
    id: idFor(scenario, ACCOUNT_ID),
    initial_cash: "10000000.0000",
    owner_user_id: `00000000-0000-4000-8000-00000000000${userIndex(scenario)}`,
    can_manage: userIndex(scenario) === userIndex(viewer),
    name: `Paper account ${userIndex(scenario)}`,
    status: "ACTIVE",
    updated_at: "2026-02-02T00:30:00Z",
  };
}

function performance(scenario) {
  return {
    account_id: idFor(scenario, ACCOUNT_ID),
    disclaimer:
      "Simulated results from a paper account. Fills are modeled, not executed in a real market, and past simulated performance is not a guarantee of future returns.",
    points: [
      {
        cash: "10000000.0000",
        currency: "KRW",
        equity: "10000000.0000",
        positions_value: "0.0000",
        return_pct: null,
        trading_date: "2026-01-30",
      },
      {
        cash: "1994320.0000",
        currency: "KRW",
        equity: "10042180.0000",
        positions_value: "8047860.0000",
        return_pct: "0.004218",
        trading_date: "2026-02-02",
      },
    ],
  };
}

function lineage(scenario) {
  return {
    account_id: idFor(scenario, ACCOUNT_ID),
    bindings: [
      {
        active: false,
        bound_at: "2026-01-02T00:30:00Z",
        strategy_config_id: idFor(scenario, PRIOR_CONFIG_ID),
        strategy_id: "buy_and_hold",
        strategy_version: "1.0.0",
        unbound_at: "2026-01-20T00:30:00Z",
      },
      {
        active: true,
        bound_at: "2026-01-20T00:30:00Z",
        strategy_config_id: idFor(scenario, CONFIG_ID),
        strategy_id: "dual_momentum",
        strategy_version: "2.3.1",
        unbound_at: null,
      },
    ],
    targets: [
      {
        computed_on: "2026-01-30",
        effective_date: "2026-02-02",
        executed_at: "2026-02-02T00:05:00Z",
        id: idFor(scenario, TARGET_ID),
        status: "EXECUTED",
      },
    ],
  };
}

const FILL_MODEL_DIFFERENCE =
  "Backtest fills come from the NautilusTrader engine's execution model; Paper fills are modeled at the next session's raw open plus the configured slippage, so identical signals can still produce different fills.";

function parity(scenario) {
  const divergent = scenario.parity === "divergent";
  const incomparable = scenario.parity === "incomparable";
  if (incomparable) {
    return {
      account_id: idFor(scenario, ACCOUNT_ID),
      as_of: "2026-01-30",
      divergences: [],
      fill_model_difference: FILL_MODEL_DIFFERENCE,
      lineage: {
        fields: [
          { backtest: "krx-eod.2026-01-29", field: "dataset_version", paper: "krx-eod.2026-01-30" },
          { backtest: "2.3.1", field: "strategy_version", paper: "2.3.1" },
        ],
      },
      status: "NOT_COMPARABLE",
      warrants_alert: true,
    };
  }
  return {
    account_id: idFor(scenario, ACCOUNT_ID),
    as_of: "2026-01-30",
    divergences: divergent
      ? [
          { backtest_weight: "0.900000", instrument_id: "069500.KRX", paper_weight: "0.600000" },
          { backtest_weight: "0.100000", instrument_id: "229200.KRX", paper_weight: "0.400000" },
        ]
      : [],
    fill_model_difference: FILL_MODEL_DIFFERENCE,
    lineage: {
      fields: [
        { backtest: "dual_momentum", field: "strategy_id", paper: "dual_momentum" },
        { backtest: "2.3.1", field: "strategy_version", paper: "2.3.1" },
        { backtest: "krx-eod.2026-01-30", field: "dataset_version", paper: "krx-eod.2026-01-30" },
        { backtest: "2026-01-30", field: "as_of", paper: "2026-01-30" },
      ],
    },
    status: divergent ? "DIVERGENT" : "MATCH",
    warrants_alert: divergent,
  };
}

function notifications(scenario) {
  // The delivery outcome is part of the fixture on purpose: an outage must
  // be visible on the page, not only in the Owner's admin view.
  const outage = scenario.notification === "outage";
  const divergent = scenario.parity === "divergent";
  return [
    {
      body: divergent
        ? "The paper session for 2026-02-02 executed, but its target weights differ from the backtest computed for the same close."
        : "The paper session for 2026-02-02 executed its target and its signals match the backtest for the same close.",
      created_at: "2026-02-02T00:06:00Z",
      deliveries: outage
        ? [
            { channel: "web", status: "SUCCESS" },
            {
              channel: "email",
              error_detail: "email delivery not configured in this release",
              status: "FAILED",
            },
          ]
        : [{ channel: "web", status: "SUCCESS" }],
      id: idFor(scenario, NOTICE_ID),
      kind: divergent ? "alert" : "job",
      title: divergent
        ? "Paper session 2026-02-02 diverged from its backtest"
        : "Paper session 2026-02-02 completed",
    },
  ];
}

export function paperStrategyConfigs(scenario) {
  return [
    {
      created_at: "2026-01-20T00:30:00Z",
      id: idFor(scenario, CONFIG_ID),
      is_active: true,
      strategy_id: "dual_momentum",
      strategy_version: "2.3.1",
      updated_at: "2026-01-20T00:30:00Z",
    },
    {
      created_at: "2026-01-02T00:30:00Z",
      id: idFor(scenario, PRIOR_CONFIG_ID),
      is_active: false,
      strategy_id: "buy_and_hold",
      strategy_version: "1.0.0",
      updated_at: "2026-01-02T00:30:00Z",
    },
  ];
}

function page(items) {
  return { body: { has_more: false, items, next_cursor: null }, status: 200 };
}

function error(status, code, message) {
  return {
    body: { error: { code, message, request_id: "request-synthetic-paper" } },
    status,
  };
}

function mutationAuthorized(headers) {
  return Boolean(headers["x-csrf-token"] && headers["idempotency-key"]);
}

export function paperResponse(request) {
  const { body, headers, method, pathname, scenario } = request;
  if (method === "GET" && pathname === "/api/v1/notifications") {
    return page(notifications(scenario));
  }
  if (!pathname.startsWith("/api/v1/paper/")) {
    return null;
  }
  if (scenario.paperEntitlement === "blocked") {
    return error(403, "DATA_ENTITLEMENT_REQUIRED", "paper entitlement is inactive");
  }
  if (method === "GET" && pathname === "/api/v1/paper/accounts") {
    if (scenario.paperAccount === "absent") {
      return page([]);
    }
    const accountScenarios = [1, 2, 3, 4, 5].map((index) => ({
      ...scenario,
      user: `u${index}`,
    }));
    return page(accountScenarios.map((candidate) => account(candidate, scenario)));
  }

  const accountScenario = [1, 2, 3, 4, 5]
    .map((index) => ({ ...scenario, user: `u${index}` }))
    .find((candidate) =>
      pathname.startsWith(`/api/v1/paper/accounts/${idFor(candidate, ACCOUNT_ID)}`),
    );
  if (accountScenario === undefined) {
    return error(404, "RESOURCE_NOT_FOUND", "account not found");
  }
  const resource = `/api/v1/paper/accounts/${idFor(accountScenario, ACCOUNT_ID)}`;
  const tail = pathname.slice(resource.length);
  if (method === "POST" && tail === "/bind-strategy") {
    if (userIndex(accountScenario) !== userIndex(scenario)) {
      return error(404, "RESOURCE_NOT_FOUND", "account not found");
    }
    if (!mutationAuthorized(headers)) {
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    }
    const configs = paperStrategyConfigs(scenario);
    const chosen = configs.find((config) => config.id === body?.strategy_config_id);
    if (chosen === undefined) {
      return error(404, "RESOURCE_NOT_FOUND", "strategy config not found");
    }
    return {
      body: {
        account_id: idFor(scenario, ACCOUNT_ID),
        bound_at: "2026-02-03T00:30:00Z",
        strategy_config_id: chosen.id,
        strategy_id: chosen.strategy_id,
        strategy_version: chosen.strategy_version,
      },
      status: 200,
    };
  }
  if (method !== "GET") {
    return null;
  }
  if (tail === "") {
    return { body: account(accountScenario, scenario), status: 200 };
  }
  if (tail === "/performance") {
    return { body: performance(accountScenario), status: 200 };
  }
  if (tail === "/lineage") {
    return { body: lineage(accountScenario), status: 200 };
  }
  if (tail.startsWith("/parity")) {
    return { body: parity(accountScenario), status: 200 };
  }
  if (tail === "/positions") {
    return page([
      {
        avg_price: "40239.3000",
        instrument_id: "069500.KRX",
        quantity: "200",
        updated_at: "2026-02-02T00:05:00Z",
      },
    ]);
  }
  if (tail === "/orders") {
    return page([
      {
        created_at: "2026-02-02T00:00:00Z",
        id: idFor(accountScenario, ORDER_ID),
        instrument_id: "069500.KRX",
        order_ref: "paper-2026-02-02-069500",
        price: "40200.0000",
        quantity: "200",
        side: "BUY",
        status: "FILLED",
        submitted_at: "2026-02-02T00:00:05Z",
      },
    ]);
  }
  return null;
}

export const PAPER_FIXTURE_IDS = { ACCOUNT_ID, CONFIG_ID, PRIOR_CONFIG_ID, idFor };
