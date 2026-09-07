import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { StockBetaChartLoadRequest } from "@/components/stock-beta/chart-load-coordinator";
import {
  type StockBetaProfileTabDefinition,
  stockBetaProfileTabs,
} from "@/components/stock-beta/dashboard/profile-tab-registry";
import { StockBetaSelectionProvider } from "@/components/stock-beta/dashboard/selection-provider";
import type {
  StockBetaChartState,
  StockBetaDashboardViewModel,
} from "@/components/stock-beta/dashboard/types";
import { SignalPreviewWidget } from "@/components/stock-beta/dashboard/widgets/signal-preview-widget";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import {
  OWNER_EQUITY_V2_CHART_RANGE_VALUES,
  type OwnerEquityV2ChartModel,
  type OwnerEquityV2ChartRange,
  type OwnerEquityV2SignalModel,
  ownerEquityV2ChartSchema,
  ownerEquityV2SignalSchema,
} from "@/lib/products/equity-signals-contracts";

const INSTRUMENT_ID = "005930.KRX";
const SNAPSHOT_ID = "00000000-0000-4000-8000-000000000101";
const AS_OF = "2026-09-03";

const ROW = ownerEquityV2SignalSchema.parse({
  average_trading_value_20: 1_000_000,
  average_volume_20: 25_000,
  condition: "BULLISH",
  generation: 3,
  instrument_id: INSTRUMENT_ID,
  max_drawdown_120: -0.2,
  rank: 1,
  return_120: 0.4,
  return_20: 0.1,
  return_60: 0.2,
  score: 1.2,
  sma_20: 71_425.5,
  sma_60: 69_882.25,
  volatility_120: 0.3,
  volatility_20: 0.1,
  volatility_60: 0.2,
  volume_ratio_20_60: 1.1,
});

const CHART = ownerEquityV2ChartSchema.parse({
  as_of: AS_OF,
  bars: [
    {
      close: 70_900,
      high: 71_200,
      low: 70_700,
      open: 71_100,
      session_date: "2026-09-01",
      sma_20: null,
      sma_60: null,
      volume: 10_000,
    },
    {
      close: 71_000,
      high: 71_300,
      low: 70_800,
      open: 71_000,
      session_date: "2026-09-02",
      sma_20: 71_000,
      sma_60: null,
      volume: 0,
    },
    {
      close: 72_100,
      high: 72_500,
      low: 71_600,
      open: 71_800,
      session_date: AS_OF,
      sma_20: 71_425.5,
      sma_60: 69_882.25,
      volume: 12_345_678,
    },
  ],
  expected_as_of: AS_OF,
  freshness: "CURRENT",
  generation: 3,
  instrument_id: INSTRUMENT_ID,
  latest: {
    change: 1_100,
    change_rate: 0.0069832402,
    close: 72_100,
    session_date: AS_OF,
    volume: 12_345_678,
  },
  price_semantics: "ORIGINAL_UNADJUSTED",
  range: "1y",
  snapshot_id: SNAPSHOT_ID,
  warnings: ["NOT_REALTIME", "CORPORATE_ACTIONS_NOT_ADJUSTED", "RESEARCH_ONLY"],
});

const CHART_REQUEST = {
  asOf: AS_OF,
  generation: CHART.generation,
  instrumentId: INSTRUMENT_ID,
  range: "1y",
  snapshotId: SNAPSHOT_ID,
} satisfies StockBetaChartLoadRequest;

const POLICY = {
  active_instruments: 1,
  max_active_instruments: 100,
  minimum_observed_sessions: 121,
  remaining_capacity: 99,
  target_observed_sessions: 261,
} as const;

type PreviewOptions = {
  readonly chartData?: OwnerEquityV2ChartModel | null;
  readonly chartRange?: OwnerEquityV2ChartRange;
  readonly chartState?: StockBetaChartState;
  readonly locale?: "en" | "ko";
  readonly rows?: readonly OwnerEquityV2SignalModel[];
};

