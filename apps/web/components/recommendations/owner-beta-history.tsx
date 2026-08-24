import Link from "next/link";
import { StatusPill } from "@/components/states/status-pill";
import type { RecommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import { formatDate, formatTimestamp } from "@/lib/products/format";
import type { OwnerBetaRunListItemModel } from "@/lib/products/owner-beta-contracts";

export type OwnerBetaHistoryProps = {
  readonly runs: readonly OwnerBetaRunListItemModel[];
  readonly t: RecommendationsDictionary;
};

export function OwnerBetaHistory({ runs, t }: OwnerBetaHistoryProps) {
  const newestFirst = [...runs].sort((left, right) =>
    right.created_at.localeCompare(left.created_at),
  );
  return (
    <section aria-labelledby="owner-beta-history-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.historyEyebrow}</p>
          <h2 id="owner-beta-history-title">{t.ownerBetaHistoryHeading}</h2>
        </div>
      </div>
      {newestFirst.length === 0 ? (
        <p className="empty-copy">{t.ownerBetaNoRunsMessage}</p>
      ) : (
        <div className="data-table-wrap">
          <table>
            <caption>{t.ownerBetaHistoryCaption}</caption>
            <thead>
              <tr>
                <th scope="col">{t.columnAsOf}</th>
                <th scope="col">{t.columnCreated}</th>
                <th scope="col">{t.columnStatus}</th>
                <th scope="col">{t.columnRunId}</th>
              </tr>
            </thead>
            <tbody>
              {newestFirst.map((run) => (
                <tr key={run.id}>
                  <th scope="row">{formatDate(run.as_of)}</th>
                  <td>{formatTimestamp(run.created_at)}</td>
                  <td>
                    <StatusPill
                      label={run.status}
                      tone={run.status === "SUCCEEDED" ? "success" : "warning"}
                    />
                  </td>
                  <td className="data-cell">
                    <Link href={`/recommendations?run_id=${encodeURIComponent(run.id)}`}>
                      {run.id}
                    </Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
