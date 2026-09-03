import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  formatStockBetaNumber,
  formatStockBetaPercent,
  InvalidStockBetaNumericValue,
} from "@/components/stock-beta/shared/formatters";
import { WidgetFrame } from "@/components/stock-beta/shared/widget-frame";
import {
  defineStockBetaWidget,
  defineStockBetaWidgetArchitecture,
  InvalidStockBetaWidgetArchitecture,
  type StockBetaWidgetArchitecture,
  type StockBetaWidgetPlacement,
  stockBetaWidgetConfiguration,
  validateStockBetaWidgetArchitecture,
} from "@/components/stock-beta/shared/widget-types";
import {
  StockBetaTerminalPage,
  StockBetaTerminalUtilityHost,
  StockBetaTerminalUtilityHostProvider,
} from "@/components/stock-beta/terminal";

type ExampleViewModel = { readonly value: number };

function ExampleWidget({ viewModel }: { readonly viewModel: ExampleViewModel }) {
  return <p>{viewModel.value}</p>;
}

const requiredDefinition = defineStockBetaWidget({
  id: "ranked-signals",
  component: ExampleWidget,
  defaultSize: "full",
  required: true,
  defaultVisible: true,
  order: 0,
});

const optionalDefinition = defineStockBetaWidget({
  id: "top-five",
  component: ExampleWidget,
  defaultSize: "small",
  required: false,
  defaultVisible: true,
  order: 1,
});

const basePlacements = [
  { id: "ranked-signals", size: "full", visible: true, order: 0 },
  { id: "top-five", size: "small", visible: true, order: 1 },
] as const satisfies readonly StockBetaWidgetPlacement<"ranked-signals" | "top-five">[];

const validArchitecture = {
  definitions: [requiredDefinition, optionalDefinition],
  requiredWidgetIds: ["ranked-signals"],
  layout: {
    desktop: basePlacements,
    tablet: basePlacements,
    mobile: basePlacements,
  },
} as const satisfies StockBetaWidgetArchitecture<
  readonly [typeof requiredDefinition, typeof optionalDefinition]
>;

describe("stock-beta widget architecture", () => {
  it("accepts typed definitions and complete breakpoint layouts", () => {
    expect(defineStockBetaWidgetArchitecture(validArchitecture)).toBe(validArchitecture);
    expect(validateStockBetaWidgetArchitecture(validArchitecture)).toEqual([]);
  });

  it("projects validated layout configuration without runtime component functions", () => {
    const configuration = stockBetaWidgetConfiguration(validArchitecture);
    const roundTrip = JSON.parse(JSON.stringify(configuration)) as typeof configuration;

    expect(roundTrip).toEqual(configuration);
    expect(roundTrip.definitions[0]).not.toHaveProperty("component");
    expect(roundTrip.layout.desktop).toEqual(basePlacements);
  });

  it("rejects duplicate widget IDs", () => {
    const invalid = {
      ...validArchitecture,
      definitions: [requiredDefinition, { ...optionalDefinition, id: "ranked-signals" }],
    };

    expect(validateStockBetaWidgetArchitecture(invalid)).toContainEqual({
      code: "duplicate-definition-id",
      path: "definitions",
    });
  });

  it("rejects a required widget missing from any layout", () => {
    const invalid = {
      ...validArchitecture,
      layout: { ...validArchitecture.layout, mobile: [basePlacements[1]] },
    };

    expect(validateStockBetaWidgetArchitecture(invalid)).toContainEqual({
      code: "missing-required-widget",
      path: "layout.mobile.ranked-signals",
    });
    expect(() => defineStockBetaWidgetArchitecture(invalid)).toThrow(
      InvalidStockBetaWidgetArchitecture,
    );
  });

  it("rejects a required policy ID missing from the registry", () => {
    const invalid = {
      ...validArchitecture,
      definitions: [optionalDefinition],
    };

    expect(validateStockBetaWidgetArchitecture(invalid)).toContainEqual({
      code: "missing-required-widget",
      path: "requiredWidgetIds.ranked-signals",
    });
  });

  it("rejects required-policy drift and hidden required widgets", () => {
    const invalid = {
      ...validArchitecture,
      definitions: [{ ...requiredDefinition, required: false }, optionalDefinition],
      layout: {
        ...validArchitecture.layout,
        tablet: [{ ...basePlacements[0], visible: false }, basePlacements[1]],
      },
    };
    const issues = validateStockBetaWidgetArchitecture(invalid);

    expect(issues).toContainEqual({
      code: "required-widget-not-required",
      path: "definitions.ranked-signals.required",
    });
    expect(issues).toContainEqual({
      code: "required-widget-hidden",
      path: "layout.tablet.ranked-signals",
    });
  });

  it("rejects unsupported sizes and ambiguous layout order", () => {
    const invalid = {
      ...validArchitecture,
      layout: {
        ...validArchitecture.layout,
        desktop: [basePlacements[0], { ...basePlacements[1], order: 0, size: "enormous" }],
      },
    };
    const issues = validateStockBetaWidgetArchitecture(invalid);

    expect(issues).toContainEqual({
      code: "invalid-size",
      path: "layout.desktop[1].size",
    });
    expect(issues).toContainEqual({
      code: "duplicate-layout-order",
      path: "layout.desktop",
    });
  });
});

