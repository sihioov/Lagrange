import { isLiveProfile, type LiveConnectionModel } from "@/lib/products/live-contracts";

export type LiveConnectionsProps = {
  readonly connections: readonly LiveConnectionModel[];
};

/**
 * Configured broker connections.
 *
 * Two things are deliberate here. A `live` profile is labelled in words, not
 * by colour alone — an operator scanning this table must be able to tell a
 * simulated connection from one that places real orders even in a screenshot,
 * a print, or with a colour-vision difference.
 *
 * And what is shown of a credential is its LOCATION. The server never sends
 * the value (the DTO has no field for one), so the worst this table can
 * disclose is which environment variable to go and read — which is what an
 * operator needs in order to fix a misconfiguration.
 */
export function LiveConnections({ connections }: LiveConnectionsProps) {
  if (connections.length === 0) {
    return (
      <section aria-labelledby="live-connections-title" className="report-section">
        <h3 id="live-connections-title">Broker connections</h3>
        <p className="supporting-copy">
          No broker connection is configured. Live trading cannot start until one exists.
        </p>
      </section>
    );
  }

  return (
    <section aria-labelledby="live-connections-title" className="report-section">
      <h3 id="live-connections-title">Broker connections</h3>
      <table>
        <caption>Configured broker connections</caption>
        <thead>
          <tr>
            <th scope="col">Connection</th>
            <th scope="col">Profile</th>
            <th scope="col">Account</th>
            <th scope="col">Credential locations</th>
          </tr>
        </thead>
        <tbody>
          {connections.map((connection) => (
            <tr key={connection.id}>
              <th scope="row">{connection.label}</th>
              <td>
                {isLiveProfile(connection) ? (
                  <strong>LIVE — places real orders</strong>
                ) : (
                  "Mock — simulated"
                )}
              </td>
              <td>
                {connection.account_no_masked} ({connection.account_product_code})
              </td>
              <td>
                <ul>
                  <li>key: {connection.kis_app_key_ref}</li>
                  <li>secret: {connection.kis_app_secret_ref}</li>
                </ul>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="supporting-copy">
        Credentials are shown as locations, never values. The server stores a reference to where
        each credential lives and has no field capable of holding the credential itself.
      </p>
    </section>
  );
}
