"use client";

import Link from "next/link";
import { StatusPill } from "@/components/states/status-pill";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../labels";
import { metricLabel, renderStockBetaMetric, STOCK_BETA_METRIC_COLUMNS } from "../metric-columns";
import { useStockBetaSelection } from "../selection-provider";
import type { StockBetaDashboardWidgetViewModel } from "../types";

function emphasisFor(viewModel: StockBetaDashboardWidgetViewModel) {
  return {
    description: viewModel.copy.topFiveInTableLabel,
    ids: new Set(viewModel.signals?.top5.map((row) => row.instrument_id) ?? []),
    isApiTopFive: true,
    label: viewModel.copy.topFiveHeading,
    testId: "stock-beta-top-five",
  };
}

export function RankedSignalsWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, locale, signals } = viewModel;
  const { searchQuery, selectRow, selectedRow, visibleMetricKeys, visibleRows } =
    useStockBetaSelection();

  if (signals === null || signals.rows.length === 0) {
    return (
      <WidgetFrame
        state={{ kind: "empty", message: t.noResultsMessage }}
        title={t.rankTableHeading}
      >
        <p>{t.noResultsMessage}</p>
      </WidgetFrame>
    );
  }

  if (visibleRows.length === 0) {
    return (
      <WidgetFrame
        state={{ kind: "empty", message: t.noSearchResultsMessage }}
        title={t.rankTableHeading}
      >
        <p>{t.noSearchResultsMessage}</p>
      </WidgetFrame>
    );
  }

  const emphasis = emphasisFor(viewModel);
  const visibleColumns = STOCK_BETA_METRIC_COLUMNS.filter((column) =>
    visibleMetricKeys.includes(column.key),
  );

  return (
    <WidgetFrame
      status={<span className={styles["tableScope"]}>{t.configuredResultsLabel}</span>}
      title={t.rankTableHeading}
    >
      <div className={styles["rankedSignals"]}>
        <div className={styles["tableHeadingRow"]}>
          <span className={styles["topFiveLegend"]} data-testid={emphasis.testId}>
            {emphasis.label}
          </span>
          <span>{emphasis.description}</span>
          <span>
            {searchQuery === ""
              ? `${signals.rows.length} ${t.resultCountLabel.toLocaleLowerCase()}`
              : `${visibleRows.length} ${t.searchMatchesLabel}`}
          </span>
        </div>
        <div className={styles["tableScroll"]}>
          <table data-testid="stock-beta-rank-table">
            <caption>
              {t.rankTableCaption} · {t.resultCountLabel}: {signals.rows.length}
            </caption>
            <thead>
              <tr>
                <th scope="col">{t.rankLabel}</th>
                <th scope="col">{t.instrumentLabel}</th>
                <th className={styles["coreColumn"]} scope="col">
                  {t.scoreLabel}
                </th>
                <th className={styles["coreColumn"]} scope="col">
                  {t.tableConditionLabel}
                </th>
                {visibleColumns
                  .filter((column) => column.key !== "score")
                  .map((column) => (
                    <th
                      className={column.core ? styles["coreColumn"] : undefined}
                      key={column.key}
                      scope="col"
                    >
                      {metricLabel(column, t)}
                    </th>
                  ))}
                <th className={styles["detailColumn"]} scope="col">
                  {t.detailLinkLabel}
                </th>
              </tr>
            </thead>
            <tbody>
              {visibleRows.map((row) => {
                const isSelected = selectedRow?.instrument_id === row.instrument_id;
                const isEmphasized = emphasis.ids.has(row.instrument_id);
                const scoreColumn = STOCK_BETA_METRIC_COLUMNS[0];
                return (
                  <tr
                    className={styles["signalRow"]}
                    data-current-result-leader={
                      !emphasis.isApiTopFive && isEmphasized ? "true" : undefined
                    }
                    data-emphasized={isEmphasized ? "true" : "false"}
                    data-rank={row.rank}
                    data-selected={isSelected ? "true" : "false"}
                    data-server-order={row.rank}
                    data-testid={`stock-beta-row-${row.instrument_id}`}
                    data-top-five={emphasis.isApiTopFive && isEmphasized ? "true" : undefined}
                    key={row.instrument_id}
                  >
                    <th scope="row">{row.rank}</th>
                    <td className={styles["instrumentCell"]}>
                      <button
                        aria-pressed={isSelected}
                        aria-label={`${t.selectForPreview}: ${row.instrument_id}`}
                        className={styles["instrumentButton"]}
                        onClick={() => selectRow(row.instrument_id)}
                        type="button"
                      >
                        <strong>{row.instrument_id}</strong>
                        <small>
                          {t.generationLabel} {row.generation}
                        </small>
                      </button>
                    </td>
                    <td className={`${styles["metricCell"]} ${styles["scoreCell"]}`}>
                      {renderStockBetaMetric({
                        column: scoreColumn,
                        exactValueLabel: t.rawValueLabel,
                        locale,
                        row,
                      })}
                    </td>
                    <td>
                      <StatusPill
                        label={`${row.condition} · ${stockBetaConditionLabel(row.condition, t)}`}
                        tone={stockBetaConditionTone(row.condition)}
                      />
                    </td>
                    {visibleColumns
                      .filter((column) => column.key !== "score")
                      .map((column) => (
                        <td className={styles["metricCell"]} key={column.key}>
                          {renderStockBetaMetric({
                            column,
                            exactValueLabel: t.rawValueLabel,
                            locale,
                            row,
                          })}
                        </td>
                      ))}
                    <td>
                      <Link
                        className={styles["detailLink"]}
                        href={`/stock-beta/${encodeURIComponent(row.instrument_id)}`}
                        prefetch={false}
                      >
                        {t.openDetailLabel}
                      </Link>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
    </WidgetFrame>
  );
}
