import styles from "../dashboard.module.css";
import type { StockBetaDashboardWidgetViewModel, StockBetaPolicyCopy } from "../types";

export function StockBetaPolicyNotice({ t }: { readonly t: StockBetaPolicyCopy }) {
  return (
    <aside aria-label={t.policyAriaLabel} className={styles["policyBoundary"]} role="note">
      <div className={styles["policySummary"]}>
        <span className={styles["policyEyebrow"]}>{t.warningLabel}</span>
        <strong>{t.policyBoundarySummary}</strong>
      </div>
      <details className={styles["policyDetails"]}>
        <summary>{t.policyBoundaryDetailsLabel}</summary>
        <div className={styles["policyDetailsContent"]}>
          <p>{t.fixedListPolicy}</p>
          <p>{t.originalPricePolicy}</p>
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
  return <StockBetaPolicyNotice t={viewModel.copy} />;
}
