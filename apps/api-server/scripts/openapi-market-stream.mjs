// Schema-2 delivery is separate from the schema-1 REST quote contract.
export const STREAM_PREFIX = "/api/v1/research/owner-beta/equity-universe-v2";
export const STREAM_ROUTES = [
  ["POST", `${STREAM_PREFIX}/stream-leases`, { owner: true, ownerMarketStream: true, mutating: true, idem: true }],
  ["DELETE", `${STREAM_PREFIX}/stream-leases/{lease_id}`, { owner: true, ownerMarketStream: true, mutating: true, idem: true }],
  ["GET", `${STREAM_PREFIX}/market-stream`, { owner: true, ownerMarketStream: true }],
];
export const STREAM_ERRORS = [
  ["FEATURE_DISABLED", 503],
  ["MARKET_STREAM_UNAVAILABLE", 503],
  ["STREAM_LEASE_SEQUENCE_CONFLICT", 409],
  ["STREAM_LEASE_CAPACITY", 409],
  ["STREAM_CONSUMER_CAPACITY", 409],
];

const ref = (name) => ({ $ref: `#/components/schemas/OwnerMarketStream${name}` });
const nullable = (schema) => ({ anyOf: [schema, { type: "null" }] });
const object = (properties, description) => ({
  type: "object", additionalProperties: false,
  required: Object.keys(properties), properties,
  ...(description ? { description } : {}),
});
const uuid = {
  type: "string", format: "uuid",
  pattern: "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
  not: { const: "00000000-0000-0000-0000-000000000000" },
};
const generation = { type: "integer", minimum: 1, maximum: Number.MAX_SAFE_INTEGER };
const sequence = { type: "integer", minimum: 0, maximum: Number.MAX_SAFE_INTEGER };
const version = { type: "integer", const: 2 };
const timestamp = { type: "string", format: "date-time" };
const date = { type: "string", format: "date", pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}$" };
const decimal = { type: "string", pattern: "^-?(0|[1-9][0-9]*)(\\.[0-9]{1,8})?$" };
const rows = { type: "array", minItems: 1, maxItems: 30, items: ref("Row"),
  description: "Ascending canonical membership UUID order; memberships and instruments must each be unique. Delta rows completely replace their previous row." };
const identities = { type: "array", minItems: 1, maxItems: 30, uniqueItems: true, items: ref("Identity"),
  description: "Ascending canonical membership UUID order; no repeated membership or instrument." };

// Canonical decimal strings with an exact i64 ceiling, without unsafe JS numbers.
function counterPattern(allowZero) {
  const maximum = "9223372036854775807";
  const alternatives = allowZero ? ["0"] : [];
  alternatives.push("[1-9][0-9]{0,17}");
  for (let i = 0; i < maximum.length; i += 1) {
    const low = i === 0 ? 1 : 0;
    const high = Number(maximum[i]) - 1;
    if (high < low) continue;
    const digit = low === high ? String(low) : `[${low}-${high}]`;
    const rest = maximum.length - i - 1;
    alternatives.push(maximum.slice(0, i) + digit + (rest ? `[0-9]{${rest}}` : ""));
  }
  alternatives.push(maximum);
  return `^(${alternatives.join("|")})$`;
}
const envelope = (body) => object({
  schema_version: version, stream_id: uuid, event_sequence: ref("PositiveCounter"),
  server_time: timestamp, body,
}, "The SSE event field supplies the event kind; the SSE id is stream_id:event_sequence. This cursor is not a broker sequence or replay authorization.");