describe("WidgetFrame", () => {
  it("renders a labelled semantic region with heading and status slots", () => {
    const markup = renderToStaticMarkup(
      <WidgetFrame status={<span>Approved snapshot</span>} title="Ranked signals">
        <p>Verified rows</p>
      </WidgetFrame>,
    );

    const headingId = /<h2[^>]*id="([^"]+)"/.exec(markup)?.[1];
    expect(headingId).toBeDefined();
    expect(markup).toContain(`aria-labelledby="${headingId}"`);
    expect(markup).toContain("Approved snapshot");
    expect(markup).toContain("Verified rows");
    expect(markup).toContain('data-state="ready"');
  });

  it("announces loading politely and suppresses stale children", () => {
    const markup = renderToStaticMarkup(
      <WidgetFrame state={{ kind: "loading", message: "Loading approved data." }} title="Signals">
        <p>Stale row must not render</p>
      </WidgetFrame>,
    );

    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain('role="status"');
    expect(markup).not.toContain("Stale row must not render");
  });

  it("announces errors assertively and suppresses unverified children", () => {
    const markup = renderToStaticMarkup(
      <WidgetFrame state={{ kind: "error", message: "Snapshot integrity failed." }} title="Signals">
        <p>Unverified row must not render</p>
      </WidgetFrame>,
    );

    expect(markup).toContain('aria-live="assertive"');
    expect(markup).toContain('role="alert"');
    expect(markup).not.toContain("Unverified row must not render");
  });
});

describe("StockBetaTerminalPage", () => {
  it("targets the route-scoped header host without rendering a second utility row", () => {
    const markup = renderToStaticMarkup(
      <StockBetaTerminalUtilityHostProvider>
        <header data-terminal-utility-bar="stock-beta">
          <StockBetaTerminalUtilityHost />
        </header>
        <main>
          <StockBetaTerminalPage
            asOf={<span>AS OF 2026-09-01</span>}
            search={
              <label>
                Instrument search
                <input type="search" />
              </label>
            }
            snapshot={
              <dl>
                <dt>As of</dt>
                <dd>2026-09-01</dd>
              </dl>
            }
            title="KR Equity Signal Board"
            titleTools={<button type="button">Filters</button>}
          >
            <p>Widget grid</p>
          </StockBetaTerminalPage>
        </main>
      </StockBetaTerminalUtilityHostProvider>,
    );

    const header =
      /<header[^>]*data-terminal-utility-bar="stock-beta"[^>]*>([\s\S]*?)<\/header>/.exec(
        markup,
      )?.[1];
    expect(header).toContain('data-terminal-utility-host="stock-beta"');
    expect(markup).not.toContain('data-terminal-utility-content="stock-beta"');
    expect(markup).not.toContain("pageUtility");
    expect(markup).toContain('data-terminal-slot="snapshot"');
    expect(markup).toContain('data-terminal-slot="title-tools"');
    expect(markup.match(/<h1/g)).toHaveLength(1);
    expect(markup.match(/<main/g)).toHaveLength(1);
    expect(markup).toContain("Widget grid");
  });
});

describe("stock-beta numeric presentation", () => {
  it("keeps the exact finite DTO value beside localized display text", () => {
    const number = formatStockBetaNumber(1234.5678, "en", { fractionDigits: 2 });
    const percent = formatStockBetaPercent(-0.12345, "ko", 2);

    expect(number).toEqual({ rawValue: 1234.5678, text: "1,234.57" });
    expect(percent.rawValue).toBe(-0.12345);
    expect(percent.text).toContain("12.35%");
  });

  it("fails closed for non-finite values and unsupported precision", () => {
    expect(() => formatStockBetaNumber(Number.NaN, "en")).toThrow(InvalidStockBetaNumericValue);
    expect(() => formatStockBetaPercent(Number.POSITIVE_INFINITY, "ko")).toThrow(
      InvalidStockBetaNumericValue,
    );
    expect(() => formatStockBetaNumber(1, "en", { fractionDigits: 13 })).toThrow(
      InvalidStockBetaNumericValue,
    );
  });
});
