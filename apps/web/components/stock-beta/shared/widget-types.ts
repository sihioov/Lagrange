import type { ComponentType } from "react";

export const STOCK_BETA_WIDGET_SIZES = ["small", "medium", "large", "full"] as const;

export type StockBetaWidgetSize = (typeof STOCK_BETA_WIDGET_SIZES)[number];

export const STOCK_BETA_WIDGET_BREAKPOINTS = ["desktop", "tablet", "mobile"] as const;

export type StockBetaWidgetBreakpoint = (typeof STOCK_BETA_WIDGET_BREAKPOINTS)[number];

export type StockBetaWidgetProps<ViewModel> = {
  readonly placementVisibility?: Readonly<Partial<Record<StockBetaWidgetBreakpoint, boolean>>>;
  readonly viewModel: ViewModel;
};

export type StockBetaWidgetPlacementState = {
  readonly size: StockBetaWidgetSize;
  readonly visible: boolean;
};

export type StockBetaWidgetGridState = {
  readonly column: number;
  readonly columnSpan: number;
  readonly row: number;
  readonly visible: boolean;
};

export type StockBetaWidgetGridPlacementState = StockBetaWidgetPlacementState &
  Omit<StockBetaWidgetGridState, "visible"> & {
    readonly empty: StockBetaWidgetGridState;
  };

export type StockBetaWidgetCatalogEntry<
  Id extends string,
  ViewModel,
  Placement extends StockBetaWidgetPlacementState = StockBetaWidgetPlacementState,
> = {
  readonly id: Id;
  readonly component: ComponentType<StockBetaWidgetProps<ViewModel>>;
  readonly required: boolean;
  readonly placements: Readonly<Partial<Record<StockBetaWidgetBreakpoint, Placement>>>;
};

export type StockBetaWidgetPlacement<
  Id extends string,
  Placement extends StockBetaWidgetPlacementState = StockBetaWidgetPlacementState,
> = Placement & { readonly id: Id };

export type StockBetaWidgetGridPlacement<Id extends string> = StockBetaWidgetPlacement<
  Id,
  StockBetaWidgetGridPlacementState
>;

export type StockBetaWidgetLayout<
  Id extends string,
  Placement extends StockBetaWidgetPlacementState = StockBetaWidgetPlacementState,
> = Readonly<Record<StockBetaWidgetBreakpoint, readonly StockBetaWidgetPlacement<Id, Placement>[]>>;

export type StockBetaWidgetGridLayout<Id extends string> = StockBetaWidgetLayout<
  Id,
  StockBetaWidgetGridPlacementState
>;

type StockBetaWidgetCatalogEntryMetadata = {
  readonly id: string;
  readonly component: unknown;
  readonly required: boolean;
  readonly placements: Readonly<
    Partial<Record<StockBetaWidgetBreakpoint, StockBetaWidgetPlacementState>>
  >;
};

type StockBetaWidgetCatalogPlacement<
  Catalog extends readonly StockBetaWidgetCatalogEntryMetadata[],
> =
  Exclude<
    Catalog[number]["placements"][StockBetaWidgetBreakpoint],
    undefined
  > extends infer Placement
    ? Placement extends StockBetaWidgetPlacementState
      ? Placement
      : never
    : never;

export type StockBetaWidgetArchitecture<
  Catalog extends readonly StockBetaWidgetCatalogEntryMetadata[],
> = {
  /** Catalog order is the canonical DOM and accessibility reading order. */
  readonly catalog: Catalog;
  readonly requiredWidgetIds: readonly Catalog[number]["id"][];
  readonly layout: StockBetaWidgetLayout<
    Catalog[number]["id"],
    StockBetaWidgetCatalogPlacement<Catalog>
  >;
};

export type StockBetaWidgetId<
  Architecture extends StockBetaWidgetArchitecture<readonly StockBetaWidgetCatalogEntryMetadata[]>,
> = Architecture["catalog"][number]["id"];

export type StockBetaWidgetConfiguration<Id extends string = string> = {
  readonly widgets: readonly {
    readonly id: Id;
    readonly required: boolean;
  }[];
  readonly requiredWidgetIds: readonly Id[];
  readonly layout: StockBetaWidgetLayout<
    Id,
    StockBetaWidgetPlacementState | StockBetaWidgetGridPlacementState
  >;
};

