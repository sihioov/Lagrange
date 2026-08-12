import { ReportFooter } from "@/components/reports/report-footer";
import { StatusPill } from "@/components/states/status-pill";
import {
  type RecommendationItemModel,
  type RecommendationRunModel,
  recommendationProvenance,
} from "@/lib/products/contracts";
import { formatDate, formatDecimal, formatPercentage } from "@/lib/products/format";

function factorValue(value: unknown): string {
  if (typeof value === "string") {
    return /^-?\d+(?:\.\d+)?$/.test(value) ? formatDecimal(value, 4) : value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return "Structured server evidence";
}

function ScoreList({ item }: { readonly item: RecommendationItemModel }) {
  const factors = Object.entries(item.factors ?? {});
  return factors.length === 0 ? (
    <span>Not reported</span>
  ) : (
    <dl className="factor-list">
      {factors.map(([name, value]) => (
        <div key={name}>
          <dt>{name.replaceAll("_", " ")}</dt>
          <dd>{factorValue(value)}</dd>
        </div>
      ))}
    </dl>
  );
}

function ReasonList({ item }: { readonly item: RecommendationItemModel }) {
  const reasons = item.reason_codes ?? [];
  return reasons.length === 0 ? (
    <span>Not reported</span>
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
};

export function RecommendationReport({ licenseState, run }: RecommendationReportProps) {
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
          <p className="eyebrow">Latest governed output</p>
          <h2 id="recommendation-report-title">Strategy-based proposal</h2>
          <p>Strategy-based proposal, not investment advice. Review warnings and the as-of date.</p>
        </div>
        <div className="status-cluster">
          <StatusPill
            label={run.status}
            tone={run.status === "SUCCEEDED" ? "success" : "warning"}
          />
          <span>As of {formatDate(run.as_of)}</span>
        </div>
      </header>
      {provenance.warnings.length === 0 ? null : (
        <aside aria-label="Recommendation warnings" className="warning-strip" role="status">
          <strong>
            {provenance.warnings.some((warning) => warning.startsWith("Stale result"))
              ? "Stale result"
              : "Warnings"}
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
          {allCash
            ? "All-cash allocation: the governed constraints did not select an instrument for this proposal."
            : `Cash allocation: ${formatPercentage(cashWeight)}.`}
        </p>
      )}
      {provenance.origin === "synthetic" ? (
        <aside className="warning-strip" role="status">
          <strong>Synthetic QA data</strong>
          <p>This proposal is based on synthetic QA data and is not a live market-data result.</p>
        </aside>
      ) : null}
      <section aria-labelledby="recommendation-lineage-title" className="report-section">
        <h3 id="recommendation-lineage-title">Run provenance</h3>
        <dl className="provenance-grid">
          <div>
            <dt>Origin</dt>
            <dd>{provenance.origin ?? "Not reported"}</dd>
          </div>
          <div>
            <dt>Dataset version</dt>
            <dd>{provenance.dataset_version ?? "Not reported"}</dd>
          </div>
          <div>
            <dt>Universe snapshot</dt>
            <dd>{provenance.universe_snapshot_id ?? "Not reported"}</dd>
          </div>
          <div>
            <dt>Factor snapshot</dt>
            <dd>{provenance.factor_snapshot_hash ?? "Not reported"}</dd>
          </div>
          <div>
            <dt>Portfolio snapshot</dt>
            <dd>{provenance.portfolio_snapshot_id ?? "Not reported"}</dd>
          </div>
          <div>
            <dt>Dataset manifest</dt>
            <dd>
              {run.provenance.dataset_manifest_sha256 ??
                provenance.manifest_sha256 ??
                "Not reported"}
            </dd>
          </div>
        </dl>
      </section>
      <section aria-labelledby="selected-candidates-title" className="report-section">
        <h3 id="selected-candidates-title">Selected candidates</h3>
        {selected.length === 0 ? (
          <p className="empty-copy">No instruments were selected.</p>
        ) : (
          <div className="data-table-wrap">
            <table>
              <caption>Selected instruments and target weights</caption>
              <thead>
                <tr>
                  <th scope="col">Rank</th>
                  <th scope="col">Instrument</th>
                  <th scope="col">Target weight</th>
                  <th scope="col">Factor scores</th>
                  <th scope="col">Selection reasons</th>
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
                      <ScoreList item={item} />
                    </td>
                    <td>
                      <ReasonList item={item} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      <section aria-labelledby="excluded-candidates-title" className="report-section">
        <h3 id="excluded-candidates-title">Exclusions</h3>
        {excluded.length === 0 ? (
          <p className="empty-copy">No instruments were excluded.</p>
        ) : (
          <div className="data-table-wrap">
            <table>
              <caption>Excluded instruments and policy reasons</caption>
              <thead>
                <tr>
                  <th scope="col">Instrument</th>
                  <th scope="col">Reason</th>
                  <th scope="col">Evidence</th>
                </tr>
              </thead>
              <tbody>
                {excluded.map((item) => (
                  <tr key={item.instrument_id}>
                    <th scope="row">{item.instrument_id}</th>
                    <td>{item.exclusion_reason ?? "No exclusion reason was reported."}</td>
                    <td>
                      <ReasonList item={item} />
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
