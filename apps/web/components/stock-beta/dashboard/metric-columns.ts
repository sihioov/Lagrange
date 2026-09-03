import { createElement, type ReactNode } from "react";
import type { Locale } from "@/lib/i18n/locale";
import type { OwnerEquityV2SignalModel } from "@/lib/products/equity-signals-contracts";
import {
  formatStockBetaNumber,
  formatStockBetaPercent,
  type StockBetaNumericPresentation,
} from "../shared/formatters";
import styles from "./dashboard.module.css";
import type { StockBetaDashboardCopy } from "./types";

export type StockBetaNumericRowKey =
  | "score"
  | "return_20"
  | "return_60"
  | "return_120"
  | "volatility_20"
  | "volatility_60"
  | "volatility_120"
  | "max_drawdown_120"
  | "sma_20"
  | "sma_60"
  | "average_volume_20"
  | "volume_ratio_20_60"
  | "average_trading_value_20";

export type StockBetaMetricLabelKey =
  | "activityProxyLabel"
  | "averageVolumeLabel"
  | "drawdown120Label"
  | "return120Label"
  | "return20Label"
  | "return60Label"
  | "scoreLabel"
  | "sma20Label"
  | "sma60Label"
  | "volatility120Label"
  | "volatility20Label"
  | "volatility60Label"
  | "volumeRatioLabel";

export type StockBetaMetricColumn = {
  readonly core: boolean;
  readonly format: "number" | "percent";
  readonly key: StockBetaNumericRowKey;
  readonly label: StockBetaMetricLabelKey;
};

export const STOCK_BETA_METRIC_COLUMNS = [
  { core: true, format: "number", key: "score", label: "scoreLabel" },
  { core: true, format: "percent", key: "return_20", label: "return20Label" },
  { core: true, format: "percent", key: "return_60", label: "return60Label" },
  { core: true, format: "percent", key: "return_120", label: "return120Label" },
  { core: false, format: "percent", key: "volatility_20", label: "volatility20Label" },
  { core: false, format: "percent", key: "volatility_60", label: "volatility60Label" },
  { core: true, format: "percent", key: "volatility_120", label: "volatility120Label" },
  { core: true, format: "percent", key: "max_drawdown_120", label: "drawdown120Label" },
  { core: false, format: "number", key: "sma_20", label: "sma20Label" },
  { core: false, format: "number", key: "sma_60", label: "sma60Label" },
  { core: false, format: "number", key: "average_volume_20", label: "averageVolumeLabel" },
  { core: false, format: "number", key: "volume_ratio_20_60", label: "volumeRatioLabel" },
  {
    core: true,
    format: "number",
    key: "average_trading_value_20",
    label: "activityProxyLabel",
  },
] as const satisfies readonly StockBetaMetricColumn[];

export function formatStockBetaMetric(
  row: OwnerEquityV2SignalModel,
  column: StockBetaMetricColumn,
  locale: Locale,
): StockBetaNumericPresentation {
  const value = row[column.key];
  return column.format === "percent"
    ? formatStockBetaPercent(value, locale)
    : formatStockBetaNumber(value, locale);
}

export function renderStockBetaMetric({
  column,
  exactValueLabel,
  locale,
  row,
}: {
  readonly column: StockBetaMetricColumn;
  readonly exactValueLabel: string;
  readonly locale: Locale;
  readonly row: OwnerEquityV2SignalModel;
}): ReactNode {
  const formatted = formatStockBetaMetric(row, column, locale);
  const rawValue = String(formatted.rawValue);

  return createElement(
    "span",
    { "data-metric-key": column.key, "data-raw-value": rawValue },
    createElement("data", { value: rawValue }, formatted.text),
    createElement(
      "small",
      { className: styles["metricRawValue"] },
      `${exactValueLabel}: ${rawValue}`,
    ),
  );
}

export function metricLabel(
  column: StockBetaMetricColumn,
  t: Pick<StockBetaDashboardCopy, StockBetaMetricLabelKey>,
): string {
  return t[column.label];
}
