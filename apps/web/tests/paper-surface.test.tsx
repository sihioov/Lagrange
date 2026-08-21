import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import PaperPage from "@/app/(authenticated)/paper/page";
import {
  MAX_POLL_ATTEMPTS,
  RebalancePreviewOutcome,
  rebalancePollDelay,
  settledPreviewState,
} from "@/components/paper/paper-rebalance-preview";
import { paperDictionary } from "@/lib/i18n/dictionaries/paper";
import {
  defaultAccount,
  parityReason,
  type RebalancePreviewModel,
} from "@/lib/products/paper-contracts";

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
const RUN_ID = "00000000-0000-4000-8000-000000000201";
const PREVIEW_ID = "00000000-0000-4000-8000-000000000501";
const TARGET_PORTFOLIO_ID = "00000000-0000-4000-8000-000000000601";
const DATASET_VERSION_ID = "00000000-0000-4000-8000-000000000701";
const JOB_ID = "00000000-0000-4000-8000-000000000801";
const SHA = "a".repeat(64);
const PREVIEW_TOKEN = "b".repeat(64);

function succeededRun() {
  return {
    as_of: "2026-01-30",
    created_at: "2026-01-30T00:10:00Z",
    id: RUN_ID,
    provenance: {},
    status: "SUCCEEDED",
    strategy_config_id: CONFIG_ID,
    trigger_kind: "MANUAL",
  };
}

function readyPreview(overrides: Partial<RebalancePreviewModel> = {}): RebalancePreviewModel {
  return {
    account_id: ACCOUNT_ID,
    applied_at: null,
    completed_at: "2026-01-30T00:20:00Z",
    created_at: "2026-01-30T00:15:00Z",
    dataset_manifest_sha256: SHA,
    dataset_version_id: DATASET_VERSION_ID,
    id: PREVIEW_ID,
    job_id: JOB_ID,
    preview_token: PREVIEW_TOKEN,
    price_basis: "RECOMMENDATION_CLOSE",
    price_date: "2026-01-30",
    proposed_effective_date: "2026-02-02",
    recommendation_run_id: RUN_ID,
    result: {
      available_cash: "500000.0000",
      buy_notional: "1000000.0000",
      cash_before: "2000000.0000",
      decisions: [
        {
          action: "BUY",
          current_quantity: "0",
          current_value: "0",
          current_weight: "0",
          delta_value: "1000000.0000",
          instrument_id: "069500.KRX",
          skip_reason: null,
          target_value: "1000000.0000",
          target_weight: "0.5",
        },
      ],
      equity: "10000000.0000",
      explicit_fees: "1500.0000",
      informational_slippage: "500.0000",
      leftover_cash: "498500.0000",
      lineage: {
        account_id: ACCOUNT_ID,
        account_state_sha256: SHA,
        account_state_version: 3,
        curated_version: 7,
        dataset_manifest_sha256: SHA,
        dataset_version_id: DATASET_VERSION_ID,
        recommendation_run_id: RUN_ID,
        strategy_config_id: CONFIG_ID,
        target_portfolio_id: TARGET_PORTFOLIO_ID,
        target_portfolio_sha256: SHA,
      },
      orders: [
        {
          commission: "300.0000",
          estimated_execution_price: "40500.0000",
          informational_slippage: "500.0000",
          instrument_id: "069500.KRX",
          notional: "1000000.0000",
          quantity: "24",
          raw_price: "40239.3000",
          side: "BUY",
          tax: "0.0000",
        },
      ],
      price_basis: "RECOMMENDATION_CLOSE",
      price_date: "2026-01-30",
      proposed_effective_date: "2026-02-02",
      schema_version: 1,
      sell_notional: "0.0000",
      warning_code: "INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED",
    },
    started_at: "2026-01-30T00:15:05Z",
    status: "READY",
    strategy_config_id: CONFIG_ID,
    target_portfolio_id: TARGET_PORTFOLIO_ID,
    target_portfolio_sha256: SHA,
    updated_at: "2026-01-30T00:20:00Z",
    ...overrides,
  };
}

