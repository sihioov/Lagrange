import { describe, expect, it } from "vitest";
import { IntradayStreamState } from "@/components/stock-beta/quote/intraday-stream-state";
import {
  IntradayStreamContractError,
  type IntradayStreamIdentity,
  type IntradayStreamRow,
} from "@/lib/products/intraday-stream-contracts";

const uuid = (number: number) => `00000000-0000-4000-8000-${String(number).padStart(12, "0")}`;
const LEASE = uuid(100);
const STREAM = uuid(101);
const NEXT_STREAM = uuid(102);
const SERVER = "2026-10-03T03:00:00.500Z";

function row(index = 1): IntradayStreamRow {
  return {
    membership_id: uuid(index),
    instrument_id: `${String(index).padStart(6, "0")}.KRX`,
    generation: 1,
    row_generation: uuid(200 + index),
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
      price: "100000",
      base_price: null,
      base_price_reason: "NOT_PROVIDED_BY_CHANNEL",
      change_from_previous_day: "0",
      change_percent_from_previous_day: "0",
      direction: "FLAT",
      trade_volume: "1",
      cumulative_volume: "100",
      halted: false,
      business_date: "2026-10-03",
      trade_time: "12:00:00",
      provider_trade_at: "2026-10-03T12:00:00+09:00",
      received_at: "2026-10-03T03:00:00.100Z",
      committed_at: "2026-10-03T03:00:00.200Z",
      epoch: uuid(300),
      quote_version: "1",
      receive_ordinal: "1",
    },
  };
}

function identity(value: IntradayStreamRow): IntradayStreamIdentity {
  return {
    membership_id: value.membership_id,
    instrument_id: value.instrument_id,
    generation: value.generation,
  };
}

function snapshot(rows = [row()], lease_id = LEASE) {
  return { lease_id, lease_expires_at: "2026-10-03T03:00:30.500Z", rows };
}

function status(overrides: Record<string, unknown> = {}) {
  return {
    connection: "CONNECTED",
    reason_code: null,
    gap_open: false,
    session_has_gap: false,
    gap_generation: "0",
    ...overrides,
  };
}

function emit(
  state: IntradayStreamState,
  token: number,
  sequence: number,
  kind: string,
  body: unknown,
  serverTime = SERVER,
  observedAt = 100,
  stream = STREAM,
) {
  return state.receive(
    token,
    kind,
    JSON.stringify({
      schema_version: 2,
      stream_id: stream,
      event_sequence: String(sequence),
      server_time: serverTime,
      body,
    }),
    `${stream}:${sequence}`,
    observedAt,
  );
}

function ready(rows = [row()]) {
  const state = new IntradayStreamState(LEASE, rows.map(identity));
  const token = state.beginConnection();
  expect(emit(state, token, 1, "snapshot", snapshot(rows))).toBe("applied");
  return { state, token };
}

