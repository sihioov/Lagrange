import { ResearchProvenance } from "@/components/candidates/research-provenance";
import { StatusPill } from "@/components/states/status-pill";
import {
  candidateProfileLabel,
  type StockAnalysisResponse,
} from "@/lib/products/candidate-contracts";
import { formatDate } from "@/lib/products/format";

type JsonRecord = Readonly<Record<string, unknown>>;

function object(value: unknown): JsonRecord | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function number(value: unknown): string {
  return typeof value === "number" && Number.isFinite(value) ? value.toFixed(4) : "Not available";
}

function score(value: number | null): string {
  return value === null ? "Not available" : value.toFixed(1);
}

function coverage(value: number): string {
  return `${(value * 100).toFixed(0)}%`;
}

function ScenarioGrid({ scenarios }: { readonly scenarios: JsonRecord }) {
  const rows = Object.entries(scenarios)
    .map(([key, value]) => ({ key, value: object(value) }))
    .filter((row): row is { key: string; value: JsonRecord } => row.value !== null);

  return (
    <div className="scenario-grid">
      {rows.map(({ key, value }) => {
        const evidence = Array.isArray(value["evidence_refs"])
          ? value["evidence_refs"].filter((entry): entry is string => typeof entry === "string")
          : [];
        return (
          <article className="scenario-panel" key={key}>
            <p className="eyebrow">{String(value["label"] ?? key)}</p>
            <h4>{String(value["title"] ?? key)}</h4>
            <p>{String(value["trigger_expression"] ?? "Trigger not reported")}</p>
            <dl>
              <dt>Evidence references</dt>
              <dd>{evidence.length === 0 ? "Not reported" : evidence.join(", ")}</dd>
            </dl>
          </article>
        );
      })}
    </div>
  );
}

function FactorTable({ factors }: { readonly factors: JsonRecord }) {
  return (
    <div className="data-table-wrap">
      <table>
        <caption>Point-in-time factors and cross-sectional normalization</caption>
        <thead>
          <tr>
            <th scope="col">Factor</th>
            <th scope="col">Raw</th>
            <th scope="col">Normalized</th>
            <th scope="col">Weight</th>
            <th scope="col">Scope</th>
          </tr>
        </thead>
        <tbody>
          {Object.entries(factors).map(([name, value]) => {
            const factor = object(value);
            return (
              <tr key={name}>
                <th scope="row">{name.replaceAll("_", " ")}</th>
                <td>{number(factor?.["raw"])}</td>
                <td>{number(factor?.["normalized"])}</td>
                <td>{number(factor?.["weight"])}</td>
                <td>{String(factor?.["normalization_scope"] ?? "Not available")}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function StockAnalysisReport({ report }: { readonly report: StockAnalysisResponse }) {
  const { analysis } = report;
  return (
    <section aria-labelledby="stock-analysis-title" className="data-report">
      <header className="report-heading">
        <div>
          <p className="eyebrow">Deep stock analysis</p>
          <h2 id="stock-analysis-title">{analysis.name ?? analysis.instrument_id}</h2>
          <p>
            {analysis.instrument_id} · {analysis.sector_code} ·{" "}
            {candidateProfileLabel(analysis.fundamental_profile)}
          </p>
        </div>
        <div className="status-cluster">
          <StatusPill
            label={report.state}
            tone={report.state === "READY" ? "success" : "warning"}
          />
          <StatusPill
            label={analysis.evidence_strength}
            tone={analysis.evidence_strength === "WEAK" ? "warning" : "info"}
          />
          <span>As of {formatDate(report.as_of)}</span>
        </div>
      </header>

      {analysis.eligible ? null : (
        <aside className="warning-strip" role="status">
          <strong>Excluded from the daily candidate ranking</strong>
          <p>
            {analysis.exclusion_codes.length === 0
              ? "The evidence gate was not satisfied."
              : analysis.exclusion_codes.join(", ")}
          </p>
        </aside>
      )}

      <section aria-labelledby="axis-score-title" className="report-section">
        <h3 id="axis-score-title">Evidence axes</h3>
        <div className="score-grid">
          <article>
            <span className="score-label">Foreign & institution flow</span>
            <strong>{score(analysis.scores.flow)}</strong>
            <small className="score-caption">{coverage(analysis.coverage.flow)} coverage</small>
          </article>
          <article>
            <span className="score-label">Fundamental</span>
            <strong>{score(analysis.scores.fundamental)}</strong>
            <small className="score-caption">
              {coverage(analysis.coverage.fundamental)} coverage
            </small>
          </article>
          <article>
            <span className="score-label">Technical</span>
            <strong>{score(analysis.scores.technical)}</strong>
            <small className="score-caption">
              {coverage(analysis.coverage.technical)} coverage
            </small>
          </article>
          <article>
            <span className="score-label">Composite</span>
            <strong>{score(analysis.scores.total)}</strong>
            <small className="score-caption">
              {analysis.normalization_scope.replaceAll("_", " ")}
            </small>
          </article>
        </div>
      </section>

      <section aria-labelledby="scenario-title" className="report-section">
        <div className="section-heading">
          <div>
            <h3 id="scenario-title">Conditional scenarios</h3>
            <p>Upside, neutral, and downside paths are rules tied to evidence—not forecasts.</p>
          </div>
        </div>
        <ScenarioGrid scenarios={analysis.scenarios} />
      </section>

      <section aria-labelledby="factor-title" className="report-section">
        <h3 id="factor-title">Factor evidence</h3>
        <FactorTable factors={analysis.factors} />
      </section>

      <ResearchProvenance {...report} />
    </section>
  );
}
