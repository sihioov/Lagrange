"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
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

  async function send(target: "enable" | "disable"): Promise<void> {
    setState({ kind: "working" });
    try {
      const response = await mutateWithCsrf(`/api/v1/admin/live/kill-switch/${target}`, {
        json: { reason: reason.trim() || `${target}d from the Live controls page` },
        method: "POST",
      });
      const result = await parseApiResponse(response, killSwitchSchema);
      setState({
        kind: "done",
        message: result.engaged
          ? "Kill switch engaged. No Live node can start."
          : "Kill switch disengaged. Live nodes may now start.",
      });
      setReason("");
      router.refresh();
    } catch (error) {
      if (error instanceof Error) {
        // A step-up refusal is actionable; say what to do rather than echoing
        // a code the reader would have to look up.
        const code = (error as { code?: string }).code ?? "";
        setState({
          kind: "error",
          message: code.startsWith("STEP_UP_") ? liveUnavailableReason(code) : error.message,
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
          <p className="eyebrow">Safety</p>
          <h2 id="live-kill-switch-title">Kill switch</h2>
        </div>
        <p>{engaged ? "Engaged — Live is stopped" : "Disengaged — Live may run"}</p>
      </div>

      {engaged ? (
        <form
          aria-label="Disengage kill switch"
          className="workflow-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void send("disable");
          }}
        >
          <label className="form-field">
            <span>Reason for disengaging</span>
            <input
              name="reason"
              onChange={(event) => setReason(event.target.value)}
              required
              value={reason}
            />
          </label>
          <p className="supporting-copy">
            Disengaging permits Live nodes to start and place real orders. The reason is recorded in
            the audit trail.
          </p>
          <button
            className="primary-action"
            disabled={state.kind === "working" || reason.trim() === ""}
            type="submit"
          >
            {state.kind === "working" ? "Disengaging" : "Disengage kill switch"}
          </button>
        </form>
      ) : (
        <form
          aria-label="Engage kill switch"
          className="workflow-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void send("enable");
          }}
        >
          <p className="supporting-copy">
            Engaging stops Live immediately. No reason is required — refusing to stop trading
            because an operator has not explained themselves would be the wrong trade in the one
            moment it matters most.
          </p>
          <button className="primary-action" disabled={state.kind === "working"} type="submit">
            {state.kind === "working" ? "Engaging" : "Engage kill switch"}
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
