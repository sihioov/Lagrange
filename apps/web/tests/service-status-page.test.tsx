import { afterEach, describe, expect, it, vi } from "vitest";
import DashboardPage from "@/app/(authenticated)/page";
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

const OWNER_ACCESS_CASES = [
  {
    label: "disabled beta access",
    owner_beta_access_mode: "disabled",
    owner_beta_paper_mode: "disabled",
    cards: ["/strategies", "/recommendations", "/backtests", "/paper", "/stock-beta"],
  },
  {
    label: "owner-only beta access",
    owner_beta_access_mode: "owner_only",
    owner_beta_paper_mode: "disabled",
    cards: ["/strategies", "/recommendations", "/backtests", "/stock-beta"],
  },
  {
    label: "owner-only beta access with Paper enabled",
    owner_beta_access_mode: "owner_only",
    owner_beta_paper_mode: "enabled",
    cards: ["/strategies", "/recommendations", "/backtests", "/paper", "/stock-beta"],
  },
] as const satisfies readonly {
  readonly label: string;
  readonly owner_beta_access_mode: ApiSession["owner_beta_access_mode"];
  readonly owner_beta_paper_mode: ApiSession["owner_beta_paper_mode"];
  readonly cards: readonly string[];
}[];

const MEMBER_ACCESS_CASES = [
  {
    label: "disabled beta access",
    owner_beta_access_mode: "disabled",
    cards: ["/strategies", "/recommendations", "/backtests", "/paper"],
  },
  {
    label: "owner-only beta access",
    owner_beta_access_mode: "owner_only",
    cards: ["/strategies"],
  },
] as const satisfies readonly {
  readonly label: string;
  readonly owner_beta_access_mode: ApiSession["owner_beta_access_mode"];
  readonly cards: readonly string[];
}[];

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

function visibleCards(markup: string): string[] {
  const workspaceStart = markup.indexOf('class="surface-panel workspace-overview"');
  const workspaceEnd = workspaceStart < 0 ? -1 : markup.indexOf("</section>", workspaceStart);
  const workspaceMarkup =
    workspaceStart < 0 || workspaceEnd < 0 ? "" : markup.slice(workspaceStart, workspaceEnd);
  return ["/strategies", "/recommendations", "/backtests", "/paper", "/stock-beta"].filter((href) =>
    workspaceMarkup.includes(`href="${href}"`),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  mocks.getLocale.mockResolvedValue("en");
});

describe("Owner service status page placement", () => {
  it("renders above permitted workspace cards for Owners in both locales and beta settings", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("Dashboard must not request product data");
    });

    for (const locale of ["en", "ko"] as const) {
      for (const accessCase of OWNER_ACCESS_CASES) {
        mocks.getLocale.mockResolvedValue(locale);
        mocks.getServerSession.mockResolvedValue({
          ...OWNER_SESSION,
          owner_beta_access_mode: accessCase.owner_beta_access_mode,
          owner_beta_paper_mode: accessCase.owner_beta_paper_mode,
        });

        const markup = await renderAsyncPage(DashboardPage());
        const statusOffset = markup.indexOf('data-testid="service-status-panel"');
        const workspaceOffset = markup.indexOf("workspace-overview");

        expect(
          statusOffset,
          `${locale}, ${accessCase.label}: Owner status panel`,
        ).toBeGreaterThanOrEqual(0);
        expect(
          workspaceOffset,
          `${locale}, ${accessCase.label}: workspace follows status`,
        ).toBeGreaterThan(statusOffset);
        expect(
          visibleCards(markup),
          `${locale}, ${accessCase.label}: permitted workspace cards`,
        ).toEqual(accessCase.cards);
      }
    }

    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("keeps permitted workspace cards for Members without mounting or fetching status", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("Dashboard must not request product data");
    });

    for (const accessCase of MEMBER_ACCESS_CASES) {
      mocks.getServerSession.mockResolvedValue({
        ...MEMBER_SESSION,
        owner_beta_access_mode: accessCase.owner_beta_access_mode,
      });
      mocks.getProductApi.mockImplementation(() => {
        throw new Error("Dashboard must not request product data");
      });

      const markup = await renderAsyncPage(DashboardPage());

      expect(markup, `${accessCase.label}: no status panel`).not.toContain(
        'data-testid="service-status-panel"',
      );
      expect(visibleCards(markup), `${accessCase.label}: permitted workspace cards`).toEqual(
        accessCase.cards,
      );
    }

    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("keeps Stock Beta lazy for a Member and does not mount service status", async () => {
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

  it("never mounts service status on Stock Beta after product or chart failures", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const failures = [
      {
        label: "product entitlement",
        result: () => problem("OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE"),
      },
      {
        label: "membership",
        result: () => {
          const api = ownerApi();
          api.getOwnerEquityV2Memberships.mockRejectedValue(
            problem("OWNER_EQUITY_INTEGRITY_FAILED"),
          );
          return api;
        },
      },
      { label: "chart", result: ownerApi },
    ];

    for (const failure of failures) {
      mocks.getServerSession.mockResolvedValue(OWNER_SESSION);
      const result = failure.result();
      if (result instanceof ApiProblem) mocks.getProductApi.mockRejectedValue(result);
      else mocks.getProductApi.mockResolvedValue(result);

      const markup = await renderAsyncPage(StockBetaPage());

      expect(markup, `${failure.label}: no status panel`).not.toContain(
        'data-testid="service-status-panel"',
      );
      expect(markup, `${failure.label}: no private error prose`).not.toContain(
        failure.label === "chart" ? "private chart error" : "private provider prose",
      );
      if (failure.label === "chart") {
        expect(markup).toContain('data-testid="stock-beta-dashboard"');
      }
    }

    expect(fetchMock).not.toHaveBeenCalled();
  });
});
