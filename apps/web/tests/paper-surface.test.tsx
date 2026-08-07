import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import PaperPage from "@/app/(authenticated)/paper/page";
import { defaultAccount, parityReason } from "@/lib/products/paper-contracts";

vi.mock("server-only", () => ({}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
}));

vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: () => ({ name: "__Host-lagrange_session", value: "member-opaque" }),
  }),
}));

const ACCOUNT_ID = "00000000-0000-4000-8000-000000000401";
const CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const NOTICE_ID = "00000000-0000-4000-8000-000000000431";
const TARGET_ID = "00000000-0000-4000-8000-000000000421";

const FILL_MODEL_DIFFERENCE =
  "Backtest fills come from the NautilusTrader engine's execution model; Paper fills are modeled at the next session's raw open plus the configured slippage.";

function matchingParity() {
  return {
    account_id: ACCOUNT_ID,
    as_of: "2026-01-30",
    divergences: [],
    fill_model_difference: FILL_MODEL_DIFFERENCE,
    lineage: {
      fields: [
        { backtest: "dual_momentum", field: "strategy_id", paper: "dual_momentum" },
        { backtest: "krx-eod.2026-01-30", field: "dataset_version", paper: "krx-eod.2026-01-30" },
      ],
    },
    status: "MATCH",
    warrants_alert: false,
  };
}

function divergentParity() {
  return {
    ...matchingParity(),
    divergences: [
      { backtest_weight: "0.900000", instrument_id: "069500.KRX", paper_weight: "0.600000" },
    ],
    status: "DIVERGENT",
    warrants_alert: true,
  };
}

function notification(deliveries: unknown[]) {
  return {
    body: "The paper session for 2026-02-02 executed its target.",
    created_at: "2026-02-02T00:06:00Z",
    deliveries,
    id: NOTICE_ID,
    kind: "job",
    title: "Paper session 2026-02-02 completed",
  };
}

type PaperFixture = {
  readonly accounts?: unknown[];
  readonly notifications?: unknown[];
  readonly parity?: unknown;
};

function syntheticPaperApi(fixture: PaperFixture = {}): typeof fetch {
  const accounts = fixture.accounts ?? [
    {
      account_type: "PAPER",
      cost_profile_id: "KRX_ETF_DEFAULT",
      cost_profile_version: 1,
      created_at: "2026-01-02T00:30:00Z",
      currency: "KRW",
      id: ACCOUNT_ID,
      initial_cash: "10000000.0000",
      name: "Paper account 1",
      status: "ACTIVE",
      updated_at: "2026-02-02T00:30:00Z",
    },
  ];
  return async (input, init) => {
    const request = new Request(input, init);
    const { pathname } = new URL(request.url);
    if (pathname === "/api/v1/paper/accounts") {
      return Response.json({ has_more: false, items: accounts, next_cursor: null });
    }
    if (pathname === `/api/v1/paper/accounts/${ACCOUNT_ID}/performance`) {
      return Response.json({
        account_id: ACCOUNT_ID,
        disclaimer:
          "Simulated results from a paper account. Past simulated performance is not a guarantee of future returns.",
        points: [
          {
            cash: "1994320.0000",
            currency: "KRW",
            equity: "10042180.0000",
            positions_value: "8047860.0000",
            return_pct: "0.004218",
            trading_date: "2026-02-02",
          },
        ],
      });
    }
    if (pathname === `/api/v1/paper/accounts/${ACCOUNT_ID}/lineage`) {
      return Response.json({
        account_id: ACCOUNT_ID,
        bindings: [
          {
            active: true,
            bound_at: "2026-01-20T00:30:00Z",
            strategy_config_id: CONFIG_ID,
            strategy_id: "dual_momentum",
            strategy_version: "2.3.1",
            unbound_at: null,
          },
        ],
        targets: [
          {
            computed_on: "2026-01-30",
            effective_date: "2026-02-02",
            executed_at: "2026-02-02T00:05:00Z",
            id: TARGET_ID,
            status: "EXECUTED",
          },
        ],
      });
    }
    if (pathname.startsWith(`/api/v1/paper/accounts/${ACCOUNT_ID}/parity`)) {
      return Response.json(fixture.parity ?? matchingParity());
    }
    if (pathname === `/api/v1/paper/accounts/${ACCOUNT_ID}/positions`) {
      return Response.json({
        has_more: false,
        items: [
          {
            avg_price: "40239.3000",
            instrument_id: "069500.KRX",
            quantity: "200",
            updated_at: "2026-02-02T00:05:00Z",
          },
        ],
        next_cursor: null,
      });
    }
    if (pathname === `/api/v1/paper/accounts/${ACCOUNT_ID}/orders`) {
      return Response.json({ has_more: false, items: [], next_cursor: null });
    }
    if (pathname === "/api/v1/strategy-configs") {
      return Response.json({
        has_more: false,
        items: [
          {
            created_at: "2026-01-20T00:30:00Z",
            id: CONFIG_ID,
            is_active: true,
            strategy_id: "dual_momentum",
            strategy_version: "2.3.1",
            updated_at: "2026-01-20T00:30:00Z",
          },
        ],
        next_cursor: null,
      });
    }
    if (pathname === "/api/v1/notifications") {
      return Response.json({
        has_more: false,
        items: fixture.notifications ?? [notification([{ channel: "web", status: "SUCCESS" }])],
        next_cursor: null,
      });
    }
    return Response.json(
      {
        error: {
          code: "RESOURCE_NOT_FOUND",
          message: `unmapped ${pathname}`,
          request_id: "request-component-paper",
        },
      },
      { status: 404 },
    );
  };
}

