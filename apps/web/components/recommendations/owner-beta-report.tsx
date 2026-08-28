import { StatePanel } from "@/components/states/state-panel";
import { StatusPill } from "@/components/states/status-pill";
import type { RecommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import { formatDate, formatPercentage, formatTimestamp } from "@/lib/products/format";
import {
  type OwnerBetaItemModel,
  type OwnerBetaRunModel,
  ownerBetaRunSchema,
} from "@/lib/products/owner-beta-contracts";

function auditValue(value: string | null | undefined, t: RecommendationsDictionary): string {
  if (value === undefined) {
    return t.notReported;
  }
  return value === null ? "null" : value;
}

type DecimalParts = {
  readonly digits: bigint;
  readonly exponent: number;
  readonly fractionDigits: number;
  readonly negative: boolean;
};

function decimalParts(value: string): DecimalParts {
  const match =
    /^(?<negative>-?)(?<integer>0|[1-9]\d*)(?:\.(?<fraction>\d+))?(?:e(?<exponent>[+-]\d{2,}))?$/.exec(
      value,
    );
  if (
    match?.groups === undefined ||
    (match.groups["fraction"] === undefined && match.groups["exponent"] === undefined)
  ) {
    throw new Error("invalid factor decimal");
  }
  const fraction = match.groups["fraction"] ?? "";
  const exponent = Number(match.groups["exponent"] ?? "0");
  if (!Number.isSafeInteger(exponent) || !Number.isFinite(Number(value))) {
    throw new Error("invalid factor decimal");
  }
  return {
    digits: BigInt(`${match.groups["integer"]}${fraction}`),
    exponent,
    fractionDigits: fraction.length,
    negative: match.groups["negative"] === "-",
  };
}

function roundHalfEven(value: bigint, divisor: bigint): bigint {
  const quotient = value / divisor;
  const remainder = value % divisor;
  const doubled = remainder * 2n;
  if (doubled > divisor || (doubled === divisor && quotient % 2n !== 0n)) {
    return quotient + 1n;
  }
  return quotient;
}

/** Format a canonical factor decimal as a two-decimal percentage. */
export function formatOwnerBetaFactor(value: string, signed: boolean): string {
  const parsed = decimalParts(value);
  const scaled =
    parsed.digits === 0n
      ? 0n
      : (() => {
          const power = parsed.exponent - parsed.fractionDigits + 4;
          if (!Number.isSafeInteger(power)) {
            throw new Error("invalid factor scale");
          }
          if (power < -10_000) {
            return 0n;
          }
          if (power >= 0) {
            return parsed.digits * 10n ** BigInt(power);
          }
          return roundHalfEven(parsed.digits, 10n ** BigInt(-power));
        })();
  const integer = scaled / 100n;
  const fraction = (scaled % 100n).toString().padStart(2, "0");
  const sign = signed ? (parsed.negative && parsed.digits !== 0n ? "-" : "+") : "";
  return `${sign}${integer.toString()}.${fraction}%`;
}

type FactorPresentation = {
  readonly label: string;
  readonly signed: boolean;
};

function factorPresentation(id: string, t: RecommendationsDictionary): FactorPresentation | null {
  const trend = /^trend_([1-9]\d*)$/.exec(id);
  if (trend !== null) {
    const window = trend[1];
    if (window === undefined) {
      return null;
    }
    return { label: t.ownerBetaTrendFactorLabel(window), signed: true };
  }
  if (id === "momentum_12_1") {
    return { label: t.ownerBetaMomentumFactorLabel, signed: true };
  }
  if (id === "return_12m") {
    return { label: t.ownerBetaReturnFactorLabel, signed: true };
  }
  const volatility = /^vol_(20|60|120)$/.exec(id);
  if (volatility !== null) {
    const window = volatility[1];
    if (window === undefined) {
      return null;
    }
    return { label: t.ownerBetaVolatilityFactorLabel(window), signed: false };
  }
  return null;
}

function factorList(item: OwnerBetaItemModel, strategyId: string, t: RecommendationsDictionary) {
  const factors = Object.entries(item.factors).sort(([left], [right]) => left.localeCompare(right));
  if (factors.length === 0) {
    return (
      <span>{strategyId === "buy_and_hold" ? t.ownerBetaNoFactorEvidence : t.notReported}</span>
    );
  }
  return (
    <dl className="factor-list">
      {factors.map(([id, value]) => {
        const presentation = factorPresentation(id, t);
        if (presentation === null) {
          return null;
        }
        return (
          <div key={id}>
            <dt>{presentation.label}</dt>
            <dd>{formatOwnerBetaFactor(value, presentation.signed)}</dd>
          </div>
        );
      })}
    </dl>
  );
}

function reasonList(item: OwnerBetaItemModel, t: RecommendationsDictionary) {
  return (
    <ul className="code-list">
      {item.reason_codes.map((reason) => (
        <li key={reason}>{t.ownerBetaReasonExplanations[reason]}</li>
      ))}
    </ul>
  );
}

function timestamp(value: string | null | undefined, t: RecommendationsDictionary): string {
  return value === undefined || value === null ? t.notReported : formatTimestamp(value);
}

export type OwnerBetaReportProps = {
  readonly run: OwnerBetaRunModel;
  readonly t: RecommendationsDictionary;
};

export function OwnerBetaReport({ run, t }: OwnerBetaReportProps) {
  const parsed = ownerBetaRunSchema.safeParse(run);
  if (!parsed.success) {
    return <StatePanel kind="error" message={t.unavailableMessage} title={t.unavailableTitle} />;
  }
  const safeRun = parsed.data;
  const items = safeRun.items;
  return (
    <section aria-labelledby="owner-beta-report-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">{t.ownerBetaReportEyebrow}</p>
          <h2 id="owner-beta-report-title">{t.ownerBetaReportHeading}</h2>
          <p>{t.proposalDisclaimer}</p>
        </div>
        <div className="status-cluster">
          <StatusPill label={safeRun.status} tone="success" />
          <span>{t.asOf(formatDate(safeRun.as_of))}</span>
        </div>
      </header>

      <aside aria-label={t.warningsAriaLabel} className="warning-strip" role="status">
        <strong>{t.warningsLabel}</strong>
        <p>{t.ownerBetaInputWarning}</p>
        <p>
          {t.ownerBetaAudienceValue} · {t.ownerBetaCapabilityValue} ·{" "}
          {t.ownerBetaVendorSnapshotValue} · {t.ownerBetaStrictPitValue}
        </p>
      </aside>

      <section aria-labelledby="owner-beta-contract-title" className="report-section">
        <h3 id="owner-beta-contract-title">{t.provenanceHeading}</h3>
        <dl className="provenance-grid">
          <div>
            <dt>{t.ownerBetaStrategyIdentityLabel}</dt>
            <dd>
              {safeRun.strategy_id}@{safeRun.strategy_version}
            </dd>
          </div>
          <div>
            <dt>{t.ownerBetaRunAsOfLabel}</dt>
            <dd>{safeRun.as_of}</dd>
          </div>
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
            <dt>{t.columnCreated}</dt>
            <dd>{formatTimestamp(safeRun.created_at)}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaStartedAtLabel}</dt>
            <dd>{timestamp(safeRun.started_at, t)}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaFinishedAtLabel}</dt>
            <dd>{timestamp(safeRun.finished_at, t)}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaUpdatedAtLabel}</dt>
            <dd>{formatTimestamp(safeRun.updated_at)}</dd>
          </div>
        </dl>
      </section>

      <details className="lineage-details">
        <summary>{t.ownerBetaAuditDetails}</summary>
        <dl className="provenance-grid">
          <div>
            <dt>{t.ownerBetaRunIdLabel}</dt>
            <dd className="data-cell">{safeRun.id}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaJobIdLabel}</dt>
            <dd className="data-cell">{safeRun.job_id}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaStrategyConfigIdLabel}</dt>
            <dd className="data-cell">{safeRun.strategy_config_id}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaStrategyConfigHashLabel}</dt>
            <dd className="data-cell">{safeRun.strategy_config_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaCandidateContentHashLabel}</dt>
            <dd className="data-cell">{safeRun.candidate_content_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaArtifactManifestHashLabel}</dt>
            <dd className="data-cell">{safeRun.artifact_manifest_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaStage5ManifestHashLabel}</dt>
            <dd className="data-cell">{safeRun.stage5_manifest_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaActionManifestHashLabel}</dt>
            <dd className="data-cell">{safeRun.action_manifest_sha256}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaApprovalRegistryHashLabel}</dt>
            <dd className="data-cell">{safeRun.approval_registry_sha256}</dd>
          </div>
          <div>
            <dt>{t.factorSnapshotLabel}</dt>
            <dd className="data-cell">{auditValue(safeRun.factor_snapshot_sha256, t)}</dd>
          </div>
          <div>
            <dt>{t.ownerBetaTargetSnapshotHashLabel}</dt>
            <dd className="data-cell">{auditValue(safeRun.target_snapshot_sha256, t)}</dd>
          </div>
        </dl>
      </details>

      <section aria-labelledby="owner-beta-results-title" className="report-section">
        <div className="section-heading">
          <div>
            <h3 id="owner-beta-results-title">{t.ownerBetaItemsHeading}</h3>
          </div>
          {safeRun.cash_weight === undefined || safeRun.cash_weight === null ? null : (
            <p>{t.cashAllocation(formatPercentage(safeRun.cash_weight))}</p>
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
                  <th scope="col">{t.ownerBetaFactorEvidence}</th>
                  <th scope="col">{t.columnSelectionReasons}</th>
                </tr>
              </thead>
              <tbody>
                {items.map((item) => (
                  <tr key={item.instrument_id}>
                    <td>{item.rank ?? "—"}</td>
                    <th scope="row">
                      <div>
                        <strong>{item.instrument.name ?? t.ownerBetaInstrumentNameMissing}</strong>
                        <div className="data-cell">{item.instrument_id}</div>
                        <div>{item.instrument.asset_class ?? t.ownerBetaAssetClassMissing}</div>
                        <div>{t.ownerBetaTrackingIndexMissing}</div>
                      </div>
                    </th>
                    <td>
                      {item.target_weight === undefined || item.target_weight === null
                        ? "—"
                        : formatPercentage(item.target_weight)}
                    </td>
                    <td>{item.excluded ? t.exclusionsHeading : t.selectedCandidatesHeading}</td>
                    <td>{factorList(item, safeRun.strategy_id, t)}</td>
                    <td>{reasonList(item, t)}</td>
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
