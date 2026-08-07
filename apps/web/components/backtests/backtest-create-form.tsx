"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
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
        message: "Enter a valid date range and a positive KRW amount.",
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
      const created = await parseApiResponse(response, backtestCreateSchema);
      setState({ kind: "queued", message: `Backtest queued (${created.id}).` });
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
      aria-label="Create backtest"
      className="workflow-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void submit(event.currentTarget);
      }}
    >
      <div className="field-grid">
        <label className="form-field">
          <span>Start date</span>
          <input defaultValue="2020-01-02" name="start_date" required type="date" />
        </label>
        <label className="form-field">
          <span>End date</span>
          <input defaultValue="2025-12-31" name="end_date" required type="date" />
        </label>
        <label className="form-field">
          <span>Initial cash (KRW)</span>
          <input defaultValue="100000000" inputMode="decimal" name="initial_cash" required />
        </label>
      </div>
      <p className="supporting-copy">
        The server applies {props.executionProfile}, {props.costProfileId}, and benchmark {props.benchmark}.
      </p>
      <button className="primary-action" disabled={state.kind === "submitting"} type="submit">
        {state.kind === "submitting" ? "Creating backtest" : "Create backtest"}
      </button>
      {state.kind === "error" || state.kind === "queued" ? (
        <p className="form-result" role={state.kind === "error" ? "alert" : "status"}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}
