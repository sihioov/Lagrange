import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import { DetailMetricCard } from "../metric-card";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function ReturnsWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t, detail } = viewModel;
  const { signal } = detail;
  const returns = [
    { key: "return_20", label: t.return20Label, value: signal.return_20 },
    { key: "return_60", label: t.return60Label, value: signal.return_60 },
    { key: "return_120", label: t.return120Label, value: signal.return_120 },
  ] as const;
  return (
    <WidgetFrame description={t.returnsDescription} title={t.returnsHeading}>
      <div className={styles["metricSection"]} data-testid="stock-beta-detail-returns">
        <section className={styles["metricSubgroup"]} aria-labelledby="stock-beta-returns-heading">
          <h3 className={styles["metricSubgroupHeading"]} id="stock-beta-returns-heading">
            {t.returnsHeading}
          </h3>
          <dl className={styles["metricGrid"]}>
            {returns.map((metric) => (
              <DetailMetricCard
                exactValueLabel={t.rawValueLabel}
                key={metric.key}
                label={metric.label}
                metricKey={metric.key}
                value={metric.value}
              />
            ))}
          </dl>
        </section>
      </div>
    </WidgetFrame>
  );
}
