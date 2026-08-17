"use client";

import { useLocale } from "@/lib/i18n/client";
import { liveDictionary } from "@/lib/i18n/dictionaries/live";
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
  const { locale } = useLocale();
  const t = liveDictionary[locale];

  if (connections.length === 0) {
    return (
      <section aria-labelledby="live-connections-title" className="report-section">
        <h3 id="live-connections-title">{t.connectionsTitle}</h3>
        <p className="supporting-copy">{t.noConnectionMessage}</p>
      </section>
    );
  }

  return (
    <section aria-labelledby="live-connections-title" className="report-section">
      <h3 id="live-connections-title">{t.connectionsTitle}</h3>
      <table>
        <caption>{t.connectionsCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnConnection}</th>
            <th scope="col">{t.columnProfile}</th>
            <th scope="col">{t.columnAccount}</th>
            <th scope="col">{t.columnCredentialLocations}</th>
          </tr>
        </thead>
        <tbody>
          {connections.map((connection) => (
            <tr key={connection.id}>
              <th scope="row">{connection.label}</th>
              <td>
                {isLiveProfile(connection) ? (
                  <strong>{t.liveProfileLabel}</strong>
                ) : (
                  t.mockProfileLabel
                )}
              </td>
              <td>
                {connection.account_no_masked} ({connection.account_product_code})
              </td>
              <td>
                <ul>
                  <li>{t.keyLabel(connection.kis_app_key_ref)}</li>
                  <li>{t.secretLabel(connection.kis_app_secret_ref)}</li>
                </ul>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="supporting-copy">{t.credentialsFootnote}</p>
    </section>
  );
}
