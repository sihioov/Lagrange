import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  stockBetaDashboardArchitecture,
  stockBetaDashboardCatalog,
} from "@/components/stock-beta/dashboard/widget-registry";
import {
  stockBetaDetailArchitecture,
  stockBetaDetailCatalog,
} from "@/components/stock-beta/detail/widget-registry";
import {
  CurrentQuoteView,
  formatIntradayQuoteSigned,
} from "@/components/stock-beta/quote/current-quote-view";
import { validateStockBetaWidgetArchitecture } from "@/components/stock-beta/shared/widget-types";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { intradayQuoteResponseSchema } from "@/lib/products/intraday-quotes-contracts";

const RESPONSE = intradayQuoteResponseSchema.parse({
  currency: "KRW",
  freshness: "RECENT",
  generation: 7,
  instrument_id: "069500.KRX",
  market_state: "OPEN",
  membership_id: "00000000-0000-4000-8000-000000000001",
  next_poll_after_ms: 5_000,
  quote: {
    base_price: "100000000000",
    change_from_previous_day: "123456789012.12345678",
    change_percent_from_previous_day: "1.25",
    direction: "UP",
    last_success_at: "2026-09-08T02:59:00Z",
    price: "100123456789.12345678",
    quote_version: "9223372036854775807",
    received_at: "2026-09-08T02:59:00Z",
  },
  reason_code: null,
  schema_version: 1,
  session: {
    calendar_content_sha256: "a".repeat(64),
    calendar_source: "kis",
    calendar_source_version: "kis-chk-holiday-v1:schema-1",
    date: "2026-09-08",
    timezone: "Asia/Seoul",
    window_contract_sha256: `sha256:${"b".repeat(64)}`,
  },
  venue: "KRX",
});

function state() {
  return {
    consumerId: "00000000-0000-4000-8000-000000000002",
    errorCode: null,
    identity: {
      generation: 7,
      instrument_id: RESPONSE.instrument_id,
      membership_id: RESPONSE.membership_id,
    },
    lastSuccessAt: RESPONSE.quote?.last_success_at ?? null,
    phase: "ready" as const,
    quote: RESPONSE,
    reasonCode: null,
  };
}

describe("Stock Beta current quote widget", () => {
  it("renders Koyfin-style Korean and English numeric strings without Number coercion", () => {
    const english = renderToStaticMarkup(
      <CurrentQuoteView copy={stockBetaDictionary.en} locale="en" state={state()} />,
    );
    const korean = renderToStaticMarkup(
      <CurrentQuoteView copy={stockBetaDictionary.ko} locale="ko" state={state()} />,
    );

    expect(english).toContain("Intraday price · periodic refresh");
    expect(korean).toContain("장중 현재가 · 주기적 조회");
    expect(english).toContain("100,123,456,789.12345678");
    expect(english).toContain("+123,456,789,012.12345678");
    expect(english).toContain("+1.25%");
    expect(korean).toContain("기준 가격");
    expect(korean).toContain("전일 종가 아님");
    expect(english).toContain('data-direction="UP"');
    expect(english).toContain("Up");
    expect(english).toContain("2026-09-08T02:59:00Z");
    expect(formatIntradayQuoteSigned("0", "en")).toBe("0");
    expect(formatIntradayQuoteSigned("-12.5", "ko")).toBe("-12.5");
  });

  it("keeps the optional current-quote entry non-overlapping and removable from both catalogs", () => {
    expect(validateStockBetaWidgetArchitecture(stockBetaDashboardArchitecture)).toEqual([]);
    expect(validateStockBetaWidgetArchitecture(stockBetaDetailArchitecture)).toEqual([]);
    const dashboard = stockBetaDashboardCatalog.find((entry) => entry.id === "current-quote");
    const detail = stockBetaDetailCatalog.find((entry) => entry.id === "current-quote");
    expect(dashboard?.required).toBe(false);
    expect(detail?.required).toBe(false);
    expect(stockBetaDashboardArchitecture.requiredWidgetIds).not.toContain("current-quote");
    expect(stockBetaDetailArchitecture.requiredWidgetIds).not.toContain("current-quote");
    expect(dashboard?.placements.desktop).toMatchObject({ column: 1, columnSpan: 12, row: 6 });
    expect(dashboard?.placements.tablet).toMatchObject({ column: 1, columnSpan: 12, row: 8 });
    expect(detail?.placements.desktop?.visible).toBe(true);
  });
});
