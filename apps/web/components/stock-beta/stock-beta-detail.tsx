import Link from "next/link";
import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type { OwnerEquityV2SignalDetailModel } from "@/lib/products/equity-signals-contracts";
import { StockBetaPolicyNotice } from "./stock-beta-workspace";

type DetailCondition = OwnerEquityV2SignalDetailModel["signal"]["condition"];

function conditionLabel(condition: DetailCondition, t: StockBetaDictionary): string {
  switch (condition) {
    case "BULLISH":
      return t.bullishLabel;
    case "NEUTRAL":
      return t.neutralLabel;
    case "BEARISH":
      return t.bearishLabel;
  }
}

function conditionTone(condition: DetailCondition): StatusTone {
  switch (condition) {
    case "BULLISH":
      return "success";
    case "NEUTRAL":
      return "neutral";
    case "BEARISH":
      return "warning";
  }
}

function formatNumber(value: number, fractionDigits = 2): string {
  return value.toLocaleString("en-US", {
    maximumFractionDigits: fractionDigits,
    minimumFractionDigits: fractionDigits,
  });
}

function formatPercent(value: number): string {
  const sign = value > 0 ? "+" : "";
  return `${sign}${formatNumber(value * 100)}%`;
}

function SnapshotSummary({
  detail,
  t,
}: {
  readonly detail: OwnerEquityV2SignalDetailModel;
  readonly t: StockBetaDictionary;
}) {
  const items = [
    [t.asOfLabel, detail.snapshot.as_of],
    [t.snapshotRowsLabel, detail.snapshot.row_count.toLocaleString("en-US")],
    [t.publishedAtLabel, detail.snapshot.published_at],
    [t.universeHashLabel, detail.snapshot.universe_sha256],
  ] as const;
  return (
    <section aria-labelledby="stock-beta-detail-snapshot-title" className="report-section">
      <div className="section-heading">
        <div>
          <h3 id="stock-beta-detail-snapshot-title">{t.snapshotHeading}</h3>
        </div>
        <p>{t.snapshotDescription}</p>
      </div>
      <dl className="provenance-grid stock-beta-detail-snapshot">
        {items.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function DetailMetrics({
  detail,
  t,
}: {
  readonly detail: OwnerEquityV2SignalDetailModel;
  readonly t: StockBetaDictionary;
}) {
  const signal = detail.signal;
  const metrics = [
    [t.scoreLabel, formatNumber(signal.score)],
    [t.return20Label, formatPercent(signal.return_20)],
    [t.return60Label, formatPercent(signal.return_60)],
    [t.return120Label, formatPercent(signal.return_120)],
    [t.volatility20Label, formatPercent(signal.volatility_20)],
    [t.volatility60Label, formatPercent(signal.volatility_60)],
    [t.volatility120Label, formatPercent(signal.volatility_120)],
    [t.drawdown120Label, formatPercent(signal.max_drawdown_120)],
    [t.sma20Label, formatNumber(signal.sma_20)],
    [t.sma60Label, formatNumber(signal.sma_60)],
    [t.averageVolumeLabel, formatNumber(signal.average_volume_20)],
    [t.volumeRatioLabel, formatNumber(signal.volume_ratio_20_60)],
    [t.activityProxyLabel, formatNumber(signal.average_trading_value_20)],
  ] as const;
  return (
    <section aria-labelledby="stock-beta-metrics-title" className="report-section">
      <h3 id="stock-beta-metrics-title">{t.signalsHeading}</h3>
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
  locale,
  t,
}: {
  readonly detail: OwnerEquityV2SignalDetailModel;
  readonly locale?: Locale | undefined;
  readonly t: StockBetaDictionary;
}) {
  const { signal } = detail;
  return (
    <>
      <StockBetaPolicyNotice locale={locale} />
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
            <h2 id="stock-beta-detail-heading">{signal.instrument_id}</h2>
            <p className="stock-beta-instrument-id">{signal.instrument_id}</p>
          </div>
          <div className="status-cluster">
            <span>
              {t.rankLabel} {signal.rank}
            </span>
            <StatusPill
              label={`${signal.condition} · ${conditionLabel(signal.condition, t)}`}
              tone={conditionTone(signal.condition)}
            />
            <span>
              {t.scoreLabel} {formatNumber(signal.score)}
            </span>
            <span>
              {t.asOfLabel} {detail.snapshot.as_of}
            </span>
          </div>
        </header>
        <p className="supporting-copy">{t.detailDescription}</p>
        <SnapshotSummary detail={detail} t={t} />
        <DetailMetrics detail={detail} t={t} />
      </section>
    </>
  );
}