export type StockBetaWidgetArchitectureIssue = {
  readonly code:
    | "duplicate-definition-id"
    | "invalid-architecture"
    | "invalid-definition"
    | "invalid-layout"
    | "invalid-grid-column"
    | "invalid-grid-column-span"
    | "invalid-grid-row"
    | "invalid-size"
    | "missing-required-widget"
    | "overlapping-layout-placement"
    | "required-widget-hidden";
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

function usesGridPlacement(value: unknown): boolean {
  return isRecord(value) && GRID_PLACEMENT_KEYS.some((key) => key in value);
}

function validateGridState(
  state: Record<string, unknown>,
  path: string,
  breakpoint: StockBetaWidgetBreakpoint,
  occupiedCells: Map<string, string>,
  stateName: "populated" | "empty",
  issues: StockBetaWidgetArchitectureIssue[],
): void {
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
  if (typeof state["visible"] !== "boolean") {
    issues.push(issue("invalid-layout", `${path}.visible`));
  }

  if (
    state["visible"] === true &&
    isPositiveInteger(column) &&
    isPositiveInteger(columnSpan) &&
    column + columnSpan - 1 <= maxColumns &&
    isPositiveInteger(row)
  ) {
    for (let occupiedColumn = column; occupiedColumn < column + columnSpan; occupiedColumn += 1) {
      const cell = `${stateName}:${row}:${occupiedColumn}`;
      if (occupiedCells.has(cell)) {
        issues.push(issue("overlapping-layout-placement", `layout.${breakpoint}.${stateName}`));
        break;
      }
      occupiedCells.set(cell, path);
    }
  }
}

export function validateStockBetaWidgetCatalog(
  input: unknown,
): readonly StockBetaWidgetArchitectureIssue[] {
  if (!Array.isArray(input)) return [issue("invalid-architecture", "catalog")];

  const issues: StockBetaWidgetArchitectureIssue[] = [];
  const ids: string[] = [];
  const usesGrid = input.some(
    (entry) =>
      isRecord(entry) &&
      isRecord(entry["placements"]) &&
      STOCK_BETA_WIDGET_BREAKPOINTS.some((breakpoint) =>
        usesGridPlacement((entry["placements"] as Record<string, unknown>)[breakpoint]),
      ),
  );
  const occupiedCells = Object.fromEntries(
    STOCK_BETA_WIDGET_BREAKPOINTS.map((breakpoint) => [breakpoint, new Map<string, string>()]),
  ) as Record<StockBetaWidgetBreakpoint, Map<string, string>>;

  input.forEach((entry, index) => {
    const path = `catalog[${index}]`;
    if (!isRecord(entry)) {
      issues.push(issue("invalid-definition", path));
      return;
    }

    const id = entry["id"];
    if (typeof id !== "string" || id.trim() === "") {
      issues.push(issue("invalid-definition", `${path}.id`));
    } else {
      ids.push(id);
    }
    if (typeof entry["component"] !== "function") {
      issues.push(issue("invalid-definition", `${path}.component`));
    }
    const required = entry["required"];
    if (typeof required !== "boolean") {
      issues.push(issue("invalid-definition", `${path}.required`));
    }

    const placements = entry["placements"];
    if (!isRecord(placements)) {
      issues.push(issue("invalid-layout", `${path}.placements`));
      return;
    }
    for (const key of Object.keys(placements)) {
      if (!STOCK_BETA_WIDGET_BREAKPOINTS.some((breakpoint) => breakpoint === key)) {
        issues.push(issue("invalid-layout", `${path}.placements.${key}`));
      }
    }

    for (const breakpoint of STOCK_BETA_WIDGET_BREAKPOINTS) {
      const placement = placements[breakpoint];
      const placementPath = `${path}.placements.${breakpoint}`;
      if (placement === undefined) {
        if (required === true) issues.push(issue("missing-required-widget", placementPath));
        continue;
      }
      if (!isRecord(placement)) {
        issues.push(issue("invalid-layout", placementPath));
        continue;
      }
      if (!isWidgetSize(placement["size"])) {
        issues.push(issue("invalid-size", `${placementPath}.size`));
      }
      if (typeof placement["visible"] !== "boolean") {
        issues.push(issue("invalid-layout", `${placementPath}.visible`));
      } else if (required === true && placement["visible"] !== true) {
        issues.push(issue("required-widget-hidden", placementPath));
      }

      if (usesGrid) {
        validateGridState(
          placement,
          placementPath,
          breakpoint,
          occupiedCells[breakpoint],
          "populated",
          issues,
        );
        const empty = placement["empty"];
        if (!isRecord(empty)) {
          issues.push(issue("invalid-layout", `${placementPath}.empty`));
        } else {
          validateGridState(
            empty,
            `${placementPath}.empty`,
            breakpoint,
            occupiedCells[breakpoint],
            "empty",
            issues,
          );
        }
      }
    }
  });

  if (duplicateValues(ids).size > 0) {
    issues.push(issue("duplicate-definition-id", "catalog"));
  }

  return issues;
}

function normalizedPlacement(
  placement: StockBetaWidgetPlacementState,
): StockBetaWidgetPlacementState | StockBetaWidgetGridPlacementState {
  if (!usesGridPlacement(placement)) {
    return { size: placement.size, visible: placement.visible };
  }
  const grid = placement as StockBetaWidgetGridPlacementState;
  return {
    size: grid.size,
    visible: grid.visible,
    column: grid.column,
    columnSpan: grid.columnSpan,
    row: grid.row,
    empty: {
      column: grid.empty.column,
      columnSpan: grid.empty.columnSpan,
      row: grid.empty.row,
      visible: grid.empty.visible,
    },
  };
}

function derivedLayout<Catalog extends readonly StockBetaWidgetCatalogEntryMetadata[]>(
  catalog: Catalog,
): StockBetaWidgetArchitecture<Catalog>["layout"] {
  return Object.fromEntries(
    STOCK_BETA_WIDGET_BREAKPOINTS.map((breakpoint) => [
      breakpoint,
      catalog.flatMap((entry) => {
        const placement = entry.placements[breakpoint];
        return placement === undefined ? [] : [{ id: entry.id, ...normalizedPlacement(placement) }];
      }),
    ]),
  ) as unknown as StockBetaWidgetArchitecture<Catalog>["layout"];
}

export function defineStockBetaWidgetCatalog<ViewModel>() {
  return function defineCatalog<
    const Catalog extends readonly StockBetaWidgetCatalogEntry<
      string,
      ViewModel,
      StockBetaWidgetPlacementState
    >[],
  >(catalog: Catalog): Catalog {
    const issues = validateStockBetaWidgetCatalog(catalog);
    if (issues.length > 0) throw new InvalidStockBetaWidgetArchitecture(issues);
    return catalog;
  };
}

export function defineStockBetaWidgetArchitecture<
  const Catalog extends readonly StockBetaWidgetCatalogEntryMetadata[],
>(catalog: Catalog): StockBetaWidgetArchitecture<Catalog> {
  const issues = validateStockBetaWidgetCatalog(catalog);
  if (issues.length > 0) throw new InvalidStockBetaWidgetArchitecture(issues);
  return {
    catalog,
    requiredWidgetIds: catalog.flatMap((entry) => (entry.required ? [entry.id] : [])),
    layout: derivedLayout(catalog),
  };
}

export function validateStockBetaWidgetArchitecture(
  input: unknown,
): readonly StockBetaWidgetArchitectureIssue[] {
  if (!isRecord(input) || !Array.isArray(input["catalog"])) {
    return [issue("invalid-architecture", "$")];
  }
  const issues = [...validateStockBetaWidgetCatalog(input["catalog"])];
  if (issues.length > 0) return issues;

  const catalog = input["catalog"] as readonly StockBetaWidgetCatalogEntryMetadata[];
  const expectedRequiredIds = catalog.flatMap((entry) => (entry.required ? [entry.id] : []));
  if (
    !Array.isArray(input["requiredWidgetIds"]) ||
    JSON.stringify(input["requiredWidgetIds"]) !== JSON.stringify(expectedRequiredIds)
  ) {
    issues.push(issue("invalid-architecture", "requiredWidgetIds"));
  }

  if (!isRecord(input["layout"])) {
    issues.push(issue("invalid-layout", "layout"));
    return issues;
  }
  for (const key of Object.keys(input["layout"])) {
    if (!STOCK_BETA_WIDGET_BREAKPOINTS.some((breakpoint) => breakpoint === key)) {
      issues.push(issue("invalid-layout", `layout.${key}`));
    }
  }
  const expectedLayout = derivedLayout(catalog);
  for (const breakpoint of STOCK_BETA_WIDGET_BREAKPOINTS) {
    if (
      JSON.stringify(input["layout"][breakpoint]) !== JSON.stringify(expectedLayout[breakpoint])
    ) {
      issues.push(issue("invalid-layout", `layout.${breakpoint}`));
    }
  }
  return issues;
}

/**
 * Produce the component-free configuration that may cross a Server/Client boundary or be stored.
 * The catalog stays module-local because React components are intentionally not serializable.
 */
export function stockBetaWidgetConfiguration<
  const Catalog extends readonly StockBetaWidgetCatalogEntryMetadata[],
>(
  architecture: StockBetaWidgetArchitecture<Catalog>,
): StockBetaWidgetConfiguration<Catalog[number]["id"]> {
  const issues = validateStockBetaWidgetArchitecture(architecture);
  if (issues.length > 0) throw new InvalidStockBetaWidgetArchitecture(issues);
  return {
    widgets: architecture.catalog.map((entry) => ({
      id: entry.id,
      required: entry.required,
    })),
    requiredWidgetIds: architecture.requiredWidgetIds,
    layout: architecture.layout,
  };
}