function failedPreview(): RebalancePreviewModel {
  return {
    ...readyPreview(),
    completed_at: "2026-01-30T00:20:00Z",
    error: {
      code: "REBALANCE_PREVIEW_DATA_BLOCKED",
      message: "Dataset was blocked for this session.",
    },
    preview_token: null,
    result: undefined,
    status: "FAILED",
  };
}

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
  readonly recommendationRuns?: unknown[];
  /**
   * Emulates the `recommendation` entitlement being refused. That use is a
   * different entitlement from the `paper_view` one every other fetch on this
   * page needs, so it must be able to fail on its own.
   */
  readonly recommendationRunsProblem?: { readonly code: string; readonly status: number };
  readonly role?: "member" | "owner";
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
      owner_user_id: "00000000-0000-4000-8000-000000000001",
      can_manage: true,
      name: "Paper account 1",
      status: "ACTIVE",
      updated_at: "2026-02-02T00:30:00Z",
    },
  ];
  const role = fixture.role ?? "owner";
  return async (input, init) => {
    const request = new Request(input, init);
    const { pathname } = new URL(request.url);
    if (pathname === "/api/v1/auth/session") {
      return Response.json({
        expires_at_secs: 2_000_000_000,
        role,
        user_id: "00000000-0000-4000-8000-000000000001",
      });
    }
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
    if (pathname === "/api/v1/recommendations/runs") {
      const problem = fixture.recommendationRunsProblem;
      if (problem !== undefined) {
        return Response.json(
          {
            error: {
              code: problem.code,
              message: "The recommendation entitlement is not active for this session.",
              request_id: "request-component-paper",
            },
          },
          { status: problem.status },
        );
      }
      return Response.json({
        has_more: false,
        items: fixture.recommendationRuns ?? [succeededRun()],
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

  it("renders a shared account read-only", async () => {
    vi.stubGlobal(
      "fetch",
      syntheticPaperApi({
        accounts: [
          {
            account_type: "PAPER",
            can_manage: false,
            cost_profile_id: "KRX_ETF_DEFAULT",
            cost_profile_version: 1,
            created_at: "2026-01-02T00:30:00Z",
            currency: "KRW",
            id: ACCOUNT_ID,
            initial_cash: "10000000.0000",
            name: "Shared paper account",
            owner_user_id: "00000000-0000-4000-8000-000000000002",
            status: "ACTIVE",
            updated_at: "2026-02-02T00:30:00Z",
          },
        ],
      }),
    );

    const markup = renderToStaticMarkup(await PaperPage());

    expect(markup).toContain("Shared account · 00000000");
    expect(markup).not.toContain("Bind strategy");
    expect(markup).not.toContain("Rebalancing preview");
  });

  it("keeps the paper data on screen when the recommendation entitlement is refused", async () => {
    // Given — `recommendation` is a different entitlement from `paper_view`.
    vi.stubGlobal(
      "fetch",
      syntheticPaperApi({
        recommendationRunsProblem: { code: "DATA_ENTITLEMENT_REQUIRED", status: 403 },
      }),
    );

    // When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then — only the preview section degrades; nothing else is withheld.
    expect(markup).not.toContain("Paper data is blocked");
    expect(markup).toContain("10042180.0000");
    expect(markup).toContain("Fill model difference");
    expect(markup).toContain("EXECUTED");
    expect(markup).toContain("Paper session 2026-02-02 completed");
    expect(markup).toContain("Rebalancing preview");
    expect(markup).toContain("No completed recommendation run is available to preview yet.");
  });

  it("renders the rebalance preview section for an owner with a completed run", async () => {
    // Given / When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then — the option names the strategy the run was produced under.
    expect(markup).toContain("Rebalancing preview");
    expect(markup).toContain("Recommendation run");
    expect(markup).toContain(`2026-01-30 · dual_momentum@2.3.1 · ${RUN_ID.slice(0, 8)}`);
  });

  it("withholds the preview section from a Member who owns the account record", async () => {
    // Given — `can_manage` is record ownership; the endpoints need the Owner role.
    vi.stubGlobal("fetch", syntheticPaperApi({ role: "member" }));

    // When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then — every action in that section would 403, so it is not offered.
    expect(markup).not.toContain("Rebalancing preview");
    expect(markup).not.toContain("Create preview");
    // The bind form has no role check on the server and stays available.
    expect(markup).toContain("Bind strategy");
  });

  it("omits runs the active binding did not produce", async () => {
    // Given — the server answers 400 BINDING_REQUIRED for a foreign run.
    vi.stubGlobal(
      "fetch",
      syntheticPaperApi({
        recommendationRuns: [
          { ...succeededRun(), strategy_config_id: "00000000-0000-4000-8000-000000000109" },
        ],
      }),
    );

    // When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then
    expect(markup).toContain("Rebalancing preview");
    expect(markup).toContain("No completed recommendation run is available to preview yet.");
    expect(markup).not.toContain(`<option value="${RUN_ID}"`);
  });

  it("omits every run while the account has no active binding", async () => {
    // Given — a run with no strategy_config_id must not match "no binding".
    vi.stubGlobal(
      "fetch",
      syntheticPaperApi({
        recommendationRuns: [{ ...succeededRun(), strategy_config_id: null }],
      }),
    );

    // When
    const markup = renderToStaticMarkup(await PaperPage());

    // Then
    expect(markup).toContain("No completed recommendation run is available to preview yet.");
  });
});

describe("rebalance preview outcome", () => {
  const t = paperDictionary.en;

  it("surfaces the indicative next-open replan warning when a preview is READY", () => {
    // Given / When
    const markup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={null}
        applying={false}
        onApply={() => undefined}
        preview={readyPreview()}
        t={t}
      />,
    );

    // Then
    expect(markup).toContain("Indicative only — next-open replan required");
    expect(markup).toContain(
      "prices decisions at the recommendation close, but the account will actually execute at the next session&#x27;s open",
    );
    expect(markup).toContain("069500.KRX");
    expect(markup).toContain("BUY");
  });

  it("surfaces the error code and message when a preview FAILED", () => {
    // Given / When
    const markup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={null}
        applying={false}
        onApply={() => undefined}
        preview={failedPreview()}
        t={t}
      />,
    );

    // Then
    expect(markup).toContain('role="alert"');
    expect(markup).toContain("Preview failed");
    expect(markup).toContain("REBALANCE_PREVIEW_DATA_BLOCKED");
    expect(markup).toContain("Dataset was blocked for this session.");
  });

  it("labels both definition lists with their existing captions", () => {
    // Given / When
    const markup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={null}
        applying={false}
        onApply={() => undefined}
        preview={readyPreview()}
        t={t}
      />,
    );

    // Then
    expect(markup).toContain('<h3 id="paper-rebalance-totals-title">Preview totals</h3>');
    expect(markup).toContain('aria-labelledby="paper-rebalance-totals-title"');
    expect(markup).toContain('<h3 id="paper-rebalance-lineage-title">Preview lineage</h3>');
    expect(markup).toContain('aria-labelledby="paper-rebalance-lineage-title"');
    // The indicative warning still precedes every number.
    expect(markup.indexOf("Indicative only")).toBeLessThan(markup.indexOf("Preview totals"));
    expect(markup.indexOf("Indicative only")).toBeLessThan(markup.indexOf("10000000.0000"));
  });

  it("gives the Apply button an accessible name containing its visible label", () => {
    // Given / When
    const markup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={null}
        applying={false}
        onApply={() => undefined}
        preview={readyPreview()}
        t={t}
      />,
    );

    // Then — WCAG 2.5.3: no aria-label may displace the visible text.
    expect(markup).not.toContain("Apply rebalance preview");
    expect(markup).toContain('<button class="primary-action" type="button">Apply preview</button>');
  });

  it("keeps the preview on screen when applying it failed", () => {
    // Given
    const stale = "Preview token no longer matches the account state.";

    // When
    const markup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={stale}
        applying={false}
        onApply={() => undefined}
        preview={readyPreview()}
        t={t}
      />,
    );

    // Then — the plan and a live Apply survive the failure.
    expect(markup).toContain("Apply failed");
    expect(markup).toContain(stale);
    expect(markup).toContain("069500.KRX");
    expect(markup).not.toContain('class="primary-action" disabled=""');
  });

  it("refuses Apply on a READY preview that carries no result", () => {
    // Given — nothing is disclosed, so nothing may be applied.
    const resultless = readyPreview({ result: undefined });

    // When
    const markup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={null}
        applying={false}
        onApply={() => undefined}
        preview={resultless}
        t={t}
      />,
    );

    // Then
    expect(markup).not.toContain("Indicative only");
    expect(markup).toContain('class="primary-action" disabled=""');
  });

  it("disables Apply when the preview has no preview_token", () => {
    // Given
    const withoutToken = readyPreview({ preview_token: null });
    const disabledAttribute = 'class="primary-action" disabled=""';

    // When
    const disabledMarkup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={null}
        applying={false}
        onApply={() => undefined}
        preview={withoutToken}
        t={t}
      />,
    );
    const enabledMarkup = renderToStaticMarkup(
      <RebalancePreviewOutcome
        applyError={null}
        applying={false}
        onApply={() => undefined}
        preview={readyPreview()}
        t={t}
      />,
    );

    // Then
    expect(disabledMarkup).toContain(disabledAttribute);
    expect(enabledMarkup).not.toContain(disabledAttribute);
  });
});

