import { afterEach, describe, expect, it, vi } from "vitest";
import StockBetaPage from "@/app/(authenticated)/stock-beta/page";
import { type ApiSession, apiErrorEnvelopeSchema } from "@/lib/api/contracts";
import { ApiProblem } from "@/lib/api/response";
import {
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2SignalSchema,
} from "@/lib/products/equity-signals-contracts";
import { renderAsyncPage } from "./support/render-async-page";

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

const SIGNALS = ownerEquityV2LatestSignalsSchema.parse({
  rows: [SIGNAL],
  top5: [SIGNAL],
  snapshot: {
    as_of: "2026-09-07",
    published_at: "2026-09-08T00:00:00Z",
    row_count: 1,
    snapshot_id: "00000000-0000-4000-8000-000000000010",
    universe_sha256: "sha256:fixture",
  },
});

const MEMBERSHIPS = ownerEquityV2MembershipListSchema.parse({
  memberships: [],
  policy: {
    active_instruments: 0,
    max_active_instruments: 100,
    minimum_observed_sessions: 121,
    remaining_capacity: 100,
    target_observed_sessions: 261,
  },
});

function problem(code: string): ApiProblem {
  return new ApiProblem(
    503,
    apiErrorEnvelopeSchema.parse({
      error: { code, message: "private provider prose", request_id: "service-status-test" },
    }),
  );
}

function ownerApi() {
  return {
    getOwnerEquityV2Memberships: vi.fn(async () => MEMBERSHIPS),
    getOwnerEquityV2LatestSignals: vi.fn(async () => SIGNALS),
    getOwnerEquityV2Chart: vi.fn(async () => {
      throw new Error("private chart error");
    }),
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  mocks.getLocale.mockResolvedValue("en");
});

describe("Owner service status page placement", () => {
  it("keeps the Owner route lazy: a Member triggers neither status nor product work", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    mocks.getServerSession.mockResolvedValue(MEMBER_SESSION);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("product client must stay lazy");
    });

    const markup = await renderAsyncPage(StockBetaPage());

    expect(markup).toContain("Owner access required");
    expect(markup).not.toContain('data-testid="service-status-panel"');
    expect(markup).not.toContain('data-testid="stock-beta-dashboard"');
    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it.each([
    ["product entitlement", () => problem("OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE")],
    [
      "membership",
      () => {
        const api = ownerApi();
        api.getOwnerEquityV2Memberships.mockRejectedValue(problem("OWNER_EQUITY_INTEGRITY_FAILED"));
        return api;
      },
    ],
  ])(
    "keeps the authorized status panel visible after a %s failure",
    async (_kind, productResult) => {
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
      const result = productResult();
      if (result instanceof ApiProblem) mocks.getProductApi.mockRejectedValue(result);
      else mocks.getProductApi.mockResolvedValue(result);

      const markup = await renderAsyncPage(StockBetaPage());

      expect(markup).toContain('data-testid="service-status-panel"');
      expect(markup).not.toContain("private provider prose");
      expect(fetchMock).not.toHaveBeenCalled();
    },
  );

  it("keeps the authorized status panel visible when chart loading fails", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
    const api = ownerApi();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = await renderAsyncPage(StockBetaPage());

    expect(markup).toContain('data-testid="service-status-panel"');
    expect(markup).toContain('data-testid="stock-beta-dashboard"');
    expect(markup).not.toContain("private chart error");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
