"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import {
  type BacktestCompareModel,
  type BacktestRunModel,
  backtestCompareSchema,
  backtestRunLabel,
} from "@/lib/products/backtest-contracts";
import { formatPercentage } from "@/lib/products/format";

export type BacktestComparisonProps = {
  readonly runs: readonly BacktestRunModel[];
};

export function BacktestComparison({ runs }: BacktestComparisonProps) {
  const [comparison, setComparison] = useState<BacktestCompareModel | null>(null);
  const [message, setMessage] = useState("");

  async function compare(form: HTMLFormElement): Promise<void> {
    const runIds = new FormData(form).getAll("run_id").filter((value) => typeof value === "string");
    if (runIds.length !== 2) {
      setMessage("Select exactly two completed runs.");
      return;
    }
    try {
      const response = await mutateWithCsrf("/api/v1/backtests/compare", {
        json: { run_ids: runIds },
        method: "POST",
      });
      setComparison(await parseApiResponse(response, backtestCompareSchema));
      setMessage("");
    } catch (error) {
      if (error instanceof Error) {
        setMessage(error.message);
        return;
      }
      throw error;
    }
  }

  return (
    <section className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Server comparison</p>
          <h2>Compare runs</h2>
        </div>
      </div>
      <form
        aria-label="Compare backtest runs"
        className="workflow-form"
        onSubmit={(event) => {
          event.preventDefault();
          void compare(event.currentTarget);
        }}
      >
        <fieldset>
          <legend>Completed runs</legend>
          <div className="field-grid">
            {runs.map((run) => (
              <label className="form-field" key={run.id}>
                <span>{backtestRunLabel(run)}</span>
                <input name="run_id" type="checkbox" value={run.id} />
              </label>
            ))}
          </div>
        </fieldset>
        <button className="secondary-action" type="submit">
          Compare selected runs
        </button>
        {message === "" ? null : (
          <p className="form-result" role="alert">
            {message}
          </p>
        )}
      </form>
      {comparison === null ? null : (
        <section aria-labelledby="comparison-result-title" className="report-section">
          <h3 id="comparison-result-title">Run comparison</h3>
          <dl className="provenance-grid">
            <div>
              <dt>Total return delta</dt>
              <dd>{formatPercentage(comparison.deltas.total_return)}</dd>
            </div>
          </dl>
        </section>
      )}
    </section>
  );
}
