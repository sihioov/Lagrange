import type { PriceChartBar, PriceChartDirection } from "./types";

export type PriceChartMargin = {
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
  readonly left: number;
};

export type PriceChartGeometryOptions = {
  readonly width?: number;
  readonly height?: number;
  readonly margin?: Partial<PriceChartMargin>;
  readonly volumeHeight?: number;
  readonly gap?: number;
  readonly maxPriceTicks?: number;
  readonly maxDateTicks?: number;
};

export const DEFAULT_PRICE_CHART_DIMENSIONS = {
  width: 960,
  height: 440,
  margin: {
    top: 18,
    right: 76,
    bottom: 48,
    left: 64,
  },
  volumeHeight: 74,
  gap: 18,
} as const satisfies Required<
  Pick<PriceChartGeometryOptions, "width" | "height" | "volumeHeight" | "gap">
> & { readonly margin: PriceChartMargin };

export type PriceChartCandleGeometry = {
  readonly sourceIndex: number;
  readonly x: number;
  readonly bodyX: number;
  readonly bodyWidth: number;
  readonly bodyY: number;
  readonly bodyHeight: number;
  readonly highY: number;
  readonly lowY: number;
  readonly openY: number;
  readonly closeY: number;
  readonly direction: PriceChartDirection;
};

export type PriceChartVolumeGeometry = {
  readonly sourceIndex: number;
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly direction: PriceChartDirection;
};

export type PriceChartLinePoint = {
  readonly sourceIndex: number;
  readonly x: number;
  readonly y: number;
};

export type PriceChartLineSegment = {
  readonly points: readonly PriceChartLinePoint[];
};

export type PriceChartPriceTick = {
  readonly value: number;
  readonly y: number;
};

export type PriceChartDateTick = {
  readonly sourceIndex: number;
  readonly sessionDate: string;
  readonly x: number;
};

export type PriceChartGeometry = {
  readonly width: number;
  readonly height: number;
  readonly plotLeft: number;
  readonly plotRight: number;
  readonly plotWidth: number;
  readonly priceTop: number;
  readonly priceBottom: number;
  readonly volumeTop: number;
  readonly volumeBottom: number;
  readonly priceDomain: { readonly min: number; readonly max: number };
  readonly priceDataMin: number;
  readonly priceDataMax: number;
  readonly volumeDomain: { readonly min: number; readonly max: number };
  readonly volumeMax: number;
  readonly candles: readonly PriceChartCandleGeometry[];
  readonly volumes: readonly PriceChartVolumeGeometry[];
  readonly sma20: readonly PriceChartLineSegment[];
  readonly sma60: readonly PriceChartLineSegment[];
  readonly priceTicks: readonly PriceChartPriceTick[];
  readonly dateTicks: readonly PriceChartDateTick[];
  readonly validSourceIndices: readonly number[];
};

type ResolvedLayout = {
  readonly width: number;
  readonly height: number;
  readonly plotLeft: number;
  readonly plotRight: number;
  readonly plotWidth: number;
  readonly priceTop: number;
  readonly priceBottom: number;
  readonly volumeTop: number;
  readonly volumeBottom: number;
  readonly maxPriceTicks: number;
  readonly maxDateTicks: number;
};

type PriceChartEntry = {
  readonly sourceIndex: number;
  readonly bar: PriceChartBar;
};

type PriceChartEntryWithX = PriceChartEntry & { readonly x: number };

type Domain = { readonly min: number; readonly max: number };

function finite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function positiveFinite(value: unknown): value is number {
  return finite(value) && value > 0;
}

function positiveInteger(value: unknown): value is number {
  return positiveFinite(value) && Number.isSafeInteger(value);
}

function nullableFinite(value: unknown): value is number | null {
  return value === null || finite(value);
}

export function isFinitePriceChartBar(bar: PriceChartBar): boolean {
  if (bar === null || typeof bar !== "object") return false;
  if (typeof bar.session_date !== "string") return false;
  if (
    !finite(bar.open) ||
    !finite(bar.high) ||
    !finite(bar.low) ||
    !finite(bar.close) ||
    !finite(bar.volume) ||
    bar.volume < 0 ||
    !nullableFinite(bar.sma_20) ||
    !nullableFinite(bar.sma_60)
  ) {
    return false;
  }

  return (
    bar.high >= bar.low &&
    bar.open >= bar.low &&
    bar.open <= bar.high &&
    bar.close >= bar.low &&
    bar.close <= bar.high
  );
}

export function directionForPriceChartBar(bar: PriceChartBar): PriceChartDirection {
  if (bar.close > bar.open) return "up";
  if (bar.close < bar.open) return "down";
  return "flat";
}

