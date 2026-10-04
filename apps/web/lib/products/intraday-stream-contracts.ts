import { z } from "zod";

export const INTRADAY_STREAM_SCHEMA_VERSION = 2 as const;
export const INTRADAY_STREAM_SOURCE = "KIS_MARKET_WS" as const;
export const INTRADAY_STREAM_WIRE_VERSION = "kis-h0stcnt0-20260914-v1" as const;
export const INTRADAY_STREAM_RECENT_MS = 30_000;

const I64_MAX = 9_223_372_036_854_775_807n;
const ZERO_UUID = "00000000-0000-0000-0000-000000000000";
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const DECIMAL = /^-?(0|[1-9][0-9]{0,11})(?:\.[0-9]{1,8})?$/;
const INTEGER = /^(0|[1-9][0-9]*)$/;
const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const TIME = /^(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]$/;
const TIMESTAMP = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2})(?:\.([0-9]{1,9}))?(Z|\+09:00)$/;

function calendarDate(value: string): boolean {
  const match = DATE.exec(value);
  if (!match) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  return year > 0 && date.toISOString().slice(0, 10) === value;
}

function timestamp(value: string): boolean {
  const match = TIMESTAMP.exec(value);
  return Boolean(match && calendarDate(match[1] ?? "") && TIME.test(match[2] ?? ""));
}

/** Preserve sub-millisecond boundaries instead of rounding a stale value to recent. */
function timestampNs(value: string): bigint {
  const match = TIMESTAMP.exec(value);
  if (!match || !timestamp(value)) throw new IntradayStreamContractError();
  const wholeSecond = Date.parse(`${match[1]}T${match[2]}${match[4]}`);
  return BigInt(wholeSecond) * 1_000_000n + BigInt((match[3] ?? "").padEnd(9, "0"));
}

function kstDate(value: string): string {
  return new Date(Date.parse(value) + 9 * 60 * 60 * 1_000).toISOString().slice(0, 10);
}

function decimalSign(value: string): -1 | 0 | 1 {
  if (/^-?0(?:\.0+)?$/.test(value)) return 0;
  return value.startsWith("-") ? -1 : 1;
}

function issue(context: z.RefinementCtx, message: string, path: string[] = []): void {
  context.addIssue({ code: "custom", message, path });
}

const uuid = z
  .string()
  .regex(UUID)
  .refine((value) => value !== ZERO_UUID);
const integer = z
  .string()
  .max(19)
  .regex(INTEGER)
  .refine((value) => value.length <= 19 && INTEGER.test(value) && BigInt(value) <= I64_MAX);
const positiveInteger = integer.refine((value) => value !== "0");
const date = z.string().regex(DATE).refine(calendarDate);
const utcTimestamp = z
  .string()
  .max(30)
  .refine((value) => value.endsWith("Z") && timestamp(value));
const providerTimestamp = z
  .string()
  .max(25)
  .refine((value) => !value.includes(".") && timestamp(value));
const decimal = z
  .string()
  .max(22)
  .regex(DECIMAL)
  .refine((value) => !(value.startsWith("-") && decimalSign(value) === 0));

export const intradayStreamReasonSchema = z.enum([
  "FEATURE_DISABLED",
  "NO_ACTIVE_DEMAND",
  "CALENDAR_UNAVAILABLE",
  "SESSION_WINDOW_UNAVAILABLE",
  "SESSION_CLOSED",
  "AWAITING_FIRST_TRADE",
  "QUOTE_STALE",
  "CONNECTION_LOST",
  "RECONNECT_GAP",
  "SUBSCRIPTION_PENDING",
  "SUBSCRIPTION_REJECTED",
  "SUBSCRIPTION_AMBIGUOUS",
  "APPROVAL_UNAVAILABLE",
  "BUDGET_EXHAUSTED",
  "PRODUCER_UNAVAILABLE",
  "PIPELINE_LAG",
  "WIRE_SCHEMA_MISMATCH",
  "PROVIDER_RESPONSE_INVALID",
  "QUOTE_VALUE_INVALID",
  "MARKET_CLASS_UNSUPPORTED",
  "LOCAL_INGRESS_LIMIT",
  "RESYNC_REQUIRED",
  "ACCESS_REVOKED",
]);

