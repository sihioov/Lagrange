import { StatusPill } from "@/components/states/status-pill";
import type { RecommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import type { RecommendationRunModel } from "@/lib/products/contracts";
import { formatDate, formatTimestamp } from "@/lib/products/format";

export type RecommendationHistoryProps = {
  readonly runs: readonly RecommendationRunModel[];
  readonly t: RecommendationsDictionary;
};

export function RecommendationHistory({ runs, t }: RecommendationHistoryProps) {
  const newestFirst = [...runs].sort((left, right) =>
    right.created_at.localeCompare(left.created_at),
  );
  return (
    <section aria-labelledby="recommendation-history-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.historyEyebrow}</p>
          <h2 id="recommendation-history-title">{t.historyHeading}</h2>
        </div>
      </div>
      {newestFirst.length === 0 ? (
        <p className="empty-copy">{t.historyEmptyMessage}</p>
      ) : (
        <div className="data-table-wrap">
          <table>
            <caption>{t.historyCaption}</caption>
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
                  <td>
                    {run.created_at === undefined ? t.notReported : formatTimestamp(run.created_at)}
                  </td>
                  <td>
                    <StatusPill
                      label={run.status}
                      tone={run.status === "SUCCEEDED" ? "success" : "warning"}
                    />
                  </td>
                  <td className="data-cell">
                    <a href={`/recommendations?run_id=${encodeURIComponent(run.id)}`}>{run.id}</a>
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
