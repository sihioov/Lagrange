import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { PriceChartBar } from "@/components/stock-beta/chart";
import {
  buildPriceChartGeometry,
  findNearestPriceChartIndexFromClientX,
  isFinitePriceChartBar,
  movePriceChartSelection,
  PriceChart,
} from "@/components/stock-beta/chart";
import { resolvePriceChartKeyboardSelection } from "@/components/stock-beta/chart/price-chart";

function sessionDate(index: number): string {
  return new Date(Date.UTC(2026, 0, 2 + index)).toISOString().slice(0, 10);
}

function makeBar(index: number, overrides: Partial<PriceChartBar> = {}): PriceChartBar {
  const open = 100 + index;
  const close = open + (index % 3 === 0 ? 2 : -1);
  return {
    close,
    high: Math.max(open, close) + 3,
    low: Math.min(open, close) - 2,
    open,
    session_date: sessionDate(index),
    sma_20: index >= 19 ? 100 + index / 2 : null,
    sma_60: index >= 59 ? 99 + index / 3 : null,
    volume: index === 1 ? 0 : 1_000 + index * 10,
    ...overrides,
  };
}

function finiteGeometryValues(value: unknown): number[] {
  if (typeof value === "number") return [value];
  if (Array.isArray(value)) return value.flatMap(finiteGeometryValues);
  if (typeof value === "object" && value !== null) {
    return Object.values(value).flatMap(finiteGeometryValues);
  }
  return [];
}

describe("stock-beta presentation-only price chart geometry", () => {
  it("keeps flat-price and zero-volume observations drawable without fake data", () => {
    const geometry = buildPriceChartGeometry([
      makeBar(0, {
        close: 10,
        high: 10,
        low: 10,
        open: 10,
        sma_20: null,
        sma_60: null,
        volume: 0,
      }),
    ]);

    expect(geometry.candles).toHaveLength(1);
    expect(geometry.candles[0]?.bodyHeight).toBeGreaterThan(0);
    expect(geometry.volumes[0]?.height).toBe(0);
    expect(geometry.priceDomain.max).toBeGreaterThan(geometry.priceDomain.min);
    expect(geometry.priceTicks.map((tick) => tick.value)).toEqual([10]);
    expect(geometry.sma20).toEqual([]);
    expect(geometry.sma60).toEqual([]);
    expect(finiteGeometryValues(geometry).every(Number.isFinite)).toBe(true);
  });

  it("guards invalid numeric bars and keeps null SMA values as visible gaps", () => {
    const bars = [
      makeBar(0, { sma_20: 100 }),
      makeBar(1, { sma_20: null }),
      makeBar(2, { sma_20: 102 }),
      makeBar(3, { high: Number.NaN, sma_20: 103 }),
      makeBar(4, { sma_20: 104 }),
    ];
    const geometry = buildPriceChartGeometry(bars);
    const invalidBar = bars[3];
    if (invalidBar === undefined) throw new Error("invalid fixture bar missing");

    expect(isFinitePriceChartBar(invalidBar)).toBe(false);
    expect(geometry.validSourceIndices).toEqual([0, 1, 2, 4]);
    expect(
      geometry.sma20.map((segment) => segment.points.map((point) => point.sourceIndex)),
    ).toEqual([[0], [2], [4]]);
    expect(JSON.stringify(geometry)).not.toMatch(/NaN|Infinity/);
  });

  it("maps pointer and touch client coordinates to the nearest rendered observation", () => {
    const geometry = buildPriceChartGeometry([makeBar(0), makeBar(1), makeBar(2)]);
    const middle = geometry.candles[1];
    if (middle === undefined) throw new Error("middle candle missing");
    const clientX = 40 + (middle.x / geometry.width) * 600;

    expect(findNearestPriceChartIndexFromClientX(clientX, 40, 600, geometry)).toBe(1);
    expect(findNearestPriceChartIndexFromClientX(40, 40, 600, geometry)).toBe(0);
    expect(findNearestPriceChartIndexFromClientX(Number.NaN, 40, 600, geometry)).toBeUndefined();
  });

  it("keeps observation navigation bounded and deterministic", () => {
    const indices = [2, 5, 9] as const;

    expect(movePriceChartSelection(5, "ArrowLeft", indices)).toBe(2);
    expect(movePriceChartSelection(5, "ArrowRight", indices)).toBe(9);
    expect(movePriceChartSelection(5, "Home", indices)).toBe(2);
    expect(movePriceChartSelection(5, "End", indices)).toBe(9);
    expect(movePriceChartSelection(2, "ArrowLeft", indices)).toBe(2);
    expect(movePriceChartSelection(9, "ArrowRight", indices)).toBe(9);
  });

  it("reveals the committed selection when keyboard navigation requests an unaccepted controlled value", () => {
    const indices = [2, 5, 9] as const;
    const selection = resolvePriceChartKeyboardSelection(5, 9, "ArrowLeft", indices, true);

    expect(selection).toEqual({ requestedIndex: 2, revealIndex: 5 });
  });

  it("reveals a boundary selection again when keyboard navigation requests no new index", () => {
    const indices = [2, 5, 9] as const;

    expect(resolvePriceChartKeyboardSelection(9, 9, "End", indices, false)).toEqual({
      requestedIndex: 9,
      revealIndex: 9,
    });
  });
});

