import assert from "node:assert/strict";
import test from "node:test";
import { auditPublication } from "./publication-audit.mjs";

function record(sequence, start, finish = start + 1000000, outcome = "committed", changed = 30) {
  return { sequence, started_elapsed_ns: start, finished_elapsed_ns: finish, planned_rows: 30, changed_rows: changed, outcome };
}

function document(records) {
  const summary = { record_count: records.length, in_flight: 0, committed: 0, confirmed_by_reread: 0, commit_uncertain: 0, dropped_before_commit: 0, attempted_changed_rows: 0, known_committed_changed_rows: 0, overflowed: false, invalid_record: false, diagnostic_error: false };
  for (const item of records) {
    summary[item.outcome]++;
    summary.attempted_changed_rows += item.changed_rows;
    if (item.outcome === "committed") summary.known_committed_changed_rows += item.changed_rows;
  }
  return { schema_version: 1, summary, records };
}

test("half-open windows accept exactly four quarter-second publications", () => {
  const value = auditPublication(document(Array.from({ length: 9 }, (_, i) => record(i + 1, i * 250000000))));
  assert.equal(value.publication_body_starts_per_rolling_second, 4);
  assert.equal(value.acknowledged_commits_per_rolling_second, 4);
  assert.equal(value.known_committed_changed_rows_per_rolling_second, 120);
  assert.equal(value.observed_rate_bounds_pass, true);
});

test("rolling audit catches bursts across wall-second boundaries", () => {
  const starts = [800000000, 850000000, 900000000, 1000000000, 1050000000];
  const value = auditPublication(document(starts.map((start, i) => record(i + 1, start))));
  assert.equal(value.publication_body_starts_per_rolling_second, 5);
  assert.equal(value.acknowledged_commits_per_rolling_second, 5);
  assert.equal(value.known_committed_changed_rows_per_rolling_second, 150);
  assert.equal(value.observed_rate_bounds_pass, false);
});

test("commit observations expose bunching independently of spaced starts", () => {
  const value = auditPublication(document(Array.from({ length: 5 }, (_, i) => record(i + 1, i * 250000000, 1100000000 + i * 1000))));
  assert.equal(value.publication_body_starts_per_rolling_second, 4);
  assert.equal(value.acknowledged_commits_per_rolling_second, 5);
  assert.equal(value.observed_rate_bounds_pass, false);
});

test("reread uncertainty and precommit drops do not inflate known committed rows", () => {
  const value = auditPublication(document([
    record(1, 0, 1, "committed", 7),
    record(2, 2, 3, "confirmed_by_reread", 20),
    record(3, 4, 5, "commit_uncertain", 10),
    record(4, 6, 7, "dropped_before_commit", 9),
  ]));
  assert.equal(value.attempted_changed_rows, 46);
  assert.equal(value.known_committed_changed_rows, 7);
  assert.equal(value.known_committed_changed_rows_per_rolling_second, 7);
  assert.equal(value.acknowledged_commits_per_rolling_second, 1);
  assert.equal(value.confirmed_by_reread, 1);
  assert.equal(value.commit_uncertain, 1);
  assert.equal(value.dropped_before_commit, 1);
  assert.equal(value.commit_outcomes_all_known, false);
});

test("malformed or incomplete evidence fails closed", () => {
  const mutations = [
    (d) => d.records[0].sequence = 2,
    (d) => d.records[0].started_elapsed_ns = -1,
    (d) => d.records[0].finished_elapsed_ns = Number.MAX_SAFE_INTEGER + 1,
    (d) => d.records[0].finished_elapsed_ns = null,
    (d) => d.records[0].changed_rows = 31,
    (d) => d.summary.known_committed_changed_rows++,
    (d) => d.summary.diagnostic_error = true,
    (d) => d.summary.invalid_record = true,
    (d) => d.summary.overflowed = true,
    (d) => d.summary.record_count = "1",
    (d) => d.records[0].extra = "unexpected",
  ];
  for (const mutate of mutations) {
    const value = document([record(1, 0)]); mutate(value);
    assert.throws(() => auditPublication(value), /^Error: PUBLICATION_/);
  }
  assert.throws(() => auditPublication(document([record(1, 0, null, "in_flight", 0)])), /PUBLICATION_UNFINISHED/);
  assert.throws(() => auditPublication(document([record(1, 100, 99)])), /PUBLICATION_FINISH/);
});

test("the full fixed capacity is accepted and an extra record is rejected", () => {
  const records = Array.from({ length: 12000 }, (_, i) => record(i + 1, i * 250000000));
  assert.equal(auditPublication(document(records)).records, 12000);
  records.push(record(12001, 3000000000000));
  assert.throws(() => auditPublication(document(records)), /PUBLICATION_CAPACITY/);
  assert.equal(auditPublication(document([])).records, 0);
});
