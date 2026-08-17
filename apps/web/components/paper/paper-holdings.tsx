import type { PaperDictionary } from "@/lib/i18n/dictionaries/paper";
import type {
  PaperAccountModel,
  PaperOrderModel,
  PaperPositionModel,
} from "@/lib/products/paper-contracts";

export type PaperHoldingsProps = {
  readonly account: PaperAccountModel;
  readonly orders: readonly PaperOrderModel[];
  readonly positions: readonly PaperPositionModel[];
  readonly t: PaperDictionary;
};

/** The account's identity, current positions, and order history. */
export function PaperHoldings({ account, orders, positions, t }: PaperHoldingsProps) {
  return (
    <section aria-labelledby="paper-holdings-title" className="report-section">
      <h3 id="paper-holdings-title">{t.holdingsTitle}</h3>
      <dl className="definition-grid">
        <dt>{t.accountLabel}</dt>
        <dd>{account.name}</dd>
        <dt>{t.statusLabel}</dt>
        <dd>{account.status}</dd>
        <dt>{t.openingCashLabel}</dt>
        <dd>{account.initial_cash ?? t.notReported}</dd>
        <dt>{t.costProfileLabel}</dt>
        <dd>
          {account.cost_profile_id}@{account.cost_profile_version}
        </dd>
      </dl>

      <table>
        <caption>{t.currentPositionsCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnInstrument}</th>
            <th scope="col">{t.columnQuantity}</th>
            <th scope="col">{t.columnAveragePrice}</th>
          </tr>
        </thead>
        <tbody>
          {positions.map((position) => (
            <tr key={position.instrument_id}>
              <th scope="row">{position.instrument_id}</th>
              <td>{position.quantity}</td>
              <td>{position.avg_price ?? t.notReported}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <table>
        <caption>{t.paperOrdersCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnOrder}</th>
            <th scope="col">{t.columnInstrument}</th>
            <th scope="col">{t.columnSide}</th>
            <th scope="col">{t.columnQuantity}</th>
            <th scope="col">{t.columnPrice}</th>
            <th scope="col">{t.statusLabel}</th>
          </tr>
        </thead>
        <tbody>
          {orders.map((order) => (
            <tr key={order.id}>
              <th scope="row">{order.order_ref}</th>
              <td>{order.instrument_id}</td>
              <td>{order.side}</td>
              <td>{order.quantity}</td>
              <td>{order.price ?? t.notReported}</td>
              <td>{order.status}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