describe("paper contract helpers", () => {
  it("prefers an ACTIVE account and never invents one", () => {
    const active = { can_manage: true, status: "ACTIVE" } as never;
    const closed = { can_manage: true, status: "CLOSED" } as never;
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

describe("rebalance preview polling schedule", () => {
  it("grows to the 8s plateau and never exceeds it", () => {
    expect(rebalancePollDelay(0)).toBe(500);
    expect(rebalancePollDelay(1)).toBe(1_000);
    expect(rebalancePollDelay(2)).toBe(2_000);
    expect(rebalancePollDelay(3)).toBe(4_000);
    expect(rebalancePollDelay(4)).toBe(8_000);
    expect(rebalancePollDelay(MAX_POLL_ATTEMPTS)).toBe(8_000);
    expect(rebalancePollDelay(1_000)).toBe(8_000);
  });

  it("caps the attempt budget so polling cannot run forever", () => {
    expect(MAX_POLL_ATTEMPTS).toBe(12);
  });

  it("treats APPLIED as terminal rather than something still computing", () => {
    // Given / When / Then
    expect(settledPreviewState(readyPreview())).toEqual({
      kind: "ready",
      preview: readyPreview(),
    });
    expect(settledPreviewState(failedPreview())?.kind).toBe("failed");
    // APPLIED is the server's last word: polling it would spend the whole
    // budget and then report a timeout contradicting the status just read.
    expect(settledPreviewState(readyPreview({ status: "APPLIED" }))?.kind).toBe("already-applied");
    expect(settledPreviewState(readyPreview({ status: "PENDING" }))).toBeNull();
    expect(settledPreviewState(readyPreview({ status: "RUNNING" }))).toBeNull();
  });
});
