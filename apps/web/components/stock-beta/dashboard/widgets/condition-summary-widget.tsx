import type { OwnerBetaEquitySignalCondition } from "@/lib/products/equity-signals-contracts";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../labels";
import type { StockBetaDashboardWidgetViewModel } from "../types";

const CONDITIONS: readonly OwnerBetaEquitySignalCondition[] = ["BULLISH", "NEUTRAL", "BEARISH"];

export function ConditionSummaryWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, data, filtered } = viewModel;
  const counts: Record<OwnerBetaEquitySignalCondition, number> = {
    BEARISH: 0,
    BULLISH: 0,
    NEUTRAL: 0,
  };
  for (const row of data.rows) counts[row.condition] += 1;

  const scopeLabel = filtered ? t.currentResultsLabel : t.configuredResultsLabel;

  return (
    <WidgetFrame description={t.conditionSummaryDescription} title={t.conditionSummaryHeading}>
      <div className={styles["conditionSummary"]} data-testid="stock-beta-condition-distribution">
        <p className={styles["scopeLabel"]}>
          {t.conditionDistributionLabel}: {scopeLabel}
        </p>
        <ul className={styles["conditionList"]}>
          {CONDITIONS.map((condition) => (
            <li key={condition}>
              <span className={styles["conditionName"]}>
                <span
                  aria-hidden="true"
                  className={styles["conditionDot"]}
                  data-tone={stockBetaConditionTone(condition)}
                />
                {conditionLabel(condition, t)}
              </span>
              <strong>{counts[condition]}</strong>
            </li>
          ))}
        </ul>
      </div>
    </WidgetFrame>
  );
}

function conditionLabel(
  condition: OwnerBetaEquitySignalCondition,
  t: StockBetaDashboardWidgetViewModel["copy"],
): string {
  return `${condition} · ${stockBetaConditionLabel(condition, t)}`;
}