const sessionSchema = z
  .object({
    date,
    timezone: z.literal("Asia/Seoul"),
    calendar_source: z.literal("kis"),
    calendar_source_version: z.literal("kis-chk-holiday-v1:schema-1"),
    calendar_content_sha256: z.string().regex(/^[0-9a-f]{64}$/),
    window_contract_sha256: z.string().regex(/^sha256:[0-9a-f]{64}$/),
  })
  .strict();

export const intradayStreamQuoteSchema = z
  .object({
    price: decimal.refine((value) => decimalSign(value) === 1),
    base_price: z.null(),
    base_price_reason: z.literal("NOT_PROVIDED_BY_CHANNEL"),
    change_from_previous_day: decimal,
    change_percent_from_previous_day: decimal,
    direction: z.enum(["UP", "DOWN", "FLAT", "LIMIT_UP", "LIMIT_DOWN"]),
    trade_volume: integer,
    cumulative_volume: integer,
    halted: z.boolean(),
    business_date: date,
    trade_time: z.string().regex(TIME),
    provider_trade_at: providerTimestamp,
    received_at: utcTimestamp,
    committed_at: utcTimestamp,
    epoch: uuid,
    quote_version: positiveInteger,
    receive_ordinal: positiveInteger,
  })
  .strict()
  .superRefine((quote, context) => {
    const change = decimalSign(quote.change_from_previous_day);
    const percent = decimalSign(quote.change_percent_from_previous_day);
    const consistent =
      quote.direction === "UP" || quote.direction === "LIMIT_UP"
        ? change === 1 && percent >= 0
        : quote.direction === "DOWN" || quote.direction === "LIMIT_DOWN"
          ? change === -1 && percent <= 0
          : change === 0 && percent === 0;
    if (!consistent) issue(context, "direction contradicts signed values", ["direction"]);
    if (
      !calendarDate(quote.business_date) ||
      !TIME.test(quote.trade_time) ||
      !timestamp(quote.provider_trade_at) ||
      !timestamp(quote.received_at) ||
      !timestamp(quote.committed_at)
    )
      return;
    const event = timestampNs(quote.provider_trade_at);
    const receipt = timestampNs(quote.received_at);
    const commit = timestampNs(quote.committed_at);
    if (event !== timestampNs(`${quote.business_date}T${quote.trade_time}+09:00`)) {
      issue(context, "provider instant does not match the KST trade date and time", [
        "provider_trade_at",
      ]);
    }
    if (
      kstDate(quote.received_at) !== quote.business_date ||
      kstDate(quote.committed_at) !== quote.business_date
    ) {
      issue(context, "quote capture crossed the business date");
    }
    if (receipt > commit || commit - receipt > 3_000_000_000n) {
      issue(context, "quote missed the publication deadline", ["committed_at"]);
    }
    if (event > receipt + 2_000_000_000n)
      issue(context, "provider time exceeds clock tolerance", ["provider_trade_at"]);
  });

export const intradayStreamRowSchema = z
  .object({
    membership_id: uuid,
    instrument_id: z.string().regex(/^[0-9]{6}\.KRX$/),
    generation: z.number().int().safe().positive(),
    row_generation: uuid,
    venue: z.literal("KRX"),
    currency: z.literal("KRW"),
    source: z.literal(INTRADAY_STREAM_SOURCE),
    wire_version: z.literal(INTRADAY_STREAM_WIRE_VERSION),
    session: sessionSchema.nullable(),
    subscription: z.enum(["DESIRED", "PENDING", "ACKED", "REJECTED", "ABSENT"]),
    connection: z.enum(["DISCONNECTED", "CONNECTING", "CONNECTED", "BACKOFF", "STOPPED"]),
    market_state: z.enum(["OPEN", "CLOSED", "UNKNOWN"]),
    freshness: z.enum(["RECENT", "STALE", "UNAVAILABLE"]),
    availability: z.enum(["LIVE", "LAST_KNOWN", "AWAITING_FIRST_TRADE", "UNAVAILABLE"]),
    reason_code: intradayStreamReasonSchema.nullable(),
    state_version: integer,
    gap_open: z.boolean(),
    session_has_gap: z.boolean(),
    gap_generation: integer,
    quote: intradayStreamQuoteSchema.nullable(),
  })
  .strict()
  .superRefine((row, context) => {
    if (row.session === null && row.market_state !== "UNKNOWN")
      issue(context, "known market state requires evidence", ["session"]);
    if (row.quote !== null) {
      if (
        row.reason_code === "FEATURE_DISABLED" ||
        row.reason_code === "CALENDAR_UNAVAILABLE" ||
        row.reason_code === "SESSION_WINDOW_UNAVAILABLE" ||
        row.reason_code === "ACCESS_REVOKED"
      )
        issue(context, "unavailable authority cannot carry quote values", ["quote"]);
      if (
        row.session === null ||
        row.market_state === "UNKNOWN" ||
        row.session.date !== row.quote.business_date
      ) {
        issue(context, "quote requires matching valid session evidence", ["quote"]);
      }
      if (
        row.availability === "UNAVAILABLE" ||
        row.availability === "AWAITING_FIRST_TRADE" ||
        row.freshness === "UNAVAILABLE"
      ) {
        issue(context, "unavailable state cannot carry quote values", ["quote"]);
      }
    } else if (
      row.availability === "LIVE" ||
      row.availability === "LAST_KNOWN" ||
      row.freshness !== "UNAVAILABLE"
    ) {
      issue(context, "quote state requires an observation", ["quote"]);
    }
    if (
      row.availability === "LIVE" &&
      (row.subscription !== "ACKED" ||
        row.connection !== "CONNECTED" ||
        row.market_state !== "OPEN" ||
        row.freshness !== "RECENT" ||
        row.reason_code !== null ||
        row.gap_open)
    ) {
      issue(context, "LIVE contradicts current delivery state", ["availability"]);
    }
    if (row.gap_open && !row.session_has_gap)
      issue(context, "open gap requires session gap history", ["session_has_gap"]);
  });

