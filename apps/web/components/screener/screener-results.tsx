import Link from "next/link";
import { ResearchProvenance } from "@/components/candidates/research-provenance";
import { StatusPill } from "@/components/states/status-pill";
import type { ScreenerResult } from "@/lib/products/candidate-contracts";

function score(value: number | null): string {
  return value === null ? "—" : value.toFixed(1);
}

export function ScreenerResults({
  nextHref,
  result,
}: {
  readonly nextHref: string | null;
  readonly result: ScreenerResult;
}) {
  return (
    <section aria-labelledby="screener-results-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">Filtered published evidence</p>
          <h2 id="screener-results-title">Screen results</h2>
          <p>{result.items.length} instruments on this page, ordered by immutable score.</p>
        </div>
        <div className="status-cluster">
          <StatusPill
            label={result.state}
            tone={result.state === "READY" ? "success" : "warning"}
          />
        </div>
      </header>
      {result.items.length === 0 ? (
        <p className="empty-copy">No eligible instruments satisfy every selected threshold.</p>
      ) : (
        <div className="data-table-wrap">
          <table>
            <caption>Candidate screener results</caption>
            <thead>
              <tr>
                <th scope="col">Instrument</th>
                <th scope="col">Sector</th>
                <th scope="col">Total</th>
                <th scope="col">Flow</th>
                <th scope="col">Fundamental</th>
                <th scope="col">Technical</th>
                <th scope="col">Evidence</th>
              </tr>
            </thead>
            <tbody>
              {result.items.map((item) => (
                <tr key={item.analysis_id}>
                  <th scope="row">
                    <Link
                      className="data-link"
                      href={`/stocks/${encodeURIComponent(item.instrument_id)}?date=${encodeURIComponent(result.as_of)}`}
                    >
                      {item.name ?? item.instrument_id}
                      {item.name === null || item.name === undefined ? null : (
                        <small>{item.instrument_id}</small>
                      )}
                    </Link>
                  </th>
                  <td>{item.sector_code}</td>
                  <td className="score-emphasis">{score(item.scores.total)}</td>
                  <td>{score(item.scores.flow)}</td>
                  <td>{score(item.scores.fundamental)}</td>
                  <td>{score(item.scores.technical)}</td>
                  <td>{item.evidence_strength}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {nextHref === null ? null : (
        <Link className="secondary-action" href={nextHref}>
          Load next result page
        </Link>
      )}
      <ResearchProvenance {...result} />
    </section>
  );
}
