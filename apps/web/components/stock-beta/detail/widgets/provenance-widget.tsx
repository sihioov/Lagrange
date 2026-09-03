import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function ProvenanceWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t, detail } = viewModel;
  const fields = [
    [t.snapshotIdLabel, detail.snapshot.snapshot_id],
    [t.asOfLabel, detail.snapshot.as_of],
    [t.universeHashLabel, detail.snapshot.universe_sha256],
    [t.snapshotRowsLabel, detail.snapshot.row_count.toLocaleString("en-US")],
    [t.publishedAtLabel, detail.snapshot.published_at],
  ] as const;
  return (
    <WidgetFrame description={t.snapshotDescription} title={t.snapshotHeading}>
      <dl className={styles["provenanceGrid"]} data-testid="stock-beta-provenance">
        {fields.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd className={styles["provenanceValue"]}>{value}</dd>
          </div>
        ))}
      </dl>
    </WidgetFrame>
  );
}
