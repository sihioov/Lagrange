import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import StockBetaDetailPage from "@/app/(authenticated)/stock-beta/[instrument]/page";
import StockBetaPage from "@/app/(authenticated)/stock-beta/page";
import { type ApiSession, apiErrorEnvelopeSchema } from "@/lib/api/contracts";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import {
  type OwnerBetaEquitySignalsSearchParams,
  ownerBetaEquitySignalsLatestSchema,
  ownerBetaEquitySignalsScreenSchema,
} from "@/lib/products/equity-signals-contracts";

const mocks = vi.hoisted(() => ({
  getLocale: vi.fn(async (): Promise<"en" | "ko"> => "en"),
  getProductApi: vi.fn(),
  getServerSession: vi.fn(),
  redirect: vi.fn((destination: string) => {
    throw Object.assign(new Error("NEXT_REDIRECT"), { destination });
  }),
}));

vi.mock("server-only", () => ({}));
vi.mock("@/lib/api/server-products", () => ({ getProductApi: mocks.getProductApi }));
vi.mock("@/lib/api/server-session", () => ({ getServerSession: mocks.getServerSession }));
vi.mock("@/lib/i18n/server", () => ({ getLocale: mocks.getLocale }));
vi.mock("next/navigation", () => ({
  redirect: mocks.redirect,
  usePathname: () => "/stock-beta",
  useRouter: () => ({ refresh: () => undefined }),
}));

const OWNER_SESSION = {
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "disabled",
  owner_beta_paper_mode: "disabled",
  role: "owner",
  user_id: "00000000-0000-4000-8000-000000000001",
} as const satisfies ApiSession;

const MEMBER_SESSION = {
  ...OWNER_SESSION,
  role: "member",
  user_id: "00000000-0000-4000-8000-000000000002",
} as const satisfies ApiSession;

const SHA = `sha256:${"a".repeat(64)}`;
const IDS = Array.from({ length: 30 }, (_, index) => `${String(index + 1).padStart(6, "0")}.KRX`);
const provenance = {
  activity_proxy: "20-session average trading value",
  artifact_content_sha256: SHA,
  as_of: "2026-08-29",
  audience: "OWNER_ONLY",
  batch_id: "batch-stock-beta-1",
  capability: "PRICE_VOLUME_RESEARCH_ONLY",
  entitlement_sha256: SHA,
  factor_version: "stock-price-beta-v1",
  index_membership: "not an index membership universe",
  materialization_status: "MATERIALIZED",
  original_price: true,
  publication_status: "OWNER_ONLY_API",
  redistribution: "restricted",
  registration_status: "registered",
  registry_sha256: SHA,
  selection_basis: "configured fixed observation list",
  snapshot_content_sha256: SHA,
  strict_pit: false,
  universe_sha256: SHA,
  vendor_snapshot: true,
  warning: "Corporate actions can distort returns and drawdowns.",
} as const;

function row(index: number) {
  return {
    average_trading_value_20: 1_000_000 + index,
    average_volume_20: 100_000 + index,
    condition: index % 3 === 0 ? "BULLISH" : index % 3 === 1 ? "NEUTRAL" : "BEARISH",
    instrument_id: IDS[index] as string,
    instrument_name: `Instrument ${index + 1}`,
    max_drawdown_120: -0.1 - index / 1_000,
    rank: index + 1,
    return_120: 0.4 - index / 100,
    return_20: 0.1 - index / 1_000,
    return_60: 0.2 - index / 1_000,
    score: 100 - index,
    sma_20: 100 + index,
    sma_60: 99 + index,
    volatility_120: 0.2 + index / 1_000,
    volatility_20: 0.1 + index / 1_000,
    volatility_60: 0.15 + index / 1_000,
    volume_ratio_20_60: 1.2 + index / 1_000,
  } as const;
}

