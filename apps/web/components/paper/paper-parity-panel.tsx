import type { PaperDictionary } from "@/lib/i18n/dictionaries/paper";
import { type PaperParityModel, parityReason } from "@/lib/products/paper-contracts";

export type PaperParityPanelProps = {
  readonly parity: PaperParityModel;
  readonly t: PaperDictionary;
};

/**
 * The backtest-vs-Paper parity report.
 *
 * A divergence and an incomparable lineage are both rendered as alerts, not
 * quiet badges: design §15.3 grades a Paper divergence WARNING, and the
 * server already told us via `warrants_alert` so the UI never re-derives it.
 * The fill-model difference is shown on EVERY status including a match —
 * a reader must never assume the two executions are interchangeable.
 */
export function PaperParityPanel({ parity, t }: PaperParityPanelProps) {
  const statusLabel: Record<PaperParityModel["status"], string> = {
    DIVERGENT: t.statusDivergent,
    MATCH: t.statusMatch,
    NOT_COMPARABLE: t.statusNotComparable,
  };
  const mismatched = parity.lineage.fields.filter((field) => field.backtest !== field.paper);

  return (
    <section aria-labelledby="paper-parity-title" className="report-section">
      <h3 id="paper-parity-title">{t.parityTitle}</h3>
      {parity.warrants_alert ? (
        <div
          aria-label={t.parityAriaLabel(statusLabel[parity.status])}
          className="state-panel"
          data-kind="warning"
          role="alert"
        >
          <strong>{statusLabel[parity.status]}</strong>
          <p>{parityReason(parity)}</p>
        </div>
      ) : (
        <p className="form-result" role="status">
          <strong>{statusLabel[parity.status]}</strong> — {parityReason(parity)}
        </p>
      )}

      <dl className="definition-grid">
        <dt>{t.sessionLabel}</dt>
        <dd>{parity.as_of}</dd>
      </dl>

      {mismatched.length > 0 ? (
        <table>
          <caption>{t.lineageDifferencesCaption}</caption>
          <thead>
            <tr>
              <th scope="col">{t.columnField}</th>
              <th scope="col">{t.columnBacktest}</th>
              <th scope="col">{t.columnPaper}</th>
            </tr>
          </thead>
          <tbody>
            {mismatched.map((field) => (
              <tr key={field.field}>
                <th scope="row">{field.field}</th>
                <td>{field.backtest}</td>
                <td>{field.paper}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}

      {parity.divergences.length > 0 ? (
        <table>
          <caption>{t.signalDivergencesCaption}</caption>
          <thead>
            <tr>
              <th scope="col">{t.columnInstrument}</th>
              <th scope="col">{t.columnBacktestWeight}</th>
              <th scope="col">{t.columnPaperWeight}</th>
            </tr>
          </thead>
          <tbody>
            {parity.divergences.map((divergence) => (
              <tr key={divergence.instrument_id}>
                <th scope="row">{divergence.instrument_id}</th>
                <td>{divergence.backtest_weight ?? t.notTargeted}</td>
                <td>{divergence.paper_weight ?? t.notTargeted}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}

      <p className="supporting-copy">
        <strong>{t.fillModelDifferenceLabel}</strong> {parity.fill_model_difference}
      </p>
    </section>
  );
}
