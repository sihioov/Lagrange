import type { CSSProperties, ReactNode } from "react";
import { ownerBetaEquitySignalsFiltersSelected } from "@/lib/products/equity-signals-contracts";
import type { StockBetaWidgetBreakpoint, StockBetaWidgetSize } from "../shared/widget-types";
import styles from "./dashboard.module.css";
import { StockBetaSelectionProvider } from "./selection-provider";
import type { StockBetaDashboardBaseViewModel } from "./types";
import { stockBetaDashboardArchitecture, stockBetaDashboardDefinitions } from "./widget-registry";
import { ActiveFilterChips } from "./widgets/filter-widget";

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

export type StockBetaDashboardProps = Omit<StockBetaDashboardBaseViewModel, "filtered"> & {
  readonly initialSelectedInstrumentId?: string;
  readonly selectionProvided?: boolean;
};

function dashboardContent(baseViewModel: StockBetaDashboardBaseViewModel): ReactNode {
  return (
    <div className={styles["dashboard"]} data-testid="stock-beta-dashboard">
      <ActiveFilterChips viewModel={baseViewModel} />
      <div className={styles["dashboardGrid"]}>
        {stockBetaDashboardDefinitions.map((definition) => {
          const desktop = placementFor(definition.id, "desktop");
          const tablet = placementFor(definition.id, "tablet");
          const mobile = placementFor(definition.id, "mobile");
          if (desktop === undefined && tablet === undefined && mobile === undefined) return null;
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
              <Widget viewModel={baseViewModel} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function StockBetaDashboard({
  copy,
  data,
  filters,
  initialSelectedInstrumentId,
  locale,
  selectionProvided = false,
}: StockBetaDashboardProps) {
  const baseViewModel: StockBetaDashboardBaseViewModel = {
    copy,
    data,
    filtered: ownerBetaEquitySignalsFiltersSelected(filters),
    filters,
    locale,
  };
  const defaultSelectionId =
    initialSelectedInstrumentId ??
    ("top5" in data ? data.top5[0]?.instrument_id : data.rows[0]?.instrument_id);
  const content = dashboardContent(baseViewModel);
  if (selectionProvided) return content;
  return (
    <StockBetaSelectionProvider
      {...(defaultSelectionId === undefined
        ? {}
        : { initialSelectedInstrumentId: defaultSelectionId })}
      rows={baseViewModel.data.rows}
    >
      {content}
    </StockBetaSelectionProvider>
  );
}
