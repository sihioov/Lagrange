import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import OwnerBetaPriceOnlySimulationPage from "@/app/(authenticated)/backtests/owner-beta/price-only/page";
import BacktestsPage from "@/app/(authenticated)/backtests/page";
import PaperPage from "@/app/(authenticated)/paper/page";
import RecommendationsPage from "@/app/(authenticated)/recommendations/page";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import type { ApiSession } from "@/lib/api/contracts";

const mocks = vi.hoisted(() => ({
  getProductApi: vi.fn(),
  getServerSession: vi.fn(),
}));

vi.mock("server-only", () => ({}));
vi.mock("@/lib/api/server-products", () => ({ getProductApi: mocks.getProductApi }));
vi.mock("@/lib/api/server-session", () => ({ getServerSession: mocks.getServerSession }));
vi.mock("@/lib/i18n/server", () => ({ getLocale: async () => "en" }));

const MEMBER_OWNER_ONLY = {
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "owner_only",
  owner_beta_paper_mode: "disabled",
  role: "member",
  user_id: "00000000-0000-4000-8000-000000000002",
} as const satisfies ApiSession;

const OWNER_OWNER_ONLY = {
  ...MEMBER_OWNER_ONLY,
  role: "owner",
  user_id: "00000000-0000-4000-8000-000000000001",
} as const satisfies ApiSession;

const ACTIVE_BACKTEST_LICENSE = {
  as_of: "2026-08-28",
  datasets: [
    {
      covered: true,
      dataset_id: "approved-etf-price-history",
      state: "ACTIVE",
      use_kind: "backtest",
    },
  ],
} as const;

const ELIGIBLE_SIMULATION_CONFIG = {
  config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200 },
  created_at: "2026-08-19T06:00:00Z",
  id: "00000000-0000-4000-8000-000000000801",
  is_active: true,
  strategy_id: "trend_following",
  strategy_version: "1.0.0",
  updated_at: "2026-08-19T06:00:00Z",
} as const;

const SIMULATION_DEFAULT_CONFIGS = [
  {
    config: { benchmark_instrument: "069500.KRX", rebalance_cadence: "none", target_weight: 1 },
    id: "00000000-0000-4000-8000-000000000801",
    strategy_id: "buy_and_hold",
  },
  {
    config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200 },
    id: "00000000-0000-4000-8000-000000000802",
    strategy_id: "trend_following",
  },
  {
    config: { lookback_months: 12, top_n: 3 },
    id: "00000000-0000-4000-8000-000000000803",
    strategy_id: "relative_momentum",
  },
  {
    config: { absolute_threshold: 0, lookback_months: 12 },
    id: "00000000-0000-4000-8000-000000000804",
    strategy_id: "dual_momentum",
  },
  {
    config: { max_weight: 0.3, vol_window: 60 },
    id: "00000000-0000-4000-8000-000000000805",
    strategy_id: "inverse_volatility",
  },
].map((config) => ({
  ...config,
  created_at: "2026-08-19T06:00:00Z",
  is_active: true,
  strategy_version: "1.0.0",
  updated_at: "2026-08-19T06:00:00Z",
}));

afterEach(() => {
  vi.clearAllMocks();
});

