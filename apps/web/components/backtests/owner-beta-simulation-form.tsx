"use client";

import { useEffect, useRef, useState } from "react";
import { StatePanel } from "@/components/states/state-panel";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { subscribeToBrowserLogout } from "@/lib/api/browser-lifecycle";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { ApiContractError, ApiProblem, isLoginRequiredError } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import { ownerBetaSimulationDictionary } from "@/lib/i18n/dictionaries/owner-beta-simulation";
import {
  OWNER_BETA_SIMULATION_RUNS_PATH,
  type OwnerBetaSimulationEnqueued,
  type OwnerBetaSimulationRunBody,
  ownerBetaSimulationEnqueuedSchema,
  ownerBetaSimulationRunBodySchema,
} from "@/lib/products/owner-beta-simulation-contracts";

export type OwnerBetaSimulationFormConfig = {
  readonly createdAt: string;
  readonly id: string;
};

export type OwnerBetaSimulationFormProps = {
  readonly configs: readonly OwnerBetaSimulationFormConfig[];
  readonly onAccessLoss: (kind: "entitlement" | "forbidden" | "session") => void;
  readonly onQueued: (
    queued: OwnerBetaSimulationEnqueued,
    body: OwnerBetaSimulationRunBody,
  ) => void;
};

type FormState = "idle" | "submitting";

function requestFingerprint(body: OwnerBetaSimulationRunBody): string {
  return JSON.stringify(body);
}

export function OwnerBetaSimulationForm({
  configs,
  onAccessLoss,
  onQueued,
}: OwnerBetaSimulationFormProps) {
  const [state, setState] = useState<FormState>("idle");
  const [error, setError] = useState<string | null>(null);
  const requestRef = useRef<AbortController | null>(null);
  const idempotencyRef = useRef<{ readonly fingerprint: string; readonly key: string } | null>(
    null,
  );
  const { locale } = useLocale();
  const t = ownerBetaSimulationDictionary[locale];

  useEffect(
    () =>
      subscribeToBrowserLogout(() => {
        requestRef.current?.abort();
        requestRef.current = null;
        idempotencyRef.current = null;
        setState("idle");
        setError(null);
      }),
    [],
  );

  useEffect(
    () => () => {
      requestRef.current?.abort();
      requestRef.current = null;
    },
    [],
  );

  async function submit(form: HTMLFormElement): Promise<void> {
    if (requestRef.current !== null) return;
    const data = new FormData(form);
    const configId = data.get("strategy_config_id");
    const startDate = data.get("start_date");
    const endDate = data.get("end_date");
    if (configId === "") {
      setError(t.configRequiredMessage);
      return;
    }
    if (
      typeof configId !== "string" ||
      typeof startDate !== "string" ||
      typeof endDate !== "string"
    ) {
      setError(t.invalidDateMessage);
      return;
    }
    const candidate = {
      strategy_config_id: configId,
      ...(startDate === "" ? {} : { start_date: startDate }),
      ...(endDate === "" ? {} : { end_date: endDate }),
    };
    const parsedBody = ownerBetaSimulationRunBodySchema.safeParse(candidate);
    if (!parsedBody.success) {
      setError(t.invalidDateMessage);
      return;
    }

    const fingerprint = requestFingerprint(parsedBody.data);
    const current = idempotencyRef.current;
    const idempotencyKey = current?.fingerprint === fingerprint ? current.key : crypto.randomUUID();
    idempotencyRef.current = { fingerprint, key: idempotencyKey };
    const request = new AbortController();
    requestRef.current = request;
    setState("submitting");
    setError(null);
    try {
      const response = await mutateWithCsrf(OWNER_BETA_SIMULATION_RUNS_PATH, {
        idempotencyKey,
        json: parsedBody.data,
        method: "POST",
        signal: request.signal,
      });
      const queued = await parseBrowserApiResponse(response, ownerBetaSimulationEnqueuedSchema, {
        signal: request.signal,
      });
      if (response.status !== 202) {
        throw new ApiContractError(response.status, "Simulation enqueue did not return 202");
      }
      if (request.signal.aborted) return;
      idempotencyRef.current = null;
      onQueued(queued, parsedBody.data);
    } catch (cause) {
      if (request.signal.aborted) return;
      if (isLoginRequiredError(cause)) {
        onAccessLoss("session");
      } else if (cause instanceof ApiProblem && cause.code === "INVALID_DATE") {
        setError(t.invalidDateMessage);
        return;
      } else if (
        cause instanceof ApiProblem &&
        (cause.code === "DATA_ENTITLEMENT_REQUIRED" ||
          cause.code === "DATASET_BLOCKED" ||
          cause.code === "DATA_STALE")
      ) {
        onAccessLoss("entitlement");
      } else if (cause instanceof ApiProblem && cause.code === "FORBIDDEN") {
        onAccessLoss("forbidden");
      }
      setError(t.genericErrorMessage);
    } finally {
      if (requestRef.current === request) requestRef.current = null;
      if (!request.signal.aborted) setState("idle");
    }
  }

  if (configs.length === 0) {
    return (
      <section aria-label={t.formSectionLabel} className="product-section">
        <StatePanel
          kind="blocked"
          message={t.noEligibleConfigMessage}
          title={t.noEligibleConfigTitle}
        />
      </section>
    );
  }

  return (
    <section aria-labelledby="owner-beta-simulation-form-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.formSectionLabel}</p>
          <h2 id="owner-beta-simulation-form-title">{t.formTitle}</h2>
        </div>
        <p>{t.formDescription}</p>
      </div>
      <form
        aria-label={t.formTitle}
        className="workflow-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void submit(event.currentTarget);
        }}
      >
        <label className="form-field">
          <span>{t.strategyConfigLabel}</span>
          <select defaultValue="" name="strategy_config_id" required>
            <option disabled value="">
              {t.selectConfigLabel}
            </option>
            {configs.map((config) => (
              <option key={config.id} value={config.id}>
                {t.configOptionLabel(config.createdAt.slice(0, 10))}
              </option>
            ))}
          </select>
        </label>
        <div className="field-grid">
          <label className="form-field">
            <span>
              {t.startDateLabel} <span className="supporting-copy">({t.optionalLabel})</span>
            </span>
            <input name="start_date" type="date" />
          </label>
          <label className="form-field">
            <span>
              {t.endDateLabel} <span className="supporting-copy">({t.optionalLabel})</span>
            </span>
            <input name="end_date" type="date" />
          </label>
        </div>
        <p className="supporting-copy">{t.defaultWindowExplanation}</p>
        <button className="primary-action" disabled={state === "submitting"} type="submit">
          {state === "submitting" ? t.submittingLabel : t.submitLabel}
        </button>
        {error === null ? null : (
          <p className="form-result" role="alert">
            {error}
          </p>
        )}
      </form>
    </section>
  );
}
