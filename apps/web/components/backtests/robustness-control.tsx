"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import { robustnessQueuedSchema } from "@/lib/products/backtest-contracts";

export type RobustnessControlProps = {
  readonly runId: string;
};

export function RobustnessControl({ runId }: RobustnessControlProps) {
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);

  async function queue(): Promise<void> {
    setSubmitting(true);
    try {
      const response = await mutateWithCsrf(`/api/v1/backtests/${runId}/robustness`, {
        json: {},
        method: "POST",
      });
      await parseApiResponse(response, robustnessQueuedSchema);
      setMessage("Robustness queued. Existing evidence remains visible while the server runs it.");
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
    <div className="workflow-form">
      <button className="secondary-action" disabled={submitting} onClick={() => void queue()} type="button">
        {submitting ? "Queueing robustness evidence" : "Run robustness evidence"}
      </button>
      {message === "" ? null : <p className="form-result" role="status">{message}</p>}
    </div>
  );
}
