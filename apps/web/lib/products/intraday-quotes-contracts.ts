import { z } from "zod";

export const INTRADAY_QUOTE_SCHEMA_VERSION = 1 as const;
export const INTRADAY_QUOTE_DEMAND_PATH =
  "/api/v1/research/owner-beta/equity-universe-v2/quote-demands" as const;
export const INTRADAY_QUOTE_CACHE_PATH =
  "/api/v1/research/owner-beta/equity-universe-v2/instruments" as const;
export const INTRADAY_QUOTE_CACHE_MAX_AGE_MS = 24 * 60 * 60 * 1_000;
export const INTRADAY_QUOTE_STALE_AFTER_MS = 30 * 1_000;
export const INTRADAY_QUOTE_POLL_INTERVAL_MS = 5_000;
export const INTRADAY_QUOTE_RENEWAL_INTERVAL_MS = 15_000;
export const INTRADAY_QUOTE_LEASE_MS = 30_000;

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const INSTRUMENT_PATTERN = /^\d{6}\.KRX$/;
const DECIMAL_PATTERN = /^-?(0|[1-9][0-9]*)(?:\.[0-9]{1,8})?$/;
const BIGINT_PATTERN = /^(0|[1-9][0-9]*)$/;
const DATE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;
const UTC_DATE_TIME_PATTERN =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.([0-9]{1,9}))?Z$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const WINDOW_CONTRACT_SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const I64_MAX = 9_223_372_036_854_775_807n;

function addIssue(context: z.RefinementCtx, message: string, path?: (string | number)[]): void {
  context.addIssue({ code: "custom", message, ...(path === undefined ? {} : { path }) });
}

