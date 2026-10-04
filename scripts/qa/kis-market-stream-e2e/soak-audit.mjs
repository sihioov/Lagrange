// Bounded DOM progress evidence for one continuously mounted 30-instrument page.
// No I/O, timers, payload history, browser patching, or synthetic receipts.
import assert from "node:assert/strict";

const DURATION_MS = 1_800_000;
const MAX_GAP_MS = 5_000;
const MAX_SAMPLES = 2_101;
const MAX_COUNTER = 9_223_372_036_854_775_807n;

function counter(text) {
  assert(typeof text === "string" && /^[1-9][0-9]{0,18}$/.test(text), "SOAK_COUNTER");
  const value = BigInt(text);
  assert(value <= MAX_COUNTER, "SOAK_COUNTER");
  return value;
}

export class SoakAudit {
  #started;
  #previous;
  #rows = new Map();
  #samples = 0;
  #advances = 0;
  #maxSampleGap = 0;
  #maxProgressGap = 0;
  #failed = false;

  observe(nowMs, rows) {
    assert(!this.#failed, "SOAK_ALREADY_FAILED");
    try {
      assert(Number.isSafeInteger(nowMs) && nowMs >= 0, "SOAK_TIME");
      assert(this.#samples < MAX_SAMPLES, "SOAK_SAMPLE_CAP");
      if (this.#previous !== undefined) {
        const gap = nowMs - this.#previous;
        assert(gap > 0 && gap <= MAX_GAP_MS, "SOAK_SAMPLE_GAP");
        this.#maxSampleGap = Math.max(this.#maxSampleGap, gap);
      } else this.#started = nowMs;
      assert(Array.isArray(rows) && rows.length === 30, "SOAK_ROW_COUNT");
      const seen = new Set();
      for (const row of rows) {
        assert(row && typeof row === "object", "SOAK_ROW");
        assert(typeof row.instrument === "string" && /^[0-9]{6}\.KRX$/.test(row.instrument), "SOAK_INSTRUMENT");
        assert(!seen.has(row.instrument), "SOAK_DUPLICATE");
        seen.add(row.instrument);
        assert(row.availability === "LIVE" && typeof row.price === "string" && row.price.length > 0, "SOAK_NOT_LIVE");
        assert(typeof row.epoch === "string" && row.epoch.length > 0 && row.epoch.length <= 64, "SOAK_EPOCH");
        const version = counter(row.version);
        const ordinal = counter(row.ordinal);
        const previous = this.#rows.get(row.instrument);
        if (this.#samples === 0) {
          this.#rows.set(row.instrument, { epoch: row.epoch, version, ordinal, advancedAt: nowMs });
          continue;
        }
        assert(previous && row.epoch === previous.epoch, "SOAK_IDENTITY_CHANGED");
        assert(version >= previous.version && ordinal >= previous.ordinal, "SOAK_REGRESSION");
        const gap = nowMs - previous.advancedAt;
        assert(gap <= MAX_GAP_MS, "SOAK_STALLED");
        this.#maxProgressGap = Math.max(this.#maxProgressGap, gap);
        if (version > previous.version) {
          assert(ordinal > previous.ordinal, "SOAK_ORDINAL_DID_NOT_ADVANCE");
          previous.version = version;
          previous.ordinal = ordinal;
          previous.advancedAt = nowMs;
          this.#advances++;
        } else assert(ordinal === previous.ordinal, "SOAK_VERSION_DID_NOT_ADVANCE");
      }
      this.#previous = nowMs;
      this.#samples++;
    } catch (error) {
      this.#failed = true;
      throw error;
    }
  }

  finish() {
    assert(!this.#failed && this.#samples >= 2 && this.#rows.size === 30, "SOAK_INCOMPLETE");
    const duration = this.#previous - this.#started;
    assert(duration >= DURATION_MS, "SOAK_TOO_SHORT");
    return {
      schema_version: 1,
      duration_ms: duration,
      samples: this.#samples,
      instruments: this.#rows.size,
      observed_version_advances: this.#advances,
      max_sample_gap_ms: this.#maxSampleGap,
      max_observed_progress_gap_ms: this.#maxProgressGap,
    };
  }
}
