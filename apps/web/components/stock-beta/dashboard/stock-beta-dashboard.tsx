import type { CSSProperties } from "react";
import type { StockBetaWidgetBreakpoint, StockBetaWidgetSize } from "../shared/widget-types";
import styles from "./dashboard.module.css";
import { StockBetaSelectionProvider } from "./selection-provider";
import type { StockBetaDashboardViewModel } from "./types";
import { stockBetaDashboardArchitecture, stockBetaDashboardDefinitions } from "./widget-registry";

type LayoutOrderStyle = CSSProperties & {
  readonly "--desktop-order": number;
  readonly "--mobile-order": number;
  readonly "--tablet-order": number;
};
type WidgetPlacement = {
  readonly size: StockBetaWidgetSize;
  readonly visible: boolean;
  readonly order: number;
};

function placementFor(
  id: string,
  breakpoint: StockBetaWidgetBreakpoint,
): WidgetPlacement | undefined {
  return stockBetaDashboardArchitecture.layout[breakpoint].find((placement) => placement.id === id);
}

function layoutStyle(
  desktop: WidgetPlacement | undefined,
  tablet: WidgetPlacement | undefined,
  mobile: WidgetPlacement | undefined,
): LayoutOrderStyle {
  return {
    "--desktop-order": desktop?.order ?? 99,
    "--tablet-order": tablet?.order ?? 99,
    "--mobile-order": mobile?.order ?? 99,
  };
}

function visibleForState(id: string, hasSnapshot: boolean): boolean {
  if (id === "signal-state") return !hasSnapshot;
  if (
    [
      "ranked-signals",
      "signal-profile",
      "signal-decomposition",
      "condition-matrix",
      "snapshot-tape",
      "provenance",
    ].includes(id)
  )
    return hasSnapshot;
  return true;
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
  const content = (
    <div
      className={styles["dashboard"]}
      data-has-snapshot={viewModel.signals === null ? "false" : "true"}
      data-testid="stock-beta-dashboard"
    >
      <div className={styles["dashboardGrid"]}>
        {stockBetaDashboardDefinitions.map((definition) => {
          if (!visibleForState(definition.id, viewModel.signals !== null)) return null;
          const desktop = placementFor(definition.id, "desktop");
          const tablet = placementFor(definition.id, "tablet");
          const mobile = placementFor(definition.id, "mobile");
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
