// Pure metadata audit. Usable global-allocator blocks and process RSS are
// distinct measurements. A failed superset bound is not stream heap attribution.
const BUDGET = 8 * 1024 * 1024;
const BUFFER_COUNTERS = ["adapter_calls", "buffered", "replaced", "stale", "offer_age_lag", "rejected", "handoff_age_lag_drops"];
const BUFFER_PEAKS = ["peak_pending_slots", "peak_high_water_slots", "peak_outstanding_slots"];
const HEAP_KEYS = ["schema_version", "scope", "generation", "elapsed_ms", "baseline_current_usable_bytes", "current_usable_bytes", "conservative_peak_usable_bytes", "diagnostic_error"];

function requireValue(condition, code) {
  if (!condition) throw new Error(code);
}
function object(value) {
  requireValue(value !== null && typeof value === "object" && !Array.isArray(value), "RESOURCE_OBJECT");
}
function keys(value, expected) {
  object(value);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  requireValue(actual.length === wanted.length && actual.every((key, index) => key === wanted[index]), "RESOURCE_KEYS");
}
function uint(value) {
  requireValue(Number.isSafeInteger(value) && value >= 0, "RESOURCE_INTEGER");
  return value;
}
function buffer(value) {
  keys(value, [...BUFFER_COUNTERS, ...BUFFER_PEAKS, "overflowed", "diagnostic_error"]);
  for (const name of BUFFER_COUNTERS) uint(value[name]);
  for (const name of BUFFER_PEAKS) requireValue(uint(value[name]) <= 30, "RESOURCE_SLOT_CAP");
  requireValue(value.overflowed === false && value.diagnostic_error === false, "RESOURCE_DIAGNOSTIC");
  const sum = ["buffered", "replaced", "stale", "offer_age_lag", "rejected"].reduce((total, name) => uint(total + value[name]), 0);
  requireValue(sum === value.adapter_calls, "RESOURCE_BUFFER_CONSERVATION");
}
function heap(value) {
  keys(value, HEAP_KEYS);
  requireValue(value.schema_version === 1 && value.scope === "global_allocator_usable_blocks", "RESOURCE_HEAP_SCOPE");
  requireValue(uint(value.generation) > 0 && value.diagnostic_error === false, "RESOURCE_DIAGNOSTIC");
  for (const key of ["elapsed_ms", "baseline_current_usable_bytes", "current_usable_bytes", "conservative_peak_usable_bytes"]) uint(value[key]);
  requireValue(value.conservative_peak_usable_bytes >= value.baseline_current_usable_bytes && value.conservative_peak_usable_bytes >= value.current_usable_bytes, "RESOURCE_HEAP_PEAK");
}
function progresses(before, after) {
  requireValue(before.heap.generation === after.heap.generation && before.heap.baseline_current_usable_bytes === after.heap.baseline_current_usable_bytes, "RESOURCE_WINDOW_IDENTITY");
  requireValue(after.heap.elapsed_ms >= before.heap.elapsed_ms && after.heap.conservative_peak_usable_bytes >= before.heap.conservative_peak_usable_bytes, "RESOURCE_WINDOW_ORDER");
  for (const name of [...BUFFER_COUNTERS, ...BUFFER_PEAKS]) requireValue(after.buffer[name] >= before.buffer[name], "RESOURCE_COUNTER_ORDER");
}

export function auditResources(document) {
  keys(document, ["buffer_summary", "heap_summary", "process_rss_bytes", "resource_failure", "observations"]);
  requireValue(document.resource_failure === null, "RESOURCE_DIAGNOSTIC");
  const final = { buffer: document.buffer_summary, heap: document.heap_summary };
  buffer(final.buffer); heap(final.heap);
  requireValue(uint(document.process_rss_bytes) > 0, "RESOURCE_RSS");
  const observations = document.observations;
  requireValue(Array.isArray(observations) && observations.length >= 2 && observations.length <= 3000, "RESOURCE_OBSERVATION_COUNT");
  let previous;
  let firstTime;
  let sumX = 0, sumY = 0, sumXX = 0, sumXY = 0, maxRss = 0;
  for (const item of observations) {
    object(item); buffer(item.buffer); heap(item.heap); uint(item.elapsed_ms);
    requireValue(uint(item.process_rss_bytes) > 0, "RESOURCE_RSS");
    if (previous) {
      requireValue(item.elapsed_ms > previous.elapsed_ms, "RESOURCE_OBSERVATION_ORDER");
      progresses(previous, item);
    } else firstTime = item.elapsed_ms;
    progresses(item, final);
    const seconds = (item.elapsed_ms - firstTime) / 1000;
    sumX += seconds; sumY += item.process_rss_bytes;
    sumXX += seconds * seconds; sumXY += seconds * item.process_rss_bytes;
    maxRss = Math.max(maxRss, item.process_rss_bytes);
    previous = item;
  }
  const count = observations.length;
  const denominator = count * sumXX - sumX * sumX;
  requireValue(denominator > 0, "RESOURCE_RSS_TIME_SPAN");
  const slope = 60 * (count * sumXY - sumX * sumY) / denominator;
  requireValue(Number.isFinite(slope), "RESOURCE_RSS_SLOPE");
  return {
    schema_version: 1,
    observation_count: count,
    observation_span_ms: previous.elapsed_ms - firstTime,
    heap_scope: final.heap.scope,
    heap_window_ms: final.heap.elapsed_ms,
    global_allocator_baseline_usable_bytes: final.heap.baseline_current_usable_bytes,
    global_allocator_current_usable_bytes: final.heap.current_usable_bytes,
    global_allocator_conservative_peak_usable_bytes: final.heap.conservative_peak_usable_bytes,
    tracked_global_superset_within_8_mib: final.heap.conservative_peak_usable_bytes <= BUDGET,
    adapter_calls: final.buffer.adapter_calls,
    newly_buffered: final.buffer.buffered,
    coalesced_replacements: final.buffer.replaced,
    stale_offers: final.buffer.stale,
    offer_age_lag: final.buffer.offer_age_lag,
    rejected_offers: final.buffer.rejected,
    handoff_age_lag_drops: final.buffer.handoff_age_lag_drops,
    coalescing_observed: final.buffer.replaced > 0,
    peak_pending_slots: final.buffer.peak_pending_slots,
    peak_high_water_slots: final.buffer.peak_high_water_slots,
    peak_outstanding_slots: final.buffer.peak_outstanding_slots,
    rss_first_bytes: observations[0].process_rss_bytes,
    rss_last_observation_bytes: previous.process_rss_bytes,
    rss_final_after_cleanup_bytes: document.process_rss_bytes,
    rss_peak_sampled_bytes: Math.max(maxRss, document.process_rss_bytes),
    rss_ols_slope_bytes_per_minute: slope,
  };
}
