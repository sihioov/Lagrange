import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { stockBetaDashboardLayout } from "@/components/stock-beta/dashboard/dashboard-layout";
import {
  renderStockBetaDashboardGrid,
  StockBetaDashboard,
} from "@/components/stock-beta/dashboard/stock-beta-dashboard";
import {
  STOCK_BETA_DASHBOARD_WIDGET_IDS,
  type StockBetaDashboardViewModel,
} from "@/components/stock-beta/dashboard/types";
import {
  stockBetaDashboardArchitecture,
  stockBetaDashboardDefinitions,
} from "@/components/stock-beta/dashboard/widget-registry";
import { stockBetaDetailArchitecture } from "@/components/stock-beta/detail/widget-registry";
import {
  defineStockBetaWidget,
  defineStockBetaWidgetArchitecture,
  stockBetaWidgetConfiguration,
  validateStockBetaWidgetArchitecture,
} from "@/components/stock-beta/shared/widget-types";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type {
  OwnerEquityV2LatestSignalsModel,
  OwnerEquityV2Lifecycle,
  OwnerEquityV2MembershipListModel,
} from "@/lib/products/equity-signals-contracts";
import {
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2SignalSchema,
} from "@/lib/products/equity-signals-contracts";

const SHA = `sha256:${"b".repeat(64)}`;
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
      snapshot_id: uuidFromNumber(601),
      universe_sha256: SHA,
    },
    rows,
    top5: rows.slice(0, 5),
  });
}

