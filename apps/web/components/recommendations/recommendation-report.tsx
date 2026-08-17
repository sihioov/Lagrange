import { ReportFooter } from "@/components/reports/report-footer";
import { StatusPill } from "@/components/states/status-pill";
import type { RecommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import {
  type RecommendationItemModel,
  type RecommendationRunModel,
  recommendationProvenance,
} from "@/lib/products/contracts";
import { formatDate, formatDecimal, formatPercentage } from "@/lib/products/format";

function factorValue(value: unknown, t: RecommendationsDictionary): string {
  if (typeof value === "string") {
    return /^-?\d+(?:\.\d+)?$/.test(value) ? formatDecimal(value, 4) : value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return t.structuredServerEvidence;
}

function ScoreList({
  item,
  t,
}: {
  readonly item: RecommendationItemModel;
  readonly t: RecommendationsDictionary;
}) {
  const factors = Object.entries(item.factors ?? {});
  return factors.length === 0 ? (
    <span>{t.notReported}</span>
  ) : (
    <dl className="factor-list">
      {factors.map(([name, value]) => (
        <div key={name}>
          <dt>{name.replaceAll("_", " ")}</dt>
          <dd>{factorValue(value, t)}</dd>
        </div>
      ))}
    </dl>
  );
}

function ReasonList({
  item,
  t,
}: {
  readonly item: RecommendationItemModel;
  readonly t: RecommendationsDictionary;
}) {
  const reasons = item.reason_codes ?? [];
  return reasons.length === 0 ? (
    <span>{t.notReported}</span>
  ) : (
    <ul className="code-list">
      {reasons.map((reason) => (
        <li key={reason}>{reason}</li>
      ))}
    </ul>
  );
}

export type RecommendationReportProps = {
  readonly licenseState: string;
  readonly run: RecommendationRunModel;
  readonly t: RecommendationsDictionary;
};

export function RecommendationReport({ licenseState, run, t }: RecommendationReportProps) {
  const items = run.items ?? [];
  const selected = items.filter((item) => !item.excluded);
  const excluded = items.filter((item) => item.excluded);
  const provenance = recommendationProvenance(run);
  const cashWeight = provenance.cash_weight;
  const allCash = cashWeight === "1.000000";
  return (
    <section aria-labelledby="recommendation-report-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">{t.reportEyebrow}</p>
          <h2 id="recommendation-report-title">{t.reportHeading}</h2>
          <p>{t.proposalDisclaimer}</p>
        </div>
        <div className="status-cluster">
          <StatusPill
            label={run.status}
            tone={run.status === "SUCCEEDED" ? "success" : "warning"}
          />
          <span>{t.asOf(formatDate(run.as_of))}</span>
        </div>
      </header>
      {provenance.warnings.length === 0 ? null : (
        <aside aria-label={t.warningsAriaLabel} className="warning-strip" role="status">
          <strong>
            {provenance.warnings.some((warning) => warning.startsWith("Stale result"))
              ? t.staleResultLabel
              : t.warningsLabel}
          </strong>
          <ul>
            {provenance.warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </aside>
      )}
      {cashWeight === undefined ? null : (
        <p className="supporting-copy">
          {allCash ? t.allCashAllocation : t.cashAllocation(formatPercentage(cashWeight))}
        </p>
      )}
      {provenance.origin === "synthetic" ? (
        <aside className="warning-strip" role="status">
          <strong>{t.syntheticDataLabel}</strong>
          <p>{t.syntheticDataMessage}</p>
        </aside>
      ) : null}
      <section aria-labelledby="recommendation-lineage-title" className="report-section">
        <h3 id="recommendation-lineage-title">{t.provenanceHeading}</h3>
        <dl className="provenance-grid">
          <div>
            <dt>{t.originLabel}</dt>
            <dd>{provenance.origin ?? t.notReported}</dd>
          </div>
          <div>
            <dt>{t.datasetVersionLabel}</dt>
            <dd>{provenance.dataset_version ?? t.notReported}</dd>
          </div>
          <div>
            <dt>{t.universeSnapshotLabel}</dt>
            <dd>{provenance.universe_snapshot_id ?? t.notReported}</dd>
          </div>
          <div>
            <dt>{t.factorSnapshotLabel}</dt>
            <dd>{provenance.factor_snapshot_hash ?? t.notReported}</dd>
          </div>
          <div>
            <dt>{t.portfolioSnapshotLabel}</dt>
            <dd>{provenance.portfolio_snapshot_id ?? t.notReported}</dd>
          </div>
          <div>
            <dt>{t.datasetManifestLabel}</dt>
            <dd>
              {run.provenance.dataset_manifest_sha256 ??
                provenance.manifest_sha256 ??
                t.notReported}
            </dd>
          </div>
        </dl>
      </section>
      <section aria-labelledby="selected-candidates-title" className="report-section">
        <h3 id="selected-candidates-title">{t.selectedCandidatesHeading}</h3>
        {selected.length === 0 ? (
          <p className="empty-copy">{t.noInstrumentsSelected}</p>
        ) : (
          <div className="data-table-wrap">
            <table>
              <caption>{t.selectedTableCaption}</caption>
              <thead>
                <tr>
                  <th scope="col">{t.columnRank}</th>
                  <th scope="col">{t.columnInstrument}</th>
                  <th scope="col">{t.columnTargetWeight}</th>
                  <th scope="col">{t.columnFactorScores}</th>
                  <th scope="col">{t.columnSelectionReasons}</th>
                </tr>
              </thead>
              <tbody>
                {selected.map((item) => (
                  <tr key={item.instrument_id}>
                    <td>{item.rank ?? "—"}</td>
                    <th scope="row">{item.instrument_id}</th>
                    <td>
                      {item.target_weight === null || item.target_weight === undefined
                        ? "—"
                        : formatPercentage(item.target_weight)}
                    </td>
                    <td>
                      <ScoreList item={item} t={t} />
                    </td>
                    <td>
                      <ReasonList item={item} t={t} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      <section aria-labelledby="excluded-candidates-title" className="report-section">
        <h3 id="excluded-candidates-title">{t.exclusionsHeading}</h3>
        {excluded.length === 0 ? (
          <p className="empty-copy">{t.noInstrumentsExcluded}</p>
        ) : (
          <div className="data-table-wrap">
            <table>
              <caption>{t.excludedTableCaption}</caption>
              <thead>
                <tr>
                  <th scope="col">{t.columnInstrument}</th>
                  <th scope="col">{t.columnReason}</th>
                  <th scope="col">{t.columnEvidence}</th>
                </tr>
              </thead>
              <tbody>
                {excluded.map((item) => (
                  <tr key={item.instrument_id}>
                    <th scope="row">{item.instrument_id}</th>
                    <td>{item.exclusion_reason ?? t.noExclusionReason}</td>
                    <td>
                      <ReasonList item={item} t={t} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      <ReportFooter asOf={run.as_of} licenseState={licenseState} provenance={provenance} />
    </section>
  );
}
