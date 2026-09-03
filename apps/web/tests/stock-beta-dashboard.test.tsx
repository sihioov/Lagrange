import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  stockBetaDetailHref,
  stockBetaFilterQueryString,
} from "@/components/stock-beta/dashboard/filter-context";
import { StockBetaDashboard } from "@/components/stock-beta/dashboard/stock-beta-dashboard";
import { stockBetaDashboardArchitecture } from "@/components/stock-beta/dashboard/widget-registry";
import { validateStockBetaWidgetArchitecture } from "@/components/stock-beta/shared/widget-types";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import {
  type OwnerBetaEquitySignalsFilters,
  ownerBetaEquitySignalRowSchema,
  ownerBetaEquitySignalsFiltersSelected,
  ownerBetaEquitySignalsLatestSchema,
  ownerBetaEquitySignalsProvenanceSchema,
  ownerBetaEquitySignalsScreenSchema,
} from "@/lib/products/equity-signals-contracts";

const precisionRow = ownerBetaEquitySignalRowSchema.parse({
  average_trading_value_20: 1_000_000.1234,
  average_volume_20: 25_000.6789,
  condition: "BULLISH",
  instrument_id: "123456.KRX",
  instrument_name: "Precision ETF",
  max_drawdown_120: -0.3456,
  rank: 1,
  return_120: 0.4567,
  return_20: 0.2345,
  return_60: 0.2345,
  score: 1.2345,
  sma_20: 101.2345,
  sma_60: 99.8765,
  volatility_120: 0.3456,
  volatility_20: 0.1234,
  volatility_60: 0.2345,
  volume_ratio_20_60: 1.1111,
});

const precisionProvenance = ownerBetaEquitySignalsProvenanceSchema.parse({
  artifact_content_sha256: "artifact-precision",
  as_of: "2026-09-01",
  audience: "OWNER_ONLY",
  batch_id: "batch-precision",
  capability: "PRICE_VOLUME_RESEARCH_ONLY",
  entitlement_sha256: "entitlement-precision",
  factor_version: "v1",
  index_membership: "FIXED_LIST",
  materialization_status: "IMMUTABLE",
  original_price: true,
  publication_status: "PRIVATE",
  redistribution: "PRIVATE",
  registration_status: "APPROVED",
  registry_sha256: "registry-precision",
  selection_basis: "FIXED_LIST",
  snapshot_content_sha256: "snapshot-precision",
  strict_pit: false,
  universe_sha256: "universe-precision",
  vendor_snapshot: true,
  warning: "static test warning",
  activity_proxy: "TRADING_VALUE_PROXY",
});

const precisionData = ownerBetaEquitySignalsLatestSchema.parse({
  provenance: precisionProvenance,
  rows: [precisionRow],
  top5: [precisionRow],
});

const filteredPrecisionData = ownerBetaEquitySignalsScreenSchema.parse({
  provenance: precisionProvenance,
  rows: [precisionRow],
});

const precisionFilters: OwnerBetaEquitySignalsFilters = {
  conditions: [],
  ranges: {},
};

const { detailTitle: _detailTitle, ...precisionCopy } = stockBetaDictionary.en;
const { detailTitle: _koreanDetailTitle, ...koreanPrecisionCopy } = stockBetaDictionary.ko;

