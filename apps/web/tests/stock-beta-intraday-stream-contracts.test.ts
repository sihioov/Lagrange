import { describe, expect, it } from "vitest";
import {
  ageIntradayStreamRow,
  IntradayStreamContractError,
  parseIntradayStreamEvent,
  parseIntradayStreamRow,
} from "@/lib/products/intraday-stream-contracts";

const SERVER_TIME = "2026-10-03T03:00:00.500Z";

function liveRow() {
  return {
    membership_id: "00000000-0000-4000-8000-000000000001",
    instrument_id: "005930.KRX",
    generation: 7,
    row_generation: "00000000-0000-4000-8000-000000000002",
    venue: "KRX",
    currency: "KRW",
    source: "KIS_MARKET_WS",
    wire_version: "kis-h0stcnt0-20260914-v1",
    session: {
      date: "2026-10-03",
      timezone: "Asia/Seoul",
      calendar_source: "kis",
      calendar_source_version: "kis-chk-holiday-v1:schema-1",
      calendar_content_sha256: "a".repeat(64),
      window_contract_sha256: `sha256:${"b".repeat(64)}`,
    },
    subscription: "ACKED",
    connection: "CONNECTED",
    market_state: "OPEN",
    freshness: "RECENT",
    availability: "LIVE",
    reason_code: null,
    state_version: "1",
    gap_open: false,
    session_has_gap: false,
    gap_generation: "0",
    quote: {
      price: "100001.00000000",
      base_price: null,
      base_price_reason: "NOT_PROVIDED_BY_CHANNEL",
      change_from_previous_day: "1",
      change_percent_from_previous_day: "0",
      direction: "UP",
      trade_volume: "0",
      cumulative_volume: "9223372036854775807",
      halted: false,
      business_date: "2026-10-03",
      trade_time: "12:00:00",
      provider_trade_at: "2026-10-03T12:00:00+09:00",
      received_at: "2026-10-03T03:00:00.000Z",
      committed_at: "2026-10-03T03:00:00.200Z",
      epoch: "00000000-0000-4000-8000-000000000003",
      quote_version: "9223372036854775807",
      receive_ordinal: "1",
    },
  };
}

function invalidQuote(overrides: Record<string, unknown>): unknown {
  const row = liveRow();
  return { ...row, quote: { ...row.quote, ...overrides } };
}

