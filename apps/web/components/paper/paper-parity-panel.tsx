import { type PaperParityModel, parityReason } from "@/lib/products/paper-contracts";

export type PaperParityPanelProps = {
  readonly parity: PaperParityModel;
};

const STATUS_LABEL: Record<PaperParityModel["status"], string> = {
  DIVERGENT: "Divergent",
  MATCH: "Match",
  NOT_COMPARABLE: "Not comparable",
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
export function PaperParityPanel({ parity }: PaperParityPanelProps) {
  const mismatched = parity.lineage.fields.filter((field) => field.backtest !== field.paper);

  return (
    <section aria-labelledby="paper-parity-title" className="report-section">
      <h3 id="paper-parity-title">Backtest parity</h3>
      {parity.warrants_alert ? (
        <div
          aria-label={`Paper parity ${STATUS_LABEL[parity.status]}`}
          className="state-panel"
          data-kind="warning"
          role="alert"
        >
          <strong>{STATUS_LABEL[parity.status]}</strong>
          <p>{parityReason(parity)}</p>
        </div>
      ) : (
        <p className="form-result" role="status">
          <strong>{STATUS_LABEL[parity.status]}</strong> — {parityReason(parity)}
        </p>
      )}

      <dl className="definition-grid">
        <dt>Session</dt>
        <dd>{parity.as_of}</dd>
      </dl>

      {mismatched.length > 0 ? (
        <table>
          <caption>Lineage differences</caption>
          <thead>
            <tr>
              <th scope="col">Field</th>
              <th scope="col">Backtest</th>
              <th scope="col">Paper</th>
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
          <caption>Signal divergences</caption>
          <thead>
            <tr>
              <th scope="col">Instrument</th>
              <th scope="col">Backtest weight</th>
              <th scope="col">Paper weight</th>
            </tr>
          </thead>
          <tbody>
            {parity.divergences.map((divergence) => (
              <tr key={divergence.instrument_id}>
                <th scope="row">{divergence.instrument_id}</th>
                <td>{divergence.backtest_weight ?? "Not targeted"}</td>
                <td>{divergence.paper_weight ?? "Not targeted"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}

      <p className="supporting-copy">
        <strong>Fill model difference</strong> {parity.fill_model_difference}
      </p>
    </section>
  );
}
