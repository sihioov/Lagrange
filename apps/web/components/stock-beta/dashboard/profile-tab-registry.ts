import { type ComponentType, type CSSProperties, createElement } from "react";
import type { OwnerEquityV2SignalModel } from "@/lib/products/equity-signals-contracts";
import { PriceChart, type PriceChartCopy } from "../chart";
import { formatStockBetaNumber, formatStockBetaPercent } from "../shared/formatters";
import styles from "./dashboard.module.css";
import type { StockBetaDashboardWidgetViewModel } from "./types";

const css = (name: string): string => styles[name] ?? "";

export type StockBetaProfileTabId = "price" | "returns" | "volatility" | "activity";
type ProfileTabProps = {
  readonly selectedRow: OwnerEquityV2SignalModel;
  readonly viewModel: StockBetaDashboardWidgetViewModel;
};
export type StockBetaProfileTabDefinition = {
  readonly id: StockBetaProfileTabId;
  readonly label: (copy: StockBetaDashboardWidgetViewModel["copy"]) => string;
  readonly renderer: ComponentType<ProfileTabProps>;
};

function ExactMetric({
  percent = false,
  value,
  viewModel,
}: {
  readonly percent?: boolean;
  readonly value: number;
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const presentation = percent
    ? formatStockBetaPercent(value, viewModel.locale)
    : formatStockBetaNumber(value, viewModel.locale);
  return createElement(
    "span",
    { className: css("exactMetric"), "data-raw-value": String(presentation.rawValue) },
    createElement("data", { value: String(presentation.rawValue) }, presentation.text),
    createElement("small", null, String(presentation.rawValue)),
  );
}

function ProfilePlot({
  metrics,
  title,
  viewModel,
}: {
  readonly metrics: readonly { readonly label: string; readonly value: number }[];
  readonly title: string;
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const maxAbs = Math.max(...metrics.map((metric) => Math.abs(metric.value)), 0);
  return createElement(
    "figure",
    { "aria-label": title, className: css("profilePlot") },
    createElement(
      "div",
      { className: css("plotAxisLabel") },
      createElement("span", null, title),
      createElement("small", null, viewModel.copy.zeroAxisLabel),
    ),
    createElement(
      "div",
      { className: css("plotRows") },
      metrics.map((metric) => {
        const direction = metric.value < 0 ? "negative" : metric.value > 0 ? "positive" : "zero";
        const barSize = maxAbs === 0 ? 0 : (Math.abs(metric.value) / maxAbs) * 42;
        return createElement(
          "div",
          { className: css("plotRow"), key: metric.label },
          createElement("span", { className: css("plotLabel") }, metric.label),
          createElement(
            "span",
            { "aria-hidden": true, className: css("plotTrack") },
            createElement("span", {
              className: css("plotBar"),
              "data-direction": direction,
              style: { "--bar-size": `${barSize}%` } as CSSProperties,
            }),
          ),
          createElement(ExactMetric, { percent: true, value: metric.value, viewModel }),
        );
      }),
    ),
  );
}

function ChartState({
  message,
  alert = false,
}: {
  readonly message: string;
  readonly alert?: boolean;
}) {
  return createElement(
    "p",
    { className: css("chartState"), ...(alert ? { role: "alert" } : {}) },
    message,
  );
}

function PriceTab({ selectedRow, viewModel }: ProfileTabProps) {
  const { chartData: chart, chartRange, chartState, copy: t, onChartRangeChange } = viewModel;
  if (chartState?.kind === "integrity")
    return createElement(ChartState, { alert: true, message: t.chartIntegrityMessage });
  if (chartState?.kind === "unavailable" || chartState?.kind === "idle")
    return createElement(ChartState, { message: t.chartPreparingMessage });
  if (chartState?.kind === "error")
    return createElement(ChartState, { alert: true, message: t.chartNetworkMessage });
  if (chartState?.kind === "loading" && chart?.instrument_id !== selectedRow.instrument_id)
    return createElement(ChartState, { message: t.chartLoadingMessage });
  if (chart?.instrument_id !== selectedRow.instrument_id)
    return createElement(ChartState, { message: t.chartNotReadyMessage });
  const isUpdating = chartState?.kind === "loading";
  const chartCopy: Partial<PriceChartCopy> = {
    chartLabel: t.priceChartLabel,
    closeLabel: t.closeLabel,
    dateLabel: t.dateLabel,
    downLabel: t.downLabel,
    highLabel: t.highLabel,
    instructions: t.chartKeyboardInstructions,
    lowLabel: t.lowLabel,
    noDataLabel: t.chartNoDataLabel,
    openLabel: t.openLabel,
    priceAxisLabel: t.priceAxisLabel,
    selectedObservationLabel: t.selectedObservationLabel,
    sma20Label: t.sma20Label,
    sma60Label: t.sma60Label,
    unchangedLabel: t.unchangedLabel,
    upLabel: t.upLabel,
    volumeAxisLabel: t.volumeAxisLabel,
    volumeLabel: t.volumeLabel,
  };
  const direction =
    chart.latest.change < 0 ? "negative" : chart.latest.change > 0 ? "positive" : "zero";
  const signedChange = `${chart.latest.change > 0 ? "+" : ""}${formatStockBetaNumber(chart.latest.change, viewModel.locale).text}`;
  const signedRate = `${chart.latest.change > 0 ? "+" : ""}${formatStockBetaPercent(chart.latest.change_rate, viewModel.locale).text}`;
  const freshness =
    chart.freshness === "STALE"
      ? createElement(
          "p",
          { className: css("chartWarning") },
          t.staleChartMessage(chart.as_of, chart.expected_as_of ?? t.notAvailableLabel),
        )
      : chart.freshness === "UNVERIFIABLE"
        ? createElement("p", { className: css("chartWarning") }, t.unverifiableChartMessage)
        : null;
  return createElement(
    "section",
    { "aria-busy": isUpdating, className: css("priceTab") },
    createElement(
      "dl",
      { className: css("latestPriceStrip") },
      createElement(
        "div",
        null,
        createElement("dt", null, t.latestCloseLabel),
        createElement(
          "dd",
          null,
          createElement(ExactMetric, { value: chart.latest.close, viewModel }),
        ),
      ),
      createElement(
        "div",
        { "data-direction": direction },
        createElement("dt", null, t.changeLabel),
        createElement(
          "dd",
          null,
          createElement("span", null, signedChange),
          createElement("small", null, signedRate),
        ),
      ),
      createElement(
        "div",
        null,
        createElement("dt", null, t.volumeLabel),
        createElement(
          "dd",
          null,
          createElement(ExactMetric, { value: chart.latest.volume, viewModel }),
        ),
      ),
      createElement(
        "div",
        null,
        createElement("dt", null, t.asOfLabel),
        createElement("dd", null, chart.as_of),
      ),
    ),
    createElement(
      "p",
      { className: css("eodStatus") },
      createElement("strong", null, t.eodCloseLabel),
      " · ",
      t.originalUnadjustedLabel,
      " · ",
      t.corporateActionCaveat,
    ),
    freshness,
    createElement(
      "div",
      { "aria-label": t.chartRangeLabel, className: css("chartRangeControl"), role: "group" },
      (["1m", "3m", "6m", "1y"] as const).map((range) =>
        createElement(
          "button",
          {
            "aria-pressed": chartRange === range,
            className: css("chartRangeButton"),
            key: range,
            onClick: () => onChartRangeChange?.(range),
            type: "button",
          },
          t.chartRangeOption(range),
        ),
      ),
      isUpdating
        ? createElement("span", { className: css("chartUpdating") }, t.chartUpdatingLabel)
        : null,
    ),
    createElement(PriceChart, {
      bars: chart.bars,
      className: css("priceChart"),
      copy: chartCopy,
      locale: viewModel.locale,
    }),
  );
}

function ReturnsTab({ selectedRow, viewModel }: ProfileTabProps) {
  const t = viewModel.copy;
  return createElement(ProfilePlot, {
    metrics: [
      { label: t.return20Label, value: selectedRow.return_20 },
      { label: t.return60Label, value: selectedRow.return_60 },
      { label: t.return120Label, value: selectedRow.return_120 },
    ],
    title: t.returnsTabLabel,
    viewModel,
  });
}
function VolatilityTab({ selectedRow, viewModel }: ProfileTabProps) {
  const t = viewModel.copy;
  return createElement(ProfilePlot, {
    metrics: [
      { label: t.volatility20Label, value: selectedRow.volatility_20 },
      { label: t.volatility60Label, value: selectedRow.volatility_60 },
      { label: t.volatility120Label, value: selectedRow.volatility_120 },
    ],
    title: t.volatilityTabLabel,
    viewModel,
  });
}
function ActivityTab({ selectedRow, viewModel }: ProfileTabProps) {
  const t = viewModel.copy;
  const metric = (label: string, value: number) =>
    createElement(
      "div",
      { key: label },
      createElement("dt", null, label),
      createElement("dd", null, createElement(ExactMetric, { value, viewModel })),
    );
  return createElement(
    "dl",
    { className: css("activityMetrics") },
    metric(t.averageVolumeLabel, selectedRow.average_volume_20),
    metric(t.volumeRatioLabel, selectedRow.volume_ratio_20_60),
    metric(t.activityProxyLabel, selectedRow.average_trading_value_20),
  );
}

/** Change this ordered registry to add, remove, or reorder profile tabs. */
export const stockBetaProfileTabs = [
  { id: "price", label: (t) => t.priceTabLabel, renderer: PriceTab },
  { id: "returns", label: (t) => t.returnsTabLabel, renderer: ReturnsTab },
  { id: "volatility", label: (t) => t.volatilityTabLabel, renderer: VolatilityTab },
  { id: "activity", label: (t) => t.activityTabLabel, renderer: ActivityTab },
] as const satisfies readonly StockBetaProfileTabDefinition[];
