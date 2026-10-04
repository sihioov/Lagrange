import assert from "node:assert/strict";
import test from "node:test";
import { auditResources } from "./resource-audit.mjs";

function sample(time, count, rss = 1000000 + time) {
  return { elapsed_ms: time, process_rss_bytes: rss,
    buffer: { adapter_calls: 2 * count, buffered: count, replaced: count, stale: 0, offer_age_lag: 0, rejected: 0, handoff_age_lag_drops: 0, peak_pending_slots: 30, peak_high_water_slots: 30, peak_outstanding_slots: 30, overflowed: false, diagnostic_error: false },
    heap: { schema_version: 1, scope: "global_allocator_usable_blocks", generation: 1, elapsed_ms: time, baseline_current_usable_bytes: 4000, current_usable_bytes: 6000, conservative_peak_usable_bytes: 8000, diagnostic_error: false } };
}
function document() {
  const observations = [sample(1000, 1), sample(2000, 2), sample(3000, 3)];
  const final = sample(4000, 4);
  return { buffer_summary: final.buffer, heap_summary: final.heap, process_rss_bytes: 900000, resource_failure: null, observations };
}

test("bounded counters, allocator peak and sampled RSS remain distinct", () => {
  const value = auditResources(document());
  assert.equal(value.adapter_calls, 8); assert.equal(value.coalesced_replacements, 4);
  assert.equal(value.global_allocator_conservative_peak_usable_bytes, 8000);
  assert.equal(value.rss_first_bytes, 1001000); assert.equal(value.rss_final_after_cleanup_bytes, 900000);
  assert.equal(value.rss_ols_slope_bytes_per_minute, 60000);
  assert.equal(value.tracked_global_superset_within_8_mib, true);
  assert.equal(value.coalescing_observed, true);
});
test("missing evidence, extra scalar fields and diagnostics fail closed", () => {
  for (const mutate of [d => { d.heap_summary = null; }, d => { d.resource_failure = "FAIL"; }, d => { d.heap_summary.extra = 1; }, d => { d.buffer_summary.overflowed = true; }, d => { d.heap_summary.scope = "rss"; }, d => { d.heap_summary.current_usable_bytes = NaN; }]) {
    const d = document(); mutate(d); assert.throws(() => auditResources(d));
  }
});
test("buffer conservation, integer overflow and every slot cap are checked", () => {
  for (const mutate of [d => { d.buffer_summary.adapter_calls++; }, d => { d.buffer_summary.buffered = Number.MAX_SAFE_INTEGER; }, ...["peak_pending_slots", "peak_high_water_slots", "peak_outstanding_slots"].map(k => d => { d.buffer_summary[k] = 31; })]) {
    const d = document(); mutate(d); assert.throws(() => auditResources(d));
  }
});
test("window identity, time, cumulative counts and peaks never regress", () => {
  for (const mutate of [d => { d.observations[1].heap.generation++; }, d => { d.observations[1].heap.baseline_current_usable_bytes++; }, d => { d.observations[1].elapsed_ms = 1000; }, d => { d.observations[1].heap.elapsed_ms = 0; }, d => { d.observations[1].heap.conservative_peak_usable_bytes = 7999; }, d => { d.observations[1].buffer = sample(0, 0).buffer; }, d => { d.heap_summary.elapsed_ms = 1; }]) {
    const d = document(); mutate(d); assert.throws(() => auditResources(d));
  }
});
test("the heap bound uses full peak without baseline or RSS subtraction", () => {
  const d = document();
  for (const h of [d.heap_summary, ...d.observations.map(o => o.heap)]) {
    h.baseline_current_usable_bytes = 7 * 1024 * 1024;
    h.conservative_peak_usable_bytes = 9 * 1024 * 1024;
  }
  const value = auditResources(d);
  assert.equal(value.global_allocator_conservative_peak_usable_bytes, 9 * 1024 * 1024);
  assert.equal(value.tracked_global_superset_within_8_mib, false);
});
test("empty, excessive and invalid observation series are rejected", () => {
  for (const observations of [[], [sample(1, 1)], Array(3001).fill(sample(1, 1)), [sample(1, 1), sample(2, 2, 0)]]) {
    const d = document(); d.observations = observations; assert.throws(() => auditResources(d));
  }
});
test("RSS reductions are reported as a negative slope, not negative heap", () => {
  const d = document(); d.observations.forEach((o, i) => { o.process_rss_bytes = 3000000 - i * 1000; });
  const value = auditResources(d);
  assert.equal(value.rss_ols_slope_bytes_per_minute, -60000);
  assert.equal(value.global_allocator_conservative_peak_usable_bytes, 8000);
});
test("zero replacements never fabricate coalescing evidence", () => {
  const d = document();
  for (const b of [d.buffer_summary, ...d.observations.map(o => o.buffer)]) { b.buffered = b.adapter_calls; b.replaced = 0; }
  assert.equal(auditResources(d).coalescing_observed, false);
});
