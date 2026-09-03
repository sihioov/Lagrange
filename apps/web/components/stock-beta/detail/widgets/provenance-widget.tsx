import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import type { StockBetaDetailWidgetViewModel } from "../types";

function provenanceBoolean(value: boolean, yes: string, no: string): string {
  return value ? yes : no;
}

export function ProvenanceWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t, detail } = viewModel;
  const provenance = detail.provenance;
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
    [t.vendorSnapshotLabel, provenanceBoolean(provenance.vendor_snapshot, t.yes, t.no)],
    [t.strictPitLabel, provenanceBoolean(provenance.strict_pit, t.yes, t.no)],
    [t.originalPriceLabel, provenanceBoolean(provenance.original_price, t.yes, t.no)],
    [t.warningLabel, provenance.warning],
    [t.activityProxyLabel, provenance.activity_proxy],
  ] as const;

  return (
    <WidgetFrame description={t.provenanceDescription} title={t.provenanceHeading}>
      <details className={styles["provenanceDetails"]} data-testid="stock-beta-provenance">
        <summary>{t.provenanceDisclosureLabel}</summary>
        <div className={styles["provenanceDetailsContent"]}>
          <dl className={styles["provenanceGrid"]}>
            {fields.map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd className={styles["provenanceValue"]}>{value}</dd>
              </div>
            ))}
          </dl>
        </div>
      </details>
    </WidgetFrame>
  );
}
