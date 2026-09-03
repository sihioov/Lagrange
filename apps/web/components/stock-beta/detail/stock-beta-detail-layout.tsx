import type { CSSProperties } from "react";
import type { StockBetaWidgetBreakpoint, StockBetaWidgetSize } from "../shared/widget-types";
import styles from "./detail.module.css";
import type { StockBetaDetailViewModel } from "./types";
import { stockBetaDetailArchitecture, stockBetaDetailDefinitions } from "./widget-registry";

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
  return stockBetaDetailArchitecture.layout[breakpoint].find((placement) => placement.id === id);
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

export function StockBetaDetailLayout({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailViewModel;
}) {
  return (
    <div className={styles["detail"]} data-testid="stock-beta-detail-board">
      <div className={styles["detailGrid"]}>
        {stockBetaDetailDefinitions.map((definition) => {
          const desktop = placementFor(definition.id, "desktop");
          const tablet = placementFor(definition.id, "tablet");
          const mobile = placementFor(definition.id, "mobile");
          if (desktop === undefined && tablet === undefined && mobile === undefined) return null;
          const Widget = definition.component;
          return (
            <div
              className={styles["detailWidget"]}
              data-desktop-size={desktop?.size ?? "full"}
              data-desktop-visible={desktop?.visible === true ? "true" : "false"}
              data-mobile-size={mobile?.size ?? "full"}
              data-mobile-visible={mobile?.visible === true ? "true" : "false"}
              data-tablet-size={tablet?.size ?? "full"}
              data-tablet-visible={tablet?.visible === true ? "true" : "false"}
              data-testid={`stock-beta-detail-widget-${definition.id}`}
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
