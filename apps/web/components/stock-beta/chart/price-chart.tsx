// biome-ignore-all lint/a11y/noNoninteractiveTabindex: The named chart region is focusable for arrow-key inspection.
"use client";

import type {
  BusinessDay,
  CandlestickData,
  HistogramData,
  IChartApi,
  ISeriesApi,
  LineData,
  Time,
  WhitespaceData,
} from "lightweight-charts";
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import {
  directionForPriceChartBar,
  movePriceChartSelection,
  type PriceChartNavigationKey,
} from "./geometry";
import { buildLightweightChartData, type LightweightChartDataResult } from "./lightweight-data";
import styles from "./price-chart.module.css";
import type {
  PriceChartBar,
  PriceChartBarLabels,
  PriceChartCopy,
  PriceChartDirection,
  PriceChartFormatters,
  PriceChartProps,
} from "./types";

export const DEFAULT_PRICE_CHART_COPY: Required<PriceChartCopy> = {
  chartLabel: "Price chart",
  instructions:
    "Focus the chart, then use ArrowLeft or ArrowRight to move one observation, or Home and End to jump.",
  noDataLabel: "No valid price observations are available.",
  selectedObservationLabel: "Selected observation",
  dateLabel: "Date",
  openLabel: "Open",
  highLabel: "High",
  lowLabel: "Low",
  closeLabel: "Close",
  volumeLabel: "Volume",
  priceAxisLabel: "Price",
  volumeAxisLabel: "Volume",
  sma20Label: "SMA20",
  sma60Label: "SMA60",
  upLabel: "Up",
  downLabel: "Down",
  unchangedLabel: "Unchanged",
  resetChartLabel: "Reset zoom",
  rendererLoadingLabel: "Loading interactive chart…",
  rendererFailureLabel: "Interactive chart unavailable. The price values are hidden.",
  attributionLabel: "Chart attribution",
};

type InteractionSource = "keyboard" | "pointer";
type NumericField = "open" | "high" | "low" | "close" | "volume" | "sma_20" | "sma_60";
type ResolvedFormatters = {
  readonly date: (sessionDate: string) => string;
  readonly price: (value: number) => string;
  readonly volume: (value: number) => string;
};
type RendererStatus = "loading" | "ready" | "error";
type CandleItem = CandlestickData<Time> | WhitespaceData<Time>;
type VolumeItem = HistogramData<Time> | WhitespaceData<Time>;
type CandleSeries = ISeriesApi<"Candlestick", Time>;
type VolumeSeries = ISeriesApi<"Histogram", Time>;
type LineSeries = ISeriesApi<"Line", Time>;
type SeriesByDirection<T> = Record<PriceChartDirection, T[]>;
type RuntimeSeriesByDirection<T> = Record<PriceChartDirection, T>;
type LightweightChartsModule = typeof import("lightweight-charts");
type CrosshairMoveEvent = Parameters<Parameters<IChartApi["subscribeCrosshairMove"]>[0]>[0];

type ChartTheme = {
  readonly panel: string;
  readonly text: string;
  readonly secondary: string;
  readonly muted: string;
  readonly border: string;
  readonly grid: string;
  readonly up: string;
  readonly down: string;
  readonly flat: string;
  readonly accent: string;
};

type ChartDatasetKeys = {
  readonly dataset: string;
  readonly visual: string;
};

type PriceChartKeyboardSelection = {
  readonly requestedIndex: number | undefined;
  readonly revealIndex: number | undefined;
};

export function resolvePriceChartKeyboardSelection(
  activeIndex: number | undefined,
  interactionIndex: number | undefined,
  key: PriceChartNavigationKey,
  sourceIndices: readonly number[],
  controlled: boolean,
): PriceChartKeyboardSelection {
  const requestedIndex = movePriceChartSelection(
    controlled ? activeIndex : interactionIndex,
    key,
    sourceIndices,
  );
  return {
    requestedIndex,
    revealIndex: controlled ? activeIndex : requestedIndex,
  };
}

type PriceChartRuntime = {
  readonly chart: IChartApi;
  readonly library: LightweightChartsModule;
  readonly candles: RuntimeSeriesByDirection<CandleSeries>;
  readonly volumes: RuntimeSeriesByDirection<VolumeSeries>;
  readonly sma20: LineSeries[];
  readonly sma60: LineSeries[];
  dispose: () => void;
  theme: ChartTheme;
  lastDatasetKey: string | undefined;
  lastVisualKey: string | undefined;
  lastWidth: number;
  lastHeight: number;
  disposed: boolean;
};

function mergeCopy(copy: Partial<PriceChartCopy> | undefined): Required<PriceChartCopy> {
  return { ...DEFAULT_PRICE_CHART_COPY, ...(copy ?? {}) };
}

function safeLocale(locale: string | undefined): string {
  if (locale === undefined || locale.trim() === "") return "en-US";
  try {
    new Intl.NumberFormat(locale);
    return locale;
  } catch {
    return "en-US";
  }
}

function defaultDateFormatter(sessionDate: string, locale: string): string {
  const date = new Date(`${sessionDate}T00:00:00Z`);
  if (!Number.isFinite(date.getTime())) return sessionDate;
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
    year: "numeric",
  }).format(date);
}

