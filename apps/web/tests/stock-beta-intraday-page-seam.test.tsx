import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import StockBetaDetailPage from "@/app/(authenticated)/stock-beta/[instrument]/page";
import StockBetaPage from "@/app/(authenticated)/stock-beta/page";
import { type ApiSession, apiErrorEnvelopeSchema } from "@/lib/api/contracts";
import { ApiProblem } from "@/lib/api/response";
import {
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2SignalDetailSchema,
  ownerEquityV2SignalSchema,
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
  it("is default-off and does not add the detail membership read", async () => {
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );

    expect(markup).not.toContain("장중 현재가");
    expect(markup).not.toContain("Intraday price");
    expect(api.getOwnerEquityV2Memberships).not.toHaveBeenCalled();
    expect(markup).toContain("Returns");
  });

  it("passes only an exact READY membership when enabled and tolerates a quote-only membership failure", async () => {
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const detailMarkup = renderToStaticMarkup(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    expect(detailMarkup).toContain("Intraday price · periodic refresh");
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
    const failureMarkup = renderToStaticMarkup(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: SIGNAL.instrument_id }) }),
    );
    expect(failureMarkup).toContain("Returns");
    expect(failureMarkup).toContain("Intraday price · periodic refresh");

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
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    expect(markup).toContain("Intraday price · periodic refresh");
    expect(markup).toContain(SIGNAL.instrument_id);
  });
});