export const STREAM_SCHEMAS = {
  OwnerMarketStreamCounter: { type: "string", maxLength: 19, pattern: counterPattern(true), description: "Canonical decimal integer in 0..9223372036854775807." },
  OwnerMarketStreamPositiveCounter: { type: "string", maxLength: 19, pattern: counterPattern(false), description: "Canonical decimal integer in 1..9223372036854775807." },
  OwnerMarketStreamIdentity: object({ membership_id: uuid, instrument_id: { type: "string", pattern: "^[0-9]{6}\\.KRX$" }, generation }),
  OwnerMarketStreamLeaseBody: object({ schema_version: version, consumer_id: uuid, renewal_sequence: sequence, identities },
    "Maximum JSON body 16384 bytes. Initial sequence is 0; each replacement or renewal increments the prior sequence by one. Entire set is replaced atomically."),
  OwnerMarketStreamReleaseBody: object({ schema_version: version, consumer_id: uuid, renewal_sequence: sequence },
    "Release carries the last accepted renewal sequence. A released consumer cannot be resurrected; use a fresh consumer UUID."),
  OwnerMarketStreamLease: object({ schema_version: version, lease_id: uuid, consumer_id: uuid, renewal_sequence: sequence,
    lease_expires_at: timestamp, renew_after_ms: { type: "integer", const: 15000 }, identities }),
  OwnerMarketStreamRelease: object({ schema_version: version, lease_id: uuid, released: { type: "boolean", const: true } }),
  OwnerMarketStreamConnection: { type: "string", enum: ["DISCONNECTED", "CONNECTING", "CONNECTED", "BACKOFF", "STOPPED"] },
  OwnerMarketStreamReasonCode: { type: "string", enum: [
    "FEATURE_DISABLED", "NO_ACTIVE_DEMAND", "CALENDAR_UNAVAILABLE", "SESSION_WINDOW_UNAVAILABLE", "SESSION_CLOSED",
    "AWAITING_FIRST_TRADE", "QUOTE_STALE", "CONNECTION_LOST", "RECONNECT_GAP", "SUBSCRIPTION_PENDING",
    "SUBSCRIPTION_REJECTED", "SUBSCRIPTION_AMBIGUOUS", "APPROVAL_UNAVAILABLE", "BUDGET_EXHAUSTED", "PRODUCER_UNAVAILABLE",
    "PIPELINE_LAG", "WIRE_SCHEMA_MISMATCH", "PROVIDER_RESPONSE_INVALID", "QUOTE_VALUE_INVALID", "MARKET_CLASS_UNSUPPORTED",
    "LOCAL_INGRESS_LIMIT", "RESYNC_REQUIRED", "ACCESS_REVOKED",
  ] },
  OwnerMarketStreamSession: object({ date, timezone: { const: "Asia/Seoul", type: "string" },
    calendar_source: { const: "kis", type: "string" }, calendar_source_version: { const: "kis-chk-holiday-v1:schema-1", type: "string" },
    calendar_content_sha256: { type: "string", pattern: "^[0-9a-f]{64}$" }, window_contract_sha256: { type: "string", pattern: "^sha256:[0-9a-f]{64}$" } }),
  OwnerMarketStreamQuote: object({
    price: { type: "string", pattern: "^(0|[1-9][0-9]*)(\\.[0-9]{1,8})?$" },
    base_price: { type: "null" }, base_price_reason: { type: "string", const: "NOT_PROVIDED_BY_CHANNEL" },
    change_from_previous_day: decimal, change_percent_from_previous_day: decimal,
    direction: { type: "string", enum: ["UP", "DOWN", "FLAT", "LIMIT_UP", "LIMIT_DOWN"] },
    trade_volume: { type: "string", pattern: "^(0|[1-9][0-9]*)$" }, cumulative_volume: { type: "string", pattern: "^(0|[1-9][0-9]*)$" },
    halted: { type: "boolean", description: "Halt observation as of this quote; does not assert current tradability." },
    business_date: date, trade_time: { type: "string", pattern: "^([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]$" },
    provider_trade_at: timestamp, received_at: timestamp, committed_at: timestamp, epoch: uuid,
    quote_version: ref("PositiveCounter"), receive_ordinal: ref("PositiveCounter"),
  }, "A same-day authorized capture retains its original timestamps, epoch and versions when shown as LAST_KNOWN."),
  OwnerMarketStreamRow: object({
    membership_id: uuid, instrument_id: { type: "string", pattern: "^[0-9]{6}\\.KRX$" }, generation, row_generation: uuid,
    venue: { type: "string", const: "KRX" }, currency: { type: "string", const: "KRW" },
    source: { type: "string", const: "KIS_MARKET_WS" }, wire_version: { type: "string", const: "kis-h0stcnt0-20260914-v1" },
    session: nullable(ref("Session")), subscription: { type: "string", enum: ["DESIRED", "PENDING", "ACKED", "REJECTED", "ABSENT"] },
    connection: ref("Connection"), market_state: { type: "string", enum: ["OPEN", "CLOSED", "UNKNOWN"] },
    freshness: { type: "string", enum: ["RECENT", "STALE", "UNAVAILABLE"] },
    availability: { type: "string", enum: ["LIVE", "LAST_KNOWN", "AWAITING_FIRST_TRADE", "UNAVAILABLE"] },
    reason_code: nullable(ref("ReasonCode")), state_version: ref("Counter"), gap_open: { type: "boolean" },
    session_has_gap: { type: "boolean" }, gap_generation: ref("Counter"), quote: nullable(ref("Quote")),
  }),
  OwnerMarketStreamSnapshotBody: object({ lease_id: uuid, lease_expires_at: timestamp, rows }),
  OwnerMarketStreamDeltaBody: object({ rows }),
  OwnerMarketStreamStatusBody: object({ connection: ref("Connection"), reason_code: nullable(ref("ReasonCode")),
    gap_open: { type: "boolean" }, session_has_gap: { type: "boolean" }, gap_generation: ref("Counter") }),
  OwnerMarketStreamResetBody: object({ reason_code: { type: "string", const: "RESYNC_REQUIRED" } }),
  OwnerMarketStreamSnapshotEvent: envelope(ref("SnapshotBody")),
  OwnerMarketStreamDeltaEvent: envelope(ref("DeltaBody")),
  OwnerMarketStreamStatusEvent: envelope(ref("StatusBody")),
  OwnerMarketStreamResetEvent: envelope(ref("ResetBody")),
};
