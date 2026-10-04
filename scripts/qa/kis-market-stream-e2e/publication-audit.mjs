// Pure publication audit. Importing this module performs no I/O or runtime work.
// Timestamps are client monotonic observations after transaction setup and after
// commit acknowledgement. They are not PostgreSQL-internal begin/commit times.
const RECORD_LIMIT = 12000;
const SECOND_NS = 1000000000;
const SUMMARY_KEYS = ["record_count", "in_flight", "committed", "confirmed_by_reread", "commit_uncertain", "dropped_before_commit", "attempted_changed_rows", "known_committed_changed_rows", "overflowed", "invalid_record", "diagnostic_error"];
const RECORD_KEYS = ["sequence", "started_elapsed_ns", "finished_elapsed_ns", "planned_rows", "changed_rows", "outcome"];
const OUTCOMES = new Set(["in_flight", "committed", "confirmed_by_reread", "commit_uncertain", "dropped_before_commit"]);

function requireValue(condition, code) {
  if (!condition) throw new Error(code);
}

function exactKeys(value, expected) {
  requireValue(value !== null && typeof value === "object" && !Array.isArray(value), "PUBLICATION_OBJECT");
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  requireValue(actual.length === wanted.length && actual.every((key, index) => key === wanted[index]), "PUBLICATION_KEYS");
}

function uint(value) {
  requireValue(Number.isSafeInteger(value) && value >= 0, "PUBLICATION_INTEGER");
  return value;
}

// All observations in (right-1s, right] are counted. Events exactly one second
// apart are excluded from the same window; wall-clock buckets are never used.
function maximumWindow(events) {
  const sorted = [...events].sort((left, right) => left.time - right.time);
  let first = 0;
  let rows = 0;
  let maxCount = 0;
  let maxRows = 0;
  for (let last = 0; last < sorted.length; last++) {
    rows += sorted[last].rows;
    while (sorted[last].time - sorted[first].time >= SECOND_NS) {
      rows -= sorted[first++].rows;
    }
    maxCount = Math.max(maxCount, last - first + 1);
    maxRows = Math.max(maxRows, rows);
  }
  return { max_count: maxCount, max_changed_rows: maxRows };
}

export function auditPublication(document) {
  exactKeys(document, ["schema_version", "summary", "records"]);
  requireValue(document.schema_version === 1, "PUBLICATION_SCHEMA");
  exactKeys(document.summary, SUMMARY_KEYS);
  const summary = document.summary;
  for (const key of SUMMARY_KEYS.slice(0, 8)) uint(summary[key]);
  for (const flag of SUMMARY_KEYS.slice(8)) requireValue(summary[flag] === false, "PUBLICATION_DIAGNOSTIC");
  requireValue(Array.isArray(document.records) && document.records.length <= RECORD_LIMIT, "PUBLICATION_CAPACITY");
  const counts = Object.fromEntries(SUMMARY_KEYS.slice(0, 8).map((key) => [key, 0]));
  const starts = [];
  const commits = [];
  let previousStart = 0;
  let lastFinish = 0;
  for (const [index, record] of document.records.entries()) {
    exactKeys(record, RECORD_KEYS);
    requireValue(uint(record.sequence) === index + 1, "PUBLICATION_SEQUENCE");
    uint(record.started_elapsed_ns);
    requireValue(record.started_elapsed_ns >= previousStart, "PUBLICATION_START_ORDER");
    previousStart = record.started_elapsed_ns;
    requireValue(uint(record.planned_rows) >= 1 && record.planned_rows <= 30, "PUBLICATION_ROWS");
    requireValue(uint(record.changed_rows) <= record.planned_rows, "PUBLICATION_ROWS");
    requireValue(OUTCOMES.has(record.outcome), "PUBLICATION_OUTCOME");
    if (record.outcome === "in_flight") {
      requireValue(record.finished_elapsed_ns === null, "PUBLICATION_FINISH");
    } else {
      uint(record.finished_elapsed_ns);
      requireValue(record.finished_elapsed_ns >= record.started_elapsed_ns, "PUBLICATION_FINISH");
      lastFinish = Math.max(lastFinish, record.finished_elapsed_ns);
    }
    counts.record_count++;
    counts[record.outcome]++;
    counts.attempted_changed_rows += record.changed_rows;
    starts.push({ time: record.started_elapsed_ns, rows: 0 });
    if (record.outcome === "committed") {
      counts.known_committed_changed_rows += record.changed_rows;
      commits.push({ time: record.finished_elapsed_ns, rows: record.changed_rows });
    }
  }
  for (const [key, count] of Object.entries(counts)) requireValue(summary[key] === count, "PUBLICATION_SUMMARY_MISMATCH");
  requireValue(counts.in_flight === 0, "PUBLICATION_UNFINISHED");
  const bodyStarts = maximumWindow(starts);
  const acknowledgedCommits = maximumWindow(commits);
  return {
    schema_version: 1,
    records: counts.record_count,
    observed_span_ns: counts.record_count ? lastFinish - document.records[0].started_elapsed_ns : 0,
    publication_body_starts_per_rolling_second: bodyStarts.max_count,
    acknowledged_commits_per_rolling_second: acknowledgedCommits.max_count,
    known_committed_changed_rows_per_rolling_second: acknowledgedCommits.max_changed_rows,
    attempted_changed_rows: counts.attempted_changed_rows,
    known_committed_changed_rows: counts.known_committed_changed_rows,
    confirmed_by_reread: counts.confirmed_by_reread,
    commit_uncertain: counts.commit_uncertain,
    dropped_before_commit: counts.dropped_before_commit,
    observed_rate_bounds_pass: bodyStarts.max_count <= 4 && acknowledgedCommits.max_count <= 4 && acknowledgedCommits.max_changed_rows <= 120,
    commit_outcomes_all_known: counts.confirmed_by_reread === 0 && counts.commit_uncertain === 0,
  };
}