function chartFor(overrides: Partial<OwnerEquityV2ChartModel> = {}): OwnerEquityV2ChartModel {
  return ownerEquityV2ChartSchema.parse({ ...CHART, ...overrides });
}

function viewModel({
  chartData = CHART,
  chartRange = "1y",
  chartState = { kind: "ready" },
  locale = "en",
}: Omit<PreviewOptions, "rows"> = {}): StockBetaDashboardViewModel {
  return {
    actionError: null,
    actionMessage: null,
    busy: false,
    chartData,
    chartError: null,
    chartRange,
    chartState,
    copy: stockBetaDictionary[locale],
    disableId: null,
    inputError: null,
    instrumentCode: "",
    locale,
    memberships: [],
    mutationPending: false,
    onAdd: async () => undefined,
    onCancelDisable: () => undefined,
    onChartRangeChange: () => undefined,
    onConfirmDisable: async () => undefined,
    onInstrumentCodeChange: () => undefined,
    onRequestDisable: () => undefined,
    onRetry: async () => undefined,
    pendingMembershipId: null,
    policy: POLICY,
    pollError: false,
    selectedInstrumentId: INSTRUMENT_ID,
    signalState: { kind: "ready" },
    signals: null,
  };
}

function renderPreview({
  chartData = CHART,
  chartRange = "1y",
  chartState = { kind: "ready" },
  locale = "en",
  rows = [ROW],
}: PreviewOptions = {}): string {
  return renderToStaticMarkup(
    <StockBetaSelectionProvider initialSelectedInstrumentId={INSTRUMENT_ID} rows={rows}>
      <SignalPreviewWidget viewModel={viewModel({ chartData, chartRange, chartState, locale })} />
    </StockBetaSelectionProvider>,
  );
}

type TestElementProps = {
  readonly children?: ReactNode;
  readonly [key: string]: unknown;
};

function descendantElements(node: ReactNode): readonly ReactElement<TestElementProps>[] {
  if (Array.isArray(node)) return node.flatMap((child) => descendantElements(child));
  if (!isValidElement(node)) return [];
  const element = node as ReactElement<TestElementProps>;
  return [element, ...descendantElements(element.props.children)];
}

function expectSingleProfileTabTarget(markup: string, tabId: string): void {
  expect(markup.split(`id="${tabId}"`).length - 1).toBe(1);
  expect(markup.split(`aria-labelledby="${tabId}"`).length - 1).toBe(1);
}

const mutableProfileTabs = stockBetaProfileTabs as unknown as StockBetaProfileTabDefinition[];
const originalProfileTabs = [...stockBetaProfileTabs];

afterEach(() => {
  mutableProfileTabs.splice(0, mutableProfileTabs.length, ...originalProfileTabs);
});

