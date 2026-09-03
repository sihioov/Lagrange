import type { ComponentType } from "react";

export const STOCK_BETA_WIDGET_SIZES = ["small", "medium", "large", "full"] as const;

export type StockBetaWidgetSize = (typeof STOCK_BETA_WIDGET_SIZES)[number];

export const STOCK_BETA_WIDGET_BREAKPOINTS = ["desktop", "tablet", "mobile"] as const;

export type StockBetaWidgetBreakpoint = (typeof STOCK_BETA_WIDGET_BREAKPOINTS)[number];

export type StockBetaWidgetProps<ViewModel> = {
  readonly viewModel: ViewModel;
};

export type StockBetaWidgetDefinition<Id extends string, ViewModel> = {
  readonly id: Id;
  readonly component: ComponentType<StockBetaWidgetProps<ViewModel>>;
  readonly defaultSize: StockBetaWidgetSize;
  readonly required: boolean;
  readonly defaultVisible: boolean;
  readonly order: number;
};

export type StockBetaWidgetPlacement<Id extends string> = {
  readonly id: Id;
  readonly size: StockBetaWidgetSize;
  readonly visible: boolean;
  readonly order: number;
};

export type StockBetaWidgetGridState = {
  readonly column: number;
  readonly columnSpan: number;
  readonly row: number;
  readonly visible: boolean;
  readonly order: number;
};

export type StockBetaWidgetGridPlacement<Id extends string> = StockBetaWidgetPlacement<Id> &
  Omit<StockBetaWidgetGridState, "visible" | "order"> & {
    readonly empty: StockBetaWidgetGridState;
  };

export type StockBetaWidgetLayout<Id extends string> = Readonly<
  Record<StockBetaWidgetBreakpoint, readonly StockBetaWidgetPlacement<Id>[]>
>;

export type StockBetaWidgetGridLayout<Id extends string> = Readonly<
  Record<StockBetaWidgetBreakpoint, readonly StockBetaWidgetGridPlacement<Id>[]>
>;

export type StockBetaWidgetDefinitionMetadata = {
  readonly id: string;
  readonly component: unknown;
  readonly defaultSize: StockBetaWidgetSize;
  readonly required: boolean;
  readonly defaultVisible: boolean;
  readonly order: number;
};

export type StockBetaWidgetArchitecture<
  Definitions extends readonly StockBetaWidgetDefinitionMetadata[],
  Layout extends StockBetaWidgetLayout<Definitions[number]["id"]> = StockBetaWidgetLayout<
    Definitions[number]["id"]
  >,
> = {
  readonly definitions: Definitions;
  readonly requiredWidgetIds: readonly Definitions[number]["id"][];
  readonly layout: Layout;
};

export type StockBetaWidgetDefinitionConfiguration<Id extends string = string> = {
  readonly id: Id;
  readonly defaultSize: StockBetaWidgetSize;
  readonly required: boolean;
  readonly defaultVisible: boolean;
  readonly order: number;
};

export type StockBetaWidgetConfiguration<
  Id extends string = string,
  Layout extends StockBetaWidgetLayout<Id> = StockBetaWidgetLayout<Id>,
> = {
  readonly definitions: readonly StockBetaWidgetDefinitionConfiguration<Id>[];
  readonly requiredWidgetIds: readonly Id[];
  readonly layout: Layout;
};

export type StockBetaWidgetArchitectureIssue = {
  readonly code:
    | "duplicate-definition-id"
    | "duplicate-definition-order"
    | "duplicate-layout-id"
    | "duplicate-layout-order"
    | "duplicate-required-id"
    | "invalid-architecture"
    | "invalid-definition"
    | "invalid-layout"
    | "invalid-grid-column"
    | "invalid-grid-column-span"
    | "invalid-grid-row"
    | "invalid-order"
    | "invalid-size"
    | "missing-required-widget"
    | "overlapping-layout-placement"
    | "required-widget-hidden"
    | "required-widget-not-required"
    | "unlisted-required-widget"
    | "unknown-widget";
  readonly path: string;
};

