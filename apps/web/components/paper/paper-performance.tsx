import type { PaperDictionary } from "@/lib/i18n/dictionaries/paper";
import type { PaperPerformanceModel } from "@/lib/products/paper-contracts";

export type PaperPerformanceProps = {
  readonly performance: PaperPerformanceModel;
  readonly t: PaperDictionary;
};

/**
 * Daily performance derived from the account's own ledger.
 *
 * The server's disclaimer is rendered verbatim and unconditionally: Paper
 * results are simulated, and the plan forbids presenting them as
 * guaranteed returns.
 */
export function PaperPerformance({ performance, t }: PaperPerformanceProps) {
  return (
    <section aria-labelledby="paper-performance-title" className="report-section">
      <h3 id="paper-performance-title">{t.performanceTitle}</h3>
      <p className="supporting-copy">{performance.disclaimer}</p>
      {performance.points.length === 0 ? (
        <p className="supporting-copy">{t.noSessionsValuedMessage}</p>
      ) : (
        <table>
          <caption>{t.ledgerEquityCaption}</caption>
          <thead>
            <tr>
              <th scope="col">{t.columnDate}</th>
              <th scope="col">{t.columnEquity}</th>
              <th scope="col">{t.columnCash}</th>
              <th scope="col">{t.columnPositions}</th>
              <th scope="col">{t.columnDailyReturn}</th>
            </tr>
          </thead>
          <tbody>
            {performance.points.map((point) => (
              <tr key={point.trading_date}>
                <th scope="row">{point.trading_date}</th>
                <td>{point.equity}</td>
                <td>{point.cash}</td>
                <td>{point.positions_value}</td>
                <td>{point.return_pct ?? t.firstSession}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
