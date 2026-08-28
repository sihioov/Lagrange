"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { useLocale } from "@/lib/i18n/client";
import { backtestsDictionary } from "@/lib/i18n/dictionaries/backtests";
import { robustnessQueuedSchema } from "@/lib/products/backtest-contracts";

export type RobustnessControlProps = {
  readonly runId: string;
};

export function RobustnessControl({ runId }: RobustnessControlProps) {
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const { locale } = useLocale();
  const t = backtestsDictionary[locale];

  async function queue(): Promise<void> {
    setSubmitting(true);
    try {
      const response = await mutateWithCsrf(`/api/v1/backtests/${runId}/robustness`, {
        json: {},
        method: "POST",
      });
      await parseBrowserApiResponse(response, robustnessQueuedSchema);
      setMessage(t.robustnessQueuedMessage);
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
      <button
        className="secondary-action"
        disabled={submitting}
        onClick={() => void queue()}
        type="button"
      >
        {submitting ? t.queueingRobustnessLabel : t.runRobustnessButton}
      </button>
      {message === "" ? null : (
        <p className="form-result" role="status">
          {message}
        </p>
      )}
    </div>
  );
}
