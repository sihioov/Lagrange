"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { useLocale } from "@/lib/i18n/client";
import { backtestsDictionary } from "@/lib/i18n/dictionaries/backtests";
import { backtestCreateSchema } from "@/lib/products/backtest-contracts";

type CreateState =
  | { readonly kind: "error"; readonly message: string }
  | { readonly kind: "idle" }
  | { readonly kind: "queued"; readonly message: string }
  | { readonly kind: "submitting" };

export type BacktestCreateFormProps = {
  readonly benchmark: string;
  readonly costProfileId: string;
  readonly datasetVersionId: string;
  readonly executionProfile: string;
  readonly strategyConfigId: string;
};

export function BacktestCreateForm(props: BacktestCreateFormProps) {
  const router = useRouter();
  const [state, setState] = useState<CreateState>({ kind: "idle" });
  const { locale } = useLocale();
  const t = backtestsDictionary[locale];

  async function submit(form: HTMLFormElement): Promise<void> {
    const data = new FormData(form);
    const startDate = data.get("start_date");
    const endDate = data.get("end_date");
    const initialCash = data.get("initial_cash");
    if (
      typeof startDate !== "string" ||
      typeof endDate !== "string" ||
      typeof initialCash !== "string" ||
      startDate === "" ||
      endDate === "" ||
      !/^\d+(?:\.\d+)?$/.test(initialCash) ||
      startDate > endDate
    ) {
      setState({
        kind: "error",
        message: t.invalidDateRangeMessage,
      });
      return;
    }
    setState({ kind: "submitting" });
    try {
      const response = await mutateWithCsrf("/api/v1/backtests", {
        json: {
          benchmark: props.benchmark,
          cost_profile_id: props.costProfileId,
          dataset_version_id: props.datasetVersionId,
          end_date: endDate,
          execution_profile: props.executionProfile,
          initial_cash: { amount: initialCash, currency: "KRW" },
          robustness: false,
          start_date: startDate,
          strategy_config_id: props.strategyConfigId,
        },
        method: "POST",
      });
      const created = await parseBrowserApiResponse(response, backtestCreateSchema);
      setState({ kind: "queued", message: t.backtestQueuedMessage(created.id) });
      router.refresh();
    } catch (error) {
      if (error instanceof Error) {
        setState({ kind: "error", message: error.message });
        return;
      }
      throw error;
    }
  }

  return (
    <form
      aria-label={t.createBacktestLabel}
      className="workflow-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void submit(event.currentTarget);
      }}
    >
      <div className="field-grid">
        <label className="form-field">
          <span>{t.startDateLabel}</span>
          <input defaultValue="2020-01-02" name="start_date" required type="date" />
        </label>
        <label className="form-field">
          <span>{t.endDateLabel}</span>
          <input defaultValue="2025-12-31" name="end_date" required type="date" />
        </label>
        <label className="form-field">
          <span>{t.initialCashLabel}</span>
          <input defaultValue="100000000" inputMode="decimal" name="initial_cash" required />
        </label>
      </div>
      <p className="supporting-copy">
        {t.supportingCopy(props.executionProfile, props.costProfileId, props.benchmark)}
      </p>
      <button className="primary-action" disabled={state.kind === "submitting"} type="submit">
        {state.kind === "submitting" ? t.creatingBacktestLabel : t.createBacktestLabel}
      </button>
      {state.kind === "error" || state.kind === "queued" ? (
        <p className="form-result" role={state.kind === "error" ? "alert" : "status"}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}
