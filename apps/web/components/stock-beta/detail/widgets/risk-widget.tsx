import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import { DetailMetricCard } from "../metric-card";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function RiskWidget({ viewModel }: { readonly viewModel: StockBetaDetailWidgetViewModel }) {
  const { copy: t, detail } = viewModel;
  const { signal } = detail;
  const volatilityMetrics = [
    { key: "volatility_20", label: t.volatility20Label, value: signal.volatility_20 },
    { key: "volatility_60", label: t.volatility60Label, value: signal.volatility_60 },
    { key: "volatility_120", label: t.volatility120Label, value: signal.volatility_120 },
  ] as const;
  const trendMetrics = [
    {
      key: "max_drawdown_120",
      label: t.drawdown120Label,
      value: signal.max_drawdown_120,
    },
    { key: "sma_20", label: t.sma20Label, value: signal.sma_20 },
    { key: "sma_60", label: t.sma60Label, value: signal.sma_60 },
  ] as const;

  return (
    <WidgetFrame description={t.riskDescription} title={t.riskHeading}>
      <div className={styles["metricSection"]} data-testid="stock-beta-detail-risk">
        <section
          className={styles["metricSubgroup"]}
          aria-labelledby="stock-beta-volatility-heading"
        >
          <h3 className={styles["metricSubgroupHeading"]} id="stock-beta-volatility-heading">
            {t.volatilityGroupHeading}
          </h3>
          <dl className={styles["metricGrid"]}>
            {volatilityMetrics.map((metric) => (
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
        <section className={styles["metricSubgroup"]} aria-labelledby="stock-beta-trend-heading">
          <h3 className={styles["metricSubgroupHeading"]} id="stock-beta-trend-heading">
            {t.trendGroupHeading}
          </h3>
          <dl className={styles["metricGrid"]}>
            {trendMetrics.map((metric) => (
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
