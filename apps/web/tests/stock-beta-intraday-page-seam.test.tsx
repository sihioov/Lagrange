import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import StockBetaDetailPage from "@/app/(authenticated)/stock-beta/[instrument]/page";
import StockBetaPage from "@/app/(authenticated)/stock-beta/page";
import type { IntradayStreamProvider } from "@/components/stock-beta/quote/intraday-stream-provider";
import { type ApiSession, apiErrorEnvelopeSchema } from "@/lib/api/contracts";
import { ApiProblem } from "@/lib/api/response";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import {
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2SignalDetailSchema,
  ownerEquityV2SignalSchema,
} from "@/lib/products/equity-signals-contracts";
import { streamMembership } from "./fixtures/market-stream";
import { renderAsyncPage } from "./support/render-async-page";

const mocks = vi.hoisted(() => ({
  getLocale: vi.fn(async (): Promise<"en" | "ko"> => "en"),
  getProductApi: vi.fn(),
  getServerSession: vi.fn(),
  streamProviderProps: vi.fn<(props: ComponentProps<typeof IntradayStreamProvider>) => void>(),
  redirect: vi.fn((destination: string) => {
    throw Object.assign(new Error("NEXT_REDIRECT"), { destination });
  }),
}));

vi.mock("server-only", () => ({}));
vi.mock("@/lib/api/server-products", () => ({ getProductApi: mocks.getProductApi }));
vi.mock("@/lib/api/server-session", () => ({ getServerSession: mocks.getServerSession }));
vi.mock("@/lib/i18n/server", () => ({ getLocale: mocks.getLocale }));
vi.mock("@/components/stock-beta/quote/intraday-stream-provider", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/components/stock-beta/quote/intraday-stream-provider")>();
  return {
    ...actual,
    IntradayStreamProvider: (props: ComponentProps<typeof actual.IntradayStreamProvider>) => {
      mocks.streamProviderProps(props);
      return <actual.IntradayStreamProvider {...props} />;
    },
  };
});
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
const SIGNAL = ownerEquityV2SignalSchema.parse({
  average_trading_value_20: 1_000_000,
  average_volume_20: 25_000,
  condition: "BULLISH",
  generation: 7,
  instrument_id: "069500.KRX",
  max_drawdown_120: -0.3,
  rank: 1,
  return_120: 0.4,
  return_20: 0.1,
  return_60: 0.2,
  score: 1.2,
  sma_20: 101,
  sma_60: 99,
  volatility_120: 0.3,
  volatility_20: 0.1,
  volatility_60: 0.2,
  volume_ratio_20_60: 1.1,
});
const SNAPSHOT = {
  as_of: "2026-09-07",
  published_at: "2026-09-08T00:00:00Z",
  row_count: 1,
  snapshot_id: "00000000-0000-4000-8000-000000000010",
  universe_sha256: "sha256:fixture",
};
const SIGNALS = ownerEquityV2LatestSignalsSchema.parse({
  rows: [SIGNAL],
  snapshot: SNAPSHOT,
  top5: [SIGNAL],
});
const DETAIL = ownerEquityV2SignalDetailSchema.parse({ signal: SIGNAL, snapshot: SNAPSHOT });
const MEMBERSHIPS = ownerEquityV2MembershipListSchema.parse({
  memberships: [
    {
      coverage: {
        first_session: "2025-01-01",
        last_session: "2026-09-07",
        minimum_observed_sessions: 121,
        observed_sessions: 261,
        target_observed_sessions: 261,
      },
      generation: 7,
      id: "00000000-0000-4000-8000-000000000020",
      instrument_id: "069500.KRX",
      lifecycle: "READY",
      requested_at: "2026-09-01T00:00:00Z",
      updated_at: "2026-09-08T00:00:00Z",
    },
  ],
  policy: {
    active_instruments: 1,
    max_active_instruments: 100,
    minimum_observed_sessions: 121,
    remaining_capacity: 99,
    target_observed_sessions: 261,
  },
});

function apiFor(membershipsError?: unknown) {
  return {
    getOwnerEquityV2LatestSignals: vi.fn(async () => SIGNALS),
    getOwnerEquityV2Memberships: vi.fn(async () => {
      if (membershipsError !== undefined) throw membershipsError;
      return MEMBERSHIPS;
    }),
    getOwnerEquityV2SignalDetail: vi.fn(async () => DETAIL),
  };
}

