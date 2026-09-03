"use client";

import Link from "next/link";
import { type CSSProperties, useState } from "react";
import { StatusPill } from "@/components/states/status-pill";
import { formatStockBetaNumber, formatStockBetaPercent } from "../../shared/formatters";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaDetailHref } from "../filter-context";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../labels";
import { useStockBetaSelection } from "../selection-provider";
import type { StockBetaDashboardWidgetViewModel } from "../types";

const PROFILE_TABS = ["returns", "volatility", "activity"] as const;
type ProfileTab = (typeof PROFILE_TABS)[number];

function exactPercent(value: number, locale: StockBetaDashboardWidgetViewModel["locale"]) {
  const presentation = formatStockBetaPercent(value, locale);
  return (
    <span className={styles["exactMetric"]} data-raw-value={String(presentation.rawValue)}>
      <data value={String(presentation.rawValue)}>{presentation.text}</data>
      <small>{String(presentation.rawValue)}</small>
    </span>
  );
}

function exactNumber(value: number, locale: StockBetaDashboardWidgetViewModel["locale"]) {
  const presentation = formatStockBetaNumber(value, locale);
  return (
    <span className={styles["exactMetric"]} data-raw-value={String(presentation.rawValue)}>
      <data value={String(presentation.rawValue)}>{presentation.text}</data>
      <small>{String(presentation.rawValue)}</small>
    </span>
  );
}

function tabLabel(tab: ProfileTab, t: StockBetaDashboardWidgetViewModel["copy"]): string {
  switch (tab) {
    case "returns":
      return t.returnsTabLabel;
    case "volatility":
      return t.volatilityTabLabel;
    case "activity":
      return t.activityTabLabel;
  }
}

function ProfilePlot({
  locale,
  metrics,
  title,
  zeroAxisLabel,
}: {
  readonly locale: StockBetaDashboardWidgetViewModel["locale"];
  readonly metrics: readonly { readonly label: string; readonly value: number }[];
  readonly title: string;
  readonly zeroAxisLabel: string;
}) {
  const maxAbs = Math.max(...metrics.map((metric) => Math.abs(metric.value)), 0);
  return (
    <figure aria-label={title} className={styles["profilePlot"]}>
      <div className={styles["plotAxisLabel"]}>
        <span>{title}</span>
        <small>{zeroAxisLabel}</small>
      </div>
      <div className={styles["plotRows"]}>
        {metrics.map((metric) => {
          const barSize = maxAbs === 0 ? 0 : (Math.abs(metric.value) / maxAbs) * 42;
          const direction = metric.value < 0 ? "negative" : metric.value > 0 ? "positive" : "zero";
          const style = { "--bar-size": `${barSize}%` } as CSSProperties;
          return (
            <div className={styles["plotRow"]} key={metric.label}>
              <span className={styles["plotLabel"]}>{metric.label}</span>
              <span aria-hidden="true" className={styles["plotTrack"]}>
                <span className={styles["plotBar"]} data-direction={direction} style={style} />
              </span>
              {exactPercent(metric.value, locale)}
            </div>
          );
        })}
      </div>
    </figure>
  );
}