const rows = IDS.map((_, index) => row(index));
const latest = ownerBetaEquitySignalsLatestSchema.parse({
  provenance,
  rows,
  top5: rows.slice(0, 5),
});
const screened = ownerBetaEquitySignalsScreenSchema.parse({
  provenance,
  rows: rows.slice(0, 2),
});
const detail = {
  condition_reasons: [
    "return_20 is at least 0.050000",
    "trend_up is true",
    "volatility_120 is at most 0.350000",
  ],
  factor_explanations: [
    { factor: "return_20", interpretation: "20-session price return", value: 0.1 },
    { factor: "return_60", interpretation: "60-session price return", value: 0.2 },
    { factor: "return_120", interpretation: "120-session price return", value: 0.4 },
    {
      factor: "volatility_120",
      interpretation: "120-session annualized volatility",
      value: 0.2,
    },
    {
      factor: "max_drawdown_120",
      interpretation: "120-session maximum drawdown",
      value: -0.1,
    },
    {
      factor: "average_trading_value_20",
      interpretation: "20-session activity proxy, not execution liquidity",
      value: 1_000_000,
    },
    { factor: "trend", interpretation: "sma20 minus sma60", value: 1 },
  ],
  provenance,
  signal: rows[0],
};

function apiFor(overrides: Record<string, unknown> = {}) {
  return {
    getOwnerBetaEquitySignalDetail: vi.fn(async () => detail),
    getOwnerBetaEquitySignalsLatest: vi.fn(async () => latest),
    screenOwnerBetaEquitySignals: vi.fn(async () => screened),
    ...overrides,
  };
}

function session(value: ApiSession = OWNER_SESSION): void {
  mocks.getServerSession.mockResolvedValue(value);
}

function renderedRowIds(markup: string): string[] {
  return [...markup.matchAll(/data-testid="stock-beta-row-([^"]+)"/g)].flatMap((match) =>
    match[1] === undefined ? [] : [match[1]],
  );
}

function renderedTapeIds(markup: string): string[] {
  return [...markup.matchAll(/data-testid="stock-beta-tape-([^"]+)"/g)].flatMap((match) =>
    match[1] === undefined ? [] : [match[1]],
  );
}

function loginError(code: "SESSION_EXPIRED" | "SESSION_UNKNOWN"): ApiProblem {
  return new ApiProblem(
    401,
    apiErrorEnvelopeSchema.parse({
      error: { code, message: "static auth failure", request_id: "request-test" },
    }),
  );
}

afterEach(() => {
  vi.clearAllMocks();
  mocks.getLocale.mockResolvedValue("en");
});

