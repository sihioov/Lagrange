import { describe, expect, it } from "vitest";
import {
  buildLightweightChartData,
  type LightweightChartInvalidReason,
} from "@/components/stock-beta/chart/lightweight-data";
import type { PriceChartBar } from "@/components/stock-beta/chart/types";

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
    sma_20: index >= 19 ? 100.25 + index / 2 : null,
    sma_60: index >= 59 ? 90.5 + index / 3 : null,
    volume: index === 1 ? 0 : 1_000 + index * 10,
    ...overrides,
  };
}

describe("Stock Beta Lightweight Charts data adapter", () => {
  it("imports in the Node test environment without browser globals", () => {
    expect(typeof window).toBe("undefined");
    expect(typeof buildLightweightChartData).toBe("function");
  });

  it("preserves all 261 daily observations and original numeric values", () => {
    const bars = Array.from({ length: 261 }, (_, index) => makeBar(index));
    const result = buildLightweightChartData(bars);

    expect(result.status).toBe("ready");
    if (result.status !== "ready") throw new Error("expected ready chart data");

    expect(result.candles).toHaveLength(261);
    expect(result.volumes).toHaveLength(261);
    expect(result.validSourceIndices).toEqual(Array.from({ length: 261 }, (_, index) => index));
    expect(result.candles[0]).toEqual({
      time: bars[0]?.session_date,
      open: bars[0]?.open,
      high: bars[0]?.high,
      low: bars[0]?.low,
      close: bars[0]?.close,
    });
    expect(result.candles[260]?.time).toBe(bars[260]?.session_date);
    expect(result.candles[260]?.close).toBe(bars[260]?.close);
    expect(result.volumes[260]?.value).toBe(bars[260]?.volume);
    expect(result.sma20Segments.map((segment) => segment.length)).toEqual([242]);
    expect(result.sma20Segments[0]?.[0]).toEqual({ time: bars[19]?.session_date, value: 109.75 });
    expect(result.sma60Segments.map((segment) => segment.length)).toEqual([202]);
    expect(result.dateToOriginalIndex.get(bars[260]?.session_date ?? "")).toBe(260);
    expect(result.originalIndexToDate.get(260)).toBe(bars[260]?.session_date);
  });

  it("keeps a flat, zero-volume candle without inventing values", () => {
    const flat = makeBar(0, {
      open: 42,
      high: 42,
      low: 42,
      close: 42,
      volume: 0,
      sma_20: null,
      sma_60: null,
    });
    const result = buildLightweightChartData([flat]);

    expect(result.status).toBe("ready");
    if (result.status !== "ready") throw new Error("expected ready chart data");
    expect(result.candles).toEqual([
      { time: flat.session_date, open: 42, high: 42, low: 42, close: 42 },
    ]);
    expect(result.volumes).toEqual([{ time: flat.session_date, value: 0 }]);
    expect(result.sma20Segments).toEqual([]);
    expect(result.sma60Segments).toEqual([]);
  });

  it("splits both moving averages at leading and interior null observations", () => {
    const bars = [
      makeBar(0, { sma_20: null, sma_60: null }),
      makeBar(1, { sma_20: null, sma_60: null }),
      makeBar(2, { sma_20: 102, sma_60: null }),
      makeBar(3, { sma_20: 103, sma_60: 203 }),
      makeBar(4, { sma_20: null, sma_60: 204 }),
      makeBar(5, { sma_20: 105, sma_60: null }),
      makeBar(6, { sma_20: 106, sma_60: 206 }),
      makeBar(7, { sma_20: null, sma_60: 207 }),
      makeBar(8, { sma_20: 108, sma_60: 208 }),
    ];
    const result = buildLightweightChartData(bars);

    expect(result.status).toBe("ready");
    if (result.status !== "ready") throw new Error("expected ready chart data");
    expect(result.sma20Segments.map((segment) => segment.map((point) => point.time))).toEqual([
      [sessionDate(2), sessionDate(3)],
      [sessionDate(5), sessionDate(6)],
      [sessionDate(8)],
    ]);
    expect(result.sma60Segments.map((segment) => segment.map((point) => point.time))).toEqual([
      [sessionDate(3), sessionDate(4)],
      [sessionDate(6), sessionDate(7), sessionDate(8)],
    ]);
  });

  it("skips invalid OHLC and non-finite SMA bars while preserving original indices and gaps", () => {
    const bars = [
      makeBar(0, { sma_20: 100 }),
      makeBar(1, { high: 90, sma_20: 101 }),
      makeBar(2, { sma_20: 102 }),
      makeBar(3, { sma_20: Number.POSITIVE_INFINITY }),
      makeBar(4, { sma_20: 104 }),
    ];
    const result = buildLightweightChartData(bars);

    expect(result.status).toBe("ready");
    if (result.status !== "ready") throw new Error("expected ready chart data");
    expect(result.validSourceIndices).toEqual([0, 2, 4]);
    expect(result.candles.map((candle) => candle.time)).toEqual([
      sessionDate(0),
      sessionDate(2),
      sessionDate(4),
    ]);
    expect(result.sma20Segments.map((segment) => segment.map((point) => point.time))).toEqual([
      [sessionDate(0)],
      [sessionDate(2)],
      [sessionDate(4)],
    ]);
    expect(result.dateToOriginalIndex.get(sessionDate(2))).toBe(2);
    expect(result.originalIndexToDate.get(2)).toBe(sessionDate(2));
    expect(JSON.stringify(result)).not.toMatch(/NaN|Infinity/);
  });

  it("returns a typed empty result when every input bar is invalid", () => {
    const result = buildLightweightChartData([
      makeBar(0, { close: Number.POSITIVE_INFINITY }),
      makeBar(1, { high: 0 }),
    ]);

    expect(result).toMatchObject({ status: "empty", reason: "no-valid-bars" });
    expect(result.candles).toEqual([]);
    expect(result.volumes).toEqual([]);
    expect(result.sma20Segments).toEqual([]);
    expect(result.sma60Segments).toEqual([]);
    expect(result.validSourceIndices).toEqual([]);
    expect(result.dateToOriginalIndex.size).toBe(0);
    expect(result.originalIndexToDate.size).toBe(0);
  });

  it("ignores an invalid date on a numeric/OHLC bar that is not renderable", () => {
    const result = buildLightweightChartData([
      makeBar(0),
      makeBar(1, { session_date: "not-a-date", close: Number.NaN }),
      makeBar(2),
    ]);

    expect(result.status).toBe("ready");
    expect(result.validSourceIndices).toEqual([0, 2]);
  });

  it.each([
    {
      label: "invalid calendar date",
      bars: [makeBar(0, { session_date: "2026-02-29" })],
      reason: "invalid-date",
    },
    {
      label: "duplicate date",
      bars: [makeBar(0), makeBar(1, { session_date: sessionDate(0) })],
      reason: "duplicate-date",
    },
    {
      label: "reversed dates",
      bars: [makeBar(1), makeBar(0)],
      reason: "reversed-date",
    },
  ] satisfies ReadonlyArray<{
    readonly label: string;
    readonly bars: readonly PriceChartBar[];
    readonly reason: LightweightChartInvalidReason;
  }>)("returns empty typed invalid data for $label", ({ bars, reason }) => {
    const result = buildLightweightChartData(bars);

    expect(result).toMatchObject({ status: "invalid", reason });
    expect(result.candles).toEqual([]);
    expect(result.volumes).toEqual([]);
    expect(result.validSourceIndices).toEqual([]);
    expect(result.dateToOriginalIndex.size).toBe(0);
    expect(result.originalIndexToDate.size).toBe(0);
  });

  it("does not mutate readonly or frozen input bars", () => {
    const bars: readonly PriceChartBar[] = Object.freeze([
      Object.freeze(makeBar(0, { sma_20: null })),
      Object.freeze(makeBar(1, { sma_20: 101 })),
      Object.freeze(makeBar(2, { high: 0, sma_20: 102 })),
    ]);
    const before = structuredClone(bars);

    buildLightweightChartData(bars);

    expect(bars).toEqual(before);
  });
});