function resolvedLayout(options?: PriceChartGeometryOptions): ResolvedLayout | undefined {
  const width = options?.width ?? DEFAULT_PRICE_CHART_DIMENSIONS.width;
  const height = options?.height ?? DEFAULT_PRICE_CHART_DIMENSIONS.height;
  const volumeHeight = options?.volumeHeight ?? DEFAULT_PRICE_CHART_DIMENSIONS.volumeHeight;
  const gap = options?.gap ?? DEFAULT_PRICE_CHART_DIMENSIONS.gap;
  const maxPriceTicks = options?.maxPriceTicks ?? 5;
  const maxDateTicks = options?.maxDateTicks ?? 6;
  const margin = options?.margin;
  const top = margin?.top ?? DEFAULT_PRICE_CHART_DIMENSIONS.margin.top;
  const right = margin?.right ?? DEFAULT_PRICE_CHART_DIMENSIONS.margin.right;
  const bottom = margin?.bottom ?? DEFAULT_PRICE_CHART_DIMENSIONS.margin.bottom;
  const left = margin?.left ?? DEFAULT_PRICE_CHART_DIMENSIONS.margin.left;

  if (
    !positiveFinite(width) ||
    !positiveFinite(height) ||
    !positiveFinite(volumeHeight) ||
    !finite(gap) ||
    gap < 0 ||
    !positiveInteger(maxPriceTicks) ||
    !positiveInteger(maxDateTicks) ||
    !finite(top) ||
    !finite(right) ||
    !finite(bottom) ||
    !finite(left) ||
    top < 0 ||
    right < 0 ||
    bottom < 0 ||
    left < 0
  ) {
    return undefined;
  }

  const plotLeft = left;
  const plotRight = width - right;
  const plotWidth = plotRight - plotLeft;
  const volumeBottom = height - bottom;
  const volumeTop = volumeBottom - volumeHeight;
  const priceBottom = volumeTop - gap;
  const priceTop = top;

  if (
    !positiveFinite(plotWidth) ||
    !positiveFinite(priceBottom - priceTop) ||
    !positiveFinite(volumeBottom - volumeTop)
  ) {
    return undefined;
  }

  return {
    width,
    height,
    plotLeft,
    plotRight,
    plotWidth,
    priceTop,
    priceBottom,
    volumeTop,
    volumeBottom,
    maxPriceTicks,
    maxDateTicks,
  };
}

function emptyGeometry(layout?: ResolvedLayout): PriceChartGeometry {
  return {
    width: layout?.width ?? 0,
    height: layout?.height ?? 0,
    plotLeft: layout?.plotLeft ?? 0,
    plotRight: layout?.plotRight ?? 0,
    plotWidth: layout?.plotWidth ?? 0,
    priceTop: layout?.priceTop ?? 0,
    priceBottom: layout?.priceBottom ?? 0,
    volumeTop: layout?.volumeTop ?? 0,
    volumeBottom: layout?.volumeBottom ?? 0,
    priceDomain: { min: 0, max: 0 },
    priceDataMin: 0,
    priceDataMax: 0,
    volumeDomain: { min: 0, max: 0 },
    volumeMax: 0,
    candles: [],
    volumes: [],
    sma20: [],
    sma60: [],
    priceTicks: [],
    dateTicks: [],
    validSourceIndices: [],
  };
}

function dataDomain(minimum: number, maximum: number): Domain | undefined {
  if (!finite(minimum) || !finite(maximum) || maximum < minimum) return undefined;
  if (minimum !== maximum) return { min: minimum, max: maximum };

  const padding = Math.max(Math.abs(minimum) * 0.02, 1);
  const min = minimum - padding;
  const max = maximum + padding;
  if (!finite(min) || !finite(max) || max <= min) return undefined;
  return { min, max };
}

function scaleValue(
  value: number,
  domain: Domain,
  rangeStart: number,
  rangeEnd: number,
): number | undefined {
  if (!finite(value) || !finite(domain.min) || !finite(domain.max)) return undefined;
  const span = domain.max - domain.min;
  if (!finite(span) || span <= 0 || !finite(rangeStart) || !finite(rangeEnd)) return undefined;
  const normalized = (value - domain.min) / span;
  if (!finite(normalized)) return undefined;
  const result = rangeStart + normalized * (rangeEnd - rangeStart);
  return finite(result) ? result : undefined;
}

