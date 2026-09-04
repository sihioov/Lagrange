"use client";

import Link from "next/link";
import { useState } from "react";
import { StatusPill } from "@/components/states/status-pill";
import { formatStockBetaNumber, formatStockBetaPercent } from "../../shared/formatters";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../labels";
import { type StockBetaProfileTabId, stockBetaProfileTabs } from "../profile-tab-registry";
import { useStockBetaSelection } from "../selection-provider";
import type { StockBetaDashboardWidgetViewModel } from "../types";

function exactNumber(value: number, locale: StockBetaDashboardWidgetViewModel["locale"]) {
  const presentation = formatStockBetaNumber(value, locale);
  return (
    <span className={styles["exactMetric"]} data-raw-value={String(presentation.rawValue)}>
      <data value={String(presentation.rawValue)}>{presentation.text}</data>
      <small>{String(presentation.rawValue)}</small>
    </span>
  );
}

function exactPercent(value: number, locale: StockBetaDashboardWidgetViewModel["locale"]) {
  const presentation = formatStockBetaPercent(value, locale);
  return (
    <span className={styles["exactMetric"]} data-raw-value={String(presentation.rawValue)}>
      <data value={String(presentation.rawValue)}>{presentation.text}</data>
      <small>{String(presentation.rawValue)}</small>
    </span>
  );
}

export function SignalPreviewWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, locale } = viewModel;
  const { selectedRow } = useStockBetaSelection();
  const [tab, setTab] = useState<StockBetaProfileTabId>("price");

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
  const activeTab = stockBetaProfileTabs.find((item) => item.id === tab) ?? stockBetaProfileTabs[0];
  if (activeTab === undefined) return null;
  const ActiveTab = activeTab.renderer;

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
            <h3>{selectedRow.instrument_id}</h3>
            <p className={styles["instrumentId"]}>
              {t.generationLabel} {selectedRow.generation}
            </p>
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
          {stockBetaProfileTabs.map((item) => {
            const selected = tab === item.id;
            return (
              <button
                aria-controls={panelId}
                aria-selected={selected}
                className={styles["profileTab"]}
                id={`${tabId}-${item.id}`}
                key={item.id}
                onClick={() => setTab(item.id)}
                role="tab"
                type="button"
              >
                {item.label(t)}
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
          <ActiveTab selectedRow={selectedRow} viewModel={viewModel} />
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
          href={`/stock-beta/${encodeURIComponent(selectedRow.instrument_id)}`}
          prefetch={false}
        >
          {t.openDetailLabel}
        </Link>
      </article>
    </WidgetFrame>
  );
}
