import type {
  PaperAccountModel,
  PaperOrderModel,
  PaperPositionModel,
} from "@/lib/products/paper-contracts";

export type PaperHoldingsProps = {
  readonly account: PaperAccountModel;
  readonly orders: readonly PaperOrderModel[];
  readonly positions: readonly PaperPositionModel[];
};

/** The account's identity, current positions, and order history. */
export function PaperHoldings({ account, orders, positions }: PaperHoldingsProps) {
  return (
    <section aria-labelledby="paper-holdings-title" className="report-section">
      <h3 id="paper-holdings-title">Account and holdings</h3>
      <dl className="definition-grid">
        <dt>Account</dt>
        <dd>{account.name}</dd>
        <dt>Status</dt>
        <dd>{account.status}</dd>
        <dt>Opening cash</dt>
        <dd>{account.initial_cash ?? "Not reported"}</dd>
        <dt>Cost profile</dt>
        <dd>
          {account.cost_profile_id}@{account.cost_profile_version}
        </dd>
      </dl>

      <table>
        <caption>Current positions</caption>
        <thead>
          <tr>
            <th scope="col">Instrument</th>
            <th scope="col">Quantity</th>
            <th scope="col">Average price</th>
          </tr>
        </thead>
        <tbody>
          {positions.map((position) => (
            <tr key={position.instrument_id}>
              <th scope="row">{position.instrument_id}</th>
              <td>{position.quantity}</td>
              <td>{position.avg_price ?? "Not reported"}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <table>
        <caption>Paper orders and fills</caption>
        <thead>
          <tr>
            <th scope="col">Order</th>
            <th scope="col">Instrument</th>
            <th scope="col">Side</th>
            <th scope="col">Quantity</th>
            <th scope="col">Price</th>
            <th scope="col">Status</th>
          </tr>
        </thead>
        <tbody>
          {orders.map((order) => (
            <tr key={order.id}>
              <th scope="row">{order.order_ref}</th>
              <td>{order.instrument_id}</td>
              <td>{order.side}</td>
              <td>{order.quantity}</td>
              <td>{order.price ?? "Not reported"}</td>
              <td>{order.status}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
