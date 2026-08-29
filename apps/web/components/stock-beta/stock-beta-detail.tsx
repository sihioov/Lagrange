import Link from "next/link";
import { StatusPill } from "@/components/states/status-pill";
import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { OwnerBetaEquitySignalsDetailModel } from "@/lib/products/equity-signals-contracts";
import {
  StockBetaPolicyNotice,
  StockBetaProvenance,
  stockBetaConditionLabel,
  stockBetaConditionTone,
  stockBetaFormatNumber,
  stockBetaFormatPercent,
} from "./stock-beta-workspace";

function DetailMetrics({
  detail,
  t,
}: {
  readonly detail: OwnerBetaEquitySignalsDetailModel;
  readonly t: StockBetaDictionary;
}) {
  const signal = detail.signal;
  const metrics = [
    [t.scoreLabel, stockBetaFormatNumber(signal.score)],
    [t.return20Label, stockBetaFormatPercent(signal.return_20)],
    [t.return60Label, stockBetaFormatPercent(signal.return_60)],
    [t.return120Label, stockBetaFormatPercent(signal.return_120)],
    [t.volatility20Label, stockBetaFormatPercent(signal.volatility_20)],
    [t.volatility60Label, stockBetaFormatPercent(signal.volatility_60)],
    [t.volatility120Label, stockBetaFormatPercent(signal.volatility_120)],
    [t.drawdown120Label, stockBetaFormatPercent(signal.max_drawdown_120)],
    [t.sma20Label, stockBetaFormatNumber(signal.sma_20)],
    [t.sma60Label, stockBetaFormatNumber(signal.sma_60)],
    [t.averageVolumeLabel, stockBetaFormatNumber(signal.average_volume_20)],
    [t.volumeRatioLabel, stockBetaFormatNumber(signal.volume_ratio_20_60)],
    [t.activityProxyLabel, stockBetaFormatNumber(signal.average_trading_value_20)],
  ] as const;
  return (
    <section aria-labelledby="stock-beta-metrics-title" className="report-section">
      <h3 id="stock-beta-metrics-title">{t.signalMetricsHeading}</h3>
      <dl className="provenance-grid stock-beta-detail-metrics">
        {metrics.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

export function StockBetaDetail({
  detail,
  t,
}: {
  readonly detail: OwnerBetaEquitySignalsDetailModel;
  readonly t: StockBetaDictionary;
}) {
  const { signal } = detail;
  return (
    <>
      <StockBetaPolicyNotice t={t} />
      <nav aria-label="Research context" className="context-navigation">
        <Link href="/stock-beta">{t.backToWorkspace}</Link>
      </nav>
      <section
        aria-labelledby="stock-beta-detail-heading"
        className="data-report stock-beta-detail-report"
      >
        <header className="report-heading">
          <div>
            <p className="eyebrow">{t.detailEyebrow}</p>
            <h2 id="stock-beta-detail-heading">{signal.instrument_name}</h2>
            <p className="stock-beta-instrument-id">{signal.instrument_id}</p>
          </div>
          <div className="status-cluster">
            <span>
              {t.rankLabel} {signal.rank}
            </span>
            <StatusPill
              label={`${signal.condition} · ${stockBetaConditionLabel(signal.condition, t)}`}
              tone={stockBetaConditionTone(signal.condition)}
            />
            <span>
              {t.scoreLabel} {stockBetaFormatNumber(signal.score)}
            </span>
            <span>
              {t.asOfLabel} {detail.provenance.as_of}
            </span>
          </div>
        </header>
        <p className="supporting-copy">{t.detailDescription}</p>

        <section aria-labelledby="stock-beta-factor-title" className="report-section">
          <h3 id="stock-beta-factor-title">{t.factorLabel} evidence</h3>
          <div className="data-table-wrap">
            <table data-testid="stock-beta-factor-table">
              <caption>{t.factorLabel} evidence returned by the API</caption>
              <thead>
                <tr>
                  <th scope="col">{t.factorLabel}</th>
                  <th scope="col">{t.interpretationLabel}</th>
                  <th scope="col">{t.valueLabel}</th>
                </tr>
              </thead>
              <tbody>
                {detail.factor_explanations.map((factor) => (
                  <tr key={factor.factor}>
                    <th scope="row">{factor.factor}</th>
                    <td>{factor.interpretation}</td>
                    <td>{String(factor.value)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>

        <DetailMetrics detail={detail} t={t} />

        <section aria-labelledby="stock-beta-reasons-title" className="report-section">
          <div className="section-heading">
            <div>
              <h3 id="stock-beta-reasons-title">{t.conditionReasonsHeading}</h3>
              <p>{t.conditionReasonsDescription}</p>
            </div>
          </div>
          {detail.condition_reasons.length === 0 ? (
            <p className="empty-copy">{t.noReasons}</p>
          ) : (
            <ul className="stock-beta-reason-list">
              {detail.condition_reasons.map((reason) => (
                <li key={reason}>{reason}</li>
              ))}
            </ul>
          )}
        </section>
      </section>
      <StockBetaProvenance provenance={detail.provenance} t={t} />
    </>
  );
}
