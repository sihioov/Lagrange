// Independent contract invariants, not an oracle generator or product test.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
const here = new URL('./', import.meta.url);
const read = name => JSON.parse(fs.readFileSync(new URL(name, here), 'utf8'));
const baseInput = read('base-input.json');
const baseOutput = read('base-output.json');
const cases = read('cases.json').cases;
const SECOND = 1_000_000n;
// Parse only the whole seconds with Date; preserve all six fractional digits.
function instant(value) {
  assert.equal(typeof value, 'string');
  const match = /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d{1,6}))?Z$/.exec(value);
  assert(match, `invalid exact instant: ${value}`);
  const milliseconds = Date.parse(`${match[1]}Z`);
  assert(Number.isFinite(milliseconds));
  return BigInt(milliseconds) * 1000n + BigInt((match[2] ?? '').padEnd(6, '0'));
}
function apply(base, operations) {
  const result = structuredClone(base);
  for (const op of operations) {
    const keys = op.path.split('/').slice(1).map(k => k.replaceAll('~1', '/').replaceAll('~0', '~'));
    assert(keys.length && keys.every(k => !['__proto__', 'constructor', 'prototype'].includes(k)));
    let parent = result;
    for (const key of keys.slice(0, -1)) { assert(Object.hasOwn(parent, key)); parent = parent[key]; }
    const key = keys.at(-1);
    if (op.op !== 'add') assert(Object.hasOwn(parent, key), op.path);
    if (op.op === 'remove') { if (Array.isArray(parent)) parent.splice(Number(key), 1); else delete parent[key]; }
    else { assert(['replace', 'add'].includes(op.op)); parent[key] = structuredClone(op.value); }
  }
  return result;
}
function bounded(value, ceiling, label) {
  assert(value !== null, `${label}: missing accepted bound`);
  assert(instant(value) <= ceiling, `${label}: ${value} exceeds prerequisite`);
}
assert.equal(cases.length, 124);
assert.equal(new Set(cases.map(c => c.id)).size, 124);
assert.equal(createHash('sha256').update(cases.map(c => c.id).join('\n')).digest('hex'),
  'cb72c2528261fa2c2beed62b1596920484a8639a85057ae9367eaed645c47a8d', 'frozen case IDs/order changed');
