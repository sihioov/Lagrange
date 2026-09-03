"use client";

import { StatusPill } from "@/components/states/status-pill";
import { formatStockBetaNumber, formatStockBetaPercent } from "../../shared/formatters";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../labels";
import { useStockBetaSelection } from "../selection-provider";
import type { StockBetaDashboardWidgetViewModel } from "../types";

type DecompositionMetric = {
  readonly format: "number" | "percent";
  readonly key: string;
  readonly label: string;
  readonly value: number;
};

function MetricValue({
  locale,
  metric,
}: {
  readonly locale: StockBetaDashboardWidgetViewModel["locale"];
  readonly metric: DecompositionMetric;
}) {
  const presentation =
    metric.format === "percent"
      ? formatStockBetaPercent(metric.value, locale)
      : formatStockBetaNumber(metric.value, locale);
  return (
    <span
      className={styles["decompositionValue"]}
      data-metric-key={metric.key}
      data-raw-value={String(presentation.rawValue)}
    >
      <data value={String(presentation.rawValue)}>{presentation.text}</data>
      <small>{`API value: ${String(presentation.rawValue)}`}</small>
    </span>
  );
}

export function SignalDecompositionWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, locale } = viewModel;
  const { selectedRow } = useStockBetaSelection();

  if (selectedRow === undefined) {
    return (
      <WidgetFrame
        state={{ kind: "empty", message: t.previewEmptyMessage }}
        title={t.signalDecompositionHeading}
      >
        <p>{t.previewEmptyMessage}</p>
      </WidgetFrame>
    );
  }

  const metrics: readonly DecompositionMetric[] = [
    { format: "percent", key: "return_20", label: t.return20Label, value: selectedRow.return_20 },
    { format: "percent", key: "return_60", label: t.return60Label, value: selectedRow.return_60 },
    {
      format: "percent",
      key: "return_120",
      label: t.return120Label,
      value: selectedRow.return_120,
    },
    {
      format: "percent",
      key: "volatility_20",
      label: t.volatility20Label,
      value: selectedRow.volatility_20,
    },
    {
      format: "percent",
      key: "volatility_60",
      label: t.volatility60Label,
      value: selectedRow.volatility_60,
    },
    {
      format: "percent",
      key: "volatility_120",
      label: t.volatility120Label,
      value: selectedRow.volatility_120,
    },
    {
      format: "percent",
      key: "max_drawdown_120",
      label: t.drawdown120Label,
      value: selectedRow.max_drawdown_120,
    },
    {
      format: "number",
      key: "average_volume_20",
      label: t.averageVolumeLabel,
      value: selectedRow.average_volume_20,
    },
    {
      format: "number",
      key: "volume_ratio_20_60",
      label: t.volumeRatioLabel,
      value: selectedRow.volume_ratio_20_60,
    },
    {
      format: "number",
      key: "average_trading_value_20",
      label: t.activityProxyLabel,
      value: selectedRow.average_trading_value_20,
    },
  ];
  const score = formatStockBetaNumber(selectedRow.score, locale);

  return (
    <WidgetFrame
      description={t.signalDecompositionDescription}
      status={
        <div className={styles["decompositionBadges"]}>
          <span>{t.betaLabel}</span>
          <span>{t.readOnlyBadgeLabel}</span>
        </div>
      }
      title={t.signalDecompositionHeading}
    >
      <article
        className={styles["signalDecomposition"]}
        data-testid="stock-beta-signal-decomposition"
      >
        <header className={styles["decompositionHeader"]}>
          <div>
            <strong>{selectedRow.instrument_id}</strong>
            <span>
              {t.generationLabel} {selectedRow.generation}
            </span>
          </div>
          <div className={styles["rawScore"]} data-raw-value={String(score.rawValue)}>
            <span>{t.scoreLabel}</span>
            <strong>
              <data value={String(score.rawValue)}>{score.text}</data>
            </strong>
            <small>{`API value: ${String(score.rawValue)}`}</small>
          </div>
        </header>
        <div className={styles["decompositionCondition"]}>
          <StatusPill
            label={`${selectedRow.condition} · ${stockBetaConditionLabel(selectedRow.condition, t)}`}
            tone={stockBetaConditionTone(selectedRow.condition)}
          />
          <span>
            {t.rankLabel} {selectedRow.rank}
          </span>
        </div>
        <dl className={styles["decompositionMetrics"]}>
          {metrics.map((metric) => (
            <div key={metric.key}>
              <dt>{metric.label}</dt>
              <dd>
                <MetricValue locale={locale} metric={metric} />
              </dd>
            </div>
          ))}
        </dl>
      </article>
    </WidgetFrame>
  );
}
