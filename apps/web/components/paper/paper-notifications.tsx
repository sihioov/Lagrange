import type { NotificationModel } from "@/lib/products/paper-contracts";

export type PaperNotificationsProps = {
  readonly notifications: readonly NotificationModel[];
};

/**
 * Session completion, block, and divergence notices with their delivery
 * outcome.
 *
 * The outcome is rendered next to every notice on purpose: a channel that
 * failed is recorded server-side (FR-RPT-002), and hiding that here would
 * put the reader back where an outage looks like silence.
 *
 * This is the actor's WHOLE feed, not a per-account slice: `notifications`
 * carries no account or resource column, so filtering to one account could
 * only be done by matching titles — a scoping this cannot honestly promise.
 * Backtest and recommendation notices will appear here too.
 */
export function PaperNotifications({ notifications }: PaperNotificationsProps) {
  if (notifications.length === 0) {
    return (
      <section aria-labelledby="paper-notifications-title" className="report-section">
        <h3 id="paper-notifications-title">Session notifications</h3>
        <p className="supporting-copy">
          No session notices yet. Completion, block, and divergence notices appear here once a
          session settles.
        </p>
      </section>
    );
  }

  return (
    <section aria-labelledby="paper-notifications-title" className="report-section">
      <h3 id="paper-notifications-title">Session notifications</h3>
      <table>
        <caption>Notices and delivery outcome</caption>
        <thead>
          <tr>
            <th scope="col">Notice</th>
            <th scope="col">Kind</th>
            <th scope="col">Raised</th>
            <th scope="col">Delivery</th>
          </tr>
        </thead>
        <tbody>
          {notifications.map((notification) => (
            <tr key={notification.id}>
              <th scope="row">
                {notification.title}
                <span className="supporting-copy"> {notification.body}</span>
              </th>
              <td>{notification.kind}</td>
              <td>{notification.created_at}</td>
              <td>
                <ul>
                  {notification.deliveries.map((delivery) => (
                    <li key={delivery.channel}>
                      {delivery.channel}: {delivery.status}
                      {delivery.status === "FAILED" ? (
                        <span role="alert">
                          {" "}
                          Delivery failed — {delivery.error_detail ?? "no detail recorded"}
                        </span>
                      ) : null}
                    </li>
                  ))}
                </ul>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
