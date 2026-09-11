import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import { type StockBetaWidgetFrameState, WidgetFrame } from "../shared/widget-frame";
import styles from "./current-quote.module.css";
import type { IntradayQuoteLoadState } from "./quote-load-coordinator";

export type IntradayQuoteDictionary = Pick<
  StockBetaDictionary,
  Extract<keyof StockBetaDictionary, `intradayQuote${string}`>
>;

function groupedInteger(value: string): string {
  const groups: string[] = [];
  for (let end = value.length; end > 0; end -= 3) {
    groups.unshift(value.slice(Math.max(0, end - 3), end));
  }
  return groups.join(",");
}

export function formatIntradayQuoteDecimal(value: string, locale: Locale): string {
  const negative = value.startsWith("-");
  const unsigned = negative ? value.slice(1) : value;
  const [integer, fraction] = unsigned.split(".");
  const separator = locale === "ko" ? "." : ".";
  return `${negative ? "-" : ""}${groupedInteger(integer ?? "0")}${fraction === undefined ? "" : `${separator}${fraction}`}`;
}

export function formatIntradayQuoteSigned(value: string, locale: Locale): string {
  if (value.startsWith("-")) return formatIntradayQuoteDecimal(value, locale);
  if (/^0(?:\.0+)?$/.test(value)) return formatIntradayQuoteDecimal(value, locale);
  return `+${formatIntradayQuoteDecimal(value, locale)}`;
}

function directionLabel(
  direction: NonNullable<NonNullable<IntradayQuoteLoadState["quote"]>["quote"]>["direction"],
  t: IntradayQuoteDictionary,
): string {
  return direction === "UP"
    ? t.intradayQuoteDirectionUp
    : direction === "DOWN"
      ? t.intradayQuoteDirectionDown
      : direction === "FLAT"
        ? t.intradayQuoteDirectionFlat
        : direction === "LIMIT_UP"
          ? t.intradayQuoteDirectionLimitUp
          : t.intradayQuoteDirectionLimitDown;
}

const RETAINED_REFRESH_FAILURE_REASONS = new Set<NonNullable<IntradayQuoteLoadState["reasonCode"]>>(
  [
    "PROVIDER_TIMEOUT",
    "PROVIDER_RATE_LIMITED",
    "PROVIDER_UNAVAILABLE",
    "PROVIDER_RESPONSE_INVALID",
    "QUOTE_VALUE_INVALID",
    "QUOTE_BUDGET_EXHAUSTED",
    "PRODUCER_UNAVAILABLE",
  ],
);

function hasVisibleQuote(state: IntradayQuoteLoadState): boolean {
  return state.quote !== null && state.quote.quote !== null;
}

function hasRetainedRefreshFailure(state: IntradayQuoteLoadState): boolean {
  return (
    hasVisibleQuote(state) &&
    state.reasonCode !== null &&
    RETAINED_REFRESH_FAILURE_REASONS.has(state.reasonCode)
  );
}

function statusText(state: IntradayQuoteLoadState, t: IntradayQuoteDictionary): string {
  if (state.phase === "offline") return t.intradayQuoteOffline;
  if (state.phase === "demanding") return t.intradayQuoteDemanding;
  const retainedRefreshFailure = hasRetainedRefreshFailure(state);
  let semanticStatus: string;
  if (retainedRefreshFailure) {
    semanticStatus = t.intradayQuoteRefreshFailed;
  } else if (state.reasonCode === "NO_ACTIVE_DEMAND" && hasVisibleQuote(state)) {
    semanticStatus = t.intradayQuoteNoActiveDemand;
  } else if (state.reasonCode === "SESSION_CLOSED") {
    semanticStatus = t.intradayQuoteClosed;
  } else if (state.reasonCode === "INSTRUMENT_HALTED") {
    semanticStatus = t.intradayQuoteHalted;
  } else if (state.phase === "polling" && state.quote !== null) {
    semanticStatus =
      state.quote.freshness === "STALE" ? t.intradayQuoteStale : t.intradayQuoteReady;
  } else if (state.phase === "polling") {
    semanticStatus = t.intradayQuotePolling;
  } else if (state.phase === "stale") {
    semanticStatus = t.intradayQuoteStale;
  } else if (state.phase === "unavailable" || state.phase === "error") {
    semanticStatus = t.intradayQuoteUnavailable;
  } else if (state.phase === "ready") {
    semanticStatus = t.intradayQuoteReady;
  } else {
    semanticStatus = t.intradayQuoteUnavailable;
  }
  const marketStatus =
    state.marketState === "HALTED"
      ? t.intradayQuoteHalted
      : state.marketState === "CLOSED"
        ? t.intradayQuoteClosed
        : state.marketState === "UNKNOWN"
          ? t.intradayQuoteUnknown
          : null;
  return marketStatus === null || marketStatus === semanticStatus
    ? semanticStatus
    : `${marketStatus} · ${semanticStatus}`;
}

