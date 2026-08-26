import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { StrategyCatalog } from "@/components/strategies/strategy-catalog";
import { LocaleProvider } from "@/lib/i18n/client";
import { strategiesDictionary } from "@/lib/i18n/dictionaries/strategies";
import { strategySchema } from "@/lib/products/contracts";

describe("baseline strategy catalog contract", () => {
  it("accepts the JSON Schema features used by baseline packages and renders code defaults", () => {
    const strategy = strategySchema.parse({
      default_parameters: {
        benchmark_instrument: "069500.KRX",
        lookback_months: 12,
        target_weight: 1,
      },
      description: "Immutable baseline strategy",
      display_name: "Baseline",
      id: "baseline",
      latest_version: "1.0.0",
      parameter_schema: {
        $schema: "https://json-schema.org/draft/2020-12/schema",
        additionalProperties: false,
        properties: {
          benchmark_instrument: {
            pattern: "^[0-9]{6}\\.KRX$",
            type: "string",
          },
          lookback_months: {
            enum: [6, 12],
            type: "integer",
          },
          target_weight: {
            exclusiveMinimum: 0,
            maximum: 1,
            type: "number",
          },
        },
        required: ["benchmark_instrument", "lookback_months", "target_weight"],
        type: "object",
      },
      risk_description: "Baseline risk",
      state: "Draft",
    });

    const markup = renderToStaticMarkup(
      <LocaleProvider initialLocale="en">
        <StrategyCatalog canConfigure strategies={[strategy]} t={strategiesDictionary.en} />
      </LocaleProvider>,
    );

    expect(markup).toContain("Benchmark Instrument");
    expect(markup).toContain('pattern="^[0-9]{6}\\.KRX$"');
    expect(markup).toContain('value="069500.KRX"');
    expect(markup).toContain('<option value="12" selected="">12</option>');
    expect(markup).toContain('value="1"');
    expect(markup).toContain("Save strategy configuration");
    expect(markup).not.toContain("No configurable parameter schema is available");
  });
});