describe("stock-beta dashboard composition", () => {
  it("keeps the dashboard registry valid and its required policy/data regions explicit", () => {
    expect(validateStockBetaWidgetArchitecture(stockBetaDashboardArchitecture)).toEqual([]);
    expect(stockBetaDashboardArchitecture.requiredWidgetIds).toEqual([
      "policy-boundary",
      "ranked-signals",
      "signal-profile",
      "signal-decomposition",
      "condition-matrix",
      "snapshot-tape",
      "provenance",
    ]);
  });

  it("reconstructs detail links from only the approved stock-beta filter keys", () => {
    const filters: OwnerBetaEquitySignalsFilters = {
      conditions: ["BULLISH", "BEARISH"],
      ranges: {
        return_20: { min: 0.1, max: 0.2 },
        score: { min: 75 },
      },
      trendUp: true,
    };

    expect(stockBetaFilterQueryString(filters)).toBe(
      "condition=BULLISH&condition=BEARISH&score_min=75&return_20_min=0.1&return_20_max=0.2&trend=up",
    );
    expect(stockBetaDetailHref("000001.KRX", filters)).toBe(
      "/stock-beta/000001.KRX?condition=BULLISH&condition=BEARISH&score_min=75&return_20_min=0.1&return_20_max=0.2&trend=up",
    );
    expect(stockBetaDetailHref("000001.KRX", filters)).not.toContain("return_to");
  });

  it("keeps high-precision DTO values in both table and selected-preview DOM", () => {
    const markup = renderToStaticMarkup(
      <StockBetaDashboard
        copy={precisionCopy}
        data={precisionData}
        filters={precisionFilters}
        locale="en"
      />,
    );

    expect(ownerBetaEquitySignalsFiltersSelected(precisionFilters)).toBe(false);
    expect(markup.match(/data-raw-value="1\.2345"/g)).toHaveLength(2);
    expect(markup.match(/<data value="1\.2345">1\.23<\/data>/g)).toHaveLength(3);
    expect(markup.match(/API value: 1\.2345/g)).toHaveLength(2);
  });

  it("emphasizes the API top5 for the latest response and reports its actual row count", () => {
    const markup = renderToStaticMarkup(
      <StockBetaDashboard
        copy={precisionCopy}
        data={precisionData}
        filters={precisionFilters}
        locale="en"
      />,
    );

    expect(markup).toContain('data-testid="stock-beta-top-five"');
    expect(markup.match(/data-top-five="true"/g)).toHaveLength(1);
    expect(markup).toContain(
      "Ranked price-and-volume signal table · Result count: 1 · Configured results",
    );
    expect(markup).not.toContain("30-row");
  });

  it("describes filtered leaders as a current-result subset and reports its actual row count", () => {
    const markup = renderToStaticMarkup(
      <StockBetaDashboard
        copy={precisionCopy}
        data={filteredPrecisionData}
        filters={{ conditions: ["BULLISH"], ranges: {} }}
        locale="en"
      />,
    );

    expect(markup).toContain('data-testid="stock-beta-current-result-leaders"');
    expect(markup).toContain("Current-result leaders");
    expect(markup).toContain(
      "Up to five rows from these current results, in the order returned by the server.",
    );
    expect(markup).not.toContain('data-testid="stock-beta-top-five"');
    expect(markup.match(/data-current-result-leader="true"/g)).toHaveLength(1);
    expect(markup).toContain(
      "Ranked price-and-volume signal table · Result count: 1 · Current results",
    );
    expect(markup).not.toContain("Top 5");
    expect(markup).not.toContain("30-row");

    const koreanMarkup = renderToStaticMarkup(
      <StockBetaDashboard
        copy={koreanPrecisionCopy}
        data={filteredPrecisionData}
        filters={{ conditions: ["BULLISH"], ranges: {} }}
        locale="ko"
      />,
    );

    expect(koreanMarkup).toContain("현재 결과 선두 행");
    expect(koreanMarkup).toContain("현재 결과에서 서버가 반환한 순서대로 최대 5개 행입니다.");
    expect(koreanMarkup).toContain("가격·거래량 신호 순위 표 · 결과 수: 1 · 현재 결과");
  });

  it("keeps the dashboard copy serializable for client selection widgets", () => {
    expect(Object.values(precisionCopy).every((value) => typeof value !== "function")).toBe(true);
    expect(Object.values(koreanPrecisionCopy).every((value) => typeof value !== "function")).toBe(
      true,
    );
  });

  it("keeps the dashboard shell and static widgets server-compatible", () => {
    const dashboardRoot = join(process.cwd(), "components/stock-beta/dashboard");
    const clientFiles = [
      "instrument-search.tsx",
      "column-visibility-control.tsx",
      "selection-provider.tsx",
      "widgets/condition-matrix-widget.tsx",
      "widgets/ranked-signals-widget.tsx",
      "widgets/signal-decomposition-widget.tsx",
      "widgets/signal-preview-widget.tsx",
    ];
    const dashboardFiles = [
      ...readdirSync(dashboardRoot, { withFileTypes: true })
        .filter((entry) => entry.isFile() && /\.tsx?$/.test(entry.name))
        .map((entry) => entry.name),
      ...readdirSync(join(dashboardRoot, "widgets"), { withFileTypes: true })
        .filter((entry) => entry.isFile() && /\.tsx?$/.test(entry.name))
        .map((entry) => `widgets/${entry.name}`),
    ];
    const actualClientFiles = dashboardFiles.filter((file) =>
      readFileSync(join(dashboardRoot, file), "utf8").trimStart().startsWith('"use client"'),
    );

    expect(actualClientFiles.sort()).toEqual(clientFiles.sort());
  });
});
