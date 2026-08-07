"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
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

  async function cancel(): Promise<void> {
    setSubmitting(true);
    try {
      const response = await mutateWithCsrf(`/api/v1/backtests/${run.id}/cancel`, {
        json: {},
        method: "POST",
      });
      await parseApiResponse(response, cancelBacktestSchema);
      setMessage("Cancellation requested. The server will preserve the job audit trail.");
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
          <p className="eyebrow">Queued execution</p>
          <h2 id="backtest-progress-title">Backtest progress</h2>
        </div>
        <strong>{run.status}</strong>
      </div>
      <p>{progress === null ? "Progress not reported" : `${progress}% complete`}</p>
      <button
        className="secondary-action"
        disabled={submitting}
        onClick={() => void cancel()}
        type="button"
      >
        {submitting ? "Requesting cancellation" : "Cancel backtest"}
      </button>
      {message === "" ? null : (
        <p className="form-result" role="status">
          {message}
        </p>
      )}
    </section>
  );
}