function frameState(
  state: IntradayQuoteLoadState,
  t: IntradayQuoteDictionary,
): StockBetaWidgetFrameState {
  if (state.quote?.quote !== null && state.quote !== null) return { kind: "ready" };
  if (state.phase === "demanding" || state.phase === "polling") {
    return { kind: "loading", message: t.intradayQuotePolling };
  }
  return { kind: "empty", message: statusText(state, t) };
}

export type CurrentQuoteViewProps = {
  readonly copy: IntradayQuoteDictionary;
  readonly locale: Locale;
  readonly state: IntradayQuoteLoadState;
};

export function CurrentQuoteView({ copy: t, locale, state }: CurrentQuoteViewProps) {
  const response = state.quote;
  const quote = response?.quote;
  const retainedRefreshFailure = hasRetainedRefreshFailure(state);
  const quoteContent =
    response === null || quote === null || quote === undefined ? null : (
      <div className={styles["quote"]} data-testid="stock-beta-current-quote">
        <div className={styles["priceRow"]}>
          <span className={styles["priceLabel"]}>{t.intradayQuotePriceLabel}</span>
          <strong className={styles["price"]} data-quote-value={quote.price}>
            {formatIntradayQuoteDecimal(quote.price, locale)}
          </strong>
          <span className={styles["currency"]}>KRW</span>
        </div>
        <dl className={styles["metrics"]}>
          <div>
            <dt>{t.intradayQuoteBasePriceLabel}</dt>
            <dd>{formatIntradayQuoteDecimal(quote.base_price, locale)}</dd>
            <small>{t.intradayQuoteBasePricePolicy}</small>
          </div>
          <div>
            <dt>{t.intradayQuoteChangeLabel}</dt>
            <dd data-direction={quote.direction}>
              {formatIntradayQuoteSigned(quote.change_from_previous_day, locale)}
            </dd>
          </div>
          <div>
            <dt>{t.intradayQuoteChangePercentLabel}</dt>
            <dd data-direction={quote.direction}>
              {formatIntradayQuoteSigned(quote.change_percent_from_previous_day, locale)}%
            </dd>
          </div>
          <div>
            <dt>{t.intradayQuoteDirectionLabel}</dt>
            <dd data-direction={quote.direction}>{directionLabel(quote.direction, t)}</dd>
          </div>
        </dl>
        <p className={styles["lastSuccess"]} data-last-success-at={quote.last_success_at}>
          {t.intradayQuoteLastSuccessLabel}: {quote.last_success_at}
        </p>
      </div>
    );
  return (
    <WidgetFrame
      description={t.intradayQuoteDescription}
      state={frameState(state, t)}
      status={
        <span
          aria-live="polite"
          className={styles["status"]}
          data-fetching={state.fetching ? "true" : "false"}
          data-market-state={state.marketState ?? undefined}
          data-request-failure={retainedRefreshFailure ? "true" : undefined}
          data-status-phase={state.phase}
        >
          {statusText(state, t)}
        </span>
      }
      title={t.intradayQuoteHeading}
    >
      {quoteContent}
    </WidgetFrame>
  );
}
