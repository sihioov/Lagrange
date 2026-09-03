import styles from "./detail.module.css";

export type StockBetaDetailMetricCardProps = {
  readonly exactValueLabel: string;
  readonly label: string;
  readonly metricKey: string;
  readonly value: number;
};

export function DetailMetricCard({
  exactValueLabel,
  label,
  metricKey,
  value,
}: StockBetaDetailMetricCardProps) {
  const rawValue = String(value);

  return (
    <div className={styles["metricCard"]} data-metric-key={metricKey} data-raw-value={rawValue}>
      <dt>{label}</dt>
      <dd>
        <data value={rawValue}>{rawValue}</data>
      </dd>
      <small className={styles["metricRawValue"]}>
        {exactValueLabel}: {rawValue}
      </small>
    </div>
  );
}