const byId = new Map();
let checkedPositiveStreams = 0;
const queueTypes = new Map([
  ['owner-equity-v2-runner', 'owner_equity_v2'],
  ['owner-beta-runner', 'owner_beta_price_recommendation'],
  ['candidate-runner', 'candidate_compute'],
  ['recommendation-runner', 'recommendation'],
  ['nt-backtest-worker-1', 'backtest'], ['nt-backtest-worker-2', 'backtest'],
]);
for (const c of cases) {
  const input = apply(baseInput, c.input_patch);
  const expected = apply(baseOutput, c.expected_patch);
  byId.set(c.id, { input, expected });
  const now = instant(input.clock);
  const badWindowResult = {
    window_missing: 'missing', window_invalid: 'invalid', window_future_evidence: 'future',
  }[c.id];
  if (badWindowResult) {
    for (const index of [12, 13]) {
      assert.deepEqual(expected.services[index].sources.window,
        { result: badWindowResult, observed_at: null, valid_until: null }, `${c.id}: invalid accepted window source`);
    }
  }
  for (const row of expected.services) {
    const database = row.sources.database;
    if (database?.valid_until !== null && database?.valid_until !== undefined) {
      bounded(database.valid_until, instant(database.observed_at) + 30n * SECOND, `${c.id}/${row.id}/database`);
    }
    if (row.valid_until !== null) {
      for (const source of Object.values(row.sources)) {
        if (source?.valid_until) bounded(row.valid_until, instant(source.valid_until), `${c.id}/${row.id}/row`);
      }
    }
    const queue = input.db_read_results.queue;
    if (queue.result === 'ok' && queueTypes.has(row.id)) {
      const jobs = queue.rows.filter(job => job.job_type === queueTypes.get(row.id));
      for (const job of jobs) {
        if (job.status === 'QUEUED' && row.reason_codes.includes('QUEUE_PENDING')) {
          const available = instant(job.available_at);
          if (available > now) bounded(row.valid_until, available, `${c.id}/queued availability`);
          else if (now - available <= 120n * SECOND) bounded(row.valid_until, available + 120n * SECOND, `${c.id}/pending threshold`);
        }
        if (job.status === 'RUNNING' && row.metrics.running > 0 && !row.reason_codes.includes('CLAIM_EXPIRED')) {
          bounded(row.valid_until, instant(job.locked_at) + BigInt(queue.lease_seconds) * SECOND, `${c.id}/claim lease`);
        }
        if (job.status === 'FAILED' && row.metrics.recent_failed > 0 && now - instant(job.finished_at) <= 86400n * SECOND) {
          bounded(row.valid_until, instant(job.finished_at) + 86400n * SECOND, `${c.id}/failure window`);
        }
      }
    }
  }
  const stream = input.db_read_results.stream;
  const row = expected.services.find(s => s.id === 'market-stream');
  if (!stream.owner_binding_valid) {
    assert.equal(stream.valid_until, null, `${c.id}: zero rows have no aggregate assurance`);
    for (const metric of ['active_demand', 'live_identities', 'halted_identities', 'last_received_at', 'oldest_provider_event_at']) {
      assert.equal(row.metrics[metric], null, `${c.id}: zero-row state exposed`);
    }
  }
  if (stream.result !== 'ok' || !stream.owner_binding_valid) continue;
  const covered = stream.live_count + stream.halted_count;
  assert(covered <= stream.demand_count, `${c.id}: exclusive counts exceed demand`);
  if (stream.valid_until !== null && row.sources.database?.result === 'ok') {
    bounded(row.sources.database.valid_until, instant(stream.valid_until), `${c.id}/G1 aggregate`);
  }
  if (!covered) continue;
  checkedPositiveStreams++;
  const observed = instant(stream.observed_at);
  const window = input.window_read;
  assert.equal(window.result, 'ok', `${c.id}: positive coverage with failed window`);
  assert.equal(window.activation_hash_matches, true);
  assert.equal(window.metadata_safe, true);
  assert.equal(window.entry.disposition, 'trading', `${c.id}: positive coverage in CLOSED entry`);
  assert(instant(window.entry.evidence_retrieved_at) <= observed, `${c.id}: future window proof`);
  assert(instant(window.entry.open_at) <= observed && observed < instant(window.entry.close_at), `${c.id}: positive coverage outside open window`);
  assert.equal(stream.producer_present, true);
  assert.equal(stream.connection_state, 'CONNECTED');
  assert.equal(stream.status_code, null);
  assert.equal(stream.lineage_valid, true);
  assert.equal(stream.gap_present, false);
  const heartbeat = instant(stream.producer_heartbeat_at);
  const lease = instant(stream.producer_lease_expires_at);
  assert(heartbeat <= observed && observed - heartbeat <= 10n * SECOND, `${c.id}: invalid positive heartbeat`);
  assert(lease > observed + 5n * SECOND, `${c.id}: invalid positive lease margin`);
  bounded(stream.valid_until, heartbeat + 10n * SECOND, `${c.id}/heartbeat assurance`);
  bounded(stream.valid_until, lease - 5n * SECOND, `${c.id}/producer lease assurance`);
  bounded(stream.valid_until, instant(window.entry.close_at), `${c.id}/window close assurance`);
  assert(instant(stream.valid_until) >= observed, `${c.id}: positive predicate already expired at observation`);
  if (covered === stream.demand_count) {
    for (const field of ['oldest_received_at', 'oldest_provider_trade_at']) {
      const captured = instant(stream[field]);
      assert(captured <= observed && observed - captured <= 30n * SECOND, `${c.id}: invalid full-coverage ${field}`);
      bounded(stream.valid_until, captured + 30n * SECOND, `${c.id}/${field} assurance`);
    }
  }
}
// These are independent normative equalities, not values computed by a reducer.
const exactBounds = [
  ['queue_age_120', 7, '2032-02-12T01:00:00Z', 'QUEUE_PENDING'],
  ['job_failure_age_86400', 5, '2032-02-12T01:00:00Z', 'RECENT_JOB_FAILURE'],
  ['process_start_age_30', 2, '2032-02-12T01:01:00Z', 'PROCESS_STARTING'],
  ['process_start_age_90', 2, '2032-02-12T01:00:00Z', 'PROCESS_STARTING'],
  ['producer_heartbeat_10', 12, '2032-02-12T01:00:00Z', 'DATA_CURRENT'],
  ['producer_lease_margin_5_000001', 12, '2032-02-12T01:00:00.000001Z', 'DATA_CURRENT'],
  ['oldest_received_at_30', 12, '2032-02-12T01:00:00Z', 'DATA_CURRENT'],
  ['oldest_provider_trade_at_30', 12, '2032-02-12T01:00:00Z', 'DATA_CURRENT'],
  ['all_demand_halted', 12, '2032-02-12T01:00:09Z', 'INSTRUMENT_HALTED'],
  ['one_stale_symbol_not_hidden', 12, '2032-02-12T01:00:09Z', 'QUOTE_STALE'],
  ['preopen', 12, '2032-02-12T01:00:00.000001Z', 'SESSION_PREOPEN'],
  ['preopen', 13, '2032-02-12T01:00:00.000001Z', 'DATA_CURRENT'],
  ['before_close', 12, '2032-02-12T01:00:00.000001Z', 'DATA_CURRENT'],
  ['before_close', 13, '2032-02-12T01:00:00.000001Z', 'DATA_CURRENT'],
];
for (const [id, index, bound, reason] of exactBounds) {
  const row = byId.get(id).expected.services[index];
  assert.equal(row.valid_until, bound, `${id}/${index}: exact conservative bound`);
  assert(row.reason_codes.includes(reason), `${id}: predicate truth at equality changed`);
}
for (const [id, bound] of [['eod_before_cutoff', '2032-02-12T07:30:00Z'], ['eod_before_grace', '2032-02-12T09:30:00Z']]) {
  const { input, expected } = byId.get(id);
  for (const index of [4, 6, 11]) {
    assert.equal(expected.services[index].valid_until, bound, `${id}/${index}: EOD transition`);
    assert(instant(expected.services[index].sources.database.valid_until) > instant(bound), `${id}: raw DB observation should retain its own ceiling`);
  }
  assert.equal(expected.services[7].valid_until, expected.services[7].sources.database.valid_until, `${id}: unrelated queue capped by EOD`);
  assert.equal(input.db_read_results.stream.demand_count, 2, `${id}: closed demand fabricated`);
}
for (const id of ['queue_age_120_000001', 'job_failure_age_86400_000001']) {
  const { input, expected } = byId.get(id);
  const index = id.startsWith('queue') ? 7 : 5;
  assert.equal(instant(expected.services[index].valid_until), instant(input.db_read_results.queue.observed_at) + 30n * SECOND, `${id}: elapsed boundary expired fresh diagnosis`);
}
const halted = byId.get('all_demand_halted').expected.services[12];
assert.equal(halted.status, 'waiting');
assert.equal(halted.work_status, 'closed');
assert.equal(byId.get('market_closed').input.db_read_results.stream.demand_count, 2);
// Metadata must retain its actual source, and active input cannot invent completion.
const missingNext = byId.get('daily_next_run_missing');
assert.equal(missingNext.input.host_read.snapshot.observations[11].timer.next_run_at, null);
assert.equal(missingNext.expected.services[11].metrics.next_run_at, null);
const active = byId.get('daily_active');
const activeHost = active.input.host_read.snapshot.observations[11];
assert.equal(activeHost.timer.last_finished_at, null);
assert.equal(activeHost.timer.exit_status, null);
assert.equal(activeHost.process.finished_at, null);
assert.equal(activeHost.process.exit_code, null);
assert.equal(activeHost.timer.last_success_at, baseInput.host_read.snapshot.observations[11].timer.last_success_at);
assert.equal(active.expected.services[11].metrics.last_success_at, baseOutput.services[11].metrics.last_success_at);
const blockedEtf = byId.get('etf_blocked');
assert.equal(blockedEtf.input.db_read_results.etf.dataset_state, 'BLOCKED');
assert.equal(blockedEtf.expected.services[11].metrics.data_as_of, null);
assert.equal(blockedEtf.expected.services[11].metrics.raw_as_of, blockedEtf.input.db_read_results.etf.raw_as_of);
assert.equal(blockedEtf.expected.services[11].metrics.last_success_at, blockedEtf.input.db_read_results.research.row.last_success_at);
// Read-only small domain vectors cover categories absent from projection cases.
const wire = new Map(read('wire-cases.json').cases.map(c => [c.id, c]));
for (const [id, duration, field, expectedWork] of [
  ['research_heartbeat_equal', 90n, 'heartbeat_at', 'active'],
  ['research_heartbeat_after', 90n, 'heartbeat_at', 'degraded'],
  ['run_timeout_equal', 43200n, 'started_at', 'active'],
  ['run_timeout_after', 43200n, 'started_at', 'failed'],
]) {
  const vector = wire.get(id);
  const age = instant(vector.input.clock) - instant(vector.input[field]);
  assert.equal(age, duration * SECOND + (id.endsWith('_after') ? 1n : 0n));
  assert.equal(vector.expected.work_status, expectedWork);
}
console.log(JSON.stringify({ projection_cases: cases.length, positive_stream_inputs: checkedPositiveStreams, exact_boundary_assertions: exactBounds.length, metadata_input_cases: 3, readonly_research_run_vectors: 4, arithmetic: 'integer microseconds', product_tests_run: 0 }));
