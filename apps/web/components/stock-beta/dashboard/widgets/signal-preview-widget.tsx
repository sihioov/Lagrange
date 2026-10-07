"use client";

import Link from "next/link";
import { useState } from "react";
import { StatusPill } from "@/components/states/status-pill";
import { formatStockBetaNumber, formatStockBetaPercent } from "../../shared/formatters";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../labels";
import {
  normalizeStockBetaProfileTabs,
  type StockBetaProfileTabId,
  stockBetaProfileTabs,
} from "../profile-tab-registry";
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

function emptyProfileState(viewModel: StockBetaDashboardWidgetViewModel) {
  const { copy: t, signalState, signals } = viewModel;
  if (signals !== null) {
    return {
      kind: "empty" as const,
      message: signals.rows.length === 0 ? t.noResultsMessage : t.previewEmptyMessage,
    };
  }
  if (signalState.kind === "unavailable")
    return { kind: "blocked" as const, message: t.signalUnavailableMessage };
  if (signalState.kind === "error")
    return { kind: "error" as const, message: t.requestFailure(signalState.code) };
  return {
    kind: "empty" as const,
    message: signalState.kind === "not-ready" ? t.notReadyMessage : t.previewEmptyMessage,
  };
}

export function SignalPreviewWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, locale } = viewModel;
  const { selectedRow } = useStockBetaSelection();
  const profileTabs = normalizeStockBetaProfileTabs(stockBetaProfileTabs);
  const [tab, setTab] = useState<StockBetaProfileTabId | undefined>(profileTabs[0]?.id);

  if (selectedRow === undefined) {
    const state = emptyProfileState(viewModel);
    return (
      <WidgetFrame state={state} title={t.signalProfileHeading}>
        <p>{state.message}</p>
      </WidgetFrame>
    );
  }

  const tabId = `stock-beta-profile-tab-${selectedRow.instrument_id}`;
  const panelId = `stock-beta-profile-panel-${selectedRow.instrument_id}`;
  const activeTab = profileTabs.find((item) => item.id === tab) ?? profileTabs[0];
  if (activeTab === undefined) return null;
  const activeTabId = activeTab.id;
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
          {profileTabs.map((item) => {
            const selected = activeTabId === item.id;
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
          aria-labelledby={`${tabId}-${activeTabId}`}
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
