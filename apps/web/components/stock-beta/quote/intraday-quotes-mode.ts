import "server-only";

/** The feature flag is intentionally server-only and exact-match. */
export function isStockBetaIntradayQuotesEnabled(): boolean {
  return stockBetaIntradayTransport() !== "off";
}

export function stockBetaIntradayTransport(): "off" | "rest" | "market_ws" {
  if (process.env["OWNER_INTRADAY_QUOTES_MODE"] !== "owner_only") return "off";
  const transport = process.env["OWNER_INTRADAY_QUOTE_TRANSPORT"];
  if (transport === undefined || transport === "rest") return "rest";
  return transport === "market_ws" ? "market_ws" : "off";
}
