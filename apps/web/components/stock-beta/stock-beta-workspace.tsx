import Link from "next/link";
import { StatePanel } from "@/components/states/state-panel";
import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type {
  OwnerBetaEquitySignalCondition,
  OwnerBetaEquitySignalRowModel,
  OwnerBetaEquitySignalsFilters,
  OwnerBetaEquitySignalsLatestModel,
  OwnerBetaEquitySignalsProvenanceModel,
  OwnerBetaEquitySignalsRangeKey,
  OwnerBetaEquitySignalsScreenModel,
} from "@/lib/products/equity-signals-contracts";

type StockBetaData = OwnerBetaEquitySignalsLatestModel | OwnerBetaEquitySignalsScreenModel;

type NumericLabelKey =
  | "scoreLabel"
  | "return20Label"
  | "return60Label"
  | "return120Label"
  | "volatility20Label"
  | "volatility60Label"
  | "volatility120Label"
  | "drawdown120Label"
  | "activityLabel";

const RANGE_FIELDS: readonly {
  readonly key: OwnerBetaEquitySignalsRangeKey;
  readonly label: NumericLabelKey;
}[] = [
  { key: "score", label: "scoreLabel" },
  { key: "return_20", label: "return20Label" },
  { key: "return_60", label: "return60Label" },
  { key: "return_120", label: "return120Label" },
  { key: "volatility_20", label: "volatility20Label" },
  { key: "volatility_60", label: "volatility60Label" },
  { key: "volatility_120", label: "volatility120Label" },
  { key: "max_drawdown_120", label: "drawdown120Label" },
  { key: "average_trading_value_20", label: "activityLabel" },
];

const CONDITION_OPTIONS: readonly OwnerBetaEquitySignalCondition[] = [
  "BULLISH",
  "NEUTRAL",
  "BEARISH",
];

function conditionLabel(condition: OwnerBetaEquitySignalCondition, t: StockBetaDictionary): string {
  switch (condition) {
    case "BULLISH":
      return t.bullishLabel;
    case "NEUTRAL":
      return t.neutralLabel;
    case "BEARISH":
      return t.bearishLabel;
  }
}

function conditionTone(condition: OwnerBetaEquitySignalCondition): StatusTone {
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

function formatFilterValue(
  filters: OwnerBetaEquitySignalsFilters,
  key: OwnerBetaEquitySignalsRangeKey,
  bound: "max" | "min",
): string {
  const value = filters.ranges[key]?.[bound];
  return value === undefined ? "" : String(value);
}

export function StockBetaPolicyNotice({ t }: { readonly t: StockBetaDictionary }) {
  return (
    <aside aria-label={t.policyAriaLabel} className="warning-strip stock-beta-policy" role="note">
      <strong>{t.warningLabel}</strong>
      <p>{t.fixedListPolicy}</p>
      <p>{t.originalPricePolicy}</p>
      <p>{t.activityPolicy}</p>
      <p>{t.conditionPolicy}</p>
    </aside>
  );
}

function StockBetaFilters({
  filters,
  t,
}: {
  readonly filters: OwnerBetaEquitySignalsFilters;
  readonly t: StockBetaDictionary;
}) {
  return (
    <section
      aria-labelledby="stock-beta-filters-title"
      className="workflow-panel stock-beta-filters"
    >
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.filtersEyebrow}</p>
          <h2 id="stock-beta-filters-title">{t.filtersHeading}</h2>
        </div>
        <p>{t.filtersDescription}</p>
      </div>
      <form action="/stock-beta" className="workflow-form" method="get">
        <fieldset>
          <legend>{t.conditionLabel}</legend>
          <div className="choice-row">
            {CONDITION_OPTIONS.map((condition) => (
              <label key={condition}>
                <input
                  defaultChecked={filters.conditions.includes(condition)}
                  name="condition"
                  type="checkbox"
                  value={condition}
                />
                {condition} · {conditionLabel(condition, t)}
              </label>
            ))}
          </div>
        </fieldset>
        <div className="field-grid stock-beta-range-grid">
          {RANGE_FIELDS.map(({ key, label }) => (
            <fieldset className="stock-beta-range-field" key={key}>
              <legend>{t[label]}</legend>
              <label className="form-field">
                <span>{t.minLabel}</span>
                <input
                  defaultValue={formatFilterValue(filters, key, "min")}
                  inputMode="decimal"
                  name={`${key}_min`}
                  step="any"
                  type="number"
                />
              </label>
              <label className="form-field">
                <span>{t.maxLabel}</span>
                <input
                  defaultValue={formatFilterValue(filters, key, "max")}
                  inputMode="decimal"
                  name={`${key}_max`}
                  step="any"
                  type="number"
                />
              </label>
            </fieldset>
          ))}
        </div>
        <label className="form-field stock-beta-trend-field">
          <span>{t.trendLabel}</span>
          <select
            defaultValue={filters.trendUp === undefined ? "" : filters.trendUp ? "up" : "down"}
            name="trend"
          >
            <option value="">—</option>
            <option value="up">{t.trendUpLabel}</option>
            <option value="down">{t.trendDownLabel}</option>
          </select>
        </label>
        <div className="inline-form">
          <button className="primary-action" type="submit">
            {t.applyFilters}
          </button>
          <Link className="quiet-action" href="/stock-beta">
            {t.clearFilters}
          </Link>
        </div>
      </form>
    </section>
  );
}

