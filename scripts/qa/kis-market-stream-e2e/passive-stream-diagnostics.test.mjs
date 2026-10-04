import assert from "node:assert/strict";
import test from "node:test";
import { PassiveStreamDiagnostics, domMetadata } from "./passive-stream-diagnostics.mjs";

const time = "2026-10-03T21:00:00.123456Z";
const row = () => ({ instrument_id: "005930.KRX", availability: "LIVE", connection: "CONNECTED",
  reason_code: null, freshness: "RECENT", gap_open: false,
  quote: { price: "PRIVATE_PRICE_SENTINEL", epoch: "PRIVATE_UUID_SENTINEL", quote_version: "1", receive_ordinal: "2",
    provider_trade_at: "2026-10-04T06:00:00+09:00", received_at: time, committed_at: time } });
const event = (body) => JSON.stringify({ schema_version: 2, event_sequence: "1", server_time: time,
  stream_id: "PRIVATE_STREAM_SENTINEL", headers: "PRIVATE_COOKIE_SENTINEL", body });

test("only bounded allowlisted row metadata survives SSE reduction", () => {
  const audit = new PassiveStreamDiagnostics(); audit.observe("snapshot", event({ rows: [row()] }), 1, 2);
  const result = audit.snapshot();
  assert.equal(result.invalid_events, 0); assert.equal(result.recent[0].rows[0].availability, "LIVE");
  assert.equal(result.recent[0].rows[0].received_at, time);
  assert(!JSON.stringify(result).includes("PRIVATE_"));
  result.recent[0].rows.length = 0; assert.equal(audit.snapshot().recent[0].rows.length, 1);
});

test("status reasons and unknown strings stay finite without losing null", () => {
  const audit = new PassiveStreamDiagnostics();
  audit.observe("status", event({ connection: "CONNECTED", reason_code: null, gap_open: false }), 1, 2);
  assert.equal(audit.snapshot().last_nonlive, null);
  audit.observe("status", event({ connection: "DISCONNECTED", reason_code: "PRODUCER_UNAVAILABLE", gap_open: false }), 2, 3);
  assert.equal(audit.snapshot().last_nonlive.reason_code, "PRODUCER_UNAVAILABLE");
  audit.observe("status", event({ connection: "PRIVATE_UNKNOWN", reason_code: "PRIVATE_UNKNOWN", gap_open: "PRIVATE_UNKNOWN" }), 3, 4);
  const last = audit.snapshot().last_nonlive;
  assert.equal(last.connection, "UNKNOWN"); assert.equal(last.reason_code, "UNKNOWN"); assert.equal(last.gap_open, null);
  assert(!JSON.stringify(last).includes("PRIVATE_"));
});

test("sixteen-event ring retains separate last nonlive evidence", () => {
  const audit = new PassiveStreamDiagnostics();
  audit.observe("reset", event({ reason_code: "RESYNC_REQUIRED" }), 0, 0);
  for (let n = 1; n <= 100; n++) audit.observe("delta", event({ rows: [row()] }), n, n);
  const result = audit.snapshot();
  assert.equal(result.events, 101); assert.equal(result.recent.length, 16);
  assert.equal(result.recent[0].at_ms, 85); assert.equal(result.last_nonlive.kind, "reset");
  assert.deepEqual(result.by_kind, { snapshot: 0, delta: 100, status: 0, reset: 1 });
});

test("malformed oversized and foreign events have no payload history", () => {
  const audit = new PassiveStreamDiagnostics();
  const cases = [["delta", "PRIVATE_INVALID"], ["delta", "X".repeat(65_537)],
    ["delta", "한".repeat(30_000)], ["foreign", event({ rows: [] })],
    ["delta", event({ rows: Array.from({ length: 31 }, row) })], ["delta", "null"]];
  for (const [kind, value] of cases) audit.observe(kind, value, 1, 2);
  audit.observe("delta", event({ rows: [] }), -1, 2);
  assert.equal(audit.snapshot().invalid_events, 7); assert.deepEqual(audit.snapshot().recent, []);
  assert(!JSON.stringify(audit.snapshot()).includes("PRIVATE"));
});

test("event count saturates and reports incomplete diagnostic coverage", () => {
  const audit = new PassiveStreamDiagnostics();
  for (let n = 0; n < 30_010; n++) audit.observe("invalid", "", n, n);
  assert.equal(audit.snapshot().events, 30_000); assert.equal(audit.snapshot().invalid_events, 30_000);
  assert.equal(audit.snapshot().event_limit_reached, true); assert.equal(audit.snapshot().recent.length, 0);
});

test("DOM failure summary excludes prices identifiers and arbitrary text", () => {
  const input = { instrument: "005930.KRX", availability: "LAST_KNOWN", version: "7", ordinal: "9", received_at: time,
    price: "PRIVATE_PRICE", epoch: "PRIVATE_EPOCH", reason: "PRIVATE_REASON" };
  const result = domMetadata(4, Array.from({ length: 31 }, () => input));
  assert.equal(result.row_count, 31); assert.equal(result.rows.length, 30);
  assert.equal(result.rows[0].price_present, true); assert(!JSON.stringify(result).includes("PRIVATE"));
  const invalid = domMetadata(NaN, [{ instrument: "PRIVATE", availability: "PRIVATE", version: "01", ordinal: "9223372036854775808", received_at: "PRIVATE" }]);
  assert.equal(invalid.at_ms, null); assert.equal(invalid.rows[0].version, null); assert.equal(invalid.rows[0].ordinal, null);
  assert(!JSON.stringify(invalid).includes("PRIVATE"));
});
