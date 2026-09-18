import type { CSSProperties, ReactNode } from "react";
import type {
  StockBetaWidgetArchitecture,
  StockBetaWidgetBreakpoint,
  StockBetaWidgetCatalogEntry,
  StockBetaWidgetGridPlacementState,
  StockBetaWidgetGridState,
} from "../shared/widget-types";
import styles from "./dashboard.module.css";
import { StockBetaSelectionProvider } from "./selection-provider";
import type { StockBetaDashboardViewModel, StockBetaDashboardWidgetViewModel } from "./types";
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
  readonly StockBetaWidgetCatalogEntry<
    string,
    StockBetaDashboardWidgetViewModel,
    StockBetaWidgetGridPlacementState
  >[]
>;

type ResolvedWidgetPlacement = StockBetaWidgetGridState & {
  readonly size: StockBetaWidgetGridPlacementState["size"];
};

function placementFor(
  architecture: DashboardArchitecture,
  id: string,
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
  catalogIndex: number,
): DashboardLayoutStyle {
  return {
    "--desktop-grid-column": desktop?.column ?? 1,
    "--desktop-grid-column-span": desktop?.columnSpan ?? 12,
    "--desktop-grid-row": desktop?.row ?? 1,
    "--desktop-order": catalogIndex,
    "--tablet-grid-column": tablet?.column ?? 1,
    "--tablet-grid-column-span": tablet?.columnSpan ?? 12,
    "--tablet-grid-row": tablet?.row ?? 1,
    "--tablet-order": catalogIndex,
    "--mobile-grid-column": mobile?.column ?? 1,
    "--mobile-grid-column-span": mobile?.columnSpan ?? 1,
    "--mobile-grid-row": mobile?.row ?? 1,
    "--mobile-order": catalogIndex,
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
      {viewModel.intradayEnabled !== true && (
        <p role="status" data-testid="stock-beta-intraday-disabled">
          {viewModel.copy.intradayDisabledNotice}
        </p>
      )}
      <div className={styles["dashboardGrid"]}>
        {architecture.catalog.map((entry, catalogIndex) => {
          if (entry.id === "current-quote" && viewModel.intradayEnabled !== true) return null;
          const desktop = placementFor(architecture, entry.id, "desktop", hasSnapshot);
          const tablet = placementFor(architecture, entry.id, "tablet", hasSnapshot);
          const mobile = placementFor(architecture, entry.id, "mobile", hasSnapshot);
          if (desktop === undefined && tablet === undefined && mobile === undefined) return null;
          if (![desktop, tablet, mobile].some((placement) => placement?.visible === true))
            return null;
          const Widget = entry.component;
          return (
            <div
              className={styles["dashboardWidget"]}
              data-desktop-size={desktop?.size ?? "full"}
              data-desktop-visible={desktop?.visible === true ? "true" : "false"}
              data-mobile-size={mobile?.size ?? "full"}
              data-mobile-visible={mobile?.visible === true ? "true" : "false"}
              data-tablet-size={tablet?.size ?? "full"}
              data-tablet-visible={tablet?.visible === true ? "true" : "false"}
              data-testid={`stock-beta-widget-${entry.id}`}
              data-widget-id={entry.id}
              key={entry.id}
              style={layoutStyle(desktop, tablet, mobile, catalogIndex)}
            >
              <Widget
                {...(entry.id === "current-quote"
                  ? {
                      placementVisibility: {
                        desktop: desktop?.visible === true,
                        mobile: mobile?.visible === true,
                        tablet: tablet?.visible === true,
                      },
                    }
                  : {})}
                viewModel={viewModel}
              />
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
