import Link from "next/link";
import {
  type OwnerBetaEquitySignalCondition,
  type OwnerBetaEquitySignalsFiniteRange,
  type OwnerBetaEquitySignalsRangeKey,
  ownerBetaEquitySignalsFiltersSelected,
} from "@/lib/products/equity-signals-contracts";
import styles from "../dashboard.module.css";
import { stockBetaFilterQueryString } from "../filter-context";
import { stockBetaConditionLabel } from "../labels";
import type { StockBetaDashboardWidgetViewModel } from "../types";

type FilterLabelKey =
  | "activityLabel"
  | "drawdown120Label"
  | "return120Label"
  | "return20Label"
  | "return60Label"
  | "scoreLabel"
  | "volatility120Label"
  | "volatility20Label"
  | "volatility60Label";

const RANGE_FIELDS: readonly {
  readonly key: OwnerBetaEquitySignalsRangeKey;
  readonly label: FilterLabelKey;
}[] = [
  { key: "score", label: "scoreLabel" },
  { key: "return_20", label: "return20Label" },
  { key: "return_60", label: "return60Label" },
  { key: "return_120", label: "return120Label" },
  { key: "volatility_20", label: "volatility20Label" },
  { key: "volatility_60", label: "volatility60Label" },
  { key: "volatility_120", label: "volatility120Label" },
  { key: "max_drawdown_120", label: "drawdown120Label" },
  { key: "average_trading_value_20", label: "activityLabel" },
];

const CONDITIONS: readonly OwnerBetaEquitySignalCondition[] = ["BULLISH", "NEUTRAL", "BEARISH"];

function filterValue(
  filters: StockBetaDashboardWidgetViewModel["filters"],
  key: OwnerBetaEquitySignalsRangeKey,
  bound: "max" | "min",
): string | undefined {
  const value = filters.ranges[key]?.[bound];
  return value === undefined || value === null ? undefined : String(value);
}

function rangeText(
  range: OwnerBetaEquitySignalsFiniteRange,
  t: StockBetaDashboardWidgetViewModel["copy"],
): string {
  const values: string[] = [];
  if (range.min !== undefined && range.min !== null) values.push(`${t.minLabel} ${range.min}`);
  if (range.max !== undefined && range.max !== null) values.push(`${t.maxLabel} ${range.max}`);
  return values.join(" · ");
}

function filterHref(filters: StockBetaDashboardWidgetViewModel["filters"]): string {
  const query = stockBetaFilterQueryString(filters);
  return query === "" ? "/stock-beta" : `/stock-beta?${query}`;
}

type ActiveFilter = {
  readonly label: string;
  readonly removeHref: string;
  readonly value: string;
};

function activeFilterValues(viewModel: StockBetaDashboardWidgetViewModel): readonly ActiveFilter[] {
  const { copy: t, filters } = viewModel;
  const values: ActiveFilter[] = [];

  filters.conditions.forEach((condition, index) => {
    values.push({
      label: t.conditionLabel,
      removeHref: filterHref({
        ...filters,
        conditions: filters.conditions.filter((_, conditionIndex) => conditionIndex !== index),
      }),
      value: `${condition} · ${stockBetaConditionLabel(condition, t)}`,
    });
  });

  for (const field of RANGE_FIELDS) {
    const range = filters.ranges[field.key];
    if (range !== undefined) {
      const nextRanges = { ...filters.ranges };
      delete nextRanges[field.key];
      values.push({
        label: t[field.label],
        removeHref: filterHref({ ...filters, ranges: nextRanges }),
        value: rangeText(range, t),
      });
    }
  }

  if (filters.trendUp !== undefined) {
    values.push({
      label: t.trendLabel,
      removeHref: filterHref({ conditions: filters.conditions, ranges: filters.ranges }),
      value: filters.trendUp ? t.trendUpLabel : t.trendDownLabel,
    });
  }

  return values;
}

export function ActiveFilterChips({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t } = viewModel;
  const activeFilters = activeFilterValues(viewModel);
  return (
    <div className={styles["activeFilters"]} data-testid="stock-beta-active-filters">
      <span className={styles["activeFiltersLabel"]}>{t.activeFiltersHeading}</span>
      {activeFilters.length === 0 ? (
        <span className={styles["noActiveFilters"]}>{t.noActiveFilters}</span>
      ) : (
        <ul>
          {activeFilters.map(({ label, removeHref, value }) => (
            <li key={`${label}-${value}`}>
              <span>{label}</span>
              <strong>{value}</strong>
              <Link
                aria-label={`${t.removeFilterLabel}: ${label} ${value}`}
                href={removeHref}
                prefetch={false}
              >
                ×
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function FilterWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, filters } = viewModel;
  const filtersSelected = ownerBetaEquitySignalsFiltersSelected(filters);

  return (
    <details className={styles["filterDrawer"]} open={filtersSelected}>
      <summary>{t.filtersSummary}</summary>
      <div className={styles["filterPanel"]}>
        <div className={styles["filterPanelHeader"]}>
          <strong>{t.filtersHeading}</strong>
          <span>{t.filtersDescription}</span>
        </div>
        <form action="/stock-beta" className={styles["filterForm"]} method="get">
          <fieldset>
            <legend>{t.conditionLabel}</legend>
            <div className={styles["choiceRow"]}>
              {CONDITIONS.map((condition) => (
                <label className={styles["choice"]} key={condition}>
                  <input
                    defaultChecked={filters.conditions.includes(condition)}
                    name="condition"
                    type="checkbox"
                    value={condition}
                  />
                  <span>{conditionLabel(condition, t)}</span>
                </label>
              ))}
            </div>
          </fieldset>
          <div className={styles["rangeGrid"]}>
            {RANGE_FIELDS.map(({ key, label }) => (
              <fieldset className={styles["rangeField"]} key={key}>
                <legend>{t[label]}</legend>
                <label className={styles["formField"]} htmlFor={`stock-beta-${key}-min`}>
                  <span>{t.minLabel}</span>
                  <input
                    defaultValue={filterValue(filters, key, "min")}
                    id={`stock-beta-${key}-min`}
                    inputMode="decimal"
                    name={`${key}_min`}
                    step="any"
                    type="number"
                  />
                </label>
                <label className={styles["formField"]} htmlFor={`stock-beta-${key}-max`}>
                  <span>{t.maxLabel}</span>
                  <input
                    defaultValue={filterValue(filters, key, "max")}
                    id={`stock-beta-${key}-max`}
                    inputMode="decimal"
                    name={`${key}_max`}
                    step="any"
                    type="number"
                  />
                </label>
              </fieldset>
            ))}
          </div>
          <label className={`${styles["formField"]} ${styles["trendField"]}`}>
            <span>{t.trendLabel}</span>
            <select
              defaultValue={filters.trendUp === undefined ? "" : filters.trendUp ? "up" : "down"}
              name="trend"
            >
              <option value="">—</option>
              <option value="up">{t.trendUpLabel}</option>
              <option value="down">{t.trendDownLabel}</option>
            </select>
          </label>
          <div className={styles["filterActions"]}>
            <button className={styles["applyButton"]} type="submit">
              {t.applyFilters}
            </button>
            <Link className={styles["clearLink"]} href="/stock-beta" prefetch={false}>
              {t.clearFilters}
            </Link>
          </div>
        </form>
      </div>
    </details>
  );
}

function conditionLabel(
  condition: OwnerBetaEquitySignalCondition,
  t: StockBetaDashboardWidgetViewModel["copy"],
): string {
  return `${condition} · ${stockBetaConditionLabel(condition, t)}`;
}
