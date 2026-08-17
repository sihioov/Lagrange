"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import { backtestsDictionary } from "@/lib/i18n/dictionaries/backtests";
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
  const { locale } = useLocale();
  const t = backtestsDictionary[locale];

  async function compare(form: HTMLFormElement): Promise<void> {
    const runIds = new FormData(form).getAll("run_id").filter((value) => typeof value === "string");
    if (runIds.length !== 2) {
      setMessage(t.selectTwoRunsMessage);
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
          <p className="eyebrow">{t.comparisonEyebrow}</p>
          <h2>{t.compareRunsHeading}</h2>
        </div>
      </div>
      <form
        aria-label={t.compareRunsAriaLabel}
        className="workflow-form"
        onSubmit={(event) => {
          event.preventDefault();
          void compare(event.currentTarget);
        }}
      >
        <fieldset>
          <legend>{t.completedRunsLegend}</legend>
          <div className="field-grid">
            {runs.map((run) => (
              <label className="form-field" key={run.id}>
                <span>
                  {backtestRunLabel(run)} ·{" "}
                  {run.can_manage
                    ? t.yourRunLabel
                    : t.sharedRunLabel(run.owner_user_id.slice(0, 8))}
                </span>
                <input name="run_id" type="checkbox" value={run.id} />
              </label>
            ))}
          </div>
        </fieldset>
        <button className="secondary-action" type="submit">
          {t.compareSelectedRunsButton}
        </button>
        {message === "" ? null : (
          <p className="form-result" role="alert">
            {message}
          </p>
        )}
      </form>
      {comparison === null ? null : (
        <section aria-labelledby="comparison-result-title" className="report-section">
          <h3 id="comparison-result-title">{t.runComparisonHeading}</h3>
          <dl className="provenance-grid">
            <div>
              <dt>{t.totalReturnDeltaLabel}</dt>
              <dd>{formatPercentage(comparison.deltas.total_return)}</dd>
            </div>
          </dl>
        </section>
      )}
    </section>
  );
}
