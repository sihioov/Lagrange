"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import { recommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import { type RecommendationRunModel, recommendationRunSchema } from "@/lib/products/contracts";
import { RecommendationRunStatus } from "./recommendation-run-status";

type RunState = "error" | "idle" | "submitting";

export type RecommendationRunFormProps = {
  readonly configs: readonly {
    readonly id: string;
    readonly label: string;
  }[];
  readonly defaultAsOf: string;
};

export function RecommendationRunForm({ configs, defaultAsOf }: RecommendationRunFormProps) {
  const [state, setState] = useState<RunState>("idle");
  const [message, setMessage] = useState("");
  const [submittedRun, setSubmittedRun] = useState<RecommendationRunModel | null>(null);
  const { locale } = useLocale();
  const t = recommendationsDictionary[locale];

  async function submit(form: HTMLFormElement): Promise<void> {
    const formData = new FormData(form);
    const asOf = formData.get("as_of");
    const strategyConfigId = formData.get("strategy_config_id");
    if (typeof asOf !== "string" || asOf === "") {
      setState("error");
      setMessage(t.asOfDateRequired);
      return;
    }
    if (typeof strategyConfigId !== "string" || strategyConfigId === "") {
      setState("error");
      setMessage(t.selectStrategyConfig);
      return;
    }
    setState("submitting");
    setSubmittedRun(null);
    try {
      const response = await mutateWithCsrf("/api/v1/recommendations/runs", {
        json: { as_of: asOf, strategy_config_id: strategyConfigId },
        method: "POST",
      });
      const run = await parseApiResponse(response, recommendationRunSchema);
      setState("idle");
      setSubmittedRun(run);
    } catch (error) {
      if (error instanceof Error) {
        setState("error");
        setMessage(error.message);
        return;
      }
      throw error;
    }
  }

  return (
    <>
      <form
        aria-label={t.generateRecommendation}
        className="workflow-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void submit(event.currentTarget);
        }}
      >
        <label className="form-field">
          <span>{t.strategyConfigurationLabel}</span>
          <select defaultValue={configs[0]?.id ?? ""} name="strategy_config_id" required>
            {configs.map((config) => (
              <option key={config.id} value={config.id}>
                {config.label}
              </option>
            ))}
          </select>
        </label>
        <label className="form-field">
          <span>{t.asOfDateLabel}</span>
          <input defaultValue={defaultAsOf} name="as_of" required type="date" />
        </label>
        <button className="primary-action" disabled={state === "submitting"} type="submit">
          {state === "submitting" ? t.generatingStrategyProposal : t.generateStrategyProposal}
        </button>
        {state === "error" ? (
          <p className="form-result" role="alert">
            {message}
          </p>
        ) : null}
      </form>
      {submittedRun === null ? null : <RecommendationRunStatus poll run={submittedRun} />}
    </>
  );
}
