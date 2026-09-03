"use client";

import styles from "./dashboard.module.css";
import { metricLabel, STOCK_BETA_METRIC_COLUMNS } from "./metric-columns";
import { useStockBetaSelection } from "./selection-provider";
import type { StockBetaDashboardCopy } from "./types";

export function StockBetaColumnVisibilityControl({
  copy: t,
}: {
  readonly copy: StockBetaDashboardCopy;
}) {
  const { toggleMetricColumn, visibleMetricKeys } = useStockBetaSelection();
  return (
    <details className={styles["columnView"]}>
      <summary>{t.columnViewLabel}</summary>
      <fieldset>
        <legend>{t.columnViewHeading}</legend>
        {STOCK_BETA_METRIC_COLUMNS.map((column) => (
          <label key={column.key}>
            <input
              checked={visibleMetricKeys.includes(column.key)}
              disabled={column.key === "score"}
              onChange={() => toggleMetricColumn(column.key)}
              type="checkbox"
            />
            <span>{metricLabel(column, t)}</span>
          </label>
        ))}
      </fieldset>
    </details>
  );
}
