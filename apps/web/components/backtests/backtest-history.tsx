import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import type { BacktestsDictionary } from "@/lib/i18n/dictionaries/backtests";
import { type BacktestRunModel, backtestRunLabel } from "@/lib/products/backtest-contracts";
import { formatDate } from "@/lib/products/format";

const STATUS_TONES = {
  CANCELED: "warning",
  FAILED: "error",
  PENDING: "neutral",
  RUNNING: "info",
  SUCCEEDED: "success",
} as const satisfies Record<BacktestRunModel["status"], StatusTone>;

export type BacktestHistoryProps = {
  readonly runs: readonly BacktestRunModel[];
  readonly t: BacktestsDictionary;
};

export function BacktestHistory({ runs, t }: BacktestHistoryProps) {
  return (
    <section aria-labelledby="backtest-history-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.historyEyebrow}</p>
          <h2 id="backtest-history-title">{t.historyHeading}</h2>
        </div>
      </div>
      <div className="data-table-wrap">
        <table>
          <caption>{t.historyCaption}</caption>
          <thead>
            <tr>
              <th scope="col">{t.runColumnHeader}</th>
              <th scope="col">{t.periodColumnHeader}</th>
              <th scope="col">{t.statusColumnHeader}</th>
              <th scope="col">{t.ownerColumnHeader}</th>
              <th scope="col">{t.runIdColumnHeader}</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((run) => (
              <tr key={run.id}>
                <th scope="row">{backtestRunLabel(run)}</th>
                <td>
                  {t.periodRange(
                    run.start_date === null || run.start_date === undefined
                      ? t.openDateLabel
                      : formatDate(run.start_date),
                    run.end_date === null || run.end_date === undefined
                      ? t.openDateLabel
                      : formatDate(run.end_date),
                  )}
                </td>
                <td>
                  <StatusPill label={run.status} tone={STATUS_TONES[run.status]} />
                </td>
                <td>
                  {run.can_manage
                    ? t.yourRunLabel
                    : t.sharedRunLabel(run.owner_user_id.slice(0, 8))}
                </td>
                <td className="data-cell">{run.id}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
