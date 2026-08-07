import type { PaperPerformanceModel } from "@/lib/products/paper-contracts";

export type PaperPerformanceProps = {
  readonly performance: PaperPerformanceModel;
};

/**
 * Daily performance derived from the account's own ledger.
 *
 * The server's disclaimer is rendered verbatim and unconditionally: Paper
 * results are simulated, and the plan forbids presenting them as
 * guaranteed returns.
 */
export function PaperPerformance({ performance }: PaperPerformanceProps) {
  return (
    <section aria-labelledby="paper-performance-title" className="report-section">
      <h3 id="paper-performance-title">Daily performance</h3>
      <p className="supporting-copy">{performance.disclaimer}</p>
      {performance.points.length === 0 ? (
        <p className="supporting-copy">
          No sessions have been valued yet. Performance appears after the first close valuation.
        </p>
      ) : (
        <table>
          <caption>Ledger-derived daily equity</caption>
          <thead>
            <tr>
              <th scope="col">Date</th>
              <th scope="col">Equity</th>
              <th scope="col">Cash</th>
              <th scope="col">Positions</th>
              <th scope="col">Daily return</th>
            </tr>
          </thead>
          <tbody>
            {performance.points.map((point) => (
              <tr key={point.trading_date}>
                <th scope="row">{point.trading_date}</th>
                <td>{point.equity}</td>
                <td>{point.cash}</td>
                <td>{point.positions_value}</td>
                <td>{point.return_pct ?? "First session"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