describe("Owner-beta stock signal workspace", () => {
  it("renders Top 5 emphasis inside the complete ranked table for the latest view", async () => {
    session();
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain("Top 5");
    expect(markup).toContain('data-testid="stock-beta-top-five"');
    expect(markup.match(/data-top-five="true"/g)).toHaveLength(5);
    expect(markup).toContain('data-testid="stock-beta-rank-table"');
    const tableBody = markup.slice(markup.indexOf("<tbody>"), markup.indexOf("</tbody>") + 8);
    expect(tableBody.match(/<tr\b/g)).toHaveLength(30);
    expect(renderedRowIds(markup)).toEqual(IDS);
    expect(renderedTapeIds(markup)).toEqual(IDS.slice(0, 5));
    expect(markup).toContain(
      'data-testid="stock-beta-snapshot-bullish"><dt>BULLISH</dt><dd><data value="10">10',
    );
    expect(markup).toContain(
      'data-testid="stock-beta-snapshot-neutral"><dt>NEUTRAL</dt><dd><data value="10">10',
    );
    expect(markup).toContain(
      'data-testid="stock-beta-snapshot-bearish"><dt>BEARISH</dt><dd><data value="10">10',
    );
    expect(markup).toContain("Instrument 30");
    expect(markup).toContain('data-testid="stock-beta-signal-preview"');
    expect(markup).toContain('data-selected="true"');
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain("20-session moving average");
    expect(markup).toContain("20-session average volume");
    expect(markup).toContain("20/60 volume ratio");
    expect(api.getOwnerBetaEquitySignalsLatest).toHaveBeenCalledOnce();
    expect(api.screenOwnerBetaEquitySignals).not.toHaveBeenCalled();
  });

  it("maps GET filters to the exact screen body and keeps the URL form server-owned", async () => {
    session();
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaPage({
        searchParams: Promise.resolve({
          activity_min: "1000",
          condition: ["BULLISH", "BEARISH"],
          max_drawdown_120_max: "-0.1",
          return_20_max: "0.2",
          return_20_min: "0.1",
          trend: "up",
          volatility_60_min: "0.3",
        }),
      }),
    );

    expect(api.getOwnerBetaEquitySignalsLatest).not.toHaveBeenCalled();
    expect(api.screenOwnerBetaEquitySignals).toHaveBeenCalledWith({
      condition: ["BULLISH", "BEARISH"],
      conditions: {
        average_trading_value_20: { min: 1000 },
        max_drawdown_120: { max: -0.1 },
        return_20: { max: 0.2, min: 0.1 },
        trend_up: true,
        volatility_60: { min: 0.3 },
      },
    });
    expect(markup).toContain('method="get"');
    expect(markup).toContain('action="/stock-beta"');
    expect(markup).toContain("Current results");
    expect(markup).toContain('data-testid="stock-beta-condition-distribution"');
    expect(markup).toContain('data-testid="stock-beta-active-filters"');
    expect(renderedRowIds(markup)).toEqual(IDS.slice(0, 2));
    expect(renderedTapeIds(markup)).toEqual(IDS.slice(0, 2));
    expect(markup).toContain(
      'data-testid="stock-beta-snapshot-bullish"><dt>BULLISH</dt><dd><data value="1">1',
    );
    expect(markup).toContain(
      'data-testid="stock-beta-snapshot-neutral"><dt>NEUTRAL</dt><dd><data value="1">1',
    );
    expect(markup).toContain(
      'data-testid="stock-beta-snapshot-bearish"><dt>BEARISH</dt><dd><data value="0">0',
    );
    expect(markup).toContain('name="return_20_min"');
    expect(markup).toContain('name="average_trading_value_20_min"');
    expect(markup).toContain(
      "The count and condition distribution describe only the rows returned",
    );
  });

  it("renders every factor, exact condition reason, rank, condition, and provenance", async () => {
    session();
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({
        params: Promise.resolve({ instrument: "000001.KRX" }),
      }),
    );

    for (const factor of detail.factor_explanations) {
      expect(markup).toContain(factor.factor);
      expect(markup).toContain(factor.interpretation);
      expect(markup).toContain(String(factor.value));
    }
    for (const reason of detail.condition_reasons) expect(markup).toContain(reason);
    expect(markup).toContain("<dt>Rank</dt><dd>1</dd>");
    expect(markup).toContain("BULLISH");
    expect(markup).toContain("OWNER_ONLY");
    expect(markup).toContain(provenance.batch_id);
    expect(markup).toContain(provenance.registry_sha256);
    expect(markup).toContain(provenance.snapshot_content_sha256);
    expect(markup).toContain('data-testid="stock-beta-detail-returns"');
    expect(markup).toContain('data-testid="stock-beta-detail-risk"');
    expect(markup).toContain('data-testid="stock-beta-detail-activity"');
    expect(markup).toContain('data-testid="stock-beta-factor-table"');
    expect(markup).toContain('data-testid="stock-beta-condition-reasons"');
    expect(markup).toContain('data-testid="stock-beta-provenance"');
    expect(markup).toContain('data-testid="stock-beta-terminal-page"');
    expect(markup).toContain('data-testid="stock-beta-detail-context-bar"');
    expect(markup).toContain('data-testid="stock-beta-detail-strip"');
    expect(markup).not.toContain('class="route-page');
    expect(markup).not.toContain("Result count");
    expect(markup).not.toContain("Condition distribution");

    const factorTable = markup.slice(
      markup.indexOf('data-testid="stock-beta-factor-table"'),
      markup.indexOf("</table>") + "</table>".length,
    );
    const factorRows = detail.factor_explanations.map((factor) => {
      const rawValue = String(factor.value);
      return `<tr data-raw-value="${rawValue}"><th scope="row">${factor.factor}</th><td>${factor.interpretation}</td><td><data value="${rawValue}">${rawValue}</data></td></tr>`;
    });
    const factorPositions = factorRows.map((row) => factorTable.indexOf(row));
    expect(factorPositions.every((position) => position >= 0)).toBe(true);
    expect(factorPositions).toEqual([...factorPositions].sort((left, right) => left - right));

    const reasonList = markup.slice(
      markup.indexOf('data-testid="stock-beta-condition-reasons"'),
      markup.indexOf("</ol>") + "</ol>".length,
    );
    const renderedReasons = [...reasonList.matchAll(/<li>([^<]*)<\/li>/g)].map((match) => match[1]);
    expect(renderedReasons).toEqual(detail.condition_reasons);

    for (const value of Object.values(provenance)) {
      if (typeof value === "string") expect(markup).toContain(value);
    }
  });

  it("reconstructs a detail back link from approved filters and keeps the detail request unchanged", async () => {
    session();
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({
        params: Promise.resolve({ instrument: "000001.KRX" }),
        searchParams: Promise.resolve({
          condition: ["BULLISH", "BEARISH"],
          return_20_max: "0.2",
          return_20_min: "0.1",
          trend: "up",
        }),
      }),
    );

    expect(markup).toContain(
      'href="/stock-beta?condition=BULLISH&amp;condition=BEARISH&amp;return_20_min=0.1&amp;return_20_max=0.2&amp;trend=up"',
    );
    expect(api.getOwnerBetaEquitySignalDetail).toHaveBeenCalledOnce();
    expect(api.getOwnerBetaEquitySignalDetail).toHaveBeenCalledWith("000001.KRX");
  });

  it("drops invalid detail query context without changing the detail API request", async () => {
    session();
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({
        params: Promise.resolve({ instrument: "000001.KRX" }),
        searchParams: Promise.resolve({ return_to: "https://evil.example/elsewhere" }),
      }),
    );

    expect(markup).toContain('href="/stock-beta">Back to stock signal beta</a>');
    expect(markup).not.toContain("return_to");
    expect(markup).not.toContain("evil.example");
    expect(api.getOwnerBetaEquitySignalDetail).toHaveBeenCalledOnce();
    expect(api.getOwnerBetaEquitySignalDetail).toHaveBeenCalledWith("000001.KRX");
  });

  it("blocks a Member detail visit before resolving context or constructing the product client", async () => {
    session(MEMBER_SESSION);
    const searchParams = new Promise<OwnerBetaEquitySignalsSearchParams>(() => undefined);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("a refused Member must not construct the product client");
    });

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({
        params: Promise.resolve({ instrument: "000001.KRX" }),
        searchParams,
      }),
    );

    expect(markup).toContain("Owner access required");
    expect(markup).not.toContain("Instrument 1");
    expect(mocks.getProductApi).not.toHaveBeenCalled();
  });

  it.each([
    ["OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE", "Signal data unavailable", 503],
    ["OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED", "Signal snapshot integrity failed", 503],
    ["RESOURCE_NOT_FOUND", "Instrument signal not found", 404],
  ] as const)("renders the detail fail-closed state for %s", async (code, title, status) => {
    session();
    const api = apiFor({
      getOwnerBetaEquitySignalDetail: vi.fn(async () => {
        throw new ApiProblem(
          status,
          apiErrorEnvelopeSchema.parse({
            error: { code, message: "static detail failure", request_id: "request-test" },
          }),
        );
      }),
    });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({
        params: Promise.resolve({ instrument: "000001.KRX" }),
      }),
    );

    expect(markup).toContain(title);
    expect(markup).not.toContain('data-testid="stock-beta-factor-table"');
    expect(markup).not.toContain('data-testid="stock-beta-detail-context-bar"');
    expect(markup).not.toContain('data-testid="stock-beta-detail-strip"');
    expect(markup).not.toContain('data-testid="stock-beta-detail-returns"');
    expect(markup).toContain('href="/stock-beta">Back to stock signal beta</a>');
  });

  it("blocks a Member direct visit before constructing the product client or rendering rows", async () => {
    session(MEMBER_SESSION);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("a refused Member must not construct the product client");
    });

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain("Owner access required");
    expect(markup).not.toContain("Instrument 1");
    expect(markup).not.toContain("stock-beta-rank-table");
    expect(mocks.getProductApi).not.toHaveBeenCalled();
  });

  it.each(["SESSION_EXPIRED", "SESSION_UNKNOWN"] as const)(
    "redirects after the initial session load when the latest product request returns %s",
    async (code) => {
      session();
      const api = apiFor({
        getOwnerBetaEquitySignalsLatest: vi.fn(async () => {
          throw loginError(code);
        }),
      });
      mocks.getProductApi.mockResolvedValue(api);

      await expect(StockBetaPage()).rejects.toMatchObject({
        destination: "/login",
        message: "NEXT_REDIRECT",
      });
      expect(mocks.getServerSession).toHaveBeenCalledOnce();
      expect(mocks.redirect).toHaveBeenCalledWith("/login");
    },
  );

  it.each(["SESSION_EXPIRED", "SESSION_UNKNOWN"] as const)(
    "redirects after the initial session load when the detail product request returns %s",
    async (code) => {
      session();
      const api = apiFor({
        getOwnerBetaEquitySignalDetail: vi.fn(async () => {
          throw loginError(code);
        }),
      });
      mocks.getProductApi.mockResolvedValue(api);

      await expect(
        StockBetaDetailPage({ params: Promise.resolve({ instrument: "000001.KRX" }) }),
      ).rejects.toMatchObject({ destination: "/login", message: "NEXT_REDIRECT" });
      expect(mocks.redirect).toHaveBeenCalledWith("/login");
    },
  );

  it.each(["SESSION_EXPIRED", "SESSION_UNKNOWN"] as const)(
    "redirects after the initial session load when the screen product request returns %s",
    async (code) => {
      session();
      const api = apiFor({
        screenOwnerBetaEquitySignals: vi.fn(async () => {
          throw loginError(code);
        }),
      });
      mocks.getProductApi.mockResolvedValue(api);

      await expect(
        StockBetaPage({ searchParams: Promise.resolve({ condition: "BULLISH" }) }),
      ).rejects.toMatchObject({ destination: "/login", message: "NEXT_REDIRECT" });
      expect(mocks.redirect).toHaveBeenCalledWith("/login");
    },
  );

  it.each([
    ["OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE", "Signal data unavailable"],
    ["OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED", "Signal snapshot integrity failed"],
  ] as const)("renders no data for %s", async (code, title) => {
    session();
    const api = apiFor({
      getOwnerBetaEquitySignalsLatest: vi.fn(async () => {
        throw new ApiProblem(
          503,
          apiErrorEnvelopeSchema.parse({
            error: { code, message: "static product failure", request_id: "request-test" },
          }),
        );
      }),
    });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain(title);
    expect(markup).not.toContain("Instrument 1");
    expect(markup).not.toContain("stock-beta-rank-table");
  });

  it("keeps the policy boundary prominent in both locales and avoids affirmative prohibited claims", async () => {
    session();
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const english = renderToStaticMarkup(await StockBetaPage());
    expect(english).toContain("Owner-only configured fixed observation list");
    expect(english).toContain("not current or historical index membership");
    expect(english).toContain("Original/unadjusted price");
    expect(english).toContain("not execution liquidity");
    expect(english).toContain(
      "not probabilities, target prices, buy/sell calls, weights, or orders",
    );

    mocks.getLocale.mockResolvedValue("ko");
    const korean = renderToStaticMarkup(await StockBetaPage());
    expect(korean).toContain("오너 전용으로 구성된 고정 관찰 목록");
    expect(korean).toContain("원주가(비조정 가격)");
    expect(korean).toContain("체결 유동성을 뜻하지 않습니다");
    expect(korean).toContain("확률, 목표가, 매수·매도 신호, 비중 또는 주문이 아닙니다");
  });

  it("does not advertise the stock beta to Members but does add it to Owner navigation", async () => {
    const { AppShell } = await import("@/components/shell/app-shell");
    const member = renderToStaticMarkup(<AppShell session={MEMBER_SESSION}>content</AppShell>);
    const owner = renderToStaticMarkup(<AppShell session={OWNER_SESSION}>content</AppShell>);

    expect(member).not.toContain('href="/stock-beta"');
    expect(owner).toContain('href="/stock-beta"');
  });

  it("keeps the shared auth predicate focused on login-required errors", () => {
    expect(isLoginRequiredError(loginError("SESSION_EXPIRED"))).toBe(true);
  });
});
