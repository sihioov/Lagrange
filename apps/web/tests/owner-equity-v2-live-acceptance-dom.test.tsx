import { chromium, type Page } from "playwright";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { renderStockBetaDetailGrid } from "@/components/stock-beta/detail/stock-beta-detail-layout";
import type {
  StockBetaDetailViewModel,
  StockBetaDetailWidgetViewModel,
} from "@/components/stock-beta/detail/types";
import { InstrumentHeaderWidget } from "@/components/stock-beta/detail/widgets/instrument-header-widget";
import { CurrentQuoteView } from "@/components/stock-beta/quote/current-quote-view";
import type { IntradayQuoteLoadState } from "@/components/stock-beta/quote/quote-load-coordinator";
import {
  defineStockBetaWidgetArchitecture,
  defineStockBetaWidgetCatalog,
  type StockBetaWidgetProps,
} from "@/components/stock-beta/shared/widget-types";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { ownerEquityV2SignalDetailSchema } from "@/lib/products/equity-signals-contracts";
import { intradayQuoteResponseSchema } from "@/lib/products/intraday-quotes-contracts";
import { extractQuoteDom } from "../../../scripts/qa/owner-equity-v2-live-acceptance.mjs";

const INSTRUMENT_ID = "069500.KRX";
const LAST_SUCCESS_AT = "2026-09-18T01:00:31.000Z";
const MEMBERSHIP_ID = "00000000-0000-4000-8000-000000000020";

const DETAIL = ownerEquityV2SignalDetailSchema.parse({
  signal: {
    average_trading_value_20: 1_000_000,
    average_volume_20: 25_000,
    condition: "BULLISH",
    generation: 7,
    instrument_id: INSTRUMENT_ID,
    max_drawdown_120: -0.3,
    rank: 1,
    return_120: 0.4,
    return_20: 0.1,
    return_60: 0.2,
    score: 1.2,
    sma_20: 101,
    sma_60: 99,
    volatility_120: 0.3,
    volatility_20: 0.1,
    volatility_60: 0.2,
    volume_ratio_20_60: 1.1,
  },
  snapshot: {
    as_of: "2026-09-07",
    published_at: "2026-09-08T00:00:00Z",
    row_count: 1,
    snapshot_id: "00000000-0000-4000-8000-000000000010",
    universe_sha256: "sha256:fixture",
  },
});

const QUOTE_RESPONSE = intradayQuoteResponseSchema.parse({
  currency: "KRW",
  freshness: "RECENT",
  generation: 7,
  instrument_id: INSTRUMENT_ID,
  market_state: "OPEN",
  membership_id: MEMBERSHIP_ID,
  next_poll_after_ms: 5_000,
  quote: {
    base_price: "99.00",
    change_from_previous_day: "1.00",
    change_percent_from_previous_day: "1.01010101",
    direction: "UP",
    last_success_at: LAST_SUCCESS_AT,
    price: "100.00",
    quote_version: "11",
    received_at: LAST_SUCCESS_AT,
  },
  reason_code: null,
  schema_version: 1,
  session: {
    calendar_content_sha256: "a".repeat(64),
    calendar_source: "kis",
    calendar_source_version: "kis-chk-holiday-v1:schema-1",
    date: "2026-09-18",
    timezone: "Asia/Seoul",
    window_contract_sha256: `sha256:${"b".repeat(64)}`,
  },
  venue: "KRX",
});

const READY_QUOTE_STATE: IntradayQuoteLoadState = {
  consumerId: null,
  errorCode: null,
  fetching: false,
  identity: null,
  lastSuccessAt: LAST_SUCCESS_AT,
  marketState: "OPEN",
  phase: "ready",
  quote: QUOTE_RESPONSE,
  reasonCode: null,
};

const VIEW_MODEL: StockBetaDetailViewModel = {
  backHref: "/stock-beta",
  copy: stockBetaDictionary.en,
  detail: DETAIL,
  intradayEnabled: true,
  intradayMembership: null,
  locale: "en",
};

