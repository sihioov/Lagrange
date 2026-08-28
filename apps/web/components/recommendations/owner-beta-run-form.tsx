"use client";

import { useState } from "react";
import { type BrowserClientOptions, mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { useLocale } from "@/lib/i18n/client";
import { recommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import {
  OWNER_BETA_PRICE_ONLY_RUNS_PATH,
  type OwnerBetaPriceOnlyRunBody,
  type OwnerBetaPriceOnlyRunResponse,
  ownerBetaPriceOnlyRunBodySchema,
  ownerBetaPriceOnlyRunResponseSchema,
} from "@/lib/products/owner-beta-contracts";
import { OwnerBetaRunStatus } from "./owner-beta-run-status";

type RunState = "error" | "idle" | "submitting";

export type OwnerBetaRunFormProps = {
  readonly configs: readonly {
    readonly id: string;
    readonly label: string;
  }[];
  readonly defaultAsOf: string;
  readonly supportedAsOf: readonly string[];
};

export function isOwnerBetaAsOfSupported(asOf: string, supportedAsOf: readonly string[]): boolean {
  return supportedAsOf.includes(asOf);
}

/** Submit only the server-owned owner-beta request body and parse its enqueue response. */
export async function submitOwnerBetaRun(
  body: OwnerBetaPriceOnlyRunBody,
  options: BrowserClientOptions = {},
): Promise<OwnerBetaPriceOnlyRunResponse> {
  const response = await mutateWithCsrf(OWNER_BETA_PRICE_ONLY_RUNS_PATH, {
    ...options,
    json: body,
    method: "POST",
  });
  return parseBrowserApiResponse(response, ownerBetaPriceOnlyRunResponseSchema);
}

export function OwnerBetaRunForm({ configs, defaultAsOf, supportedAsOf }: OwnerBetaRunFormProps) {
  const [state, setState] = useState<RunState>("idle");
  const [message, setMessage] = useState("");
  const [submittedRun, setSubmittedRun] = useState<OwnerBetaPriceOnlyRunResponse | null>(null);
  const { locale } = useLocale();
  const t = recommendationsDictionary[locale];

  async function submit(form: HTMLFormElement): Promise<void> {
    const formData = new FormData(form);
    const asOf = formData.get("as_of");
    const strategyConfigId = formData.get("strategy_config_id");
    if (typeof asOf !== "string" || typeof strategyConfigId !== "string") {
      setState("error");
      setMessage(t.asOfDateRequired);
      return;
    }
    if (!isOwnerBetaAsOfSupported(asOf, supportedAsOf)) {
      setState("error");
      setMessage(t.ownerBetaUnsupportedAsOf);
      return;
    }
    const body = ownerBetaPriceOnlyRunBodySchema.safeParse({
      as_of: asOf,
      strategy_config_id: strategyConfigId,
    });
    if (!body.success) {
      setState("error");
      setMessage(strategyConfigId === "" ? t.selectStrategyConfig : t.asOfDateRequired);
      return;
    }

    setState("submitting");
    setMessage("");
    setSubmittedRun(null);
    try {
      const run = await submitOwnerBetaRun(body.data);
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

  const canSubmit = isOwnerBetaAsOfSupported(defaultAsOf, supportedAsOf);

  return (
    <>
      <form
        aria-label={t.ownerBetaSubmit}
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
          <span>{t.ownerBetaSealedAsOfLabel}</span>
          <select defaultValue={defaultAsOf} name="as_of" required>
            {supportedAsOf.map((asOf) => (
              <option key={asOf} value={asOf}>
                {asOf}
              </option>
            ))}
          </select>
          <small>
            {t.ownerBetaLatestSupportedAsOfLabel}: {defaultAsOf}
          </small>
        </label>
        <button
          className="primary-action"
          disabled={state === "submitting" || !canSubmit}
          type="submit"
        >
          {state === "submitting" ? t.ownerBetaSubmitting : t.ownerBetaSubmit}
        </button>
        {state === "error" ? (
          <p className="form-result" role="alert">
            {message}
          </p>
        ) : null}
      </form>
      {submittedRun === null ? null : (
        <OwnerBetaRunStatus
          initialStatus={submittedRun.status}
          key={submittedRun.run_id}
          runId={submittedRun.run_id}
        />
      )}
    </>
  );
}