afterEach(() => {
  vi.unstubAllEnvs();
  vi.clearAllMocks();
  mocks.getLocale.mockResolvedValue("en");
});

describe("Stock Beta intraday page seams", () => {
  // Observe props at the real page -> detail/workspace -> provider boundary.
  // SSR drives rendering only; membership assertions do not inspect its HTML.
  it.each([0, 1])(
    "passes only selected READY membership %s to the detail provider",
    async (index) => {
      vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
      vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
      mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
      const memberships = [streamMembership(0), streamMembership(1)];
      const selected = streamMembership(index);
      const api = apiFor();
      api.getOwnerEquityV2Memberships.mockResolvedValue({ ...MEMBERSHIPS, memberships });
      api.getOwnerEquityV2SignalDetail.mockResolvedValue({
        ...DETAIL,
        signal: {
          ...SIGNAL,
          instrument_id: selected.instrument_id,
          generation: selected.generation,
        },
      });
      mocks.getProductApi.mockResolvedValue(api);

      await renderAsyncPage(
        await StockBetaDetailPage({
          params: Promise.resolve({ instrument: selected.instrument_id }),
        }),
      );

      expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({
          enabled: true,
          sessionKey: `${OWNER_SESSION.user_id}:${OWNER_SESSION.expires_at_secs}`,
          memberships: [selected],
        }),
      );
      expect(api.getOwnerEquityV2SignalDetail).toHaveBeenCalledExactlyOnceWith(
        selected.instrument_id,
      );
      expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    },
  );

  it.each(["generation mismatch", "not READY"])(
    "passes no detail demand for %s even with another READY membership",
    async (reason) => {
      vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
      vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
      mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
      const selected = streamMembership(0);
      const api = apiFor();
      api.getOwnerEquityV2Memberships.mockResolvedValue({
        ...MEMBERSHIPS,
        memberships: [
          {
            ...selected,
            generation:
              reason === "generation mismatch" ? selected.generation + 1 : selected.generation,
            lifecycle: reason === "not READY" ? "BACKFILLING" : "READY",
          },
          streamMembership(1),
        ],
      });
      api.getOwnerEquityV2SignalDetail.mockResolvedValue({
        ...DETAIL,
        signal: {
          ...SIGNAL,
          instrument_id: selected.instrument_id,
          generation: selected.generation,
        },
      });
      mocks.getProductApi.mockResolvedValue(api);

      await renderAsyncPage(
        await StockBetaDetailPage({
          params: Promise.resolve({ instrument: selected.instrument_id }),
        }),
      );

      expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({ enabled: true, memberships: [] }),
      );
    },
  );

  it("preserves both READY memberships at the board provider", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const memberships = [streamMembership(0), streamMembership(1)];
    const api = apiFor();
    api.getOwnerEquityV2Memberships.mockResolvedValue({ ...MEMBERSHIPS, memberships });
    mocks.getProductApi.mockResolvedValue(api);

    await renderAsyncPage(await StockBetaPage());

    expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ enabled: true, memberships }),
    );
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
  });

  it("disables both providers when quotes are off while preserving the board server read", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "off");
    vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const memberships = [streamMembership(0), streamMembership(1)];
    const api = apiFor();
    api.getOwnerEquityV2Memberships.mockResolvedValue({ ...MEMBERSHIPS, memberships });
    mocks.getProductApi.mockResolvedValue(api);

    await renderAsyncPage(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );
    expect(api.getOwnerEquityV2Memberships).not.toHaveBeenCalled();
    expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ enabled: false, memberships: [] }),
    );
    mocks.streamProviderProps.mockClear();

    await renderAsyncPage(await StockBetaPage());
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ enabled: false, memberships }),
    );
  });

  it("shows the WS board and its selected READY instrument even without EOD signals", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    api.getOwnerEquityV2Memberships.mockResolvedValue({
      ...MEMBERSHIPS,
      memberships: [streamMembership(0)],
    });
    api.getOwnerEquityV2LatestSignals.mockRejectedValue(
      new ApiProblem(
        503,
        apiErrorEnvelopeSchema.parse({
          error: {
            code: "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE",
            message: "fixture unavailable",
            request_id: "stream-fixture",
          },
        }),
      ),
    );
    mocks.getProductApi.mockResolvedValue(api);
    const markup = await renderAsyncPage(await StockBetaPage());
    expect(markup.match(/data-stream-instrument=/g)).toHaveLength(30);
    expect(markup).toContain('data-testid="stock-beta-stream-selected"');
    expect(markup.replaceAll("<!-- -->", "")).toContain(`${streamMembership(0).instrument_id} ·`);
    expect(markup).not.toContain("periodic refresh");
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
  });

  it("uses the stream detail and never renders the REST widget in WS mode", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);
    const markup = await renderAsyncPage(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );
    expect(markup).toContain('data-testid="stock-beta-stream-selected"');
    expect(markup).not.toContain("periodic refresh");
    expect(markup).toContain("Returns");
  });

  it("refuses a Member before constructing any product client in WS mode", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
    mocks.getServerSession.mockResolvedValue({ ...OWNER_SESSION, role: "member" });
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("must remain lazy");
    });
    for (const page of [
      () => StockBetaPage(),
      () => StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    ]) {
      const markup = await renderAsyncPage(await page());
      expect(markup).toContain("Owner access required");
      expect(markup).not.toContain("stock-beta-stream");
    }
    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(mocks.streamProviderProps).not.toHaveBeenCalled();
  });

  it("is default-off and does not add the detail membership read", async () => {
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = await renderAsyncPage(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );

    expect(markup).not.toContain("장중 현재가");
    expect(markup).not.toContain("Intraday price");
    expect(api.getOwnerEquityV2Memberships).not.toHaveBeenCalled();
    expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ enabled: false, memberships: [] }),
    );
    expect(markup).toContain("Returns");
  });

  it("passes only an exact READY membership when enabled and tolerates a quote-only membership failure", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const detailMarkup = await renderAsyncPage(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    expect(detailMarkup).toContain('data-testid="stock-beta-stream-selected"');
    expect(detailMarkup).not.toContain("periodic refresh");
    expect(detailMarkup).toContain("Returns");

    const unavailableApi = apiFor(
      new ApiProblem(
        503,
        apiErrorEnvelopeSchema.parse({
          error: {
            code: "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND",
            message: "typed fixture error",
            request_id: "request-fixture",
          },
        }),
      ),
    );
    mocks.getProductApi.mockResolvedValue(unavailableApi);
    const failureMarkup = await renderAsyncPage(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );
    expect(failureMarkup).toContain("Returns");
    expect(failureMarkup).toContain('data-testid="stock-beta-stream-selected"');
    expect(failureMarkup).not.toContain("periodic refresh");

    const authFailureApi = apiFor(
      new ApiProblem(
        401,
        apiErrorEnvelopeSchema.parse({
          error: {
            code: "SESSION_EXPIRED",
            message: "session expired",
            request_id: "request-auth",
          },
        }),
      ),
    );
    mocks.getProductApi.mockResolvedValue(authFailureApi);
    await expect(
      StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    ).rejects.toMatchObject({ destination: "/login" });
  });

  it("keeps the dashboard READY membership lookup on the existing server read", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", "market_ws");
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = await renderAsyncPage(await StockBetaPage());
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    expect(markup).toContain('data-testid="stock-beta-stream-board"');
    expect(markup).not.toContain("periodic refresh");
    expect(markup).toContain(SIGNAL.instrument_id);
  });

  it.each([undefined, "rest"])(
    "never enables quote polling with transport %s",
    async (transport) => {
      vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
      vi.stubEnv("OWNER_INTRADAY_QUOTE_TRANSPORT", transport);
      mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
      const api = apiFor();
      mocks.getProductApi.mockResolvedValue(api);

      const detail = await renderAsyncPage(
        await StockBetaDetailPage({
          params: Promise.resolve({ instrument: SIGNAL.instrument_id }),
        }),
      );
      expect(api.getOwnerEquityV2Memberships).not.toHaveBeenCalled();
      expect(detail).not.toContain("periodic refresh");
      expect(detail).not.toContain("stock-beta-stream-selected");
      expect(detail).toContain("Returns");
      expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({ enabled: false, memberships: [] }),
      );
      mocks.streamProviderProps.mockClear();

      const dashboard = await renderAsyncPage(await StockBetaPage());
      expect(dashboard).not.toContain("periodic refresh");
      expect(dashboard).not.toContain("stock-beta-stream-board");
      expect(dashboard).toContain(SIGNAL.instrument_id);
      expect(dashboard).toContain('data-testid="stock-beta-widget-signal-profile"');
      expect(dashboard).toContain(stockBetaDictionary.en.chartPreparingMessage);
      expect(mocks.streamProviderProps).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({ enabled: false }),
      );
    },
  );
});