describe("PriceChart SSR shell and accessible HTML contract", () => {
  it("renders a focusable HTML region, live summary, OHLCV tooltip, legend, and attribution", () => {
    const markup = renderToStaticMarkup(
      <PriceChart
        bars={[makeBar(0), makeBar(1, { close: 102, high: 104, open: 100 }), makeBar(2)]}
        copy={{
          chartLabel: "EOD price chart",
          instructions: "Use the arrow keys to inspect observations.",
          upLabel: "Rising",
        }}
        formatters={{
          date: (date) => `D:${date}`,
          price: (value) => `P:${value}`,
          volume: (value) => `V:${value}`,
        }}
      />,
    );

    expect(markup).toContain('data-testid="stock-beta-price-chart"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-surface"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-canvas"');
    expect(markup).toContain('data-renderer-status="loading"');
    expect(markup).toMatch(
      /<section[^>]*aria-label="EOD price chart"[^>]*data-testid="stock-beta-price-chart-surface"/,
    );
    expect(markup).toContain('tabindex="0"');
    expect(markup).not.toContain('role="button"');
    expect(markup).not.toMatch(/<button[^>]*data-testid="stock-beta-price-chart-surface"/);
    expect(markup).toContain('data-testid="stock-beta-price-chart-reset"');
    expect(markup).not.toContain("<svg");
    expect(markup).toContain('aria-keyshortcuts="ArrowLeft ArrowRight Home End"');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-summary"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-tooltip"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-legend"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-reset"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-attribution"');
    expect(markup).toContain("D:2026-01-04");
    expect(markup).toContain("Open");
    expect(markup).toContain("P:");
    expect(markup).toContain("Volume");
    expect(markup).toContain("V:");
    expect(markup).toContain("Rising");
    expect(markup).toContain("Down");
    expect(markup).toContain("SMA20");
    expect(markup).toContain("SMA60");
    expect(markup).toContain("TradingView Lightweight Charts™");
    expect(markup).toContain("Copyright (с) 2025 TradingView, Inc.");
    expect(markup).toContain('href="https://www.tradingview.com/"');
    expect(markup).toContain('href="/licenses/lightweight-charts/NOTICE.txt"');
    expect(markup).toContain('href="/licenses/lightweight-charts/LICENSE.txt"');
    expect(markup).not.toMatch(/NaN|Infinity/);
  });

  it("keeps the fixed chart shell and latest selection for a full 261-observation dataset", () => {
    const bars = Array.from({ length: 261 }, (_, index) => makeBar(index));
    const markup = renderToStaticMarkup(<PriceChart bars={bars} />);

    expect(markup).toContain('data-testid="stock-beta-price-chart-canvas"');
    expect(markup).toContain('data-selected-index="260"');
    expect(markup).toContain(`Date: ${sessionDate(260)}`);
    expect(markup).toContain('data-sma="20"');
    expect(markup).toContain('data-sma="60"');
    expect(markup).not.toContain("data-candle-index");
    expect(markup).not.toContain("data-volume-index");
  });

  it("preserves controlled and initial selection by original source index across skipped bars", () => {
    const bars = [makeBar(0), makeBar(1, { high: Number.NaN }), makeBar(2)];
    const initialMarkup = renderToStaticMarkup(<PriceChart bars={bars} initialSelectedIndex={0} />);
    const controlledMarkup = renderToStaticMarkup(<PriceChart bars={bars} selectedIndex={2} />);

    expect(initialMarkup).toContain('data-selected-index="0"');
    expect(initialMarkup).toContain(`Date: ${sessionDate(0)}`);
    expect(controlledMarkup).toContain('data-selected-index="2"');
    expect(controlledMarkup).toContain(`Date: ${sessionDate(2)}`);
  });

  it("fails closed to a textual empty state when no finite bars are renderable", () => {
    const invalidBar = makeBar(0, { close: Number.POSITIVE_INFINITY });
    const markup = renderToStaticMarkup(<PriceChart bars={[invalidBar]} />);

    expect(markup).toContain("No valid price observations are available.");
    expect(markup).toContain('data-selected-index="none"');
    expect(markup).toContain('data-testid="stock-beta-price-chart-canvas"');
    expect(markup).not.toMatch(/NaN|Infinity/);
  });
});
