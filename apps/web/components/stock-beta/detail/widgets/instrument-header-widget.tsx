import { StatusPill } from "@/components/states/status-pill";
import { stockBetaConditionLabel, stockBetaConditionTone } from "../../dashboard/labels";
import { formatStockBetaNumber } from "../../shared/formatters";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function InstrumentHeaderWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t, detail, locale } = viewModel;
  const { signal } = detail;
  const score = formatStockBetaNumber(signal.score, locale);

  return (
    <WidgetFrame
      description={t.instrumentHeaderDescription}
      status={
        <StatusPill
          label={`${signal.condition} · ${stockBetaConditionLabel(signal.condition, t)}`}
          tone={stockBetaConditionTone(signal.condition)}
        />
      }
      title={signal.instrument_name}
    >
      <div className={styles["instrumentHeader"]} data-testid="stock-beta-instrument-header">
        <p className={styles["eyebrow"]}>{t.detailEyebrow}</p>
        <p className={styles["instrumentId"]}>{signal.instrument_id}</p>
        <dl className={styles["instrumentFacts"]}>
          <div>
            <dt>{t.rankLabel}</dt>
            <dd>{signal.rank}</dd>
          </div>
          <div data-raw-value={String(score.rawValue)}>
            <dt>{t.scoreLabel}</dt>
            <dd>
              <data value={String(score.rawValue)}>{score.text}</data>
            </dd>
            <small className={styles["metricRawValue"]}>
              {t.rawValueLabel}: {String(score.rawValue)}
            </small>
          </div>
          <div>
            <dt>{t.conditionLabel}</dt>
            <dd>{signal.condition}</dd>
          </div>
          <div>
            <dt>{t.asOfLabel}</dt>
            <dd>{detail.provenance.as_of}</dd>
          </div>
        </dl>
      </div>
    </WidgetFrame>
  );
}
