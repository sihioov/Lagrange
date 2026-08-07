"use client";

import { useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import {
  type ParameterDefinition,
  type ParameterSchema,
  strategyConfigSchema,
  type StrategyCatalogItem,
} from "@/lib/products/contracts";

type SubmissionState =
  | { readonly kind: "error"; readonly message: string }
  | { readonly kind: "idle" }
  | { readonly kind: "saved"; readonly message: string }
  | { readonly kind: "submitting" };

type ValidationResult =
  | { readonly kind: "invalid"; readonly message: string }
  | { readonly kind: "valid"; readonly value: unknown };

function invalid(message: string): ValidationResult {
  return { kind: "invalid", message };
}

function parseParameter(
  name: string,
  definition: ParameterDefinition,
  formData: FormData,
): ValidationResult {
  const raw = formData.get(name);
  switch (definition.type) {
    case "boolean":
      return { kind: "valid", value: raw === "on" };
    case "integer":
    case "number": {
      if (typeof raw !== "string" || raw.trim() === "") {
        return invalid(`${definition.title} is required.`);
      }
      const value = Number(raw);
      if (!Number.isFinite(value) || (definition.type === "integer" && !Number.isInteger(value))) {
        return invalid(`${definition.title} must be a valid ${definition.type}.`);
      }
      if (definition.minimum !== undefined && value < definition.minimum) {
        const maximum = definition.maximum ?? "the allowed maximum";
        return invalid(`${definition.title} must be between ${definition.minimum} and ${maximum}.`);
      }
      if (definition.maximum !== undefined && value > definition.maximum) {
        const minimum = definition.minimum ?? "the allowed minimum";
        return invalid(`${definition.title} must be between ${minimum} and ${definition.maximum}.`);
      }
      return { kind: "valid", value };
    }
    case "string":
      return typeof raw === "string" && raw.trim() !== ""
        ? { kind: "valid", value: raw.trim() }
        : invalid(`${definition.title} is required.`);
  }
}

function strategyConfigPath(strategyId: string): `/api/v1/strategies/${string}/configs` {
  return `/api/v1/strategies/${encodeURIComponent(strategyId)}/configs`;
}

export type StrategyConfigFormProps = {
  readonly strategy: StrategyCatalogItem;
};

export function StrategyConfigForm({ strategy }: StrategyConfigFormProps) {
  const [state, setState] = useState<SubmissionState>({ kind: "idle" });
  const schema = strategy.parameter_schema;
  const version = strategy.latest_version;
  if (schema === undefined || version === undefined || version === null) {
    return <p className="supporting-copy">No configurable parameter schema is available.</p>;
  }
  const activeSchema: ParameterSchema = schema;

  async function submit(form: HTMLFormElement): Promise<void> {
    const config: Record<string, unknown> = {};
    const formData = new FormData(form);
    for (const [name, definition] of Object.entries(activeSchema.properties)) {
      const result = parseParameter(name, definition, formData);
      if (result.kind === "invalid") {
        setState({ kind: "error", message: result.message });
        return;
      }
      config[name] = result.value;
    }
    setState({ kind: "submitting" });
    try {
      const response = await mutateWithCsrf(strategyConfigPath(strategy.id), {
        json: { config, is_active: true, strategy_version: version },
        method: "POST",
      });
      const saved = await parseApiResponse(response, strategyConfigSchema);
      setState({ kind: "saved", message: `Configuration saved (${saved.id}).` });
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
      aria-label={`Configure ${strategy.display_name}`}
      className="config-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void submit(event.currentTarget);
      }}
    >
      <fieldset disabled={state.kind === "submitting"}>
        <legend>Allowed parameters</legend>
        <div className="field-grid">
          {Object.entries(activeSchema.properties).map(([name, definition]) => (
            <label className="form-field" key={name}>
              <span>{definition.title}</span>
              {definition.enum === undefined ? (
                <input
                  defaultChecked={definition.type === "boolean" && definition.default === true}
                  defaultValue={definition.type === "boolean" ? undefined : String(definition.default ?? "")}
                  max={definition.maximum}
                  min={definition.minimum}
                  name={name}
                  required={activeSchema.required.includes(name) && definition.type !== "boolean"}
                  step={definition.type === "integer" ? 1 : "any"}
                  type={definition.type === "boolean" ? "checkbox" : definition.type === "string" ? "text" : "number"}
                />
              ) : (
                <select defaultValue={String(definition.default ?? definition.enum[0] ?? "")} name={name}>
                  {definition.enum.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </select>
              )}
              {definition.description === undefined ? null : <small>{definition.description}</small>}
            </label>
          ))}
        </div>
      </fieldset>
      <button className="primary-action" disabled={state.kind === "submitting"} type="submit">
        {state.kind === "submitting" ? "Saving configuration" : "Save strategy configuration"}
      </button>
      {state.kind === "error" || state.kind === "saved" ? (
        <p className="form-result" role={state.kind === "error" ? "alert" : "status"}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}
