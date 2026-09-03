import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type {
  OwnerBetaEquitySignalsFilters,
  OwnerBetaEquitySignalsLatestModel,
  OwnerBetaEquitySignalsProvenanceModel,
  OwnerBetaEquitySignalsScreenModel,
} from "@/lib/products/equity-signals-contracts";
import { StockBetaColumnVisibilityControl } from "./dashboard/column-visibility-control";
import styles from "./dashboard/dashboard.module.css";
import { StockBetaInstrumentSearch } from "./dashboard/instrument-search";
import { StockBetaSelectionProvider } from "./dashboard/selection-provider";
import { StockBetaSnapshotStrip } from "./dashboard/snapshot-strip";
import { StockBetaDashboard } from "./dashboard/stock-beta-dashboard";
import type { StockBetaDashboardBaseViewModel } from "./dashboard/types";
import { FilterWidget } from "./dashboard/widgets/filter-widget";
import { StockBetaTerminalPage } from "./terminal";

export { stockBetaFormatNumber, stockBetaFormatPercent } from "./dashboard/formatters";
export { stockBetaConditionLabel, stockBetaConditionTone } from "./dashboard/labels";

// This compatibility export is used by the fail-closed page state. The successful workspace
// uses the same compact policy treatment through the registered policy widget.
export function StockBetaPolicyNotice({ t }: { readonly t: StockBetaDictionary }) {
  return (
    <aside aria-label={t.policyAriaLabel} className={styles["policyBoundary"]} role="note">
      <div className={styles["policySummary"]}>
        <span className={styles["policyEyebrow"]}>{t.warningLabel}</span>
        <strong>{t.policyBoundarySummary}</strong>
      </div>
      <details className={styles["policyDetails"]}>
        <summary>{t.policyBoundaryDetailsLabel}</summary>
        <div className={styles["policyDetailsContent"]}>
          <p>{t.fixedListPolicy}</p>
          <p>{t.originalPricePolicy}</p>
          <p>{t.activityPolicy}</p>
          <p>{t.conditionPolicy}</p>
        </div>
      </details>
    </aside>
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

type StockBetaData = OwnerBetaEquitySignalsLatestModel | OwnerBetaEquitySignalsScreenModel;

function initialSelectionId(data: StockBetaData): string | undefined {
  if ("top5" in data) {
    const topRow = data.top5[0];
    if (
      topRow !== undefined &&
      data.rows.some((row) => row.instrument_id === topRow.instrument_id)
    ) {
      return topRow.instrument_id;
    }
  }
  return data.rows[0]?.instrument_id;
}

export function StockBetaWorkspace({
  data,
  filters,
  locale,
  t,
}: {
  readonly data: StockBetaData;
  readonly filters: OwnerBetaEquitySignalsFilters;
  readonly locale: Locale;
  readonly t: StockBetaDictionary;
}) {
  const { detailTitle: _detailTitle, ...dashboardCopy } = t;
  const filtered =
    filters.conditions.length > 0 ||
    Object.keys(filters.ranges).length > 0 ||
    filters.trendUp !== undefined;
  const viewModel: StockBetaDashboardBaseViewModel = {
    copy: dashboardCopy,
    data,
    filtered,
    filters,
    locale,
  };
  const selectedInstrumentId = initialSelectionId(data);

  return (
    <StockBetaSelectionProvider
      {...(selectedInstrumentId === undefined
        ? {}
        : { initialSelectedInstrumentId: selectedInstrumentId })}
      rows={data.rows}
    >
      <StockBetaTerminalPage
        asOf={
          <span className={styles["terminalAsOf"]}>
            {t.asOfLabel} {data.provenance.as_of}
          </span>
        }
        context={<span>{t.terminalContextLabel}</span>}
        search={<StockBetaInstrumentSearch copy={dashboardCopy} rows={data.rows} />}
        snapshot={<StockBetaSnapshotStrip copy={dashboardCopy} data={data} filtered={filtered} />}
        title={t.pageTitle}
        titleTools={
          <div className={styles["titleTools"]}>
            <FilterWidget viewModel={viewModel} />
            <StockBetaColumnVisibilityControl copy={dashboardCopy} />
          </div>
        }
      >
        <StockBetaDashboard
          copy={dashboardCopy}
          data={data}
          filters={filters}
          locale={locale}
          selectionProvided={true}
        />
      </StockBetaTerminalPage>
    </StockBetaSelectionProvider>
  );
}
