import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BacktestsPage from "@/app/(authenticated)/backtests/page";

vi.mock("server-only", () => ({}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
}));

vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: () => ({ name: "__Host-lagrange_session", value: "member-opaque" }),
  }),
}));

const BASELINE_RUN_ID = "00000000-0000-4000-8000-000000000301";

function backtestRun() {
  return {
    benchmark: "069500.KRX",
    config_sha256: "sha256:backtest-config",
    created_at: "2026-01-31T07:00:00Z",
    dataset_version: "krx-eod@2025-12-31",
    end_date: "2025-12-31",
    engine: "NautilusTrader",
    engine_version: "nautilus@1.231.0",
    finished_at: "2026-01-31T07:10:00Z",
    id: BASELINE_RUN_ID,
    job_id: "00000000-0000-4000-8000-000000000401",
    start_date: "2020-01-02",
    started_at: "2026-01-31T07:01:00Z",
    status: "SUCCEEDED",
    strategy_id: "dual_momentum",
    strategy_version: "2.3.1",
    summary: {
      cost_profile_id: "krx-default@2026-01",
      data_version: "krx-eod@2025-12-31",
      dataset_version_id: "00000000-0000-4000-8000-000000000601",
      engine_version: "nautilus@1.231.0",
      execution_profile: "daily-close-next-open@1",
      run_label: "Dual momentum baseline",
      strategy_config_id: "00000000-0000-4000-8000-000000000101",
      strategy_version: "dual_momentum@2.3.1",
      warnings: ["Next-open execution can differ from close-to-close benchmarks."],
    },
  };
}

function syntheticBacktestApi(): typeof fetch {
  return async (input, init) => {
    const request = new Request(input, init);
    const { pathname } = new URL(request.url);
    if (pathname === "/api/v1/licensing-status") {
      return Response.json({
        as_of: "2026-01-31",
        datasets: [
          {
            covered: true,
            dataset_id: "krx-eod",
            effective_from: "2026-01-01",
            effective_until: "2026-12-31",
            state: "ACTIVE",
            use_kind: "backtest",
          },
        ],
      });
    }
    if (pathname === "/api/v1/backtests") {
      return Response.json({ has_more: false, items: [backtestRun()], next_cursor: null });
    }
    if (pathname === `/api/v1/backtests/${BASELINE_RUN_ID}/metrics`) {
      return Response.json({
        items: [
          { metric_key: "ending_equity", metric_value: "128450000.00" },
          { metric_key: "maximum_drawdown", metric_value: "-0.1842" },
          { metric_key: "total_cost", metric_value: "128450.00" },
        ],
      });
    }
    if (pathname === `/api/v1/backtests/${BASELINE_RUN_ID}/equity`) {
      return Response.json({
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
      });
    }
    if (pathname === `/api/v1/backtests/${BASELINE_RUN_ID}/trades`) {
      return Response.json({
        has_more: false,
        items: [
          {
            cost: "128450.00",
            executed_at: "2025-12-01T00:00:00Z",
            instrument_id: "069500.KRX",
            quantity: "120",
            side: "BUY",
            trade_id: "trade-1",
          },
        ],
        next_cursor: null,
        total_count: 1,
      });
    }
    return Response.json(
      {
        error: {
          code: "RESOURCE_NOT_FOUND",
          message: `No synthetic response for ${pathname}`,
          request_id: "request-component-backtest",
        },
      },
      { status: 404 },
    );
  };
}

beforeEach(() => {
  vi.stubEnv("API_INTERNAL_URL", "https://api.internal");
  vi.stubGlobal("fetch", syntheticBacktestApi());
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("backtest product surface", () => {
  it("renders server-produced results, provenance, and robustness controls", async () => {
    // Given
    const page = await BacktestsPage();

    // When
    const markup = renderToStaticMarkup(page);

    // Then
    expect(markup).toContain("Create backtest");
    expect(markup).toContain("Equity and drawdown");
    expect(markup).toContain("Monthly returns");
    expect(markup).toContain("Trades and costs");
    expect(markup).toContain("128,450,000.00");
    expect(markup).toContain("−18.42%");
    expect(markup).toContain("069500.KRX");
    expect(markup).toContain("dual_momentum@2.3.1");
    expect(markup).toContain("krx-eod@2025-12-31");
    expect(markup).toContain("nautilus@1.231.0");
    expect(markup).toContain("Next-open execution can differ");
    expect(markup).toContain("Run robustness evidence");
    expect(markup).not.toContain("calculateBacktest");
  });
});