function StockBetaTopFive({
  rows,
  t,
}: {
  readonly rows: readonly OwnerBetaEquitySignalRowModel[];
  readonly t: StockBetaDictionary;
}) {
  if (rows.length === 0) {
    return <StatePanel kind="empty" message={t.noResultsMessage} title={t.noResultsTitle} />;
  }
  return (
    <section
      aria-labelledby="stock-beta-top-five-title"
      className="data-report stock-beta-top-five"
    >
      <header className="report-heading">
        <div>
          <p className="eyebrow">{t.topFiveEyebrow}</p>
          <h2 id="stock-beta-top-five-title">{t.topFiveHeading}</h2>
          <p>{t.topFiveDescription}</p>
        </div>
      </header>
      <div className="stock-beta-top-grid" data-testid="stock-beta-top-five">
        {rows.map((row) => (
          <article className="stock-beta-top-card" key={row.instrument_id}>
            <div className="stock-beta-card-meta">
              <span>
                {t.rankLabel} {row.rank}
              </span>
              <StatusPill label={row.condition} tone={conditionTone(row.condition)} />
            </div>
            <h3>
              <Link href={`/stock-beta/${encodeURIComponent(row.instrument_id)}`}>
                {row.instrument_name}
              </Link>
            </h3>
            <p className="stock-beta-instrument-id">{row.instrument_id}</p>
            <dl className="stock-beta-card-metrics">
              <div>
                <dt>{t.scoreLabel}</dt>
                <dd>{formatNumber(row.score)}</dd>
              </div>
              <div>
                <dt>{t.return20Label}</dt>
                <dd>{formatPercent(row.return_20)}</dd>
              </div>
              <div>
                <dt>{t.activityProxyLabel}</dt>
                <dd>{formatNumber(row.average_trading_value_20)}</dd>
              </div>
            </dl>
          </article>
        ))}
      </div>
    </section>
  );
}

