import type { CandidateFeed } from "@/lib/products/candidate-contracts";
import { formatDate, formatTimestamp } from "@/lib/products/format";

type ResearchProvenanceProps = Pick<
  CandidateFeed,
  "as_of" | "cutoff_at" | "dataset_pins" | "disclaimer" | "license_attributions" | "scoring_config"
>;

export function ResearchProvenance({
  as_of: asOf,
  cutoff_at: cutoffAt,
  dataset_pins: pins,
  disclaimer,
  license_attributions: licenses,
  scoring_config: scoring,
}: ResearchProvenanceProps) {
  return (
    <footer className="report-footer candidate-provenance">
      <div>
        <h3>Point-in-time provenance</h3>
        <p className="supporting-copy">
          Every score is tied to the source versions visible by the {formatDate(asOf)} cutoff.
        </p>
      </div>
      <dl className="provenance-grid">
        <div>
          <dt>As of</dt>
          <dd>{formatDate(asOf)}</dd>
        </div>
        <div>
          <dt>Cutoff</dt>
          <dd>{formatTimestamp(cutoffAt)}</dd>
        </div>
        <div>
          <dt>Scoring contract</dt>
          <dd>{scoring.version}</dd>
        </div>
        <div>
          <dt>Price curated version</dt>
          <dd>v{pins.price.curated_version}</dd>
        </div>
        <div>
          <dt>Universe snapshot</dt>
          <dd>{pins.universe_snapshot_id}</dd>
        </div>
        <div>
          <dt>Sector version</dt>
          <dd>{pins.sector_version_id}</dd>
        </div>
        <div>
          <dt>Input identity</dt>
          <dd>{pins.input_identity_sha256}</dd>
        </div>
        <div>
          <dt>Scoring hash</dt>
          <dd>{scoring.sha256}</dd>
        </div>
      </dl>
      <details className="lineage-details">
        <summary>Exact dataset pins</summary>
        <dl className="factor-list">
          <div>
            <dt>Price dataset</dt>
            <dd>{pins.price.dataset_version_id}</dd>
          </div>
          <div>
            <dt>Price manifest</dt>
            <dd>{pins.price.manifest_sha256}</dd>
          </div>
          <div>
            <dt>Market-status dataset</dt>
            <dd>{pins.market_status.dataset_version_id}</dd>
          </div>
          <div>
            <dt>Market-status manifest</dt>
            <dd>{pins.market_status.manifest_sha256}</dd>
          </div>
          <div>
            <dt>Investor-flow dataset</dt>
            <dd>{pins.flow.dataset_version_id}</dd>
          </div>
          <div>
            <dt>Investor-flow manifest</dt>
            <dd>{pins.flow.manifest_sha256}</dd>
          </div>
          <div>
            <dt>Fundamental dataset</dt>
            <dd>{pins.fundamental.dataset_version_id}</dd>
          </div>
          <div>
            <dt>Fundamental manifest</dt>
            <dd>{pins.fundamental.manifest_sha256}</dd>
          </div>
        </dl>
      </details>
      <div className="report-warnings">
        <h3>Source licenses</h3>
        <ul>
          {licenses.map((license) => (
            <li key={`${license.source}:${license.dataset_id}`}>
              <strong>{license.source}</strong> · {license.dataset_id}
              {license.license_ref === null || license.license_ref === undefined
                ? null
                : ` · ${license.license_ref}`}
              <small>
                {" "}
                · entitlement {license.entitlement_id} · contract {license.contract_reference}
              </small>
            </li>
          ))}
        </ul>
      </div>
      <aside className="warning-strip" role="note">
        <strong>Research limitation</strong>
        <p>{disclaimer}</p>
        <p>Scenarios are deterministic evidence triggers, not probabilities or target prices.</p>
      </aside>
    </footer>
  );
}
