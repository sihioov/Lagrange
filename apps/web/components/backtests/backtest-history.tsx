import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import {
  backtestRunLabel,
  type BacktestRunModel,
} from "@/lib/products/backtest-contracts";
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
};

export function BacktestHistory({ runs }: BacktestHistoryProps) {
  return (
    <section aria-labelledby="backtest-history-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Private run history</p>
          <h2 id="backtest-history-title">Backtest runs</h2>
        </div>
      </div>
      <div className="data-table-wrap">
        <table>
          <caption>Backtest jobs and result availability</caption>
          <thead><tr><th scope="col">Run</th><th scope="col">Period</th><th scope="col">Status</th><th scope="col">Run ID</th></tr></thead>
          <tbody>
            {runs.map((run) => (
              <tr key={run.id}>
                <th scope="row">{backtestRunLabel(run)}</th>
                <td>{run.start_date === null || run.start_date === undefined ? "Open" : formatDate(run.start_date)} to {run.end_date === null || run.end_date === undefined ? "Open" : formatDate(run.end_date)}</td>
                <td><StatusPill label={run.status} tone={STATUS_TONES[run.status]} /></td>
                <td className="data-cell">{run.id}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
