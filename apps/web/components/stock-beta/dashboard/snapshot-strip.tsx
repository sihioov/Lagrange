import type {
  OwnerBetaEquitySignalsLatestModel,
  OwnerBetaEquitySignalsScreenModel,
} from "@/lib/products/equity-signals-contracts";
import styles from "./dashboard.module.css";
import type { StockBetaDashboardCopy } from "./types";

type SnapshotData = OwnerBetaEquitySignalsLatestModel | OwnerBetaEquitySignalsScreenModel;

const CONDITIONS = ["BULLISH", "NEUTRAL", "BEARISH"] as const;

export function StockBetaSnapshotStrip({
  copy: t,
  data,
  filtered,
}: {
  readonly copy: StockBetaDashboardCopy;
  readonly data: SnapshotData;
  readonly filtered: boolean;
}) {
  const counts = {
    BEARISH: 0,
    BULLISH: 0,
    NEUTRAL: 0,
  };
  for (const row of data.rows) counts[row.condition] += 1;

  const scopeLabel = filtered ? t.currentResultsLabel : t.configuredResultsLabel;
  return (
    <div className={styles["snapshotStrip"]} data-testid="stock-beta-snapshot-strip">
      <dl className={styles["snapshotCells"]} data-testid="stock-beta-condition-distribution">
        <div data-testid="stock-beta-snapshot-as-of">
          <dt>{t.asOfLabel}</dt>
          <dd>
            <data value={data.provenance.as_of}>{data.provenance.as_of}</data>
          </dd>
        </div>
        <div data-testid="stock-beta-snapshot-universe">
          <dt>{t.universeLabel}</dt>
          <dd>
            <data value={String(data.rows.length)}>{data.rows.length}</data>
            <small>{scopeLabel}</small>
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
              <small>{t.registrationStatusLabel}</small>
              <strong>{data.provenance.registration_status}</strong>
            </span>
            <span>
              <small>{t.publicationStatusLabel}</small>
              <strong>{data.provenance.publication_status}</strong>
            </span>
            <span>
              <small>{t.materializationStatusLabel}</small>
              <strong>{data.provenance.materialization_status}</strong>
            </span>
          </dd>
        </div>
      </dl>
    </div>
  );
}