describe("bounded market stream delivery state", () => {
  it("requires a full initial snapshot and discards callbacks from replaced connections", () => {
    const { state, token } = ready();
    const next = state.beginConnection();
    expect(state.view(100).rows[0]?.availability).toBe("LAST_KNOWN");
    expect(emit(state, token, 99, "status", status({ reason_code: "ACCESS_REVOKED" }))).toBe(
      "ignored",
    );
    expect(
      emit(state, next, 1, "reset", { reason_code: "RESYNC_REQUIRED" }, SERVER, 100, NEXT_STREAM),
    ).toBe("applied");
    expect(state.view(100).rows).toEqual([]);
    expect(emit(state, next, 2, "snapshot", snapshot(), SERVER, 100, NEXT_STREAM)).toBe("applied");
    expect(state.view(100).rows[0]?.availability).toBe("LIVE");
    const newToken = state.beginConnection();
    expect(emit(state, newToken, 1, "snapshot", snapshot(), SERVER, 100, uuid(103))).toBe("resync");
    expect(state.view(100).rows).toEqual([]);
  });

  it("ignores old event sequences without refreshing the silence watchdog", () => {
    const { state, token } = ready();
    expect(emit(state, token, 1, "snapshot", snapshot(), SERVER, 4_000)).toBe("ignored");
    expect(state.view(5_100).phase).toBe("disconnected");
    expect(state.view(5_100).rows[0]?.availability).toBe("LAST_KNOWN");
    expect(emit(state, token, 2, "status", status(), "2026-10-03T03:00:05.500Z", 5_100)).toBe(
      "applied",
    );
    expect(state.view(5_100).phase).toBe("ready");
  });

  it("applies ordered delivery overlays at equal cache versions and preserves captured quote fields", () => {
    const { state, token } = ready();
    const original = state.view(100).rows[0];
    const gap = {
      ...row(),
      connection: "BACKOFF" as const,
      availability: "LAST_KNOWN" as const,
      reason_code: "RECONNECT_GAP" as const,
      gap_open: true,
      session_has_gap: true,
      gap_generation: "1",
    };
    expect(emit(state, token, 2, "delta", { rows: [gap] })).toBe("applied");
    expect(state.view(100).rows[0]?.quote).toEqual(original?.quote);
    expect(state.view(100).rows[0]?.state_version).toBe("1");
    expect(state.view(100).rows[0]?.availability).toBe("LAST_KNOWN");
    expect(emit(state, token, 3, "delta", { rows: [{ ...row(), session_has_gap: true }] })).toBe(
      "applied",
    );
    expect(state.view(100).rows[0]?.availability).toBe("LIVE");
    expect(state.view(100).rows[0]?.session_has_gap).toBe(true);
  });

  it("uses status server time to age both event and receipt without fabricating versions", () => {
    const { state, token } = ready();
    state.confirmLeaseExpiry(LEASE, "2026-10-03T03:01:00Z");
    expect(emit(state, token, 2, "status", status(), "2026-10-03T03:00:30Z", 200)).toBe("applied");
    expect(state.view(200).rows[0]?.freshness).toBe("RECENT");
    expect(emit(state, token, 3, "status", status(), "2026-10-03T03:00:30.001Z", 201)).toBe(
      "applied",
    );
    const stale = state.view(201).rows[0];
    expect(stale?.freshness).toBe("STALE");
    expect(stale?.availability).toBe("LAST_KNOWN");
    expect(stale?.quote).toEqual(row().quote);
    expect(stale?.state_version).toBe("1");
  });

  it("purges access loss, rejects late callbacks and prevents reopening the same consumer", () => {
    const { state, token } = ready();
    expect(emit(state, token, 2, "status", status({ reason_code: "ACCESS_REVOKED" }))).toBe(
      "terminated",
    );
    expect(state.view(100)).toEqual({
      phase: "terminated",
      rows: [],
      terminalReason: "access_revoked",
    });
    expect(emit(state, token, 3, "snapshot", snapshot())).toBe("ignored");
    expect(() => state.beginConnection()).toThrow(IntradayStreamContractError);
    expect(() => state.confirmLeaseExpiry(LEASE, "2026-10-03T03:01:00Z")).toThrow(
      IntradayStreamContractError,
    );
  });

  it("removes values on disabled or invalid-proof status without retaining a last-known price", () => {
    for (const reason_code of [
      "FEATURE_DISABLED",
      "CALENDAR_UNAVAILABLE",
      "SESSION_WINDOW_UNAVAILABLE",
    ]) {
      const { state, token } = ready();
      expect(emit(state, token, 2, "status", status({ reason_code }))).toBe("applied");
      expect(state.view(100).rows[0]?.quote).toBeNull();
      expect(state.view(100).rows[0]?.availability).toBe("UNAVAILABLE");
    }
  });

  it("also purges row-level access revocation and admits no partially authorized snapshot", () => {
    const { state, token } = ready([row(1), row(2)]);
    const revoked: IntradayStreamRow = {
      ...row(2),
      quote: null,
      freshness: "UNAVAILABLE",
      availability: "UNAVAILABLE",
      reason_code: "ACCESS_REVOKED",
    };
    expect(emit(state, token, 2, "delta", { rows: [row(1), revoked] })).toBe("terminated");
    expect(state.view(100).rows).toEqual([]);
    expect(state.view(100).terminalReason).toBe("access_revoked");
  });

  it("checks exact lease and identity scope atomically and clears rows omitted by a replacement snapshot", () => {
    const { state, token } = ready([row(1), row(2)]);
    expect(emit(state, token, 2, "snapshot", snapshot([row(2)]))).toBe("applied");
    expect(state.view(100).rows.map((value) => value.membership_id)).toEqual([uuid(2)]);
    expect(emit(state, token, 3, "delta", { rows: [row(1)] })).toBe("resync");
    expect(state.view(100).rows).toEqual([]);
    for (const bad of [
      { ...row(), generation: 2 },
      { ...row(), instrument_id: "005930.KRX" },
      row(2),
    ]) {
      const scope = ready();
      expect(emit(scope.state, scope.token, 2, "delta", { rows: [bad] })).toBe("resync");
      expect(scope.state.view(100).rows).toEqual([]);
    }
    const scope = ready();
    expect(emit(scope.state, scope.token, 2, "snapshot", snapshot([row()], uuid(999)))).toBe(
      "resync",
    );
  });

  it("rejects counter rollback, receipt rewrites and epoch substitution at equal quote versions", () => {
    const quote = row().quote;
    expect(quote).not.toBeNull();
    if (!quote) throw new Error("fixture quote missing");
    for (const bad of [
      { ...row(), state_version: "0" },
      { ...row(), quote: { ...quote, price: "100001" } },
      { ...row(), quote: { ...quote, epoch: uuid(301) } },
      { ...row(), quote: { ...quote, received_at: "2026-10-03T03:00:00.150Z" } },
      { ...row(), quote: { ...quote, quote_version: "2", receive_ordinal: "2" } },
      { ...row(), state_version: "2", quote: { ...quote, quote_version: "2" } },
    ]) {
      const { state, token } = ready();
      expect(emit(state, token, 2, "delta", { rows: [bad] })).toBe("resync");
      expect(state.view(100).rows).toEqual([]);
    }
    const { state, token } = ready();
    const next = {
      ...row(),
      state_version: "2",
      quote: { ...quote, quote_version: "2", receive_ordinal: "2" },
    };
    expect(emit(state, token, 2, "delta", { rows: [next] })).toBe("applied");
    expect(emit(state, token, 3, "delta", { rows: [{ ...row(), state_version: "3" }] })).toBe(
      "resync",
    );
  });

  it("allows real row namespace replacement while retaining bounded version guards after a null quote", () => {
    const { state, token } = ready();
    const second = row();
    if (!second.quote) throw new Error("fixture quote missing");
    second.state_version = "2";
    second.quote.quote_version = "2";
    second.quote.receive_ordinal = "2";
    expect(emit(state, token, 2, "delta", { rows: [second] })).toBe("applied");
    const unavailable: IntradayStreamRow = {
      ...second,
      quote: null,
      freshness: "UNAVAILABLE",
      availability: "UNAVAILABLE",
      reason_code: "SESSION_WINDOW_UNAVAILABLE",
    };
    expect(emit(state, token, 3, "delta", { rows: [unavailable] })).toBe("applied");
    expect(emit(state, token, 4, "delta", { rows: [{ ...row(), state_version: "3" }] })).toBe(
      "resync",
    );
    const fresh = ready();
    expect(
      emit(fresh.state, fresh.token, 2, "delta", {
        rows: [{ ...row(), row_generation: uuid(999) }],
      }),
    ).toBe("applied");
    expect(fresh.state.view(100).rows[0]?.row_generation).toBe(uuid(999));
  });

  it("bounds state at 30 identities, keeps no tick history and returns detached view objects", () => {
    const rows = Array.from({ length: 30 }, (_, index) => row(index + 1));
    expect(
      () => new IntradayStreamState(LEASE, [...rows.map(identity), identity(row(31))]),
    ).toThrow(IntradayStreamContractError);
    expect(() => new IntradayStreamState(LEASE, [identity(row()), identity(row())])).toThrow(
      IntradayStreamContractError,
    );
    const { state, token } = ready(rows);
    for (let sequence = 2; sequence < 40; sequence++) {
      const next = row(1);
      if (!next.quote) throw new Error("fixture quote missing");
      next.state_version = String(sequence);
      next.quote.quote_version = String(sequence);
      next.quote.receive_ordinal = String(sequence);
      expect(emit(state, token, sequence, "delta", { rows: [next] })).toBe("applied");
      expect(state.view(100).rows).toHaveLength(30);
    }
    const detached = state.view(100).rows[0];
    if (detached?.quote) detached.quote.price = "1";
    expect(state.view(100).rows[0]?.quote?.price).toBe("100000");
    expect(state.view(100).rows[0]?.quote?.quote_version).toBe("39");
  });

  it("expires at the confirmed lease deadline, honors renewal and purges on explicit close", () => {
    const { state, token } = ready();
    state.confirmLeaseExpiry(LEASE, "2026-10-03T03:01:00Z");
    expect(state.view(30_100).terminalReason).toBeNull();
    expect(state.view(59_600).terminalReason).toBe("lease_expired");
    expect(state.view(59_600).rows).toEqual([]);
    expect(emit(state, token, 99, "snapshot", snapshot())).toBe("ignored");
    const closed = ready();
    closed.state.close();
    expect(closed.state.view(100)).toEqual({
      phase: "terminated",
      rows: [],
      terminalReason: "closed",
    });
  });

  it("fails closed on backward clocks, stream substitution, malformed events and delta-before-snapshot", () => {
    const scope = ready();
    expect(emit(scope.state, scope.token, 2, "status", status(), SERVER, 99)).toBe("resync");
    const backward = ready();
    expect(
      emit(backward.state, backward.token, 2, "status", status(), "2026-10-03T03:00:00Z"),
    ).toBe("resync");
    const swapped = ready();
    expect(
      emit(swapped.state, swapped.token, 2, "status", status(), SERVER, 100, NEXT_STREAM),
    ).toBe("resync");
    const malformed = ready();
    expect(
      malformed.state.receive(malformed.token, "snapshot", "{private", `${STREAM}:2`, 100),
    ).toBe("resync");
    const initial = new IntradayStreamState(LEASE, [identity(row())]);
    expect(emit(initial, initial.beginConnection(), 1, "delta", { rows: [row()] })).toBe("resync");
  });
});
