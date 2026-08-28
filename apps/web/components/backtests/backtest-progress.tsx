"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { useLocale } from "@/lib/i18n/client";
import { backtestsDictionary } from "@/lib/i18n/dictionaries/backtests";
import {
  type BacktestRunModel,
  backtestProgress,
  cancelBacktestSchema,
} from "@/lib/products/backtest-contracts";

export type BacktestProgressProps = {
  readonly run: BacktestRunModel;
};

export function BacktestProgress({ run }: BacktestProgressProps) {
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const progress = backtestProgress(run);
  const { locale } = useLocale();
  const t = backtestsDictionary[locale];

  async function cancel(): Promise<void> {
    setSubmitting(true);
    try {
      const response = await mutateWithCsrf(`/api/v1/backtests/${run.id}/cancel`, {
        json: {},
        method: "POST",
      });
      await parseBrowserApiResponse(response, cancelBacktestSchema);
      setMessage(t.cancellationRequestedMessage);
    } catch (error) {
      if (error instanceof Error) {
        setMessage(error.message);
        return;
      }
      throw error;
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <section aria-labelledby="backtest-progress-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.progressEyebrow}</p>
          <h2 id="backtest-progress-title">{t.progressHeading}</h2>
        </div>
        <strong>{run.status}</strong>
      </div>
      <p>{progress === null ? t.progressNotReported : t.percentComplete(progress)}</p>
      {run.can_manage ? (
        <button
          className="secondary-action"
          disabled={submitting}
          onClick={() => void cancel()}
          type="button"
        >
          {submitting ? t.requestingCancellationLabel : t.cancelBacktestButton}
        </button>
      ) : null}
      {message === "" ? null : (
        <p className="form-result" role="status">
          {message}
        </p>
      )}
    </section>
  );
}
