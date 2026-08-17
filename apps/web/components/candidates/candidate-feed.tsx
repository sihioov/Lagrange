import Link from "next/link";
import { ResearchProvenance } from "@/components/candidates/research-provenance";
import { UniverseBadge } from "@/components/candidates/universe-badge";
import { StatusPill } from "@/components/states/status-pill";
import type { CandidateAnalysis, CandidateFeed } from "@/lib/products/candidate-contracts";
import { universeLabel } from "@/lib/products/candidate-contracts";
import { formatDate, formatTimestamp } from "@/lib/products/format";

function score(value: number | null): string {
  return value === null ? "Not available" : value.toFixed(1);
}

function evidenceTone(value: CandidateAnalysis["evidence_strength"]) {
  if (value === "STRONG") return "success" as const;
  if (value === "MODERATE") return "info" as const;
  return "warning" as const;
}

export function CandidateFeedReport({ feed }: { readonly feed: CandidateFeed }) {
  return (
    <section aria-labelledby="candidate-feed-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">Common daily Top 5 · {universeLabel(feed.universe)}</p>
          <h2 id="candidate-feed-title">Evidence-ranked stock candidates</h2>
          <p>Post-close research within the selected point-in-time universe.</p>
        </div>
        <div className="status-cluster">
          <UniverseBadge universe={feed.universe} />
          <StatusPill label={feed.state} tone={feed.state === "READY" ? "success" : "warning"} />
          <span>As of {formatDate(feed.as_of)}</span>
        </div>
      </header>

      {feed.state === "STALE" ? (
        <aside className="warning-strip" role="status">
          <strong>Stale research snapshot</strong>
          <p>The most recent governed feed is from {formatDate(feed.as_of)}.</p>
        </aside>
      ) : null}

      <div className="data-table-wrap">
        <table>
          <caption>
            Daily candidates ranked by the governed composite score within this universe
          </caption>
          <thead>
            <tr>
              <th scope="col">Rank</th>
              <th scope="col">Instrument</th>
              <th scope="col">Total</th>
              <th scope="col">Foreign / institution</th>
              <th scope="col">Fundamental</th>
              <th scope="col">Technical</th>
              <th scope="col">Evidence</th>
            </tr>
          </thead>
          <tbody>
            {feed.items.map((item) => (
              <tr key={item.analysis_id}>
                <td>{item.rank ?? "—"}</td>
                <th scope="row">
                  <Link
                    className="data-link"
                    href={`/stocks/${encodeURIComponent(item.instrument_id)}?date=${encodeURIComponent(feed.as_of)}&universe=${encodeURIComponent(feed.universe)}`}
                  >
                    {item.name ?? item.instrument_id}
                    {item.name === null || item.name === undefined ? null : (
                      <small>{item.instrument_id}</small>
                    )}
                  </Link>
                </th>
                <td className="score-emphasis">{score(item.scores.total)}</td>
                <td>{score(item.scores.flow)}</td>
                <td>{score(item.scores.fundamental)}</td>
                <td>{score(item.scores.technical)}</td>
                <td>
                  <StatusPill
                    label={item.evidence_strength}
                    tone={evidenceTone(item.evidence_strength)}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="candidate-actions">
        <Link
          className="secondary-action"
          href={`/screener?as_of=${encodeURIComponent(feed.as_of)}&universes=${encodeURIComponent(feed.universe)}`}
        >
          Screen the full universe
        </Link>
        <span>
          Published {formatTimestamp(feed.published_at)} · computation {feed.computation_seq}
        </span>
      </div>

      <ResearchProvenance {...feed} />
    </section>
  );
}
