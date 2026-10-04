import { ownerEquityV2MembershipSchema } from "@/lib/products/equity-signals-contracts";
import { intradayStreamRowSchema } from "@/lib/products/intraday-stream-contracts";
import { MARKET_STREAM_INSTRUMENTS } from "@/lib/products/intraday-stream-universe";

// Synthetic observations only: no provider output, credentials or actual market-data evidence.
export const STREAM_FIXTURE_TIME = "2026-10-03T03:00:00.500Z";
export const streamFixtureUuid = (n: number) =>
  `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

export function streamMembership(index: number) {
  const instrument = MARKET_STREAM_INSTRUMENTS[index];
  if (!instrument) throw new Error("Fixture index outside fixed universe");
  return ownerEquityV2MembershipSchema.parse({
    id: streamFixtureUuid(index + 1),
    instrument_id: instrument.id,
    lifecycle: "READY",
    generation: 7,
    coverage: {
      first_session: "2025-01-01",
      last_session: "2026-10-02",
      minimum_observed_sessions: 121,
      observed_sessions: 261,
      target_observed_sessions: 261,
    },
    requested_at: "2026-10-01T03:00:00Z",
    updated_at: "2026-10-02T06:00:00Z",
  });
}

export function streamRow(index = 0, time = STREAM_FIXTURE_TIME) {
  const member = streamMembership(index);
  const server = Date.parse(time);
  const tradeUtc = Math.floor((server - 400) / 1000) * 1000;
  const tradeKst = new Date(tradeUtc + 9 * 60 * 60 * 1000).toISOString();
  return intradayStreamRowSchema.parse({
    membership_id: member.id,
    instrument_id: member.instrument_id,
    generation: member.generation,
    row_generation: streamFixtureUuid(500 + index),
    venue: "KRX",
    currency: "KRW",
    source: "KIS_MARKET_WS",
    wire_version: "kis-h0stcnt0-20260914-v1",
    session: {
      date: tradeKst.slice(0, 10),
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
      price: "100123456789.12345678",
      base_price: null,
      base_price_reason: "NOT_PROVIDED_BY_CHANNEL",
      change_from_previous_day: "-123456789.12345678",
      change_percent_from_previous_day: "-1.25",
      direction: "DOWN",
      trade_volume: "100",
      cumulative_volume: "1234567",
      halted: false,
      business_date: tradeKst.slice(0, 10),
      trade_time: tradeKst.slice(11, 19),
      provider_trade_at: `${tradeKst.slice(0, 19)}+09:00`,
      received_at: new Date(server - 400).toISOString(),
      committed_at: new Date(server - 100).toISOString(),
      epoch: streamFixtureUuid(999),
      quote_version: "1",
      receive_ordinal: "1",
    },
  });
}