export type IntradayStreamRow = z.infer<typeof intradayStreamRowSchema>;
export type IntradayStreamQuote = z.infer<typeof intradayStreamQuoteSchema>;

export class IntradayStreamContractError extends Error {
  override readonly name = "IntradayStreamContractError";
  readonly code = "INTRADAY_STREAM_CONTRACT_INVALID" as const;

  constructor() {
    super("Market stream data did not match the approved contract");
  }
}

/** Validate against authenticated event time, not the browser's possibly skewed clock. */
export function parseIntradayStreamRow(input: unknown, serverTime: string): IntradayStreamRow {
  const parsed = intradayStreamRowSchema.safeParse(input);
  if (!parsed.success || !utcTimestamp.safeParse(serverTime).success)
    throw new IntradayStreamContractError();
  const row = parsed.data;
  if (row.session !== null && row.session.date !== kstDate(serverTime))
    throw new IntradayStreamContractError();
  if (row.quote === null) return row;
  const now = timestampNs(serverTime);
  const receipt = timestampNs(row.quote.received_at);
  const event = timestampNs(row.quote.provider_trade_at);
  if (timestampNs(row.quote.committed_at) > now || receipt > now)
    throw new IntradayStreamContractError();
  if (
    row.freshness === "RECENT" &&
    [receipt, event].some((time) => time > now || now - time > 30_000_000_000n)
  ) {
    throw new IntradayStreamContractError();
  }
  return row;
}

export const intradayStreamIdentitySchema = z
  .object({
    membership_id: uuid,
    instrument_id: z.string().regex(/^[0-9]{6}\.KRX$/),
    generation: z.number().int().safe().positive(),
  })
  .strict();

export type IntradayStreamIdentity = z.infer<typeof intradayStreamIdentitySchema>;
export const intradayStreamUuidSchema = uuid;

const eventHeader = {
  schema_version: z.literal(INTRADAY_STREAM_SCHEMA_VERSION),
  stream_id: uuid,
  event_sequence: positiveInteger,
  server_time: utcTimestamp,
};
const eventRows = z.array(intradayStreamRowSchema).min(1).max(30);
const snapshotEventSchema = z
  .object({
    ...eventHeader,
    body: z.object({ lease_id: uuid, lease_expires_at: utcTimestamp, rows: eventRows }).strict(),
  })
  .strict();
const deltaEventSchema = z
  .object({ ...eventHeader, body: z.object({ rows: eventRows }).strict() })
  .strict();
const statusEventSchema = z
  .object({
    ...eventHeader,
    body: z
      .object({
        connection: intradayStreamRowSchema.shape.connection,
        reason_code: intradayStreamReasonSchema.nullable(),
        gap_open: z.boolean(),
        session_has_gap: z.boolean(),
        gap_generation: integer,
      })
      .strict(),
  })
  .strict();