function defaultFormatters(
  locale: string,
  formatters: PriceChartFormatters | undefined,
): ResolvedFormatters {
  const priceFormatter = new Intl.NumberFormat(locale, {
    maximumFractionDigits: 2,
    minimumFractionDigits: 0,
  });
  const volumeFormatter = new Intl.NumberFormat(locale, {
    maximumFractionDigits: 0,
    minimumFractionDigits: 0,
  });
  return {
    date: formatters?.date ?? ((sessionDate) => defaultDateFormatter(sessionDate, locale)),
    price: formatters?.price ?? ((value) => priceFormatter.format(value)),
    volume: formatters?.volume ?? ((value) => volumeFormatter.format(value)),
  };
}

function providedLabel(bar: PriceChartBar, field: keyof PriceChartBarLabels): string | undefined {
  const fromLabels = bar.labels?.[field];
  if (typeof fromLabels === "string") return fromLabels;
  const fromFormatted = bar.formatted?.[field];
  return typeof fromFormatted === "string" ? fromFormatted : undefined;
}

function formatBarValue(
  bar: PriceChartBar,
  field: NumericField,
  formatters: ResolvedFormatters,
): string {
  const label = providedLabel(bar, field);
  if (label !== undefined) return label;
  const value = bar[field];
  if (value === null) return "—";
  return field === "volume" ? formatters.volume(value) : formatters.price(value);
}

function formatDateValue(bar: PriceChartBar, formatters: ResolvedFormatters): string {
  return providedLabel(bar, "session_date") ?? formatters.date(bar.session_date);
}

function directionLabel(
  bar: PriceChartBar,
  direction: PriceChartDirection,
  copy: Required<PriceChartCopy>,
): string {
  return (
    providedLabel(bar, "direction") ??
    (direction === "up"
      ? copy.upLabel
      : direction === "down"
        ? copy.downLabel
        : copy.unchangedLabel)
  );
}

function resolveSelectedIndex(
  requested: number | undefined,
  available: readonly number[],
): number | undefined {
  if (available.length === 0) return undefined;
  if (requested !== undefined && Number.isSafeInteger(requested)) {
    if (available.includes(requested)) return requested;
    let nearest = available[0];
    if (nearest !== undefined) {
      let distance = Math.abs(requested - nearest);
      for (const candidate of available.slice(1)) {
        const candidateDistance = Math.abs(requested - candidate);
        if (candidateDistance < distance) {
          nearest = candidate;
          distance = candidateDistance;
        }
      }
      return nearest;
    }
  }
  return available[available.length - 1];
}

function navigationKey(key: string): PriceChartNavigationKey | undefined {
  return key === "ArrowLeft" || key === "ArrowRight" || key === "Home" || key === "End"
    ? key
    : undefined;
}

function selectedSummary(
  bar: PriceChartBar | undefined,
  direction: PriceChartDirection | undefined,
  copy: Required<PriceChartCopy>,
  formatters: ResolvedFormatters,
): string {
  if (bar === undefined || direction === undefined) return copy.noDataLabel;
  return [
    `${copy.selectedObservationLabel}: ${formatDateValue(bar, formatters)}`,
    `${copy.dateLabel}: ${bar.session_date}`,
    directionLabel(bar, direction, copy),
    `${copy.openLabel}: ${formatBarValue(bar, "open", formatters)}`,
    `${copy.highLabel}: ${formatBarValue(bar, "high", formatters)}`,
    `${copy.lowLabel}: ${formatBarValue(bar, "low", formatters)}`,
    `${copy.closeLabel}: ${formatBarValue(bar, "close", formatters)}`,
    `${copy.volumeLabel}: ${formatBarValue(bar, "volume", formatters)}`,
  ].join(" · ");
}

function detailsForBar(
  bar: PriceChartBar,
  direction: PriceChartDirection,
  copy: Required<PriceChartCopy>,
  formatters: ResolvedFormatters,
): readonly { readonly label: string; readonly value: string }[] {
  return [
    {
      label: copy.dateLabel,
      value: `${formatDateValue(bar, formatters)} · ${directionLabel(bar, direction, copy)}`,
    },
    { label: copy.openLabel, value: formatBarValue(bar, "open", formatters) },
    { label: copy.highLabel, value: formatBarValue(bar, "high", formatters) },
    { label: copy.lowLabel, value: formatBarValue(bar, "low", formatters) },
    { label: copy.closeLabel, value: formatBarValue(bar, "close", formatters) },
    { label: copy.volumeLabel, value: formatBarValue(bar, "volume", formatters) },
  ];
}

function datasetKeys(data: LightweightChartDataResult): ChartDatasetKeys {
  const visual = JSON.stringify([
    data.candles,
    data.volumes,
    data.sma20Segments,
    data.sma60Segments,
  ]);
  const statusDetails =
    data.status === "ready"
      ? data.validSourceIndices
      : data.status === "invalid"
        ? [data.reason, data.sourceIndex, data.sessionDate, data.previousSessionDate]
        : [data.reason];
  return { dataset: JSON.stringify([data.status, visual, statusDetails]), visual };
}

function emptyBuckets<T>(): SeriesByDirection<T> {
  return { up: [], down: [], flat: [] };
}

