export const PRICE_CHART_DIRECTIONS = ["up", "down", "flat"] as const;

export type PriceChartDirection = (typeof PRICE_CHART_DIRECTIONS)[number];

export type PriceChartBarLabels = {
  readonly session_date?: string;
  readonly open?: string;
  readonly high?: string;
  readonly low?: string;
  readonly close?: string;
  readonly volume?: string;
  readonly sma_20?: string | null;
  readonly sma_60?: string | null;
  readonly direction?: string;
};

/**
 * The chart's deliberately small presentation contract. Product/API models
 * should be adapted to this shape before crossing into the renderer.
 */
export type PriceChartBar = {
  readonly session_date: string;
  readonly open: number;
  readonly high: number;
  readonly low: number;
  readonly close: number;
  readonly volume: number;
  readonly sma_20: number | null;
  readonly sma_60: number | null;
  readonly labels?: PriceChartBarLabels;
  readonly formatted?: PriceChartBarLabels;
};

export type PriceChartCopy = {
  readonly chartLabel: string;
  readonly instructions: string;
  readonly noDataLabel: string;
  readonly selectedObservationLabel: string;
  readonly dateLabel: string;
  readonly openLabel: string;
  readonly highLabel: string;
  readonly lowLabel: string;
  readonly closeLabel: string;
  readonly volumeLabel: string;
  readonly priceAxisLabel: string;
  readonly volumeAxisLabel: string;
  readonly sma20Label: string;
  readonly sma60Label: string;
  readonly upLabel: string;
  readonly downLabel: string;
  readonly unchangedLabel: string;
};

export type PriceChartFormatters = {
  readonly date?: (sessionDate: string) => string;
  readonly price?: (value: number) => string;
  readonly volume?: (value: number) => string;
};

export type PriceChartProps = {
  readonly bars: readonly PriceChartBar[];
  readonly locale?: string;
  readonly copy?: Partial<PriceChartCopy>;
  readonly formatters?: PriceChartFormatters;
  /** Alias for callers that prefer a singular format prop. */
  readonly format?: PriceChartFormatters;
  readonly initialSelectedIndex?: number;
  readonly selectedIndex?: number;
  readonly onObservationChange?: (index: number, bar: PriceChartBar) => void;
  readonly onSelectedIndexChange?: (index: number, bar: PriceChartBar) => void;
  readonly className?: string;
};
