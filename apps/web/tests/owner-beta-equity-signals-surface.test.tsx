import { readFileSync } from "node:fs";
import { join } from "node:path";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import StockBetaDetailPage from "@/app/(authenticated)/stock-beta/[instrument]/page";
import StockBetaPage from "@/app/(authenticated)/stock-beta/page";
import { type ApiSession, apiErrorEnvelopeSchema } from "@/lib/api/contracts";
import { ApiProblem } from "@/lib/api/response";
import {
  type OwnerEquityV2LatestSignalsModel,
  type OwnerEquityV2Lifecycle,
  type OwnerEquityV2MembershipListModel,
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

const MEMBER_SESSION = {
  ...OWNER_SESSION,
  role: "member",
  user_id: "00000000-0000-4000-8000-000000000002",
} as const satisfies ApiSession;

const SHA = `sha256:${"a".repeat(64)}`;
const BASE_TIME = "2026-08-30T06:00:00Z";
const LIFECYCLE_COVERAGE: Record<OwnerEquityV2Lifecycle, number> = {
  BACKFILLING: 80,
  DISABLED: 261,
  FAILED: 40,
  INSUFFICIENT_HISTORY: 40,
  MATERIALIZING: 200,
  READY: 261,
  REQUESTED: 0,
  VALIDATING: 20,
};

function uuidFromNumber(value: number): string {
  return `00000000-0000-4000-8000-${String(value).padStart(12, "0")}`;
}

function instrumentId(index: number): string {
  return `${String(index + 1).padStart(6, "0")}.KRX`;
}

function signal(index: number) {
  return ownerEquityV2SignalSchema.parse({
    average_trading_value_20: 1_000_000.1234 + index,
    average_volume_20: 25_000.6789 + index,
    condition: index % 3 === 0 ? "BULLISH" : index % 3 === 1 ? "NEUTRAL" : "BEARISH",
    generation: 7,
    instrument_id: instrumentId(index),
    max_drawdown_120: -0.3456 - index / 1_000,
    rank: index + 1,
    return_120: 0.4567 - index / 100,
    return_20: 0.1234 + index / 10_000,
    return_60: 0.2345 + index / 10_000,
    score: 1.2345 + index / 10,
    sma_20: 101.2345 + index,
    sma_60: 99.8765 + index,
    volatility_120: 0.3456 + index / 1_000,
    volatility_20: 0.1234 + index / 1_000,
    volatility_60: 0.2345 + index / 1_000,
    volume_ratio_20_60: 1.1111 + index / 1_000,
  });
}

function latestFor(rowCount: number): OwnerEquityV2LatestSignalsModel {
  const rows = Array.from({ length: rowCount }, (_, index) => signal(index));
  return ownerEquityV2LatestSignalsSchema.parse({
    snapshot: {
      as_of: "2026-08-29",
      published_at: BASE_TIME,
      row_count: rowCount,
      snapshot_id: uuidFromNumber(401),
      universe_sha256: SHA,
    },
    rows,
    top5: rows.slice(0, 5),
  });
}

function membershipFor(
  index: number,
  lifecycle: OwnerEquityV2Lifecycle = "READY",
  retryable = true,
) {
  const materialized = lifecycle === "READY" || lifecycle === "DISABLED";
  return ownerEquityV2MembershipListSchema.shape.memberships.element.parse({
    coverage: {
      ...(materialized ? { first_session: "2025-08-01", last_session: "2026-08-29" } : {}),
      minimum_observed_sessions: 121,
      observed_sessions: LIFECYCLE_COVERAGE[lifecycle],
      target_observed_sessions: 261,
    },
    ...(lifecycle === "FAILED"
      ? { failure: { code: "OWNER_EQUITY_PREPARATION_FAILED", retryable } }
      : {}),
    ...(lifecycle === "DISABLED" ? { disabled_at: "2026-09-02T06:00:00Z" } : {}),
    generation: materialized ? 7 : 0,
    id: uuidFromNumber(index + 501),
    instrument_id: instrumentId(index),
    lifecycle,
    requested_at: BASE_TIME,
    updated_at: BASE_TIME,
  });
}

function membershipsFor(
  lifecycles: readonly OwnerEquityV2Lifecycle[] = ["READY"],
): OwnerEquityV2MembershipListModel {
  const memberships = lifecycles.map((lifecycle, index) => membershipFor(index, lifecycle));
  const active = memberships.filter((membership) => membership.lifecycle !== "DISABLED").length;
  return ownerEquityV2MembershipListSchema.parse({
    memberships,
    policy: {
      active_instruments: active,
      max_active_instruments: 100,
      minimum_observed_sessions: 121,
      remaining_capacity: 100 - active,
      target_observed_sessions: 261,
    },
  });
}

function detailFor(index = 0) {
  return ownerEquityV2SignalDetailSchema.parse({
    signal: signal(index),
    snapshot: latestFor(1).snapshot,
  });
}

function apiFor({
  detail = detailFor(),
  detailError,
  latest = latestFor(31),
  latestError,
  memberships = membershipsFor(),
  membershipsError,
}: {
  readonly detail?: ReturnType<typeof detailFor>;
  readonly detailError?: unknown;
  readonly latest?: OwnerEquityV2LatestSignalsModel;
  readonly latestError?: unknown;
  readonly memberships?: OwnerEquityV2MembershipListModel;
  readonly membershipsError?: unknown;
} = {}) {
  return {
    getOwnerEquityV2LatestSignals: vi.fn(async () => {
      if (latestError !== undefined) throw latestError;
      return latest;
    }),
    getOwnerEquityV2Memberships: vi.fn(async () => {
      if (membershipsError !== undefined) throw membershipsError;
      return memberships;
    }),
    getOwnerEquityV2SignalDetail: vi.fn(async () => {
      if (detailError !== undefined) throw detailError;
      return detail;
    }),
  };
}

function session(value: ApiSession = OWNER_SESSION): void {
  mocks.getServerSession.mockResolvedValue(value);
}

function problem(
  code:
    | "OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE"
    | "OWNER_EQUITY_INTEGRITY_FAILED"
    | "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND"
    | "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE"
    | "RESOURCE_NOT_FOUND"
    | "SESSION_EXPIRED"
    | "SESSION_UNKNOWN",
): ApiProblem {
  return new ApiProblem(
    code === "SESSION_EXPIRED" || code === "SESSION_UNKNOWN" ? 401 : 503,
    apiErrorEnvelopeSchema.parse({
      error: { code, message: "typed test failure", request_id: "request-test" },
    }),
  );
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

function expectNoLegacyEvidence(markup: string): void {
  for (const marker of [
    "artifact_content_sha256",
    "condition_reasons",
    "factor_explanations",
    "instrument_name",
    "selection_reason",
    "vendor_snapshot",
  ]) {
    expect(markup).not.toContain(marker);
  }
}

afterEach(() => {
  vi.clearAllMocks();
  mocks.getLocale.mockResolvedValue("en");
});

describe("Owner stock signal beta V2 surface", () => {
  it.each([31, 100])("renders a dynamic %s-row ranked snapshot", async (rowCount) => {
    session();
    const latest = latestFor(rowCount);
    const api = apiFor({ latest });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());
    const tableBody = markup.match(/<tbody>([\s\S]*?)<\/tbody>/)?.[1] ?? "";

    expect(markup).toContain("Ranked signals");
    expect(markup).toContain('data-testid="stock-beta-snapshot-strip"');
    expect(markup).toContain(latest.snapshot.snapshot_id);
    expect(markup).toContain(latest.snapshot.as_of);
    expect(markup).toContain(latest.snapshot.published_at);
    expect(markup).toContain(latest.snapshot.universe_sha256);
    expect(markup).toContain(`>${latest.snapshot.row_count}<`);
    expect(tableBody.match(/<tr\b/g) ?? []).toHaveLength(rowCount);
    expect(renderedRowIds(markup)).toEqual(latest.rows.map((row) => row.instrument_id));
    expect(renderedTapeIds(markup)).toEqual(latest.top5.map((row) => row.instrument_id));
    expect(markup).toContain(instrumentId(rowCount - 1));
    expect(markup).not.toMatch(/30[- ]row/);
    expectNoLegacyEvidence(markup);
    expect(api.getOwnerEquityV2Memberships).toHaveBeenCalledOnce();
    expect(api.getOwnerEquityV2LatestSignals).toHaveBeenCalledOnce();
  });

  it("renders a zero-row V2 snapshot without inventing a row or stale signal", async () => {
    session();
    const latest = latestFor(0);
    const api = apiFor({ latest, memberships: membershipsFor([]) });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain(latest.snapshot.snapshot_id);
    expect(markup).toContain('data-testid="stock-beta-snapshot-strip"');
    expect(markup).toContain("The current V2 snapshot has no signal rows.");
    expect(renderedRowIds(markup)).toEqual([]);
    expect(renderedTapeIds(markup)).toEqual([]);
    expect(markup).not.toContain("000001.KRX");
    expectNoLegacyEvidence(markup);
  });

  it("renders the empty-capacity workflow with server policy and no signal rows", async () => {
    session();
    const api = apiFor({
      latestError: new ApiProblem(
        404,
        apiErrorEnvelopeSchema.parse({
          error: {
            code: "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE",
            message: "typed test failure",
            request_id: "request-test",
          },
        }),
      ),
      memberships: membershipsFor([]),
    });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());
    const capacityStart = markup.indexOf('data-testid="stock-beta-policy-capacity"');
    const capacity = markup.slice(capacityStart);

    expect(markup).toContain("No research instruments are configured");
    expect(markup).toContain("Signal snapshot unavailable");
    expect(markup).toContain('data-testid="stock-beta-policy-capacity"');
    expect(markup).toContain('id="stock-beta-instrument-code"');
    expect(markup).toContain('pattern="[0-9]{6}"');
    expect(capacityStart).toBeGreaterThanOrEqual(0);
    expect(capacity).toContain(">0<");
    expect(capacity).toContain(">100<");
    expect(capacity).toContain(">261<");
    expect(capacity).toContain(">121<");
    expect(renderedRowIds(markup)).toEqual([]);
    expect(markup).not.toContain('data-testid="stock-beta-snapshot-strip"');
    expectNoLegacyEvidence(markup);
  });

  it("renders every V2 detail snapshot and signal field without legacy evidence", async () => {
    session();
    const detail = detailFor(2);
    const api = apiFor({ detail });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({
        params: Promise.resolve({ instrument: detail.signal.instrument_id }),
      }),
    );

    for (const value of Object.values(detail.snapshot)) expect(markup).toContain(String(value));
    for (const value of Object.values(detail.signal)) expect(markup).toContain(String(value));
    expect(markup).toContain('data-testid="stock-beta-instrument-header"');
    expect(markup).toContain('data-testid="stock-beta-detail-returns"');
    expect(markup).toContain('data-testid="stock-beta-detail-risk"');
    expect(markup).toContain('data-testid="stock-beta-detail-activity"');
    expect(markup).toContain('data-testid="stock-beta-provenance"');
    for (const policyText of [
      "Owner-only managed KRX research instruments; this is not index membership or the whole market.",
      "Signals are shown only from the current published V2 snapshot.",
      "Original/unadjusted prices are used; corporate actions can distort returns and drawdowns.",
      "This is not strict point-in-time evidence; dates identify the current snapshot.",
      "Volume and trading value are activity/liquidity proxies, not execution liquidity.",
      "BULLISH / NEUTRAL / BEARISH are conditions, not probabilities, target prices, buy/sell calls, weights, or orders.",
    ]) {
      expect(markup).toContain(policyText);
    }
    expectNoLegacyEvidence(markup);
    expect(api.getOwnerEquityV2SignalDetail).toHaveBeenCalledWith(detail.signal.instrument_id);
  });

  it("keeps active Stock Beta page data access on the V2 client surface", () => {
    const sourceFiles = [
      "app/(authenticated)/stock-beta/page.tsx",
      "app/(authenticated)/stock-beta/[instrument]/page.tsx",
      "components/stock-beta/stock-beta-workspace.tsx",
    ];

    for (const relativePath of sourceFiles) {
      const source = readFileSync(join(process.cwd(), relativePath), "utf8");
      expect(source).not.toMatch(
        /getOwnerBetaEquitySignalsLatest|screenOwnerBetaEquitySignals|getOwnerBetaEquitySignalDetail/,
      );
    }
  });

  it("refuses the Member before constructing either V2 product page", async () => {
    session(MEMBER_SESSION);
    const api = apiFor();
    mocks.getProductApi.mockResolvedValue(api);

    const dashboardMarkup = renderToStaticMarkup(await StockBetaPage());
    const detailMarkup = renderToStaticMarkup(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: instrumentId(0) }) }),
    );

    expect(dashboardMarkup).not.toContain('data-testid="stock-beta-dashboard"');
    expect(detailMarkup).not.toContain('data-testid="stock-beta-detail-board"');
    expect(dashboardMarkup).toContain('href="/"');
    expect(detailMarkup).toContain('href="/"');
    expect(dashboardMarkup).not.toContain("V2 universe management");
    expect(detailMarkup).not.toContain("V2 signal");
    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(api.getOwnerEquityV2Memberships).not.toHaveBeenCalled();
    expect(api.getOwnerEquityV2SignalDetail).not.toHaveBeenCalled();
  });

  it("redirects expired owner requests to login before exposing a V2 state", async () => {
    session();
    const api = apiFor({ latestError: problem("SESSION_EXPIRED") });
    mocks.getProductApi.mockResolvedValue(api);

    const result = await StockBetaPage();
    expect(mocks.redirect).toHaveBeenCalledWith("/login");
    expect(api.getOwnerEquityV2LatestSignals).toHaveBeenCalledOnce();
    expect(renderToStaticMarkup(result)).not.toContain('data-testid="stock-beta-dashboard"');
  });

  it.each([
    ["OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE", "Signal snapshot unavailable"],
    ["OWNER_EQUITY_INTEGRITY_FAILED", "Signal snapshot integrity failed"],
  ] as const)("renders the typed dashboard %s state", async (code, title) => {
    session();
    const api = apiFor({ membershipsError: problem(code) });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(await StockBetaPage());

    expect(markup).toContain(title);
    expect(markup).not.toContain('data-testid="stock-beta-dashboard"');
    expectNoLegacyEvidence(markup);
  });

  it.each([
    ["OWNER_EQUITY_SNAPSHOT_UNAVAILABLE", "Signal snapshot unavailable"],
    ["OWNER_EQUITY_INTEGRITY_FAILED", "Signal snapshot integrity failed"],
    ["RESOURCE_NOT_FOUND", "Instrument signal not found"],
    ["OWNER_EQUITY_MEMBERSHIP_NOT_FOUND", "Instrument signal not found"],
  ] as const)("renders the typed detail %s state without children", async (code, title) => {
    session();
    const api = apiFor({ detailError: problem(code) });
    mocks.getProductApi.mockResolvedValue(api);

    const markup = renderToStaticMarkup(
      await StockBetaDetailPage({ params: Promise.resolve({ instrument: instrumentId(0) }) }),
    );

    expect(markup).toContain(title);
    expect(markup).not.toContain('data-testid="stock-beta-detail-board"');
    expect(markup).not.toContain('data-testid="stock-beta-instrument-header"');
    expectNoLegacyEvidence(markup);
  });

  it("removes stale signals before refreshing membership state after disable", () => {
    const source = readFileSync(
      join(process.cwd(), "components/stock-beta/stock-beta-workspace.tsx"),
      "utf8",
    );
    const disableStart = source.indexOf("async function confirmDisable");
    const pendingRemoval = source.indexOf(
      "setPendingSignalRemovalInstrument(result.resource.instrument_id)",
      disableStart,
    );
    const clearSignals = source.indexOf("setSignals(null)", pendingRemoval);
    const refreshMemberships = source.indexOf("await refreshMemberships()", pendingRemoval);

    expect(disableStart).toBeGreaterThanOrEqual(0);
    expect(pendingRemoval).toBeGreaterThan(disableStart);
    expect(clearSignals).toBeGreaterThan(pendingRemoval);
    expect(refreshMemberships).toBeGreaterThan(clearSignals);
    expect(source).toContain(
      "next.rows.some((row) => row.instrument_id === pendingSignalRemovalInstrument)",
    );
    expect(source).toContain(
      "ownerEquityV2AddBodySchema.safeParse({ instrument_code: instrumentCode })",
    );
    expect(source).toContain("addOwnerEquityV2Membership(parsed.data)");
    expect(source).toContain("retryOwnerEquityV2Membership(membershipId)");
    expect(source).toContain("disableOwnerEquityV2Membership(disableId)");
    expect(source).toContain("ownerEquityV2PollDelay");
    expect(source).toContain("getOwnerEquityV2LatestSignals");
  });
});
