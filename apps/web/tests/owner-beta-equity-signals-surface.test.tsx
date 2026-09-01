import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import StockBetaDetailPage from "@/app/(authenticated)/stock-beta/[instrument]/page";
import StockBetaPage from "@/app/(authenticated)/stock-beta/page";
import { type ApiSession, apiErrorEnvelopeSchema } from "@/lib/api/contracts";
import { ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import {
  type OwnerEquityV2LatestSignalsModel,
  type OwnerEquityV2MembershipListModel,
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2SignalDetailSchema,
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
const BASE_TIME = "2026-08-30T06:00:00Z";

function instrumentId(index: number): string {
  return `${String(index + 1).padStart(6, "0")}.KRX`;
}

function signal(index: number) {
  return {
    average_trading_value_20: 1_000_000 + index,
    average_volume_20: 100_000 + index,
    condition: index % 3 === 0 ? "BULLISH" : index % 3 === 1 ? "NEUTRAL" : "BEARISH",
    instrument_id: instrumentId(index),
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
    generation: 1,
  } as const;
}

function latestFor(rowCount: number): OwnerEquityV2LatestSignalsModel {
  const rows = Array.from({ length: rowCount }, (_, index) => signal(index));
  return ownerEquityV2LatestSignalsSchema.parse({
    snapshot: {
      as_of: "2026-08-29",
      published_at: BASE_TIME,
      row_count: rows.length,
      snapshot_id: "00000000-0000-4000-8000-000000000401",
      universe_sha256: SHA,
    },
    rows,
    top5: rows.slice(0, 5),
  });
}

function membershipsFor(
  memberships: readonly { readonly index: number; readonly lifecycle?: "READY" | "FAILED" }[] = [
    { index: 0 },
  ],
): OwnerEquityV2MembershipListModel {
  return ownerEquityV2MembershipListSchema.parse({
    memberships: memberships.map(({ index, lifecycle = "READY" }) => ({
      coverage: {
        first_session: "2025-08-01",
        last_session: "2026-08-29",
        minimum_observed_sessions: 121,
        observed_sessions: lifecycle === "READY" ? 261 : 40,
        target_observed_sessions: 261,
      },
      failure:
        lifecycle === "FAILED"
          ? { code: "OWNER_EQUITY_BACKFILL_RETRYABLE", retryable: true }
          : undefined,
      generation: 1,
      id: `00000000-0000-4000-8000-${String(index + 501).padStart(12, "0")}`,
      instrument_id: instrumentId(index),
      lifecycle,
      requested_at: BASE_TIME,
      updated_at: BASE_TIME,
    })),
    policy: {
      active_instruments: memberships.length,
      max_active_instruments: 100,
      minimum_observed_sessions: 121,
      remaining_capacity: 100 - memberships.length,
      target_observed_sessions: 261,
    },
  });
}

const latest = latestFor(31);
const memberships = membershipsFor();
const detail = ownerEquityV2SignalDetailSchema.parse({
  signal: latest.rows[0],
  snapshot: latest.snapshot,
});

function apiFor(overrides: Record<string, unknown> = {}) {
  return {
    getOwnerEquityV2LatestSignals: vi.fn(async () => latest),
    getOwnerEquityV2Memberships: vi.fn(async () => memberships),
    getOwnerEquityV2SignalDetail: vi.fn(async () => detail),
    ...overrides,
  };
}

function session(value: ApiSession = OWNER_SESSION): void {
  mocks.getServerSession.mockResolvedValue(value);
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

describe("Owner stock signal beta V2 surface", () => {
  it.each([31, 100])("renders a dynamic %s-row ranked snapshot", async (rowCount) => {
    session();
    const api = apiFor({ getOwnerEquityV2LatestSignals: vi.fn(async () => latestFor(rowCount)) });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());
    const tableBody = markup.slice(markup.indexOf("<tbody>"), markup.indexOf("</tbody>") + 8);

    expect(markup).toContain("Latest signals");
    expect(markup).toContain('data-testid="stock-beta-top-five"');
    expect(markup.match(/stock-beta-top-card/g)).toHaveLength(5);
    expect(tableBody.match(/<tr>/g)).toHaveLength(rowCount);
    expect(markup).toContain(`${String(rowCount).padStart(6, "0")}.KRX`);
    expect(markup).not.toContain("30-row");
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    expect(api.getOwnerEquityV2LatestSignals).toHaveBeenCalledOnce();
  });

  it("renders an empty managed universe and server-provided capacity without signal rows", async () => {
    session();
    const emptyMemberships = membershipsFor([]);
    const api = apiFor({
      getOwnerEquityV2LatestSignals: vi.fn(async () => {
        throw new ApiProblem(
          404,
          apiErrorEnvelopeSchema.parse({
            error: {
              code: "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE",
              message: "not rendered",
              request_id: "request-test",
            },
          }),
        );
      }),
      getOwnerEquityV2Memberships: vi.fn(async () => emptyMemberships),
    });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain("No configured instruments");
    expect(markup).toContain('data-testid="stock-beta-policy-capacity"');
    expect(markup).toContain("100");
    expect(markup).toContain("Signals are not ready");
    expect(markup).not.toContain('data-testid="stock-beta-rank-table"');
  });

  it("renders V2 detail metadata and does not render V1 factors or provenance", async () => {
    session();
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: "000001.KRX" }) }),
    );

    expect(markup).toContain("000001.KRX");
    expect(markup).toContain("Snapshot");
    expect(markup).toContain(SHA);
    expect(markup).not.toContain("Exact condition reasons");
    expect(markup).not.toContain("API provenance");
    expect(api.getOwnerEquityV2SignalDetail).toHaveBeenCalledWith("000001.KRX");
  });

  it("blocks a Member direct visit before constructing the product client or rendering rows", async () => {
    session(MEMBER_SESSION);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("a refused Member must not construct the product client");
    });

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain("Owner access required");
    expect(markup).not.toContain("000001.KRX");
    expect(markup).not.toContain("stock-beta-rank-table");
    expect(mocks.getProductApi).not.toHaveBeenCalled();
  });

  it("redirects to login when the authenticated V2 request expires", async () => {
    session();
    const api = apiFor({
      getOwnerEquityV2Memberships: vi.fn(async () => {
        throw loginError("SESSION_EXPIRED");
      }),
    });
    mocks.getProductApi.mockResolvedValue(api);

    await expect(StockBetaPage()).rejects.toMatchObject({
      destination: "/login",
      message: "NEXT_REDIRECT",
    });
    expect(mocks.redirect).toHaveBeenCalledWith("/login");
  });

  it("renders a typed integrity failure without provider prose or rows", async () => {
    session();
    const api = apiFor({
      getOwnerEquityV2LatestSignals: vi.fn(async () => {
        throw new ApiProblem(
          503,
          apiErrorEnvelopeSchema.parse({
            error: {
              code: "OWNER_EQUITY_INTEGRITY_FAILED",
              message: "provider response must not be rendered",
              request_id: "request-test",
            },
          }),
        );
      }),
    });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain("Signal snapshot integrity failed");
    expect(markup).not.toContain("provider response must not be rendered");
    expect(markup).not.toContain('data-testid="stock-beta-rank-table"');
  });

  it("keeps the policy boundary prominent in both locales", async () => {
    session();
    mocks.getProductApi.mockResolvedValue(apiFor());

    const english = renderToStaticMarkup(await StockBetaPage());
    expect(english).toContain("Owner-only configured research instrument universe");
    expect(english).toContain("vendor snapshot");
    expect(english).toContain("Original/unadjusted prices");
    expect(english).toContain("not point-in-time (PIT)");
    expect(english).toContain("not execution liquidity");

    mocks.getLocale.mockResolvedValue("ko");
    const korean = renderToStaticMarkup(await StockBetaPage());
    expect(korean).toContain("오너 전용으로 구성된 연구 종목군");
    expect(korean).toContain("벤더 스냅샷");
    expect(korean).toContain("원주가(비조정 가격)");
    expect(korean).toContain("PIT(시점 일치)");
    expect(korean).toContain("체결 유동성을 뜻하지 않습니다");
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
