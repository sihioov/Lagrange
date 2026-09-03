import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import { DetailMetricCard } from "../metric-card";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function ActivityWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t, detail } = viewModel;
  const { signal } = detail;
  const metrics = [
    { key: "sma_20", label: t.sma20Label, value: signal.sma_20 },
    { key: "sma_60", label: t.sma60Label, value: signal.sma_60 },
    {
      key: "average_volume_20",
      label: t.averageVolumeLabel,
      value: signal.average_volume_20,
    },
    {
      key: "volume_ratio_20_60",
      label: t.volumeRatioLabel,
      value: signal.volume_ratio_20_60,
    },
    {
      key: "average_trading_value_20",
      label: t.activityProxyLabel,
      value: signal.average_trading_value_20,
    },
  ] as const;

  return (
    <WidgetFrame description={t.activityDescription} title={t.activityHeading}>
      <div className={styles["metricSection"]} data-testid="stock-beta-detail-activity">
        <dl className={styles["metricGrid"]}>
          {metrics.map((metric) => (
            <DetailMetricCard
              exactValueLabel={t.rawValueLabel}
              key={metric.key}
              label={metric.label}
              metricKey={metric.key}
              value={metric.value}
            />
          ))}
        </dl>
      </div>
    </WidgetFrame>
  );
}
