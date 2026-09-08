import type { CSSProperties, ReactNode } from "react";
import type {
  StockBetaWidgetArchitecture,
  StockBetaWidgetBreakpoint,
  StockBetaWidgetCatalogEntry,
  StockBetaWidgetPlacementState,
} from "../shared/widget-types";
import styles from "./detail.module.css";
import type { StockBetaDetailViewModel, StockBetaDetailWidgetViewModel } from "./types";
import { stockBetaDetailArchitecture } from "./widget-registry";

type LayoutOrderStyle = CSSProperties & {
  readonly "--desktop-order": number;
  readonly "--mobile-order": number;
  readonly "--tablet-order": number;
};

type WidgetPlacement = {
  readonly size: StockBetaWidgetPlacementState["size"];
  readonly visible: StockBetaWidgetPlacementState["visible"];
};

type DetailArchitecture = StockBetaWidgetArchitecture<
  readonly StockBetaWidgetCatalogEntry<
    string,
    StockBetaDetailWidgetViewModel,
    StockBetaWidgetPlacementState
  >[]
>;

function placementFor(
  architecture: DetailArchitecture,
  id: string,
  breakpoint: StockBetaWidgetBreakpoint,
): WidgetPlacement | undefined {
  return architecture.layout[breakpoint].find((placement) => placement.id === id);
}

function layoutStyle(catalogIndex: number): LayoutOrderStyle {
  return {
    "--desktop-order": catalogIndex,
    "--tablet-order": catalogIndex,
    "--mobile-order": catalogIndex,
  };
}

export function renderStockBetaDetailGrid(
  architecture: DetailArchitecture,
  viewModel: StockBetaDetailViewModel,
): ReactNode {
  return (
    <div className={styles["detail"]} data-testid="stock-beta-detail-board">
      <div className={styles["detailGrid"]}>
        {architecture.catalog.map((entry, catalogIndex) => {
          if (entry.id === "current-quote" && viewModel.intradayEnabled !== true) return null;
          const desktop = placementFor(architecture, entry.id, "desktop");
          const tablet = placementFor(architecture, entry.id, "tablet");
          const mobile = placementFor(architecture, entry.id, "mobile");
          if (desktop === undefined && tablet === undefined && mobile === undefined) return null;
          const Widget = entry.component;
          return (
            <div
              className={styles["detailWidget"]}
              data-desktop-size={desktop?.size ?? "full"}
              data-desktop-visible={desktop?.visible === true ? "true" : "false"}
              data-mobile-size={mobile?.size ?? "full"}
              data-mobile-visible={mobile?.visible === true ? "true" : "false"}
              data-tablet-size={tablet?.size ?? "full"}
              data-tablet-visible={tablet?.visible === true ? "true" : "false"}
              data-testid={`stock-beta-detail-widget-${entry.id}`}
              data-widget-id={entry.id}
              key={entry.id}
              style={layoutStyle(catalogIndex)}
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

export function StockBetaDetailLayout({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailViewModel;
}) {
  return renderStockBetaDetailGrid(stockBetaDetailArchitecture, viewModel);
}