function priceTicks(
  domain: Domain,
  layout: ResolvedLayout,
  priceY: (value: number) => number | undefined,
): readonly PriceChartPriceTick[] {
  if (domain.min === domain.max) {
    const y = priceY(domain.min);
    return y === undefined ? [] : [{ value: domain.min, y }];
  }

  const span = domain.max - domain.min;
  if (!finite(span)) return [];
  const ticks: PriceChartPriceTick[] = [];
  for (let index = 0; index < layout.maxPriceTicks; index += 1) {
    const ratio = layout.maxPriceTicks === 1 ? 0 : index / (layout.maxPriceTicks - 1);
    const value = domain.max - span * ratio;
    const y = priceY(value);
    if (finite(value) && y !== undefined) ticks.push({ value, y });
  }
  return ticks;
}

function dateTicks(
  entries: readonly PriceChartEntry[],
  layout: ResolvedLayout,
  xForPosition: (position: number) => number,
): readonly PriceChartDateTick[] {
  if (entries.length === 0) return [];
  const count = Math.min(entries.length, layout.maxDateTicks);
  const indices = new Set<number>();
  for (let tick = 0; tick < count; tick += 1) {
    const ratio = count === 1 ? 0 : tick / (count - 1);
    indices.add(Math.round(ratio * (entries.length - 1)));
  }

  return [...indices].flatMap((position) => {
    const entry = entries[position];
    return entry === undefined
      ? []
      : [
          {
            sourceIndex: entry.sourceIndex,
            sessionDate: entry.bar.session_date,
            x: xForPosition(position),
          },
        ];
  });
}

function lineSegments(
  entries: readonly PriceChartEntryWithX[],
  field: "sma_20" | "sma_60",
  priceY: (value: number) => number | undefined,
): readonly PriceChartLineSegment[] {
  const segments: PriceChartLineSegment[] = [];
  let points: PriceChartLinePoint[] = [];
  let previousSourceIndex: number | undefined;

  const finish = () => {
    if (points.length > 0) segments.push({ points });
    points = [];
  };

  for (const entry of entries) {
    if (previousSourceIndex !== undefined && entry.sourceIndex !== previousSourceIndex + 1) {
      finish();
    }
    previousSourceIndex = entry.sourceIndex;

    const value = entry.bar[field];
    const y = value === null ? undefined : priceY(value);
    if (y === undefined) {
      finish();
      continue;
    }
    points.push({ sourceIndex: entry.sourceIndex, x: entry.x, y });
  }
  finish();
  return segments;
}