function splitChartData(
  data: LightweightChartDataResult,
  bars: readonly PriceChartBar[],
): {
  readonly candles: SeriesByDirection<CandleItem>;
  readonly volumes: SeriesByDirection<VolumeItem>;
} {
  const candles = emptyBuckets<CandleItem>();
  const volumes = emptyBuckets<VolumeItem>();
  if (data.status !== "ready") return { candles, volumes };

  data.candles.forEach((candle, index) => {
    const sourceIndex = data.validSourceIndices[index];
    const bar = sourceIndex === undefined ? undefined : bars[sourceIndex];
    if (bar === undefined) return;
    const direction = directionForPriceChartBar(bar);
    const volume = data.volumes[index];
    for (const candidate of ["up", "down", "flat"] as const) {
      if (direction === candidate) {
        candles[candidate].push(candle);
        if (volume !== undefined) volumes[candidate].push(volume);
      } else {
        candles[candidate].push({ time: candle.time });
        if (volume !== undefined) volumes[candidate].push({ time: volume.time });
      }
    }
  });
  return { candles, volumes };
}

function isoDateForTime(time: Time): string | undefined {
  if (typeof time === "string") {
    return /^\d{4}-\d{2}-\d{2}$/.test(time) ? time : undefined;
  }
  if (typeof time !== "object") return undefined;
  const businessDay = time as BusinessDay;
  if (
    !Number.isInteger(businessDay.year) ||
    !Number.isInteger(businessDay.month) ||
    !Number.isInteger(businessDay.day)
  ) {
    return undefined;
  }
  return `${String(businessDay.year).padStart(4, "0")}-${String(businessDay.month).padStart(2, "0")}-${String(businessDay.day).padStart(2, "0")}`;
}

function originalIndexForTime(data: LightweightChartDataResult, time: Time): number | undefined {
  const direct = data.dateToOriginalIndex.get(time);
  if (direct !== undefined) return direct;
  const sessionDate = isoDateForTime(time);
  return sessionDate === undefined ? undefined : data.dateToOriginalIndex.get(sessionDate);
}

function computedColor(root: HTMLElement, name: string, fallback: string): string {
  const value = getComputedStyle(root).getPropertyValue(name).trim();
  return value || fallback;
}

function resolveTheme(root: HTMLElement, forcedColors: boolean): ChartTheme {
  if (forcedColors) {
    return {
      panel: "Canvas",
      text: "CanvasText",
      secondary: "CanvasText",
      muted: "GrayText",
      border: "CanvasText",
      grid: "GrayText",
      up: "Highlight",
      down: "LinkText",
      flat: "GrayText",
      accent: "Highlight",
    };
  }
  return {
    panel: computedColor(root, "--chart-panel", "#11161b"),
    text: computedColor(root, "--chart-text", "#dce3e9"),
    secondary: computedColor(root, "--chart-secondary", "#9aa6b2"),
    muted: computedColor(root, "--chart-muted", "#7c8792"),
    border: computedColor(root, "--chart-border", "#252c33"),
    grid: computedColor(root, "--chart-border", "#252c33"),
    up: computedColor(root, "--chart-up", "#20b486"),
    down: computedColor(root, "--chart-down", "#ec5f67"),
    flat: computedColor(root, "--chart-flat", "#7c8792"),
    accent: computedColor(root, "--chart-accent", "#35a8ff"),
  };
}

function applySeriesTheme(runtime: PriceChartRuntime, theme: ChartTheme): void {
  runtime.candles.up.applyOptions({
    upColor: theme.up,
    downColor: theme.up,
    borderUpColor: theme.up,
    borderDownColor: theme.up,
    wickUpColor: theme.up,
    wickDownColor: theme.up,
  });
  runtime.candles.down.applyOptions({
    upColor: theme.panel,
    downColor: theme.panel,
    borderUpColor: theme.down,
    borderDownColor: theme.down,
    wickUpColor: theme.down,
    wickDownColor: theme.down,
  });
  runtime.candles.flat.applyOptions({
    upColor: theme.flat,
    downColor: theme.flat,
    borderUpColor: theme.flat,
    borderDownColor: theme.flat,
    wickUpColor: theme.flat,
    wickDownColor: theme.flat,
  });

  runtime.volumes.up.applyOptions({ color: theme.up });
  runtime.volumes.down.applyOptions({ color: theme.down });
  runtime.volumes.flat.applyOptions({ color: theme.flat });
  for (const series of runtime.sma20) series.applyOptions({ color: theme.accent });
  for (const series of runtime.sma60) series.applyOptions({ color: theme.secondary });
}

