import type { CandlestickData, HistogramData, LineData, Time } from "lightweight-charts";
import { isFinitePriceChartBar } from "./geometry";
import type { PriceChartBar } from "./types";

export type LightweightChartCandle = CandlestickData<Time>;
export type LightweightChartVolume = HistogramData<Time>;
export type LightweightChartSmaPoint = LineData<Time>;

/** Shared, empty-safe series and index mapping fields returned by the adapter. */
export type LightweightChartSeriesData = {
  /** Daily chart times remain the original YYYY-MM-DD strings. */
  readonly candles: readonly LightweightChartCandle[];
  readonly volumes: readonly LightweightChartVolume[];
  /** Each inner array is one contiguous run and must become its own line series. */
  readonly sma20Segments: readonly (readonly LightweightChartSmaPoint[])[];
  /** Each inner array is one contiguous run and must become its own line series. */
  readonly sma60Segments: readonly (readonly LightweightChartSmaPoint[])[];
  /** Original positions in the caller's bars array, in rendered order. */
  readonly validSourceIndices: readonly number[];
  /** Maps each daily time to its original bars-array position. */
  readonly dateToOriginalIndex: ReadonlyMap<Time, number>;
  /** Maps each original bars-array position back to its unchanged daily date. */
  readonly originalIndexToDate: ReadonlyMap<number, string>;
};

export type LightweightChartInvalidReason = "invalid-date" | "duplicate-date" | "reversed-date";

export type LightweightChartDataResult =
  | (LightweightChartSeriesData & { readonly status: "ready" })
  | (LightweightChartSeriesData & {
      readonly status: "empty";
      readonly reason: "no-valid-bars";
    })
  | (LightweightChartSeriesData & {
      readonly status: "invalid";
      readonly reason: LightweightChartInvalidReason;
      readonly sourceIndex: number;
      readonly sessionDate: string;
      readonly previousSessionDate: string | null;
    });

type ValidEntry = {
  readonly sourceIndex: number;
  readonly bar: PriceChartBar;
  readonly time: Time;
};

function isIsoCalendarDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (match === null) return false;

  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12 || day < 1) return false;

  const leapYear = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const daysInMonth = [31, leapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  const monthLength = daysInMonth[month - 1];
  return monthLength !== undefined && day <= monthLength;
}

function emptySeriesData(): LightweightChartSeriesData {
  return {
    candles: [],
    volumes: [],
    sma20Segments: [],
    sma60Segments: [],
    validSourceIndices: [],
    dateToOriginalIndex: new Map<Time, number>(),
    originalIndexToDate: new Map<number, string>(),
  };
}

function buildSmaSegments(
  entries: readonly ValidEntry[],
  field: "sma_20" | "sma_60",
): LightweightChartSmaPoint[][] {
  const segments: LightweightChartSmaPoint[][] = [];
  let points: LightweightChartSmaPoint[] = [];
  let previousSourceIndex: number | undefined;

  const finishSegment = () => {
    if (points.length > 0) segments.push(points);
    points = [];
  };

  for (const entry of entries) {
    if (previousSourceIndex !== undefined && entry.sourceIndex !== previousSourceIndex + 1) {
      finishSegment();
    }
    previousSourceIndex = entry.sourceIndex;

    const value = entry.bar[field];
    if (value === null || !Number.isFinite(value)) {
      finishSegment();
      continue;
    }

    points.push({ time: entry.time, value });
  }

  finishSegment();
  return segments;
}

/**
 * Converts presentation bars to Lightweight Charts series data without changing
 * source values or dates. Invalid numeric/OHLC bars are skipped with their
 * original indices retained; invalid date order fails closed with empty series.
 */
export function buildLightweightChartData(
  bars: readonly PriceChartBar[],
): LightweightChartDataResult {
  const inputBars = Array.isArray(bars) ? bars : [];
  const entries: ValidEntry[] = [];

  for (let sourceIndex = 0; sourceIndex < inputBars.length; sourceIndex += 1) {
    const bar = inputBars[sourceIndex];
    if (bar === undefined || !isFinitePriceChartBar(bar)) continue;

    const date = bar.session_date;
    if (!isIsoCalendarDate(date)) {
      return {
        ...emptySeriesData(),
        status: "invalid",
        reason: "invalid-date",
        sourceIndex,
        sessionDate: date,
        previousSessionDate: entries.at(-1)?.bar.session_date ?? null,
      };
    }

    const previousDate = entries.at(-1)?.bar.session_date;
    if (previousDate !== undefined && date <= previousDate) {
      return {
        ...emptySeriesData(),
        status: "invalid",
        reason: date === previousDate ? "duplicate-date" : "reversed-date",
        sourceIndex,
        sessionDate: date,
        previousSessionDate: previousDate,
      };
    }

    entries.push({ sourceIndex, bar, time: date });
  }

  if (entries.length === 0) {
    return { ...emptySeriesData(), status: "empty", reason: "no-valid-bars" };
  }

  const candles: LightweightChartCandle[] = [];
  const volumes: LightweightChartVolume[] = [];
  const validSourceIndices: number[] = [];
  const dateToOriginalIndex = new Map<Time, number>();
  const originalIndexToDate = new Map<number, string>();

  for (const { sourceIndex, bar, time } of entries) {
    candles.push({
      time,
      open: bar.open,
      high: bar.high,
      low: bar.low,
      close: bar.close,
    });
    volumes.push({ time, value: bar.volume });
    validSourceIndices.push(sourceIndex);
    dateToOriginalIndex.set(time, sourceIndex);
    originalIndexToDate.set(sourceIndex, bar.session_date);
  }

  return {
    status: "ready",
    candles,
    volumes,
    sma20Segments: buildSmaSegments(entries, "sma_20"),
    sma60Segments: buildSmaSegments(entries, "sma_60"),
    validSourceIndices,
    dateToOriginalIndex,
    originalIndexToDate,
  };
}
