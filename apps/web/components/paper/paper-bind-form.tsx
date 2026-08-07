"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import { bindStrategySchema } from "@/lib/products/paper-contracts";

type BindState =
  | { readonly kind: "bound"; readonly message: string }
  | { readonly kind: "error"; readonly message: string }
  | { readonly kind: "idle" }
  | { readonly kind: "submitting" };

export type PaperBindFormProps = {
  readonly accountId: string;
  readonly activeStrategy: string | null;
  readonly configs: readonly { readonly id: string; readonly label: string }[];
};

/**
 * Rebinding an account's strategy — the account-branching control.
 *
 * The server closes the previous binding and opens a new one atomically, so
 * this form is a branch, not an edit: prior sessions keep the strategy
 * version they actually ran, and the lineage table below shows the branch.
 * That is why the copy says "branch", and why the form does not offer to
 * "change" the strategy of past sessions.
 */
export function PaperBindForm({ accountId, activeStrategy, configs }: PaperBindFormProps) {
  const router = useRouter();
  const [state, setState] = useState<BindState>({ kind: "idle" });

  async function submit(form: HTMLFormElement): Promise<void> {
    const configId = new FormData(form).get("strategy_config_id");
    if (typeof configId !== "string" || configId === "") {
      setState({ kind: "error", message: "Select a strategy configuration to bind." });
      return;
    }
    setState({ kind: "submitting" });
    try {
      const response = await mutateWithCsrf(
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/bind-strategy`,
        { json: { strategy_config_id: configId }, method: "POST" },
      );
      const bound = await parseApiResponse(response, bindStrategySchema);
      setState({
        kind: "bound",
        message: `Bound ${bound.strategy_id}@${bound.strategy_version}. Sessions from the next close run on this version; earlier sessions keep theirs.`,
      });
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
      aria-label="Bind strategy"
      className="workflow-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void submit(event.currentTarget);
      }}
    >
      <label className="form-field">
        <span>Strategy configuration</span>
        <select defaultValue={configs[0]?.id ?? ""} name="strategy_config_id" required>
          {configs.map((config) => (
            <option key={config.id} value={config.id}>
              {config.label}
            </option>
          ))}
        </select>
      </label>
      <p className="supporting-copy">
        {activeStrategy === null
          ? "This account has no active binding yet."
          : `Currently bound to ${activeStrategy}.`}{" "}
        Binding a different configuration branches the account: the current binding is closed and a
        new one opens, so execution history never mixes strategy versions.
      </p>
      <button className="primary-action" disabled={state.kind === "submitting"} type="submit">
        {state.kind === "submitting" ? "Binding strategy" : "Bind strategy"}
      </button>
      {state.kind === "error" || state.kind === "bound" ? (
        <p className="form-result" role={state.kind === "error" ? "alert" : "status"}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}