function applyRuntimeTheme(
  runtime: PriceChartRuntime,
  theme: ChartTheme,
  locale: string,
  formatters: ResolvedFormatters,
): void {
  runtime.theme = theme;
  runtime.chart.applyOptions({
    layout: {
      background: { type: runtime.library.ColorType.Solid, color: theme.panel },
      textColor: theme.text,
      panes: { separatorColor: theme.border, separatorHoverColor: theme.border },
    },
    grid: {
      vertLines: { color: theme.grid },
      horzLines: { color: theme.grid },
    },
    rightPriceScale: { borderColor: theme.border, textColor: theme.secondary },
    timeScale: {
      borderColor: theme.border,
      tickMarkFormatter: (time: Time) => {
        const sessionDate = isoDateForTime(time);
        return sessionDate === undefined ? "" : formatters.date(sessionDate);
      },
    },
    crosshair: {
      mode: runtime.library.CrosshairMode.Magnet,
      vertLine: {
        color: theme.accent,
        style: runtime.library.LineStyle.Dashed,
        width: 1,
        labelBackgroundColor: theme.accent,
      },
      horzLine: {
        color: theme.accent,
        style: runtime.library.LineStyle.Dashed,
        width: 1,
        labelBackgroundColor: theme.accent,
      },
    },
    localization: {
      locale,
      priceFormatter: formatters.price,
      timeFormatter: (time: Time) => {
        const sessionDate = isoDateForTime(time);
        return sessionDate === undefined ? "" : formatters.date(sessionDate);
      },
    },
  });

  const candleStyle = {
    borderVisible: true,
    wickVisible: true,
    priceLineVisible: false,
    lastValueVisible: false,
  } as const;
  runtime.candles.up.applyOptions(candleStyle);
  runtime.candles.down.applyOptions(candleStyle);
  runtime.candles.flat.applyOptions(candleStyle);
  runtime.volumes.up.applyOptions({
    priceLineVisible: false,
    lastValueVisible: false,
    priceFormat: { type: "custom", formatter: formatters.volume, minMove: 1 },
  });
  runtime.volumes.down.applyOptions({
    priceLineVisible: false,
    lastValueVisible: false,
    priceFormat: { type: "custom", formatter: formatters.volume, minMove: 1 },
  });
  runtime.volumes.flat.applyOptions({
    priceLineVisible: false,
    lastValueVisible: false,
    priceFormat: { type: "custom", formatter: formatters.volume, minMove: 1 },
  });
  for (const series of runtime.sma20) {
    series.applyOptions({
      color: theme.accent,
      lineWidth: 2,
      lineStyle: runtime.library.LineStyle.Solid,
      priceLineVisible: false,
      lastValueVisible: false,
    });
  }
  for (const series of runtime.sma60) {
    series.applyOptions({
      color: theme.secondary,
      lineWidth: 2,
      lineStyle: runtime.library.LineStyle.LargeDashed,
      priceLineVisible: false,
      lastValueVisible: false,
    });
  }
  applySeriesTheme(runtime, theme);
}

function addSmaSeries(
  runtime: PriceChartRuntime,
  target: LineSeries[],
  segments: readonly (readonly LineData<Time>[])[],
  color: string,
  dashed: boolean,
): void {
  while (target.length > segments.length) {
    const series = target.pop();
    if (series !== undefined) runtime.chart.removeSeries(series);
  }
  while (target.length < segments.length) {
    target.push(
      runtime.chart.addSeries(
        runtime.library.LineSeries,
        {
          color,
          lineWidth: 2,
          lineStyle: dashed
            ? runtime.library.LineStyle.LargeDashed
            : runtime.library.LineStyle.Solid,
          priceLineVisible: false,
          lastValueVisible: false,
          crosshairMarkerVisible: false,
        },
        0,
      ),
    );
  }
  segments.forEach((segment, index) => {
    const series = target[index];
    if (series !== undefined) series.setData([...segment]);
  });
}

function fitLoadedData(runtime: PriceChartRuntime, data: LightweightChartDataResult): void {
  const timeScale = runtime.chart.timeScale();
  timeScale.fitContent();
  if (data.status !== "ready") return;
  // Keep the first candle's rounded body and wick inside the Canvas at compact widths.
  timeScale.setVisibleLogicalRange({
    from: -1,
    to: data.validSourceIndices.length - 1 + runtime.chart.options().timeScale.rightOffset,
  });
}

function applyChartData(
  runtime: PriceChartRuntime,
  data: LightweightChartDataResult,
  bars: readonly PriceChartBar[],
  keys: ChartDatasetKeys,
): void {
  if (runtime.lastDatasetKey === keys.dataset) return;
  if (runtime.lastVisualKey !== keys.visual) {
    const split = splitChartData(data, bars);
    runtime.candles.up.setData(split.candles.up);
    runtime.candles.down.setData(split.candles.down);
    runtime.candles.flat.setData(split.candles.flat);
    runtime.volumes.up.setData(split.volumes.up);
    runtime.volumes.down.setData(split.volumes.down);
    runtime.volumes.flat.setData(split.volumes.flat);
    addSmaSeries(runtime, runtime.sma20, data.sma20Segments, runtime.theme.accent, false);
    addSmaSeries(runtime, runtime.sma60, data.sma60Segments, runtime.theme.secondary, true);
    runtime.lastVisualKey = keys.visual;
  }
  runtime.lastDatasetKey = keys.dataset;
  runtime.chart.clearCrosshairPosition();
  fitLoadedData(runtime, data);
}

function ensureSelectedPointVisible(
  runtime: PriceChartRuntime,
  data: LightweightChartDataResult,
  sourceIndex: number,
): void {
  if (data.status !== "ready") return;
  const logicalIndex = data.validSourceIndices.indexOf(sourceIndex);
  if (logicalIndex < 0) return;
  const visible = runtime.chart.timeScale().getVisibleLogicalRange();
  if (visible === null || (logicalIndex >= visible.from && logicalIndex <= visible.to)) return;

  const width = Math.max(1, visible.to - visible.from);
  if (logicalIndex < visible.from) {
    const from = Math.max(0, logicalIndex - Math.min(2, width / 4));
    runtime.chart.timeScale().setVisibleLogicalRange({ from, to: from + width });
    return;
  }
  const to = logicalIndex + Math.min(2, width / 4);
  runtime.chart.timeScale().setVisibleLogicalRange({ from: Math.max(0, to - width), to });
}

function revealSelectedPoint(
  runtime: PriceChartRuntime,
  data: LightweightChartDataResult,
  bars: readonly PriceChartBar[],
  sourceIndex: number,
): void {
  ensureSelectedPointVisible(runtime, data, sourceIndex);
  const bar = bars[sourceIndex];
  if (bar === undefined) return;
  const direction = directionForPriceChartBar(bar);
  runtime.chart.setCrosshairPosition(bar.close, bar.session_date, runtime.candles[direction]);
}

