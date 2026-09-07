"use client";

import { type KeyboardEvent, type PointerEvent, type TouchEvent, useId, useState } from "react";
import {
  buildPriceChartGeometry,
  findNearestPriceChartIndexFromClientX,
  movePriceChartSelection,
  type PriceChartGeometry,
  type PriceChartNavigationKey,
} from "./geometry";
import styles from "./price-chart.module.css";
import type {
  PriceChartBar,
  PriceChartBarLabels,
  PriceChartCopy,
  PriceChartDirection,
  PriceChartFormatters,
  PriceChartProps,
} from "./types";

export const DEFAULT_PRICE_CHART_COPY: PriceChartCopy = {
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
};

type InteractionSource = "keyboard" | "pointer" | "touch";
type NumericField = "open" | "high" | "low" | "close" | "volume" | "sma_20" | "sma_60";

type ResolvedFormatters = {
  readonly date: (sessionDate: string) => string;
  readonly price: (value: number) => string;
  readonly volume: (value: number) => string;
};

function mergeCopy(copy: Partial<PriceChartCopy> | undefined): PriceChartCopy {
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
  copy: PriceChartCopy,
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

function sourceIndices(geometry: PriceChartGeometry): readonly number[] {
  return geometry.validSourceIndices;
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

function linePoints(points: readonly { readonly x: number; readonly y: number }[]): string {
  return points.map((point) => `${point.x},${point.y}`).join(" ");
}

function joinClasses(...classNames: readonly (string | undefined)[]): string {
  return classNames.filter((className): className is string => className !== undefined).join(" ");
}

function tooltipPosition(
  geometry: PriceChartGeometry,
  selectedX: number,
): {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
} {
  const width = Math.min(188, Math.max(0, geometry.width - 8));
  const height = 116;
  const preferredX =
    selectedX > geometry.plotLeft + geometry.plotWidth / 2
      ? geometry.plotLeft + 8
      : geometry.plotRight - width - 8;
  const x = Math.max(0, Math.min(Math.max(0, geometry.width - width), preferredX));
  const preferredY = geometry.priceTop + 8;
  const y = Math.max(0, Math.min(Math.max(0, geometry.height - height), preferredY));
  return { x, y, width, height };
}

function candleLabel(
  bar: PriceChartBar,
  direction: PriceChartDirection,
  copy: PriceChartCopy,
  formatters: ResolvedFormatters,
): string {
  return [
    `${formatDateValue(bar, formatters)} · ${directionLabel(bar, direction, copy)}`,
    `${copy.openLabel}: ${formatBarValue(bar, "open", formatters)}`,
    `${copy.highLabel}: ${formatBarValue(bar, "high", formatters)}`,
    `${copy.lowLabel}: ${formatBarValue(bar, "low", formatters)}`,
    `${copy.closeLabel}: ${formatBarValue(bar, "close", formatters)}`,
    `${copy.volumeLabel}: ${formatBarValue(bar, "volume", formatters)}`,
  ].join("; ");
}

function selectedSummary(
  bar: PriceChartBar | undefined,
  direction: PriceChartDirection | undefined,
  copy: PriceChartCopy,
  formatters: ResolvedFormatters,
): string {
  if (bar === undefined || direction === undefined) return copy.noDataLabel;
  return [
    `${copy.selectedObservationLabel}: ${formatDateValue(bar, formatters)}`,
    `${copy.dateLabel}: ${bar.session_date}`,
    `${directionLabel(bar, direction, copy)}`,
    `${copy.openLabel}: ${formatBarValue(bar, "open", formatters)}`,
    `${copy.highLabel}: ${formatBarValue(bar, "high", formatters)}`,
    `${copy.lowLabel}: ${formatBarValue(bar, "low", formatters)}`,
    `${copy.closeLabel}: ${formatBarValue(bar, "close", formatters)}`,
    `${copy.volumeLabel}: ${formatBarValue(bar, "volume", formatters)}`,
  ].join(" · ");
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
  const copy = mergeCopy(copyInput);
  const geometry = buildPriceChartGeometry(bars);
  const localeValue = safeLocale(locale);
  const formatters = defaultFormatters(localeValue, formattersInput ?? format);
  const available = sourceIndices(geometry);
  const [internalSelectedIndex, setInternalSelectedIndex] = useState<number | undefined>(() =>
    resolveSelectedIndex(initialSelectedIndex, available),
  );
  const [interactionSource, setInteractionSource] = useState<InteractionSource>("keyboard");
  const instanceId = useId();
  const titleId = `${instanceId}-title`;
  const instructionsId = `${instanceId}-instructions`;
  const summaryId = `${instanceId}-summary`;
  const activeSelectedIndex = resolveSelectedIndex(
    selectedIndex ?? internalSelectedIndex,
    available,
  );
  const selectedBar = activeSelectedIndex === undefined ? undefined : bars[activeSelectedIndex];
  const selectedCandle =
    activeSelectedIndex === undefined
      ? undefined
      : geometry.candles.find((candle) => candle.sourceIndex === activeSelectedIndex);
  const selectedDirection = selectedCandle?.direction;
  const summary = selectedSummary(selectedBar, selectedDirection, copy, formatters);
  const rootClassName = joinClasses(styles["chartRoot"], className);

  const selectIndex = (nextIndex: number | undefined, source: InteractionSource): void => {
    if (nextIndex === undefined) return;
    const nextBar = bars[nextIndex];
    if (nextBar === undefined) return;
    const resolvedIndex = resolveSelectedIndex(nextIndex, available);
    if (resolvedIndex === undefined) return;
    setInteractionSource(source);
    if (selectedIndex === undefined) setInternalSelectedIndex(resolvedIndex);
    if (resolvedIndex === activeSelectedIndex) return;
    onObservationChange?.(resolvedIndex, nextBar);
    onSelectedIndexChange?.(resolvedIndex, nextBar);
  };

  const selectFromClientX = (target: Element, clientX: number, source: InteractionSource): void => {
    const rect = target.getBoundingClientRect();
    const nextIndex = findNearestPriceChartIndexFromClientX(
      clientX,
      rect.left,
      rect.width,
      geometry,
    );
    selectIndex(nextIndex, source);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>): void => {
    const key = navigationKey(event.key);
    if (key === undefined) return;
    event.preventDefault();
    selectIndex(movePriceChartSelection(activeSelectedIndex, key, available), "keyboard");
  };

  const handlePointerMove = (event: PointerEvent<HTMLButtonElement>): void => {
    selectFromClientX(event.currentTarget, event.clientX, "pointer");
  };

  const handlePointerDown = (event: PointerEvent<HTMLButtonElement>): void => {
    selectFromClientX(event.currentTarget, event.clientX, "pointer");
  };

  const handleTouch = (event: TouchEvent<HTMLButtonElement>): void => {
    const touch = event.touches.item(0) ?? event.changedTouches.item(0);
    if (touch === null) return;
    event.preventDefault();
    selectFromClientX(event.currentTarget, touch.clientX, "touch");
  };

  const tooltip =
    selectedCandle === undefined || selectedBar === undefined || selectedDirection === undefined
      ? null
      : (() => {
          const position = tooltipPosition(geometry, selectedCandle.x);
          const date = formatDateValue(selectedBar, formatters);
          const lines = [
            {
              key: "date",
              text: `${date} · ${directionLabel(selectedBar, selectedDirection, copy)}`,
            },
            {
              key: "open",
              text: `${copy.openLabel}: ${formatBarValue(selectedBar, "open", formatters)}`,
            },
            {
              key: "high",
              text: `${copy.highLabel}: ${formatBarValue(selectedBar, "high", formatters)}`,
            },
            {
              key: "low",
              text: `${copy.lowLabel}: ${formatBarValue(selectedBar, "low", formatters)}`,
            },
            {
              key: "close",
              text: `${copy.closeLabel}: ${formatBarValue(selectedBar, "close", formatters)}`,
            },
            {
              key: "volume",
              text: `${copy.volumeLabel}: ${formatBarValue(selectedBar, "volume", formatters)}`,
            },
          ];
          return (
            <g className={styles["tooltip"]} data-testid="stock-beta-price-chart-tooltip">
              <title>{candleLabel(selectedBar, selectedDirection, copy, formatters)}</title>
              <rect
                className={styles["tooltipSurface"]}
                height={position.height}
                rx={3}
                width={position.width}
                x={position.x}
                y={position.y}
              />
              <text className={styles["tooltipText"]} x={position.x + 10} y={position.y + 18}>
                {lines.map((line, index) => (
                  <tspan dy={index === 0 ? 0 : 15} key={line.key} x={position.x + 10}>
                    {line.text}
                  </tspan>
                ))}
              </text>
            </g>
          );
        })();

  return (
    <section
      aria-label={copy.chartLabel}
      className={rootClassName}
      data-interaction-source={interactionSource}
      data-selected-index={activeSelectedIndex ?? "none"}
      data-testid="stock-beta-price-chart"
    >
      <p className={styles["srOnly"]} id={instructionsId}>
        {copy.instructions}
      </p>
      <button
        aria-describedby={`${instructionsId} ${summaryId}`}
        aria-keyshortcuts="ArrowLeft ArrowRight Home End"
        aria-label={copy.chartLabel}
        aria-labelledby={titleId}
        className={styles["chartSurface"]}
        data-selected-index={activeSelectedIndex ?? "none"}
        data-testid="stock-beta-price-chart-surface"
        onKeyDown={handleKeyDown}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onTouchMove={handleTouch}
        onTouchStart={handleTouch}
        tabIndex={0}
        type="button"
      >
        <svg
          aria-labelledby={titleId}
          className={styles["chartSvg"]}
          data-selected-index={activeSelectedIndex ?? "none"}
          focusable="true"
          height={geometry.height}
          role="img"
          viewBox={`0 0 ${geometry.width} ${geometry.height}`}
          width={geometry.width}
        >
          <title id={titleId}>{copy.chartLabel}</title>
          <desc>{copy.instructions}</desc>
          {geometry.candles.length === 0 ? (
            <text
              className={styles["emptyLabel"]}
              dominantBaseline="middle"
              textAnchor="middle"
              x={geometry.width / 2}
              y={geometry.height / 2}
            >
              {copy.noDataLabel}
            </text>
          ) : (
            <>
              <g className={styles["grid"]}>
                {geometry.priceTicks.map((tick) => (
                  <line
                    className={styles["gridLine"]}
                    key={`price-grid-${tick.y}`}
                    x1={geometry.plotLeft}
                    x2={geometry.plotRight}
                    y1={tick.y}
                    y2={tick.y}
                  />
                ))}
                <line
                  className={styles["volumeDivider"]}
                  x1={geometry.plotLeft}
                  x2={geometry.plotRight}
                  y1={geometry.volumeTop - 1}
                  y2={geometry.volumeTop - 1}
                />
              </g>
              <g className={styles["axes"]}>
                <text
                  className={styles["axisTitle"]}
                  x={geometry.plotLeft}
                  y={geometry.priceTop - 5}
                >
                  {copy.priceAxisLabel}
                </text>
                {geometry.priceTicks.map((tick) => (
                  <text
                    className={styles["axisLabel"]}
                    key={`price-label-${tick.y}`}
                    textAnchor="end"
                    x={geometry.plotLeft - 8}
                    y={tick.y + 3}
                  >
                    {formatters.price(tick.value)}
                  </text>
                ))}
                <text
                  className={styles["axisTitle"]}
                  textAnchor="end"
                  x={geometry.plotRight}
                  y={geometry.volumeBottom + 22}
                >
                  {copy.volumeAxisLabel}
                </text>
                {geometry.dateTicks.map((tick, index) => {
                  const dateBar = bars[tick.sourceIndex];
                  const anchor =
                    index === 0
                      ? "start"
                      : index === geometry.dateTicks.length - 1
                        ? "end"
                        : "middle";
                  return (
                    <text
                      className={styles["axisLabel"]}
                      key={`date-label-${tick.sourceIndex}`}
                      textAnchor={anchor}
                      x={tick.x}
                      y={geometry.height - 12}
                    >
                      {dateBar === undefined
                        ? tick.sessionDate
                        : formatDateValue(dateBar, formatters)}
                    </text>
                  );
                })}
              </g>
              <g className={styles["smaLines"]}>
                {geometry.sma20.map((segment) => (
                  <polyline
                    className={styles["sma20"]}
                    data-sma="20"
                    fill="none"
                    key={`sma20-${segment.points[0]?.sourceIndex ?? "empty"}`}
                    points={linePoints(segment.points)}
                  />
                ))}
                {geometry.sma60.map((segment) => (
                  <polyline
                    className={styles["sma60"]}
                    data-sma="60"
                    fill="none"
                    key={`sma60-${segment.points[0]?.sourceIndex ?? "empty"}`}
                    points={linePoints(segment.points)}
                  />
                ))}
              </g>
              <g className={styles["volumeBars"]}>
                {geometry.volumes.map((volume) => (
                  <rect
                    className={styles["volumeBar"]}
                    data-direction={volume.direction}
                    data-volume-index={volume.sourceIndex}
                    height={volume.height}
                    key={`volume-${volume.sourceIndex}`}
                    width={volume.width}
                    x={volume.x - volume.width / 2}
                    y={volume.y}
                  />
                ))}
              </g>
              <g className={styles["candles"]}>
                {geometry.candles.map((candle) => {
                  const bar = bars[candle.sourceIndex];
                  if (bar === undefined) return null;
                  const selected = candle.sourceIndex === activeSelectedIndex;
                  return (
                    <g
                      className={joinClasses(
                        styles["candleGroup"],
                        selected ? styles["candleSelected"] : undefined,
                      )}
                      data-candle-index={candle.sourceIndex}
                      data-direction={candle.direction}
                      id={`${instanceId}-candle-${candle.sourceIndex}`}
                      key={`candle-${candle.sourceIndex}`}
                    >
                      <title>{candleLabel(bar, candle.direction, copy, formatters)}</title>
                      <line
                        className={styles["candleWick"]}
                        x1={candle.x}
                        x2={candle.x}
                        y1={candle.highY}
                        y2={candle.lowY}
                      />
                      <rect
                        className={styles["candleBody"]}
                        data-direction={candle.direction}
                        height={candle.bodyHeight}
                        width={candle.bodyWidth}
                        x={candle.bodyX}
                        y={candle.bodyY}
                      />
                    </g>
                  );
                })}
              </g>
              {selectedCandle === undefined ? null : (
                <g className={styles["crosshair"]}>
                  <line
                    x1={selectedCandle.x}
                    x2={selectedCandle.x}
                    y1={geometry.priceTop}
                    y2={geometry.volumeBottom}
                  />
                  <line
                    x1={geometry.plotLeft}
                    x2={geometry.plotRight}
                    y1={selectedCandle.closeY}
                    y2={selectedCandle.closeY}
                  />
                  <circle cx={selectedCandle.x} cy={selectedCandle.closeY} r={3} />
                </g>
              )}
              {tooltip}
            </>
          )}
        </svg>
      </button>
      <div
        aria-live="polite"
        className={styles["summary"]}
        data-direction={selectedDirection ?? "none"}
        data-selected-date={selectedBar?.session_date}
        data-testid="stock-beta-price-chart-summary"
        id={summaryId}
        role="status"
      >
        {summary}
      </div>
      <ul aria-label={copy.chartLabel} className={styles["legend"]}>
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
    </section>
  );
}

export default PriceChart;