function isLeapYear(year: number): boolean {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

function daysInMonth(year: number, month: number): number {
  if (month === 2) return isLeapYear(year) ? 29 : 28;
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

function isCalendarDate(value: string): boolean {
  const match = DATE_PATTERN.exec(value);
  if (match === null) return false;
  const year = Number.parseInt(match[1] ?? "", 10);
  const month = Number.parseInt(match[2] ?? "", 10);
  const day = Number.parseInt(match[3] ?? "", 10);
  return year >= 1 && month >= 1 && month <= 12 && day >= 1 && day <= daysInMonth(year, month);
}

function isUtcDateTime(value: string): boolean {
  const match = UTC_DATE_TIME_PATTERN.exec(value);
  if (match === null || !isCalendarDate(value.slice(0, 10))) return false;
  const hour = Number.parseInt(match[4] ?? "", 10);
  const minute = Number.parseInt(match[5] ?? "", 10);
  const second = Number.parseInt(match[6] ?? "", 10);
  return (
    hour >= 0 &&
    hour <= 23 &&
    minute >= 0 &&
    minute <= 59 &&
    second >= 0 &&
    second <= 59 &&
    Number.isFinite(Date.parse(value))
  );
}

function isZeroDecimal(value: string): boolean {
  const unsigned = value.startsWith("-") ? value.slice(1) : value;
  const [integer, fraction = ""] = unsigned.split(".");
  return /^0+$/.test(integer ?? "") && (fraction === "" || /^0+$/.test(fraction));
}

function decimalIntegerDigits(value: string): number {
  return (value.startsWith("-") ? value.slice(1) : value).split(".")[0]?.length ?? 0;
}

function isCanonicalDecimal(value: string): boolean {
  return (
    DECIMAL_PATTERN.test(value) &&
    !(value.startsWith("-") && isZeroDecimal(value)) &&
    decimalIntegerDigits(value) <= 12
  );
}

function decimalSchema(): z.ZodType<string> {
  return z
    .string()
    .regex(DECIMAL_PATTERN)
    .superRefine((value, context) => {
      if (decimalIntegerDigits(value) > 12)
        addIssue(context, "decimal integer part exceeds 12 digits");
      if (value.startsWith("-") && isZeroDecimal(value))
        addIssue(context, "negative zero is not canonical");
    });
}

const decimalStringSchema = decimalSchema();
const positiveDecimalStringSchema = decimalStringSchema.superRefine((value, context) => {
  if (value.startsWith("-") || isZeroDecimal(value)) addIssue(context, "decimal must be positive");
});

const uuidSchema = z.string().regex(UUID_PATTERN);
const instrumentIdSchema = z.string().regex(INSTRUMENT_PATTERN);
const positiveSafeIntegerSchema = z.number().int().safe().positive();
const nonnegativeSafeIntegerSchema = z.number().int().safe().nonnegative();
const dateSchema = z
  .string()
  .regex(DATE_PATTERN)
  .superRefine((value, context) => {
    if (!isCalendarDate(value)) addIssue(context, "date is not a genuine calendar date");
  });
const utcDateTimeSchema = z
  .string()
  .regex(UTC_DATE_TIME_PATTERN)
  .superRefine((value, context) => {
    if (!isUtcDateTime(value)) addIssue(context, "datetime is not a genuine UTC ISO datetime");
  });

export const intradayQuoteDirectionSchema = z.enum([
  "UP",
  "DOWN",
  "FLAT",
  "LIMIT_UP",
  "LIMIT_DOWN",
]);
export type IntradayQuoteDirection = z.infer<typeof intradayQuoteDirectionSchema>;

export const intradayQuoteMarketStateSchema = z.enum(["OPEN", "CLOSED", "HALTED", "UNKNOWN"]);
export type IntradayQuoteMarketState = z.infer<typeof intradayQuoteMarketStateSchema>;

export const intradayQuoteFreshnessSchema = z.enum(["RECENT", "STALE", "UNAVAILABLE"]);
export type IntradayQuoteFreshness = z.infer<typeof intradayQuoteFreshnessSchema>;

export const intradayQuoteReasonCodeSchema = z.enum([
  "NO_ACTIVE_DEMAND",
  "QUOTE_PENDING",
  "QUOTE_STALE",
  "PROVIDER_TIMEOUT",
  "PROVIDER_RATE_LIMITED",
  "PROVIDER_UNAVAILABLE",
  "PROVIDER_RESPONSE_INVALID",
  "QUOTE_VALUE_INVALID",
  "QUOTE_BUDGET_EXHAUSTED",
  "CALENDAR_UNAVAILABLE",
  "SESSION_WINDOW_UNAVAILABLE",
  "SESSION_CLOSED",
  "INSTRUMENT_HALTED",
  "PRODUCER_UNAVAILABLE",
  "FEATURE_DISABLED",
]);
export type IntradayQuoteReasonCode = z.infer<typeof intradayQuoteReasonCodeSchema>;

export const intradayQuoteIdentitySchema = z
  .object({
    membership_id: uuidSchema,
    instrument_id: instrumentIdSchema,
    generation: positiveSafeIntegerSchema,
  })
  .strict();
export type IntradayQuoteIdentity = z.infer<typeof intradayQuoteIdentitySchema>;

export const intradayQuoteDemandRequestSchema = z
  .object({
    schema_version: z.literal(INTRADAY_QUOTE_SCHEMA_VERSION),
    consumer_id: uuidSchema,
    membership_id: uuidSchema,
    generation: positiveSafeIntegerSchema,
    renewal_sequence: nonnegativeSafeIntegerSchema,
  })
  .strict();
export type IntradayQuoteDemandRequest = z.infer<typeof intradayQuoteDemandRequestSchema>;

export const intradayQuoteDemandResponseSchema = z
  .object({
    schema_version: z.literal(INTRADAY_QUOTE_SCHEMA_VERSION),
    demand_id: uuidSchema,
    consumer_id: uuidSchema,
    membership_id: uuidSchema,
    instrument_id: instrumentIdSchema,
    generation: positiveSafeIntegerSchema,
    renewal_sequence: nonnegativeSafeIntegerSchema,
    lease_expires_at: utcDateTimeSchema,
    renew_after_ms: z.literal(INTRADAY_QUOTE_RENEWAL_INTERVAL_MS),
  })
  .strict();
export type IntradayQuoteDemandResponse = z.infer<typeof intradayQuoteDemandResponseSchema>;

export const intradayQuoteReleaseRequestSchema = z
  .object({
    schema_version: z.literal(INTRADAY_QUOTE_SCHEMA_VERSION),
    consumer_id: uuidSchema,
    renewal_sequence: nonnegativeSafeIntegerSchema,
  })
  .strict();
export type IntradayQuoteReleaseRequest = z.infer<typeof intradayQuoteReleaseRequestSchema>;

const intradayQuoteSessionSchema = z
  .object({
    date: dateSchema,
    timezone: z.literal("Asia/Seoul"),
    calendar_source: z.literal("kis"),
    calendar_source_version: z.literal("kis-chk-holiday-v1:schema-1"),
    calendar_content_sha256: z.string().regex(SHA256_PATTERN),
    window_contract_sha256: z.string().regex(WINDOW_CONTRACT_SHA256_PATTERN),
  })
  .strict();

const intradayQuoteVersionSchema = z
  .string()
  .regex(BIGINT_PATTERN)
  .superRefine((value, context) => {
    try {
      if (BigInt(value) > I64_MAX) addIssue(context, "quote_version exceeds signed 64-bit range");
    } catch {
      addIssue(context, "quote_version is not a canonical bigint");
    }
  });

export const intradayQuotePayloadSchema = z
  .object({
    price: positiveDecimalStringSchema,
    base_price: positiveDecimalStringSchema,
    change_from_previous_day: decimalStringSchema,
    change_percent_from_previous_day: decimalStringSchema,
    direction: intradayQuoteDirectionSchema,
    received_at: utcDateTimeSchema,
    last_success_at: utcDateTimeSchema,
    quote_version: intradayQuoteVersionSchema,
  })
  .strict()
  .superRefine((quote, context) => {
    if (quote.received_at !== quote.last_success_at) {
      addIssue(context, "received_at must equal last_success_at", ["last_success_at"]);
    }
    const changeIsPositive =
      !quote.change_from_previous_day.startsWith("-") &&
      !isZeroDecimal(quote.change_from_previous_day);
    const percentIsPositive =
      !quote.change_percent_from_previous_day.startsWith("-") &&
      !isZeroDecimal(quote.change_percent_from_previous_day);
    const changeIsNegative =
      quote.change_from_previous_day.startsWith("-") &&
      !isZeroDecimal(quote.change_from_previous_day);
    const percentIsNegative =
      quote.change_percent_from_previous_day.startsWith("-") &&
      !isZeroDecimal(quote.change_percent_from_previous_day);
    const changeIsFlat = isZeroDecimal(quote.change_from_previous_day);
    const percentIsFlat = isZeroDecimal(quote.change_percent_from_previous_day);
    const signMatches =
      quote.direction === "UP" || quote.direction === "LIMIT_UP"
        ? changeIsPositive && percentIsPositive
        : quote.direction === "DOWN" || quote.direction === "LIMIT_DOWN"
          ? changeIsNegative && percentIsNegative
          : changeIsFlat && percentIsFlat;
    if (!signMatches)
      addIssue(context, "direction does not match both signed change values", ["direction"]);
  });

export type IntradayQuotePayload = z.infer<typeof intradayQuotePayloadSchema>;

export const intradayQuoteResponseSchema = z
  .object({
    schema_version: z.literal(INTRADAY_QUOTE_SCHEMA_VERSION),
    membership_id: uuidSchema,
    instrument_id: instrumentIdSchema,
    venue: z.literal("KRX"),
    currency: z.literal("KRW"),
    generation: positiveSafeIntegerSchema,
    session: intradayQuoteSessionSchema.nullable(),
    market_state: intradayQuoteMarketStateSchema,
    freshness: intradayQuoteFreshnessSchema,
    reason_code: intradayQuoteReasonCodeSchema.nullable(),
    quote: intradayQuotePayloadSchema.nullable(),
    next_poll_after_ms: z.literal(INTRADAY_QUOTE_POLL_INTERVAL_MS),
  })
  .strict()
  .superRefine((response, context) => {
    if (response.session === null && response.market_state !== "UNKNOWN") {
      addIssue(context, "known market state requires session evidence", ["session"]);
    }
    if (response.market_state === "UNKNOWN" && response.quote !== null) {
      addIssue(context, "UNKNOWN market state cannot carry a quote", ["quote"]);
    }
    if (response.quote !== null && response.session === null) {
      addIssue(context, "quote requires session evidence", ["quote"]);
    }
    if (response.quote !== null && response.freshness === "UNAVAILABLE") {
      addIssue(context, "UNAVAILABLE freshness cannot carry a quote", ["quote"]);
    }
  });

export type IntradayQuoteResponse = z.infer<typeof intradayQuoteResponseSchema>;

export class IntradayQuoteContractError extends Error {
  override readonly name = "IntradayQuoteContractError";
  readonly code = "INTRADAY_QUOTE_CONTRACT_INVALID" as const;

  constructor() {
    super("Intraday quote response did not match the approved contract");
  }
}

function kstDateAt(nowMs: number): string {
  const parts = new Intl.DateTimeFormat("en-CA", {
    day: "2-digit",
    month: "2-digit",
    timeZone: "Asia/Seoul",
    year: "numeric",
  }).formatToParts(new Date(nowMs));
  const values = new Map(parts.map((part) => [part.type, part.value]));
  return `${values.get("year") ?? ""}-${values.get("month") ?? ""}-${values.get("day") ?? ""}`;
}

export type IntradayQuoteValidationOptions = {
  readonly nowMs?: number;
};

/**
 * Parse the strict application response and, when a clock is supplied, apply the
 * point-in-time rules that cannot be represented by a static Zod schema.
 */
export function parseIntradayQuoteResponse(
  input: unknown,
  options: IntradayQuoteValidationOptions = {},
): IntradayQuoteResponse {
  const parsed = intradayQuoteResponseSchema.safeParse(input);
  if (!parsed.success) throw new IntradayQuoteContractError();
  const { quote, session } = parsed.data;
  if (quote === null || session === null) return parsed.data;
  const receivedMs = Date.parse(quote.received_at);
  const lastSuccessMs = Date.parse(quote.last_success_at);
  if (
    !Number.isFinite(receivedMs) ||
    !Number.isFinite(lastSuccessMs) ||
    kstDateAt(receivedMs) !== session.date ||
    kstDateAt(lastSuccessMs) !== session.date
  ) {
    throw new IntradayQuoteContractError();
  }
  if (options.nowMs === undefined) return parsed.data;
  const nowMs = options.nowMs;
  if (
    !Number.isFinite(nowMs) ||
    receivedMs > nowMs ||
    lastSuccessMs > nowMs ||
    nowMs - lastSuccessMs > INTRADAY_QUOTE_CACHE_MAX_AGE_MS ||
    kstDateAt(nowMs) !== session.date
  ) {
    throw new IntradayQuoteContractError();
  }
  return parsed.data;
}

export function intradayQuotePath(identity: IntradayQuoteIdentity): string {
  const parsed = intradayQuoteIdentitySchema.parse(identity);
  return `${INTRADAY_QUOTE_CACHE_PATH}/${encodeURIComponent(parsed.instrument_id)}/quote?membership_id=${encodeURIComponent(parsed.membership_id)}&generation=${encodeURIComponent(String(parsed.generation))}`;
}

export function intradayQuoteDemandPath(demandId: string): string {
  const parsed = uuidSchema.safeParse(demandId);
  if (!parsed.success) throw new IntradayQuoteContractError();
  return `${INTRADAY_QUOTE_DEMAND_PATH}/${encodeURIComponent(parsed.data)}`;
}

export function isIntradayQuoteIdentityMatch(
  value: Pick<IntradayQuoteResponse, "membership_id" | "instrument_id" | "generation">,
  identity: IntradayQuoteIdentity,
): boolean {
  return (
    value.membership_id === identity.membership_id &&
    value.instrument_id === identity.instrument_id &&
    value.generation === identity.generation
  );
}

export function intradayQuoteSessionKey(response: IntradayQuoteResponse): string | null {
  if (response.session === null) return null;
  return [
    response.session.date,
    response.session.calendar_content_sha256,
    response.session.window_contract_sha256,
    response.session.calendar_source,
    response.session.calendar_source_version,
  ].join("\u0000");
}

export function intradayQuoteVersion(value: IntradayQuotePayload): bigint {
  return BigInt(value.quote_version);
}

export function isCanonicalIntradayDecimal(value: string): boolean {
  return isCanonicalDecimal(value);
}
