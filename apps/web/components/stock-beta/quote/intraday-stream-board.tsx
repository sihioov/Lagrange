"use client";

import { intradayStreamCopy } from "@/lib/i18n/dictionaries/intraday-stream";
import type { Locale } from "@/lib/i18n/locale";
import type {
  IntradayStreamIdentity,
  IntradayStreamRow,
} from "@/lib/products/intraday-stream-contracts";
import { MARKET_STREAM_INSTRUMENTS } from "@/lib/products/intraday-stream-universe";
import { formatIntradayQuoteDecimal, formatIntradayQuoteSigned } from "./current-quote-view";
import styles from "./intraday-stream.module.css";
import { usePageIntradayStream } from "./intraday-stream-provider";

function rowState(row: IntradayStreamRow | undefined, locale: Locale): string {
  const t = intradayStreamCopy(locale);
  if (!row) return t.awaitingTrade;
  if (row.market_state === "CLOSED") return t.closed;
  if (row.freshness === "STALE") return t.stale;
  if (row.availability === "LIVE") return t.live;
  if (row.availability === "LAST_KNOWN") return t.lastKnown;
  if (row.availability === "AWAITING_FIRST_TRADE") return t.awaitingTrade;
  return t.unavailable;
}

function receiptTime(value: string, locale: Locale): string {
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
    timeZone: "Asia/Seoul",
  }).format(new Date(value));
}

export function IntradayStreamBoard({
  locale,
  selectedInstrumentId,
  onSelect,
}: {
  readonly locale: Locale;
  readonly selectedInstrumentId: string | null;
  readonly onSelect: (instrumentId: string) => void;
}) {
  const { view, identities } = usePageIntradayStream();
  const t = intradayStreamCopy(locale);
  const rows = new Map(view.rows.map((row) => [row.instrument_id, row]));
  const ready = new Set(identities.map((identity) => identity.instrument_id));
  return (
    <section className={styles["board"]} aria-label={t.title} data-testid="stock-beta-stream-board">
      <header>
        <h2>{t.title}</h2>
        <span role="status">
          {view.lifecycle === "failed"
            ? t.failed
            : view.lifecycle === "inactive"
              ? t.paused
              : view.lifecycle === "active"
                ? t.connected
                : t.connecting}
        </span>
      </header>
      <div className={styles["scroll"]}>
        <table>
          <caption>{t.description}</caption>
          <thead>
            <tr>
              {[t.instrument, t.price, t.change, t.tradeAt, t.receivedAt, t.state].map((label) => (
                <th key={label} scope="col">
                  {label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {MARKET_STREAM_INSTRUMENTS.map((instrument) => {
              const row = rows.get(instrument.id);
              const quote = row?.quote;
              return (
                <tr
                  key={instrument.id}
                  data-stream-instrument={instrument.id}
                  data-availability={row?.availability ?? "UNAVAILABLE"}
                  data-quote-version={quote?.quote_version}
                  data-epoch={quote?.epoch}
                  data-received-at={quote?.received_at}
                  data-receive-ordinal={quote?.receive_ordinal}
                >
                  <th scope="row">
                    <button
                      type="button"
                      aria-pressed={selectedInstrumentId === instrument.id}
                      onClick={() => onSelect(instrument.id)}
                    >
                      {instrument.name}
                      <small>{instrument.id}</small>
                    </button>
                  </th>
                  <td data-quote-value={quote?.price}>
                    {quote ? formatIntradayQuoteDecimal(quote.price, locale) : "—"}
                  </td>
                  <td data-direction={quote?.direction}>
                    {quote
                      ? `${formatIntradayQuoteSigned(quote.change_percent_from_previous_day, locale)}%`
                      : "—"}
                  </td>
                  <td>
                    {quote ? (
                      <time dateTime={quote.provider_trade_at}>{quote.trade_time}</time>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td>
                    {quote ? (
                      <time dateTime={quote.received_at}>
                        {receiptTime(quote.received_at, locale)}
                      </time>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td>
                    {ready.has(instrument.id) ? rowState(row, locale) : t.awaitingAdmission}
                    {quote?.halted === true ? <small>{t.halted}</small> : null}
                    {row?.session_has_gap ? <small>{t.gap}</small> : null}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

export function StreamSelectedQuote({
  identity,
  instrumentId,
  locale,
}: {
  readonly identity: IntradayStreamIdentity | null;
  readonly instrumentId?: string | null;
  readonly locale: Locale;
}) {
  const { view } = usePageIntradayStream();
  const t = intradayStreamCopy(locale);
  const row =
    identity === null
      ? undefined
      : view.rows.find(
          (candidate) =>
            candidate.membership_id === identity.membership_id &&
            candidate.instrument_id === identity.instrument_id &&
            candidate.generation === identity.generation,
        );
  const quote = row?.quote;
  return (
    <section
      className={styles["detail"]}
      data-testid="stock-beta-stream-selected"
      data-availability={row?.availability ?? "UNAVAILABLE"}
    >
      <h3>{t.detail}</h3>
      <p>
        {identity?.instrument_id ?? instrumentId ?? t.noSelection} · {rowState(row, locale)}
        {quote?.halted === true ? <small>{t.halted}</small> : null}
      </p>
      <strong data-quote-value={quote?.price}>
        {quote ? formatIntradayQuoteDecimal(quote.price, locale) : "—"} KRW
      </strong>
      <dl>
        <div>
          <dt>{t.change}</dt>
          <dd>
            {quote
              ? `${formatIntradayQuoteSigned(quote.change_from_previous_day, locale)} (${formatIntradayQuoteSigned(quote.change_percent_from_previous_day, locale)}%)`
              : "—"}
          </dd>
        </div>
        <div>
          <dt>{t.basePrice}</dt>
          <dd>{t.baseAbsent}</dd>
        </div>
        <div>
          <dt>{t.volume}</dt>
          <dd>
            {quote
              ? `${formatIntradayQuoteDecimal(quote.trade_volume, locale)} / ${formatIntradayQuoteDecimal(quote.cumulative_volume, locale)}`
              : "—"}
          </dd>
        </div>
        <div>
          <dt>{t.tradeAt}</dt>
          <dd>
            {quote ? <time dateTime={quote.provider_trade_at}>{quote.trade_time}</time> : "—"}
          </dd>
        </div>
        <div>
          <dt>{t.receivedAt}</dt>
          <dd>
            {quote ? (
              <time dateTime={quote.received_at}>{receiptTime(quote.received_at, locale)}</time>
            ) : (
              "—"
            )}
          </dd>
        </div>
      </dl>
      {row?.session_has_gap ? <p>{t.gap}</p> : null}
    </section>
  );
}
