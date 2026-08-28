"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { useLocale } from "@/lib/i18n/client";
import { liveDictionary } from "@/lib/i18n/dictionaries/live";
import { killSwitchSchema, liveUnavailableReason } from "@/lib/products/live-contracts";

type SwitchState =
  | { readonly kind: "done"; readonly message: string }
  | { readonly kind: "error"; readonly message: string }
  | { readonly kind: "idle" }
  | { readonly kind: "working" };

export type LiveKillSwitchProps = {
  readonly engaged: boolean;
};

/**
 * The Live kill switch.
 *
 * Engaging and disengaging are separate actions with separate confirmations,
 * not one toggle, because they are not symmetric. Engaging stops trading and
 * is always safe to do in a hurry — so it is a single click with no ceremony.
 * DISENGAGING permits real orders, so it asks for a typed reason first: the
 * reason lands in the audit trail, and the small friction is deliberate at the
 * one moment where an accidental click has the largest consequence.
 */
export function LiveKillSwitch({ engaged }: LiveKillSwitchProps) {
  const router = useRouter();
  const [state, setState] = useState<SwitchState>({ kind: "idle" });
  const [reason, setReason] = useState("");
  const { locale } = useLocale();
  const t = liveDictionary[locale];

  async function send(target: "enable" | "disable"): Promise<void> {
    setState({ kind: "working" });
    try {
      const response = await mutateWithCsrf(`/api/v1/admin/live/kill-switch/${target}`, {
        json: { reason: reason.trim() || `${target}d from the Live controls page` },
        method: "POST",
      });
      const result = await parseBrowserApiResponse(response, killSwitchSchema);
      setState({
        kind: "done",
        message: result.engaged ? t.engagedMessage : t.disengagedMessage,
      });
      setReason("");
      router.refresh();
    } catch (error) {
      if (error instanceof Error) {
        // A step-up or reconciliation refusal is actionable; say what to do
        // rather than echoing a code the reader would have to look up. Every
        // other error keeps the server's own message, which is more specific
        // than anything this component could invent.
        const code = (error as { code?: string }).code ?? "";
        const actionable = code.startsWith("STEP_UP_") || code === "LIVE_RECONCILIATION_REQUIRED";
        setState({
          kind: "error",
          message: actionable ? liveUnavailableReason(code) : error.message,
        });
        return;
      }
      throw error;
    }
  }

  return (
    <section aria-labelledby="live-kill-switch-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.safetyEyebrow}</p>
          <h2 id="live-kill-switch-title">{t.killSwitchTitle}</h2>
        </div>
        <p>{engaged ? t.engagedStatus : t.disengagedStatus}</p>
      </div>

      {engaged ? (
        <form
          aria-label={t.disengageAriaLabel}
          className="workflow-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void send("disable");
          }}
        >
          <label className="form-field">
            <span>{t.reasonForDisengagingLabel}</span>
            <input
              name="reason"
              onChange={(event) => setReason(event.target.value)}
              required
              value={reason}
            />
          </label>
          <p className="supporting-copy">{t.disengagingSupportingCopy}</p>
          <button
            className="caution-action"
            disabled={state.kind === "working" || reason.trim() === ""}
            type="submit"
          >
            {state.kind === "working" ? t.disengageButtonDisengaging : t.disengageButtonDisengage}
          </button>
        </form>
      ) : (
        <form
          aria-label={t.engageAriaLabel}
          className="workflow-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void send("enable");
          }}
        >
          <p className="supporting-copy">{t.engagingSupportingCopy}</p>
          <button className="primary-action" disabled={state.kind === "working"} type="submit">
            {state.kind === "working" ? t.engageButtonEngaging : t.engageButtonEngage}
          </button>
        </form>
      )}

      {state.kind === "error" || state.kind === "done" ? (
        <p className="form-result" role={state.kind === "error" ? "alert" : "status"}>
          {state.message}
        </p>
      ) : null}
    </section>
  );
}
