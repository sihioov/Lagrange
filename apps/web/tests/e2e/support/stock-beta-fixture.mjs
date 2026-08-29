const SIGNAL_PATH = "/api/v1/research/owner-beta/equity-price-signals";
const HASH = `sha256:${"a".repeat(64)}`;

const instrumentIds = Object.freeze(
  Array.from({ length: 30 }, (_, index) => `${String(index + 1).padStart(6, "0")}.KRX`),
);

const provenance = Object.freeze({
  activity_proxy: "20-session average trading value; activity/liquidity proxy",
  artifact_content_sha256: HASH,
  as_of: "2026-08-29",
  audience: "OWNER_ONLY",
  batch_id: "batch-stock-beta-synthetic",
  capability: "PRICE_VOLUME_RESEARCH_ONLY",
  entitlement_sha256: HASH,
  factor_version: "stock-price-beta-v1",
  index_membership: "not current or historical index membership",
  materialization_status: "MATERIALIZED",
  original_price: true,
  publication_status: "OWNER_ONLY_API",
  redistribution: "owner-only access",
  registration_status: "registered",
  registry_sha256: HASH,
  selection_basis: "configured fixed observation list",
  snapshot_content_sha256: HASH,
  strict_pit: false,
  universe_sha256: HASH,
  vendor_snapshot: true,
  warning: "Original/unadjusted prices; corporate actions can distort returns and drawdowns.",
});

function row(index) {
  return {
    average_trading_value_20: 1_000_000 + index,
    average_volume_20: 100_000 + index,
    condition: index % 3 === 0 ? "BULLISH" : index % 3 === 1 ? "NEUTRAL" : "BEARISH",
    instrument_id: instrumentIds[index],
    instrument_name: `Configured instrument ${index + 1}`,
    max_drawdown_120: -0.1 - index / 1_000,
    rank: index + 1,
    return_120: 0.4 - index / 100,
    return_20: 0.1 - index / 1_000,
    return_60: 0.2 - index / 1_000,
    score: 100 - index,
    sma_20: 100 + index,
    sma_60: 99 + index,
    volatility_120: 0.2 + index / 1_000,
    volatility_20: 0.1 + index / 1_000,
    volatility_60: 0.15 + index / 1_000,
    volume_ratio_20_60: 1.2 + index / 1_000,
  };
}

const rows = Object.freeze(instrumentIds.map((_, index) => row(index)));

const factorExplanations = Object.freeze([
  { factor: "return_20", interpretation: "20-session price return", value: 0.1 },
  { factor: "return_60", interpretation: "60-session price return", value: 0.2 },
  { factor: "return_120", interpretation: "120-session price return", value: 0.4 },
  {
    factor: "volatility_120",
    interpretation: "120-session annualized volatility",
    value: 0.2,
  },
  {
    factor: "max_drawdown_120",
    interpretation: "120-session maximum drawdown",
    value: -0.1,
  },
  {
    factor: "average_trading_value_20",
    interpretation: "20-session activity proxy, not execution liquidity",
    value: 1_000_000,
  },
  { factor: "trend", interpretation: "sma20 minus sma60", value: 1 },
]);

function error(status, code) {
  return {
    body: { error: { code, message: code, request_id: "request-synthetic-stock-beta" } },
    status,
  };
}

export function stockBetaResponse(request) {
  const { body, method, pathname, scenario } = request;
  if (pathname !== SIGNAL_PATH && !pathname.startsWith(`${SIGNAL_PATH}/`)) return null;
  if (scenario.role !== "owner") return error(403, "FORBIDDEN");
  if (scenario.stockBeta === "unavailable") {
    return error(503, "OWNER_BETA_EQUITY_SIGNALS_UNAVAILABLE");
  }
  if (scenario.stockBeta === "integrity") {
    return error(503, "OWNER_BETA_EQUITY_SIGNALS_INTEGRITY_FAILED");
  }
  if (method === "GET" && pathname === `${SIGNAL_PATH}/latest`) {
    return { body: { provenance, rows, top5: rows.slice(0, 5) }, status: 200 };
  }
  if (method === "POST" && pathname === `${SIGNAL_PATH}/screen`) {
    const conditions = Array.isArray(body?.condition) ? new Set(body.condition) : null;
    const screened =
      conditions === null ? rows : rows.filter((item) => conditions.has(item.condition));
    return { body: { provenance, rows: screened }, status: 200 };
  }
  if (method === "GET" && pathname.startsWith(`${SIGNAL_PATH}/instruments/`)) {
    const instrumentId = decodeURIComponent(pathname.slice(`${SIGNAL_PATH}/instruments/`.length));
    const signal = rows.find((item) => item.instrument_id === instrumentId);
    if (signal === undefined) return error(404, "NOT_FOUND");
    return {
      body: {
        condition_reasons: [
          "return_20 is at least 0.050000",
          "trend_up is true",
          "volatility_120 is at most 0.350000",
        ],
        factor_explanations: factorExplanations,
        provenance,
        signal,
      },
      status: 200,
    };
  }
  return null;
}