function RenderedCurrentQuote({ viewModel }: StockBetaWidgetProps<StockBetaDetailWidgetViewModel>) {
  return (
    <CurrentQuoteView copy={viewModel.copy} locale={viewModel.locale} state={READY_QUOTE_STATE} />
  );
}

const defineCatalog = defineStockBetaWidgetCatalog<StockBetaDetailWidgetViewModel>();
const ACTUAL_RENDER_CATALOG = defineCatalog([
  {
    id: "instrument-header",
    component: InstrumentHeaderWidget,
    required: true,
    placements: {
      desktop: { size: "full", visible: true },
      mobile: { size: "full", visible: true },
      tablet: { size: "full", visible: true },
    },
  },
  {
    id: "current-quote",
    component: RenderedCurrentQuote,
    required: false,
    placements: {
      desktop: { size: "full", visible: true },
      mobile: { size: "full", visible: true },
      tablet: { size: "full", visible: true },
    },
  },
]);

const ACTUAL_RENDER_ARCHITECTURE = defineStockBetaWidgetArchitecture(ACTUAL_RENDER_CATALOG);

type ExtractedQuote = {
  instrument_id: string;
  last_success_at: string;
  price: string;
  status_phase: string;
} | null;

function actualMarkup(): string {
  return renderToStaticMarkup(renderStockBetaDetailGrid(ACTUAL_RENDER_ARCHITECTURE, VIEW_MODEL));
}

async function extractInActualBrowser(
  page: Page,
  expectedInstrument = INSTRUMENT_ID,
): Promise<ExtractedQuote> {
  return page.evaluate(extractQuoteDom, expectedInstrument) as Promise<ExtractedQuote>;
}

async function setActualMarkup(page: Page): Promise<void> {
  await page.setContent(`<main data-synthetic-dom-fixture="true">${actualMarkup()}</main>`);
}

describe("SYNTHETIC local Chromium DOM boundary for owner equity v2 receipts", () => {
  it("runs production extractQuoteDom against actual React SSR CurrentQuoteView, WidgetFrame, and detail wrapper markup", async () => {
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage();

      await setActualMarkup(page);
      await expect(extractInActualBrowser(page)).resolves.toEqual({
        instrument_id: INSTRUMENT_ID,
        last_success_at: LAST_SUCCESS_AT,
        price: "100.00",
        status_phase: "ready",
      });
      await expect(extractInActualBrowser(page, "005930.KRX")).resolves.toBeNull();

      await setActualMarkup(page);
      await page
        .locator('[data-testid="stock-beta-detail-widget-current-quote"]')
        .evaluate((element) => {
          (element as HTMLElement).style.display = "none";
        });
      await expect(extractInActualBrowser(page)).resolves.toBeNull();

      await setActualMarkup(page);
      await page.locator("[data-status-phase]").evaluate((element) => {
        element.setAttribute("data-status-phase", "stale");
      });
      await expect(extractInActualBrowser(page)).resolves.toBeNull();

      await setActualMarkup(page);
      await page
        .locator('[data-testid="stock-beta-detail-widget-current-quote"]')
        .evaluate((element) => {
          element.setAttribute("data-widget-id", "wrong-widget");
        });
      await expect(extractInActualBrowser(page)).resolves.toBeNull();

      await setActualMarkup(page);
      await page.evaluate(() => {
        document.body.insertAdjacentHTML(
          "afterbegin",
          '<div data-testid="stock-beta-detail-widget-current-quote" data-widget-id="current-quote">decoy outside board</div>',
        );
      });
      await expect(extractInActualBrowser(page)).resolves.toMatchObject({
        instrument_id: INSTRUMENT_ID,
      });
      await page.locator('[data-testid="stock-beta-detail-board"]').evaluate((element) => {
        element.removeAttribute("data-testid");
      });
      await expect(extractInActualBrowser(page)).resolves.toBeNull();
    } finally {
      await browser.close();
    }
  });
});