function StockBetaRankTable({
  rows,
  t,
}: {
  readonly rows: readonly OwnerBetaEquitySignalRowModel[];
  readonly t: StockBetaDictionary;
}) {
  if (rows.length === 0) return null;
  return (
    <section
      aria-labelledby="stock-beta-ranked-table-title"
      className="data-report stock-beta-rank-report"
    >
      <header className="report-heading">
        <div>
          <p className="eyebrow">{t.rankTableEyebrow}</p>
          <h2 id="stock-beta-ranked-table-title">{t.rankTableHeading}</h2>
        </div>
      </header>
      <div className="data-table-wrap">
        <table data-testid="stock-beta-rank-table">
          <caption>{t.rankTableCaption}</caption>
          <thead>
            <tr>
              <th scope="col">{t.rankLabel}</th>
              <th scope="col">{t.instrumentLabel}</th>
              <th scope="col">{t.scoreLabel}</th>
              <th scope="col">{t.tableConditionLabel}</th>
              <th scope="col">{t.return20Label}</th>
              <th scope="col">{t.return60Label}</th>
              <th scope="col">{t.return120Label}</th>
              <th scope="col">{t.volatility20Label}</th>
              <th scope="col">{t.volatility60Label}</th>
              <th scope="col">{t.volatility120Label}</th>
              <th scope="col">{t.drawdown120Label}</th>
              <th scope="col">{t.sma20Label}</th>
              <th scope="col">{t.sma60Label}</th>
              <th scope="col">{t.averageVolumeLabel}</th>
              <th scope="col">{t.volumeRatioLabel}</th>
              <th scope="col">{t.activityProxyLabel}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.instrument_id}>
                <th scope="row">{row.rank}</th>
                <td>
                  <Link
                    className="data-link"
                    href={`/stock-beta/${encodeURIComponent(row.instrument_id)}`}
                  >
                    {row.instrument_name}
                    <small>{row.instrument_id}</small>
                  </Link>
                </td>
                <td className="score-emphasis">{formatNumber(row.score)}</td>
                <td>
                  <StatusPill label={row.condition} tone={conditionTone(row.condition)} />
                </td>
                <td>{formatPercent(row.return_20)}</td>
                <td>{formatPercent(row.return_60)}</td>
                <td>{formatPercent(row.return_120)}</td>
                <td>{formatPercent(row.volatility_20)}</td>
                <td>{formatPercent(row.volatility_60)}</td>
                <td>{formatPercent(row.volatility_120)}</td>
                <td>{formatPercent(row.max_drawdown_120)}</td>
                <td>{formatNumber(row.sma_20)}</td>
                <td>{formatNumber(row.sma_60)}</td>
                <td>{formatNumber(row.average_volume_20)}</td>
                <td>{formatNumber(row.volume_ratio_20_60)}</td>
                <td>{formatNumber(row.average_trading_value_20)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function provenanceBoolean(value: boolean, t: StockBetaDictionary): string {
  return value ? t.yes : t.no;
}

export function StockBetaProvenance({
  provenance,
  t,
}: {
  readonly provenance: OwnerBetaEquitySignalsProvenanceModel;
  readonly t: StockBetaDictionary;
}) {
  const fields = [
    [t.audienceLabel, provenance.audience],
    [t.capabilityLabel, provenance.capability],
    [t.selectionBasisLabel, provenance.selection_basis],
    [t.indexMembershipLabel, provenance.index_membership],
    [t.redistributionLabel, provenance.redistribution],
    [t.publicationStatusLabel, provenance.publication_status],
    [t.materializationStatusLabel, provenance.materialization_status],
    [t.registrationStatusLabel, provenance.registration_status],
    [t.universeHashLabel, provenance.universe_sha256],
    [t.entitlementHashLabel, provenance.entitlement_sha256],
    [t.registryHashLabel, provenance.registry_sha256],
    [t.artifactHashLabel, provenance.artifact_content_sha256],
    [t.snapshotHashLabel, provenance.snapshot_content_sha256],
    [t.batchIdLabel, provenance.batch_id],
    [t.asOfLabel, provenance.as_of],
    [t.factorVersionLabel, provenance.factor_version],
    [t.vendorSnapshotLabel, provenanceBoolean(provenance.vendor_snapshot, t)],
    [t.strictPitLabel, provenanceBoolean(provenance.strict_pit, t)],
    [t.originalPriceLabel, provenanceBoolean(provenance.original_price, t)],
    [t.warningLabel, provenance.warning],
    [t.activityProxyLabel, provenance.activity_proxy],
  ] as const;
  return (
    <footer className="report-footer stock-beta-provenance" data-testid="stock-beta-provenance">
      <div>
        <h2>{t.provenanceHeading}</h2>
        <p className="supporting-copy">{t.provenanceDescription}</p>
      </div>
      <dl className="provenance-grid">
        {fields.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </footer>
  );
}

export function StockBetaWorkspace({
  data,
  filters,
  t,
}: {
  readonly data: StockBetaData;
  readonly filters: OwnerBetaEquitySignalsFilters;
  readonly t: StockBetaDictionary;
}) {
  const topFive = "top5" in data ? data.top5 : data.rows.slice(0, 5);
  return (
    <>
      <StockBetaPolicyNotice t={t} />
      <StockBetaFilters filters={filters} t={t} />
      <StockBetaTopFive rows={topFive} t={t} />
      <StockBetaRankTable rows={data.rows} t={t} />
      <StockBetaProvenance provenance={data.provenance} t={t} />
    </>
  );
}

export function stockBetaConditionLabel(
  condition: OwnerBetaEquitySignalCondition,
  t: StockBetaDictionary,
): string {
  return conditionLabel(condition, t);
}

export function stockBetaConditionTone(condition: OwnerBetaEquitySignalCondition): StatusTone {
  return conditionTone(condition);
}

export function stockBetaFormatNumber(value: number, fractionDigits = 2): string {
  return formatNumber(value, fractionDigits);
}

export function stockBetaFormatPercent(value: number): string {
  return formatPercent(value);
}
