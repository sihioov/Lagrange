import assert from "node:assert/strict";
import test from "node:test";
import { SoakAudit } from "./soak-audit.mjs";

const rows = (version) => Array.from({ length: 30 }, (_, index) => ({ instrument: `${String(index + 1).padStart(6, "0")}.KRX`, availability: "LIVE", epoch: "one-epoch", version: String(version), ordinal: String(version), price: "1" }));

test("thirty minutes of advancing rows yields bounded scalar evidence", () => {
  const audit = new SoakAudit();
  for (let second = 0; second <= 1800; second++) audit.observe(second * 1000, rows(second + 1));
  assert.deepEqual(audit.finish(), { schema_version: 1, duration_ms: 1800000, samples: 1801, instruments: 30, observed_version_advances: 54000, max_sample_gap_ms: 1000, max_observed_progress_gap_ms: 1000 });
});
test("short duration and a jump over an unobserved interval cannot pass", () => {
  const audit = new SoakAudit(); audit.observe(0, rows(1)); audit.observe(1000, rows(2));
  assert.throws(() => audit.finish(), /SOAK_TOO_SHORT/);
  assert.throws(() => audit.observe(1800000, rows(3)), /SOAK_SAMPLE_GAP/);
  assert.throws(() => audit.finish(), /SOAK_INCOMPLETE/);
});
test("sample times are increasing and the sample count is bounded", () => {
  for (const time of [0, -1, NaN, 0.5]) {
    const audit = new SoakAudit(); audit.observe(0, rows(1)); assert.throws(() => audit.observe(time, rows(2)));
  }
  const audit = new SoakAudit();
  for (let i = 0; i < 2101; i++) audit.observe(i, rows(i + 1));
  assert.throws(() => audit.observe(2101, rows(2102)), /SOAK_SAMPLE_CAP/);
});
test("duplicate, missing, foreign or nonlive instruments fail closed", () => {
  for (const alter of [r => r.pop(), r => { r[1] = r[0]; }, r => { r[0].instrument = "999999.KRX"; }, r => { r[0].availability = "STALE"; }, r => { r[0].price = null; }, r => { r[0].epoch = "new-epoch"; }]) {
    const audit = new SoakAudit(); audit.observe(0, rows(1)); const next = rows(2); alter(next); assert.throws(() => audit.observe(1000, next));
    assert.throws(() => audit.observe(2000, rows(3)), /SOAK_ALREADY_FAILED/);
  }
});
test("regression, unsafe decimal counters and inconsistent receipt progress fail", () => {
  for (const [version, ordinal] of [["1", "1"], ["03", "3"], ["9223372036854775808", "3"], ["3", "2"], ["2", "3"]]) {
    const audit = new SoakAudit(); audit.observe(0, rows(2)); const next = rows(3); next[0].version = version; next[0].ordinal = ordinal;
    assert.throws(() => audit.observe(1000, next));
  }
});
test("a stalled symbol cannot hide behind progress in the other twenty nine", () => {
  const audit = new SoakAudit(); audit.observe(0, rows(1));
  for (let second = 1; second <= 5; second++) { const next = rows(second + 1); next[0] = rows(1)[0]; audit.observe(second * 1000, next); }
  assert.throws(() => audit.observe(6000, rows(7)), /SOAK_STALLED/);
});