export function SignalPreviewWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, filters, locale } = viewModel;
  const { selectedRow } = useStockBetaSelection();
  const [tab, setTab] = useState<ProfileTab>("returns");

  if (selectedRow === undefined) {
    return (
      <WidgetFrame
        state={{ kind: "empty", message: t.previewEmptyMessage }}
        title={t.signalProfileHeading}
      >
        <p>{t.previewEmptyMessage}</p>
      </WidgetFrame>
    );
  }

  const tabId = `stock-beta-profile-tab-${selectedRow.instrument_id}`;
  const panelId = `stock-beta-profile-panel-${selectedRow.instrument_id}`;
  const metrics =
    tab === "returns"
      ? [
          { label: t.return20Label, value: selectedRow.return_20 },
          { label: t.return60Label, value: selectedRow.return_60 },
          { label: t.return120Label, value: selectedRow.return_120 },
        ]
      : tab === "volatility"
        ? [
            { label: t.volatility20Label, value: selectedRow.volatility_20 },
            { label: t.volatility60Label, value: selectedRow.volatility_60 },
            { label: t.volatility120Label, value: selectedRow.volatility_120 },
          ]
        : [];

  return (
    <WidgetFrame
      description={t.signalProfileDescription}
      status={
        <StatusPill
          label={`${selectedRow.condition} · ${stockBetaConditionLabel(selectedRow.condition, t)}`}
          tone={stockBetaConditionTone(selectedRow.condition)}
        />
      }
      title={t.signalProfileHeading}
    >
      <article
        className={styles["signalPreview"]}
        data-selected-instrument={selectedRow.instrument_id}
        data-testid="stock-beta-signal-preview"
      >
        <header className={styles["previewHeader"]}>
          <div>
            <p className={styles["previewEyebrow"]}>{t.instrumentLabel}</p>
            <h3>{selectedRow.instrument_name}</h3>
            <p className={styles["instrumentId"]}>{selectedRow.instrument_id}</p>
          </div>
          <dl className={styles["previewIdentity"]}>
            <div>
              <dt>{t.rankLabel}</dt>
              <dd>{selectedRow.rank}</dd>
            </div>
            <div>
              <dt>{t.scoreLabel}</dt>
              <dd>
                {formatStockBetaNumber(selectedRow.score, locale).text}
                <small>{String(selectedRow.score)}</small>
              </dd>
            </div>
          </dl>
        </header>
        <div className={styles["profileTabs"]} role="tablist" aria-label={t.signalMetricsHeading}>
          {PROFILE_TABS.map((item) => {
            const selected = tab === item;
            return (
              <button
                aria-controls={panelId}
                aria-selected={selected}
                className={styles["profileTab"]}
                id={`${tabId}-${item}`}
                key={item}
                onClick={() => setTab(item)}
                role="tab"
                type="button"
              >
                {tabLabel(item, t)}
              </button>
            );
          })}
        </div>
        <div
          aria-labelledby={`${tabId}-${tab}`}
          className={styles["profilePanel"]}
          data-testid="stock-beta-signal-profile"
          id={panelId}
          role="tabpanel"
        >
          {tab === "activity" ? (
            <dl className={styles["activityMetrics"]}>
              <div>
                <dt>{t.averageVolumeLabel}</dt>
                <dd>{exactNumber(selectedRow.average_volume_20, locale)}</dd>
              </div>
              <div>
                <dt>{t.volumeRatioLabel}</dt>
                <dd>{exactNumber(selectedRow.volume_ratio_20_60, locale)}</dd>
              </div>
              <div>
                <dt>{t.activityProxyLabel}</dt>
                <dd>{exactNumber(selectedRow.average_trading_value_20, locale)}</dd>
              </div>
            </dl>
          ) : (
            <ProfilePlot
              locale={locale}
              metrics={metrics}
              title={tabLabel(tab, t)}
              zeroAxisLabel={t.zeroAxisLabel}
            />
          )}
        </div>
        <dl className={styles["profileMetricStrip"]} data-testid="stock-beta-signal-metric-strip">
          <div>
            <dt>{t.sma20Label}</dt>
            <dd>{exactNumber(selectedRow.sma_20, locale)}</dd>
          </div>
          <div>
            <dt>{t.sma60Label}</dt>
            <dd>{exactNumber(selectedRow.sma_60, locale)}</dd>
          </div>
          <div>
            <dt>{t.drawdown120Label}</dt>
            <dd>{exactPercent(selectedRow.max_drawdown_120, locale)}</dd>
          </div>
        </dl>
        <Link
          className={styles["previewDetailLink"]}
          href={stockBetaDetailHref(selectedRow.instrument_id, filters)}
          prefetch={false}
        >
          {t.openDetailLabel}
        </Link>
      </article>
    </WidgetFrame>
  );
}
