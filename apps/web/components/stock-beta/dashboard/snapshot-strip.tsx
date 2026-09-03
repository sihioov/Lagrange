import type { OwnerEquityV2LatestSignalsModel } from "@/lib/products/equity-signals-contracts";
import styles from "./dashboard.module.css";
import type { StockBetaDashboardCopy } from "./types";

const CONDITIONS = ["BULLISH", "NEUTRAL", "BEARISH"] as const;

export function StockBetaSnapshotStrip({
  copy: t,
  data,
}: {
  readonly copy: StockBetaDashboardCopy;
  readonly data: OwnerEquityV2LatestSignalsModel;
}) {
  const counts = { BEARISH: 0, BULLISH: 0, NEUTRAL: 0 };
  for (const row of data.rows) counts[row.condition] += 1;

  return (
    <div className={styles["snapshotStrip"]} data-testid="stock-beta-snapshot-strip">
      <dl className={styles["snapshotCells"]} data-testid="stock-beta-condition-distribution">
        <div data-testid="stock-beta-snapshot-as-of">
          <dt>{t.asOfLabel}</dt>
          <dd>
            <data value={data.snapshot.as_of}>{data.snapshot.as_of}</data>
          </dd>
        </div>
        <div data-testid="stock-beta-snapshot-universe">
          <dt>{t.universeLabel}</dt>
          <dd>
            <data value={String(data.snapshot.row_count)}>{data.snapshot.row_count}</data>
          </dd>
        </div>
        {CONDITIONS.map((condition) => (
          <div data-testid={`stock-beta-snapshot-${condition.toLowerCase()}`} key={condition}>
            <dt>{condition}</dt>
            <dd>
              <data value={String(counts[condition])}>{counts[condition]}</data>
            </dd>
          </div>
        ))}
        <div className={styles["snapshotStatusCell"]} data-testid="stock-beta-snapshot-statuses">
          <dt>{t.currentSnapshotLabel}</dt>
          <dd className={styles["snapshotStatuses"]}>
            <span>
              <small>{t.snapshotIdLabel}</small>
              <strong>{data.snapshot.snapshot_id}</strong>
            </span>
            <span>
              <small>{t.publishedAtLabel}</small>
              <strong>{data.snapshot.published_at}</strong>
            </span>
          </dd>
        </div>
      </dl>
    </div>
  );
}
