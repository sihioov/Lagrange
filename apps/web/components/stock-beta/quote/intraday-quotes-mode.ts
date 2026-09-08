import "server-only";

/** The feature flag is intentionally server-only and exact-match. */
export function isStockBetaIntradayQuotesEnabled(): boolean {
  return process.env["OWNER_INTRADAY_QUOTES_MODE"] === "owner_only";
}