describe("owner-beta product page boundary", () => {
  it("blocks all three direct Member pages before a product client is created", async () => {
    mocks.getServerSession.mockResolvedValue(MEMBER_OWNER_ONLY);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("product API must not be constructed for a refused Member");
    });

    for (const page of [() => RecommendationsPage({}), BacktestsPage, () => PaperPage({})]) {
      const markup = renderToStaticMarkup(await page());
      expect(markup).toContain("Owner access required");
      expect(markup).toContain("This area is restricted to the Owner.");
      expect(markup).toContain('href="/"');
      expect(markup).toContain('role="alert"');
      expect(markup).toContain('aria-live="assertive"');
    }

    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(mocks.getServerSession).toHaveBeenCalledTimes(3);
  });

  it("blocks the dedicated simulation route for Members before creating a product client", async () => {
    mocks.getServerSession.mockResolvedValue(MEMBER_OWNER_ONLY);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("a refused Member must not construct the product API");
    });

    const markup = renderToStaticMarkup(await OwnerBetaPriceOnlySimulationPage());

    expect(markup).toContain("Owner access required");
    expect(markup).toContain("Backtests");
    expect(markup).not.toContain("Owner-only price-only simulation");
    expect(markup).not.toContain("OWNER_ONLY");
    expect(mocks.getProductApi).not.toHaveBeenCalled();
  });

  it("keeps the direct simulation route lazy unless the exact owner-only mode is active", async () => {
    mocks.getServerSession.mockResolvedValue({
      ...OWNER_OWNER_ONLY,
      owner_beta_access_mode: "disabled",
    });
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("disabled owner mode must not construct the product API");
    });

    const markup = renderToStaticMarkup(await OwnerBetaPriceOnlySimulationPage());

    expect(markup).toContain("Owner-only mode is required");
    expect(markup).toContain("OWNER_ONLY");
    expect(markup).toContain("PRICE_RETURN_ONLY");
    expect(mocks.getProductApi).not.toHaveBeenCalled();
  });

  it("checks Backtest data rights before loading strategy configurations", async () => {
    const getStrategyConfigs = vi.fn();
    const getLicensingStatus = vi.fn().mockResolvedValue({
      ...ACTIVE_BACKTEST_LICENSE,
      datasets: ACTIVE_BACKTEST_LICENSE.datasets.map((dataset) => ({
        ...dataset,
        covered: false,
        state: "REVOKED",
      })),
    });
    mocks.getServerSession.mockResolvedValue(OWNER_OWNER_ONLY);
    mocks.getProductApi.mockResolvedValue({ getLicensingStatus, getStrategyConfigs });

    const markup = renderToStaticMarkup(await OwnerBetaPriceOnlySimulationPage());

    expect(markup).toContain("Simulation data is blocked");
    expect(markup).toContain("OWNER_ONLY");
    expect(getLicensingStatus).toHaveBeenCalledTimes(1);
    expect(getStrategyConfigs).not.toHaveBeenCalled();
  });

  it("loads only eligible existing configurations after both gates pass", async () => {
    const getLicensingStatus = vi.fn().mockResolvedValue(ACTIVE_BACKTEST_LICENSE);
    const getStrategyConfigs = vi.fn().mockResolvedValue({
      has_more: false,
      items: [
        ...SIMULATION_DEFAULT_CONFIGS,
        {
          ...ELIGIBLE_SIMULATION_CONFIG,
          id: "00000000-0000-4000-8000-000000000806",
          config: { benchmark_instrument: "069500.KRX", fast_ma: 50, slow_ma: 200, custom: true },
        },
      ],
      next_cursor: null,
    });
    mocks.getServerSession.mockResolvedValue(OWNER_OWNER_ONLY);
    mocks.getProductApi.mockResolvedValue({ getLicensingStatus, getStrategyConfigs });

    const markup = renderToStaticMarkup(await OwnerBetaPriceOnlySimulationPage());

    expect(markup).toContain("Queue a simulation");
    expect(markup).toContain("Trend following · 50/200");
    expect(markup).toContain("Buy and hold · 069500");
    expect(markup).toContain("Relative momentum · 12m / top 3");
    expect(markup).toContain("Dual momentum · 12m");
    expect(markup).toContain("Inverse volatility · 60d");
    expect(markup).not.toContain("00000000-0000-4000-8000-000000000806");
    expect((markup.match(/ · saved 2026-08-19/g) ?? []).length).toBe(5);
    expect(getLicensingStatus).toHaveBeenCalledTimes(1);
    expect(getStrategyConfigs).toHaveBeenCalledTimes(1);
  });

  it("shows the dedicated entry link only for exact owner-only access", async () => {
    const getBacktestRuns = vi
      .fn()
      .mockResolvedValue({ items: [], has_more: false, next_cursor: null });
    const getLicensingStatus = vi.fn().mockResolvedValue(ACTIVE_BACKTEST_LICENSE);
    mocks.getProductApi.mockResolvedValue({ getLicensingStatus, getBacktestRuns });

    mocks.getServerSession.mockResolvedValueOnce(OWNER_OWNER_ONLY);
    const ownerMarkup = renderToStaticMarkup(await BacktestsPage());
    expect(ownerMarkup).toContain('href="/backtests/owner-beta/price-only"');

    mocks.getServerSession.mockResolvedValueOnce({
      ...OWNER_OWNER_ONLY,
      owner_beta_access_mode: "disabled",
    });
    const disabledMarkup = renderToStaticMarkup(await BacktestsPage());
    expect(disabledMarkup).not.toContain('href="/backtests/owner-beta/price-only"');
  });

  it("keeps the direct Paper page lazy for an Owner while Paper is disabled", async () => {
    mocks.getServerSession.mockResolvedValue(OWNER_OWNER_ONLY);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("locked Paper must not construct its product API");
    });

    const markup = renderToStaticMarkup(await PaperPage({}));

    expect(markup).toContain("Paper beta is not enabled");
    expect(markup).toContain(
      "Paper remains unavailable until the separate beta readiness check is complete.",
    );
    expect(markup).not.toContain("Member access");
    expect(markup).toContain('role="alert"');
    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(mocks.getServerSession).toHaveBeenCalledTimes(1);
  });

  it("renders lazily for the Owner and for the normal disabled mode", async () => {
    const renderProduct = vi.fn(() => <p>product content</p>);

    mocks.getServerSession
      .mockResolvedValueOnce(OWNER_OWNER_ONLY)
      .mockResolvedValueOnce({
        ...MEMBER_OWNER_ONLY,
        owner_beta_access_mode: "disabled",
      })
      .mockResolvedValueOnce({
        ...OWNER_OWNER_ONLY,
        owner_beta_paper_mode: "enabled",
      });

    const owner = await OwnerBetaProductRoute({
      product: "recommendations",
      renderProduct,
      title: "Protected product",
    });
    const normal = await OwnerBetaProductRoute({
      product: "paper",
      renderProduct,
      title: "Normal product",
    });
    const paper = await OwnerBetaProductRoute({
      product: "paper",
      renderProduct,
      title: "Enabled Paper",
    });

    expect(renderToStaticMarkup(owner)).toContain("product content");
    expect(renderToStaticMarkup(normal)).toContain("product content");
    expect(renderToStaticMarkup(paper)).toContain("product content");
    expect(renderProduct).toHaveBeenCalledTimes(3);
  });
});
