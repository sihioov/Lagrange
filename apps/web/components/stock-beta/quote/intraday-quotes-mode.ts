import "server-only";

/** The feature flag is intentionally server-only and exact-match. */
export function isStockBetaIntradayQuotesEnabled(): boolean {
  return stockBetaIntradayTransport() === "market_ws";
}

export function stockBetaIntradayTransport(): "off" | "market_ws" {
  if (process.env["OWNER_INTRADAY_QUOTES_MODE"] !== "owner_only") return "off";
  return process.env["OWNER_INTRADAY_QUOTE_TRANSPORT"] === "market_ws" ? "market_ws" : "off";
}
