import type { CSSProperties, ReactNode } from "react";
import type {
  StockBetaWidgetArchitecture,
  StockBetaWidgetBreakpoint,
  StockBetaWidgetDefinition,
  StockBetaWidgetGridLayout,
  StockBetaWidgetGridState,
  StockBetaWidgetSize,
} from "../shared/widget-types";
import styles from "./dashboard.module.css";
import { StockBetaSelectionProvider } from "./selection-provider";
import type {
  StockBetaDashboardViewModel,
  StockBetaDashboardWidgetId,
  StockBetaDashboardWidgetViewModel,
} from "./types";
import { stockBetaDashboardArchitecture } from "./widget-registry";

type DashboardLayoutStyle = CSSProperties & {
  readonly "--desktop-grid-column": number;
  readonly "--desktop-grid-column-span": number;
  readonly "--desktop-grid-row": number;
  readonly "--desktop-order": number;
  readonly "--mobile-grid-column": number;
  readonly "--mobile-grid-column-span": number;
  readonly "--mobile-grid-row": number;
  readonly "--mobile-order": number;
  readonly "--tablet-grid-column": number;
  readonly "--tablet-grid-column-span": number;
  readonly "--tablet-grid-row": number;
  readonly "--tablet-order": number;
};

type DashboardArchitecture = StockBetaWidgetArchitecture<
  readonly StockBetaWidgetDefinition<
    StockBetaDashboardWidgetId,
    StockBetaDashboardWidgetViewModel
  >[],
  StockBetaWidgetGridLayout<StockBetaDashboardWidgetId>
>;

type ResolvedWidgetPlacement = StockBetaWidgetGridState & {
  readonly size: StockBetaWidgetSize;
};

function placementFor(
  architecture: DashboardArchitecture,
  id: StockBetaDashboardWidgetId,
  breakpoint: StockBetaWidgetBreakpoint,
  hasSnapshot: boolean,
): ResolvedWidgetPlacement | undefined {
  const placement = architecture.layout[breakpoint].find((candidate) => candidate.id === id);
  if (placement === undefined) return undefined;
  const state = hasSnapshot ? placement : placement.empty;
  return { ...state, size: placement.size };
}

function layoutStyle(
  desktop: ResolvedWidgetPlacement | undefined,
  tablet: ResolvedWidgetPlacement | undefined,
  mobile: ResolvedWidgetPlacement | undefined,
): DashboardLayoutStyle {
  return {
    "--desktop-grid-column": desktop?.column ?? 1,
    "--desktop-grid-column-span": desktop?.columnSpan ?? 12,
    "--desktop-grid-row": desktop?.row ?? 1,
    "--desktop-order": desktop?.order ?? 99,
    "--tablet-grid-column": tablet?.column ?? 1,
    "--tablet-grid-column-span": tablet?.columnSpan ?? 12,
    "--tablet-grid-row": tablet?.row ?? 1,
    "--tablet-order": tablet?.order ?? 99,
    "--mobile-grid-column": mobile?.column ?? 1,
    "--mobile-grid-column-span": mobile?.columnSpan ?? 1,
    "--mobile-grid-row": mobile?.row ?? 1,
    "--mobile-order": mobile?.order ?? 99,
  };
}

export function renderStockBetaDashboardGrid(
  architecture: DashboardArchitecture,
  viewModel: StockBetaDashboardViewModel,
): ReactNode {
  const hasSnapshot = viewModel.signals !== null;
  return (
    <div
      className={styles["dashboard"]}
      data-has-snapshot={hasSnapshot ? "true" : "false"}
      data-testid="stock-beta-dashboard"
    >
      <div className={styles["dashboardGrid"]}>
        {architecture.definitions.map((definition) => {
          const desktop = placementFor(architecture, definition.id, "desktop", hasSnapshot);
          const tablet = placementFor(architecture, definition.id, "tablet", hasSnapshot);
          const mobile = placementFor(architecture, definition.id, "mobile", hasSnapshot);
          if (desktop === undefined && tablet === undefined && mobile === undefined) return null;
          if (![desktop, tablet, mobile].some((placement) => placement?.visible === true))
            return null;
          const Widget = definition.component;
          return (
            <div
              className={styles["dashboardWidget"]}
              data-desktop-size={desktop?.size ?? "full"}
              data-desktop-visible={desktop?.visible === true ? "true" : "false"}
              data-mobile-size={mobile?.size ?? "full"}
              data-mobile-visible={mobile?.visible === true ? "true" : "false"}
              data-tablet-size={tablet?.size ?? "full"}
              data-tablet-visible={tablet?.visible === true ? "true" : "false"}
              data-testid={`stock-beta-widget-${definition.id}`}
              data-widget-id={definition.id}
              key={definition.id}
              style={layoutStyle(desktop, tablet, mobile)}
            >
              <Widget viewModel={viewModel} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function StockBetaDashboard({
  selectionProvided = false,
  viewModel,
}: {
  readonly selectionProvided?: boolean;
  readonly viewModel: StockBetaDashboardViewModel;
}) {
  const rows = viewModel.signals?.rows ?? [];
  const defaultSelectionId = viewModel.signals?.top5[0]?.instrument_id ?? rows[0]?.instrument_id;
  const content = renderStockBetaDashboardGrid(stockBetaDashboardArchitecture, viewModel);
  if (selectionProvided) return content;
  return (
    <StockBetaSelectionProvider
      {...(defaultSelectionId === undefined
        ? {}
        : { initialSelectedInstrumentId: defaultSelectionId })}
      rows={rows}
    >
      {content}
    </StockBetaSelectionProvider>
  );
}