const resetEventSchema = z
  .object({
    ...eventHeader,
    body: z.object({ reason_code: z.literal("RESYNC_REQUIRED") }).strict(),
  })
  .strict();

export type IntradayStreamEvent =
  | ({ kind: "snapshot" } & z.infer<typeof snapshotEventSchema>)
  | ({ kind: "delta" } & z.infer<typeof deltaEventSchema>)
  | ({ kind: "status" } & z.infer<typeof statusEventSchema>)
  | ({ kind: "reset" } & z.infer<typeof resetEventSchema>);

/** Event names and IDs are transport fields, never duplicated inside the strict JSON body. */
export function parseIntradayStreamEvent(
  kind: string,
  data: string,
  eventId: string,
): IntradayStreamEvent {
  // Bound before JSON parsing; ASCII is the approved contract, but count UTF-8 bytes too.
  if (data.length > 256 * 1024 || new TextEncoder().encode(data).length > 256 * 1024)
    throw new IntradayStreamContractError();
  let input: unknown;
  try {
    input = JSON.parse(data);
  } catch {
    throw new IntradayStreamContractError();
  }
  let event: IntradayStreamEvent;
  if (kind === "snapshot") {
    const parsed = snapshotEventSchema.safeParse(input);
    if (!parsed.success) throw new IntradayStreamContractError();
    event = { ...parsed.data, kind };
    if (timestampNs(event.body.lease_expires_at) <= timestampNs(event.server_time))
      throw new IntradayStreamContractError();
  } else if (kind === "delta") {
    const parsed = deltaEventSchema.safeParse(input);
    if (!parsed.success) throw new IntradayStreamContractError();
    event = { ...parsed.data, kind };
  } else if (kind === "status") {
    const parsed = statusEventSchema.safeParse(input);
    if (!parsed.success || (parsed.data.body.gap_open && !parsed.data.body.session_has_gap))
      throw new IntradayStreamContractError();
    event = { ...parsed.data, kind };
  } else if (kind === "reset") {
    const parsed = resetEventSchema.safeParse(input);
    if (!parsed.success) throw new IntradayStreamContractError();
    event = { ...parsed.data, kind };
  } else {
    throw new IntradayStreamContractError();
  }
  if (eventId !== `${event.stream_id}:${event.event_sequence}`)
    throw new IntradayStreamContractError();
  if (event.kind === "snapshot" || event.kind === "delta") {
    let previousMembership = "";
    const instruments = new Set<string>();
    for (const row of event.body.rows) {
      // The canonical wire order follows the exact membership set, not display ranking.
      if (row.membership_id <= previousMembership || instruments.has(row.instrument_id))
        throw new IntradayStreamContractError();
      previousMembership = row.membership_id;
      instruments.add(row.instrument_id);
      parseIntradayStreamRow(row, event.server_time);
    }
  }
  return event;
}

export function intradayStreamTimeNs(value: string): bigint {
  if (!utcTimestamp.safeParse(value).success) throw new IntradayStreamContractError();
  return timestampNs(value);
}

/** A display overlay never changes the captured quote, its versions or timestamps. */
export function ageIntradayStreamRow(
  row: IntradayStreamRow,
  serverTime: string,
  elapsedMs = 0,
): IntradayStreamRow {
  if (!Number.isFinite(elapsedMs) || elapsedMs < 0 || elapsedMs > 90 * 86400_000)
    throw new IntradayStreamContractError();
  const now = intradayStreamTimeNs(serverTime) + BigInt(Math.ceil(elapsedMs * 1_000_000));
  const day = new Date(Number(now / 1_000_000n) + 9 * 3600_000).toISOString().slice(0, 10);
  if (row.session !== null && row.session.date !== day) {
    return {
      ...row,
      session: null,
      quote: null,
      market_state: "UNKNOWN",
      freshness: "UNAVAILABLE",
      availability: "UNAVAILABLE",
      reason_code: "CALENDAR_UNAVAILABLE",
    };
  }
  if (
    row.quote !== null &&
    [timestampNs(row.quote.received_at), timestampNs(row.quote.provider_trade_at)].some(
      (time) => time > now || now - time > 30_000_000_000n,
    )
  ) {
    return {
      ...row,
      freshness: "STALE",
      availability: "LAST_KNOWN",
      reason_code: row.reason_code ?? "QUOTE_STALE",
    };
  }
  return row;
}
