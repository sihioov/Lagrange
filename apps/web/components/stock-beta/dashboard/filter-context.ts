import {
  OWNER_BETA_EQUITY_SIGNALS_RANGE_KEYS,
  type OwnerBetaEquitySignalsFilters,
} from "@/lib/products/equity-signals-contracts";

export function stockBetaFilterQueryString(filters: OwnerBetaEquitySignalsFilters): string {
  const params = new URLSearchParams();

  for (const condition of filters.conditions) params.append("condition", condition);

  for (const key of OWNER_BETA_EQUITY_SIGNALS_RANGE_KEYS) {
    const range = filters.ranges[key];
    if (range?.min !== undefined && range.min !== null) {
      params.set(`${key}_min`, String(range.min));
    }
    if (range?.max !== undefined && range.max !== null) {
      params.set(`${key}_max`, String(range.max));
    }
  }

  if (filters.trendUp !== undefined) params.set("trend", filters.trendUp ? "up" : "down");

  return params.toString();
}

export function stockBetaDetailHref(
  instrumentId: string,
  filters: OwnerBetaEquitySignalsFilters,
): string {
  const path = `/stock-beta/${encodeURIComponent(instrumentId)}`;
  const query = stockBetaFilterQueryString(filters);
  return query.length === 0 ? path : `${path}?${query}`;
}
