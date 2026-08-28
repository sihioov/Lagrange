"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { useLocale } from "@/lib/i18n/client";
import { paperDictionary } from "@/lib/i18n/dictionaries/paper";
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
  const { locale } = useLocale();
  const t = paperDictionary[locale];

  async function submit(form: HTMLFormElement): Promise<void> {
    const configId = new FormData(form).get("strategy_config_id");
    if (typeof configId !== "string" || configId === "") {
      setState({ kind: "error", message: t.bindFormSelectConfigError });
      return;
    }
    setState({ kind: "submitting" });
    try {
      const response = await mutateWithCsrf(
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/bind-strategy`,
        { json: { strategy_config_id: configId }, method: "POST" },
      );
      const bound = await parseBrowserApiResponse(response, bindStrategySchema);
      setState({
        kind: "bound",
        message: t.bindFormBoundMessage(bound.strategy_id, bound.strategy_version),
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
      aria-label={t.bindFormAriaLabel}
      className="workflow-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void submit(event.currentTarget);
      }}
    >
      <label className="form-field">
        <span>{t.bindFormStrategyConfigLabel}</span>
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
          ? t.bindFormNoActiveBinding
          : t.bindFormCurrentlyBoundTo(activeStrategy)}{" "}
        {t.bindFormBranchExplanation}
      </p>
      <button className="primary-action" disabled={state.kind === "submitting"} type="submit">
        {state.kind === "submitting" ? t.bindFormButtonBinding : t.bindFormButtonBind}
      </button>
      {state.kind === "error" || state.kind === "bound" ? (
        <p className="form-result" role={state.kind === "error" ? "alert" : "status"}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}
