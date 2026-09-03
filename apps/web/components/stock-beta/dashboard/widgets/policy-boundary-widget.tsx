import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import type { StockBetaDashboardWidgetViewModel } from "../types";

export function StockBetaPolicyNotice({ t }: { readonly t: StockBetaDictionary }) {
  return (
    <aside aria-label={t.policyAriaLabel} className={styles["policyBoundary"]} role="note">
      <div className={styles["policySummary"]}>
        <span className={styles["policyEyebrow"]}>{t.warningLabel}</span>
        <strong>{t.policyBoundarySummary}</strong>
      </div>
      <details className={styles["policyDetails"]}>
        <summary>{t.policyBoundaryDetailsLabel}</summary>
        <div className={styles["policyDetailsContent"]}>
          <p>{t.ownerOnlyPolicy}</p>
          <p>{t.vendorSnapshotPolicy}</p>
          <p>{t.originalPricePolicy}</p>
          <p>{t.nonPitPolicy}</p>
          <p>{t.activityPolicy}</p>
          <p>{t.conditionPolicy}</p>
        </div>
      </details>
    </aside>
  );
}

export function PolicyBoundaryWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  return (
    <WidgetFrame
      description={viewModel.copy.policyBoundaryDescription}
      title={viewModel.copy.policyBoundaryHeading}
    >
      <StockBetaPolicyNotice t={viewModel.copy} />
    </WidgetFrame>
  );
}