export function buildPriceChartGeometry(
  bars: readonly PriceChartBar[],
  options?: PriceChartGeometryOptions,
): PriceChartGeometry {
  const layout = resolvedLayout(options);
  if (layout === undefined) return emptyGeometry();

  const inputBars = Array.isArray(bars) ? bars : [];
  const entries = inputBars.flatMap((bar, sourceIndex) =>
    isFinitePriceChartBar(bar) ? [{ sourceIndex, bar }] : [],
  );
  if (entries.length === 0) return emptyGeometry(layout);

  const prices = entries.flatMap(({ bar }) => [bar.open, bar.high, bar.low, bar.close]);
  const minimum = Math.min(...prices);
  const maximum = Math.max(...prices);
  const domain = dataDomain(minimum, maximum);
  if (domain === undefined) return emptyGeometry(layout);

  const volumeMax = Math.max(...entries.map(({ bar }) => bar.volume), 0);
  const volumeDomain: Domain = { min: 0, max: volumeMax === 0 ? 1 : volumeMax };
  const priceY = (value: number) => scaleValue(value, domain, layout.priceBottom, layout.priceTop);
  const volumeY = (value: number) =>
    scaleValue(value, volumeDomain, layout.volumeBottom, layout.volumeTop);
  const step = layout.plotWidth / entries.length;
  const bodyWidth = Math.max(2, Math.min(16, step * 0.72));
  if (!finite(step) || !finite(bodyWidth)) return emptyGeometry(layout);

  const renderedEntries: Array<PriceChartEntry & { readonly x: number }> = [];
  const candles: PriceChartCandleGeometry[] = [];
  const volumes: PriceChartVolumeGeometry[] = [];

  entries.forEach((entry, position) => {
    const x =
      entries.length === 1
        ? layout.plotLeft + layout.plotWidth / 2
        : layout.plotLeft + (position + 0.5) * step;
    const highY = priceY(entry.bar.high);
    const lowY = priceY(entry.bar.low);
    const openY = priceY(entry.bar.open);
    const closeY = priceY(entry.bar.close);
    const volumeBaseY = volumeY(0);
    const volumeValueY = volumeY(entry.bar.volume);
    if (
      !finite(x) ||
      highY === undefined ||
      lowY === undefined ||
      openY === undefined ||
      closeY === undefined ||
      volumeBaseY === undefined ||
      volumeValueY === undefined
    ) {
      return;
    }

    const rawBodyHeight = Math.abs(openY - closeY);
    const bodyHeight =
      rawBodyHeight === 0 ? Math.min(2, layout.priceBottom - layout.priceTop) : rawBodyHeight;
    const volumeHeight = Math.max(0, volumeBaseY - volumeValueY);
    if (!finite(bodyHeight) || !finite(volumeHeight)) return;

    const renderedEntry = { ...entry, x };
    renderedEntries.push(renderedEntry);
    candles.push({
      sourceIndex: entry.sourceIndex,
      x,
      bodyX: x - bodyWidth / 2,
      bodyWidth,
      bodyY: Math.min(openY, closeY) - (rawBodyHeight === 0 ? 1 : 0),
      bodyHeight,
      highY,
      lowY,
      openY,
      closeY,
      direction: directionForPriceChartBar(entry.bar),
    });
    volumes.push({
      sourceIndex: entry.sourceIndex,
      x,
      y: volumeBaseY - volumeHeight,
      width: bodyWidth,
      height: volumeHeight,
      direction: directionForPriceChartBar(entry.bar),
    });
  });

  if (renderedEntries.length === 0) return emptyGeometry(layout);

  const priceTickList = priceTicks({ min: minimum, max: maximum }, layout, priceY);
  const renderedEntriesWithX: readonly PriceChartEntryWithX[] = renderedEntries;
  const xForPosition = (position: number) => renderedEntriesWithX[position]?.x ?? layout.plotLeft;
  const sma20 = lineSegments(renderedEntriesWithX, "sma_20", priceY);
  const sma60 = lineSegments(renderedEntriesWithX, "sma_60", priceY);

  return {
    width: layout.width,
    height: layout.height,
    plotLeft: layout.plotLeft,
    plotRight: layout.plotRight,
    plotWidth: layout.plotWidth,
    priceTop: layout.priceTop,
    priceBottom: layout.priceBottom,
    volumeTop: layout.volumeTop,
    volumeBottom: layout.volumeBottom,
    priceDomain: domain,
    priceDataMin: minimum,
    priceDataMax: maximum,
    volumeDomain,
    volumeMax,
    candles,
    volumes,
    sma20,
    sma60,
    priceTicks: priceTickList,
    dateTicks: dateTicks(renderedEntriesWithX, layout, xForPosition),
    validSourceIndices: candles.map((candle) => candle.sourceIndex),
  };
}

export function findNearestPriceChartIndex(
  x: number,
  candles: readonly PriceChartCandleGeometry[],
): number | undefined {
  if (!finite(x) || candles.length === 0) return undefined;
  let nearest = candles[0];
  if (nearest === undefined) return undefined;
  let distance = Math.abs(x - nearest.x);
  for (const candle of candles.slice(1)) {
    const nextDistance = Math.abs(x - candle.x);
    if (nextDistance < distance) {
      nearest = candle;
      distance = nextDistance;
    }
  }
  return nearest.sourceIndex;
}

export function clientXToPriceChartX(
  clientX: number,
  left: number,
  width: number,
  viewBoxWidth: number,
): number | undefined {
  if (
    !finite(clientX) ||
    !finite(left) ||
    !positiveFinite(width) ||
    !positiveFinite(viewBoxWidth)
  ) {
    return undefined;
  }
  const x = ((clientX - left) / width) * viewBoxWidth;
  return finite(x) ? x : undefined;
}

export function findNearestPriceChartIndexFromClientX(
  clientX: number,
  left: number,
  width: number,
  geometry: PriceChartGeometry,
): number | undefined {
  const x = clientXToPriceChartX(clientX, left, width, geometry.width);
  return x === undefined ? undefined : findNearestPriceChartIndex(x, geometry.candles);
}

export type PriceChartNavigationKey = "ArrowLeft" | "ArrowRight" | "Home" | "End";

export function movePriceChartSelection(
  currentIndex: number | undefined,
  key: PriceChartNavigationKey,
  sourceIndices: readonly number[],
): number | undefined {
  if (sourceIndices.length === 0) return undefined;
  const currentPosition =
    currentIndex === undefined ? sourceIndices.length - 1 : sourceIndices.indexOf(currentIndex);
  const position = currentPosition < 0 ? sourceIndices.length - 1 : currentPosition;

  switch (key) {
    case "ArrowLeft":
      return sourceIndices[Math.max(0, position - 1)];
    case "ArrowRight":
      return sourceIndices[Math.min(sourceIndices.length - 1, position + 1)];
    case "Home":
      return sourceIndices[0];
    case "End":
      return sourceIndices[sourceIndices.length - 1];
  }
}
