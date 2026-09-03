import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function ConditionReasonsWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t, detail } = viewModel;

  if (detail.condition_reasons.length === 0) {
    return (
      <WidgetFrame
        description={t.conditionReasonsDescription}
        state={{ kind: "empty", message: t.noReasons }}
        title={t.conditionReasonsHeading}
      >
        <p>{t.noReasons}</p>
      </WidgetFrame>
    );
  }

  const reasonKeys = new Map<string, number>();

  return (
    <WidgetFrame description={t.conditionReasonsDescription} title={t.conditionReasonsHeading}>
      <ul className={styles["reasonList"]} data-testid="stock-beta-condition-reasons">
        {detail.condition_reasons.map((reason) => {
          const occurrence = reasonKeys.get(reason) ?? 0;
          reasonKeys.set(reason, occurrence + 1);
          return <li key={`${reason}-${occurrence}`}>{reason}</li>;
        })}
      </ul>
    </WidgetFrame>
  );
}
