import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import type { StockBetaDashboardWidgetViewModel, StockBetaProvenanceCopy } from "../types";

function provenanceBoolean(value: boolean, t: StockBetaProvenanceCopy): string {
  return value ? t.yes : t.no;
}

function provenanceFields(
  provenance: StockBetaDashboardWidgetViewModel["data"]["provenance"],
  t: StockBetaProvenanceCopy,
) {
  return [
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
}

function ProvenanceFields({
  provenance,
  t,
}: {
  readonly provenance: StockBetaDashboardWidgetViewModel["data"]["provenance"];
  readonly t: StockBetaProvenanceCopy;
}) {
  return (
    <dl className={styles["provenanceGrid"]}>
      {provenanceFields(provenance, t).map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function StockBetaProvenance({
  provenance,
  t,
}: {
  readonly provenance: StockBetaDashboardWidgetViewModel["data"]["provenance"];
  readonly t: StockBetaProvenanceCopy;
}) {
  return (
    <footer className={styles["provenanceFooter"]} data-testid="stock-beta-provenance">
      <div>
        <h2>{t.provenanceHeading}</h2>
        <p>{t.provenanceDescription}</p>
      </div>
      <ProvenanceFields provenance={provenance} t={t} />
    </footer>
  );
}

export function ProvenanceWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, data } = viewModel;
  return (
    <WidgetFrame description={t.provenanceDescription} title={t.provenanceHeading}>
      <details className={styles["provenanceDetails"]}>
        <summary>{t.provenanceDisclosureLabel}</summary>
        <div className={styles["provenanceDetailsContent"]}>
          <ProvenanceFields provenance={data.provenance} t={t} />
        </div>
      </details>
    </WidgetFrame>
  );
}
