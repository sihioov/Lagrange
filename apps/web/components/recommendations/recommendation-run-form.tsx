"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import { recommendationRunSchema } from "@/lib/products/contracts";

type RunState = "error" | "idle" | "submitting" | "submitted";

export type RecommendationRunFormProps = {
  readonly defaultAsOf: string;
  readonly strategyConfigId: string;
};

export function RecommendationRunForm({
  defaultAsOf,
  strategyConfigId,
}: RecommendationRunFormProps) {
  const router = useRouter();
  const [state, setState] = useState<RunState>("idle");
  const [message, setMessage] = useState("");

  async function submit(form: HTMLFormElement): Promise<void> {
    const formData = new FormData(form);
    const asOf = formData.get("as_of");
    if (typeof asOf !== "string" || asOf === "") {
      setState("error");
      setMessage("As-of date is required.");
      return;
    }
    setState("submitting");
    try {
      const response = await mutateWithCsrf("/api/v1/recommendations/runs", {
        json: { as_of: asOf, strategy_config_id: strategyConfigId },
        method: "POST",
      });
      const run = await parseApiResponse(response, recommendationRunSchema);
      setState("submitted");
      setMessage(`Strategy proposal queued (${run.id}).`);
      router.refresh();
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
    <form
      aria-label="Generate recommendation"
      className="workflow-form"
      onSubmit={(event) => {
        event.preventDefault();
        void submit(event.currentTarget);
      }}
    >
      <label className="form-field">
        <span>As-of date</span>
        <input defaultValue={defaultAsOf} name="as_of" required type="date" />
      </label>
      <button className="primary-action" disabled={state === "submitting"} type="submit">
        {state === "submitting" ? "Generating strategy proposal" : "Generate strategy proposal"}
      </button>
      {state === "error" || state === "submitted" ? (
        <p className="form-result" role={state === "error" ? "alert" : "status"}>
          {message}
        </p>
      ) : null}
    </form>
  );
}
