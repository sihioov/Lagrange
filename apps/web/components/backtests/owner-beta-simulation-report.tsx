import { ownerBetaSimulationDictionary } from "@/lib/i18n/dictionaries/owner-beta-simulation";
import type { Locale } from "@/lib/i18n/locale";
import type {
  OwnerBetaSimulationPair,
  SimulationLedgerMetrics,
  SimulationResult,
} from "@/lib/products/owner-beta-simulation-contracts";

const RECENT_DAILY_ROWS = 30;
const MONEY_DIVISOR = 100n;
const RATIO_PERCENT_CENT_DIVISOR = 100_000_000n;
type NumericMetric =
  | "period_return"
  | "ending_nav"
  | "ending_cash"
  | "ending_holdings"
  | "cumulative_costs"
  | "buy_notional"
  | "sell_notional"
  | "maximum_drawdown"
  | "period_turnover"
  | "fill_count";

function roundHalfEven(value: bigint, divisor: bigint): bigint {
  const quotient = value / divisor;
  const remainder = value % divisor;
  const doubled = remainder * 2n;
  if (doubled > divisor || (doubled === divisor && quotient % 2n !== 0n)) {
    return quotient + 1n;
  }
  return quotient;
}

function grouped(integer: string): string {
  return integer.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

function fixedTwo(value: bigint, suffix: string, prefix = ""): string {
  const negative = value < 0n;
  const magnitude = negative ? -value : value;
  const digits = magnitude.toString().padStart(3, "0");
  const integer = grouped(digits.slice(0, -2));
  const fraction = digits.slice(-2);
  const sign = negative && magnitude !== 0n ? "−" : "";
  return `${sign}${prefix}${integer}.${fraction}${suffix}`;
}

/** Formats a ratio with integer arithmetic only: input scale 12, rounded to hundredths of a percent. */
export function formatSimulationPercent(value: string, suffix = "%"): string {
  const negative = value.startsWith("-");
  const units = BigInt(value.replace(/^-/, "").replace(".", ""));
  const rounded = roundHalfEven(units, RATIO_PERCENT_CENT_DIVISOR);
  return fixedTwo(negative ? -rounded : rounded, suffix);
}

/** Formats a scale-4 KRW amount with integer arithmetic and ties-to-even rounding. */
export function formatSimulationMoney(value: string): string {
  const negative = value.startsWith("-");
  const units = BigInt(value.replace(/^-/, "").replace(".", ""));
  const rounded = roundHalfEven(units, MONEY_DIVISOR);
  return fixedTwo(negative ? -rounded : rounded, "", "₩");
}

function formatCount(value: number): string {
  return grouped(String(value));
}

function formatHoldings(metrics: SimulationLedgerMetrics, locale: Locale): string {
  const t = ownerBetaSimulationDictionary[locale];
  if (metrics.ending_holdings.length === 0) return t.zeroSharesLabel;
  return metrics.ending_holdings
    .map((holding) => `${formatCount(holding.quantity)} ${t.sharesLabel}`)
    .join(", ");
}

function metricValue(
  metric: NumericMetric,
  metrics: SimulationLedgerMetrics,
  locale: Locale,
): string {
  switch (metric) {
    case "period_return":
    case "maximum_drawdown":
    case "period_turnover":
      return formatSimulationPercent(metrics[metric]);
    case "ending_nav":
    case "ending_cash":
    case "cumulative_costs":
    case "buy_notional":
    case "sell_notional":
      return formatSimulationMoney(metrics[metric]);
    case "ending_holdings":
      return formatHoldings(metrics, locale);
    case "fill_count":
      return formatCount(metrics.fill_count);
  }
}

function rawMetric(metric: NumericMetric, metrics: SimulationLedgerMetrics): string | null {
  return metric === "ending_holdings" ? null : String(metrics[metric]);
}

function ExactMetricCell({
  locale,
  metric,
  metrics,
}: {
  readonly locale: Locale;
  readonly metric: NumericMetric;
  readonly metrics: SimulationLedgerMetrics;
}) {
  const raw = rawMetric(metric, metrics);
  return (
    <td>
      {metricValue(metric, metrics, locale)}
      {raw === null ? null : (
        <details>
          <summary>{ownerBetaSimulationDictionary[locale].exactValueLabel}</summary>
          <code>{raw}</code>
        </details>
      )}
    </td>
  );
}

function ExactRatioCell({
  locale,
  suffix = "%",
  value,
}: {
  readonly locale: Locale;
  readonly suffix?: string;
  readonly value: string | undefined;
}) {
  if (value === undefined) {
    return <td>{ownerBetaSimulationDictionary[locale].notApplicableLabel}</td>;
  }
  return (
    <td>
      {formatSimulationPercent(value, suffix)}
      <details>
        <summary>{ownerBetaSimulationDictionary[locale].exactValueLabel}</summary>
        <code>{value}</code>
      </details>
    </td>
  );
}

type ProfileTableProps = {
  readonly result: SimulationResult;
  readonly title: string;
  readonly locale: Locale;
};

function ProfileTable({ result, title, locale }: ProfileTableProps) {
  const t = ownerBetaSimulationDictionary[locale];
  const rows = [
    { key: "period_return" as const, label: t.periodReturnLabel },
    { key: "ending_nav" as const, label: t.endingNavLabel },
    { key: "ending_cash" as const, label: t.endingCashLabel },
    { key: "ending_holdings" as const, label: t.endingHoldingsLabel },
    { key: "cumulative_costs" as const, label: t.cumulativeCostsLabel },
    { key: "buy_notional" as const, label: t.buyNotionalLabel },
    { key: "sell_notional" as const, label: t.sellNotionalLabel },
    { key: "maximum_drawdown" as const, label: t.maximumDrawdownLabel },
    { key: "period_turnover" as const, label: t.periodTurnoverLabel },
    { key: "fill_count" as const, label: t.fillCountLabel },
  ];
  return (
    <div className="report-section">
      <h3>{title}</h3>
      <div className="data-table-wrap">
        <table>
          <thead>
            <tr>
              <th scope="col">{t.metricColumn}</th>
              <th scope="col">{t.strategyColumn}</th>
              <th scope="col">{t.benchmarkColumn}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.key}>
                <th scope="row">{row.label}</th>
                <ExactMetricCell
                  locale={locale}
                  metric={row.key}
                  metrics={result.strategy_ledger.metrics}
                />
                <ExactMetricCell
                  locale={locale}
                  metric={row.key}
                  metrics={result.benchmark_ledger.metrics}
                />
              </tr>
            ))}
            <tr>
              <th scope="row">{t.benchmarkDifferenceLabel}</th>
              <ExactRatioCell
                locale={locale}
                suffix=" pp"
                value={result.strategy_ledger.benchmark_return_difference}
              />
              <td>{t.notApplicableLabel}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}