beforeEach(() => {
  vi.stubEnv("API_INTERNAL_URL", "https://api.internal");
  vi.stubGlobal("fetch", syntheticPaperApi());
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("paper product surface", () => {
  it("renders ledger-derived equity, the disclaimer, and full lineage", async () => {
    // Given / When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then
    expect(markup).toContain("10042180.0000");
    expect(markup).toContain("8047860.0000");
    expect(markup).toContain("0.004218");
    expect(markup).toContain("not a guarantee of future returns");
    expect(markup).toContain("dual_momentum");
    expect(markup).toContain("2.3.1");
    expect(markup).toContain("2026-01-30");
    expect(markup).toContain("EXECUTED");
  });

  it("states the fill-model difference even when parity matches", async () => {
    // Given / When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then — a match must never read as "the two are interchangeable".
    expect(markup).toContain("Fill model difference");
    expect(markup).toContain("modeled at the next session&#x27;s raw open");
    expect(markup).toContain('role="status"');
  });

  it("raises a divergence as an alert with its reason and weights", async () => {
    // Given
    vi.stubGlobal("fetch", syntheticPaperApi({ parity: divergentParity() }));

    // When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then
    expect(markup).toContain('role="alert"');
    expect(markup).toContain("different target weights for 1 instrument(s)");
    expect(markup).toContain("0.900000");
    expect(markup).toContain("0.600000");
  });

  it("shows a failed delivery with its reason rather than nothing", async () => {
    // Given
    vi.stubGlobal(
      "fetch",
      syntheticPaperApi({
        notifications: [
          notification([
            { channel: "web", status: "SUCCESS" },
            {
              channel: "email",
              error_detail: "email delivery not configured in this release",
              status: "FAILED",
            },
          ]),
        ],
      }),
    );

    // When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then
    expect(markup).toContain("email: FAILED");
    expect(markup).toContain("email delivery not configured in this release");
    expect(markup).toContain("web: SUCCESS");
  });

  it("renders the empty state instead of inventing an account", async () => {
    // Given
    vi.stubGlobal("fetch", syntheticPaperApi({ accounts: [] }));

    // When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then
    expect(markup).toContain("No paper account selected");
    expect(markup).not.toContain("Daily performance");
  });
});

describe("paper contract helpers", () => {
  it("prefers an ACTIVE account and never invents one", () => {
    const active = { status: "ACTIVE" } as never;
    const closed = { status: "CLOSED" } as never;
    expect(defaultAccount([closed, active])).toBe(active);
    expect(defaultAccount([closed])).toBe(closed);
    expect(defaultAccount([])).toBeNull();
  });

  it("names the mismatching fields when parity cannot be claimed", () => {
    const reason = parityReason({
      ...matchingParity(),
      lineage: {
        fields: [
          { backtest: "krx-eod.2026-01-29", field: "dataset_version", paper: "krx-eod.2026-01-30" },
        ],
      },
      status: "NOT_COMPARABLE",
      warrants_alert: true,
    } as never);
    expect(reason).toContain("dataset_version");
    expect(reason).toContain("no parity claim is possible");
  });
});
