import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { stockBetaDashboardArchitecture } from "@/components/stock-beta/dashboard/widget-registry";
import { stockBetaDetailArchitecture } from "@/components/stock-beta/detail/widget-registry";
import {
  formatStockBetaNumber,
  formatStockBetaPercent,
  InvalidStockBetaNumericValue,
} from "@/components/stock-beta/shared/formatters";
import { WidgetFrame } from "@/components/stock-beta/shared/widget-frame";
import {
  defineStockBetaWidgetArchitecture,
  defineStockBetaWidgetCatalog,
  InvalidStockBetaWidgetArchitecture,
  stockBetaWidgetConfiguration,
  validateStockBetaWidgetArchitecture,
  validateStockBetaWidgetCatalog,
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

const defineExampleCatalog = defineStockBetaWidgetCatalog<ExampleViewModel>();
const requiredPlacements = {
  desktop: { size: "full", visible: true },
  tablet: { size: "full", visible: true },
  mobile: { size: "full", visible: true },
} as const;
const optionalPlacements = {
  desktop: { size: "small", visible: true },
  tablet: { size: "small", visible: true },
  mobile: { size: "small", visible: true },
} as const;
const validCatalog = defineExampleCatalog([
  {
    id: "ranked-signals",
    component: ExampleWidget,
    required: true,
    placements: requiredPlacements,
  },
  {
    id: "top-five",
    component: ExampleWidget,
    required: false,
    placements: optionalPlacements,
  },
]);
const validArchitecture = defineStockBetaWidgetArchitecture(validCatalog);

const validGridCatalog = defineExampleCatalog([
  {
    id: "ranked-signals",
    component: ExampleWidget,
    required: true,
    placements: {
      desktop: {
        size: "full",
        visible: true,
        column: 1,
        columnSpan: 8,
        row: 1,
        empty: { column: 1, columnSpan: 12, row: 1, visible: true },
      },
      tablet: {
        size: "full",
        visible: true,
        column: 1,
        columnSpan: 8,
        row: 1,
        empty: { column: 1, columnSpan: 12, row: 1, visible: true },
      },
      mobile: {
        size: "full",
        visible: true,
        column: 1,
        columnSpan: 1,
        row: 1,
        empty: { column: 1, columnSpan: 1, row: 1, visible: true },
      },
    },
  },
  {
    id: "top-five",
    component: ExampleWidget,
    required: false,
    placements: {
      desktop: {
        size: "small",
        visible: true,
        column: 9,
        columnSpan: 4,
        row: 1,
        empty: { column: 1, columnSpan: 12, row: 2, visible: true },
      },
      tablet: {
        size: "small",
        visible: true,
        column: 9,
        columnSpan: 4,
        row: 1,
        empty: { column: 1, columnSpan: 12, row: 2, visible: true },
      },
      mobile: {
        size: "small",
        visible: true,
        column: 1,
        columnSpan: 1,
        row: 2,
        empty: { column: 1, columnSpan: 1, row: 2, visible: true },
      },
    },
  },
]);
const validGridArchitecture = defineStockBetaWidgetArchitecture(validGridCatalog);

describe("stock-beta widget architecture", () => {
  it("derives required IDs and breakpoint layouts from typed catalog entries", () => {
    expect(validateStockBetaWidgetArchitecture(validArchitecture)).toEqual([]);
    expect(validArchitecture.requiredWidgetIds).toEqual(["ranked-signals"]);
    expect(validArchitecture.layout.desktop).toEqual([
      { id: "ranked-signals", size: "full", visible: true },
      { id: "top-five", size: "small", visible: true },
    ]);
  });

  it("rejects architecture views that drift from their catalog", () => {
    expect(
      validateStockBetaWidgetArchitecture({
        ...validArchitecture,
        requiredWidgetIds: [],
        layout: { ...validArchitecture.layout, mobile: [] },
      }),
    ).toEqual([
      { code: "invalid-architecture", path: "requiredWidgetIds" },
      { code: "invalid-layout", path: "layout.mobile" },
    ]);
  });

  it("rejects unknown derived layout keys and fails closed before projection", () => {
    const invalidArchitecture = {
      ...validArchitecture,
      layout: { ...validArchitecture.layout, future: [] },
    };

    expect(validateStockBetaWidgetArchitecture(invalidArchitecture)).toContainEqual({
      code: "invalid-layout",
      path: "layout.future",
    });
    expect(() => stockBetaWidgetConfiguration(invalidArchitecture)).toThrow(
      InvalidStockBetaWidgetArchitecture,
    );
  });

  it("projects validated layout configuration without runtime component functions", () => {
    const configuration = stockBetaWidgetConfiguration(validArchitecture);
    const roundTrip = JSON.parse(JSON.stringify(configuration)) as typeof configuration;

    expect(roundTrip).toEqual(configuration);
    expect(roundTrip.widgets).toEqual([
      { id: "ranked-signals", required: true },
      { id: "top-five", required: false },
    ]);
    expect(JSON.stringify(configuration)).not.toContain("component");
  });

  it("rejects duplicate widget IDs", () => {
    const invalid = [validCatalog[0], { ...validCatalog[1], id: "ranked-signals" }];

    expect(validateStockBetaWidgetCatalog(invalid)).toContainEqual({
      code: "duplicate-definition-id",
      path: "catalog",
    });
  });

  it("rejects a required widget missing from any breakpoint", () => {
    const invalid = [
      {
        ...validCatalog[0],
        placements: { desktop: requiredPlacements.desktop, tablet: requiredPlacements.tablet },
      },
      validCatalog[1],
    ];

    expect(validateStockBetaWidgetCatalog(invalid)).toContainEqual({
      code: "missing-required-widget",
      path: "catalog[0].placements.mobile",
    });
    expect(() => defineExampleCatalog(invalid)).toThrow(InvalidStockBetaWidgetArchitecture);
  });

  it("rejects hidden required widgets", () => {
    const invalid = [
      {
        ...validCatalog[0],
        placements: {
          ...validCatalog[0].placements,
          tablet: { ...validCatalog[0].placements.tablet, visible: false },
        },
      },
      validCatalog[1],
    ];
    const issues = validateStockBetaWidgetCatalog(invalid);

    expect(issues).toContainEqual({
      code: "required-widget-hidden",
      path: "catalog[0].placements.tablet",
    });
  });

  it("rejects unsupported sizes", () => {
    const invalid = [
      validCatalog[0],
      {
        ...validCatalog[1],
        placements: {
          ...validCatalog[1].placements,
          desktop: { size: "enormous", visible: true },
        },
      },
    ];
    const issues = validateStockBetaWidgetCatalog(invalid);

    expect(issues).toContainEqual({
      code: "invalid-size",
      path: "catalog[1].placements.desktop.size",
    });
  });

  it("accepts complete grid placement metadata for populated and empty states", () => {
    expect(validateStockBetaWidgetArchitecture(validGridArchitecture)).toEqual([]);
  });

  it("rejects incomplete grid placement metadata", () => {
    const invalid = validGridCatalog.map((entry, index) =>
      index === 0
        ? {
            ...entry,
            placements: {
              ...entry.placements,
              desktop: { ...entry.placements.desktop, empty: undefined },
            },
          }
        : entry,
    );

    expect(validateStockBetaWidgetCatalog(invalid)).toContainEqual({
      code: "invalid-layout",
      path: "catalog[0].placements.desktop.empty",
    });
  });

  it("rejects out-of-range grid coordinates and spans", () => {
    const invalid = validGridCatalog.map((entry, index) => {
      if (index === 0) {
        return {
          ...entry,
          placements: {
            ...entry.placements,
            tablet: { ...entry.placements.tablet, column: 0 },
            mobile: {
              ...entry.placements.mobile,
              empty: { ...entry.placements.mobile.empty, row: 0 },
            },
          },
        };
      }
      return {
        ...entry,
        placements: {
          ...entry.placements,
          desktop: { ...entry.placements.desktop, column: 12, columnSpan: 2 },
        },
      };
    });
    const issues = validateStockBetaWidgetCatalog(invalid);

    expect(issues).toContainEqual({
      code: "invalid-grid-column-span",
      path: "catalog[1].placements.desktop.columnSpan",
    });
    expect(issues).toContainEqual({
      code: "invalid-grid-column",
      path: "catalog[0].placements.tablet.column",
    });
    expect(issues).toContainEqual({
      code: "invalid-grid-row",
      path: "catalog[0].placements.mobile.empty.row",
    });
  });

  it("rejects overlapping visible grid cells", () => {
    const invalid = [
      validGridCatalog[0],
      {
        ...validGridCatalog[1],
        placements: {
          ...validGridCatalog[1].placements,
          desktop: { ...validGridCatalog[1].placements.desktop, column: 1, columnSpan: 4 },
        },
      },
    ];
    const issues = validateStockBetaWidgetCatalog(invalid);

    expect(issues).toContainEqual({
      code: "overlapping-layout-placement",
      path: "layout.desktop.populated",
    });
  });

  it("keeps the accepted V2 dashboard and detail registries valid", () => {
    expect(validateStockBetaWidgetArchitecture(stockBetaDashboardArchitecture)).toEqual([]);
    expect(validateStockBetaWidgetArchitecture(stockBetaDetailArchitecture)).toEqual([]);
    expect(stockBetaDashboardArchitecture.requiredWidgetIds).not.toContain("signal-state");
    expect(stockBetaDetailArchitecture.requiredWidgetIds).toContain("snapshot");
    const detailCatalogIds = stockBetaDetailArchitecture.catalog.map((entry) => entry.id);
    expect(stockBetaDetailArchitecture.requiredWidgetIds).toEqual(
      detailCatalogIds.filter((id) => id !== "current-quote"),
    );
    for (const breakpoint of ["desktop", "tablet", "mobile"] as const) {
      expect(
        stockBetaDetailArchitecture.layout[breakpoint].map((placement) => placement.id),
      ).toEqual(detailCatalogIds);
    }
  });

  it("projects the production registries to configuration without component functions", () => {
    const dashboardConfiguration = stockBetaWidgetConfiguration(stockBetaDashboardArchitecture);
    const detailConfiguration = stockBetaWidgetConfiguration(stockBetaDetailArchitecture);
    for (const configuration of [dashboardConfiguration, detailConfiguration]) {
      expect(JSON.parse(JSON.stringify(configuration))).toEqual(configuration);
      expect(JSON.stringify(configuration)).not.toContain("component");
      expect(configuration.widgets.every((widget) => !("component" in widget))).toBe(true);
    }
  });

  it("keeps dashboard widgets data-bound through props rather than direct V2 fetches", () => {
    const widgetRoot = join(process.cwd(), "components/stock-beta/dashboard/widgets");
    const widgetFiles = readdirSync(widgetRoot, { withFileTypes: true }).filter(
      (entry) => entry.isFile() && entry.name.endsWith(".tsx"),
    );

    for (const entry of widgetFiles) {
      const source = readFileSync(join(widgetRoot, entry.name), "utf8");
      expect(source).not.toMatch(/\bfetch\s*\(/);
      expect(source).not.toMatch(/getOwnerEquityV2|screenOwnerEquityV2/);
    }
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
