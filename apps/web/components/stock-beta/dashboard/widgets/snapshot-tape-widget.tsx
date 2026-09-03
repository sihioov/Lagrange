import { formatStockBetaNumber, formatStockBetaPercent } from "../../shared/formatters";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import { stockBetaConditionLabel } from "../labels";
import type { StockBetaDashboardWidgetViewModel } from "../types";

export function SnapshotTapeWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, locale, signals } = viewModel;
  const leaders = signals?.top5.length ? signals.top5 : (signals?.rows.slice(0, 5) ?? []);
  const scopeLabel = t.topFiveHeading;

  if (leaders.length === 0) {
    return (
      <WidgetFrame
        state={{ kind: "empty", message: t.noResultsMessage }}
        title={t.snapshotTapeHeading}
      >
        <p>{t.noResultsMessage}</p>
      </WidgetFrame>
    );
  }

  return (
    <WidgetFrame
      description={t.snapshotTapeDescription}
      status={<span className={styles["tableScope"]}>{scopeLabel}</span>}
      title={t.snapshotTapeHeading}
    >
      <div className={styles["snapshotTape"]} data-testid="stock-beta-snapshot-tape">
        <p className={styles["tapeScope"]}>{t.currentSnapshotLabel}</p>
        <ul>
          {leaders.map((row) => (
            <li
              data-server-order={row.rank}
              data-testid={`stock-beta-tape-${row.instrument_id}`}
              key={row.instrument_id}
            >
              <span className={styles["tapeRank"]}>{row.rank}</span>
              <strong>{row.instrument_id}</strong>
              <span>{stockBetaConditionLabel(row.condition, t)}</span>
              <data data-metric-key="score" value={String(row.score)}>
                {formatStockBetaNumber(row.score, locale).text}
              </data>
              <data data-metric-key="return_20" value={String(row.return_20)}>
                {formatStockBetaPercent(row.return_20, locale).text}
              </data>
            </li>
          ))}
        </ul>
        <p className={styles["tapeNote"]}>{t.currentResultsDescription}</p>
      </div>
    </WidgetFrame>
  );
}
