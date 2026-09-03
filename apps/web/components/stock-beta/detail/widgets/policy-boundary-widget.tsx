import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function StockBetaDetailPolicyNotice({ t }: { readonly t: StockBetaDictionary }) {
  return (
    <aside aria-label={t.policyAriaLabel} className={styles["policyBoundary"]} role="note">
      <div className={styles["policySummary"]}>
        <span aria-hidden="true" className={styles["policyMarker"]}>
          !
        </span>
        <div>
          <strong>{t.warningLabel}</strong>
          <p>{t.policyBoundarySummary}</p>
        </div>
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
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t } = viewModel;
  return (
    <WidgetFrame description={t.policyBoundaryDescription} title={t.policyBoundaryHeading}>
      <StockBetaDetailPolicyNotice t={t} />
    </WidgetFrame>
  );
}
