import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../detail.module.css";
import type { StockBetaDetailWidgetViewModel } from "../types";

export function FactorEvidenceWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDetailWidgetViewModel;
}) {
  const { copy: t, detail } = viewModel;
  const factorKeys = new Map<string, number>();

  const reasonKeys = new Map<string, number>();

  return (
    <WidgetFrame description={t.factorEvidenceDescription} title={t.factorEvidenceHeading}>
      <div className={styles["evidenceContent"]} data-testid="stock-beta-factor-evidence">
        <div className={styles["factorTableWrap"]}>
          <table className={styles["factorTable"]} data-testid="stock-beta-factor-table">
            <caption>{t.factorEvidenceCaption}</caption>
            <thead>
              <tr>
                <th scope="col">{t.factorLabel}</th>
                <th scope="col">{t.interpretationLabel}</th>
                <th scope="col">{t.valueLabel}</th>
              </tr>
            </thead>
            <tbody>
              {detail.factor_explanations.map((factor) => {
                const rawValue = String(factor.value);
                const baseKey = `${factor.factor}:${rawValue}:${factor.interpretation}`;
                const occurrence = factorKeys.get(baseKey) ?? 0;
                factorKeys.set(baseKey, occurrence + 1);
                return (
                  <tr data-raw-value={rawValue} key={`${baseKey}-${occurrence}`}>
                    <th scope="row">{factor.factor}</th>
                    <td>{factor.interpretation}</td>
                    <td>
                      <data value={rawValue}>{rawValue}</data>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <section
          aria-labelledby="stock-beta-condition-reasons-heading"
          className={styles["evidenceReasons"]}
        >
          <div className={styles["evidenceSectionHeading"]}>
            <h3 id="stock-beta-condition-reasons-heading">{t.conditionReasonsHeading}</h3>
            <p>{t.conditionReasonsDescription}</p>
          </div>
          {detail.condition_reasons.length === 0 ? (
            <p className={styles["emptyCopy"]}>{t.noReasons}</p>
          ) : (
            <ol className={styles["reasonList"]} data-testid="stock-beta-condition-reasons">
              {detail.condition_reasons.map((reason) => {
                const occurrence = reasonKeys.get(reason) ?? 0;
                reasonKeys.set(reason, occurrence + 1);
                return <li key={`${reason}-${occurrence}`}>{reason}</li>;
              })}
            </ol>
          )}
        </section>
      </div>
    </WidgetFrame>
  );
}