describe("schema-2 market stream row contract", () => {
  it("preserves exact decimals, i64 text counters, nullable base price and rounded-zero percentage", () => {
    const value = parseIntradayStreamRow(liveRow(), SERVER_TIME);
    expect(value.quote?.price).toBe("100001.00000000");
    expect(value.quote?.quote_version).toBe("9223372036854775807");
    expect(value.quote?.base_price).toBeNull();
    expect(value.quote?.change_percent_from_previous_day).toBe("0");
    expect(parseIntradayStreamRow(invalidQuote({ halted: true }), SERVER_TIME).quote?.halted).toBe(
      true,
    );
  });

  it("accepts a quiet observation for 30 seconds without relaxing the three-second commit deadline", () => {
    expect(parseIntradayStreamRow(liveRow(), "2026-10-03T03:00:05Z").availability).toBe("LIVE");
    expect(parseIntradayStreamRow(liveRow(), "2026-10-03T03:00:30Z").freshness).toBe("RECENT");
    expect(() => parseIntradayStreamRow(liveRow(), "2026-10-03T03:00:30.000000001Z")).toThrow(
      IntradayStreamContractError,
    );
    expect(() =>
      parseIntradayStreamRow(
        invalidQuote({ committed_at: "2026-10-03T03:00:03.000000001Z" }),
        "2026-10-03T03:00:04Z",
      ),
    ).toThrow(IntradayStreamContractError);
  });

  it("ages the event independently of the receipt and preserves known stale/closed values", () => {
    const quote = {
      ...liveRow().quote,
      trade_time: "11:59:31",
      provider_trade_at: "2026-10-03T11:59:31+09:00",
    };
    expect(() => parseIntradayStreamRow({ ...liveRow(), quote }, "2026-10-03T03:00:02Z")).toThrow(
      IntradayStreamContractError,
    );
    const stale = {
      ...liveRow(),
      quote,
      availability: "LAST_KNOWN",
      freshness: "STALE",
      reason_code: "QUOTE_STALE",
    };
    expect(parseIntradayStreamRow(stale, "2026-10-03T03:00:02Z").quote?.provider_trade_at).toBe(
      quote.provider_trade_at,
    );
    expect(
      parseIntradayStreamRow(
        { ...stale, market_state: "CLOSED", reason_code: "SESSION_CLOSED" },
        "2026-10-03T03:00:02Z",
      ).market_state,
    ).toBe("CLOSED");
    expect(() => parseIntradayStreamRow(stale, "2026-10-03T15:00:00Z")).toThrow(
      IntradayStreamContractError,
    );
  });

  it("accepts equivalent provider UTC time while binding the original KST date and second", () => {
    expect(
      parseIntradayStreamRow(
        invalidQuote({ provider_trade_at: "2026-10-03T03:00:00Z" }),
        SERVER_TIME,
      ).quote?.trade_time,
    ).toBe("12:00:00");
    for (const overrides of [
      { provider_trade_at: "2026-10-03T12:00:01+09:00" },
      { business_date: "2026-10-02" },
      { trade_time: "24:00:00" },
      { provider_trade_at: "2026-02-30T12:00:00+09:00" },
      { received_at: "2026-10-03T03:00:00+00:00" },
      { committed_at: "2026-10-03T02:59:59Z" },
    ])
      expect(() => parseIntradayStreamRow(invalidQuote(overrides), SERVER_TIME)).toThrow(
        IntradayStreamContractError,
      );
  });

  it("rejects malformed values and sign contradictions instead of manufacturing a price", () => {
    for (const price of [
      "0",
      "-1",
      "+1",
      "01",
      "1e3",
      "NaN",
      " 1",
      "1.123456789",
      "1234567890123",
      1,
    ]) {
      expect(() => parseIntradayStreamRow(invalidQuote({ price }), SERVER_TIME)).toThrow(
        IntradayStreamContractError,
      );
    }
    for (const overrides of [
      { base_price: "100000" },
      { base_price_reason: "DERIVED" },
      { halted: null },
      { halted: undefined },
      { direction: "UP", change_from_previous_day: "-1" },
      { direction: "DOWN", change_percent_from_previous_day: "1" },
      { direction: "FLAT", change_from_previous_day: "1" },
      { change_percent_from_previous_day: "-0.000" },
      { trade_volume: "-1" },
      { cumulative_volume: "9223372036854775808" },
      { quote_version: "01" },
      { quote_version: "invalid" },
      { receive_ordinal: "0" },
      { quote_version: 1 },
    ])
      expect(() => parseIntradayStreamRow(invalidQuote(overrides), SERVER_TIME)).toThrow(
        IntradayStreamContractError,
      );
    expect(
      parseIntradayStreamRow(
        invalidQuote({
          direction: "DOWN",
          change_from_previous_day: "-1",
          change_percent_from_previous_day: "0",
        }),
        SERVER_TIME,
      ).quote?.direction,
    ).toBe("DOWN");
  });

  it("rejects extra fields at every level, legacy REST data and noncanonical identities", () => {
    for (const input of [
      { ...liveRow(), credential_slot_id: "private" },
      { ...liveRow(), owner_user_id: "private" },
      { ...liveRow(), session_hash: "private" },
      { ...liveRow(), next_poll_after_ms: 5000 },
      { ...liveRow(), source: "KIS_REST" },
      { ...liveRow(), wire_version: "unknown" },
      { ...liveRow(), instrument_id: "005930.NXT" },
      { ...liveRow(), generation: Number.MAX_SAFE_INTEGER + 1 },
      { ...liveRow(), membership_id: "00000000-0000-0000-0000-000000000000" },
      { ...liveRow(), membership_id: "aaaaaaaa-AAAA-4000-8000-000000000001" },
      { ...liveRow(), session: { ...liveRow().session, rights_bytes: "private" } },
      invalidQuote({ fencing_token: "1" }),
      { ...liveRow(), reason_code: "free form broker text" },
    ])
      expect(() => parseIntradayStreamRow(input, SERVER_TIME)).toThrow(IntradayStreamContractError);
  });

  it("requires actual quote evidence and all current LIVE conditions", () => {
    for (const reason_code of [
      "FEATURE_DISABLED",
      "CALENDAR_UNAVAILABLE",
      "SESSION_WINDOW_UNAVAILABLE",
      "ACCESS_REVOKED",
    ]) {
      expect(() =>
        parseIntradayStreamRow(
          { ...liveRow(), availability: "LAST_KNOWN", reason_code },
          SERVER_TIME,
        ),
      ).toThrow(IntradayStreamContractError);
    }
    for (const overrides of [
      { quote: null },
      { session: null },
      { connection: "BACKOFF" },
      { market_state: "CLOSED" },
      { subscription: "PENDING" },
      { freshness: "STALE" },
      { reason_code: "PIPELINE_LAG" },
      { gap_open: true, session_has_gap: true },
      { availability: "UNAVAILABLE" },
      { availability: "AWAITING_FIRST_TRADE" },
    ])
      expect(() => parseIntradayStreamRow({ ...liveRow(), ...overrides }, SERVER_TIME)).toThrow(
        IntradayStreamContractError,
      );
    const waiting = {
      ...liveRow(),
      quote: null,
      availability: "AWAITING_FIRST_TRADE",
      freshness: "UNAVAILABLE",
      reason_code: "AWAITING_FIRST_TRADE",
    };
    expect(parseIntradayStreamRow(waiting, SERVER_TIME).quote).toBeNull();
    expect(
      parseIntradayStreamRow(
        {
          ...waiting,
          session: null,
          market_state: "UNKNOWN",
          availability: "UNAVAILABLE",
          reason_code: "SESSION_WINDOW_UNAVAILABLE",
        },
        SERVER_TIME,
      ).session,
    ).toBeNull();
  });

  it("retains session gap history after a row recovers and never relabels a gap as LIVE", () => {
    expect(
      parseIntradayStreamRow(
        { ...liveRow(), session_has_gap: true, gap_generation: "1" },
        SERVER_TIME,
      ).availability,
    ).toBe("LIVE");
    const gap = {
      ...liveRow(),
      availability: "LAST_KNOWN",
      connection: "BACKOFF",
      reason_code: "RECONNECT_GAP",
      gap_open: true,
      session_has_gap: true,
      gap_generation: "1",
    };
    expect(parseIntradayStreamRow(gap, SERVER_TIME).quote?.epoch).toBe(liveRow().quote.epoch);
    expect(() => parseIntradayStreamRow({ ...gap, session_has_gap: false }, SERVER_TIME)).toThrow(
      IntradayStreamContractError,
    );
  });

  it("rejects future commits, future LIVE event times and malformed server time without echoing data", () => {
    for (const server of ["bad private payload", "2026-02-30T03:00:00Z", "2026-10-03T03:00:00Z"]) {
      expect(() => parseIntradayStreamRow(liveRow(), server)).toThrow(IntradayStreamContractError);
    }
    const future = invalidQuote({
      trade_time: "12:00:01",
      provider_trade_at: "2026-10-03T12:00:01+09:00",
    });
    expect(() => parseIntradayStreamRow(future, SERVER_TIME)).toThrow(IntradayStreamContractError);
    try {
      parseIntradayStreamRow({ ...liveRow(), private_payload: "do-not-print" }, SERVER_TIME);
      expect.unreachable();
    } catch (error) {
      expect(error).toBeInstanceOf(IntradayStreamContractError);
      expect(String(error)).not.toContain("do-not-print");
    }
  });
});

