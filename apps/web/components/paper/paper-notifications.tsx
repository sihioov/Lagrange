import type { PaperDictionary } from "@/lib/i18n/dictionaries/paper";
import type { NotificationModel } from "@/lib/products/paper-contracts";

export type PaperNotificationsProps = {
  readonly notifications: readonly NotificationModel[];
  readonly t: PaperDictionary;
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
export function PaperNotifications({ notifications, t }: PaperNotificationsProps) {
  if (notifications.length === 0) {
    return (
      <section aria-labelledby="paper-notifications-title" className="report-section">
        <h3 id="paper-notifications-title">{t.notificationsTitle}</h3>
        <p className="supporting-copy">{t.noNoticesMessage}</p>
      </section>
    );
  }

  return (
    <section aria-labelledby="paper-notifications-title" className="report-section">
      <h3 id="paper-notifications-title">{t.notificationsTitle}</h3>
      <table>
        <caption>{t.noticesCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnNotice}</th>
            <th scope="col">{t.columnKind}</th>
            <th scope="col">{t.columnRaised}</th>
            <th scope="col">{t.columnDelivery}</th>
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
                          {t.deliveryFailedMessage(delivery.error_detail ?? t.noDetailRecorded)}
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
