import { ReportFooter } from "@/components/reports/report-footer";
import { StatusPill } from "@/components/states/status-pill";
import type { BacktestsDictionary } from "@/lib/i18n/dictionaries/backtests";
import {
  type BacktestReportModel,
  backtestRobustness,
  metricValue,
} from "@/lib/products/backtest-contracts";
import { formatDate, formatKrw, formatPercentage, formatTimestamp } from "@/lib/products/format";
import { RobustnessControl } from "./robustness-control";

export type BacktestReportProps = {
  readonly licenseState: string;
  readonly report: BacktestReportModel;
  readonly t: BacktestsDictionary;
};

export function BacktestReport({ licenseState, report, t }: BacktestReportProps) {
  const endingEquity = metricValue(report.metrics, "ending_equity");
  const maximumDrawdown = metricValue(report.metrics, "maximum_drawdown");
  const totalCost = metricValue(report.metrics, "total_cost");
  const robustness = backtestRobustness(report.run);
  const asOf = report.run.end_date ?? report.run.created_at?.slice(0, 10) ?? null;
  return (
    <section aria-labelledby="backtest-report-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">{t.reportEyebrow}</p>
          <h2 id="backtest-report-title">{t.reportHeading}</h2>
          <p>{t.reportSubheading}</p>
        </div>
        <div className="status-cluster">
          <StatusPill label={report.run.status} tone="success" />
          <span>{t.asOfLabel(asOf === null ? t.notReported : formatDate(asOf))}</span>
        </div>
      </header>
      <aside aria-label={t.warningsAriaLabel} className="warning-strip">
        <strong>{t.warningsHeading}</strong>
        {report.provenance.warnings.length === 0 ? (
          <p>{t.noWarnings}</p>
        ) : (
          <ul>
            {report.provenance.warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        )}
      </aside>
      <section aria-labelledby="equity-drawdown-title" className="report-section">
        <h3 id="equity-drawdown-title">{t.equityDrawdownHeading}</h3>
        <dl className="provenance-grid">
          <div>
            <dt>{t.endingEquityLabel}</dt>
            <dd>{endingEquity === null ? t.notReported : formatKrw(endingEquity)}</dd>
          </div>
          <div>
            <dt>{t.maximumDrawdownLabel}</dt>
            <dd>{maximumDrawdown === null ? t.notReported : formatPercentage(maximumDrawdown)}</dd>
          </div>
        </dl>
        <div className="data-table-wrap">
          <table>
            <caption>{t.equityCurveCaption}</caption>
            <thead>
              <tr>
                <th scope="col">{t.dateColumnHeader}</th>
                <th scope="col">{t.equityColumnHeader}</th>
              </tr>
            </thead>
            <tbody>
              {report.equity.summary.equity_curve.map((point) => (
                <tr key={point.date}>
                  <th scope="row">{formatDate(point.date)}</th>
                  <td>{formatKrw(point.value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="data-table-wrap">
          <table>
            <caption>{t.drawdownCurveCaption}</caption>
            <thead>
              <tr>
                <th scope="col">{t.dateColumnHeader}</th>
                <th scope="col">{t.drawdownColumnHeader}</th>
              </tr>
            </thead>
            <tbody>
              {report.equity.summary.drawdown_curve.map((point) => (
                <tr key={point.date}>
                  <th scope="row">{formatDate(point.date)}</th>
                  <td>{formatPercentage(point.value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <section aria-labelledby="monthly-returns-title" className="report-section">
        <h3 id="monthly-returns-title">{t.monthlyReturnsHeading}</h3>
        <div className="data-table-wrap">
          <table>
            <caption>{t.monthlyReturnsCaption}</caption>
            <thead>
              <tr>
                <th scope="col">{t.monthColumnHeader}</th>
                <th scope="col">{t.returnColumnHeader}</th>
              </tr>
            </thead>
            <tbody>
              {report.equity.summary.monthly_returns.map((item) => (
                <tr key={item.month}>
                  <th scope="row">{item.month}</th>
                  <td>{formatPercentage(item.value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <section aria-labelledby="trades-costs-title" className="report-section">
        <h3 id="trades-costs-title">{t.tradesCostsHeading}</h3>
        <p>
          {t.tradesSummary(
            report.trades.total_count.toLocaleString("en-US"),
            totalCost === null ? t.notReported : formatKrw(totalCost),
          )}
        </p>
        <div className="data-table-wrap">
          <table>
            <caption>{t.tradesCaption}</caption>
            <thead>
              <tr>
                <th scope="col">{t.tradeColumnHeader}</th>
                <th scope="col">{t.timeColumnHeader}</th>
                <th scope="col">{t.instrumentColumnHeader}</th>
                <th scope="col">{t.sideColumnHeader}</th>
                <th scope="col">{t.quantityColumnHeader}</th>
                <th scope="col">{t.costColumnHeader}</th>
              </tr>
            </thead>
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
      {report.run.can_manage ? <RobustnessControl runId={report.run.id} /> : null}
      {robustness === null ? null : (
        <section aria-labelledby="robustness-evidence-title" className="report-section">
          <h3 id="robustness-evidence-title">{t.robustnessEvidenceHeading}</h3>
          <dl className="provenance-grid">
            <div>
              <dt>{t.parameterSensitivityLabel}</dt>
              <dd>{robustness.parameter_sensitivity}</dd>
            </div>
            <div>
              <dt>{t.costStressLabel}</dt>
              <dd>{robustness.cost_stress}</dd>
            </div>
            <div>
              <dt>{t.validationPeriodsLabel}</dt>
              <dd>{robustness.validation_periods}</dd>
            </div>
          </dl>
        </section>
      )}
      <ReportFooter asOf={asOf} licenseState={licenseState} provenance={report.provenance} />
    </section>
  );
}