describe("Stock Beta chart-capable signal profile", () => {
  it("defaults to Price and renders the exact latest EOD values and accessible chart summary", () => {
    const markup = renderPreview();

    expect(markup).toContain('data-testid="stock-beta-signal-preview"');
    expect(markup).toContain('data-testid="stock-beta-price-chart"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-summary"');
    expect(markup).toContain('data-selected-instrument="005930.KRX"');
    expect(markup).toContain('aria-selected="true"');
    expect(markup).toContain('data-raw-value="72100"');
    expect(markup).toContain('<data value="72100">72,100.00</data>');
    expect(markup).toContain("+1,100.00");
    expect(markup).toContain("<small>+0.70%</small>");
    expect(markup).toContain('data-raw-value="12345678"');
    expect(markup).toContain("12,345,678");
    expect(markup).toContain(AS_OF);
    expect(markup).toContain("Selected observation");
    expect(markup).toContain("Date: 2026-09-03");
    expect(markup).toContain("Open: 71,800");
    expect(markup).toContain("High: 72,500");
    expect(markup).toContain("Low: 71,600");
    expect(markup).toContain("Close: 72,100");
    expect(markup).toContain("Volume: 12,345,678");
    expect(markup).toContain('aria-keyshortcuts="ArrowLeft ArrowRight Home End"');
    expect(markup).toContain(stockBetaDictionary.en.chartKeyboardInstructions);
  });

  it("exposes all four tabs with Price selected and localized labels", () => {
    const markup = renderPreview();
    const tabButtons = markup.match(/<button[^>]*role="tab"[^>]*>/g) ?? [];

    expect(tabButtons).toHaveLength(4);
    expect(tabButtons[0]).toContain('aria-selected="true"');
    expect(tabButtons.slice(1).every((button) => button.includes('aria-selected="false"'))).toBe(
      true,
    );
    expect(markup).toContain('role="tablist"');
    expect(markup).toContain('aria-label="Signal metrics"');
    expect(markup).toContain('role="tabpanel"');
    expectSingleProfileTabTarget(markup, "stock-beta-profile-tab-005930.KRX-price");
    for (const label of ["Price", "Returns", "Volatility", "Activity"]) {
      expect(markup).toContain(`>${label}<`);
    }
  });

  it("keeps the central profile-tab registry in the exact Price/Returns/Volatility/Activity order", () => {
    expect(stockBetaProfileTabs.map((tab) => tab.id)).toEqual([
      "price",
      "returns",
      "volatility",
      "activity",
    ]);
    expect(stockBetaProfileTabs.map((tab) => tab.label(stockBetaDictionary.en))).toEqual([
      "Price",
      "Returns",
      "Volatility",
      "Activity",
    ]);
  });

  it("normalizes fallback selection and ARIA references when tabs are reordered, removed, duplicated, or empty", () => {
    const activity = originalProfileTabs[3];
    if (activity === undefined) throw new Error("Activity profile tab is missing");

    mutableProfileTabs.splice(
      0,
      mutableProfileTabs.length,
      activity,
      ...originalProfileTabs.filter((tab) => tab.id !== activity.id),
    );
    const reorderedMarkup = renderPreview();
    expect(reorderedMarkup.indexOf(">Activity<")).toBeLessThan(reorderedMarkup.indexOf(">Price<"));
    expect(reorderedMarkup.match(/aria-selected="true"/g)).toHaveLength(1);
    expectSingleProfileTabTarget(reorderedMarkup, "stock-beta-profile-tab-005930.KRX-activity");

    mutableProfileTabs.splice(0, mutableProfileTabs.length, activity);
    const removedMarkup = renderPreview();
    expect(removedMarkup.match(/role="tab"/g)).toHaveLength(1);
    expect(removedMarkup).toContain(">Activity<");
    expect(removedMarkup.match(/aria-selected="true"/g)).toHaveLength(1);
    expectSingleProfileTabTarget(removedMarkup, "stock-beta-profile-tab-005930.KRX-activity");
    expect(removedMarkup).toContain(stockBetaDictionary.en.averageVolumeLabel);
    expect(removedMarkup).not.toContain('data-testid="stock-beta-price-chart"');

    const injected: StockBetaProfileTabDefinition = {
      id: "price",
      label: () => "Registry probe",
      renderer: () => createElement("p", { "data-testid": "registry-panel-probe" }, "registry"),
    };
    mutableProfileTabs.splice(0, mutableProfileTabs.length, injected, ...originalProfileTabs);

    const duplicateMarkup = renderPreview();
    expect(duplicateMarkup.match(/role="tab"/g)).toHaveLength(originalProfileTabs.length);
    expect(duplicateMarkup).toContain(">Registry probe<");
    expect(duplicateMarkup).toContain('data-testid="registry-panel-probe"');
    expect(duplicateMarkup.match(/aria-selected="true"/g)).toHaveLength(1);
    expectSingleProfileTabTarget(duplicateMarkup, "stock-beta-profile-tab-005930.KRX-price");

    mutableProfileTabs.splice(0, mutableProfileTabs.length);
    const emptyMarkup = renderPreview();
    expect(emptyMarkup).toBe("");
  });

  it("renders a new unique profile tab and panel through the public registry path", () => {
    const injected: StockBetaProfileTabDefinition = {
      id: "registry-probe",
      label: () => "Registry probe",
      renderer: () => createElement("p", { "data-testid": "registry-probe-panel" }, "registry"),
    };
    mutableProfileTabs.splice(0, mutableProfileTabs.length, injected, ...originalProfileTabs);

    const markup = renderPreview();
    const tabId = `stock-beta-profile-tab-${INSTRUMENT_ID}-registry-probe`;
    const panelId = `stock-beta-profile-panel-${INSTRUMENT_ID}`;

    expect(markup.match(/role="tab"/g)).toHaveLength(originalProfileTabs.length + 1);
    expect(markup).toContain(">Registry probe<");
    expect(markup).toContain('data-testid="registry-probe-panel"');
    expect(markup).toContain(`id="${tabId}"`);
    expect(markup).toContain(`aria-controls="${panelId}"`);
    expect(markup).toContain(`aria-labelledby="${tabId}"`);
    expect(markup.match(/aria-selected="true"/g)).toHaveLength(1);
    expectSingleProfileTabTarget(markup, tabId);
  });

  it("normalizes a stale selected tab ID when its registry entry is absent", async () => {
    vi.resetModules();
    vi.doMock("react", async () => {
      const actual = await vi.importActual<typeof import("react")>("react");
      return {
        ...actual,
        useState: <T,>(initialState: T) => {
          const staleState = initialState === "activity" ? ("price" as T) : initialState;
          return [staleState, vi.fn()];
        },
      };
    });

    try {
      const registry = await import("@/components/stock-beta/dashboard/profile-tab-registry");
      const isolatedProfileTabs =
        registry.stockBetaProfileTabs as unknown as StockBetaProfileTabDefinition[];
      const isolatedActivity = isolatedProfileTabs[3];
      if (isolatedActivity === undefined) throw new Error("Activity profile tab is missing");
      isolatedProfileTabs.splice(0, isolatedProfileTabs.length, isolatedActivity);

      const [
        { SignalPreviewWidget: MockedSignalPreviewWidget },
        { StockBetaSelectionProvider: MockedSelectionProvider },
      ] = await Promise.all([
        import("@/components/stock-beta/dashboard/widgets/signal-preview-widget"),
        import("@/components/stock-beta/dashboard/selection-provider"),
      ]);
      const markup = renderToStaticMarkup(
        <MockedSelectionProvider initialSelectedInstrumentId={INSTRUMENT_ID} rows={[ROW]}>
          <MockedSignalPreviewWidget viewModel={viewModel()} />
        </MockedSelectionProvider>,
      );

      expect(markup.match(/role="tab"/g)).toHaveLength(1);
      expect(markup).toContain(">Activity<");
      expect(markup).toContain('aria-selected="true"');
      expectSingleProfileTabTarget(markup, "stock-beta-profile-tab-005930.KRX-activity");
    } finally {
      vi.doUnmock("react");
      vi.resetModules();
    }
  });

  it("ignores whitespace-only profile tab IDs before selecting and labelling the fallback", () => {
    const activity = originalProfileTabs[3];
    if (activity === undefined) throw new Error("Activity profile tab is missing");
    const whitespace: StockBetaProfileTabDefinition = {
      id: " \t ",
      label: () => "Whitespace probe",
      renderer: () => createElement("p", { "data-testid": "whitespace-probe-panel" }, "invalid"),
    };
    mutableProfileTabs.splice(0, mutableProfileTabs.length, whitespace, activity);

    const markup = renderPreview();

    expect(markup.match(/role="tab"/g)).toHaveLength(1);
    expect(markup).toContain(">Activity<");
    expect(markup).not.toContain(">Whitespace probe<");
    expect(markup).not.toContain('data-testid="whitespace-probe-panel"');
    expectSingleProfileTabTarget(markup, "stock-beta-profile-tab-005930.KRX-activity");
  });

  it("invokes the range callback once for each ordered canonical range button", () => {
    const onChartRangeChange = vi.fn<(range: OwnerEquityV2ChartRange) => void>();
    const model = viewModel({ chartData: CHART, chartState: { kind: "ready" } });
    const profile = stockBetaProfileTabs[0];
    if (profile === undefined) throw new Error("Price profile tab is missing");

    const rendered = profile.renderer({
      selectedRow: ROW,
      viewModel: { ...model, onChartRangeChange },
    });
    const rangeButtons = descendantElements(rendered).filter(
      (element) =>
        element.type === "button" &&
        typeof element.props["aria-pressed"] === "boolean" &&
        typeof element.props["onClick"] === "function",
    );

    expect(rangeButtons).toHaveLength(OWNER_EQUITY_V2_CHART_RANGE_VALUES.length);
    expect(rangeButtons.map((button) => button.props.children)).toEqual(
      OWNER_EQUITY_V2_CHART_RANGE_VALUES.map((range) =>
        stockBetaDictionary.en.chartRangeOption(range),
      ),
    );
    expect(rangeButtons.map((button) => button.props["aria-pressed"])).toEqual(
      OWNER_EQUITY_V2_CHART_RANGE_VALUES.map((range) => range === CHART.range),
    );

    for (const button of rangeButtons) {
      const onClick = button.props["onClick"];
      if (typeof onClick !== "function") throw new Error("range button callback is missing");
      onClick();
    }

    expect(onChartRangeChange).toHaveBeenCalledTimes(OWNER_EQUITY_V2_CHART_RANGE_VALUES.length);
    expect(onChartRangeChange.mock.calls.map(([range]) => range)).toEqual(
      OWNER_EQUITY_V2_CHART_RANGE_VALUES,
    );
  });

  it.each([
    ["not-ready", { kind: "ready" } as const, null, "chartNotReadyMessage" as const, false],
    ["preparing", { kind: "idle" } as const, CHART, "chartPreparingMessage" as const, false],
    [
      "unavailable",
      { code: "OWNER_EQUITY_CHART_UNAVAILABLE", kind: "unavailable" } as const,
      CHART,
      "chartPreparingMessage" as const,
      false,
    ],
    [
      "integrity",
      { code: "OWNER_EQUITY_INTEGRITY_FAILED", kind: "integrity" } as const,
      CHART,
      "chartIntegrityMessage" as const,
      true,
    ],
    [
      "network error",
      { code: "NETWORK_ERROR", kind: "error" } as const,
      CHART,
      "chartNetworkMessage" as const,
      true,
    ],
    [
      "forbidden",
      { code: "FORBIDDEN", kind: "error" } as const,
      CHART,
      "chartNetworkMessage" as const,
      true,
    ],
  ] as const)("fails closed for the %s chart state", (_name, state, chartData, copyKey, alert) => {
    const markup = renderPreview({ chartData, chartState: state });
    const message = stockBetaDictionary.en[copyKey];

    expect(markup).toContain(message);
    expect(markup).not.toContain("72100");
    expect(markup).not.toContain(AS_OF);
    expect(markup).not.toContain('data-testid="stock-beta-price-chart"');
    if (alert) expect(markup).toContain('role="alert"');
    else expect(markup).not.toContain('role="alert"');
  });

  it("shows loading copy without a previous chart, while an existing chart stays busy during update", () => {
    const loadingWithoutData = renderPreview({
      chartData: null,
      chartState: { kind: "loading", request: CHART_REQUEST },
    });
    expect(loadingWithoutData).toContain(stockBetaDictionary.en.chartLoadingMessage);
    expect(loadingWithoutData).not.toContain("72100");
    expect(loadingWithoutData).not.toContain('data-testid="stock-beta-price-chart"');

    const loadingWithData = renderPreview({
      chartData: CHART,
      chartState: { kind: "loading", request: CHART_REQUEST },
    });
    expect(loadingWithData).toContain('aria-busy="true"');
    expect(loadingWithData).toContain(stockBetaDictionary.en.chartUpdatingLabel);
    expect(loadingWithData).toContain("72100");
  });

  it("keeps the previous chart range selected while a cross-range update is loading", () => {
    const markup = renderPreview({
      chartRange: "1m",
      chartState: { kind: "loading", request: { ...CHART_REQUEST, range: "1m" } },
    });

    expect(markup).toContain('data-testid="stock-beta-price-chart"');
    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain(stockBetaDictionary.en.chartUpdatingLabel);
    expect(markup).toContain("72,100.00");
    expect(markup).toMatch(/<button[^>]*aria-pressed="true"[^>]*>1Y<\/button>/);
    expect(markup).toMatch(/<button[^>]*aria-pressed="false"[^>]*>1M<\/button>/);
  });

  it("distinguishes an empty selection from chart not-ready state", () => {
    const emptyMarkup = renderPreview({ rows: [] });
    expect(emptyMarkup).toContain(stockBetaDictionary.en.previewEmptyMessage);
    expect(emptyMarkup).not.toContain('data-testid="stock-beta-price-chart"');

    const notReadyMarkup = renderPreview({ chartData: null, chartState: { kind: "ready" } });
    expect(notReadyMarkup).toContain(stockBetaDictionary.en.chartNotReadyMessage);
    expect(notReadyMarkup).not.toContain("72100");
  });

  it("fails closed when the selected row and otherwise-ready chart name different instruments", () => {
    const mismatchedChart = chartFor({ instrument_id: "000660.KRX" });
    const markup = renderPreview({ chartData: mismatchedChart });

    expect(markup).toContain(stockBetaDictionary.en.chartNotReadyMessage);
    expect(markup).not.toContain('data-testid="stock-beta-price-chart"');
    expect(markup).not.toContain("72,100.00");
    expect(markup).not.toContain("+1,100.00");
  });

  it.each([
    ["STALE", "2026-09-02", "2026-09-02"],
    ["UNVERIFIABLE", null, null],
  ] as const)(
    "shows the %s freshness warning without changing the latest EOD values",
    (freshness, expectedAsOf, _warningDate) => {
      const markup = renderPreview({
        chartData: chartFor({ freshness, expected_as_of: expectedAsOf }),
      });

      if (freshness === "STALE") {
        expect(markup).toContain(stockBetaDictionary.en.staleChartMessage(AS_OF, expectedAsOf));
      } else {
        expect(markup).toContain(stockBetaDictionary.en.unverifiableChartMessage);
      }
      expect(markup).toContain("72,100.00");
      expect(markup).toContain(AS_OF);
    },
  );

  it.each([
    [
      "en",
      "Completed EOD close",
      "Original / unadjusted price",
      "Corporate actions are not adjusted and can create discontinuities or distort returns.",
      "Low",
      "Loading EOD chart data…",
    ],
    [
      "ko",
      "완료 EOD 종가",
      "원주가 / 비조정 가격",
      "기업행동은 조정하지 않아 차트 단절 또는 수익률 왜곡이 생길 수 있습니다.",
      "저가",
      "EOD 차트 데이터를 불러오는 중…",
    ],
  ] as const)(
    "keeps exact %s EOD, original-price, corporate-action, Low, and loading copy",
    (locale, eod, original, corporateAction, low, loading) => {
      const markup = renderPreview({ locale });
      expect(markup).toContain(eod);
      expect(markup).toContain(original);
      expect(markup).toContain(corporateAction);
      expect(markup).toContain(low);

      const loadingMarkup = renderPreview({
        chartData: null,
        chartState: { kind: "loading", request: CHART_REQUEST },
        locale,
      });
      expect(loadingMarkup).toContain(loading);
    },
  );

  it("uses textual direction labels and shape markers in addition to direction metadata", () => {
    const markup = renderPreview();

    expect(markup).toContain('data-direction="up"');
    expect(markup).toContain('data-direction="down"');
    expect(markup).toContain('data-direction="flat"');
    expect(markup).toContain(">Up<");
    expect(markup).toContain(">Down<");
    expect(markup).toContain(">Unchanged<");
    expect(markup).toContain(">▲</span>");
    expect(markup).toContain(">▼</span>");
    expect(markup).toContain(">—</span>");
  });
});