function membershipFor(index: number, lifecycle: OwnerEquityV2Lifecycle) {
  const materialized = lifecycle === "READY" || lifecycle === "DISABLED";
  return ownerEquityV2MembershipListSchema.shape.memberships.element.parse({
    coverage: {
      ...(materialized ? { first_session: "2025-08-01", last_session: "2026-08-29" } : {}),
      minimum_observed_sessions: 121,
      observed_sessions: LIFECYCLE_COVERAGE[lifecycle],
      target_observed_sessions: 261,
    },
    ...(lifecycle === "FAILED"
      ? { failure: { code: "OWNER_EQUITY_PREPARATION_FAILED", retryable: true } }
      : {}),
    ...(lifecycle === "DISABLED" ? { disabled_at: "2026-09-02T06:00:00Z" } : {}),
    generation: materialized ? 7 : 0,
    id: uuidFromNumber(index + 701),
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

function dashboardViewModel(
  signals: OwnerEquityV2LatestSignalsModel | null,
  list: OwnerEquityV2MembershipListModel = membershipsFor(),
  overrides: Partial<StockBetaDashboardViewModel> = {},
): StockBetaDashboardViewModel {
  return {
    actionError: null,
    actionMessage: null,
    busy: false,
    copy: stockBetaDictionary.en,
    disableId: null,
    inputError: null,
    instrumentCode: "",
    locale: "en",
    memberships: list.memberships,
    mutationPending: false,
    onAdd: async () => undefined,
    onCancelDisable: () => undefined,
    onConfirmDisable: async () => undefined,
    onInstrumentCodeChange: () => undefined,
    onRequestDisable: () => undefined,
    onRetry: async () => undefined,
    pendingMembershipId: null,
    policy: list.policy,
    pollError: false,
    signalState: signals === null ? { kind: "not-ready" } : { kind: "ready" },
    signals,
    ...overrides,
  };
}

function renderedRowIds(markup: string): string[] {
  return [...markup.matchAll(/data-testid="stock-beta-row-([^"]+)"/g)].flatMap((match) =>
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

function renderedWidgetTag(markup: string, widgetId: string): string {
  const tag = new RegExp(`<div(?=[^>]*data-testid="stock-beta-widget-${widgetId}")[^>]*>`).exec(
    markup,
  )?.[0];
  if (tag === undefined) throw new Error(`widget ${widgetId} was not rendered`);
  return tag;
}

type RuntimeDashboardArchitecture = {
  readonly definitions: readonly unknown[];
  readonly layout: unknown;
  readonly requiredWidgetIds: readonly string[];
};

function renderDashboardWithArchitecture(
  architecture: RuntimeDashboardArchitecture,
  viewModel: StockBetaDashboardViewModel,
): string {
  const centralArchitecture = stockBetaDashboardArchitecture as unknown as {
    definitions: readonly unknown[];
    layout: unknown;
    requiredWidgetIds: readonly string[];
  };
  const original = {
    definitions: centralArchitecture.definitions,
    layout: centralArchitecture.layout,
    requiredWidgetIds: centralArchitecture.requiredWidgetIds,
  };

  try {
    Object.assign(centralArchitecture, architecture);
    return renderToStaticMarkup(<StockBetaDashboard viewModel={viewModel} />);
  } finally {
    Object.assign(centralArchitecture, original);
  }
}

function OptionalRendererProbe({ viewModel }: { readonly viewModel: StockBetaDashboardViewModel }) {
  return (
    <p data-testid="stock-beta-optional-renderer-probe">{viewModel.policy.remaining_capacity}</p>
  );
}

describe("stock-beta V2 dashboard composition", () => {
  it("keeps the V2 registry, required regions, and responsive order explicit", () => {
    expect(validateStockBetaWidgetArchitecture(stockBetaDashboardArchitecture)).toEqual([]);
    expect(stockBetaDashboardDefinitions.map((definition) => definition.id)).toEqual(
      STOCK_BETA_DASHBOARD_WIDGET_IDS,
    );
    expect(stockBetaDashboardArchitecture.requiredWidgetIds).toEqual([
      "universe-management",
      "membership-status",
      "ranked-signals",
      "signal-profile",
      "signal-decomposition",
      "condition-matrix",
      "snapshot-tape",
      "policy-boundary",
      "provenance",
    ]);
    expect(stockBetaDashboardLayout.desktop.map((placement) => placement.id)).toEqual([
      "ranked-signals",
      "signal-profile",
      "signal-decomposition",
      "condition-matrix",
      "snapshot-tape",
      "universe-management",
      "membership-status",
      "signal-state",
      "policy-boundary",
      "provenance",
    ]);
    expect(stockBetaDashboardLayout.desktop.slice(0, 5)).toMatchObject([
      { id: "ranked-signals", column: 1, columnSpan: 3, row: 1 },
      { id: "signal-profile", column: 4, columnSpan: 6, row: 1 },
      { id: "signal-decomposition", column: 10, columnSpan: 3, row: 1 },
      { id: "condition-matrix", column: 1, columnSpan: 3, row: 2 },
      { id: "snapshot-tape", column: 4, columnSpan: 9, row: 2 },
    ]);
    expect(
      stockBetaDashboardLayout.desktop
        .filter((placement) => placement.empty.visible)
        .sort((left, right) => left.empty.order - right.empty.order)
        .map((placement) => placement.id),
    ).toEqual(["universe-management", "membership-status", "signal-state", "policy-boundary"]);
  });

  it.each([0, 31, 100])(
    "renders the V2 dashboard for %s rows without a fixed capacity",
    (rowCount) => {
      const data = latestFor(rowCount);
      const markup = renderToStaticMarkup(
        <StockBetaDashboard viewModel={dashboardViewModel(data)} />,
      );
      const tableBody = markup.match(/<tbody>([\s\S]*?)<\/tbody>/)?.[1] ?? "";

      expect(markup).toContain(`data-has-snapshot="true"`);
      expect(markup).toContain(`data-testid="stock-beta-widget-ranked-signals"`);
      if (rowCount === 0) {
        expect(markup).not.toContain('data-testid="stock-beta-rank-table"');
        expect(renderedRowIds(markup)).toEqual([]);
      } else {
        expect(tableBody.match(/<tr\b/g) ?? []).toHaveLength(rowCount);
        expect(renderedRowIds(markup)).toEqual(data.rows.map((row) => row.instrument_id));
        expect(markup).toContain(instrumentId(rowCount - 1));
      }
      expect(markup).not.toMatch(/30[- ]row/);
      expectNoLegacyEvidence(markup);
    },
  );

  it("renders exact V2 signal values in selected profile and decomposition widgets", () => {
    const data = latestFor(1);
    const row = data.rows[0];
    if (row === undefined) throw new Error("fixture row missing");
    const markup = renderToStaticMarkup(
      <StockBetaDashboard viewModel={dashboardViewModel(data)} />,
    );

    expect(markup).toContain(row.instrument_id);
    expect(markup).toContain(`Generation ${row.generation}`);
    expect(markup).toContain(`data-raw-value="${row.score}"`);
    const metricKeys = [
      "return_20",
      "return_60",
      "return_120",
      "volatility_20",
      "volatility_60",
      "volatility_120",
      "max_drawdown_120",
      "average_volume_20",
      "volume_ratio_20_60",
      "average_trading_value_20",
    ] as const;
    expect(metricKeys.filter((key) => !markup.includes(`data-metric-key="${key}"`))).toEqual([]);
    expectNoLegacyEvidence(markup);
  });

  it("keeps the initial selection synchronized across rank, preview, matrix, and decomposition", () => {
    const data = latestFor(31);
    const selected = data.top5[0];
    if (selected === undefined) throw new Error("fixture selection missing");
    const markup = renderToStaticMarkup(
      <StockBetaDashboard viewModel={dashboardViewModel(data)} />,
    );
    const escapedId = selected.instrument_id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

    expect(markup).toMatch(
      new RegExp(
        `data-selected="true"[^>]*data-testid="stock-beta-row-${escapedId}"|data-testid="stock-beta-row-${escapedId}"[^>]*data-selected="true"`,
      ),
    );
    expect(markup).toContain(`data-selected-instrument="${selected.instrument_id}"`);
    expect(markup).toContain(`data-testid="stock-beta-matrix-${selected.instrument_id}"`);
    expect(markup).toContain(`data-testid="stock-beta-signal-decomposition"`);
    expect(markup).toContain(`>${selected.instrument_id}<`);
    expect(markup).toContain(`data-testid="stock-beta-tape-${selected.instrument_id}"`);

    for (const relativePath of [
      "widgets/condition-matrix-widget.tsx",
      "widgets/ranked-signals-widget.tsx",
      "widgets/signal-decomposition-widget.tsx",
      "widgets/signal-preview-widget.tsx",
    ]) {
      const source = readFileSync(
        join(process.cwd(), "components/stock-beta/dashboard", relativePath),
        "utf8",
      );
      expect(source).toContain("useStockBetaSelection");
    }
  });

  it("presents every V2 membership lifecycle, coverage, retry, and disable boundary", () => {
    const lifecycles: readonly OwnerEquityV2Lifecycle[] = [
      "REQUESTED",
      "VALIDATING",
      "BACKFILLING",
      "MATERIALIZING",
      "READY",
      "INSUFFICIENT_HISTORY",
      "FAILED",
      "DISABLED",
    ];
    const list = membershipsFor(lifecycles);
    const markup = renderToStaticMarkup(
      <StockBetaDashboard
        viewModel={dashboardViewModel(null, list, {
          busy: true,
          pollError: true,
        })}
      />,
    );

    for (const lifecycle of lifecycles) {
      expect(markup).toContain(`data-lifecycle="${lifecycle}"`);
    }
    for (const label of [
      "Requested",
      "Validating",
      "Backfilling",
      "Materializing",
      "Ready",
      "Insufficient history",
      "Failed",
      "Disabled",
    ]) {
      expect(markup).toContain(label);
    }
    expect(markup).toContain("0/261");
    expect(markup).toContain("261/261");
    expect(markup).toContain("OWNER_EQUITY_PREPARATION_FAILED");
    expect(markup.match(/>Retry<\/button>/g) ?? []).toHaveLength(2);
    expect(markup.match(/>Open detail<\/a>/g) ?? []).toHaveLength(1);
    expect(markup.match(/>Disable<\/button>/g) ?? []).toHaveLength(7);
    expect(markup).toContain(stockBetaDictionary.en.pollingMessage);
    expect(markup).toContain(stockBetaDictionary.en.pollErrorMessage);
    expect(markup).not.toContain('data-testid="stock-beta-rank-table"');
    expect(markup).toContain("000001.KRX</strong>");
  });

  it("suppresses every signal widget when the current snapshot becomes unavailable", () => {
    const stale = signal(30);
    const markup = renderToStaticMarkup(
      <StockBetaDashboard
        viewModel={dashboardViewModel(null, membershipsFor([]), {
          signalState: { kind: "unavailable" },
        })}
      />,
    );

    expect(markup).toContain('data-has-snapshot="false"');
    expect(markup).toContain(stockBetaDictionary.en.signalUnavailableMessage);
    expect(markup).not.toContain(stale.instrument_id);
    for (const widgetId of [
      "ranked-signals",
      "signal-profile",
      "signal-decomposition",
      "condition-matrix",
      "snapshot-tape",
      "provenance",
    ]) {
      expect(markup).not.toContain(`data-testid="stock-beta-widget-${widgetId}"`);
    }
  });

  it("accepts registry-driven optional removal and reordering while retaining required widgets", () => {
    const desktop = stockBetaDashboardArchitecture.layout.desktop;
    const tablet = stockBetaDashboardArchitecture.layout.tablet;
    const mobile = stockBetaDashboardArchitecture.layout.mobile;
    const reorder = <T extends { readonly order: number }>(
      placements: readonly T[],
      predicate: (placement: T) => boolean,
      offset = 0,
    ) =>
      placements
        .filter(predicate)
        .map((placement, order) => ({ ...placement, order: order + offset }));
    const candidate = {
      definitions: stockBetaDashboardArchitecture.definitions,
      requiredWidgetIds: stockBetaDashboardArchitecture.requiredWidgetIds,
      layout: {
        desktop: [
          ...reorder(desktop, (placement) => placement.id === "universe-management", 0),
          ...reorder(desktop, (placement) => placement.id === "ranked-signals", 1),
          ...reorder(
            desktop,
            (placement) =>
              placement.id !== "universe-management" && placement.id !== "ranked-signals",
          ).map((placement, order) => ({ ...placement, order: order + 2 })),
        ],
        tablet: reorder(tablet, (placement) => placement.id !== "signal-state"),
        mobile: reorder(mobile, (placement) => placement.id !== "signal-state"),
      },
    };

    expect(validateStockBetaWidgetArchitecture(candidate)).toEqual([]);
    const configuration = stockBetaWidgetConfiguration(candidate);
    expect(configuration.layout.desktop.slice(0, 2).map((placement) => placement.id)).toEqual([
      "universe-management",
      "ranked-signals",
    ]);
    expect(configuration.layout.tablet.some((placement) => placement.id === "signal-state")).toBe(
      false,
    );
    expect(JSON.parse(JSON.stringify(configuration))).toEqual(configuration);
  });

  it("applies reordered optional-widget placement metadata in the actual dashboard renderer", () => {
    const reorderedDesktop = stockBetaDashboardArchitecture.layout.desktop.map((placement) => {
      if (placement.id === "signal-state") {
        return {
          ...placement,
          empty: { ...placement.empty, column: 8, columnSpan: 5, row: 1, order: 1 },
        };
      }
      if (placement.id === "membership-status") {
        return {
          ...placement,
          empty: { ...placement.empty, column: 1, columnSpan: 12, row: 2, order: 2 },
        };
      }
      return placement;
    });
    const reorderedArchitecture = defineStockBetaWidgetArchitecture({
      ...stockBetaDashboardArchitecture,
      layout: { ...stockBetaDashboardArchitecture.layout, desktop: reorderedDesktop },
    });
    const viewModel = dashboardViewModel(null);
    const defaultMarkup = renderToStaticMarkup(
      renderStockBetaDashboardGrid(stockBetaDashboardArchitecture, viewModel),
    );
    const reorderedMarkup = renderToStaticMarkup(
      renderStockBetaDashboardGrid(reorderedArchitecture, viewModel),
    );
    const defaultSignalState = renderedWidgetTag(defaultMarkup, "signal-state");
    const reorderedSignalState = renderedWidgetTag(reorderedMarkup, "signal-state");

    expect(defaultSignalState).toContain("--desktop-grid-column:1");
    expect(defaultSignalState).toContain("--desktop-grid-column-span:12");
    expect(defaultSignalState).toContain("--desktop-grid-row:2");
    expect(defaultSignalState).toContain("--desktop-order:2");
    expect(reorderedSignalState).toContain("--desktop-grid-column:8");
    expect(reorderedSignalState).toContain("--desktop-grid-column-span:5");
    expect(reorderedSignalState).toContain("--desktop-grid-row:1");
    expect(reorderedSignalState).toContain("--desktop-order:1");
    expect(reorderedSignalState).not.toBe(defaultSignalState);
  });

  it("removes an optional widget from the real StockBetaDashboard render via architecture only", () => {
    const viewModel = dashboardViewModel(null);
    const baselineMarkup = renderToStaticMarkup(<StockBetaDashboard viewModel={viewModel} />);
    const architectureWithoutSignalState = defineStockBetaWidgetArchitecture({
      definitions: stockBetaDashboardArchitecture.definitions.filter(
        (definition) => definition.id !== "signal-state",
      ),
      requiredWidgetIds: [...stockBetaDashboardArchitecture.requiredWidgetIds],
      layout: {
        desktop: stockBetaDashboardArchitecture.layout.desktop.filter(
          (placement) => placement.id !== "signal-state",
        ),
        tablet: stockBetaDashboardArchitecture.layout.tablet.filter(
          (placement) => placement.id !== "signal-state",
        ),
        mobile: stockBetaDashboardArchitecture.layout.mobile.filter(
          (placement) => placement.id !== "signal-state",
        ),
      },
    });
    const removedMarkup = renderDashboardWithArchitecture(
      architectureWithoutSignalState,
      viewModel,
    );

    expect(baselineMarkup).toContain('data-testid="stock-beta-widget-signal-state"');
    expect(removedMarkup).not.toContain('data-testid="stock-beta-widget-signal-state"');
    expect(removedMarkup).not.toContain(stockBetaDictionary.en.notReadyMessage);
  });

  it("adds an optional widget to the real StockBetaDashboard with metadata-driven placement", () => {
    const definitionOrder =
      Math.max(
        ...stockBetaDashboardArchitecture.definitions.map((definition) => definition.order),
      ) + 1;
    const desktopOrder =
      Math.max(
        ...stockBetaDashboardArchitecture.layout.desktop.map((placement) => placement.order),
      ) + 1;
    const tabletOrder =
      Math.max(
        ...stockBetaDashboardArchitecture.layout.tablet.map((placement) => placement.order),
      ) + 1;
    const desktopRow =
      Math.max(...stockBetaDashboardArchitecture.layout.desktop.map((placement) => placement.row)) +
      1;
    const tabletRow =
      Math.max(...stockBetaDashboardArchitecture.layout.tablet.map((placement) => placement.row)) +
      1;
    const optionalDefinition = defineStockBetaWidget({
      id: "optional-renderer-probe",
      component: OptionalRendererProbe,
      defaultSize: "full",
      required: false,
      defaultVisible: true,
      order: definitionOrder,
    });
    const desktopPlacement = {
      id: optionalDefinition.id,
      size: "full",
      column: 2,
      columnSpan: 10,
      row: desktopRow,
      visible: true,
      order: desktopOrder,
      empty: { column: 1, columnSpan: 12, row: desktopRow, visible: false, order: desktopOrder },
    } as const;
    const tabletPlacement = {
      id: optionalDefinition.id,
      size: "large",
      column: 3,
      columnSpan: 8,
      row: tabletRow,
      visible: true,
      order: tabletOrder,
      empty: { column: 1, columnSpan: 12, row: tabletRow, visible: false, order: tabletOrder },
    } as const;
    const architectureWithOptionalWidget = defineStockBetaWidgetArchitecture({
      definitions: [...stockBetaDashboardArchitecture.definitions, optionalDefinition],
      requiredWidgetIds: [...stockBetaDashboardArchitecture.requiredWidgetIds],
      layout: {
        desktop: [...stockBetaDashboardArchitecture.layout.desktop, desktopPlacement],
        tablet: [...stockBetaDashboardArchitecture.layout.tablet, tabletPlacement],
        mobile: [...stockBetaDashboardArchitecture.layout.mobile],
      },
    });
    const markup = renderDashboardWithArchitecture(
      architectureWithOptionalWidget,
      dashboardViewModel(latestFor(1)),
    );
    const optionalWidgetTag = renderedWidgetTag(markup, optionalDefinition.id);

    expect(markup).toContain('data-testid="stock-beta-optional-renderer-probe"');
    expect(optionalWidgetTag).toContain(`--desktop-grid-column:${desktopPlacement.column}`);
    expect(optionalWidgetTag).toContain(
      `--desktop-grid-column-span:${desktopPlacement.columnSpan}`,
    );
    expect(optionalWidgetTag).toContain(`--desktop-grid-row:${desktopPlacement.row}`);
    expect(optionalWidgetTag).toContain(`--desktop-order:${desktopPlacement.order}`);
    expect(optionalWidgetTag).toContain(`--tablet-grid-column:${tabletPlacement.column}`);
    expect(optionalWidgetTag).toContain(`--tablet-grid-column-span:${tabletPlacement.columnSpan}`);
    expect(optionalWidgetTag).toContain(`--tablet-grid-row:${tabletPlacement.row}`);
    expect(optionalWidgetTag).toContain(`--tablet-order:${tabletPlacement.order}`);
  });

  it("keeps widget-specific CSS free of grid placement rules", () => {
    const css = readFileSync(
      join(process.cwd(), "components/stock-beta/dashboard/dashboard.module.css"),
      "utf8",
    );
    const widgetSpecificBlocks =
      css.match(/\.dashboardWidget\[data-widget-id=[^\]]+\][^{]*\{[^}]*\}/g) ?? [];

    expect(css).toContain("grid-column: var(--desktop-grid-column)");
    expect(css).toContain("grid-column: var(--tablet-grid-column)");
    expect(widgetSpecificBlocks.some((block) => /grid-(?:column|row)|\border:/.test(block))).toBe(
      false,
    );
  });

  it("keeps V2 dashboard/detail configuration serializable and widgets free of direct fetches", () => {
    const dashboardConfiguration = stockBetaWidgetConfiguration(stockBetaDashboardArchitecture);
    const detailConfiguration = stockBetaWidgetConfiguration(stockBetaDetailArchitecture);
    for (const configuration of [dashboardConfiguration, detailConfiguration]) {
      expect(JSON.parse(JSON.stringify(configuration))).toEqual(configuration);
      expect(JSON.stringify(configuration)).not.toContain("component");
    }

    const dashboardRoot = join(process.cwd(), "components/stock-beta/dashboard");
    const widgetFiles = readdirSync(join(dashboardRoot, "widgets"), { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith(".tsx"))
      .map((entry) => join(dashboardRoot, "widgets", entry.name));
    for (const file of widgetFiles) {
      const source = readFileSync(file, "utf8");
      expect(source).not.toMatch(/\bfetch\s*\(/);
      expect(source).not.toMatch(/getOwnerEquityV2|screenOwnerEquityV2/);
    }
  });

  it("keeps only interactive V2 controls behind the client boundary", () => {
    const dashboardRoot = join(process.cwd(), "components/stock-beta/dashboard");
    const clientFiles = [
      "column-visibility-control.tsx",
      "instrument-search.tsx",
      "selection-provider.tsx",
      "widgets/condition-matrix-widget.tsx",
      "widgets/membership-status-widget.tsx",
      "widgets/ranked-signals-widget.tsx",
      "widgets/signal-decomposition-widget.tsx",
      "widgets/signal-preview-widget.tsx",
      "widgets/universe-management-widget.tsx",
    ];
    const dashboardFiles = [
      ...readdirSync(dashboardRoot, { withFileTypes: true })
        .filter((entry) => entry.isFile() && /\.tsx?$/.test(entry.name))
        .map((entry) => entry.name),
      ...readdirSync(join(dashboardRoot, "widgets"), { withFileTypes: true })
        .filter((entry) => entry.isFile() && /\.tsx?$/.test(entry.name))
        .map((entry) => `widgets/${entry.name}`),
    ];
    const actualClientFiles = dashboardFiles.filter((file) =>
      readFileSync(join(dashboardRoot, file), "utf8").trimStart().startsWith('"use client"'),
    );

    expect(actualClientFiles.sort()).toEqual(clientFiles.sort());
  });
});