function restoreSelectedCrosshair(
  runtime: PriceChartRuntime,
  data: LightweightChartDataResult,
  bars: readonly PriceChartBar[],
  sourceIndex: number | undefined,
): void {
  const logicalIndex =
    sourceIndex === undefined ? -1 : data.validSourceIndices.indexOf(sourceIndex);
  const visible = runtime.chart.timeScale().getVisibleLogicalRange();
  const bar = sourceIndex === undefined ? undefined : bars[sourceIndex];
  if (
    bar === undefined ||
    logicalIndex < 0 ||
    visible === null ||
    logicalIndex < visible.from ||
    logicalIndex > visible.to
  ) {
    // The library clamps synthetic positions to visible bars, so hide an offscreen selection.
    runtime.chart.clearCrosshairPosition();
    return;
  }
  const direction = directionForPriceChartBar(bar);
  runtime.chart.setCrosshairPosition(bar.close, bar.session_date, runtime.candles[direction]);
}

function chartDimensions(element: HTMLElement): {
  readonly width: number;
  readonly height: number;
} {
  const rect = element.getBoundingClientRect();
  return {
    width: Math.max(1, Math.round(rect.width || element.clientWidth || 640)),
    height: Math.max(1, Math.round(rect.height || element.clientHeight || 320)),
  };
}

function joinClasses(...classNames: readonly (string | undefined)[]): string {
  return classNames.filter((className): className is string => className !== undefined).join(" ");
}

