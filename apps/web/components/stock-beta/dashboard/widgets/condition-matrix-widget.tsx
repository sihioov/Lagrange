"use client";

import { formatStockBetaNumber } from "../../shared/formatters";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../labels";
import { useStockBetaSelection } from "../selection-provider";
import type { StockBetaDashboardWidgetViewModel } from "../types";

export function ConditionMatrixWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, locale } = viewModel;
  const { searchQuery, selectRow, selectedRow, visibleRows } = useStockBetaSelection();

  if (viewModel.signals === null || viewModel.signals.rows.length === 0) {
    return (
      <WidgetFrame
        state={{ kind: "empty", message: t.noResultsMessage }}
        title={t.conditionMatrixHeading}
      >
        <p>{t.noResultsMessage}</p>
      </WidgetFrame>
    );
  }

  if (visibleRows.length === 0) {
    return (
      <WidgetFrame
        state={{ kind: "empty", message: t.noSearchResultsMessage }}
        title={t.conditionMatrixHeading}
      >
        <p>{t.noSearchResultsMessage}</p>
      </WidgetFrame>
    );
  }

  return (
    <WidgetFrame description={t.conditionMatrixDescription} title={t.conditionMatrixHeading}>
      <ul
        aria-label={t.conditionMatrixHeading}
        className={styles["conditionMatrix"]}
        data-search-query={searchQuery || undefined}
        data-testid="stock-beta-condition-matrix"
      >
        {visibleRows.map((row) => {
          const selected = selectedRow?.instrument_id === row.instrument_id;
          const tone = stockBetaConditionTone(row.condition);
          return (
            <li key={row.instrument_id}>
              <button
                aria-label={`${row.instrument_id}, ${row.condition} · ${stockBetaConditionLabel(row.condition, t)}, ${t.scoreLabel} ${row.score}`}
                aria-pressed={selected}
                className={styles["conditionTile"]}
                data-condition={row.condition}
                data-selected={selected ? "true" : "false"}
                data-testid={`stock-beta-matrix-${row.instrument_id}`}
                data-tone={tone}
                onClick={() => selectRow(row.instrument_id)}
                type="button"
              >
                <span>{row.rank}</span>
                <strong>{row.instrument_id}</strong>
                <small>{stockBetaConditionLabel(row.condition, t)}</small>
                <data value={String(row.score)}>
                  {formatStockBetaNumber(row.score, locale).text}
                </data>
              </button>
            </li>
          );
        })}
      </ul>
    </WidgetFrame>
  );
}
