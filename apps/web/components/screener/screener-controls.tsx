import {
  DEFAULT_UNIVERSE,
  type ScreenCriteria,
  UNIVERSE_KEYS,
  universeLabel,
} from "@/lib/products/candidate-contracts";

function threshold(value: number | null | undefined): string | number {
  return value ?? "";
}

export function ScreenerControls({
  asOf,
  criteria,
}: {
  readonly asOf: string;
  readonly criteria: ScreenCriteria;
}) {
  const evidence = new Set(criteria.evidence_strength ?? []);
  const universes = new Set(criteria.universes ?? [DEFAULT_UNIVERSE]);
  return (
    <section aria-labelledby="screener-controls-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Immutable run filter</p>
          <h2 id="screener-controls-title">Screen the governed universe</h2>
        </div>
        <p>Filters only narrow an already published run; they never recompute or re-rank it.</p>
      </div>
      <form className="workflow-form" method="get">
        <fieldset>
          <legend>Candidate universes</legend>
          <div className="choice-row">
            {UNIVERSE_KEYS.map((universe) => (
              <label key={universe}>
                <input
                  defaultChecked={universes.has(universe)}
                  name="universes"
                  type="checkbox"
                  value={universe}
                />
                <span>{universeLabel(universe)}</span>
              </label>
            ))}
          </div>
          <small>Select one universe or both; each result stays in its own ranking context.</small>
        </fieldset>
        <div className="field-grid">
          <label className="form-field">
            <span>As-of date</span>
            <input defaultValue={asOf} name="as_of" required type="date" />
          </label>
          <label className="form-field">
            <span>Sector codes</span>
            <input
              defaultValue={(criteria.sectors ?? []).join(", ")}
              name="sectors"
              placeholder="G25, G35"
              type="text"
            />
            <small>Comma-separated exact classification codes.</small>
          </label>
          <label className="form-field">
            <span>Minimum total score</span>
            <input
              defaultValue={threshold(criteria.min_total_score)}
              max="100"
              min="0"
              name="min_total_score"
              step="0.1"
              type="number"
            />
          </label>
          <label className="form-field">
            <span>Minimum flow score</span>
            <input
              defaultValue={threshold(criteria.min_flow_score)}
              max="100"
              min="0"
              name="min_flow_score"
              step="0.1"
              type="number"
            />
          </label>
          <label className="form-field">
            <span>Minimum fundamental score</span>
            <input
              defaultValue={threshold(criteria.min_fundamental_score)}
              max="100"
              min="0"
              name="min_fundamental_score"
              step="0.1"
              type="number"
            />
          </label>
          <label className="form-field">
            <span>Minimum technical score</span>
            <input
              defaultValue={threshold(criteria.min_technical_score)}
              max="100"
              min="0"
              name="min_technical_score"
              step="0.1"
              type="number"
            />
          </label>
        </div>
        <fieldset>
          <legend>Evidence strength</legend>
          <div className="choice-row">
            {(["STRONG", "MODERATE", "WEAK"] as const).map((value) => (
              <label key={value}>
                <input
                  defaultChecked={evidence.has(value)}
                  name="evidence"
                  type="checkbox"
                  value={value}
                />
                <span>{value}</span>
              </label>
            ))}
          </div>
        </fieldset>
        <button className="primary-action" type="submit">
          Apply screen
        </button>
      </form>
    </section>
  );
}