export function PriceChart({
  bars,
  className,
  copy: copyInput,
  format,
  formatters: formattersInput,
  initialSelectedIndex,
  locale,
  onObservationChange,
  onSelectedIndexChange,
  selectedIndex,
}: PriceChartProps) {
  const copy = useMemo(() => mergeCopy(copyInput), [copyInput]);
  const localeValue = safeLocale(locale);
  const formatters = useMemo(
    () => defaultFormatters(localeValue, formattersInput ?? format),
    [format, formattersInput, localeValue],
  );
  const data = useMemo(() => buildLightweightChartData(bars), [bars]);
  const keys = useMemo(() => datasetKeys(data), [data]);
  const available = data.status === "ready" ? data.validSourceIndices : [];
  const [uncontrolledSelection, setUncontrolledSelection] = useState(() => ({
    dataset: keys.dataset,
    index: resolveSelectedIndex(initialSelectedIndex, available),
  }));
  const activeSelectedIndex = resolveSelectedIndex(
    selectedIndex ??
      (uncontrolledSelection.dataset === keys.dataset ? uncontrolledSelection.index : undefined),
    available,
  );
  const selectedBar = activeSelectedIndex === undefined ? undefined : bars[activeSelectedIndex];
  const selectedDirection =
    selectedBar === undefined ? undefined : directionForPriceChartBar(selectedBar);
  const summary = selectedSummary(selectedBar, selectedDirection, copy, formatters);
  const instanceId = useId();
  const instructionsId = `${instanceId}-instructions`;
  const summaryId = `${instanceId}-summary`;
  const detailId = `${instanceId}-detail`;
  const rootRef = useRef<HTMLDivElement | null>(null);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const runtimeRef = useRef<PriceChartRuntime | null>(null);
  const dataRef = useRef(data);
  const barsRef = useRef(bars);
  const formattersRef = useRef(formatters);
  const localeRef = useRef(localeValue);
  const handlersRef = useRef<{
    readonly select: (index: number, source: InteractionSource) => void;
    readonly controlled: boolean;
    readonly activeIndex: number | undefined;
  }>({ select: () => undefined, controlled: false, activeIndex: undefined });
  const controlledSelectionRef = useRef(selectedIndex);
  const datasetRef = useRef(keys.dataset);
  const interactionIndexRef = useRef(activeSelectedIndex);
  const controlledKeyboardRequestRef = useRef<number | undefined>(undefined);
  const [rendererStatus, setRendererStatus] = useState<RendererStatus>("loading");

  dataRef.current = data;
  barsRef.current = bars;
  formattersRef.current = formatters;
  localeRef.current = localeValue;

  if (controlledSelectionRef.current !== selectedIndex || datasetRef.current !== keys.dataset) {
    controlledSelectionRef.current = selectedIndex;
    datasetRef.current = keys.dataset;
    interactionIndexRef.current = activeSelectedIndex;
    controlledKeyboardRequestRef.current = undefined;
  }

  useEffect(() => {
    if (selectedIndex === undefined && uncontrolledSelection.dataset !== keys.dataset) {
      setUncontrolledSelection({
        dataset: keys.dataset,
        index: resolveSelectedIndex(undefined, available),
      });
    }
  }, [available, keys.dataset, selectedIndex, uncontrolledSelection.dataset]);

  const selectIndex = (nextIndex: number | undefined, source: InteractionSource): void => {
    if (nextIndex === undefined) return;
    const resolvedIndex = resolveSelectedIndex(nextIndex, available);
    if (resolvedIndex === undefined) return;
    const controlled = selectedIndex !== undefined;
    if (source === "keyboard" && controlled) {
      if (
        resolvedIndex === activeSelectedIndex ||
        resolvedIndex === controlledKeyboardRequestRef.current
      ) {
        return;
      }
    } else if (
      resolvedIndex === interactionIndexRef.current ||
      (source === "pointer" && controlled && resolvedIndex === activeSelectedIndex)
    ) {
      interactionIndexRef.current = resolvedIndex;
      return;
    }
    const nextBar = bars[resolvedIndex];
    if (nextBar === undefined) return;

    if (source === "keyboard" && controlled) {
      controlledKeyboardRequestRef.current = resolvedIndex;
    } else {
      interactionIndexRef.current = resolvedIndex;
    }
    if (!controlled) {
      setUncontrolledSelection({ dataset: keys.dataset, index: resolvedIndex });
    }
    onObservationChange?.(resolvedIndex, nextBar);
    onSelectedIndexChange?.(resolvedIndex, nextBar);
  };
  handlersRef.current = {
    select: selectIndex,
    controlled: selectedIndex !== undefined,
    activeIndex: activeSelectedIndex,
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    const key = navigationKey(event.key);
    if (key === undefined) return;
    event.preventDefault();
    const selection = resolvePriceChartKeyboardSelection(
      activeSelectedIndex,
      interactionIndexRef.current,
      key,
      available,
      selectedIndex !== undefined,
    );
    selectIndex(selection.requestedIndex, "keyboard");
    const runtime = runtimeRef.current;
    if (runtime !== null && selection.revealIndex !== undefined) {
      revealSelectedPoint(runtime, dataRef.current, barsRef.current, selection.revealIndex);
    }
  };

  useEffect(() => {
    let cancelled = false;
    let chart: IChartApi | undefined;
    let runtime: PriceChartRuntime | null = null;
    let resizeObserver: ResizeObserver | undefined;
    let themeObserver: MutationObserver | undefined;
    let forcedColorsQuery: MediaQueryList | undefined;
    let pendingResizeFrame: number | undefined;
    const host = hostRef.current;
    const root = rootRef.current;
    if (host === null || root === null) return;

    const destroyRuntime = (): void => {
      if (runtime !== null) {
        runtime.dispose();
        return;
      }
      if (chart !== undefined) {
        chart.remove();
        chart = undefined;
        host.replaceChildren();
      }
    };

    const failRuntime = (): void => {
      destroyRuntime();
      if (!cancelled) setRendererStatus("error");
    };

    const refreshTheme = (): void => {
      if (runtime === null || runtime.disposed) return;
      try {
        const theme = resolveTheme(root, forcedColorsQuery?.matches ?? false);
        applyRuntimeTheme(runtime, theme, localeRef.current, formattersRef.current);
      } catch {
        failRuntime();
      }
    };

    const initialize = async (): Promise<void> => {
      try {
        const library = await import("lightweight-charts");
        if (cancelled) return;

        forcedColorsQuery = window.matchMedia?.("(forced-colors: active)");
        const theme = resolveTheme(root, forcedColorsQuery?.matches ?? false);
        const dimensions = chartDimensions(host);
        chart = library.createChart(host, {
          width: dimensions.width,
          height: dimensions.height,
          layout: {
            background: { type: library.ColorType.Solid, color: theme.panel },
            textColor: theme.text,
            attributionLogo: false,
            fontFamily: getComputedStyle(root).fontFamily || "sans-serif",
            panes: { separatorColor: theme.border, separatorHoverColor: theme.border },
          },
          grid: {
            vertLines: { color: theme.grid },
            horzLines: { color: theme.grid },
          },
          rightPriceScale: { borderColor: theme.border, textColor: theme.secondary },
          timeScale: { borderColor: theme.border, rightOffset: 3, timeVisible: false },
          crosshair: { mode: library.CrosshairMode.Magnet },
          handleScroll: {
            mouseWheel: false,
            pressedMouseMove: true,
            horzTouchDrag: true,
            vertTouchDrag: false,
          },
          handleScale: { mouseWheel: true, pinch: true, axisPressedMouseMove: true },
          kineticScroll: { touch: true, mouse: true },
          localization: {
            locale: localeRef.current,
            priceFormatter: (value: number) => formattersRef.current.price(value),
          },
        });
        const currentChart = chart;
        currentChart.addPane();
        currentChart.panes()[0]?.setStretchFactor(3);
        currentChart.panes()[1]?.setStretchFactor(1);

        const handleCrosshairMove = (event: CrosshairMoveEvent): void => {
          const handlers = handlersRef.current;
          // Scale redraws can emit crosshair moves without a user event.
          if (event.sourceEvent !== undefined && event.time !== undefined) {
            const sourceIndex = originalIndexForTime(dataRef.current, event.time);
            if (sourceIndex !== undefined) handlers.select(sourceIndex, "pointer");
          }
          if (
            (handlers.controlled ||
              (event.sourceEvent === undefined && event.point !== undefined)) &&
            runtime !== null &&
            !runtime.disposed
          ) {
            restoreSelectedCrosshair(
              runtime,
              dataRef.current,
              barsRef.current,
              handlers.activeIndex,
            );
          }
        };

        const candleOptions = {
          borderVisible: true,
          wickVisible: true,
          priceLineVisible: false,
          lastValueVisible: false,
        } as const;
        const upCandle = chart.addSeries(
          library.CandlestickSeries,
          {
            ...candleOptions,
            upColor: theme.up,
            downColor: theme.up,
            borderUpColor: theme.up,
            borderDownColor: theme.up,
            wickUpColor: theme.up,
            wickDownColor: theme.up,
          },
          0,
        );
        const downCandle = chart.addSeries(
          library.CandlestickSeries,
          {
            ...candleOptions,
            upColor: theme.panel,
            downColor: theme.panel,
            borderUpColor: theme.down,
            borderDownColor: theme.down,
            wickUpColor: theme.down,
            wickDownColor: theme.down,
          },
          0,
        );
        const flatCandle = chart.addSeries(
          library.CandlestickSeries,
          {
            ...candleOptions,
            upColor: theme.flat,
            downColor: theme.flat,
            borderUpColor: theme.flat,
            borderDownColor: theme.flat,
            wickUpColor: theme.flat,
            wickDownColor: theme.flat,
          },
          0,
        );
        const volumeSeries = (color: string): VolumeSeries =>
          currentChart.addSeries(
            library.HistogramSeries,
            {
              color,
              priceLineVisible: false,
              lastValueVisible: false,
              priceFormat: {
                type: "custom",
                formatter: (value: number) => formattersRef.current.volume(value),
                minMove: 1,
              },
            },
            1,
          );
        const upVolume = volumeSeries(theme.up);
        const downVolume = volumeSeries(theme.down);
        const flatVolume = volumeSeries(theme.flat);
        upVolume.priceScale().applyOptions({ scaleMargins: { top: 0.1, bottom: 0 } });
        downVolume.priceScale().applyOptions({ scaleMargins: { top: 0.1, bottom: 0 } });
        flatVolume.priceScale().applyOptions({ scaleMargins: { top: 0.1, bottom: 0 } });

        const currentDimensions = dimensions;
        const nextRuntime: PriceChartRuntime = {
          chart: currentChart,
          library,
          candles: { up: upCandle, down: downCandle, flat: flatCandle },
          volumes: { up: upVolume, down: downVolume, flat: flatVolume },
          sma20: [],
          sma60: [],
          theme,
          lastDatasetKey: undefined,
          lastVisualKey: undefined,
          lastWidth: currentDimensions.width,
          lastHeight: currentDimensions.height,
          disposed: false,
          dispose: () => undefined,
        };
        runtime = nextRuntime;
        runtimeRef.current = nextRuntime;
        runtime.dispose = () => {
          if (runtime === null || runtime.disposed) return;
          runtime.disposed = true;
          resizeObserver?.disconnect();
          themeObserver?.disconnect();
          forcedColorsQuery?.removeEventListener("change", refreshTheme);
          if (pendingResizeFrame !== undefined) {
            cancelAnimationFrame(pendingResizeFrame);
            pendingResizeFrame = undefined;
          }
          try {
            runtime.chart.unsubscribeCrosshairMove(handleCrosshairMove);
          } catch {
            // The chart removal below still owns the canvas and pending work.
          }
          try {
            runtime.chart.remove();
          } finally {
            if (runtimeRef.current === runtime) runtimeRef.current = null;
            host.replaceChildren();
          }
        };

        currentChart.subscribeCrosshairMove(handleCrosshairMove);
        applyRuntimeTheme(nextRuntime, theme, localeRef.current, formattersRef.current);
        applyChartData(nextRuntime, dataRef.current, barsRef.current, datasetKeys(dataRef.current));
        if (cancelled) {
          destroyRuntime();
          return;
        }
        setRendererStatus("ready");

        if (typeof ResizeObserver !== "undefined") {
          resizeObserver = new ResizeObserver(() => {
            if (pendingResizeFrame !== undefined) return;
            pendingResizeFrame = requestAnimationFrame(() => {
              pendingResizeFrame = undefined;
              if (cancelled || runtime === null || runtime.disposed) return;
              const nextDimensions = chartDimensions(host);
              if (
                nextDimensions.width === runtime.lastWidth &&
                nextDimensions.height === runtime.lastHeight
              ) {
                return;
              }
              runtime.chart.resize(nextDimensions.width, nextDimensions.height);
              runtime.lastWidth = nextDimensions.width;
              runtime.lastHeight = nextDimensions.height;
            });
          });
          resizeObserver.observe(host);
        }

        if (forcedColorsQuery !== undefined) {
          forcedColorsQuery.addEventListener("change", refreshTheme);
        }
        if (typeof MutationObserver !== "undefined") {
          themeObserver = new MutationObserver(refreshTheme);
          const attributes: MutationObserverInit = {
            attributes: true,
            attributeFilter: ["class", "style", "data-theme"],
          };
          themeObserver.observe(document.documentElement, attributes);
          if (document.body !== null) themeObserver.observe(document.body, attributes);
          themeObserver.observe(root, attributes);
        }
      } catch {
        failRuntime();
      }
    };

    void initialize();

    return () => {
      cancelled = true;
      resizeObserver?.disconnect();
      themeObserver?.disconnect();
      forcedColorsQuery?.removeEventListener("change", refreshTheme);
      if (pendingResizeFrame !== undefined) cancelAnimationFrame(pendingResizeFrame);
      destroyRuntime();
    };
  }, []);

  useEffect(() => {
    const runtime = runtimeRef.current;
    const root = rootRef.current;
    if (runtime === null || root === null || runtime.disposed) return;
    try {
      const forcedColors = window.matchMedia?.("(forced-colors: active)").matches ?? false;
      applyRuntimeTheme(runtime, resolveTheme(root, forcedColors), localeValue, formatters);
    } catch {
      runtime.dispose();
      setRendererStatus("error");
    }
  }, [formatters, localeValue]);

  useEffect(() => {
    const runtime = runtimeRef.current;
    if (runtime === null || runtime.disposed) return;
    try {
      applyChartData(runtime, data, bars, keys);
    } catch {
      runtime.dispose();
      setRendererStatus("error");
    }
  }, [bars, data, keys]);

  useEffect(() => {
    const runtime = runtimeRef.current;
    if (
      runtime === null ||
      runtime.disposed ||
      rendererStatus !== "ready" ||
      runtime.lastDatasetKey !== keys.dataset ||
      activeSelectedIndex === undefined
    ) {
      return;
    }
    try {
      revealSelectedPoint(runtime, dataRef.current, barsRef.current, activeSelectedIndex);
    } catch {
      runtime.dispose();
      setRendererStatus("error");
    }
  }, [activeSelectedIndex, keys.dataset, rendererStatus]);

  const resetZoom = (): void => {
    const runtime = runtimeRef.current;
    if (runtime !== null) fitLoadedData(runtime, dataRef.current);
  };
  const direction = selectedDirection;
  const details =
    selectedBar === undefined || direction === undefined
      ? null
      : detailsForBar(selectedBar, direction, copy, formatters);
  const rootClassName = joinClasses(styles["chartRoot"], className);

  return (
    <div
      className={rootClassName}
      data-renderer-status={rendererStatus}
      data-selected-index={activeSelectedIndex ?? "none"}
      data-testid="stock-beta-price-chart"
      ref={rootRef}
    >
      <p className={styles["srOnly"]} id={instructionsId}>
        {copy.instructions}
      </p>
      <div aria-hidden="true" className={styles["axisLabels"]}>
        <span>{copy.priceAxisLabel}</span>
        <span>{copy.volumeAxisLabel}</span>
      </div>
      <section
        aria-describedby={`${instructionsId} ${summaryId} ${detailId}`}
        aria-keyshortcuts="ArrowLeft ArrowRight Home End"
        aria-label={copy.chartLabel}
        className={styles["chartSurface"]}
        data-selected-index={activeSelectedIndex ?? "none"}
        data-testid="stock-beta-price-chart-surface"
        onKeyDown={handleKeyDown}
        tabIndex={0}
      >
        <div
          aria-hidden="true"
          className={styles["chartCanvas"]}
          data-testid="stock-beta-price-chart-canvas"
          ref={hostRef}
        />
      </section>
      <p
        aria-live="polite"
        aria-atomic="true"
        className={styles["rendererStatus"]}
        data-testid="stock-beta-price-chart-renderer-status"
      >
        {rendererStatus === "loading"
          ? copy.rendererLoadingLabel
          : rendererStatus === "error"
            ? copy.rendererFailureLabel
            : ""}
      </p>
      <p
        aria-live="polite"
        aria-atomic="true"
        className={styles["summary"]}
        data-direction={direction ?? "none"}
        data-testid="stock-beta-price-chart-summary"
        id={summaryId}
        role="status"
      >
        {rendererStatus === "error" ? copy.rendererFailureLabel : summary}
      </p>
      {rendererStatus !== "error" && details !== null ? (
        <div
          aria-label={copy.selectedObservationLabel}
          className={styles["tooltip"]}
          data-testid="stock-beta-price-chart-tooltip"
          id={detailId}
          role="tooltip"
        >
          {details.map((detail) => (
            <span className={styles["tooltipItem"]} key={detail.label}>
              <span>{detail.label}</span>
              <strong>{detail.value}</strong>
            </span>
          ))}
        </div>
      ) : (
        <span className={styles["srOnly"]} id={detailId}>
          {rendererStatus === "error" ? copy.rendererFailureLabel : copy.noDataLabel}
        </span>
      )}
      <div className={styles["chartFooter"]}>
        <ul
          aria-label={copy.chartLabel}
          className={styles["legend"]}
          data-testid="stock-beta-price-chart-legend"
        >
          <li className={styles["legendItem"]}>
            <span aria-hidden="true" className={styles["directionMarker"]} data-direction="up">
              ▲
            </span>
            {copy.upLabel}
          </li>
          <li className={styles["legendItem"]}>
            <span aria-hidden="true" className={styles["directionMarker"]} data-direction="down">
              ▼
            </span>
            {copy.downLabel}
          </li>
          <li className={styles["legendItem"]}>
            <span aria-hidden="true" className={styles["directionMarker"]} data-direction="flat">
              —
            </span>
            {copy.unchangedLabel}
          </li>
          <li className={styles["legendItem"]}>
            <span aria-hidden="true" className={styles["smaMarker"]} data-sma="20" />
            {copy.sma20Label}
          </li>
          <li className={styles["legendItem"]}>
            <span aria-hidden="true" className={styles["smaMarker"]} data-sma="60" />
            {copy.sma60Label}
          </li>
        </ul>
        <button
          className={styles["resetButton"]}
          data-testid="stock-beta-price-chart-reset"
          disabled={rendererStatus !== "ready" || data.status !== "ready"}
          onClick={resetZoom}
          type="button"
        >
          {copy.resetChartLabel}
        </button>
      </div>
      <p className={styles["attribution"]} data-testid="stock-beta-price-chart-attribution">
        <span className={styles["attributionLabel"]}>{copy.attributionLabel}: </span>
        <span>TradingView Lightweight Charts™</span>
        <br />
        <span>Copyright (с) 2025 TradingView, Inc. </span>
        <a href="https://www.tradingview.com/">https://www.tradingview.com/</a>
        <span> · </span>
        <a href="/licenses/lightweight-charts/NOTICE.txt">NOTICE</a>
        <span> · </span>
        <a href="/licenses/lightweight-charts/LICENSE.txt">LICENSE</a>
      </p>
    </div>
  );
}

export default PriceChart;
