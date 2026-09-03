import {
  OWNER_BETA_EQUITY_SIGNALS_RANGE_KEYS,
  type OwnerBetaEquitySignalsSearchParams,
  ownerBetaEquitySignalsFiltersSelected,
  parseOwnerBetaEquitySignalsSearchParams,
} from "@/lib/products/equity-signals-contracts";
import { stockBetaFilterQueryString } from "../dashboard/filter-context";

const APPROVED_FILTER_KEYS = new Set<string>([
  "condition",
  "trend",
  ...OWNER_BETA_EQUITY_SIGNALS_RANGE_KEYS.flatMap((key) => [`${key}_min`, `${key}_max`]),
]);

export const STOCK_BETA_DETAIL_APPROVED_FILTER_KEYS = Object.freeze(
  [...APPROVED_FILTER_KEYS].sort(),
);

function hasOnlyApprovedFilterKeys(searchParams: OwnerBetaEquitySignalsSearchParams): boolean {
  return Object.keys(searchParams).every((key) => APPROVED_FILTER_KEYS.has(key));
}

/**
 * Rebuild the list destination from parsed filters only. The detail URL never supplies a return
 * target, and a malformed or unknown query key deliberately drops all filter context.
 */
export function stockBetaDetailBackHref(
  searchParams: OwnerBetaEquitySignalsSearchParams | undefined,
): string {
  if (searchParams === undefined || !hasOnlyApprovedFilterKeys(searchParams)) return "/stock-beta";

  try {
    const filters = parseOwnerBetaEquitySignalsSearchParams(searchParams);
    if (!ownerBetaEquitySignalsFiltersSelected(filters)) return "/stock-beta";
    const query = stockBetaFilterQueryString(filters);
    return query.length === 0 ? "/stock-beta" : `/stock-beta?${query}`;
  } catch {
    return "/stock-beta";
  }
}

export function safeStockBetaDetailBackHref(href: string): string {
  if (href === "/stock-beta") return href;
  if (!href.startsWith("/stock-beta?")) return "/stock-beta";

  const searchParams: Record<string, string | readonly string[]> = {};
  for (const [key, value] of new URLSearchParams(href.slice("/stock-beta?".length))) {
    const current = searchParams[key];
    if (current === undefined) {
      searchParams[key] = value;
    } else if (typeof current === "string") {
      searchParams[key] = [current, value];
    } else {
      searchParams[key] = [...current, value];
    }
  }

  return stockBetaDetailBackHref(searchParams);
}
