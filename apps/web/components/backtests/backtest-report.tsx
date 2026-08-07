import { ReportFooter } from "@/components/reports/report-footer";
import { StatusPill } from "@/components/states/status-pill";
import {
  backtestRobustness,
  metricValue,
  type BacktestReportModel,
} from "@/lib/products/backtest-contracts";
import { formatDate, formatKrw, formatPercentage, formatTimestamp } from "@/lib/products/format";
import { RobustnessControl } from "./robustness-control";

export type BacktestReportProps = {
  readonly licenseState: string;
  readonly report: BacktestReportModel;
};

export function BacktestReport({ licenseState, report }: BacktestReportProps) {
  const endingEquity = metricValue(report.metrics, "ending_equity");
  const maximumDrawdown = metricValue(report.metrics, "maximum_drawdown");
  const totalCost = metricValue(report.metrics, "total_cost");
  const robustness = backtestRobustness(report.run);
  return (
    <section aria-labelledby="backtest-report-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">Verified server result</p>
          <h2 id="backtest-report-title">Backtest result</h2>
          <p>Historical strategy simulation. Review execution assumptions and warnings.</p>
        </div>
        <div className="status-cluster">
          <StatusPill label={report.run.status} tone="success" />
          <span>As of {formatDate(report.run.end_date ?? report.run.created_at?.slice(0, 10) ?? "2026-01-01")}</span>
        </div>
      </header>
      <aside aria-label="Backtest warnings" className="warning-strip" role="status">
        <strong>Warnings</strong>
        {report.provenance.warnings.length === 0 ? (
          <p>No server warnings.</p>
        ) : (
          <ul>
            {report.provenance.warnings.map((warning) => <li key={warning}>{warning}</li>)}
          </ul>
        )}
      </aside>
      <section aria-labelledby="equity-drawdown-title" className="report-section">
        <h3 id="equity-drawdown-title">Equity and drawdown</h3>
        <dl className="provenance-grid">
          <div><dt>Ending equity</dt><dd>{endingEquity === null ? "Not reported" : formatKrw(endingEquity)}</dd></div>
          <div><dt>Maximum drawdown</dt><dd>{maximumDrawdown === null ? "Not reported" : formatPercentage(maximumDrawdown)}</dd></div>
        </dl>
        <div className="data-table-wrap">
          <table>
            <caption>Server-provided equity curve</caption>
            <thead><tr><th scope="col">Date</th><th scope="col">Equity</th></tr></thead>
            <tbody>
              {report.equity.summary.equity_curve.map((point) => (
                <tr key={point.date}><th scope="row">{formatDate(point.date)}</th><td>{formatKrw(point.value)}</td></tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="data-table-wrap">
          <table>
            <caption>Server-provided drawdown curve</caption>
            <thead><tr><th scope="col">Date</th><th scope="col">Drawdown</th></tr></thead>
            <tbody>
              {report.equity.summary.drawdown_curve.map((point) => (
                <tr key={point.date}><th scope="row">{formatDate(point.date)}</th><td>{formatPercentage(point.value)}</td></tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <section aria-labelledby="monthly-returns-title" className="report-section">
        <h3 id="monthly-returns-title">Monthly returns</h3>
        <div className="data-table-wrap">
          <table>
            <caption>Server-provided monthly returns</caption>
            <thead><tr><th scope="col">Month</th><th scope="col">Return</th></tr></thead>
            <tbody>
              {report.equity.summary.monthly_returns.map((item) => (
                <tr key={item.month}><th scope="row">{item.month}</th><td>{formatPercentage(item.value)}</td></tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <section aria-labelledby="trades-costs-title" className="report-section">
        <h3 id="trades-costs-title">Trades and costs</h3>
        <p>{report.trades.total_count.toLocaleString("en-US")} trades. Total cost {totalCost === null ? "Not reported" : formatKrw(totalCost)}.</p>
        <div className="data-table-wrap">
          <table>
            <caption>Executed trades and server-calculated costs</caption>
            <thead><tr><th scope="col">Trade</th><th scope="col">Time</th><th scope="col">Instrument</th><th scope="col">Side</th><th scope="col">Quantity</th><th scope="col">Cost</th></tr></thead>
            <tbody>
              {report.trades.items.map((trade) => (
                <tr key={trade.trade_id}>
                  <th scope="row">{trade.trade_id}</th>
                  <td>{formatTimestamp(trade.executed_at)}</td>
                  <td>{trade.instrument_id}</td>
                  <td>{trade.side}</td>
                  <td>{trade.quantity}</td>
                  <td>{formatKrw(trade.cost)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <RobustnessControl runId={report.run.id} />
      {robustness === null ? null : (
        <section aria-labelledby="robustness-evidence-title" className="report-section">
          <h3 id="robustness-evidence-title">Robustness evidence</h3>
          <dl className="provenance-grid">
            <div><dt>Parameter sensitivity</dt><dd>{robustness.parameter_sensitivity}</dd></div>
            <div><dt>Cost stress</dt><dd>{robustness.cost_stress}</dd></div>
            <div><dt>Validation periods</dt><dd>{robustness.validation_periods}</dd></div>
          </dl>
        </section>
      )}
      <ReportFooter
        asOf={report.run.end_date ?? report.run.created_at?.slice(0, 10) ?? "2026-01-01"}
        licenseState={licenseState}
        provenance={report.provenance}
      />
    </section>
  );
}
