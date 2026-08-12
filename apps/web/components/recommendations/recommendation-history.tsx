import { StatusPill } from "@/components/states/status-pill";
import type { RecommendationRunModel } from "@/lib/products/contracts";
import { formatDate, formatTimestamp } from "@/lib/products/format";

export type RecommendationHistoryProps = {
  readonly runs: readonly RecommendationRunModel[];
};

export function RecommendationHistory({ runs }: RecommendationHistoryProps) {
  const newestFirst = [...runs].sort((left, right) =>
    right.created_at.localeCompare(left.created_at),
  );
  return (
    <section aria-labelledby="recommendation-history-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Historical runs</p>
          <h2 id="recommendation-history-title">Recommendation history</h2>
        </div>
      </div>
      {newestFirst.length === 0 ? (
        <p className="empty-copy">No historical recommendation runs are available.</p>
      ) : (
        <div className="data-table-wrap">
          <table>
            <caption>Recommendation run history</caption>
            <thead>
              <tr>
                <th scope="col">As of</th>
                <th scope="col">Created</th>
                <th scope="col">Status</th>
                <th scope="col">Run ID</th>
              </tr>
            </thead>
            <tbody>
              {newestFirst.map((run) => (
                <tr key={run.id}>
                  <th scope="row">{formatDate(run.as_of)}</th>
                  <td>
                    {run.created_at === undefined
                      ? "Not reported"
                      : formatTimestamp(run.created_at)}
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
