const BASELINE_RUN_ID = "00000000-0000-4000-8000-000000000301";
const RUNNING_RUN_ID = "00000000-0000-4000-8000-000000000302";
const HIGH_COST_RUN_ID = "00000000-0000-4000-8000-000000000303";
const FAILED_RUN_ID = "00000000-0000-4000-8000-000000000304";
const CANCELED_RUN_ID = "00000000-0000-4000-8000-000000000305";

function provenance(warnings = []) {
  return {
    data_version: "krx-eod@2025-12-31",
    engine_version: "nautilus@1.231.0",
    strategy_version: "dual_momentum@2.3.1",
    warnings,
  };
}

function run(id, status, label, summary = {}) {
  return {
    benchmark: "069500.KRX",
    config_sha256: `sha256:${id.slice(-12)}`,
    created_at: "2026-01-31T07:00:00Z",
    dataset_version: "krx-eod@2025-12-31",
    end_date: "2025-12-31",
    engine: "NautilusTrader",
    engine_version: "nautilus@1.231.0",
    finished_at: status === "SUCCEEDED" ? "2026-01-31T07:10:00Z" : null,
    id,
    job_id: `00000000-0000-4000-8000-0000000004${id.slice(-2)}`,
    start_date: "2020-01-02",
    started_at: "2026-01-31T07:01:00Z",
    status,
    strategy_id: "dual_momentum",
    strategy_version: "2.3.1",
    summary: {
      ...provenance(),
      cost_profile_id: "krx-default@2026-01",
      dataset_version_id: "00000000-0000-4000-8000-000000000601",
      execution_profile: "daily-close-next-open@1",
      run_label: label,
      strategy_config_id: "00000000-0000-4000-8000-000000000101",
      ...summary,
    },
  };
}

const baseline = run(BASELINE_RUN_ID, "SUCCEEDED", "Dual momentum baseline", {
  robustness_evidence: {
    cost_stress: "Stable through the adverse cost profile.",
    parameter_sensitivity: "Neighboring lookbacks remain within the approved dispersion band.",
    validation_periods: "Concentrated in three trades; inspect the holdout contribution.",
  },
  warnings: ["Next-open execution can differ from close-to-close benchmarks."],
});

const higherCosts = run(HIGH_COST_RUN_ID, "SUCCEEDED", "Dual momentum higher costs", {
  warnings: ["Higher costs reduce the server-reported total return."],
});

function runsFor(scenario) {
  if (scenario.backtest === "failed-canceled") {
    return [
      run(FAILED_RUN_ID, "FAILED", "Failed validation run", {
        failure_reason: "Worker exited before producing a verified result.",
      }),
      run(CANCELED_RUN_ID, "CANCELED", "Canceled member run"),
      baseline,
      higherCosts,
    ];
  }
  return [
    run(RUNNING_RUN_ID, "RUNNING", "Backtest progress", { progress_percent: "65" }),
    baseline,
    higherCosts,
  ];
}

function error(status, code, message) {
  return {
    body: { error: { code, message, request_id: "request-synthetic-backtest" } },
    status,
  };
}

function mutationAuthorized(headers) {
  return Boolean(headers["x-csrf-token"] && headers["idempotency-key"]);
}

function trades(scenario) {
  const total = scenario.tradePagination === "huge" ? 1200 : 2;
  return Array.from({ length: total }, (_, index) => ({
    cost: index === total - 1 ? "128450.00" : "1070.42",
    executed_at: "2025-12-01T00:00:00Z",
    instrument_id: index % 2 === 0 ? "069500.KRX" : "229200.KRX",
    quantity: String(100 + index),
    side: index % 2 === 0 ? "BUY" : "SELL",
    trade_id: `Trade ${index + 1}`,
  }));
}

export function backtestResponse(request) {
  const { body, headers, method, pathname, scenario } = request;
  if (scenario.entitlement === "blocked" && pathname.startsWith("/api/v1/backtests")) {
    return error(403, "DATA_ENTITLEMENT_REQUIRED", "backtest entitlement is inactive");
  }
  if (method === "GET" && pathname === "/api/v1/backtests") {
    return {
      body: { has_more: false, items: runsFor(scenario), next_cursor: null },
      status: 200,
    };
  }
  if (method === "POST" && pathname === "/api/v1/backtests") {
    if (!mutationAuthorized(headers)) {
      return error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
    }
    return {
      body: run("00000000-0000-4000-8000-000000000306", "PENDING", "New member run"),
      status: 201,
    };
  }
  if (method === "POST" && pathname === `/api/v1/backtests/${RUNNING_RUN_ID}/cancel`) {
    return mutationAuthorized(headers)
      ? {
          body: {
            job_id: "00000000-0000-4000-8000-000000000402",
            run_id: RUNNING_RUN_ID,
            status: "CANCEL_REQUESTED",
          },
          status: 202,
        }
      : error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
  }
  if (method === "POST" && pathname === "/api/v1/backtests/compare") {
    return mutationAuthorized(headers)
      ? {
          body: {
            deltas: { total_return: "-0.0321" },
            run_ids: body.run_ids,
            runs: [baseline, higherCosts].map((item) => ({
              run_id: item.id,
              status: item.status,
              strategy_id: item.strategy_id,
              summary: item.summary,
            })),
          },
          status: 200,
        }
      : error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
  }
  if (method === "POST" && pathname === `/api/v1/backtests/${BASELINE_RUN_ID}/robustness`) {
    return mutationAuthorized(headers)
      ? {
          body: {
            job_id: "00000000-0000-4000-8000-000000000499",
            run_id: BASELINE_RUN_ID,
            status: "QUEUED",
          },
          status: 202,
        }
      : error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
  }
  if (method === "GET" && pathname === `/api/v1/backtests/${BASELINE_RUN_ID}/metrics`) {
    return {
      body: {
        items: [
          { metric_key: "ending_equity", metric_value: "128450000.00" },
          { metric_key: "maximum_drawdown", metric_value: "-0.1842" },
          { metric_key: "total_cost", metric_value: "128450.00" },
        ],
      },
      status: 200,
    };
  }
  if (method === "GET" && pathname === `/api/v1/backtests/${BASELINE_RUN_ID}/equity`) {
    return {
      body: {
        artifact: {
          artifact_type: "EQUITY_CURVE",
          download_path: `/api/v1/artifacts/${BASELINE_RUN_ID}/download`,
          id: "00000000-0000-4000-8000-000000000501",
          row_count: 2,
          run_id: BASELINE_RUN_ID,
          sha256: "sha256:equity",
          size_bytes: 256,
        },
        run_id: BASELINE_RUN_ID,
        summary: {
          drawdown_curve: [
            { date: "2025-11-28", value: "-0.1842" },
            { date: "2025-12-31", value: "-0.0821" },
          ],
          equity_curve: [
            { date: "2025-11-28", value: "119000000.00" },
            { date: "2025-12-31", value: "128450000.00" },
          ],
          monthly_returns: [
            { month: "2025-11", value: "-0.0321" },
            { month: "2025-12", value: "0.0794" },
          ],
        },
      },
      status: 200,
    };
  }
  if (method === "GET" && pathname === `/api/v1/backtests/${BASELINE_RUN_ID}/trades`) {
    const items = trades(scenario);
    return {
      body: { has_more: false, items, next_cursor: null, total_count: items.length },
      status: 200,
    };
  }
  return null;
}
