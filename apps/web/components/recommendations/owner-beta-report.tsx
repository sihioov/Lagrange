import { StatusPill } from "@/components/states/status-pill";
import type { RecommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import { formatDate, formatPercentage, formatTimestamp } from "@/lib/products/format";
import type { OwnerBetaItemModel, OwnerBetaRunModel } from "@/lib/products/owner-beta-contracts";

function hash(value: string | null | undefined, t: RecommendationsDictionary): string {
  return value ?? t.notReported;
}

function factorList(item: OwnerBetaItemModel, t: RecommendationsDictionary) {
  const factors = Object.entries(item.factors);
  return factors.length === 0 ? (
    <span>{t.notReported}</span>
  ) : (
    <dl className="factor-list">
      {factors.map(([name, value]) => (
        <div key={name}>
          <dt>{name.replaceAll("_", " ")}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function reasonList(item: OwnerBetaItemModel, t: RecommendationsDictionary) {
  return item.reason_codes.length === 0 ? (
    <span>{t.notReported}</span>
  ) : (
    <ul className="code-list">
      {item.reason_codes.map((reason) => (
        <li key={reason}>{reason}</li>
      ))}
    </ul>
  );
}

export type OwnerBetaReportProps = {
  readonly run: OwnerBetaRunModel;
  readonly t: RecommendationsDictionary;
};

export function OwnerBetaReport({ run, t }: OwnerBetaReportProps) {
  const items = run.items ?? [];
  return (
    <section aria-labelledby="owner-beta-report-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">{t.ownerBetaReportEyebrow}</p>
          <h2 id="owner-beta-report-title">{t.ownerBetaReportHeading}</h2>
          <p>{t.proposalDisclaimer}</p>
        </div>
        <div className="status-cluster">
          <StatusPill label={run.status} tone="success" />
          <span>{t.asOf(formatDate(run.as_of))}</span>
        </div>
      </header>

      <section aria-labelledby="owner-beta-contract-title" className="report-section">
        <h3 id="owner-beta-contract-title">{t.provenanceHeading}</h3>
        <dl className="provenance-grid">
          <div>
            <dt>{t.ownerBetaAudienceLabel}</dt>
            <dd>{t.ownerBetaAudienceValue}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaCapabilityLabel}</dt>
            <dd>{t.ownerBetaCapabilityValue}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaVendorSnapshotLabel}</dt>
            <dd>{t.ownerBetaVendorSnapshotValue}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaStrictPitLabel}</dt>
            <dd>{t.ownerBetaStrictPitValue}</dd>
          </div>
          <div>
            <dt>{t.strategyConfigurationLabel}</dt>
            <dd>
              {run.strategy_id}@{run.strategy_version} ({run.strategy_config_id})
            </dd>
          </div>
          <div>
            <dt>{t.ownerBetaStrategyConfigHashLabel}</dt>
            <dd>{hash(run.strategy_config_sha256, t)}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaCandidateContentHashLabel}</dt>
            <dd>{hash(run.candidate_content_sha256, t)}</dd>
          </div>
          <div>
            <dt>{t.factorSnapshotLabel}</dt>
            <dd>{hash(run.factor_snapshot_sha256, t)}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaTargetSnapshotHashLabel}</dt>
            <dd>{hash(run.target_snapshot_sha256, t)}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaArtifactManifestHashLabel}</dt>
            <dd>{run.artifact_manifest_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaStage5ManifestHashLabel}</dt>
            <dd>{run.stage5_manifest_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaActionManifestHashLabel}</dt>
            <dd>{run.action_manifest_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaApprovalRegistryHashLabel}</dt>
            <dd>{run.approval_registry_sha256}</dd>
          </div>
          <div>
            <dt>{t.columnCreated}</dt>
            <dd>{formatTimestamp(run.created_at)}</dd>
          </div>
          <div>
            <dt>{t.columnRunId}</dt>
            <dd className="data-cell">{run.id}</dd>
          </div>
        </dl>
      </section>

      <section aria-labelledby="owner-beta-results-title" className="report-section">
        <div className="section-heading">
          <div>
            <h3 id="owner-beta-results-title">{t.ownerBetaItemsHeading}</h3>
          </div>
          {run.cash_weight === undefined || run.cash_weight === null ? null : (
            <p>{t.cashAllocation(formatPercentage(run.cash_weight))}</p>
          )}
        </div>
        {items.length === 0 ? (
          <p className="empty-copy">{t.noInstrumentsSelected}</p>
        ) : (
          <div className="data-table-wrap">
            <table>
              <caption>{t.ownerBetaItemsCaption}</caption>
              <thead>
                <tr>
                  <th scope="col">{t.columnRank}</th>
                  <th scope="col">{t.columnInstrument}</th>
                  <th scope="col">{t.columnTargetWeight}</th>
                  <th scope="col">{t.columnStatus}</th>
                  <th scope="col">{t.columnFactorScores}</th>
                  <th scope="col">{t.columnSelectionReasons}</th>
                </tr>
              </thead>
              <tbody>
                {items.map((item) => (
                  <tr key={item.instrument_id}>
                    <td>{item.rank ?? "—"}</td>
                    <th scope="row">{item.instrument_id}</th>
                    <td>
                      {item.target_weight === undefined || item.target_weight === null
                        ? "—"
                        : formatPercentage(item.target_weight)}
                    </td>
                    <td>{item.excluded ? t.exclusionsHeading : t.selectedCandidatesHeading}</td>
                    <td>{factorList(item, t)}</td>
                    <td>
                      {item.excluded && item.exclusion_reason !== undefined
                        ? `${item.exclusion_reason}: `
                        : null}
                      {reasonList(item, t)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </section>
  );
}