export class InvalidStockBetaWidgetArchitecture extends Error {
  override readonly name = "InvalidStockBetaWidgetArchitecture";

  constructor(readonly issues: readonly StockBetaWidgetArchitectureIssue[]) {
    super(
      `Invalid stock-beta widget architecture: ${issues
        .map((issue) => `${issue.code} at ${issue.path}`)
        .join(", ")}`,
    );
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isWidgetSize(value: unknown): value is StockBetaWidgetSize {
  return typeof value === "string" && STOCK_BETA_WIDGET_SIZES.some((size) => size === value);
}

function isOrder(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 1;
}

const GRID_COLUMNS: Readonly<Record<StockBetaWidgetBreakpoint, number>> = {
  desktop: 12,
  tablet: 12,
  mobile: 1,
};

const GRID_PLACEMENT_KEYS = ["column", "columnSpan", "row", "empty"] as const;

function issue(
  code: StockBetaWidgetArchitectureIssue["code"],
  path: string,
): StockBetaWidgetArchitectureIssue {
  return { code, path };
}

function duplicateValues(values: readonly unknown[]): ReadonlySet<unknown> {
  const seen = new Set<unknown>();
  const duplicates = new Set<unknown>();
  for (const value of values) {
    if (seen.has(value)) duplicates.add(value);
    seen.add(value);
  }
  return duplicates;
}

export function validateStockBetaWidgetArchitecture(
  input: unknown,
): readonly StockBetaWidgetArchitectureIssue[] {
  if (!isRecord(input)) return [issue("invalid-architecture", "$")];

  const issues: StockBetaWidgetArchitectureIssue[] = [];
  const definitions = input["definitions"];
  const requiredWidgetIds = input["requiredWidgetIds"];
  const layout = input["layout"];

  if (!Array.isArray(definitions)) issues.push(issue("invalid-architecture", "definitions"));
  if (!Array.isArray(requiredWidgetIds)) {
    issues.push(issue("invalid-architecture", "requiredWidgetIds"));
  }
  if (!isRecord(layout)) issues.push(issue("invalid-layout", "layout"));

  if (!Array.isArray(definitions) || !Array.isArray(requiredWidgetIds) || !isRecord(layout)) {
    return issues;
  }

  const definitionIds: string[] = [];
  const definitionOrders: number[] = [];
  const requiredByDefinition = new Set<string>();

  definitions.forEach((definition, index) => {
    const path = `definitions[${index}]`;
    if (!isRecord(definition)) {
      issues.push(issue("invalid-definition", path));
      return;
    }

    const id = definition["id"];
    if (typeof id !== "string" || id.trim() === "") {
      issues.push(issue("invalid-definition", `${path}.id`));
    } else {
      definitionIds.push(id);
      if (definition["required"] === true) requiredByDefinition.add(id);
    }

    if (typeof definition["component"] !== "function") {
      issues.push(issue("invalid-definition", `${path}.component`));
    }
    if (!isWidgetSize(definition["defaultSize"])) {
      issues.push(issue("invalid-size", `${path}.defaultSize`));
    }
    if (typeof definition["required"] !== "boolean") {
      issues.push(issue("invalid-definition", `${path}.required`));
    }
    if (typeof definition["defaultVisible"] !== "boolean") {
      issues.push(issue("invalid-definition", `${path}.defaultVisible`));
    }
    if (!isOrder(definition["order"])) {
      issues.push(issue("invalid-order", `${path}.order`));
    } else {
      definitionOrders.push(definition["order"]);
    }
    if (definition["required"] === true && definition["defaultVisible"] !== true) {
      issues.push(issue("required-widget-hidden", `${path}.defaultVisible`));
    }
  });

  if (duplicateValues(definitionIds).size > 0) {
    issues.push(issue("duplicate-definition-id", "definitions"));
  }
  if (duplicateValues(definitionOrders).size > 0) {
    issues.push(issue("duplicate-definition-order", "definitions"));
  }

  const requiredIds: string[] = [];
  requiredWidgetIds.forEach((id, index) => {
    if (typeof id !== "string" || id.trim() === "") {
      issues.push(issue("invalid-architecture", `requiredWidgetIds[${index}]`));
      return;
    }
    requiredIds.push(id);
  });
  if (duplicateValues(requiredIds).size > 0) {
    issues.push(issue("duplicate-required-id", "requiredWidgetIds"));
  }

  const definitionIdSet = new Set(definitionIds);
  const requiredIdSet = new Set(requiredIds);
  for (const id of requiredIdSet) {
    if (!definitionIdSet.has(id)) {
      issues.push(issue("missing-required-widget", `requiredWidgetIds.${id}`));
    } else if (!requiredByDefinition.has(id)) {
      issues.push(issue("required-widget-not-required", `definitions.${id}.required`));
    }
  }
  for (const id of requiredByDefinition) {
    if (!requiredIdSet.has(id)) {
      issues.push(issue("unlisted-required-widget", `definitions.${id}.required`));
    }
  }

  const layoutKeys = Object.keys(layout);
  for (const key of layoutKeys) {
    if (!STOCK_BETA_WIDGET_BREAKPOINTS.some((breakpoint) => breakpoint === key)) {
      issues.push(issue("invalid-layout", `layout.${key}`));
    }
  }

  for (const breakpoint of STOCK_BETA_WIDGET_BREAKPOINTS) {
    const placements = layout[breakpoint];
    if (!Array.isArray(placements)) {
      issues.push(issue("invalid-layout", `layout.${breakpoint}`));
      continue;
    }

    const placementIds: string[] = [];
    const placementOrders: number[] = [];
    const emptyPlacementOrders: number[] = [];
    const visibleIds = new Set<string>();
    const usesGridPlacement = placements.some(
      (placement) => isRecord(placement) && GRID_PLACEMENT_KEYS.some((key) => key in placement),
    );
    const occupiedCells = new Map<string, string>();

    const validateGridState = (
      state: Record<string, unknown>,
      path: string,
      stateName: "populated" | "empty",
    ): void => {
      const column = state["column"];
      const columnSpan = state["columnSpan"];
      const row = state["row"];
      const maxColumns = GRID_COLUMNS[breakpoint];

      if (!isPositiveInteger(column) || column > maxColumns) {
        issues.push(issue("invalid-grid-column", `${path}.column`));
      }
      if (
        !isPositiveInteger(columnSpan) ||
        (isPositiveInteger(column) && column + columnSpan - 1 > maxColumns)
      ) {
        issues.push(issue("invalid-grid-column-span", `${path}.columnSpan`));
      }
      if (!isPositiveInteger(row)) issues.push(issue("invalid-grid-row", `${path}.row`));
      if (stateName === "empty") {
        if (typeof state["visible"] !== "boolean") {
          issues.push(issue("invalid-layout", `${path}.visible`));
        }
        if (!isOrder(state["order"])) {
          issues.push(issue("invalid-order", `${path}.order`));
        } else {
          emptyPlacementOrders.push(state["order"]);
        }
      }

      if (
        state["visible"] === true &&
        isPositiveInteger(column) &&
        isPositiveInteger(columnSpan) &&
        column + columnSpan - 1 <= maxColumns &&
        isPositiveInteger(row)
      ) {
        for (
          let occupiedColumn = column;
          occupiedColumn < column + columnSpan;
          occupiedColumn += 1
        ) {
          const cell = `${stateName}:${row}:${occupiedColumn}`;
          if (occupiedCells.has(cell)) {
            issues.push(issue("overlapping-layout-placement", `layout.${breakpoint}.${stateName}`));
            break;
          }
          occupiedCells.set(cell, path);
        }
      }
    };

    placements.forEach((placement, index) => {
      const path = `layout.${breakpoint}[${index}]`;
      if (!isRecord(placement)) {
        issues.push(issue("invalid-layout", path));
        return;
      }

      const id = placement["id"];
      if (typeof id !== "string" || id.trim() === "") {
        issues.push(issue("invalid-layout", `${path}.id`));
      } else {
        placementIds.push(id);
        if (!definitionIdSet.has(id)) issues.push(issue("unknown-widget", `${path}.id`));
        if (placement["visible"] === true) visibleIds.add(id);
      }
      if (!isWidgetSize(placement["size"])) issues.push(issue("invalid-size", `${path}.size`));
      if (typeof placement["visible"] !== "boolean") {
        issues.push(issue("invalid-layout", `${path}.visible`));
      }
      if (!isOrder(placement["order"])) {
        issues.push(issue("invalid-order", `${path}.order`));
      } else {
        placementOrders.push(placement["order"]);
      }

      if (usesGridPlacement) {
        validateGridState(placement, path, "populated");
        const empty = placement["empty"];
        if (!isRecord(empty)) {
          issues.push(issue("invalid-layout", `${path}.empty`));
        } else {
          validateGridState(empty, `${path}.empty`, "empty");
        }
      }
    });

    if (duplicateValues(placementIds).size > 0) {
      issues.push(issue("duplicate-layout-id", `layout.${breakpoint}`));
    }
    if (duplicateValues(placementOrders).size > 0) {
      issues.push(issue("duplicate-layout-order", `layout.${breakpoint}`));
    }
    if (usesGridPlacement && duplicateValues(emptyPlacementOrders).size > 0) {
      issues.push(issue("duplicate-layout-order", `layout.${breakpoint}.empty`));
    }
    for (const id of requiredIdSet) {
      if (!placementIds.includes(id)) {
        issues.push(issue("missing-required-widget", `layout.${breakpoint}.${id}`));
      } else if (!visibleIds.has(id)) {
        issues.push(issue("required-widget-hidden", `layout.${breakpoint}.${id}`));
      }
    }
  }

  return issues;
}

export function assertValidStockBetaWidgetArchitecture(input: unknown): void {
  const issues = validateStockBetaWidgetArchitecture(input);
  if (issues.length > 0) throw new InvalidStockBetaWidgetArchitecture(issues);
}

export function defineStockBetaWidget<Id extends string, ViewModel>(
  definition: StockBetaWidgetDefinition<Id, ViewModel>,
): StockBetaWidgetDefinition<Id, ViewModel> {
  return definition;
}

export function defineStockBetaWidgetArchitecture<
  const Definitions extends readonly StockBetaWidgetDefinitionMetadata[],
  const Layout extends StockBetaWidgetLayout<Definitions[number]["id"]>,
>(
  architecture: StockBetaWidgetArchitecture<Definitions, Layout>,
): StockBetaWidgetArchitecture<Definitions, Layout> {
  assertValidStockBetaWidgetArchitecture(architecture);
  return architecture;
}

/**
 * Produce the component-free configuration that may cross a Server/Client boundary or be stored.
 * The runtime registry stays local because React components are intentionally not serializable.
 */
export function stockBetaWidgetConfiguration<
  const Definitions extends readonly StockBetaWidgetDefinitionMetadata[],
  const Layout extends StockBetaWidgetLayout<Definitions[number]["id"]>,
>(
  architecture: StockBetaWidgetArchitecture<Definitions, Layout>,
): StockBetaWidgetConfiguration<Definitions[number]["id"], Layout> {
  assertValidStockBetaWidgetArchitecture(architecture);
  return {
    definitions: architecture.definitions.map((definition) => ({
      id: definition.id,
      defaultSize: definition.defaultSize,
      required: definition.required,
      defaultVisible: definition.defaultVisible,
      order: definition.order,
    })),
    requiredWidgetIds: architecture.requiredWidgetIds,
    layout: architecture.layout,
  };
}