describe("schema-2 SSE envelopes", () => {
  const stream = "00000000-0000-4000-8000-000000000010";
  const lease = "00000000-0000-4000-8000-000000000011";
  const header = {
    schema_version: 2,
    stream_id: stream,
    event_sequence: "1",
    server_time: SERVER_TIME,
  };
  const snapshot = {
    ...header,
    body: {
      lease_id: lease,
      lease_expires_at: "2026-10-03T03:00:30.500Z",
      rows: [liveRow()],
    },
  };

  it("decodes the four exact event types and binds the transport ID to the cursor", () => {
    expect(parseIntradayStreamEvent("snapshot", JSON.stringify(snapshot), `${stream}:1`).kind).toBe(
      "snapshot",
    );
    expect(
      parseIntradayStreamEvent(
        "delta",
        JSON.stringify({ ...header, body: { rows: [liveRow()] } }),
        `${stream}:1`,
      ).kind,
    ).toBe("delta");
    expect(
      parseIntradayStreamEvent(
        "status",
        JSON.stringify({
          ...header,
          body: {
            connection: "CONNECTED",
            reason_code: null,
            gap_open: false,
            session_has_gap: false,
            gap_generation: "0",
          },
        }),
        `${stream}:1`,
      ).kind,
    ).toBe("status");
    expect(
      parseIntradayStreamEvent(
        "reset",
        JSON.stringify({ ...header, body: { reason_code: "RESYNC_REQUIRED" } }),
        `${stream}:1`,
      ).kind,
    ).toBe("reset");
    for (const id of ["", `${stream}:0`, `${lease}:1`, `${stream}:01`])
      expect(() => parseIntradayStreamEvent("snapshot", JSON.stringify(snapshot), id)).toThrow(
        IntradayStreamContractError,
      );
  });

  it("rejects expired leases, oversized or malformed JSON, unknown fields and mixed schemas", () => {
    for (const input of [
      { ...snapshot, schema_version: 1 },
      { ...snapshot, stream_id: `${lease.toUpperCase()}x` },
      { ...snapshot, event_sequence: "0" },
      { ...snapshot, event_sequence: 1 },
      { ...snapshot, event_sequence: "9223372036854775808" },
      { ...snapshot, owner: "private" },
      { ...snapshot, body: { ...snapshot.body, lease_expires_at: SERVER_TIME } },
      { ...snapshot, body: { ...snapshot.body, next_poll_after_ms: 5000 } },
      { ...snapshot, body: { ...snapshot.body, rows: [] } },
      { ...snapshot, body: { ...snapshot.body, rows: Array.from({ length: 31 }, liveRow) } },
    ])
      expect(() =>
        parseIntradayStreamEvent("snapshot", JSON.stringify(input), `${stream}:1`),
      ).toThrow(IntradayStreamContractError);
    for (const input of ["{private", " ".repeat(256 * 1024 + 1), "가".repeat(100_000)])
      expect(() => parseIntradayStreamEvent("snapshot", input, `${stream}:1`)).toThrow(
        IntradayStreamContractError,
      );
    expect(() =>
      parseIntradayStreamEvent("quote", JSON.stringify(snapshot), `${stream}:1`),
    ).toThrow(IntradayStreamContractError);
  });

  it("requires unique sorted identities and validates every row against event time", () => {
    const second = {
      ...liveRow(),
      membership_id: "00000000-0000-4000-8000-000000000004",
      instrument_id: "000660.KRX",
    };
    const decode = (rows: unknown[], server_time = SERVER_TIME) =>
      parseIntradayStreamEvent(
        "delta",
        JSON.stringify({ ...header, server_time, body: { rows } }),
        `${stream}:1`,
      );
    expect(decode([liveRow(), second]).kind).toBe("delta");
    for (const rows of [
      [second, liveRow()],
      [liveRow(), liveRow()],
      [liveRow(), { ...second, instrument_id: liveRow().instrument_id }],
      [liveRow(), { ...second, generation: 0 }],
    ])
      expect(() => decode(rows)).toThrow(IntradayStreamContractError);
    expect(() => decode([liveRow()], "2026-10-03T03:00:31Z")).toThrow(IntradayStreamContractError);
  });

  it("ages display values from server time and monotonic elapsed time without rewriting receipt proof", () => {
    const original = parseIntradayStreamRow(liveRow(), SERVER_TIME);
    const recent = ageIntradayStreamRow(original, SERVER_TIME, 29_500);
    const stale = ageIntradayStreamRow(original, SERVER_TIME, 29_500.001);
    expect(recent.availability).toBe("LIVE");
    expect(stale.availability).toBe("LAST_KNOWN");
    expect(stale.freshness).toBe("STALE");
    expect(stale.quote).toEqual(original.quote);
    expect(stale.state_version).toBe(original.state_version);
    expect(original.availability).toBe("LIVE");
    const tomorrow = ageIntradayStreamRow(original, "2026-10-03T15:00:00Z");
    expect(tomorrow.quote).toBeNull();
    expect(tomorrow.session).toBeNull();
    expect(tomorrow.market_state).toBe("UNKNOWN");
    for (const elapsed of [-1, Number.NaN, Number.POSITIVE_INFINITY])
      expect(() => ageIntradayStreamRow(original, SERVER_TIME, elapsed)).toThrow(
        IntradayStreamContractError,
      );
  });
});