export type OwnerBetaSimulationReportProps = {
  readonly locale: Locale;
  readonly pair: OwnerBetaSimulationPair;
};

export function OwnerBetaSimulationReport({ locale, pair }: OwnerBetaSimulationReportProps) {
  const t = ownerBetaSimulationDictionary[locale];
  const illustrative = pair.illustrative_10bp_per_side.result;
  const zeroCost = pair.zero_cost_reference.result;
  const sessions = illustrative.schedule.evaluation_sessions;
  const visibleDates = sessions.slice(-RECENT_DAILY_ROWS);
  const illustrativeStrategy = new Map(
    illustrative.strategy_ledger.metrics.daily_returns.map(
      (item) => [item.session_date, item.value] as const,
    ),
  );
  const illustrativeBenchmark = new Map(
    illustrative.benchmark_ledger.metrics.daily_returns.map(
      (item) => [item.session_date, item.value] as const,
    ),
  );
  const zeroStrategy = new Map(
    zeroCost.strategy_ledger.metrics.daily_returns.map(
      (item) => [item.session_date, item.value] as const,
    ),
  );
  const zeroBenchmark = new Map(
    zeroCost.benchmark_ledger.metrics.daily_returns.map(
      (item) => [item.session_date, item.value] as const,
    ),
  );

  return (
    <section
      aria-labelledby="owner-beta-simulation-report-title"
      className="product-section"
      data-testid="owner-beta-simulation-report"
    >
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.reportSectionLabel}</p>
          <h2 id="owner-beta-simulation-report-title">{t.reportTitle}</h2>
        </div>
        <p>{t.reportDescription}</p>
      </div>
      <ProfileTable locale={locale} result={illustrative} title={t.illustrativeProfileTitle} />
      <ProfileTable locale={locale} result={zeroCost} title={t.zeroCostProfileTitle} />
      <div className="report-section">
        <h3>{t.dailyReturnsTitle}</h3>
        <div className="data-table-wrap">
          <table>
            <caption>{t.dailyReturnsCaption(visibleDates.length, sessions.length)}</caption>
            <thead>
              <tr>
                <th scope="col">{t.dailyDateColumn}</th>
                <th scope="col">{t.dailyIllustrativeStrategyColumn}</th>
                <th scope="col">{t.dailyIllustrativeBenchmarkColumn}</th>
                <th scope="col">{t.dailyZeroStrategyColumn}</th>
                <th scope="col">{t.dailyZeroBenchmarkColumn}</th>
              </tr>
            </thead>
            <tbody>
              {visibleDates.map((date) => (
                <tr key={date}>
                  <th scope="row">{date}</th>
                  <ExactRatioCell locale={locale} value={illustrativeStrategy.get(date)} />
                  <ExactRatioCell locale={locale} value={illustrativeBenchmark.get(date)} />
                  <ExactRatioCell locale={locale} value={zeroStrategy.get(date)} />
                  <ExactRatioCell locale={locale} value={zeroBenchmark.get(date)} />
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
