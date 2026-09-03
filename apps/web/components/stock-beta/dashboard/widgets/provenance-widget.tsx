import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import type { StockBetaDashboardWidgetViewModel } from "../types";

export function ProvenanceWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, signals } = viewModel;
  if (signals === null) return null;
  const fields = [
    [t.snapshotIdLabel, signals.snapshot.snapshot_id],
    [t.asOfLabel, signals.snapshot.as_of],
    [t.universeHashLabel, signals.snapshot.universe_sha256],
    [t.snapshotRowsLabel, signals.snapshot.row_count.toLocaleString("en-US")],
    [t.publishedAtLabel, signals.snapshot.published_at],
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
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        </div>
      </details>
    </WidgetFrame>
  );
}
