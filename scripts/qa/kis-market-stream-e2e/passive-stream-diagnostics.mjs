// Inert, bounded metadata reduction for the owned synthetic browser harness.
// No raw SSE, prices, credentials, UUIDs, headers, or free-form messages survive.
const REASONS = new Set([
  "FEATURE_DISABLED", "NO_ACTIVE_DEMAND", "CALENDAR_UNAVAILABLE", "SESSION_WINDOW_UNAVAILABLE",
  "SESSION_CLOSED", "AWAITING_FIRST_TRADE", "QUOTE_STALE", "CONNECTION_LOST", "RECONNECT_GAP",
  "SUBSCRIPTION_PENDING", "SUBSCRIPTION_REJECTED", "SUBSCRIPTION_AMBIGUOUS", "APPROVAL_UNAVAILABLE",
  "BUDGET_EXHAUSTED", "PRODUCER_UNAVAILABLE", "PIPELINE_LAG", "WIRE_SCHEMA_MISMATCH",
  "PROVIDER_RESPONSE_INVALID", "QUOTE_VALUE_INVALID", "MARKET_CLASS_UNSUPPORTED", "LOCAL_INGRESS_LIMIT",
  "RESYNC_REQUIRED", "ACCESS_REVOKED",
]);
const AVAILABILITY = new Set(["LIVE", "LAST_KNOWN", "AWAITING_FIRST_TRADE", "UNAVAILABLE"]);
const CONNECTION = new Set(["DISCONNECTED", "CONNECTING", "CONNECTED", "BACKOFF", "STOPPED"]);
const FRESHNESS = new Set(["RECENT", "STALE", "UNAVAILABLE"]);
const KINDS = new Set(["snapshot", "delta", "status", "reset"]);
const MAX_EVENTS = 30_000;
const MAX_BYTES = 65_536;
const finite = (n) => Number.isSafeInteger(n) && n >= 0 ? n : null;
const code = (s, set) => set.has(s) ? s : "UNKNOWN";
const reason = (s) => s === null ? null : code(s, REASONS);
const counter = (s) => typeof s === "string" && /^[1-9][0-9]{0,18}$/.test(s) && BigInt(s) <= 9_223_372_036_854_775_807n ? s : null;
const instrument = (s) => typeof s === "string" && /^[0-9]{6}\.KRX$/.test(s) ? s : null;
const timestamp = (s) => typeof s === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|\+09:00)$/.test(s) ? s : null;
const flag = (v) => typeof v === "boolean" ? v : null;

function rowMetadata(row) {
  return {
    instrument: instrument(row?.instrument_id),
    availability: code(row?.availability, AVAILABILITY),
    connection: code(row?.connection, CONNECTION),
    reason_code: reason(row?.reason_code),
    freshness: code(row?.freshness, FRESHNESS),
    gap_open: flag(row?.gap_open),
    quote_present: row?.quote !== null && typeof row?.quote === "object",
    version: counter(row?.quote?.quote_version),
    ordinal: counter(row?.quote?.receive_ordinal),
    provider_trade_at: timestamp(row?.quote?.provider_trade_at),
    received_at: timestamp(row?.quote?.received_at),
    committed_at: timestamp(row?.quote?.committed_at),
  };
}

export function domMetadata(atMs, rows) {
  return {
    at_ms: finite(atMs),
    row_count: Array.isArray(rows) ? rows.length : null,
    rows: Array.isArray(rows) ? rows.slice(0, 30).map((row) => ({
      instrument: instrument(row?.instrument),
      availability: code(row?.availability, AVAILABILITY),
      price_present: typeof row?.price === "string" && row.price.length > 0,
      version: counter(row?.version), ordinal: counter(row?.ordinal),
      received_at: timestamp(row?.received_at),
    })) : [],
  };
}

export class PassiveStreamDiagnostics {
  #events = 0;
  #invalid = 0;
  #limit = false;
  #byKind = { snapshot: 0, delta: 0, status: 0, reset: 0 };
  #recent = [];
  #lastNonlive = null;

  observe(kind, data, atMs, cdpMs) {
    // Diagnostic failures never alter application delivery or throw into CDP.
    if (this.#events >= MAX_EVENTS) { this.#limit = true; return; }
    this.#events++;
    try {
      if (!KINDS.has(kind) || finite(atMs) === null || finite(cdpMs) === null ||
          typeof data !== "string" || data.length > MAX_BYTES || Buffer.byteLength(data, "utf8") > MAX_BYTES) throw 0;
      const input = JSON.parse(data);
      if (input?.schema_version !== 2 || !input.body || typeof input.body !== "object") throw 0;
      const output = { at_ms: atMs, cdp_ms: cdpMs, kind,
        sequence: counter(input.event_sequence), server_time: timestamp(input.server_time) };
      let nonlive = false;
      if (kind === "snapshot" || kind === "delta") {
        if (!Array.isArray(input.body.rows) || input.body.rows.length > 30) throw 0;
        output.rows = input.body.rows.map(rowMetadata);
        nonlive = output.rows.some((row) => row.availability !== "LIVE");
      } else {
        output.reason_code = reason(input.body.reason_code);
        if (kind === "status") {
          output.connection = code(input.body.connection, CONNECTION);
          output.gap_open = flag(input.body.gap_open);
          nonlive = output.connection !== "CONNECTED" || output.reason_code !== null || output.gap_open !== false;
        } else nonlive = true;
      }
      this.#byKind[kind]++;
      if (nonlive) this.#lastNonlive = output;
      this.#recent.push(output);
      if (this.#recent.length > 16) this.#recent.shift();
    } catch { this.#invalid++; }
  }

  snapshot() {
    return structuredClone({ schema_version: 1, events: this.#events, invalid_events: this.#invalid,
      event_limit_reached: this.#limit, by_kind: this.#byKind, recent: this.#recent,
      last_nonlive: this.#lastNonlive });
  }
}
