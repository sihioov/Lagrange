import { StatusPill } from "@/components/states/status-pill";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import type { StockBetaDashboardWidgetViewModel } from "../types";

export function SnapshotStatusWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, data, filtered } = viewModel;
  const isLatest = "top5" in data;
  const scopeLabel = filtered ? t.currentResultsLabel : t.configuredResultsLabel;

  return (
    <WidgetFrame
      description={t.snapshotDescription}
      status={<StatusPill label={t.approvedSnapshotLabel} tone="success" />}
      title={t.snapshotHeading}
    >
      <div className={styles["snapshotStatus"]} data-testid="stock-beta-snapshot-status">
        <dl className={styles["snapshotGrid"]}>
          <div>
            <dt>{t.asOfLabel}</dt>
            <dd>{data.provenance.as_of}</dd>
          </div>
          <div>
            <dt>{t.resultCountLabel}</dt>
            <dd>
              {data.rows.length} <span className={styles["scopeLabel"]}>{scopeLabel}</span>
            </dd>
          </div>
          <div>
            <dt>{t.snapshotStatusLabel}</dt>
            <dd>{isLatest ? t.latestSnapshotLabel : t.filteredSnapshotLabel}</dd>
          </div>
          <div>
            <dt>{t.batchLabel}</dt>
            <dd>{data.provenance.batch_id}</dd>
          </div>
          <div>
            <dt>{t.vendorSnapshotLabel}</dt>
            <dd>{data.provenance.vendor_snapshot ? t.yes : t.no}</dd>
          </div>
          <div>
            <dt>{t.strictPitLabel}</dt>
            <dd>{data.provenance.strict_pit ? t.yes : t.no}</dd>
          </div>
        </dl>
        <p className={styles["snapshotNote"]}>
          {filtered ? t.currentResultsDescription : t.configuredResultsDescription}
        </p>
      </div>
    </WidgetFrame>
  );
}
